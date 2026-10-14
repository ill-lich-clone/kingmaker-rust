//! Actual market demand drives a readonly choice plan; only the closed phase can publish it.
use kingmaker_core::world_read::WorldRead;
use crate::{
    budget_funding::error,
    calendar::{add_days, serial_date},
    income_capital::build_input_supply_index,
    income_recognition::required_income_operations,
    market_operations::invoice,
    money_math::multiply_money,
    needs::{js_string, js_truthy},
    operation_session::{OpResult, OperationSession},
    profit_rights::integer,
    transport_validation::actor,
};
use kingmaker_core::{
    calendar::GameDate,
    income::IncomeRequest,
    market::MarketStage,
    operations::{EconomicOperation, OperationKind},
    production::ArtisanEnterprise,
    ExternalId, Money,
};
use num_bigint::BigInt;
use num_traits::{Signed, ToPrimitive, Zero};
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, Serialize)]
pub struct ArtisanChoice {
    pub method_id: ExternalId,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub margin_minor: Option<Money>,
    pub retooling: Option<Value>,
    pub reason: String,
}
#[derive(Debug, Clone, Serialize)]
pub struct ArtisanChoicesPlan {
    pub operations: Vec<EconomicOperation>,
    pub enterprises: Vec<ArtisanEnterprise>,
    pub diagnostics: Vec<Value>,
}
fn as_minor(n: BigInt) -> OpResult<Money> {
    n.to_i64()
        .map(Money)
        .ok_or_else(|| error("Artisan choice money overflow"))
}
fn integral_number(v: &Value) -> Option<i64> {
    v.as_f64()
        .filter(|n| n.is_finite() && n.fract() == 0. && n.abs() <= 9_007_199_254_740_991.)
        .map(|n| n as i64)
}
fn date(w: &dyn WorldRead) -> GameDate {
    GameDate {
        year: w.campaign().year,
        month: w.campaign().month,
        day: w.campaign().day,
    }
}
fn accounting_currency(w: &dyn WorldRead, e: &ArtisanEnterprise) -> Option<ExternalId> {
    let owner = e.base.owner_id.as_ref()?;
    let purpose = e
        .base
        .payment_purpose
        .as_deref()
        .unwrap_or("operating_capital");
    w.accounts().values()
        .find(|a| &a.owner_id == owner && a.purpose == purpose)
        .map(|a| a.currency_id.clone())
        .or_else(|| {
            e.working_capital
                .as_ref()?
                .get("currency_id")
                .and_then(|v| actor(v).ok())
        })
        .or_else(|| {
            w.accounts().values()
                .find(|a| &a.owner_id == owner && a.purpose == "personal_savings")
                .map(|a| a.currency_id.clone())
        })
}
pub fn build_artisan_quotes(w: &dyn WorldRead, r: &IncomeRequest) -> OpResult<Vec<Value>> {
    required_income_operations(w, r)?;
    let receipts: Vec<_> = w.all_market_phase_receipts().select(|p| p.request.period == r.period && p.request.stage == MarketStage::ClosingMarket)?;
    if receipts.len() != 1 {
        return Err(error("Artisan quotes require unique closing market"));
    }
    let receipt = &*receipts[0];
    let mut filled: HashMap<String, f64> = HashMap::new();
    for op in &receipt.operations {
        if op.kind == OperationKind::Purchase {
            let order = op
                .meta
                .get("order_id")
                .and_then(Value::as_str)
                .ok_or_else(|| error("Artisan quote purchase order missing"))?;
            let quantity = op
                .meta
                .get("quantity")
                .and_then(Value::as_f64)
                .filter(|q| q.is_finite() && *q >= 0.)
                .ok_or_else(|| error("Artisan quote purchase quantity invalid"))?;
            *filled.entry(order.into()).or_default() += quantity;
        }
    }
    build_quotes(w, &receipt.orders, &filled)
}
pub(crate) fn build_opening_artisan_quotes(
    w: &dyn WorldRead,
    profile: &kingmaker_core::market::MarketProfile,
) -> OpResult<Vec<Value>> {
    let needs = crate::needs::build_need_orders(w, profile)?;
    build_quotes(w, &needs.orders, &HashMap::new())
}
fn build_quotes(
    w: &dyn WorldRead,
    orders: &[kingmaker_core::market::MarketOrder],
    filled: &HashMap<String, f64>,
) -> OpResult<Vec<Value>> {
    let mut demand: HashMap<_, f64> = HashMap::new();
    for order in orders {
        *demand
            .entry((
                order.province_id,
                order.currency_id.clone(),
                order.goods_id.clone(),
            ))
            .or_default() +=
            (order.quantity - filled.get(&order.id).copied().unwrap_or(0.)).max(0.);
    }
    let mut places = vec![];
    let mut managed = HashSet::new();
    for e in w.artisans().values() {
        if let Some(currency) = accounting_currency(w, e) {
            let key = (e.base.province_id, currency);
            if !places.contains(&key) {
                places.push(key.clone());
            }
            if e.working_capital.is_some()
                || e.base
                    .extra
                    .get("income_policy")
                    .and_then(|p| p.get("source"))
                    .is_some_and(js_truthy)
            {
                managed.insert(key);
            }
        }
    }
    let supply = (!managed.is_empty())
        .then(|| build_input_supply_index(w))
        .transpose()?;
    let mut quotes = vec![];
    for (province, currency) in places {
        if let Some(prices) = w.price_calibration()
            .as_ref()
            .and_then(|p| p.prices_by_currency.get(&js_string(&currency.json())))
        {
            for (good, price) in prices {
                let Some(price) = price.as_f64().filter(|p| p.is_finite() && *p > 0.) else {
                    continue;
                };
                let mut q = json!({"province_id":province.0,"currency_id":currency,"goods_id":good,"price":price,"demand":demand.get(&(province,Some(currency.clone()),ExternalId::from(good.clone()))).copied().unwrap_or(0.)});
                if managed.contains(&(province, currency.clone())) {
                    let supply = supply.as_ref().unwrap();
                    q["available"] = json!(supply
                        .available
                        .get(&(
                            supply.scope.key(province),
                            Some(currency.clone()),
                            good.clone().into()
                        ))
                        .copied()
                        .unwrap_or(0.));
                }
                quotes.push(q);
            }
        }
    }
    Ok(quotes)
}
fn quote<'a>(
    quotes: &'a [Value],
    good: &ExternalId,
    province: kingmaker_core::ProvinceId,
) -> Option<&'a Value> {
    quotes.iter().find(|q| {
        q.get("goods_id") == Some(&good.json())
            && (q.get("province_id").is_none_or(Value::is_null)
                || js_string(&q["province_id"]) == province.0.to_string())
    })
}
fn quoted_invoice(q: &Value, amount: f64) -> OpResult<BigInt> {
    if !amount.is_finite() || amount < 0. {
        return Err(error("Invalid artisan quoted quantity"));
    }
    if let Some(price) = q.get("price").filter(|p| !p.is_null()) {
        let price = price
            .as_f64()
            .filter(|p| p.is_finite() && *p > 0.)
            .ok_or_else(|| error("Invalid artisan quote price"))?;
        invoice(amount, price)
    } else {
        let price = as_minor(integer(&q["price_minor"])?)?;
        if price.0 < 0 {
            return Err(error("Negative artisan quote price"));
        }
        Ok(BigInt::from(multiply_money(price, amount)?.0))
    }
}
pub fn choose_artisan_method(
    w: &dyn WorldRead,
    id: &ExternalId,
    quotes: &[Value],
    capital_budget: Option<Money>,
) -> OpResult<ArtisanChoice> {
    let e = w.artisans().values()
        .find(|e| &e.base.id == id)
        .ok_or_else(|| error("Unknown artisan enterprise"))?;
    let cash = OperationSession::new(w)?;
    let stocks = choice_stocks(w)?;
    choose_for_enterprise(w, e, quotes, capital_budget, &cash, &stocks)
}
type ChoiceStockIndex = HashMap<(ExternalId, kingmaker_core::ProvinceId, ExternalId), f64>;
pub(crate) fn choice_stocks(w: &dyn WorldRead) -> OpResult<ChoiceStockIndex> {
    let mut stocks = HashMap::new();
    for stock in w.stockpiles().values() {
        if stock.purpose == "food_reserve" {
            continue;
        }
        if !stock.quantity.is_finite() || stock.quantity < 0. {
            return Err(error("Invalid artisan choice stock"));
        }
        *stocks
            .entry((
                stock.owner_id.clone(),
                stock.province_id,
                stock.goods.external_id(),
            ))
            .or_default() += stock.quantity;
    }
    Ok(stocks)
}
pub(crate) fn choose_for_enterprise(
    w: &dyn WorldRead,
    e: &ArtisanEnterprise,
    quotes: &[Value],
    capital_budget: Option<Money>,
    cash: &OperationSession,
    stocks: &ChoiceStockIndex,
) -> OpResult<ArtisanChoice> {
    if let Some(pending) = &e.pending_retooling {
        return Ok(ArtisanChoice {
            method_id: e.base.method_id.clone(),
            margin_minor: None,
            retooling: Some(pending.clone()),
            reason: "retooling in progress".into(),
        });
    }
    let fallback = || ArtisanChoice {
        method_id: e.base.method_id.clone(),
        margin_minor: None,
        retooling: None,
        reason: "no feasible profitable alternative".into(),
    };
    let catalog = w.production_catalog()
        .as_ref()
        .ok_or_else(|| error("Artisan catalog missing"))?;
    let owner = e
        .base
        .owner_id
        .as_ref()
        .ok_or_else(|| error("Artisan owner missing"))?;
    let fte = e
        .base
        .assignments
        .iter()
        .map(|a| a.workers_count * a.time_fraction)
        .sum::<f64>();
    if !fte.is_finite() || fte < 0. {
        return Err(error("Invalid artisan choice FTE"));
    }
    let mut candidates = vec![];
    for m in &catalog.methods {
        if m.kind != "artisan"
            || m.status != "ready"
            || m.technology
                .iter()
                .any(|t| !e.implemented_technologies.contains(t))
        {
            continue;
        }
        let Some(sale) = quote(quotes, &m.output_goods_id, e.base.province_id) else {
            continue;
        };
        let Some(demand) = sale
            .get("demand")
            .and_then(Value::as_f64)
            .filter(|d| d.is_finite() && *d > 0.)
        else {
            continue;
        };
        if sale.get("price").is_none_or(Value::is_null)
            && sale.get("price_minor").is_none_or(Value::is_null)
        {
            continue;
        }
        let currency = actor(&sale["currency_id"])?;
        let productivity = m.weekly_productivity.unwrap_or(0.);
        if !productivity.is_finite() || productivity < 0. {
            return Err(error("Invalid artisan choice productivity"));
        }
        let mut output = (fte * productivity).min(demand);
        let mut valid = true;
        let mut inputs = vec![];
        for input in &m.inputs {
            if !input.amount.is_finite() || input.amount <= 0. {
                return Err(error("Invalid artisan choice recipe"));
            }
            let Some(q) = quote(quotes, &input.goods_id, e.base.province_id) else {
                valid = false;
                break;
            };
            if q.get("currency_id") != Some(&currency.json())
                || (q.get("price").is_none_or(Value::is_null)
                    && q.get("price_minor").is_none_or(Value::is_null))
            {
                valid = false;
                break;
            }
            let owned = stocks
                .get(&(owner.clone(), e.base.province_id, input.goods_id.clone()))
                .copied()
                .unwrap_or(0.);
            let available = q.get("available").and_then(Value::as_f64).unwrap_or(0.);
            if !available.is_finite() {
                return Err(error("Invalid artisan quote availability"));
            }
            output = output.min((owned + available.max(0.)) / input.amount);
            inputs.push((input, q, owned));
        }
        if !valid {
            continue;
        }
        let purchase_cost = |output: f64| -> OpResult<BigInt> {
            inputs
                .iter()
                .try_fold(BigInt::zero(), |sum, (input, q, owned)| {
                    Ok(sum + quoted_invoice(q, (output * input.amount - owned).max(0.))?)
                })
        };
        let budget = capital_budget.unwrap_or_else(|| {
            cash.cash(
                owner,
                &currency,
                e.base
                    .payment_purpose
                    .as_deref()
                    .unwrap_or("operating_capital"),
            )
        });
        if budget.0 < 0 {
            return Err(error("Negative artisan choice budget"));
        }
        if purchase_cost(output)? > BigInt::from(budget.0) {
            let mut low = 0.;
            let mut high = output;
            for _ in 0..64 {
                let mid = (low + high) / 2.;
                if mid == low || mid == high {
                    break;
                }
                if purchase_cost(mid)? <= BigInt::from(budget.0) {
                    low = mid;
                } else {
                    high = mid;
                }
            }
            output = low;
        }
        let mut margin = quoted_invoice(sale, output)?;
        for (input, q, _) in &inputs {
            margin -= quoted_invoice(q, input.amount * output)?;
        }
        if margin <= BigInt::zero() || output <= 0. {
            continue;
        }
        let mut cost = BigInt::zero();
        let mut retooling = None;
        if m.method_id != e.base.method_id {
            let Some(terms) = e.base.extra.get("retooling") else {
                continue;
            };
            if !terms.get("source").is_some_and(js_truthy)
                || !terms.get("provider_id").is_some_and(js_truthy)
                || terms.get("currency_id") != Some(&currency.json())
                || terms
                    .get("duration_days")
                    .and_then(integral_number)
                    .is_none_or(|d| d < 1)
            {
                continue;
            }
            cost = integer(&terms["cost_minor"])?;
            let purpose = terms.get("purpose").and_then(Value::as_str).unwrap_or("");
            let retool_cash =
                capital_budget.unwrap_or_else(|| cash.cash(owner, &currency, purpose));
            if cost.is_negative() || BigInt::from(retool_cash.0) < cost {
                continue;
            }
            retooling = Some(terms.clone());
        }
        if margin <= cost {
            continue;
        }
        candidates.push(ArtisanChoice {
            method_id: m.method_id.clone(),
            margin_minor: Some(as_minor(margin - cost)?),
            retooling,
            reason: "owned inputs, demand and positive margin".into(),
        });
    }
    let collator = crate::ordering::collator();
    candidates.sort_by(|a, b| {
        b.margin_minor
            .unwrap()
            .0
            .cmp(&a.margin_minor.unwrap().0)
            .then_with(|| {
                collator.compare(
                    &js_string(&a.method_id.json()),
                    &js_string(&b.method_id.json()),
                )
            })
    });
    Ok(candidates.into_iter().next().unwrap_or_else(fallback))
}
pub fn plan_artisan_choices(
    w: &dyn WorldRead,
    r: &IncomeRequest,
    opening: bool,
) -> OpResult<ArtisanChoicesPlan> {
    if r.expected_revision != w.campaign().revision || r.period.is_empty() {
        return Err(error("Invalid artisan choice period/revision"));
    }
    serial_date(date(w))?;
    let mut result = ArtisanChoicesPlan {
        operations: vec![],
        enterprises: w.artisans().values().cloned().collect::<Vec<_>>(),
        diagnostics: vec![],
    };
    if opening {
        for e in &mut result.enterprises {
            let Some(pending) = e.pending_retooling.as_mut() else {
                continue;
            };
            let remaining = pending
                .get("remaining_days")
                .and_then(integral_number)
                .ok_or_else(|| error("Invalid retooling countdown"))?
                .checked_sub(7)
                .ok_or_else(|| error("Retooling countdown overflow"))?;
            pending["remaining_days"] = json!(remaining);
            let due = if let Some(ready) = pending.get("ready_date").filter(|v| !v.is_null()) {
                serial_date(date(w))? >= serial_date(serde_json::from_value(ready.clone())?)?
            } else {
                remaining <= 0
            };
            if due {
                let method = actor(&pending["method_id"])?;
                let catalog = w.production_catalog()
                    .as_ref()
                    .ok_or_else(|| error("Retooling catalog missing"))?;
                if !catalog.methods.iter().any(|m| {
                    m.method_id == method
                        && m.kind == "artisan"
                        && m.status == "ready"
                        && m.technology
                            .iter()
                            .all(|t| e.implemented_technologies.contains(t))
                }) {
                    return Err(error("Pending retooling method unavailable"));
                }
                e.base.method_id = method;
                e.pending_retooling = None;
                result.operations.push(serde_json::from_value(json!({"event_id":format!("artisan-retooling-complete:{}:{}",r.period,js_string(&e.base.id.json())),"type":"artisan_retooling_complete","enterprise_id":e.base.id}))?);
            }
        }
        return Ok(result);
    }
    let quotes = build_artisan_quotes(w, r)?;
    let stocks = choice_stocks(w)?;
    let mut quotes_by_place: HashMap<_, Vec<Value>> = HashMap::new();
    for q in quotes {
        quotes_by_place
            .entry((
                crate::transport_validation::province(&q["province_id"])?,
                actor(&q["currency_id"])?,
            ))
            .or_default()
            .push(q);
    }
    let mut session = OperationSession::new(w)?;
    for e in &mut result.enterprises {
        if e.base.extra.get("automatic_choice") != Some(&json!(true)) {
            continue;
        }
        let currency = e
            .base
            .extra
            .get("retooling")
            .and_then(|t| t.get("currency_id"))
            .map(actor)
            .transpose()?;
        let local = currency
            .as_ref()
            .and_then(|c| quotes_by_place.get(&(e.base.province_id, c.clone())))
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        let choice = choose_for_enterprise(w, e, local, None, &session, &stocks)?;
        if choice.method_id == e.base.method_id || choice.retooling.is_none() {
            continue;
        }
        let event: ExternalId = format!(
            "artisan-choice:{}:{}",
            r.period,
            js_string(&e.base.id.json())
        )
        .into();
        let op = stage_retooling(w, e, choice, &mut session, event, false)?;
        result.operations.push(op);
    }
    Ok(result)
}
pub(crate) fn stage_retooling(
    w: &dyn WorldRead,
    e: &mut ArtisanEnterprise,
    choice: ArtisanChoice,
    session: &mut OperationSession<'_>,
    event: ExternalId,
    opening: bool,
) -> OpResult<EconomicOperation> {
    let terms = choice
        .retooling
        .ok_or_else(|| error("Retooling terms missing"))?;
    let owner = e
        .base
        .owner_id
        .as_ref()
        .ok_or_else(|| error("Artisan owner missing"))?;
    let provider = actor(&terms["provider_id"])?;
    let currency = actor(&terms["currency_id"])?;
    let purpose = terms
        .get("purpose")
        .and_then(Value::as_str)
        .filter(|p| !p.is_empty())
        .ok_or_else(|| error("Retooling purpose unresolved"))?;
    let duration = terms
        .get("duration_days")
        .and_then(integral_number)
        .filter(|d| *d >= 1)
        .ok_or_else(|| error("Retooling duration unresolved"))?;
    let cost = as_minor(integer(&terms["cost_minor"])?)?;
    if &provider == owner || cost.0 < 0 || session.cash(owner, &currency, purpose).0 < cost.0 {
        return Err(error("Retooling provider or funding unresolved"));
    }
    let receipts = e
        .base
        .extra
        .entry("retooling_receipts".into())
        .or_insert(json!([]))
        .as_array_mut()
        .ok_or_else(|| error("Invalid artisan retooling receipts"))?;
    if receipts.iter().any(|old| old["event_id"] == event.json()) {
        return Err(error("Orphan retooling receipt"));
    }
    let ready = add_days(
        if opening {
            date(w)
        } else {
            add_days(date(w), 7)?
        },
        duration,
    )?;
    e.pending_retooling = Some(
        json!({"event_id":event,"method_id":choice.method_id,"remaining_days":duration,"ready_date":ready}),
    );
    receipts.push(json!({"event_id":event,"method_id":choice.method_id}));
    let op: EconomicOperation = serde_json::from_value(
        json!({"event_id":event,"type":"artisan_retooling","debits":if cost.0>0 {vec![json!({"owner_id":owner,"currency_id":currency,"purpose":purpose,"amount_minor":cost})]}else{vec![]},"credits":if cost.0>0 {vec![json!({"owner_id":provider,"currency_id":currency,"purpose":"operating_capital","amount_minor":cost})]}else{vec![]} }),
    )?;
    session.stage_income(&op)?;
    Ok(op)
}
