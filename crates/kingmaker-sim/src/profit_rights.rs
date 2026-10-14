//! Retained factory rights are weighted by actual positive sector profits, not RGO shares.
use kingmaker_core::world_read::WorldRead;
use crate::{
    budget_funding::error, income_basis::IncomeOpeningBasis, market_operations::invoice,
    money_math::multiply_money, needs::js_string, operation_session::OpResult,
    transport_validation::actor,
};
use kingmaker_core::{
    operations::{EconomicOperation, OperationKind},
    ExternalId, Money, ProvinceId,
};
use num_bigint::BigInt;
use num_traits::{Signed, Zero};
use serde_json::{json, Value};
use std::collections::HashMap;
pub(crate) type OwnerCurrency = (ExternalId, ExternalId);
pub(crate) fn integer(v: &Value) -> OpResult<BigInt> {
    js_string(v)
        .parse()
        .map_err(|_| error("Invalid ownership integer"))
}
pub(crate) fn mixed_producer_rights(
    w: &dyn WorldRead,
    basis: &IncomeOpeningBasis,
    operations: &[EconomicOperation],
) -> OpResult<HashMap<OwnerCurrency, Value>> {
    let overrides = w.factory_beneficiaries().values()
        .map(|v| Ok(((actor(&v["owner_id"])?, actor(&v["currency_id"])?), v)))
        .collect::<OpResult<HashMap<_, _>>>()?;
    if overrides.is_empty() {
        return Ok(HashMap::new());
    }
    let stocks = basis
        .stocks
        .iter()
        .chain(&w.stockpiles())
        .map(|s| (&s.external_id, s))
        .collect::<HashMap<_, _>>();
    let subjects = w.resource_operations().values()
        .map(|e| ("rgo", &e.base))
        .chain(
            w.factories().values()
                .map(|e| ("factory", &e.base)),
        )
        .collect::<Vec<_>>();
    let methods = w.production_catalog()
        .as_ref()
        .ok_or_else(|| error("Profit rights catalog missing"))?
        .methods
        .iter()
        .map(|m| (&m.method_id, m))
        .collect::<HashMap<_, _>>();
    let mut kinds = HashMap::new();
    let mut by_subject = HashMap::new();
    let mut by_employment = HashMap::new();
    for (kind, e) in &subjects {
        by_subject.insert(&e.id, (*kind, *e));
        if let (Some(owner), Some(m)) = (&e.owner_id, methods.get(&e.method_id)) {
            kinds.insert(
                (owner.clone(), e.province_id, m.output_goods_id.clone()),
                *kind,
            );
            let alias = match m.output_goods_id.as_str() {
                Some("grain") => Some("flour"),
                Some("livestock") => Some("meat"),
                _ => None,
            };
            if let Some(alias) = alias {
                kinds.insert((owner.clone(), e.province_id, alias.into()), *kind);
            }
        }
        for a in &e.assignments {
            if let Some(id) = &a.employment_id {
                by_employment.insert(id, *kind);
            }
        }
    }
    let contracts = w.contracts().values()
        .map(|c| (&c.contract_id, c))
        .collect::<HashMap<_, _>>();
    let employment = w.employment().values()
        .map(|e| (&e.employment_id, e))
        .collect::<HashMap<_, _>>();
    let classify =
        |owner: &ExternalId, province: ProvinceId, goods: &ExternalId, kind: Option<&str>| {
            kind.map(str::to_owned).or_else(|| {
                kinds
                    .get(&(owner.clone(), province, goods.clone()))
                    .map(|v| (*v).to_owned())
            })
        };
    let mut buckets = HashMap::<OwnerCurrency, (BigInt, BigInt)>::new();
    let mut add = |owner: ExternalId, currency: ExternalId, kind: Option<&str>, amount: BigInt| {
        let key = (owner, currency);
        if !overrides.contains_key(&key) {
            return;
        }
        if matches!(kind, Some("rgo" | "factory")) {
            let b = buckets
                .entry(key)
                .or_insert((BigInt::zero(), BigInt::zero()));
            if kind == Some("rgo") {
                b.0 += amount
            } else {
                b.1 += amount
            }
        }
    };
    for op in operations {
        match op.kind {
            OperationKind::Purchase => {
                for credit in &op.credits {
                    if !overrides
                        .contains_key(&(credit.owner_id.clone(), credit.currency_id.clone()))
                    {
                        continue;
                    }
                    let sold = op
                        .stock_changes
                        .iter()
                        .filter(|c| {
                            c.delta < 0.
                                && stocks
                                    .get(&c.stock_id)
                                    .is_some_and(|s| s.owner_id == credit.owner_id)
                        })
                        .collect::<Vec<_>>();
                    let total = sold.iter().map(|c| -c.delta).sum::<f64>();
                    if total <= 0. {
                        continue;
                    }
                    for change in sold {
                        let stock = stocks[&change.stock_id];
                        if stock.inventory_origin.as_deref() != Some("produced") {
                            continue;
                        }
                        let kind = classify(
                            &credit.owner_id,
                            stock.province_id,
                            &stock.goods.external_id(),
                            stock.method_kind.as_deref(),
                        )
                        .ok_or_else(|| error("Producer profit kind unresolved"))?;
                        add(
                            credit.owner_id.clone(),
                            credit.currency_id.clone(),
                            Some(&kind),
                            BigInt::from(
                                multiply_money(credit.amount_minor, -change.delta / total)?.0,
                            ),
                        );
                    }
                }
            }
            OperationKind::ShipmentArrival
                if op.meta.get("goods_origin").and_then(Value::as_str) == Some("produced") =>
            {
                let id = actor(&op.meta["shipment_id"])?;
                let s = w.shipments().values()
                    .find(|s| s["id"] == id.json())
                    .ok_or_else(|| error("Shipment profit reference missing"))?;
                let p = crate::transport_validation::province(&s["from_province_id"])?;
                let good = actor(&s["goods_id"])?;
                for credit in &op.credits {
                    if overrides
                        .contains_key(&(credit.owner_id.clone(), credit.currency_id.clone()))
                    {
                        let kind = classify(&credit.owner_id, p, &good, None)
                            .ok_or_else(|| error("Shipment profit kind unresolved"))?;
                        add(
                            credit.owner_id.clone(),
                            credit.currency_id.clone(),
                            Some(&kind),
                            BigInt::from(credit.amount_minor.0),
                        );
                    }
                }
            }
            OperationKind::Payroll => {
                let id = actor(&op.meta["employment_id"])?;
                let mut kind = by_employment.get(&id).copied();
                if kind.is_none() {
                    kind = employment
                        .get(&id)
                        .and_then(|e| contracts.get(&e.contract_id))
                        .and_then(|c| c.extra.get("building_id"))
                        .and_then(|v| actor(v).ok())
                        .and_then(|id| by_subject.get(&id).map(|s| s.0));
                }
                let amount: Money = serde_json::from_value(op.meta["paid_gross_minor"].clone())?;
                add(
                    actor(&op.meta["employer_id"])?,
                    actor(&op.meta["currency_id"])?,
                    kind,
                    -BigInt::from(amount.0),
                );
            }
            OperationKind::Production => {
                let id = actor(&op.meta["subject_id"])?;
                let Some((kind, e)) = by_subject.get(&id) else {
                    continue;
                };
                let owner = e
                    .owner_id
                    .as_ref()
                    .ok_or_else(|| error("Profit owner missing"))?;
                for change in op.stock_changes.iter().filter(|c| c.delta < 0.) {
                    let stock = stocks
                        .get(&change.stock_id)
                        .ok_or_else(|| error("Profit input stock missing"))?;
                    if let Some(currency) = &stock.currency_id {
                        let price = w.price_calibration()
                            .as_ref()
                            .and_then(|p| p.prices_by_currency.get(&js_string(&currency.json())))
                            .and_then(|p| p.get(&js_string(&stock.goods.external_id().json())))
                            .and_then(Value::as_f64)
                            .filter(|p| *p > 0. && p.is_finite());
                        if let Some(price) = price {
                            add(
                                owner.clone(),
                                currency.clone(),
                                Some(kind),
                                -invoice(-change.delta, price)?,
                            );
                        }
                    }
                }
            }
            _ => {}
        }
    }
    let mut result = HashMap::new();
    for (key, factory) in overrides {
        let Some(base) = w.production_beneficiaries().values()
            .find(|v| v["owner_id"] == key.0.json() && v["currency_id"] == key.1.json())
        else {
            continue;
        };
        let (rgo, made) = buckets
            .get(&key)
            .cloned()
            .unwrap_or((BigInt::zero(), BigInt::zero()));
        let raw = rgo.clone().max(BigInt::zero());
        let positive = made.clone().max(BigInt::zero());
        let total = &raw + &positive;
        if total.is_zero() {
            continue;
        }
        let mut shares = vec![];
        for (rights, weight) in [(base, &raw), (factory, &positive)] {
            if weight.is_zero() {
                continue;
            }
            let rows = rights["beneficiaries"]
                .as_array()
                .ok_or_else(|| error("Producer sector rights missing"))?;
            for row in rows {
                let mut share = row.clone();
                share["numerator"] = json!((integer(&row["numerator"])? * weight).to_string());
                share["denominator"] = json!((integer(&row["denominator"])? * &total).to_string());
                shares.push(share);
            }
        }
        let mut rights = base.clone();
        rights["source"]=json!("RGO and retained factory rights weighted by positive sector operating profit; shared holding costs/reserves retained");
        rights["beneficiaries"] = json!(shares);
        rights["profit_by_kind"] = json!({"rgo":rgo.to_string(),"factory":made.to_string()});
        result.insert(key, rights);
    }
    Ok(result)
}
pub(crate) fn validate_shares(shares: &[Value]) -> OpResult<()> {
    let mut n = BigInt::zero();
    let mut d = BigInt::from(1);
    for s in shares {
        let sn = integer(&s["numerator"])?;
        let sd = integer(&s["denominator"])?;
        if sn.is_negative()
            || sd <= BigInt::zero()
            || !crate::needs::js_truthy(&s["owner_id"])
            || !s["purpose"].as_str().is_some_and(|p| !p.is_empty())
        {
            return Err(error("Invalid production beneficiary rights"));
        }
        n = &n * &sd + sn * &d;
        d *= &sd;
        let (mut a, mut b) = (n.clone(), d.clone());
        while !b.is_zero() {
            let rem = &a % &b;
            a = b;
            b = rem;
        }
        if !a.is_zero() {
            n /= &a;
            d /= a;
        }
    }
    if n != d {
        return Err(error("Production beneficial shares must sum to one"));
    }
    Ok(())
}
