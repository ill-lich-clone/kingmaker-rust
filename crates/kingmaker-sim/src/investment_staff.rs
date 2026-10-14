//! Funded construction crews are activated atomically from current source assignments.
use kingmaker_core::world_read::WorldRead;
use crate::{
    budget_funding::error,
    construction_operations::stage_construction_operations,
    operation_session::{OpResult, OperationSession},
    transport_validation::actor,
};
use kingmaker_core::{
    construction::ConstructionRequest, operations::EconomicOperation,
};
use serde_json::{json, Value};
use std::collections::HashMap;
#[path="investment_labor_index.rs"] mod labor_index;
pub(crate) fn activate_staff(
    w: &mut crate::world_transaction::WorldTransaction<'_>,
    r: &ConstructionRequest,
) -> OpResult<(Vec<EconomicOperation>, Vec<Value>)> {
    let mut operations = vec![];
    let mut blocked = vec![];
    let projects:Vec<_>=w.construction_projects().keyed().enumerate().filter_map(|(i,(id,p))| (p.status=="funded" && p.extra.get("staff_activated")!=Some(&json!(true)) && p.extra.get("planned_assignments").and_then(Value::as_array).is_some()).then_some((i,id))).collect();
    if projects.is_empty(){return Ok((operations,blocked));}
    let mut labor=labor_index::LaborIndex::new();
    let mut raw_positions=HashMap::new();
    for (i,row) in w.resource_operations().values().enumerate(){crate::run_control::work("investment_staff_index")?;raw_positions.insert(row.base.id.clone(),i);labor.set_source((0,i),row.base.assignments.clone())?;}
    for (i,row) in w.factories().values().enumerate(){crate::run_control::work("investment_staff_index")?;if row.status.as_deref()==Some("operating"){labor.set_source((1,i),row.base.assignments.clone())?;}}
    for (i,row) in w.artisans().values().enumerate(){crate::run_control::work("investment_staff_index")?;labor.set_source((2,i),row.base.assignments.clone())?;}
    for (i,row) in w.construction_projects().values().enumerate(){crate::run_control::work("investment_staff_index")?;if !crate::labor_roster::INACTIVE_PROJECTS.contains(&row.status.as_str()){labor.set_source((3,i),row.assignments.clone())?;}}
    let mut employment_positions=HashMap::new();
    for (i,row) in w.employment().values().enumerate(){crate::run_control::work("investment_staff_index")?;employment_positions.insert(row.employment_id.clone(),i);labor.set_employment(i,row.clone())?;}
    for (index,project_id) in projects {
        crate::run_control::work("investment_staff")?;
        let original=w.construction_projects().get(&project_id).expect("indexed project");
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
        let mut raw=HashMap::<kingmaker_core::ExternalId,kingmaker_core::production::ResourceOperation>::new();
        let mut employment=HashMap::<kingmaker_core::ExternalId,kingmaker_core::budget::Employment>::new();
        let mut valid = true;
        for transfer in &transfers {
            crate::run_control::work("investment_staff_transfer")?;
            let source = actor(&transfer["source_rgo_id"])?;
            let id = actor(&transfer["employment_id"])?;
            let pop = actor(&transfer["pop_id"])?;
            if !raw.contains_key(&source) {
                let Some(row)=w.resource_operations().get(&source) else {valid=false;break;};
                raw.insert(source.clone(),row.clone());
            }
            let source=raw.get_mut(&source).unwrap();
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
            if !employment.contains_key(&id) {if let Some(row)=w.employment().get(&id){employment.insert(id.clone(),row.clone());}}
            if let Some(e) = employment.get_mut(&id) {
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
        let mut undo=vec![];
        for (id,row) in &raw {let key=(0,raw_positions[id]);undo.push((key,labor.set_source(key,row.base.assignments.clone())?));}
        undo.push(((3,index),labor.set_source((3,index),p.assignments.clone())?));
        let mut employment_undo=vec![];
        for (id,e) in &employment {let i=employment_positions[id];let old=labor.set_employment(i,e.clone())?.expect("indexed employment");employment_undo.push((i,old));}
        labor.check_fallback()?;
        if let Err(err)=labor.validate(w) {
            crate::run_control::check("investment_staff_validation")?;
            for (key,rows) in undo.into_iter().rev(){labor.set_source(key,rows)?;}
            for (i,e) in employment_undo{labor.set_employment(i,e)?;}
            blocked.push(json!({"project_id":p.id,"reason":"construction_staff_plan_unavailable","detail":err.to_string()}));
            continue;
        }
        p.extra.insert("staff_activated".into(), json!(true));
        let op: EconomicOperation = serde_json::from_value(
            json!({"event_id":format!("project-staff:{}:{}",r.period,crate::needs::js_string(&p.id.json())),"type":"construction_staff_transfer","project_id":p.id,"transfers":transfers}),
        )?;
        for (id,row) in raw {w.put_resource_operations(id,row)?;}
        for (id,row) in employment {w.put_employment(id,row)?;}
        w.put_construction_projects(project_id,p)?;
        operations.push(op);
    }
    if !operations.is_empty() {
        let mut session=OperationSession::new(w)?;
        stage_construction_operations(&mut session,&operations)?;
        let changes=session.finish()?;w.apply_phase(changes)?;
    }
    Ok((operations, blocked))
}
