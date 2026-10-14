//! Funded profession transitions, whole-child previews and one atomic phase publication.
use kingmaker_core::world_read::WorldRead;
use crate::{
    budget_funding::error,
    cohort_scope::CohortScope,
    labor_offers::build_job_offers,
    labor_roster::{build_labor_roster, reconcile_labor_in},
    local_hiring::{apply_private_transfers, plan_local_hiring_scoped},
    operation_session::*,
    profession_scope::{FinanceIndex, ProfessionIndex},
};
use kingmaker_core::{labor::LaborRequest, mobility::*, operations::OperationKind, *};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Reads { hashes: Option<Vec<[u8;32]>> }
impl Reads {
    pub(crate) fn capture(w:&dyn WorldRead)->Self {
        let hashes=(|| -> OpResult<Vec<[u8;32]>> { Ok(vec![
            crate::read_guard::fingerprint(&w.provinces())?,
            crate::read_guard::fingerprint(&w.pops())?,
            crate::read_guard::fingerprint(&w.enterprises())?,
            crate::read_guard::fingerprint(&w.stockpiles())?,
            crate::read_guard::fingerprint(&w.accounts())?,
            crate::read_guard::fingerprint(&w.resource_operations())?,
            crate::read_guard::fingerprint(&w.factories())?,
            crate::read_guard::fingerprint(&w.artisans())?,
            crate::read_guard::fingerprint(&w.contracts())?,
            crate::read_guard::fingerprint(&w.expansions())?,
            crate::read_guard::fingerprint(&w.construction_projects())?,
            crate::read_guard::fingerprint(&w.completed_construction())?,
            crate::read_guard::fingerprint(&w.storage_facilities())?,
            crate::read_guard::fingerprint(&w.storage_reservations())?,
            crate::read_guard::fingerprint(&w.goods_reservations())?,
            crate::read_guard::fingerprint(&w.money_reservations())?,
            crate::read_guard::fingerprint(&w.obligations())?,
            crate::read_guard::fingerprint(&w.all_committed_operations())?,
            crate::read_guard::fingerprint(&w.territory())?,
            crate::read_guard::fingerprint(&w.maintenance_agreements())?,
            crate::read_guard::fingerprint(&w.sale_offers())?,
            crate::read_guard::fingerprint(&w.employment())?,
            crate::read_guard::fingerprint(&w.adventurer_income_rates())?,
            crate::read_guard::fingerprint(&w.stock_contributions())?,
            crate::read_guard::fingerprint(&w.institutions())?,
            crate::read_guard::fingerprint(&w.market_construction_catalog())?,
            crate::read_guard::fingerprint(&w.jobs())?,
            crate::read_guard::fingerprint(&w.profiles())?,
            crate::read_guard::fingerprint(&w.species())?,
            crate::read_guard::fingerprint(&w.capitalist_entities())?,
            crate::read_guard::fingerprint(&w.project_investments())?,
            crate::read_guard::fingerprint(&w.production_beneficiaries())?,
            crate::read_guard::fingerprint(&w.factory_beneficiaries())?,
            crate::read_guard::fingerprint(&w.shipments())?,
            crate::read_guard::fingerprint(&w.transport_assets())?,
            crate::read_guard::fingerprint(&w.transport_terminals())?,
            crate::read_guard::fingerprint(&w.supply_routes())?,
            crate::read_guard::fingerprint(&w.route_reservations())?,
            crate::read_guard::fingerprint(&w.navigation_geography())?,
            crate::read_guard::fingerprint(&w.shipment_recovery_plans())?,
            crate::read_guard::fingerprint(&w.producer_income())?,
            crate::read_guard::fingerprint(&w.all_production_phase_receipts())?,
            crate::read_guard::fingerprint(&w.all_market_phase_receipts())?,
            crate::read_guard::fingerprint(&w.all_budget_phase_receipts())?,
            crate::read_guard::fingerprint(&w.all_budget_opening_receipts())?,
            crate::read_guard::fingerprint(&w.all_labor_phase_receipts())?,
            crate::read_guard::fingerprint(&w.all_transport_phase_receipts())?,
            crate::read_guard::fingerprint(&w.all_income_phase_receipts())?,
            crate::read_guard::fingerprint(&w.all_mobility_history_receipts())?,
            crate::read_guard::fingerprint(&w.all_mobility_opening_receipts())?,
            crate::read_guard::fingerprint(&w.all_mobility_migration_receipts())?,
            crate::read_guard::fingerprint(&w.all_mobility_profession_receipts())?,
            crate::read_guard::fingerprint(&w.all_mobility_cohort_receipts())?,
            crate::read_guard::fingerprint(&w.campaign())?,
            crate::read_guard::fingerprint(&w.production_policies())?,
            crate::read_guard::fingerprint(&w.production_catalog())?,
            crate::read_guard::fingerprint(&w.needs())?,
            crate::read_guard::fingerprint(&w.price_calibration())?,
            crate::read_guard::fingerprint(&w.market_parameters())?,
            crate::read_guard::fingerprint(&w.consumer_facility_by_province())?,
            crate::read_guard::fingerprint(&w.budget_profile_id())?,
            crate::read_guard::fingerprint(&w.foundation_budget_policy())?,
            crate::read_guard::fingerprint(&w.institution_rules())?,
            crate::read_guard::fingerprint(&w.pop_mobility())?,
            crate::read_guard::fingerprint(&w.mobility_reference())?,
            crate::read_guard::fingerprint(&w.automatic_land_paths())?,
        ]) })().ok();
        Self {hashes}
    }
    pub(crate) fn unchanged(&self,w:&dyn WorldRead)->bool { self.hashes.is_some() && self.hashes==Self::capture(w).hashes }
}
pub struct PreparedProfessionChanges {
    reads: Option<Reads>,
    state: Option<crate::world_transaction::PreparedWorldDelta>,
    receipt: ProfessionPhaseReceipt,
}
fn replay(w: &dyn WorldRead, r: &LaborRequest) -> OpResult<Option<ProfessionPhaseReceipt>> {
    let rows: Vec<_> = w.all_mobility_profession_receipts().select(|p| p.request.period == r.period)?;
    if rows.is_empty() {
        return Ok(None);
    }
    if rows.len() != 1 || rows[0].request != *r || rows[0].campaign_id != w.campaign().campaign_id {
        return Err(error("Conflicting profession phase request"));
    }
    let history = crate::operation_history::CheckedHistory::new(w)?;
    for op in &rows[0].operations {
        if !history.matches(op) {
            return Err(error("Missing/conflicting profession history"));
        }
    }
    crate::cohort_cycle::validate_cohort_receipts(w, &rows[0].operations, r.expected_revision)?;
    Ok(Some(rows[0].clone()))
}
fn order(w: &dyn WorldRead, r: &LaborRequest) -> OpResult<()> {
    if w.budget_phase_receipts().values()
        .any(|p| p.request.period == r.period)
        || w.market_phase_receipts().values()
            .any(|p| p.request.period == r.period)
        || w.production_phase_receipts().values()
            .any(|p| p.request.period == r.period)
    {
        return Err(error(
            "Profession opening must precede budget, market and production",
        ));
    }
    Ok(())
}
pub fn prepare_profession_changes(
    w: &dyn WorldRead,
    r: &LaborRequest,
) -> OpResult<PreparedProfessionChanges> {
    if r.period.is_empty() || r.expected_revision != w.campaign().revision {
        return Err(error("Invalid profession period/revision"));
    }
    if let Some(receipt) = replay(w, r)? {
        return Ok(PreparedProfessionChanges {
            reads: None,
            state: None,
            receipt,
        });
    }
    order(w, r)?;
    let old = w.pop_mobility()
        .clone()
        .unwrap_or_else(|| json!({}));
    if !old.is_object() {
        return Err(error("Invalid profession mobility registry"));
    }
    if old["profession_period"].as_str() == Some(&r.period) {
        return Err(error("Orphan profession period without receipt"));
    }
    for key in ["unemployment", "remainders", "changed_period"] {
        if old
            .get(key)
            .is_some_and(|v| !v.is_object() && !v.is_null() && !crate::needs::undefined(v))
        {
            return Err(error("Invalid profession mobility map"));
        }
    }
    let roster = build_labor_roster(w)?;
    let offers = build_job_offers(w, &roster)?.offers;
    let mut by_province = HashMap::<_, Vec<_>>::new();
    for offer in offers {
        by_province
            .entry(offer.province_id)
            .or_default()
            .push(offer)
    }
    let mut requirements = HashMap::new();
    if let Some(rows) = w.mobility_reference()
        .as_ref()
        .and_then(|v| v.get("professions"))
        .and_then(Value::as_array)
    {
        for p in rows {
            if let (Some(id), Some(rule)) = (p.get("pop_type_id"), p.get("transition_requirements"))
            {
                requirements.insert(
                    ExternalId::from_json(id.clone()).map_err(OperationError)?,
                    rule,
                );
            }
        }
    }
    let mut state=crate::world_transaction::WorldTransaction::new(w);
    let mut cohorts = CohortScope::new(&state)?;
    let mut scope = ProfessionIndex::new(&state, &r.policies, &roster)?;
    let mut finance = FinanceIndex::new(&state);
    state.set_pop_mobility(Some(old.clone()))?;
    let mobility = state.edit_pop_mobility().as_mut().unwrap();
    mobility["profession_period"] = json!(r.period);
    for key in ["remainders", "changed_period"] {
        if !mobility[key].is_object() {
            mobility[key] = json!({})
        }
    }
    let mut receipt = ProfessionPhaseReceipt {
        campaign_id: w.campaign().campaign_id.clone(),
        request: r.clone(),
        changed_workforce: 0,
        changes: vec![],
        blocked: vec![],
        operations: vec![],
    };
    for p in w.pops().values() {
        crate::run_control::work("profession_people")?;
        let key = external_text(&p.external_id);
        if matches!(
            p.pop_type_id.as_str(),
            Some("aristocrats" | "capitalists" | "adventurers" | "slaves")
        ) || p.extra.get("mobility_status").and_then(Value::as_str) == Some("in_transit")
            || old["changed_period"].get(&key).and_then(Value::as_str) == Some(&r.period)
        {
            continue;
        }
        let free = roster
            .by_pop
            .get(&p.external_id)
            .map(|a| a.free.floor())
            .unwrap_or(0.);
        let mut aged = 0.;
        if let Some(rows) = old["unemployment"].get(&key) {
            let rows = rows
                .as_array()
                .ok_or_else(|| error("Invalid profession unemployment cohorts"))?;
            for c in rows {
                let workers = c["workers"]
                    .as_f64()
                    .filter(|n| n.is_finite() && *n >= 0.)
                    .ok_or_else(|| error("Invalid profession aged workers"))?;
                let weeks = c["weeks"]
                    .as_u64()
                    .ok_or_else(|| error("Invalid profession unemployment age"))?;
                if weeks >= 1 {
                    aged += workers;
                }
            }
        }
        let eligible = free.min(aged).floor() as u64;
        if eligible == 0 {
            continue;
        }
        let previous = old["remainders"]
            .get(&key)
            .and_then(|v| v.get("profession"))
            .filter(|v| crate::needs::js_truthy(v))
            .map(|v| {
                v.as_f64()
                    .filter(|n| n.is_finite() && *n >= 0.)
                    .ok_or_else(|| error("Invalid profession remainder"))
            })
            .transpose()?
            .unwrap_or(0.);
        let accrued = eligible as f64 / 10. + previous;
        let whole = (accrued + 1e-9).floor();
        let limit = eligible.min(whole as u64);
        let m = state.edit_pop_mobility().as_mut().unwrap();
        if !m["remainders"].get(&key).is_some_and(Value::is_object) {
            m["remainders"][&key] = json!({})
        }
        m["remainders"][&key]["profession"] = json!((accrued - whole).max(0.));
        if limit == 0 {
            continue;
        }
        for offer in by_province
            .get(&p.province_id)
            .into_iter()
            .flatten()
            .filter(|o| !o.staffing.contains(&p.pop_type_id))
        {
            let Some(target) = offer.staffing.iter().find(|id| {
                requirements.get(*id).is_some_and(|rule| {
                    rule["kind"].as_str() == Some("funded_vacancy")
                        && rule.get("source").is_some_and(crate::needs::js_truthy)
                })
            }) else {
                receipt.blocked.push(json!({"pop_id":p.external_id,"entity_id":offer.id,"reason":if offer.staffing.iter().any(|id|requirements.contains_key(id)){"profession_requirements_not_met"}else{"profession_requirements_unknown"}}));
                continue;
            };
            let mut count = limit.min(offer.places);
            let mut accepted = false;
            while count > 0 {
                let request = CohortSplitRequest {
                    expected_revision: r.expected_revision,
                    event_id: format!("profession:{}", json!([r.period, p.external_id, offer.id])),
                    pop_id: p.external_id.clone(),
                    workforce_count: count,
                    target: SplitTarget {
                        pop_type_id: Some(target.clone()),
                        ..Default::default()
                    },
                };
                let (split, split_receipt) = cohorts.preview(&state, &request)?;
                if let Some(reason) = split_receipt.blocked_reason {
                    receipt
                        .blocked
                        .push(json!({"pop_id":p.external_id,"reason":reason}));
                    break;
                }
                let child = split_receipt.child_id.as_ref().unwrap();
                let (mut selected, hiring_scope) =
                    scope.select(&state, offer, &split, child, &r.policies)?;
                let hire =
                    plan_local_hiring_scoped(&selected, &r.period, &r.policies, &hiring_scope)?;
                let hired = hire.hired_fte();
                if hired != count as f64 {
                    count = hired.floor() as u64;
                    if count == 0 {
                        receipt.blocked.push(json!({"pop_id":p.external_id,"reason":"destination_not_funded_or_qualified"}));
                    }
                    continue;
                }
                let hire_ops = hire.operations;
                let hires = hire.hires;
                hire.rows.install(&mut selected);
                apply_private_transfers(&mut selected, &hire_ops)?;
                cohorts.commit_registry(&mut state, &p.external_id, &split)?;
                let child_position = state.pops().len() - 1;
                let child_key=state.pops().keyed().nth(child_position).ok_or_else(||error("Missing child POP"))?.0;
                state.edit_pop(&child_key)?.id=PopId(finance.allocate()?);
                finance.commit(&mut state, &selected)?;
                scope.commit_hiring(&mut state, &selected, child, &r.policies, &hires)?;
                receipt.operations.extend(split_receipt.operations);
                receipt.operations.extend(hire_ops);
                receipt.changed_workforce = receipt
                    .changed_workforce
                    .checked_add(count)
                    .ok_or_else(|| error("Profession changed workforce overflow"))?;
                receipt.changes.push(json!({"source_pop_id":p.external_id,"pop_id":child,"type":target,"workers":count}));
                let mobility = state.edit_pop_mobility().as_mut().unwrap();
                mobility["changed_period"][&key] = json!(r.period);
                mobility["changed_period"][external_text(child)] = json!(r.period);
                mobility["unemployment"][external_text(child)] = json!([]);
                accepted = true;
                break;
            }
            if accepted {
                break;
            }
        }
    }
    let mut money = None;
    if receipt.changed_workforce > 0 {
        let final_roster = build_labor_roster(&state)?;
        reconcile_labor_in(&mut state, &final_roster)?;
        let before_people: u64 = w.pops().values().map(|p| u64::from(p.size)).sum();
        let after_people: u64 = state.pops().iter().map(|p| u64::from(p.size)).sum();
        if before_people != after_people
            || w.pops().values().map(|p| p.workforce).sum::<f64>()
                != state.pops().iter().map(|p| p.workforce).sum::<f64>()
        {
            return Err(error("Profession people/workforce conservation failed"));
        }
        let original: HashSet<_> = w.stockpiles().values().map(|s| &s.external_id).collect();
        let templates: Vec<_> = state.stockpiles()
            .iter()
            .filter(|s| !original.contains(&s.external_id))
            .map(|s| (s.external_id.clone(), s.clone()))
            .collect();
        let mut session = OperationSession::new(w)?;
        for op in &receipt.operations {
            match op.kind {
                OperationKind::PopCohortSplit | OperationKind::PopCohortClaim => {
                    session.stage_cohort(op, &templates)?;
                }
                OperationKind::LaborHiringFunding | OperationKind::StateBudgetFunding => {
                    session.stage_labor(op)?;
                }
                _ => return Err(error("Unexpected profession operation kind")),
            }
        }
        let mut next = session.next_ordinal()?;
        let original: HashSet<_> = w.pops().values().map(|p| &p.external_id).collect();
        for key in state.pops().keyed().map(|(key,_)|key).collect::<Vec<_>>() {
            let p=state.edit_pop(&key)?;
            if !original.contains(&p.external_id) {
                p.id = PopId(next);
                next = next
                    .checked_add(1)
                    .ok_or_else(|| error("Profession POP ordinal overflow"))?;
            }
            p.account_id = p.currency_id.as_ref().and_then(|c| {
                session
                    .account(&AccountAddress {
                        owner_id: p.external_id.clone(),
                        currency_id: c.clone(),
                        purpose: "personal_savings".into(),
                    })
                    .map(|a| a.id)
            });
            if p.account_id.is_some() {
                p.account_status = "resolved".into();
            }
        }
        state.replace_enterprises((&state as &dyn WorldRead).derive_enterprise_views(
                session.accounts(),
                state.factories().iter(),
                u64::from(next),
            ).map_err(OperationError)?)?;
        money = Some(session.finish()?);
        let children: HashMap<ExternalId, ExternalId> = receipt
            .changes
            .iter()
            .map(|change| {
                Ok((
                    ExternalId::from_json(change["source_pop_id"].clone())
                        .map_err(OperationError)?,
                    ExternalId::from_json(change["pop_id"].clone()).map_err(OperationError)?,
                ))
            })
            .collect::<OpResult<_>>()?;
        crate::world_transaction::regroup_population(&mut state,w,Some(&children))?;
    }
    let key=state.mobility_profession_receipts().next_key().map_err(OperationError)?;
    state.put_mobility_profession_receipts(key,receipt.clone())?;
    let mut published=crate::world_transaction::WorldTransaction::new(w);
    let has_money=money.is_some();
    if let Some(money)=money {published.apply_phase(money)?;}
    published.adopt_projection(w,state.finish()?,if has_money {&["pops","provinces","enterprises","contracts","resource_operations","factories","artisans","storage_facilities","storage_reservations","goods_reservations","jobs","employment","needs","sale_offers","maintenance_agreements","pop_mobility","capitalist_entities","project_investments","production_beneficiaries","factory_beneficiaries","shipments","mobility_history_receipts","mobility_opening_receipts","mobility_migration_receipts","mobility_profession_receipts","mobility_cohort_receipts","mobility_reference","shipment_recovery_plans"]} else {&["pop_mobility","shipment_recovery_plans","capitalist_entities","project_investments","production_beneficiaries","factory_beneficiaries","shipments","mobility_history_receipts","mobility_opening_receipts","mobility_migration_receipts","mobility_profession_receipts","mobility_cohort_receipts","mobility_reference"]})?;
    Ok(PreparedProfessionChanges {
        reads: Some(Reads::capture(w)),
        state: Some(published.finish()?),
        receipt,
    })
}
pub fn apply_profession_changes(
    w: &mut WorldState,
    p: PreparedProfessionChanges,
) -> OpResult<ProfessionPhaseReceipt> {
    crate::world_transaction::execute_atomic(w,|w|apply_profession_changes_in(w,p))
}
pub(crate) fn apply_profession_changes_in(
    w: &mut crate::world_transaction::WorldTransaction<'_>,
    p: PreparedProfessionChanges,
) -> OpResult<ProfessionPhaseReceipt> {
    if let Some(receipt) = replay(w, &p.receipt.request)? {
        return Ok(receipt);
    }
    order(w, &p.receipt.request)?;
    if p.reads.as_ref().is_none_or(|r| !r.unchanged(w)) {
        return Err(error("Prepared profession consulted rows changed"));
    }
    w.apply_delta(p.state.ok_or_else(||error("Prepared mobility state missing"))?)?;
    Ok(p.receipt)
}
pub fn execute_profession_changes(
    w: &mut WorldState,
    r: &LaborRequest,
) -> OpResult<ProfessionPhaseReceipt> {
    crate::world_transaction::execute_atomic(w,|w|execute_profession_changes_in(w,r))
}
pub(crate) fn execute_profession_changes_in(
    w: &mut crate::world_transaction::WorldTransaction<'_>,
    r: &LaborRequest,
) -> OpResult<ProfessionPhaseReceipt> {
    let p = prepare_profession_changes(w, r)?;
    apply_profession_changes_in(w, p)
}
