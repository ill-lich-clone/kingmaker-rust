//! Closed walking departures and due arrivals; passengers never move stock or work in transit.
use kingmaker_core::world_read::WorldRead;
use crate::{
    budget_funding::error,
    calendar::{add_days, serial_date},
    cohort_scope::CohortScope,
    labor_offers::{build_job_offers, JobOffer},
    labor_roster::{build_labor_roster, reconcile_labor_in},
    local_hiring::{DestinationFunding},
    migration_path::WalkingPaths,
    operation_session::*,
    profession_cycle::Reads,
    profession_scope::FinanceIndex,
};
use kingmaker_core::{
    calendar::GameDate, labor::LaborRequest, mobility::*, operations::*, production::Extra, *,
};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
pub struct PreparedMigration {
    reads: Option<Reads>,
    state: Option<crate::world_transaction::PreparedWorldDelta>,
    receipt: MigrationPhaseReceipt,
}
fn replay(
    w: &dyn WorldRead,
    r: &LaborRequest,
    phase: MigrationPhase,
) -> OpResult<Option<MigrationPhaseReceipt>> {
    let rows: Vec<_> = w.all_mobility_migration_receipts().select(|p| p.request.period == r.period && p.phase == phase)?;
    if rows.is_empty() {
        return Ok(None);
    }
    if rows.len() != 1 || rows[0].request != *r || rows[0].campaign_id != w.campaign().campaign_id {
        return Err(error("Conflicting migration phase request"));
    }
    let history = crate::operation_history::CheckedHistory::new(w)?;
    for op in &rows[0].operations {
        if !history.matches(op) {
            return Err(error("Missing/conflicting migration history"));
        }
    }
    crate::cohort_cycle::validate_cohort_receipts(w, &rows[0].operations, r.expected_revision)?;
    Ok(Some(rows[0].clone()))
}
fn order(w: &dyn WorldRead, r: &LaborRequest, phase: MigrationPhase) -> OpResult<()> {
    if w.budget_phase_receipts().values()
        .any(|p| p.request.period == r.period)
        || w.market_phase_receipts().values()
            .any(|p| p.request.period == r.period)
        || w.production_phase_receipts().values()
            .any(|p| p.request.period == r.period)
    {
        return Err(error(
            "Migration opening must precede budget, market and production",
        ));
    }
    if phase == MigrationPhase::Arrivals
        && (w.labor_phase_receipts().values()
            .any(|p| p.request.period == r.period)
            || w.mobility_profession_receipts().values()
                .any(|p| p.request.period == r.period)
            || w.mobility_migration_receipts().values()
                .any(|p| p.request.period == r.period && p.phase == MigrationPhase::Departures))
    {
        return Err(error(
            "Migration arrivals must precede local work and departures",
        ));
    }
    Ok(())
}
fn start<'a>(
    w: &'a dyn WorldRead,
    r: &LaborRequest,
    phase: MigrationPhase,
) -> OpResult<(crate::world_transaction::WorldTransaction<'a>, MigrationPhaseReceipt)> {
    if r.period.is_empty() || r.expected_revision != w.campaign().revision {
        return Err(error("Invalid migration period/revision"));
    }
    order(w, r, phase)?;
    serial_date(GameDate::from(w.campaign()))?;
    let mut state=crate::world_transaction::WorldTransaction::new(w);
    let old = state.edit_pop_mobility()
        .get_or_insert_with(|| json!({}));
    if !old.is_object() {
        return Err(error("Invalid migration mobility registry"));
    }
    for key in ["unemployment", "remainders", "changed_period"] {
        if old
            .get(key)
            .is_some_and(|v| !v.is_object() && !v.is_null() && !crate::needs::undefined(v))
        {
            return Err(error("Invalid migration mobility map"));
        }
    }
    if old
        .get("journeys")
        .is_some_and(|v| !v.is_array() && !v.is_null() && !crate::needs::undefined(v))
    {
        return Err(error("Invalid migration journeys"));
    }
    for key in ["remainders", "changed_period"] {
        if !old[key].is_object() {
            old[key] = json!({})
        }
    }
    if !old["journeys"].is_array() {
        old["journeys"] = json!([])
    }
    Ok((
        state,
        MigrationPhaseReceipt {
            campaign_id: w.campaign().campaign_id.clone(),
            request: r.clone(),
            phase,
            workforce: 0,
            movements: vec![],
            blocked: vec![],
            operations: vec![],
        },
    ))
}
fn number(v: &Value, label: &str) -> OpResult<u64> {
    v.as_f64()
        .filter(|n| n.is_finite() && *n >= 0. && n.fract() == 0. && *n <= 9_007_199_254_740_991.)
        .map(|n| n as u64)
        .ok_or_else(|| error(label))
}
fn validate_travelers(w: &dyn WorldRead, roster: &crate::labor_roster::LaborRoster) -> OpResult<()> {
    let pops: HashMap<_, _> = w.pops().values().map(|p| (&p.external_id, p)).collect();
    let provinces: HashSet<_> = w.provinces().values().map(|p| p.id).collect();
    let mut ids = HashSet::new();
    let mut travelers = HashSet::new();
    if let Some(rows) = w.pop_mobility()
        .as_ref()
        .and_then(|m| m.get("journeys"))
        .and_then(Value::as_array)
    {
        for j in rows {
            let journey = ExternalId::from_json(j["id"].clone()).map_err(OperationError)?;
            if !ids.insert(journey) {
                return Err(error("Duplicate migration journey ID"));
            }
            if j["status"].as_str() != Some("in_transit") {
                continue;
            }
            let id = ExternalId::from_json(j["pop_id"].clone()).map_err(OperationError)?;
            let p = pops
                .get(&id)
                .ok_or_else(|| error("Active migration traveler reference missing"))?;
            if !travelers.insert(id.clone())
                || p.extra.get("mobility_status").and_then(Value::as_str) != Some("in_transit")
                || roster.by_pop[&id].occupied > 0.
                || number(&j["workforce"], "Invalid traveler workforce")? as f64 != p.workforce
                || p.workforce <= 0.
            {
                return Err(error("Invalid active migration traveler"));
            }
            let from = ProvinceId::from_external(
                &ExternalId::from_json(j["from_province_id"].clone()).map_err(OperationError)?,
            )
            .map_err(OperationError)?;
            let to = ProvinceId::from_external(
                &ExternalId::from_json(j["to_province_id"].clone()).map_err(OperationError)?,
            )
            .map_err(OperationError)?;
            if from != p.province_id || !provinces.contains(&to) {
                return Err(error("Active migration province reference invalid"));
            }
            let departed: GameDate = serde_json::from_value(j["departed_date"].clone())?;
            let arrival: GameDate = serde_json::from_value(j["arrival_date"].clone())?;
            if serial_date(arrival)? < serial_date(departed)? {
                return Err(error("Migration arrival precedes departure"));
            }
        }
    }
    if w.pops().values().any(|p| {
        p.extra.get("mobility_status").and_then(Value::as_str) == Some("in_transit")
            && !travelers.contains(&p.external_id)
    }) {
        return Err(error("In-transit POP has no active migration journey"));
    }
    Ok(())
}
fn finish(
    w: &dyn WorldRead,
    mut state: crate::world_transaction::WorldTransaction<'_>,
    receipt: MigrationPhaseReceipt,
) -> OpResult<PreparedMigration> {
    let mut money = None;
    if !receipt.operations.is_empty() {
        let before: u64 = w.pops().values().map(|p| u64::from(p.size)).sum();
        let after: u64 = state.pops().iter().map(|p| u64::from(p.size)).sum();
        if before != after
            || w.pops().values().map(|p| p.workforce).sum::<f64>()
                != state.pops().iter().map(|p| p.workforce).sum::<f64>()
        {
            return Err(error("Migration people/workforce conservation failed"));
        }
        let roster = build_labor_roster(&state)?;
        reconcile_labor_in(&mut state, &roster)?;
        if state.pops().iter().any(|p| {
            p.extra.get("mobility_status").and_then(Value::as_str) == Some("in_transit")
                && p.employed > 0.
        }) {
            return Err(error("Migrating people have employment"));
        }
        let existing: HashSet<_> = w.stockpiles().values().map(|s| &s.external_id).collect();
        let templates: Vec<_> = state.stockpiles()
            .iter()
            .filter(|s| !existing.contains(&s.external_id))
            .map(|s| (s.external_id.clone(), s.clone()))
            .collect();
        let mut session = OperationSession::new(w)?;
        for op in &receipt.operations {
            if op.kind == OperationKind::PopMigrationArrival {
                session.stage_migration_arrival(op)?;
            } else {
                session.stage_cohort(op, &templates)?;
            }
        }
        let old: HashSet<_> = w.pops().values().map(|p| &p.external_id).collect();
        let mut next = session.next_ordinal()?;
        for key in state.pops().keyed().map(|(key,_)|key).collect::<Vec<_>>() {
            let p=state.edit_pop(&key)?;
            if !old.contains(&p.external_id) {
                p.id = PopId(next);
                next = next
                    .checked_add(1)
                    .ok_or_else(|| error("Migration POP ordinal overflow"))?;
            }
            p.account_id = p.currency_id.as_ref().and_then(|currency| {
                session
                    .account(&AccountAddress {
                        owner_id: p.external_id.clone(),
                        currency_id: currency.clone(),
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
        if receipt.phase == MigrationPhase::Departures {
            let children: HashMap<ExternalId, ExternalId> = receipt
                .movements
                .iter()
                .map(|j| {
                    Ok((
                        ExternalId::from_json(j["source_pop_id"].clone())
                            .map_err(OperationError)?,
                        ExternalId::from_json(j["pop_id"].clone()).map_err(OperationError)?,
                    ))
                })
                .collect::<OpResult<_>>()?;
            crate::world_transaction::regroup_population(&mut state,w,Some(&children))?;
        } else {crate::world_transaction::regroup_population(&mut state,w,None)?;}

    }
    let key=state.mobility_migration_receipts().next_key().map_err(OperationError)?;
    state.put_mobility_migration_receipts(key,receipt.clone())?;
    let mut published=crate::world_transaction::WorldTransaction::new(w);
    let has_money=money.is_some();
    if let Some(money)=money {published.apply_phase(money)?;}
    published.adopt_projection(w,state.finish()?,if has_money {&["pops","provinces","enterprises","contracts","artisans","storage_facilities","storage_reservations","goods_reservations","jobs","needs","territory","sale_offers","maintenance_agreements","pop_mobility","capitalist_entities","project_investments","production_beneficiaries","factory_beneficiaries","shipments","mobility_history_receipts","mobility_opening_receipts","mobility_migration_receipts","mobility_profession_receipts","mobility_cohort_receipts","mobility_reference","shipment_recovery_plans"]} else {&["pop_mobility","shipment_recovery_plans","capitalist_entities","project_investments","production_beneficiaries","factory_beneficiaries","shipments","mobility_history_receipts","mobility_opening_receipts","mobility_migration_receipts","mobility_profession_receipts","mobility_cohort_receipts","mobility_reference"]})?;
    Ok(PreparedMigration {
        reads: Some(Reads::capture(w)),
        state: Some(published.finish()?),
        receipt,
    })
}
pub fn prepare_migration_departures(
    w: &dyn WorldRead,
    r: &LaborRequest,
) -> OpResult<PreparedMigration> {
    if let Some(receipt) = replay(w, r, MigrationPhase::Departures)? {
        return Ok(PreparedMigration {
            reads: None,
            state: None,
            receipt,
        });
    }
    let (mut state, mut receipt) = start(w, r, MigrationPhase::Departures)?;
    let old = w.pop_mobility()
        .clone()
        .unwrap_or_else(|| json!({}));
    if old["migration_period"].as_str() == Some(&r.period) {
        return Err(error("Orphan migration period without receipt"));
    }
    state.edit_pop_mobility().as_mut().unwrap()["migration_period"] = json!(r.period);
    let roster = build_labor_roster(w)?;
    validate_travelers(w, &roster)?;
    let offers = build_job_offers(w, &roster)?.offers;
    let mut by_type = HashMap::<ExternalId, Vec<&JobOffer>>::new();
    let mut vacancies = HashMap::new();
    for offer in &offers {
        vacancies.insert(offer.id.clone(), offer.places);
        for id in &offer.staffing {
            by_type.entry(id.clone()).or_default().push(offer)
        }
    }
    if let Some(journeys) = old["journeys"].as_array() {
        for j in journeys
            .iter()
            .filter(|j| j["status"].as_str() == Some("in_transit"))
        {
            let id =
                ExternalId::from_json(j["destination_offer_id"].clone()).map_err(OperationError)?;
            if let Some(v) = vacancies.get_mut(&id) {
                *v = v.saturating_sub(number(&j["workforce"], "Invalid journey workforce")?);
            }
        }
    }
    let territories: HashMap<_, _> = w.territory().values()
        .map(|t| (t.province_id, t))
        .collect();
    let mut paths = WalkingPaths::new(&w.territory(), &r.policies)?;
    let mut funding = DestinationFunding::new(w, &r.policies, &roster)?;
    let mut cohorts = CohortScope::new(&state)?;
    let mut finance = FinanceIndex::new(&state);
    let mut needs = HashMap::<ProvinceId, (f64, u64)>::new();
    for p in w.pops().values() {
        crate::run_control::work("migration_people")?;
        if let Some(value) = p
            .extra
            .get("life_needs_satisfaction")
            .and_then(Value::as_f64)
            .filter(|n| n.is_finite())
        {
            if p.size > 0 {
                let value_sum = needs.entry(p.province_id).or_default();
                value_sum.0 += value * f64::from(p.size);
                value_sum.1 += u64::from(p.size);
            }
        }
    }
    let satisfaction = |id: ProvinceId| {
        needs
            .get(&id)
            .map(|(n, count)| *n / (*count as f64))
            .unwrap_or(-1.)
    };
    for p in w.pops().values() {
        crate::run_control::work("migration_people")?;
        let key = external_text(&p.external_id);
        if matches!(
            p.pop_type_id.as_str(),
            Some("slaves" | "aristocrats" | "capitalists" | "adventurers")
        ) || p.extra.get("mobility_status").and_then(Value::as_str) == Some("in_transit")
            || old["changed_period"].get(&key).and_then(Value::as_str) == Some(&r.period)
        {
            continue;
        }
        let mut aged = 0.;
        let mut external = 0.;
        if let Some(rows) = old["unemployment"].get(&key) {
            for c in rows
                .as_array()
                .ok_or_else(|| error("Invalid migration unemployment cohorts"))?
            {
                let workers = c["workers"]
                    .as_f64()
                    .filter(|n| n.is_finite() && *n >= 0.)
                    .ok_or_else(|| error("Invalid migration aged workers"))?;
                let weeks = number(&c["weeks"], "Invalid migration age")?;
                if weeks >= 2 {
                    aged += workers
                }
                if weeks >= 3 {
                    external += workers
                }
            }
        }
        let eligible = roster
            .by_pop
            .get(&p.external_id)
            .map(|a| a.free)
            .unwrap_or(0.)
            .min(aged)
            .floor() as u64;
        if eligible == 0 {
            continue;
        }
        let previous = old["remainders"]
            .get(&key)
            .and_then(|v| v.get("migration"))
            .filter(|v| crate::needs::js_truthy(v))
            .map(|v| {
                v.as_f64()
                    .filter(|n| n.is_finite() && *n >= 0.)
                    .ok_or_else(|| error("Invalid migration remainder"))
            })
            .transpose()?
            .unwrap_or(0.);
        let accrued = eligible as f64 / 20. + previous;
        let whole = (accrued + 1e-9).floor();
        let limit = eligible.min(whole as u64);
        let m = state.edit_pop_mobility().as_mut().unwrap();
        if !m["remainders"].get(&key).is_some_and(Value::is_object) {
            m["remainders"][&key] = json!({})
        }
        m["remainders"][&key]["migration"] = json!((accrued - whole).max(0.));
        if limit == 0 {
            continue;
        }
        let mut candidates = vec![];
        for offer in by_type.get(&p.pop_type_id).into_iter().flatten() {
            let vacancy = vacancies[&offer.id];
            if vacancy == 0 || offer.province_id == p.province_id {
                continue;
            }
            let Some(destination) = territories.get(&offer.province_id) else {
                continue;
            };
            let foreign = destination.extra.get("owner_state_id")
                != p.state_id.as_ref().map(ExternalId::json).as_ref();
            if foreign && external.floor() == 0. {
                continue;
            }
            let Some(path) = paths.path(p.province_id, offer.province_id)? else {
                continue;
            };
            let capacity = match funding.capacity(p, offer) {
                Ok(capacity) => capacity,
                Err(e) => {
                    receipt.blocked.push(json!({"pop_id":p.external_id,"entity_id":offer.id,"reason":"destination_capacity_unmeasured","detail":e.0}));
                    continue;
                }
            };
            if capacity > 0 {
                candidates.push((
                    *offer,
                    path,
                    foreign,
                    capacity.min(vacancy).min(if foreign {
                        external.floor() as u64
                    } else {
                        eligible
                    }),
                ));
            }
        }
        candidates.sort_by(|a, b| {
            a.2.cmp(&b.2)
                .then_with(|| {
                    satisfaction(b.0.province_id).total_cmp(&satisfaction(a.0.province_id))
                })
                .then_with(|| a.1.len().cmp(&b.1.len()))
                .then_with(|| {
                    crate::ordering::collator().compare(
                        &a.0.province_id.0.to_string(),
                        &b.0.province_id.0.to_string(),
                    )
                })
        });
        let Some((offer, path, _, available)) = candidates.into_iter().next() else {
            receipt
                .blocked
                .push(json!({"pop_id":p.external_id,"reason":"no_permitted_funded_destination"}));
            continue;
        };
        let count = limit.min(available);
        if count == 0 {
            continue;
        }
        let event = format!("migration:{}", json!([r.period, p.external_id]));
        let request = CohortSplitRequest {
            expected_revision: r.expected_revision,
            event_id: event.clone(),
            pop_id: p.external_id.clone(),
            workforce_count: count,
            target: SplitTarget::default(),
        };
        let (mut split, split_receipt) = cohorts.preview(&state, &request)?;
        if let Some(reason) = split_receipt.blocked_reason {
            receipt
                .blocked
                .push(json!({"pop_id":p.external_id,"reason":reason}));
            continue;
        }
        let child = split_receipt.child_id.as_ref().unwrap();
        split
            .pops
            .iter_mut()
            .find(|p| &p.external_id == child)
            .unwrap()
            .extra
            .insert("mobility_status".into(), json!("in_transit"));
        cohorts.commit_registry(&mut state, &p.external_id, &split)?;
        let child_key=state.pops().keyed().last().ok_or_else(||error("Missing child POP"))?.0;
        state.edit_pop(&child_key)?.id=PopId(finance.allocate()?);
        finance.commit(&mut state, &split)?;
        let date = GameDate::from(w.campaign());
        let days = i64::try_from(path.len() - 1)
            .ok()
            .and_then(|n| n.checked_mul(7))
            .ok_or_else(|| error("Walking days overflow"))?;
        let journey = json!({"id":event,"pop_id":child,"status":"in_transit","from_province_id":p.province_id.0,"to_province_id":offer.province_id.0,"to_state_id":territories[&offer.province_id].extra.get("owner_state_id"),"destination_offer_id":offer.id,"path":path.iter().map(|p|p.0).collect::<Vec<_>>(),"departed_date":date,"arrival_date":add_days(date,days)?,"paid_costs":[],"authorization":"user-approved-walking:2026-10-11","workforce":count});
        let mobility = state.edit_pop_mobility().as_mut().unwrap();
        mobility["changed_period"][&key] = json!(r.period);
        mobility["changed_period"][external_text(child)] = json!(r.period);
        mobility["journeys"]
            .as_array_mut()
            .unwrap()
            .push(journey.clone());
        let mut movement = journey;
        movement["source_pop_id"] = p.external_id.json();
        receipt.movements.push(movement);
        receipt.operations.extend(split_receipt.operations);
        receipt.workforce += count;
        *vacancies.get_mut(&offer.id).unwrap() -= count;
    }
    finish(w, state, receipt)
}
pub fn prepare_migration_arrivals(w: &dyn WorldRead, r: &LaborRequest) -> OpResult<PreparedMigration> {
    if let Some(receipt) = replay(w, r, MigrationPhase::Arrivals)? {
        return Ok(PreparedMigration {
            reads: None,
            state: None,
            receipt,
        });
    }
    let (mut state, mut receipt) = start(w, r, MigrationPhase::Arrivals)?;
    let now = serial_date(GameDate::from(w.campaign()))?;
    let roster = build_labor_roster(w)?;
    validate_travelers(w, &roster)?;
    let pops: HashMap<_, _> = w.pops().values()
        .enumerate()
        .map(|(i, p)| (&p.external_id, i))
        .collect();
    let provinces: HashSet<_> = w.provinces().values().map(|p| p.id).collect();
    let mut delta = HashMap::<ProvinceId, i64>::new();
    let mut arrived = HashMap::<ExternalId, (ProvinceId, ExternalId)>::new();
    let mut moved_pops=vec![];
    let journeys = state.edit_pop_mobility().as_mut().unwrap()["journeys"]
        .as_array_mut()
        .unwrap();
    for j in journeys
        .iter_mut()
        .filter(|j| j["status"].as_str() == Some("in_transit"))
    {
        let due: GameDate = serde_json::from_value(j["arrival_date"].clone())?;
        if serial_date(due)? > now {
            continue;
        }
        let id = ExternalId::from_json(j["pop_id"].clone()).map_err(OperationError)?;
        if arrived.contains_key(&id) {
            return Err(error("Two due journeys for one POP"));
        }
        let &i = pops.get(&id).ok_or_else(|| error("Arrival POP missing"))?;
        let p = w.pops().at(i).expect("validated row position");
        if p.extra.get("mobility_status").and_then(Value::as_str) != Some("in_transit")
            || roster.by_pop[&id].occupied > 0.
        {
            return Err(error("Arrival traveler has work or invalid status"));
        }
        let destination = ProvinceId::from_external(
            &ExternalId::from_json(j["to_province_id"].clone()).map_err(OperationError)?,
        )
        .map_err(OperationError)?;
        if !provinces.contains(&destination) {
            return Err(error("Arrival destination province missing"));
        }
        let country = ExternalId::from_json(j["to_state_id"].clone()).map_err(OperationError)?;
        let workers = number(&j["workforce"], "Invalid arrival workforce")?;
        if workers as f64 != p.workforce {
            return Err(error("Arrival workforce differs from traveler"));
        }
        *delta.entry(p.province_id).or_default() -= i64::from(p.size);
        *delta.entry(destination).or_default() += i64::from(p.size);
        let mut moved=p.clone();
        moved.province_id = destination;
        moved.state_id = Some(country.clone());
        moved.employed = 0.;
        moved.unemployed = moved.workforce;
        moved
            .extra
            .insert("mobility_status".into(), json!("settled"));
        moved_pops.push(moved);
        arrived.insert(id.clone(), (destination, country));
        j["status"] = json!("arrived");
        j["arrived_period"] = json!(r.period);
        receipt.movements.push(j.clone());
        receipt.workforce += workers;
        let event = format!(
            "{}:arrival",
            external_text(&ExternalId::from_json(j["id"].clone()).map_err(OperationError)?)
        );
        receipt.operations.push(EconomicOperation {
            event_id: event.into(),
            kind: OperationKind::PopMigrationArrival,
            debits: vec![],
            credits: vec![],
            stock_changes: vec![],
            obligation: None,
            meta: Extra::from([
                ("pop_id".into(), id.json()),
                ("income_recognized".into(), json!(false)),
            ]),
        });
    }
    for p in moved_pops {state.put_pops(p.external_id.clone(),p)?;}
    if !arrived.is_empty() {
        let mut found = HashSet::new();
        let mut territory_updates=vec![];
        for (key,t) in state.territory().keyed() {
            if let Some(change) = delta.get(&t.province_id) {
                let total = t
                    .extra
                    .get("population_total")
                    .and_then(Value::as_f64)
                    .filter(|n| n.is_finite() && *n >= 0.)
                    .ok_or_else(|| error("Arrival province population unmeasured"))?;
                let new = total + (*change as f64);
                if new < 0. {
                    return Err(error("Negative arrival province population"));
                }
                let mut t=t.clone();t.extra.insert("population_total".into(), json!(new));
                territory_updates.push((key,t.clone()));
                found.insert(t.province_id);
            }
        }
        for (key,t) in territory_updates {state.put_territory(key,t)?;}
        if delta.keys().any(|p| !found.contains(p)) {
            return Err(error("Arrival territory missing"));
        }
        if let Some(needs) = state.edit_needs() {
            for refrow in &mut needs.profile_refs {
                if let Some((province, country)) = arrived.get(&refrow.pop_id) {
                    refrow.extra.insert("province_id".into(), json!(province.0));
                    refrow.extra.insert("state_id".into(), country.json());
                }
            }
        }
        let mobility = state.edit_pop_mobility().as_mut().unwrap();
        for id in arrived.keys() {
            mobility["changed_period"][external_text(id)] = json!(r.period);
        }
    }
    finish(w, state, receipt)
}
fn apply(w: &mut crate::world_transaction::WorldTransaction<'_>, p: PreparedMigration) -> OpResult<MigrationPhaseReceipt> {
    if let Some(receipt) = replay(w, &p.receipt.request, p.receipt.phase)? {
        return Ok(receipt);
    }
    order(w, &p.receipt.request, p.receipt.phase)?;
    if p.reads.as_ref().is_none_or(|r| !r.unchanged(w)) {
        return Err(error("Prepared migration consulted rows changed"));
    }
    w.apply_delta(p.state.ok_or_else(||error("Prepared mobility state missing"))?)?;
    Ok(p.receipt)
}
pub fn apply_migration_departures(
    w: &mut WorldState,
    p: PreparedMigration,
) -> OpResult<MigrationPhaseReceipt> {
    crate::world_transaction::execute_atomic(w,|w|apply_migration_departures_in(w,p))
}
pub(crate) fn apply_migration_departures_in(
    w: &mut crate::world_transaction::WorldTransaction<'_>,
    p: PreparedMigration,
) -> OpResult<MigrationPhaseReceipt> {
    if p.receipt.phase != MigrationPhase::Departures {
        return Err(error("Wrong migration publication phase"));
    }
    apply(w, p)
}
pub fn apply_migration_arrivals(
    w: &mut WorldState,
    p: PreparedMigration,
) -> OpResult<MigrationPhaseReceipt> {
    crate::world_transaction::execute_atomic(w,|w|apply_migration_arrivals_in(w,p))
}
pub(crate) fn apply_migration_arrivals_in(
    w: &mut crate::world_transaction::WorldTransaction<'_>,
    p: PreparedMigration,
) -> OpResult<MigrationPhaseReceipt> {
    if p.receipt.phase != MigrationPhase::Arrivals {
        return Err(error("Wrong migration publication phase"));
    }
    apply(w, p)
}
pub fn execute_migration_departures(
    w: &mut WorldState,
    r: &LaborRequest,
) -> OpResult<MigrationPhaseReceipt> {
    crate::world_transaction::execute_atomic(w,|w|execute_migration_departures_in(w,r))
}
pub(crate) fn execute_migration_departures_in(
    w: &mut crate::world_transaction::WorldTransaction<'_>,
    r: &LaborRequest,
) -> OpResult<MigrationPhaseReceipt> {
    let p = prepare_migration_departures(w, r)?;
    apply_migration_departures_in(w, p)
}
pub fn execute_migration_arrivals(
    w: &mut WorldState,
    r: &LaborRequest,
) -> OpResult<MigrationPhaseReceipt> {
    crate::world_transaction::execute_atomic(w,|w|execute_migration_arrivals_in(w,r))
}
pub(crate) fn execute_migration_arrivals_in(
    w: &mut crate::world_transaction::WorldTransaction<'_>,
    r: &LaborRequest,
) -> OpResult<MigrationPhaseReceipt> {
    let p = prepare_migration_arrivals(w, r)?;
    apply_migration_arrivals_in(w, p)
}
