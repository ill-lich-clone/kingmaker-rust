//! Operation-aware audit on compact overlays, before publication. No full-world clone/digest.
use kingmaker_core::world_read::WorldRead;
use crate::{operation_session::*, production_invariants::rows_match};
use kingmaker_core::{audit::*, market::*, operations::*, storage::*, *};
use std::collections::{HashMap, HashSet};
fn check(ok: bool, reason: &str) -> CheckStatus {
    if ok {
        CheckStatus::Passed
    } else {
        CheckStatus::Failed {
            reason: reason.into(),
        }
    }
}
fn close(a: f64, b: f64) -> bool {
    a.is_finite() && b.is_finite() && (a - b).abs() <= 1e-9 * a.abs().max(b.abs()).max(1.0)
}
struct View<'a> {
    accounts: Vec<&'a AccountBalance>,
    stocks: Vec<&'a StockpileRecord>,
    storage: Vec<&'a StorageReservation>,
    debts: Vec<&'a Obligation>,
    pops: kingmaker_core::row_store::RowView<'a,ExternalId,PopRecord>,
    data: kingmaker_core::world_groups::ProductionView<'a>,
}
impl<'a> View<'a> {
    fn world(w: &'a dyn WorldRead) -> Self {
        Self {
            accounts: w.accounts().values().collect(),
            stocks: w.stockpiles().values().collect(),
            storage: w.storage_reservations().values().collect(),
            debts: w.obligations().values().collect(),
            pops: w.pops(),
            data: w.production_data(),
        }
    }
}
pub(crate) fn check_session_market_operations(
    s: &OperationSession,
    r: &MarketPhaseReceipt,
) -> InvariantReport {
    let after = View {
        accounts: s.accounts().collect(),
        stocks: s.stocks().collect(),
        storage: s.storage_reservations().collect(),
        debts: s.obligations().collect(),
        pops: s.base.pops(),
        data: s.base.production_data(),
    };
    audit(View::world(s.base), after, r, true)
}
pub fn check_market_operations(
    before: &dyn WorldRead,
    after: &dyn WorldRead,
    r: &MarketPhaseReceipt,
) -> InvariantReport {
    let history = after.committed_operations().values()
        .map(|o| (&o.event_id, &o.value))
        .collect::<HashMap<_, _>>();
    let valid = r
        .operations
        .iter()
        .all(|o| canonical_operation(o).is_ok_and(|v| history.get(&o.event_id) == Some(&&v)));
    audit(View::world(before), View::world(after), r, valid)
}
fn audit(before: View, after: View, r: &MarketPhaseReceipt, history_ok: bool) -> InvariantReport {
    let mut cash = HashMap::<AccountAddress, i128>::new();
    let mut currencies = HashMap::<ExternalId, i128>::new();
    let mut deltas = HashMap::<ExternalId, f64>::new();
    let mut creates = HashMap::new();
    let mut events = HashSet::new();
    let mut money_ok = true;
    let mut goods_ok = true;
    let mut rights_ok = history_ok;
    for op in &r.operations {
        rights_ok &= events.insert(&op.event_id);
        goods_ok &= matches!(
            op.kind,
            OperationKind::Purchase | OperationKind::FoodConversion
        ) || op.stock_changes.is_empty();
        for (sign, legs) in [(-1i128, &op.debits), (1i128, &op.credits)] {
            for leg in legs {
                money_ok &= leg.amount_minor.0 >= 0;
                *cash.entry(leg.address()).or_default() += sign * i128::from(leg.amount_minor.0);
                *currencies.entry(leg.currency_id.clone()).or_default() +=
                    sign * i128::from(leg.amount_minor.0);
            }
        }
        for d in &op.stock_changes {
            goods_ok &= d.delta.is_finite();
            *deltas.entry(d.stock_id.clone()).or_default() += d.delta;
            if let Some(c) = &d.create {
                if let Some(old) = creates.insert(d.stock_id.clone(), c) {
                    rights_ok &= old == c;
                }
            }
        }
    }
    money_ok &= currencies.values().all(|d| *d == 0);
    let old = before
        .accounts
        .iter()
        .map(|a| (a.address(), *a))
        .collect::<HashMap<_, _>>();
    let new = after
        .accounts
        .iter()
        .map(|a| (a.address(), *a))
        .collect::<HashMap<_, _>>();
    money_ok &= old.len() == before.accounts.len() && new.len() == after.accounts.len();
    for (key, a) in &new {
        money_ok &= a.balance.0 >= 0
            && i128::from(a.balance.0)
                == old.get(key).map(|a| i128::from(a.balance.0)).unwrap_or(0)
                    + cash.get(key).copied().unwrap_or(0);
        if let Some(before) = old.get(key) {
            let mut normalized = (*a).clone();
            normalized.balance = before.balance;
            rights_ok &= &normalized == *before;
        } else {
            rights_ok &= cash.contains_key(key)
                && a.external_id
                    == ExternalId::from(
                        serde_json::json!([a.owner_id, a.currency_id, a.purpose]).to_string(),
                    );
        }
    }
    money_ok &= old.keys().chain(cash.keys()).all(|k| new.contains_key(k));
    let old = before
        .stocks
        .iter()
        .map(|s| (&s.external_id, *s))
        .collect::<HashMap<_, _>>();
    let new = after
        .stocks
        .iter()
        .map(|s| (&s.external_id, *s))
        .collect::<HashMap<_, _>>();
    goods_ok &= old.len() == before.stocks.len() && new.len() == after.stocks.len();
    for (id, s) in &new {
        goods_ok &= s.quantity >= 0.0
            && close(
                s.quantity,
                old.get(id).map(|s| s.quantity).unwrap_or(0.0)
                    + deltas.get(*id).copied().unwrap_or(0.0),
            );
        if let Some(before) = old.get(id) {
            let mut normalized = (*s).clone();
            normalized.quantity = before.quantity;
            rights_ok &= &normalized == *before;
        } else if let Some(c) = creates.get(*id) {
            rights_ok &= s.owner_id == c.owner_id
                && s.province_id == c.province_id
                && s.goods.external_id() == c.goods_id
                && s.unit.as_ref() == Some(&c.unit)
                && s.facility_id == c.facility_id
                && s.currency_id == c.currency_id
                && s.inventory_origin.as_deref() == Some(c.inventory_origin.as_str())
                && s.method_kind == c.method_kind
                && s.purpose
                    == c.extra
                        .get("inventory_purpose")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("commercial")
                && s.beneficiary_pop_id
                    == c.extra
                        .get("beneficiary_pop_id")
                        .filter(|v| !v.is_null())
                        .and_then(|v| ExternalId::from_json(v.clone()).ok());
        } else {
            goods_ok = false;
            rights_ok = false;
        }
    }
    goods_ok &=
        old.keys().all(|k| new.contains_key(k)) && deltas.keys().all(|k| new.contains_key(k));
    let changes = &r.registry_changes;
    rights_ok &= rows_match(&before.storage, &after.storage, &changes.storage, |r| {
        r.id.clone()
    }) && rows_match(&before.debts, &after.debts, &changes.obligations, |r| {
        r.obligation_id.clone()
    });
    let mut used = HashMap::new();
    let facilities = after
        .data
        .storage_facilities
        .iter()
        .map(|f| (&f.id, f))
        .collect::<HashMap<_, _>>();
    for (facility, good, unit, q) in after
        .stocks
        .iter()
        .filter_map(|s| {
            s.facility_id
                .as_ref()
                .map(|id| (id, s.goods.external_id(), s.unit.as_ref(), s.quantity))
        })
        .chain(
            after
                .storage
                .iter()
                .filter(|r| r.status == "held")
                .map(|r| {
                    (
                        &r.facility_id,
                        r.goods_id.clone(),
                        Some(&r.unit),
                        r.quantity,
                    )
                }),
        )
    {
        if let Some(mapping) = facilities
            .get(facility)
            .and_then(|f| f.goods.get(&crate::needs::js_string(&good.json())))
        {
            if unit == Some(&mapping.unit) {
                *used.entry(facility.clone()).or_insert(0.0) += q * mapping.space_per_unit;
            } else {
                rights_ok = false;
            }
        } else {
            rights_ok = false;
        }
    }
    let changed = changes
        .storage
        .iter()
        .map(|r| &r.after.facility_id)
        .collect::<HashSet<_>>();
    for f in &after.data.storage_facilities {
        if changed.contains(&f.id) {
            rights_ok &= f.capacity.is_some_and(|c| {
                c.is_finite() && used.get(&f.id).copied().unwrap_or(0.0) <= c + 1e-8
            });
        }
    }
    let personal_accounts = after
        .accounts
        .iter()
        .filter(|a| a.purpose == "personal_savings")
        .map(|a| ((&a.owner_id, &a.currency_id), a.id))
        .collect::<HashMap<_, _>>();
    let people_ok = before.pops.len() == after.pops.len()
        && before.pops.iter().zip(after.pops).all(|(b, a)| {
            let mut normalized = a.clone();
            normalized.account_id = b.account_id;
            normalized.account_status = b.account_status.clone();
            let binding = if a.account_id == b.account_id && a.account_status == b.account_status {
                true
            } else {
                a.account_status == "resolved"
                    && a.currency_id.as_ref().is_some_and(|currency| {
                        personal_accounts.get(&(&a.external_id, currency)).copied() == a.account_id
                    })
            };
            rights_ok &= binding;
            normalized == *b
        });
    let labor_ok = people_ok
        && before.data.factories == after.data.factories
        && before.data.artisans == after.data.artisans
        && before.data.resource_operations == after.data.resource_operations
        && before.data.construction_projects == after.data.construction_projects;
    InvariantReport {
        money: check(money_ok, "Unexplained market account/currency delta"),
        goods: check(goods_ok, "Unexplained market quantity delta"),
        people: check(people_ok, "Market changed POPs"),
        labor: check(labor_ok, "Market changed labor assignments or producers"),
        rights: if rights_ok {
            CheckStatus::NotMeasured{reason:"Operational titles/reservations checked; full legal property rights are not implemented".into()}
        } else {
            CheckStatus::Failed {
                reason: "Market history/title/storage rights mismatch".into(),
            }
        },
    }
}
