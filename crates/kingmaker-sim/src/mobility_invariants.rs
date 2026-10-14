//! Operation evidence for changing cohorts; equal global totals alone are insufficient.
use kingmaker_core::world_read::WorldRead;
use crate::{
    cohort_assets::plan_assets, labor_roster::build_labor_roster,
    operation_session::canonical_operation,
};
use kingmaker_core::{audit::*, mobility::*, operations::*, *};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
fn status(ok: bool, reason: &str) -> CheckStatus {
    if ok {
        CheckStatus::Passed
    } else {
        CheckStatus::Failed {
            reason: reason.into(),
        }
    }
}
fn close(a: f64, b: f64) -> bool {
    a.is_finite() && b.is_finite() && (a - b).abs() <= 1e-9 * a.abs().max(b.abs()).max(1.)
}
pub fn check_mobility_operations(
    before: &dyn WorldRead,
    after: &dyn WorldRead,
    r: &MobilityOpeningReceipt,
) -> InvariantReport {
    let mut money_ok = true;
    let mut goods_ok = true;
    let mut rights_ok = true;
    let mut people_ok = true;
    let history: HashMap<_, _> = after.committed_operations().values()
        .map(|e| (&e.event_id, &e.value))
        .collect();
    rights_ok &= history.len() == after.committed_operations().len();
    let mut events = HashSet::new();
    let mut cash: HashMap<_, _> = before.accounts().values()
        .map(|a| (a.address(), i128::from(a.balance.0)))
        .collect();
    money_ok &= cash.len() == before.accounts().len();
    let mut claims: HashMap<_, _> = before.obligations().values()
        .map(|d| (d.obligation_id.clone(), d.clone()))
        .collect();
    rights_ok &= claims.len() == before.obligations().len();
    let mut stocks: HashMap<_, _> = before.stockpiles().values()
        .map(|s| (s.external_id.clone(), s.clone()))
        .collect();
    goods_ok &= stocks.len() == before.stockpiles().len();
    for op in &r.operations {
        if let Some(d) = &op.obligation {
            rights_ok &= op.kind == OperationKind::PopCohortClaim && d.amount_minor.0 >= 0;
            claims.insert(d.obligation_id.clone(), d.clone());
        }
        rights_ok &= events.insert(&op.event_id)
            && canonical_operation(op).is_ok_and(|v| history.get(&op.event_id) == Some(&&v));
        money_ok &= matches!(
            op.kind,
            OperationKind::LaborHiringFunding
                | OperationKind::StateBudgetFunding
                | OperationKind::PopCohortSplit
                | OperationKind::PopCohortClaim
                | OperationKind::PopMigrationArrival
        );
        let mut currencies = HashMap::<_, i128>::new();
        for d in &op.debits {
            money_ok &= d.amount_minor.0 >= 0;
            *cash.entry(d.address()).or_default() -= i128::from(d.amount_minor.0);
            *currencies.entry(d.currency_id.clone()).or_default() -= i128::from(d.amount_minor.0);
        }
        for c in &op.credits {
            money_ok &= c.amount_minor.0 >= 0;
            *cash.entry(c.address()).or_default() += i128::from(c.amount_minor.0);
            *currencies.entry(c.currency_id.clone()).or_default() += i128::from(c.amount_minor.0);
        }
        money_ok &= currencies.values().all(|n| *n == 0);
        if !op.stock_changes.is_empty() {
            goods_ok &= op.kind == OperationKind::PopCohortSplit;
            let child = op
                .meta
                .get("child_pop_id")
                .cloned()
                .and_then(|v| ExternalId::from_json(v).ok());
            let parent = op
                .meta
                .get("pop_id")
                .cloned()
                .and_then(|v| ExternalId::from_json(v).ok());
            for pair in op.stock_changes.chunks(2) {
                if pair.len() != 2 {
                    goods_ok = false;
                    continue;
                }
                let (old, new) = (&pair[0], &pair[1]);
                let Some(source) = stocks.get(&old.stock_id).cloned() else {
                    goods_ok = false;
                    continue;
                };
                let Some(child) = &child else {
                    rights_ok = false;
                    continue;
                };
                let expected: ExternalId = format!(
                    "{}:{}",
                    crate::operation_session::external_text(&old.stock_id),
                    crate::operation_session::external_text(child)
                )
                .into();
                goods_ok &= old.delta <= 0.
                    && new.delta >= 0.
                    && close(-old.delta, new.delta)
                    && old.create.is_none()
                    && new.create.is_some()
                    && new.stock_id == expected
                    && !stocks.contains_key(&new.stock_id)
                    && source.quantity + old.delta >= -1e-9;
                rights_ok &= parent.as_ref() == Some(&source.owner_id);
                let mut destination = source.clone();
                destination.external_id = new.stock_id.clone();
                destination.owner_id = child.clone();
                destination.quantity = new.delta;
                if destination.beneficiary_pop_id == parent {
                    destination.beneficiary_pop_id = Some(child.clone())
                }
                let mut source = source;
                source.quantity = (source.quantity + old.delta).max(0.);
                stocks.insert(old.stock_id.clone(), source);
                stocks.insert(new.stock_id.clone(), destination);
            }
        }
    }
    let actual: HashMap<_, _> = after.accounts().values()
        .map(|a| (a.address(), i128::from(a.balance.0)))
        .collect();
    money_ok &=
        actual.len() == after.accounts().len() && cash == actual && actual.values().all(|n| *n >= 0);
    let original: HashMap<_, _> = before.accounts().values().map(|a| (a.address(), a)).collect();
    for a in after.accounts().values() {
        if let Some(old) = original.get(&a.address()) {
            rights_ok &= old.id == a.id && old.external_id == a.external_id;
        }
    }
    let actual: HashMap<_, _> = after.stockpiles().values()
        .map(|s| (&s.external_id, s))
        .collect();
    goods_ok &= actual.len() == after.stockpiles().len() && actual.len() == stocks.len();
    for (id, expected) in &stocks {
        if let Some(s) = actual.get(id) {
            goods_ok &= close(s.quantity, expected.quantity) && s.quantity >= 0.;
            let mut expected = expected.clone();
            expected.id = s.id;
            expected.quantity = s.quantity;
            rights_ok &= expected == **s;
        } else {
            goods_ok = false;
        }
    }
    let known: HashSet<_> = before.mobility_cohort_receipts().values()
        .map(|c| &c.request.event_id)
        .collect();
    // The archive cannot change during a mobility transaction; inspect its new cohort delta.
    rights_ok &= before.history().as_ref().map(|h|h.token())==after.history().as_ref().map(|h|h.token());
    let fresh: Vec<_> = after.mobility_cohort_receipts().values().filter(|c| !known.contains(&c.request.event_id)).collect();
    let cohort_ops: HashSet<_> = r
        .operations
        .iter()
        .filter(|op| op.kind == OperationKind::PopCohortSplit)
        .map(|op| &op.event_id)
        .collect();
    people_ok &= fresh.len() == cohort_ops.len();
    let mut people: HashMap<_, _> = before.pops().values()
        .map(|p| (p.external_id.clone(), p.clone()))
        .collect();
    people_ok &= people.len() == before.pops().len();
    let mut titles=crate::world_transaction::WorldTransaction::new(before);
    for c in fresh {
        let event: ExternalId = c.request.event_id.clone().into();
        people_ok &= cohort_ops.contains(&event)
            && c.blocked_reason.is_none()
            && c.campaign_id == before.campaign().campaign_id;
        let Some(child) = &c.child_id else {
            people_ok = false;
            continue;
        };
        let Some(mut parent) = people.get(&c.request.pop_id).cloned() else {
            people_ok = false;
            continue;
        };
        if parent.workforce <= 0.
            || parent.workforce.fract() != 0.
            || c.workforce_moved == 0
            || c.workforce_moved as f64 > parent.workforce
            || people.contains_key(child)
        {
            people_ok = false;
            continue;
        }
        let moved = (u128::from(parent.size) * u128::from(c.workforce_moved)
            / (parent.workforce as u128)) as u32;
        people_ok &= moved == c.people_moved && c.request.workforce_count == c.workforce_moved;
        let mut next = parent.clone();
        next.external_id = child.clone();
        next.size = moved;
        next.workforce = c.workforce_moved as f64;
        next.extra
            .insert("parent_pop_id".into(), parent.external_id.json());
        if let Some(id) = &c.request.target.pop_type_id {
            next.pop_type_id = id.clone();
        }
        if let Some(id) = &c.request.target.currency_id {
            next.currency_id = Some(id.clone());
        }
        if let Some(n) = parent
            .extra
            .get("literate_count")
            .filter(|v| !v.is_null() && !crate::needs::undefined(v))
        {
            if let Some(n) = n.as_u64() {
                let share = n * u64::from(moved) / u64::from(parent.size);
                next.extra
                    .insert("literate_count".into(), serde_json::json!(share));
                parent
                    .extra
                    .insert("literate_count".into(), serde_json::json!(n - share));
            } else {
                people_ok = false;
            }
        }
        match plan_assets(&titles, &parent, child, moved == parent.size) {
            Ok(plan) => {
                rights_ok &= plan.blocked.is_none();
                rights_ok &= plan.install_in(&mut titles).is_ok();
            }
            Err(_) => rights_ok = false,
        }
        parent.size -= moved;
        parent.workforce -= c.workforce_moved as f64;
        people.insert(parent.external_id.clone(), parent);
        people.insert(child.clone(), next);
    }
    let actual: HashMap<_, _> = after.pops().values().map(|p| (&p.external_id, p)).collect();
    people_ok &= actual.len() == after.pops().len() && actual.len() == people.len();
    for (id, p) in &people {
        people_ok &= actual.get(id).is_some_and(|a| {
            a.size == p.size
                && a.workforce == p.workforce
                && a.pop_type_id == p.pop_type_id
                && a.currency_id == p.currency_id
                && a.culture_id == p.culture_id
                && a.religion_id == p.religion_id
                && a.extra.get("literate_count") == p.extra.get("literate_count")
        });
    }
    rights_ok &= titles.storage_facilities()
        == after.storage_facilities()
        && titles.storage_reservations()
            == after.storage_reservations()
        && titles.goods_reservations() == after.goods_reservations()
        && titles.sale_offers() == after.sale_offers()
        && titles.maintenance_agreements() == after.maintenance_agreements()
        && titles.shipments() == after.shipments()
        && before.capitalist_entities() == after.capitalist_entities()
        && before.project_investments() == after.project_investments()
        && before.production_beneficiaries()
            == after.production_beneficiaries()
        && before.factory_beneficiaries() == after.factory_beneficiaries();
    let labor_ok = build_labor_roster(after).is_ok_and(|roster| {
        after.pops().values().all(|p| {
            roster.by_pop.get(&p.external_id).is_some_and(|a| {
                close(p.employed, a.occupied)
                    && close(p.unemployed, a.free)
                    && !(p.extra.get("mobility_status").and_then(Value::as_str)
                        == Some("in_transit")
                        && a.occupied > 0.)
            })
        })
    });
    let actual_claims: HashMap<_, _> = after.obligations().values()
        .map(|d| (d.obligation_id.clone(), d.clone()))
        .collect();
    rights_ok &=
        actual_claims.len() == after.obligations().len() && claims == actual_claims;
    rights_ok &= after.validate_links().errors.is_empty();
    InvariantReport {
        money: status(
            money_ok,
            "Mobility money not explained by conserved operations",
        ),
        goods: status(goods_ok, "Mobility stock quantity not explained by splits"),
        people: status(
            people_ok,
            "Mobility people/workforce/literates not explained by cohorts",
        ),
        labor: status(
            labor_ok,
            "Mobility actual labor counters or physical groups invalid",
        ),
        rights: status(
            rights_ok,
            "Mobility history, titles, leases or references invalid",
        ),
    }
}
