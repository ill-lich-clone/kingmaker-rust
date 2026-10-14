//! Production-aware evidence, checked on overlays before any publication.
use kingmaker_core::row_store::ReadSequence;
use kingmaker_core::world_read::WorldRead;
use crate::{operation_session::*, production_activities::*};
use kingmaker_core::*;
use kingmaker_core::{audit::*, operations::*, production::*, storage::*};
use std::collections::{HashMap, HashSet};
fn failed(s: &str) -> CheckStatus {
    CheckStatus::Failed { reason: s.into() }
}
fn checked(value: bool, reason: &str) -> CheckStatus {
    if value {
        CheckStatus::Passed
    } else {
        failed(reason)
    }
}
struct View<'a> {
    accounts: Vec<&'a AccountBalance>,
    stocks: Vec<&'a StockpileRecord>,
    pops: kingmaker_core::row_store::RowView<'a,ExternalId,PopRecord>,
    storage: Vec<&'a StorageReservation>,
    factories: Vec<&'a FactoryEnterprise>,
    debts: Vec<&'a Obligation>,
    reserves: Vec<&'a MoneyReservation>,
    data: kingmaker_core::world_groups::ProductionView<'a>,
}
impl<'a> View<'a> {
    fn world(w: &'a dyn WorldRead) -> Self {
        Self {
            accounts: w.accounts().values().collect(),
            stocks: w.stockpiles().values().collect(),
            pops: w.pops(),
            storage: w.storage_reservations().values().collect(),
            factories: w.factories().values().collect(),
            debts: w.obligations().values().collect(),
            reserves: w.money_reservations().values().collect(),
            data: w.production_data(),
        }
    }
}
pub(crate) fn rows_match<T: PartialEq>(
    before: &[&T],
    after: &[&T],
    changes: &[RecordTransition<T>],
    key: impl Fn(&T) -> ExternalId,
) -> bool {
    let old: HashMap<_, _> = before.iter().map(|r| (key(r), *r)).collect();
    let new: HashMap<_, _> = after.iter().map(|r| (key(r), *r)).collect();
    let transitions: HashMap<_, _> = changes.iter().map(|r| (key(&r.after), r)).collect();
    if old.len() != before.len() || new.len() != after.len() || transitions.len() != changes.len() {
        return false;
    }
    for (id, row) in &old {
        if !transitions.contains_key(id) && new.get(id) != Some(row) {
            return false;
        }
    }
    for (id, row) in &new {
        if let Some(change) = transitions.get(id) {
            if old.get(id).copied() != change.before.as_ref() || *row != &change.after {
                return false;
            }
        } else if old.get(id) != Some(row) {
            return false;
        }
    }
    transitions.keys().all(|id| new.contains_key(id))
}
pub(crate) fn check_session_production_operations(
    s: &OperationSession,
    r: &ProductionPhaseReceipt,
) -> InvariantReport {
    let c = &r.registry_changes;
    let storage_map: HashMap<_, _> = c.storage.iter().map(|v| (&v.after.id, &v.after)).collect();
    let factory_map: HashMap<_, _> = c
        .factories
        .iter()
        .map(|v| (&v.after.base.id, &v.after))
        .collect();
    let debt_map: HashMap<_, _> = c
        .obligations
        .iter()
        .map(|v| (&v.after.obligation_id, &v.after))
        .collect();
    let reserve_key = |v: &MoneyReservation| {
        serde_json::json!([v.owner_id, v.currency_id, v.purpose]).to_string()
    };
    let reserve_map: HashMap<_, _> = c
        .money_reservations
        .iter()
        .map(|v| (reserve_key(&v.after), &v.after))
        .collect();
    let after = View {
        accounts: s.accounts().collect(),
        stocks: s.stocks().collect(),
        pops: s.base.pops(),
        storage: s
            .base.storage_reservations().values()
            .filter(|v| !storage_map.contains_key(&v.id))
            .chain(c.storage.iter().map(|v| &v.after))
            .collect(),
        factories: s
            .base.factories().values()
            .map(|v| factory_map.get(&v.base.id).copied().unwrap_or(v))
            .collect(),
        debts: s
            .base.obligations().values()
            .filter(|v| !debt_map.contains_key(&v.obligation_id))
            .chain(c.obligations.iter().map(|v| &v.after))
            .collect(),
        reserves: s
            .base.money_reservations().values()
            .map(|v| reserve_map.get(&reserve_key(v)).copied().unwrap_or(v))
            .collect(),
        data: s.base.production_data(),
    };
    let all_ops: Result<Vec<EconomicOperation>, _> = r
        .event_ids
        .iter()
        .map(|id| {
            s.committed(id)
                .ok_or_else(|| "Missing phase evidence".to_owned())
                .and_then(|v| serde_json::from_value(v.into_owned()).map_err(|e| e.to_string()))
        })
        .collect();
    let Ok(all_ops) = all_ops else {
        let status = failed("Missing/invalid phase event evidence");
        return InvariantReport {
            money: status.clone(),
            goods: status.clone(),
            people: status.clone(),
            labor: status.clone(),
            rights: status,
        };
    };
    audit(
        s.base,
        View::world(s.base),
        after,
        r,
        s.operations(),
        &all_ops,
    )
}
pub fn check_production_operations(
    before: &dyn WorldRead,
    after: &dyn WorldRead,
    r: &ProductionPhaseReceipt,
) -> InvariantReport {
    let history: HashMap<_, _> = after.committed_operations().values()
        .map(|e| (&e.event_id, &e.value))
        .collect();
    let old: HashSet<_> = before.committed_operations().values()
        .map(|e| &e.event_id)
        .collect();
    let parsed: Result<Vec<EconomicOperation>, _> = r
        .event_ids
        .iter()
        .map(|id| {
            history
                .get(id)
                .ok_or_else(|| "Missing committed phase event".to_owned())
                .and_then(|v| serde_json::from_value((*v).clone()).map_err(|e| e.to_string()))
        })
        .collect();
    match parsed {
        Ok(all) => {
            let fresh: Vec<_> = all
                .iter()
                .filter(|op| !old.contains(&op.event_id))
                .cloned()
                .collect();
            audit(
                before,
                View::world(before),
                View::world(after),
                r,
                &fresh,
                &all,
            )
        }
        Err(e) => {
            let s = failed(&e);
            InvariantReport {
                money: s.clone(),
                goods: s.clone(),
                people: s.clone(),
                labor: s.clone(),
                rights: s,
            }
        }
    }
}
fn audit(
    world: &dyn WorldRead,
    before: View,
    after: View,
    r: &ProductionPhaseReceipt,
    ops: &(impl ReadSequence<EconomicOperation> + ?Sized),
    all_ops: &(impl ReadSequence<EconomicOperation> + ?Sized),
) -> InvariantReport {
    let mut cash_delta: HashMap<AccountAddress, i128> = HashMap::new();
    let mut stock_delta: HashMap<ExternalId, f64> = HashMap::new();
    let mut created = HashMap::new();
    let mut currency = HashMap::new();
    let mut money_ok = true;
    let mut goods_ok = true;
    for op in ops.iter() {
        for (legs, sign) in [(&op.debits, -1i128), (&op.credits, 1i128)] {
            for leg in legs {
                money_ok &= leg.amount_minor.0 >= 0;
                *cash_delta.entry(leg.address()).or_default() +=
                    sign * i128::from(leg.amount_minor.0);
                *currency.entry(leg.currency_id.clone()).or_insert(0i128) +=
                    sign * i128::from(leg.amount_minor.0);
            }
        }
        if op.kind != OperationKind::Production && !op.stock_changes.is_empty() {
            goods_ok = false;
        }
        for delta in &op.stock_changes {
            goods_ok &= delta.delta.is_finite();
            *stock_delta.entry(delta.stock_id.clone()).or_default() += delta.delta;
            if let Some(create) = &delta.create {
                created.insert(&delta.stock_id, create);
            }
        }
    }
    money_ok &= currency.values().all(|v| *v == 0);
    let old_cash: HashMap<_, _> = before.accounts.iter().map(|a| (a.address(), *a)).collect();
    let new_cash: HashMap<_, _> = after.accounts.iter().map(|a| (a.address(), *a)).collect();
    money_ok &= old_cash.len() == before.accounts.len() && new_cash.len() == after.accounts.len();
    for (key, row) in &new_cash {
        let old = old_cash.get(key);
        let expected = old.map(|a| i128::from(a.balance.0)).unwrap_or(0)
            + cash_delta.get(key).copied().unwrap_or(0);
        money_ok &= row.balance.0 >= 0 && i128::from(row.balance.0) == expected;
        if let Some(old) = old {
            let mut normalized = (*row).clone();
            normalized.balance = old.balance;
            money_ok &= &normalized == *old;
        } else {
            money_ok &= cash_delta.contains_key(key)
                && row.external_id
                    == ExternalId::from(
                        serde_json::json!([row.owner_id, row.currency_id, row.purpose]).to_string(),
                    );
        }
    }
    money_ok &= old_cash.keys().all(|k| new_cash.contains_key(k))
        && cash_delta.keys().all(|k| new_cash.contains_key(k));
    let old_stocks: HashMap<_, _> = before.stocks.iter().map(|s| (&s.external_id, *s)).collect();
    let new_stocks: HashMap<_, _> = after.stocks.iter().map(|s| (&s.external_id, *s)).collect();
    goods_ok &= old_stocks.len() == before.stocks.len() && new_stocks.len() == after.stocks.len();
    for (id, row) in &new_stocks {
        let old = old_stocks.get(id);
        let expected =
            old.map(|s| s.quantity).unwrap_or(0.0) + stock_delta.get(*id).copied().unwrap_or(0.0);
        goods_ok &= row.quantity.is_finite()
            && row.quantity >= 0.0
            && (row.quantity - expected).abs() <= 1e-8;
        if let Some(old) = old {
            let mut normalized = (*row).clone();
            normalized.quantity = old.quantity;
            goods_ok &= &normalized == *old;
        } else if let Some(c) = created.get(id) {
            goods_ok &= row.owner_id == c.owner_id
                && row.province_id == c.province_id
                && row.goods.external_id() == c.goods_id
                && row.unit.as_ref() == Some(&c.unit)
                && row.facility_id == c.facility_id
                && row.currency_id == c.currency_id
                && row.inventory_origin.as_deref() == Some(&c.inventory_origin)
                && row.method_kind == c.method_kind;
        } else {
            goods_ok = false;
        }
    }
    goods_ok &= old_stocks.keys().all(|k| new_stocks.contains_key(k))
        && stock_delta.keys().all(|k| new_stocks.contains_key(k));
    let people_ok = before.pops.len() == after.pops.len()
        && before.pops.iter().zip(after.pops).all(|(b, a)| {
            let mut a = a.clone();
            a.account_id = b.account_id;
            a.account_status = b.account_status.clone();
            &a == b
        });
    let storage_ok = rows_match(
        &before.storage,
        &after.storage,
        &r.registry_changes.storage,
        |r| r.id.clone(),
    );
    let factories_ok = rows_match(
        &before.factories,
        &after.factories,
        &r.registry_changes.factories,
        |r| r.base.id.clone(),
    );
    let debt_ok = rows_match(
        &before.debts,
        &after.debts,
        &r.registry_changes.obligations,
        |r| r.obligation_id.clone(),
    );
    let reserve_ok = rows_match(
        &before.reserves,
        &after.reserves,
        &r.registry_changes.money_reservations,
        |r| ExternalId::from(serde_json::json!([r.owner_id, r.currency_id, r.purpose]).to_string()),
    );
    money_ok &= debt_ok && reserve_ok;
    goods_ok &= storage_ok;
    for op in ops.iter() {
        if let Some(d) = &op.obligation {
            money_ok &= after
                .debts
                .iter()
                .find(|r| r.obligation_id == d.obligation_id)
                .is_some_and(|r| *r == d);
        }
    }
    let mut labor_ok = factories_ok;
    let mut assignments: Vec<_> = r
        .outputs
        .iter()
        .flat_map(|o| o.assignments.clone())
        .collect();
    assignments.extend(
        after
            .data
            .construction_projects
            .iter()
            .filter(|p| !crate::labor_roster::INACTIVE_PROJECTS.contains(&p.status.as_str()))
            .flat_map(|p| p.assignments.clone()),
    );
    labor_ok &= validate_production_assignments(&after.pops, &assignments).is_ok();
    let mut operation_index=HashMap::new();
    let mut produced_by_activity=HashMap::<&str,f64>::new();
    for op in all_ops.iter() {
        if operation_index.insert(&op.event_id,op).is_some() {labor_ok=false;money_ok=false;goods_ok=false;}
        if op.kind==OperationKind::Production {
            if let (Some(id),Some(q))=(op.meta.get("activity_id").and_then(|v|v.as_str()),op.meta.get("output_quantity").and_then(|v|v.as_f64())) {
                *produced_by_activity.entry(id).or_default()+=q;
            }
        }
    }
    let pop_index:HashMap<_,_>=world.pops().values().map(|p|(&p.external_id,p)).collect();
    let mut payroll_index=HashMap::new();
    for payroll in &r.payroll {if payroll_index.insert((&payroll.employment_id,&payroll.pop_id),payroll).is_some(){labor_ok=false;}}
    let mut planned_index=HashMap::new();
    for output in &r.planned_outputs {if planned_index.insert(&output.activity_id,output).is_some(){labor_ok=false;}}
    let mut factory_outputs=HashMap::new();
    for output in &r.outputs {if output.method_kind=="factory" {factory_outputs.entry(&output.subject_id).or_insert(output);}}
    let mut producer_assignments=HashMap::new();
    let assignment_key=|a:&LaborAssignment|(a.pop_id.clone(),a.employment_id.clone(),a.worker_group_id.clone(),if a.workers_count==0. {0}else{a.workers_count.to_bits()});
    for producer in world.resource_operations().values().map(|p|&p.base).chain(world.factories().values().map(|p|&p.base)).chain(world.artisans().values().map(|p|&p.base)) {
        let mut assignments=HashMap::new();
        for a in &producer.assignments {
            let entry=assignments.entry(assignment_key(a)).or_insert(a.time_fraction);
            if a.time_fraction>*entry {*entry=a.time_fraction;}
        }
        if producer_assignments.insert(&producer.id,assignments).is_some(){labor_ok=false;}
    }
    let mut ids = HashSet::new();
    for payroll in &r.payroll {
        labor_ok &= operation_index.get(&payroll.event_id).copied()
            .filter(|op|op.kind==OperationKind::Payroll)
            .and_then(|op| serde_json::to_value(op).ok())
            .and_then(|v| serde_json::from_value::<PayrollReceipt>(v).ok())
            .as_ref()
            == Some(payroll);
        labor_ok &= ids.insert(&payroll.employment_id)
            && payroll.actual_fte.is_finite()
            && payroll.actual_fte >= 0.0;
    }
    for change in &r.registry_changes.factories {
        let expected = change.before.as_ref().and_then(|before| {
            factory_outputs.get(&before.base.id).copied()
                .and_then(|output| update_factory_tenure(before, output.fte).ok())
        });
        labor_ok &= expected.as_ref() == Some(&change.after);
    }
    let mut output_ids = HashSet::new();
    for actual in &r.outputs {
        labor_ok &= output_ids.insert(&actual.activity_id);
        let planned = planned_index.get(&actual.activity_id).copied();
        labor_ok &= planned.is_some_and(|p| {
            actual.quantity.is_finite()
                && actual.quantity >= 0.0
                && actual.quantity <= p.quantity + 1e-8
                && actual.fte.is_finite()
                && actual.fte >= 0.0
                && actual.fte <= p.fte + 1e-7
        });
        let produced=produced_by_activity.get(actual.activity_id.as_str()).copied().unwrap_or(0.);
        goods_ok &= (actual.quantity - produced).abs() <= 1e-8;
        let original = producer_assignments.get(&actual.subject_id);
        for a in &actual.assignments {
            labor_ok &= original.and_then(|rows|rows.get(&assignment_key(a))).is_some_and(|time|a.time_fraction<=time+1e-7);
            let pop = pop_index.get(&a.pop_id).copied();
            labor_ok &= pop.is_some();
            if actual.method_kind != "artisan"
                && a.workers_count * a.time_fraction > 0.0
                && pop.is_some_and(|p| p.pop_type_id.as_str() != Some("slaves"))
            {
                labor_ok &= a
                    .employment_id
                    .as_ref()
                    .and_then(|id| {
                        payroll_index.get(&(id,&a.pop_id)).copied()
                    })
                    .is_some_and(|p| a.workers_count * a.time_fraction <= p.actual_fte + 1e-7);
            }
        }
    }
    let rights_same = before.data.catalog == after.data.catalog
        && before.data.resource_operations == after.data.resource_operations
        && before.data.artisans == after.data.artisans
        && before.data.contracts == after.data.contracts
        && before.data.storage_facilities == after.data.storage_facilities
        && before.data.goods_reservations == after.data.goods_reservations
        && before.data.completed_construction == after.data.completed_construction;
    let rights = if rights_same {
        CheckStatus::NotMeasured{reason:"Full legal-title validation not ported; phase owner/unit/input/storage boundaries enforced by operation validators".into()}
    } else {
        failed("Authored production rights/terms changed")
    };
    InvariantReport {
        money: checked(money_ok, "Unexplained money/obligation/reservation delta"),
        goods: checked(goods_ok, "Unexplained stock/unit/storage delta"),
        people: checked(people_ok, "People or authored POP attributes changed"),
        labor: checked(labor_ok, "Unexplained labor or tenure transition"),
        rights,
    }
}

#[cfg(test)]
mod remediation_tests {
    use super::*;
    use crate::production_invariants_fixture as fixture;
    use std::cell::Cell;
    struct CountOps {rows:Vec<EconomicOperation>,visits:Cell<usize>}
    impl ReadSequence<EconomicOperation> for CountOps {
        fn iter(&self)->Box<dyn Iterator<Item=&EconomicOperation>+'_>{Box::new(self.rows.iter().inspect(|_|self.visits.set(self.visits.get()+1)))}
        fn get(&self,i:usize)->Option<&EconomicOperation>{self.rows.get(i)}
        fn len(&self)->usize{self.rows.len()}
    }
    #[test]
    fn remediation_production_linear_audit() {
        let before=fixture::world();let mut after=before.clone();
        let receipt=crate::production::execute_production_cycle(&mut after,&fixture::request()).unwrap();
        for n in [1000,2000,4000] {
            let mut r=receipt.clone();r.outputs=vec![receipt.outputs[0].clone();n];
            let op: EconomicOperation=serde_json::from_value(serde_json::json!({"event_id":"unused","type":"blocked"})).unwrap();
            let rows=(0..n).map(|i|{let mut op=op.clone();op.event_id=format!("unused-{i}").into();op}).collect();
            let ops=CountOps{rows,visits:Cell::new(0)};
            let result=audit(&before,View::world(&before),View::world(&after),&r,&[] as &[EconomicOperation],&ops);
            assert!(!result.labor.is_passed(),"duplicate outputs must be rejected");
            assert!(ops.visits.get()<=16*(n+r.outputs.len()+1),"n={n}, visits={}",ops.visits.get());
        }
    }
}
