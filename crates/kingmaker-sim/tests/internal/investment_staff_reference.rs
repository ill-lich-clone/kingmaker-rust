//! Funded construction crews are activated atomically from current source assignments.
use kingmaker_core::world_read::WorldRead;
use crate::{
    budget_funding::error,
    construction_operations::stage_construction_operations,
    operation_session::{OpResult, OperationSession},
    transport_validation::actor,
};
use kingmaker_core::{
    construction::ConstructionRequest, operations::EconomicOperation, production::LaborAssignment,
};
use serde_json::{json, Value};
use std::collections::HashSet;
pub(crate) fn activate_staff(
    w: &mut crate::world_transaction::WorldTransaction<'_>,
    r: &ConstructionRequest,
) -> OpResult<(Vec<EconomicOperation>, Vec<Value>)> {
    let mut operations = vec![];
    let mut blocked = vec![];
    for index in 0..w.construction_projects().len() {
        crate::run_control::work("investment_staff")?;
        let original = w.construction_projects().at(index).expect("validated row position");
        if original.status != "funded"
            || original.extra.get("staff_activated") == Some(&json!(true))
        {
            continue;
        }
        let Some(planned) = original
            .extra
            .get("planned_assignments")
            .and_then(Value::as_array)
        else {
            continue;
        };
        let mut p = original.clone();
        p.assignments = serde_json::from_value(json!(planned))?;
        let transfers = p
            .extra
            .get("staff_transfers")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let mut raw = w.resource_operations().iter().cloned().collect::<Vec<_>>();
        let mut employment = w.employment().iter().cloned().collect::<Vec<_>>();
        let mut valid = true;
        for transfer in &transfers {
            let source = actor(&transfer["source_rgo_id"])?;
            let id = actor(&transfer["employment_id"])?;
            let pop = actor(&transfer["pop_id"])?;
            let Some(source) = raw.iter_mut().find(|r| r.base.id == source) else {
                valid = false;
                break;
            };
            let Some(a) = source
                .base
                .assignments
                .iter_mut()
                .find(|a| a.employment_id.as_ref() == Some(&id) && a.pop_id == pop)
            else {
                valid = false;
                break;
            };
            let expected = transfer["expected_workers_count"]
                .as_f64()
                .ok_or_else(|| error("Expected staff count missing"))?;
            let count = transfer["workers_count"]
                .as_f64()
                .filter(|n| n.is_finite() && *n >= 0. && n.fract() == 0.)
                .ok_or_else(|| error("Invalid staff transfer quantity"))?;
            if a.workers_count != expected || a.workers_count < count {
                valid = false;
                break;
            }
            a.workers_count -= count;
            if a.fte.is_some() {
                a.fte = Some(a.workers_count * a.time_fraction);
            }
            if let Some(e) = employment.iter_mut().find(|e| e.employment_id == id) {
                e.extra
                    .insert("workers_count".into(), json!(a.workers_count));
                e.fte = a.workers_count * a.time_fraction;
                let coefficient = e
                    .extra
                    .get("species_labor_coefficient")
                    .and_then(Value::as_f64)
                    .unwrap_or(1.);
                e.extra
                    .insert("effective_labor".into(), json!(e.fte * coefficient));
            }
        }
        if !valid {
            blocked.push(
                json!({"project_id":p.id,"reason":"construction_staff_transfer_unavailable"}),
            );
            continue;
        }
        let mut rows: Vec<LaborAssignment> = raw
            .iter()
            .flat_map(|e| e.base.assignments.clone())
            .chain(
                w.factories()
                    .iter()
                    .filter(|f| f.status.as_deref() == Some("operating"))
                    .flat_map(|e| e.base.assignments.clone()),
            )
            .chain(
                w.artisans()
                    .iter()
                    .flat_map(|e| e.base.assignments.clone()),
            )
            .collect();
        for (other_index, project) in w.construction_projects().iter().enumerate() {
            if !crate::labor_roster::INACTIVE_PROJECTS.contains(&project.status.as_str()) {
                rows.extend(if other_index == index {
                    p.assignments.clone()
                } else {
                    project.assignments.clone()
                });
            }
        }
        let typed_ids: HashSet<_> = rows
            .iter()
            .filter_map(|a| a.employment_id.clone())
            .collect();
        for e in &employment {
            if typed_ids.contains(&e.employment_id)
                || e.extra.get("direct_output_labor") == Some(&json!(true))
            {
                continue;
            }
            let workers = e
                .extra
                .get("workers_count")
                .and_then(Value::as_f64)
                .unwrap_or(e.fte.ceil());
            let time = e
                .extra
                .get("time_fraction")
                .and_then(Value::as_f64)
                .unwrap_or(if workers > 0. { e.fte / workers } else { 0. });
            rows.push(serde_json::from_value(json!({"pop_id":e.pop_id,"employment_id":e.employment_id,"contract_id":e.contract_id,"workers_count":workers,"time_fraction":time,"fte":e.fte}))?);
        }
        if let Err(err) =
            crate::production_activities::validate_production_assignments(&w.pops(), &rows)
        {
            blocked.push(json!({"project_id":p.id,"reason":"construction_staff_plan_unavailable","detail":err.to_string()}));
            continue;
        }
        p.extra.insert("staff_activated".into(), json!(true));
        let op: EconomicOperation = serde_json::from_value(
            json!({"event_id":format!("project-staff:{}:{}",r.period,crate::needs::js_string(&p.id.json())),"type":"construction_staff_transfer","project_id":p.id,"transfers":transfers}),
        )?;
        let mut session = OperationSession::new(w)?;
        stage_construction_operations(&mut session, std::slice::from_ref(&op))?;
        let changes = session.finish()?;
        w.apply_phase(changes)?;
        w.replace_resource_operations(raw)?;
        w.replace_employment(employment)?;
        let key=w.construction_projects().keyed().nth(index).ok_or_else(||error("Missing selected construction_projects row"))?.0;
        w.put_construction_projects(key,p)?;
        operations.push(op);
    }
    Ok((operations, blocked))
}
