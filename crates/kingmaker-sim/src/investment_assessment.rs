//! Actual current labor, authored final norms, quotes and consented local supply.
use kingmaker_core::world_read::WorldRead;
use crate::{
    budget_funding::error,
    market_operations::invoice,
    money_math::{multiply_money, rate_pay},
    needs::js_string,
    operation_session::OpResult,
    transport_validation::{actor, province},
};
use kingmaker_core::{
    construction::ConstructionRequest, ExternalId, Money, ProvinceId,
};
use num_bigint::BigInt;
use num_traits::{ToPrimitive, Zero};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
type InputKey = (ProvinceId, Option<ExternalId>, ExternalId);
fn number(v: &Value) -> f64 {
    v.as_f64()
        .or_else(|| v.as_str().and_then(|v| v.parse().ok()))
        .unwrap_or(0.)
}
fn offered(v: &kingmaker_core::market::SaleOffer) -> bool {
    crate::storage::approved(
        v.extra.get("status").and_then(Value::as_str),
        v.extra.get("source"),
        v.extra.get("approval_ref"),
    )
}
pub fn assess_investment_opportunities(
    w: &dyn WorldRead,
    r: &ConstructionRequest,
) -> OpResult<Vec<Value>> {
    let catalog = w.production_catalog()
        .as_ref()
        .ok_or_else(|| error("Investment production catalog missing"))?;
    let methods: HashMap<_, _> = catalog.methods.iter().map(|m| (&m.method_id, m)).collect();
    let pops: HashMap<_, _> = w.pops().values().map(|p| (&p.external_id, p)).collect();
    let contracts: HashMap<_, _> = w.contracts().values()
        .map(|c| (&c.contract_id, c))
        .collect();
    let roster = crate::labor_roster::build_labor_roster(w)?;
    let used: HashMap<_, _> = roster
        .by_pop
        .iter()
        .map(|(id, a)| (id.clone(), a.occupied))
        .collect();
    let policies = crate::producer_income::effective_income_policies(
        w,
        &kingmaker_core::income::IncomeRequest {
            expected_revision: r.expected_revision,
            period: r.period.clone(),
            policies: r.policies.clone(),
        },
    );
    let mut population = HashMap::<ProvinceId, u64>::new();
    for p in w.pops().values() {
        *population.entry(p.province_id).or_default() += u64::from(p.size);
    }
    let popowners: HashSet<_> = w.pops().values().map(|p| &p.external_id).collect();
    let mut outputs = HashSet::new();
    let mut protected = HashSet::new();
    for e in w.resource_operations().values()
        .map(|e| &e.base)
        .chain(w.factories().values().map(|e| &e.base))
        .chain(w.artisans().values().map(|e| &e.base))
    {
        if let (Some(owner), Some(m)) = (&e.owner_id, methods.get(&e.method_id)) {
            outputs.insert((owner.clone(), e.province_id, m.output_goods_id.clone()));
            for i in &m.inputs {
                protected.insert((owner.clone(), e.province_id, i.goods_id.clone()));
            }
        }
    }
    let offers: HashMap<_, _> = w.sale_offers().values()
        .filter(|o| offered(o))
        .map(|o| (&o.stock_id, o))
        .collect();
    let mut local = HashSet::<InputKey>::new();
    for stock in w.stockpiles().values() {
        let key = (
            stock.owner_id.clone(),
            stock.province_id,
            stock.goods.external_id(),
        );
        let consented = offers.get(&stock.external_id).is_some_and(|o| {
            o.seller_id == stock.owner_id && o.goods_id == stock.goods.external_id()
        });
        let standing = outputs.contains(&key)
            && !protected.contains(&key)
            && (!popowners.contains(&stock.owner_id)
                || stock.inventory_origin.as_deref() == Some("produced"));
        if stock.quantity > 0. && (consented || standing) {
            local.insert((
                stock.province_id,
                stock.currency_id.clone(),
                stock.goods.external_id(),
            ));
        }
    }
    let mut employer_currencies = HashMap::new();
    for a in w.accounts().values() {
        if a.purpose == "employer_operating_capital" {
            employer_currencies
                .entry(&a.owner_id)
                .or_insert(&a.currency_id);
        }
    }
    for raw in w.resource_operations().values() {
        if raw.land_access == Some(true)
            && matches!(raw.resource_status.as_deref(), Some("source" | "approved"))
            && raw
                .base
                .assignments
                .iter()
                .any(|a| a.workers_count * a.time_fraction > 0.)
        {
            if let Some(currency) = raw
                .base
                .owner_id
                .as_ref()
                .and_then(|id| employer_currencies.get(id))
            {
                local.insert((
                    raw.base.province_id,
                    Some((*currency).clone()),
                    raw.resource_id.clone(),
                ));
            }
        }
    }
    let mut result = vec![];
    for o in w.investment_opportunities().values() {
        crate::run_control::work("investment_assessment_offer")?;
        if o["assessment_policy"] != "native_weekly_v1" {
            result.push(o.clone());
            continue;
        }
        let mid = actor(&o["method_id"])?;
        let m = methods
            .get(&mid)
            .ok_or_else(|| error("Unknown investment method"))?;
        let currency = actor(&o["currency_id"])?;
        let place = province(&o["province_id"])?;
        let prices = w.price_calibration()
            .as_ref()
            .and_then(|p| p.prices_by_currency.get(&js_string(&currency.json())));
        let norm = w.mobility_reference()
            .as_ref()
            .and_then(|r| r["goods"].as_array())
            .and_then(|g| g.iter().find(|g| g["goods_id"] == m.output_goods_id.json()))
            .map(|g| number(&g["Конечная норма / 1000 / неделю"]))
            .unwrap_or(0.);
        let demand = norm * population.get(&place).copied().unwrap_or(0) as f64 / 1000.;
        // Each proposal is independent. Copy only workers touched by this proposal;
        // untouched workers retain their actual occupied labor from the roster.
        let mut taken = HashMap::new();
        let mut fte = 0.;
        let mut wages = BigInt::zero();
        let mut unresolved = false;
        let mut available = true;
        for a in o["future_assignments"]
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or(&[])
        {
            crate::run_control::work("investment_assessment_assignment")?;
            let popid = actor(&a["pop_id"])?;
            let cid = actor(&a["contract_id"])?;
            let Some(p) = pops.get(&popid) else {
                unresolved = true;
                continue;
            };
            let Some(c) = contracts.get(&cid) else {
                unresolved = true;
                continue;
            };
            let Some(rate) = &c.weekly_wage_rate else {
                unresolved = true;
                continue;
            };
            if c.currency_id.as_ref() != Some(&currency)
                || !matches!(
                    js_string(&p.pop_type_id.json()).as_str(),
                    "craftsmen" | "clerks"
                )
                || p.province_id != place
            {
                unresolved = true;
                continue;
            }
            let component = |v: &Value| v.as_f64().or_else(|| v.as_str().and_then(|s| s.parse::<f64>().ok()));
            let workers = component(&a["workers_count"]).ok_or_else(|| error("Invalid investment future workers"))?;
            let fraction = component(&a["time_fraction"]).ok_or_else(|| error("Invalid investment future time"))?;
            let wanted = workers * fraction;
            if !workers.is_finite() || workers < 0. || workers.fract() != 0. || workers > 9007199254740991.
                || !fraction.is_finite() || !(0. ..=1.).contains(&fraction)
                || !wanted.is_finite() || wanted < 0. {
                return Err(error("Invalid investment future labor"));
            }
            let occupied = taken.entry(popid.clone()).or_insert_with(|| used.get(&popid).copied().unwrap_or(0.));
            let free = (p.workforce - *occupied).max(0.);
            let time = wanted.min(free);
            if time + 1e-9 < wanted {
                available = false;
            }
            *occupied += time;
            if p.pop_type_id == ExternalId::from("craftsmen") {
                fte += time;
            }
            let multiplier = c
                .employer_entity_id
                .as_ref()
                .and_then(|id| policies.wages.get(&js_string(&id.json())))
                .copied()
                .unwrap_or(1.);
            wages += BigInt::from(
                rate_pay(rate, time, multiplier)
                    .map_err(|e| error(&e.to_string()))?
                    .0,
            );
        }
        let output = fte * m.weekly_productivity.unwrap_or(0.);
        let mut margin = BigInt::zero();
        let output_price = prices
            .and_then(|p| p.get(&js_string(&m.output_goods_id.json())))
            .map(number)
            .unwrap_or(0.);
        if output > 0. && output_price > 0. {
            margin = invoice(output, output_price)? - wages;
            for input in &m.inputs {
                let price = prices
                    .and_then(|p| p.get(&js_string(&input.goods_id.json())))
                    .map(number)
                    .unwrap_or(0.);
                if price <= 0. {
                    unresolved = true;
                    continue;
                }
                margin -= invoice(output * input.amount, price)?;
            }
            if margin > BigInt::zero() {
                let state = w.territory().values()
                    .find(|p| p.province_id == place)
                    .and_then(|p| p.extra.get("owner_state_id"))
                    .map(js_string);
                let tax = state
                    .and_then(|s| policies.taxes.get(&s))
                    .copied()
                    .unwrap_or(0.);
                if !tax.is_finite() || !(0.0..=1.0).contains(&tax) {
                    return Err(error("Invalid investment tax policy"));
                }
                let value = margin
                    .to_i64()
                    .ok_or_else(|| error("Investment margin overflow"))?;
                margin -= BigInt::from(
                    multiply_money(Money(value), tax)
                        .map_err(|e| error(&e.to_string()))?
                        .0,
                );
            }
        } else {
            unresolved = true;
        }
        let accessible = m
            .inputs
            .iter()
            .all(|i| local.contains(&(place, Some(currency.clone()), i.goods_id.clone())));
        let mut row = o.clone();
        row["demand"] = json!(demand);
        row["workforce_available"] = json!(if available { fte } else { 0. });
        row["inputs_accessible"] = json!(accessible);
        row["margin_minor"] = json!(if unresolved {
            "0".to_string()
        } else {
            margin.to_string()
        });
        row["assessment"] = json!({"staff_plan_available":available,"period_date":{"year":w.campaign().year,"month":w.campaign().month,"day":w.campaign().day},"demand_basis":"authored native final weekly norm times current actual population","margin_basis":"current weekly native output/input quotes minus current individual contracts and dividend tax; conservative base staffing, no promised sale","input_basis":"existing local stock or staffed approved local raw source; purchase and paid actual production still required","unresolved":unresolved});
        result.push(row);
    }
    Ok(result)
}
