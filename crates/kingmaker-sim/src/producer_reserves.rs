//! Exact current wage, sponsorship, factory-input and debt reserves.
use kingmaker_core::world_read::WorldRead;
use crate::{
    budget_funding::{error, public_state},
    income_recognition::RecognizedProducerIncome,
    market_operations::invoice,
    money_math::{rate_pay, round_ratio},
    needs::{js_string, js_truthy},
    operation_session::{OpResult, OperationSession},
    production_activities::{factory_activity, method},
};
use kingmaker_core::{production::ProductionPolicies, ExternalId, Money};
use num_bigint::BigInt;
use num_traits::Zero;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
type Key = (ExternalId, ExternalId);

/// A natural deposit is not an operating business until land and ownership exist.
/// Shared by admission and finance; empty ownership alone never grants this exception.
pub(crate) fn is_unclaimed_empty_rgo(
    w: &dyn WorldRead,
    r: &kingmaker_core::production::ResourceOperation,
) -> bool {
    let e = &r.base;
    if e.owner_id.is_some()
        || e.facility_id.is_some()
        || r.land_access != Some(false)
        || !e.assignments.is_empty()
        || e.extra
            .get("shares")
            .and_then(Value::as_array)
            .is_some_and(|s| !s.is_empty())
    {
        return false;
    }
    let Some(province) = w.provinces().values().find(|p| p.id == e.province_id) else {
        return false;
    };
    if province.owner_state_id.is_some()
        || province.controller_state_id.is_some()
        || w.pops().values()
            .any(|p| p.province_id == e.province_id && p.size > 0)
    {
        return false;
    }
    let contracts = w.contracts().values()
        .filter(|c| c.employer_entity_id.as_ref() == Some(&e.id))
        .map(|c| &c.contract_id)
        .collect::<HashSet<_>>();
    !w.employment().values()
        .any(|a| a.fte > 0. && contracts.contains(&a.contract_id))
        && !w.accounts().values()
            .any(|a| a.owner_id == e.id && a.balance.0 > 0)
        && !w.stockpiles().values()
            .any(|s| s.owner_id == e.id && s.quantity > 0.)
        && !w.obligations().values()
            .any(|d| d.amount_minor.0 > 0 && (d.payer_id == e.id || d.payee_id == e.id))
        && !w.money_reservations().values()
            .any(|r| r.owner_id == e.id && r.amount_minor.0 > 0)
        && !w.storage_reservations().values()
            .any(|r| r.owner_id == e.id && r.quantity > 0.)
}
pub(crate) struct Reserves {
    pub values: HashMap<Key, BigInt>,
    pub unresolved: HashSet<Key>,
    pub diagnostics: Vec<Value>,
}
fn ensure(rows: &mut Vec<RecognizedProducerIncome>, key: &Key) {
    if !rows
        .iter()
        .any(|r| r.owner_id == key.0 && r.currency_id == key.1)
    {
        rows.push(RecognizedProducerIncome {
            owner_id: key.0.clone(),
            currency_id: key.1.clone(),
            revenue_minor: Money::ZERO,
            opening_stock_revenue_minor: Money::ZERO,
            recognized_cost_minor: Some(Money::ZERO),
            diagnostics: vec![],
        });
    }
}
pub(crate) fn producer_reserves(
    w: &dyn WorldRead,
    period: &str,
    policies: &ProductionPolicies,
    rows: &mut Vec<RecognizedProducerIncome>,
) -> OpResult<Reserves> {
    let subjects = w.resource_operations().values()
        .filter(|e| !is_unclaimed_empty_rgo(w, e))
        .map(|e| &e.base)
        .chain(w.factories().values().map(|e| &e.base))
        .collect::<Vec<_>>();
    let owners = subjects
        .iter()
        .filter_map(|e| e.owner_id.as_ref())
        .collect::<HashSet<_>>();
    let contracts = w.contracts().values()
        .map(|c| (&c.contract_id, c))
        .collect::<HashMap<_, _>>();
    let mut values = HashMap::<Key, BigInt>::new();
    for e in &subjects {
        let owner = e
            .owner_id
            .as_ref()
            .ok_or_else(|| error("Reserve owner missing"))?;
        for a in &e.assignments {
            let Some(c) = a.contract_id.as_ref().and_then(|id| contracts.get(id)) else {
                continue;
            };
            if let Some(rate) = &c.weekly_wage_rate {
                let currency = c
                    .currency_id
                    .as_ref()
                    .ok_or_else(|| error("Reserve currency missing"))?;
                let key = (owner.clone(), currency.clone());
                ensure(rows, &key);
                let multiplier = c
                    .employer_entity_id
                    .as_ref()
                    .and_then(|id| policies.wages.get(&js_string(&id.json())))
                    .copied()
                    .unwrap_or(1.);
                *values.entry(key).or_default() +=
                    BigInt::from(rate_pay(rate, a.workers_count * a.time_fraction, multiplier)?.0)
                        * 2;
            }
        }
    }
    let session = OperationSession::new(w)?;
    let active = session.active_labor_ids()?;
    let mut requirements = HashMap::<(ExternalId, ExternalId, String), BigInt>::new();
    for e in w.employment().values() {
        if e.fte <= 0. || !active.contains(&e.employment_id) {
            continue;
        }
        let c = contracts
            .get(&e.contract_id)
            .ok_or_else(|| error("Reserve employment contract missing"))?;
        if public_state(c)?.is_some() {
            continue;
        }
        if let Some(rate) = &c.weekly_wage_rate {
            let owner = c
                .employer_entity_id
                .as_ref()
                .ok_or_else(|| error("Budget reserve owner missing"))?;
            let currency = c
                .currency_id
                .as_ref()
                .ok_or_else(|| error("Budget reserve currency missing"))?;
            let factor = policies
                .wages
                .get(&js_string(&owner.json()))
                .copied()
                .unwrap_or(1.);
            *requirements
                .entry((
                    owner.clone(),
                    currency.clone(),
                    c.payment_purpose
                        .clone()
                        .unwrap_or("employer_operating_capital".into()),
                ))
                .or_default() += BigInt::from(rate_pay(rate, e.fte, factor)?.0) * 2;
        }
    }
    for agreement in w.maintenance_agreements().values() {
        if let Some(amount) = agreement.extra.get("weekly_amount_minor") {
            let amount: Money = serde_json::from_value(amount.clone())?;
            if amount.0 < 0 {
                return Err(error("Negative sponsored maintenance reserve"));
            }
            *requirements
                .entry((
                    agreement.payer_id.clone(),
                    agreement.currency_id.clone(),
                    "sponsored_maintenance_reserve".into(),
                ))
                .or_default() += BigInt::from(amount.0) * 2;
        }
    }
    for ((owner, currency, purpose), amount) in requirements {
        if !owners.contains(&owner) {
            continue;
        }
        let key = (owner, currency);
        ensure(rows, &key);
        let reserve = values.entry(key.clone()).or_default();
        if purpose == "employer_operating_capital" {
            *reserve = reserve.clone().max(amount)
        } else if purpose == "sponsored_maintenance_reserve" {
            let cash = BigInt::from(session.cash(&key.0, &key.1, &purpose).0);
            if amount > cash {
                *reserve += amount - cash;
            }
        }
    }
    let mut unresolved = HashSet::new();
    let mut diagnostics = vec![];
    if let Some(buffer) = w.market_parameters()
        .as_ref()
        .and_then(|p| p.extra.get("operating_reserve_days"))
        .filter(|v| js_truthy(&v["source"]))
    {
        if buffer["value"] != json!(25) {
            return Err(error("Unversioned factory input reserve"));
        }
        let pops = w.pops().values()
            .map(|p| (&p.external_id, p))
            .collect::<HashMap<_, _>>();
        for f in w.factories().values() {
            if f.status.as_deref() != Some("operating") {
                continue;
            }
            let owner = f
                .base
                .owner_id
                .as_ref()
                .ok_or_else(|| error("Factory input reserve owner missing"))?;
            let Some(account) = w.accounts().values()
                .find(|a| &a.owner_id == owner && a.purpose == "employer_operating_capital")
            else {
                continue;
            };
            let key = (owner.clone(), account.currency_id.clone());
            ensure(rows, &key);
            let m = method(w, &f.base.method_id)?;
            let output = factory_activity(w, f, &pops, period)?.output;
            let mut weekly = BigInt::zero();
            for input in &m.inputs {
                let p = w.price_calibration()
                    .as_ref()
                    .and_then(|p| {
                        p.prices_by_currency
                            .get(&js_string(&account.currency_id.json()))
                    })
                    .and_then(|p| p.get(&js_string(&input.goods_id.json())))
                    .and_then(Value::as_f64)
                    .filter(|p| *p > 0. && p.is_finite());
                if let Some(p) = p {
                    weekly += invoice(output * input.amount, p)?
                } else {
                    unresolved.insert(key.clone());
                    diagnostics.push(
                        json!({"id":f.base.id,"reason":"factory_input_reserve_quote_unknown"}),
                    );
                }
            }
            *values.entry(key).or_default() += round_ratio(&(weekly * 25), &BigInt::from(7))?;
        }
    }
    for debt in w.obligations().values() {
        if owners.contains(&debt.payer_id) {
            if debt.amount_minor.0 < 0 {
                return Err(error("Negative producer debt reserve"));
            }
            let key = (debt.payer_id.clone(), debt.currency_id.clone());
            ensure(rows, &key);
            *values.entry(key).or_default() += BigInt::from(debt.amount_minor.0);
        }
    }
    Ok(Reserves {
        values,
        unresolved,
        diagnostics,
    })
}
