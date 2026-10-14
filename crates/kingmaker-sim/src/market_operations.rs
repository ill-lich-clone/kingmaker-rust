//! Concrete private authorization for real market operations; no external stock-write capability.
use kingmaker_core::world_read::WorldRead;
use crate::operation_session::{OpResult, OperationError, OperationSession};
use crate::{
    market_policy::MarketPolicy,
    money_math::{decimal_ratio, round_ratio},
    operation_session::{id_meta, number_meta},
};
use kingmaker_core::operations::{MoneyLeg, OperationKind};
use kingmaker_core::{market::MarketOrder, operations::EconomicOperation, ExternalId, Money};
use num_bigint::BigInt;
use num_traits::ToPrimitive;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone)]
pub(crate) struct SaleAllocation {
    pub stock_id: ExternalId,
    pub quantity: f64,
}
#[derive(Debug, Clone)]
pub(crate) struct PurchaseEvidence {
    pub budget_minor: Money,
    pub allocations: Vec<SaleAllocation>,
}
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct MarketReadSet { hashes: Option<Vec<[u8;32]>> }
impl MarketReadSet {
    pub(crate) fn capture(w:&dyn WorldRead)->Self {
        let hashes=(|| -> OpResult<Vec<[u8;32]>> { Ok(vec![
            crate::read_guard::fingerprint(&w.pops())?,
            crate::read_guard::fingerprint(&w.stockpiles())?,
            crate::read_guard::fingerprint(&w.accounts())?,
            crate::read_guard::fingerprint(&w.resource_operations())?,
            crate::read_guard::fingerprint(&w.factories())?,
            crate::read_guard::fingerprint(&w.artisans())?,
            crate::read_guard::fingerprint(&w.expansions())?,
            crate::read_guard::fingerprint(&w.construction_projects())?,
            crate::read_guard::fingerprint(&w.completed_construction())?,
            crate::read_guard::fingerprint(&w.storage_facilities())?,
            crate::read_guard::fingerprint(&w.storage_reservations())?,
            crate::read_guard::fingerprint(&w.goods_reservations())?,
            crate::read_guard::fingerprint(&w.money_reservations())?,
            crate::read_guard::fingerprint(&w.territory())?,
            crate::read_guard::fingerprint(&w.maintenance_agreements())?,
            crate::read_guard::fingerprint(&w.sale_offers())?,
            crate::read_guard::fingerprint(&w.institutions())?,
            crate::read_guard::fingerprint(&w.market_construction_catalog())?,
            crate::read_guard::fingerprint(&w.all_production_phase_receipts())?,
            crate::read_guard::fingerprint(&w.all_market_phase_receipts())?,
            crate::read_guard::fingerprint(&w.needs())?,
            crate::read_guard::fingerprint(&w.price_calibration())?,
            crate::read_guard::fingerprint(&w.market_parameters())?,
            crate::read_guard::fingerprint(&w.consumer_facility_by_province())?,
        ]) })().ok();
        Self {hashes}
    }
    pub(crate) fn unchanged(&self,w:&dyn WorldRead)->bool { self.hashes.is_some() && self.hashes==Self::capture(w).hashes }
}
fn error(s: &str) -> OperationError {
    OperationError(s.into())
}
pub(crate) fn invoice(quantity: f64, price: f64) -> OpResult<BigInt> {
    if !quantity.is_finite() || quantity < 0.0 || !price.is_finite() || price <= 0.0 {
        return Err(error("Invalid market invoice quantity/price"));
    }
    let (qn, qd) = decimal_ratio(&crate::needs::js_string(&json!(quantity)))?;
    let (pn, pd) = decimal_ratio(&crate::needs::js_string(&json!(price)))?;
    Ok(round_ratio(&(qn * pn * BigInt::from(100)), &(qd * pd))?)
}
fn close(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-9 * a.abs().max(b.abs()).max(1.0)
}
pub(crate) fn validate_creation_headers(op: &EconomicOperation) -> OpResult<()> {
    let headers = [
        "owner_id",
        "province_id",
        "goods_id",
        "unit",
        "facility_id",
        "currency_id",
        "inventory_origin",
        "method_kind",
    ];
    if op
        .stock_changes
        .iter()
        .filter_map(|d| d.create.as_ref())
        .any(|c| headers.iter().any(|key| c.extra.contains_key(*key)))
    {
        return Err(error("Creation metadata shadows typed stock title/headers"));
    }
    Ok(())
}
pub(crate) fn purchased_stock_id(order: &MarketOrder) -> ExternalId {
    let facility = order
        .facility_id
        .as_ref()
        .map(ExternalId::json)
        .filter(crate::needs::js_truthy)
        .unwrap_or(Value::Null);
    let mut key = vec![
        order.owner_id.json(),
        json!(order.province_id.0.to_string()),
        order.goods_id.json(),
        order
            .currency_id
            .as_ref()
            .map(ExternalId::json)
            .unwrap_or(Value::Null),
        facility,
    ];
    if let Some(purpose) = order
        .extra
        .get("inventory_purpose")
        .filter(|v| crate::needs::js_truthy(v))
    {
        key.push(purpose.clone());
        key.push(
            order
                .pop_id
                .as_ref()
                .map(ExternalId::json)
                .filter(crate::needs::js_truthy)
                .unwrap_or(Value::Null),
        );
    }
    format!("purchased:{}", json!(key)).into()
}
pub(crate) fn validate_purchase(
    s: &OperationSession,
    op: &EconomicOperation,
    order: &MarketOrder,
    proof: &PurchaseEvidence,
) -> OpResult<()> {
    validate_creation_headers(op)?;
    if op.kind != OperationKind::Purchase || s.period.is_none() || op.obligation.is_some() {
        return Err(error("Purchase requires configured current phase"));
    }
    let prefix = format!(
        "market:{}:{}",
        s.base.campaign().campaign_id,
        s.period.as_deref().unwrap()
    );
    if !op
        .event_id
        .as_str()
        .is_some_and(|id| id.starts_with(&(prefix + ":")))
        || op.meta.get("order_id").and_then(Value::as_str) != Some(order.id.as_str())
    {
        return Err(error("Purchase event/order period mismatch"));
    }
    let currency = order
        .currency_id
        .as_ref()
        .ok_or_else(|| error("Purchase currency absent"))?;
    if id_meta(op, "currency_id")? != *currency
        || id_meta(op, "goods_id")? != order.goods_id
        || kingmaker_core::ProvinceId::from_external(&id_meta(op, "province_id")?)
            .map_err(OperationError)?
            != order.province_id
        || op.meta.get("consumed").and_then(Value::as_bool) != Some(order.consume)
        || op
            .meta
            .get("maintenance")
            .and_then(Value::as_bool)
            .unwrap_or(false)
            != order.maintenance
    {
        return Err(error("Purchase does not match order identity/consumption"));
    }
    if op
        .meta
        .get("pop_id")
        .filter(|v| !v.is_null())
        .map(|v| ExternalId::from_json(v.clone()))
        .transpose()
        .map_err(OperationError)?
        != order.pop_id
        || op
            .meta
            .get("need_id")
            .filter(|v| !v.is_null())
            .map(|v| ExternalId::from_json(v.clone()))
            .transpose()
            .map_err(OperationError)?
            != order.need_id
        || op.meta.get("priority").and_then(Value::as_str) != Some(order.priority.as_str())
    {
        return Err(error("Purchase POP/need/priority mismatch"));
    }
    let quantity = number_meta(op, "quantity")?;
    let requested = number_meta(op, "requested_quantity")?;
    if quantity <= 0.0
        || !close(requested, order.quantity)
        || quantity > order.quantity + 1e-9
        || proof.budget_minor.0 < 0
    {
        return Err(error("Invalid purchase quantity/budget"));
    }
    let generated;
    let policy = if let Some(policy) = s.purchase_policy.as_ref() {
        policy
    } else {
        generated = MarketPolicy::from_session(s)?;
        &generated
    };
    let price = MarketPolicy::quote(s.base, order)
        .ok_or_else(|| error("Unknown positive currency quote"))?;
    let mut ids = HashSet::new();
    let mut allocations = HashMap::new();
    let mut total = 0.0;
    let mut own_only = true;
    let mut unit = None;
    for a in &proof.allocations {
        if !a.quantity.is_finite() || a.quantity <= 0.0 || !ids.insert(&a.stock_id) {
            return Err(error("Invalid/duplicate seller allocation"));
        }
        let stock = s
            .stock(&a.stock_id)
            .ok_or_else(|| error("Purchase seller stock missing"))?;
        let can_settle = stock.owner_id == order.owner_id
            || stock.currency_id.as_ref() == Some(currency)
            || s.accounts()
                .any(|a| a.owner_id == stock.owner_id && &a.currency_id == currency);
        if stock.goods.external_id() != order.goods_id
            || !policy.scope.same(stock.province_id, order.province_id)
            || !policy.can_sell(stock, order)
            || !can_settle
            || a.quantity > policy.free(stock) + 1e-9
        {
            return Err(error("Seller stock is unavailable/held/unauthorized"));
        }
        let u = stock
            .unit
            .as_ref()
            .filter(|u| !u.is_null() && !crate::needs::undefined(u))
            .ok_or_else(|| error("Unknown seller commodity unit"))?;
        if unit.as_ref().is_some_and(|old| old != u) {
            return Err(error("Mixed seller commodity units"));
        }
        unit = Some(u.clone());
        if let Some(catalog) = &s.base.production_catalog() {
            if let Some(authored) = catalog
                .goods
                .iter()
                .find(|g| g.goods_id == order.goods_id)
                .and_then(|g| g.extra.get("unit"))
            {
                if authored != u {
                    return Err(error("Seller commodity catalogue unit mismatch"));
                }
            }
        }
        own_only &= stock.owner_id == order.owner_id;
        total += a.quantity;
        allocations.insert(a.stock_id.clone(), a.quantity);
    }
    if proof.allocations.is_empty() || !close(total, quantity) {
        return Err(error("Seller allocation quantity mismatch"));
    }
    let requested_minor = invoice(order.quantity, price)?;
    let actual = invoice(quantity, price)?;
    let cost = if own_only {
        Money(0)
    } else {
        if requested_minor > BigInt::from(0) {
            let r = requested_minor
                .to_f64()
                .ok_or_else(|| error("Unrepresentable requested budget ratio"))?;
            let wanted = order.quantity * proof.budget_minor.0 as f64 / r;
            if !wanted.is_finite() || quantity > wanted + 1e-9 {
                return Err(error("Purchase exceeded proportional buyer budget"));
            }
        }
        Money(
            actual
                .min(BigInt::from(proof.budget_minor.0))
                .to_i64()
                .ok_or_else(|| error("Invoice overflow"))?,
        )
    };
    let paid: Money = serde_json::from_value(
        op.meta
            .get("paid_minor")
            .cloned()
            .ok_or_else(|| error("Purchase paid_minor missing"))?,
    )?;
    if paid != cost {
        return Err(error("Purchase invoice/paid mismatch"));
    }
    let debits = if cost.0 > 0 {
        vec![MoneyLeg {
            owner_id: order.owner_id.clone(),
            currency_id: currency.clone(),
            purpose: order.purpose.clone(),
            amount_minor: cost,
        }]
    } else {
        vec![]
    };
    if op.debits != debits {
        return Err(error("Purchase buyer debit mismatch"));
    }
    let collation = crate::ordering::collator();
    let mut sellers = proof.allocations.clone();
    sellers.sort_by(|a, b| {
        collation.compare(
            &crate::needs::js_string(&a.stock_id.json()),
            &crate::needs::js_string(&b.stock_id.json()),
        )
    });
    let weights = sellers
        .iter()
        .map(|a| BigInt::from((a.quantity / quantity * 1_000_000_000.0).round() as i64))
        .collect::<Vec<_>>();
    let weight_sum = weights.iter().sum::<BigInt>();
    let mut used = 0i64;
    let mut credits = Vec::new();
    if cost.0 > 0 {
        if weight_sum <= BigInt::from(0) {
            return Err(error("Empty seller invoice weights"));
        }
        for (i, a) in sellers.iter().enumerate() {
            let amount = if i + 1 == sellers.len() {
                cost.0 - used
            } else {
                (BigInt::from(cost.0) * &weights[i] / &weight_sum)
                    .to_i64()
                    .ok_or_else(|| error("Seller allocation overflow"))?
            };
            used += amount;
            let stock = s.stock(&a.stock_id).unwrap();
            credits.push(MoneyLeg {
                owner_id: stock.owner_id.clone(),
                currency_id: currency.clone(),
                purpose: policy.seller_purpose(stock),
                amount_minor: Money(amount),
            });
        }
    }
    if op.credits != credits {
        return Err(error("Purchase actual seller credits mismatch"));
    }
    let mut negative = HashMap::new();
    let mut positive = Vec::new();
    for d in &op.stock_changes {
        if d.delta < 0.0 {
            if negative.insert(&d.stock_id, -d.delta).is_some() {
                return Err(error("Duplicate purchase stock debit"));
            }
        } else if d.delta > 0.0 {
            positive.push(d);
        } else {
            return Err(error("Zero purchase stock change"));
        }
    }
    if negative.len() != allocations.len()
        || allocations
            .iter()
            .any(|(id, q)| negative.get(id).is_none_or(|actual| !close(*actual, *q)))
    {
        return Err(error("Purchase stock debits mismatch"));
    }
    if order.consume {
        if !positive.is_empty() {
            return Err(error("Consumed purchase created stored goods"));
        }
    } else {
        if positive.len() != 1 || !close(positive[0].delta, quantity) {
            return Err(error("Stored purchase output mismatch"));
        }
        let c = positive[0]
            .create
            .as_ref()
            .ok_or_else(|| error("Purchased stock lacks explicit creation terms"))?;
        if positive[0].stock_id != purchased_stock_id(order) {
            return Err(error("Purchased stock ID does not belong to buyer target"));
        }
        if c.owner_id != order.owner_id
            || c.province_id != order.province_id
            || c.goods_id != order.goods_id
            || c.unit != unit.unwrap()
            || c.currency_id.as_ref() != Some(currency)
            || c.facility_id != order.facility_id
            || c.facility_id.is_none()
            || c.inventory_origin != "purchased"
            || c.method_kind.is_some()
        {
            return Err(error("Purchased stock title/unit/facility mismatch"));
        }
        let intended = order
            .extra
            .get("inventory_purpose")
            .filter(|v| crate::needs::js_truthy(v));
        if c.extra.get("inventory_purpose") != intended
            || c.extra.get("beneficiary_pop_id")
                != intended
                    .and(order.pop_id.as_ref())
                    .map(ExternalId::json)
                    .as_ref()
        {
            return Err(error("Purchased reserve purpose/beneficiary mismatch"));
        }
        if let Some(existing) = s.stock(&positive[0].stock_id) {
            if existing.owner_id != c.owner_id
                || existing.province_id != c.province_id
                || existing.goods.external_id() != c.goods_id
                || existing.unit.as_ref() != Some(&c.unit)
                || existing.currency_id != c.currency_id
                || existing.facility_id != c.facility_id
                || existing.purpose != intended.and_then(Value::as_str).unwrap_or("commercial")
                || existing.beneficiary_pop_id != intended.and(order.pop_id.as_ref()).cloned()
            {
                return Err(error("Existing purchased stock title/beneficiary mismatch"));
            }
        }
        if !s.storage_reservations().any(|r| {
            r.owner_id == order.owner_id
                && Some(&r.facility_id) == order.facility_id.as_ref()
                && r.goods_id == order.goods_id
                && r.status == "held"
                && r.period.as_deref().is_some_and(|period| {
                    op.event_id.as_str().is_some_and(|event| {
                        event.starts_with(&format!(
                            "market:{}:{period}:phase",
                            s.base.campaign().campaign_id
                        ))
                    })
                })
                && r.quantity + 1e-9 >= quantity
                && r.id.as_str().is_some_and(|id| {
                    id.starts_with("market-storage:") && id.ends_with(&format!(":{}", order.id))
                })
        }) {
            return Err(error("Stored purchase lacks paid destination reservation"));
        }
    }
    Ok(())
}
pub(crate) fn validate_food_conversion(
    s: &OperationSession,
    op: &EconomicOperation,
) -> OpResult<()> {
    use kingmaker_core::market::MarketStage;
    validate_creation_headers(op)?;
    let request = s
        .market_request
        .as_ref()
        .ok_or_else(|| error("Food conversion requires configured market phase"))?;
    if op.kind != OperationKind::FoodConversion
        || !op.debits.is_empty()
        || !op.credits.is_empty()
        || op.obligation.is_some()
        || op.stock_changes.len() != 2
    {
        return Err(error(
            "Food conversion cannot move money or unrelated goods",
        ));
    }
    let input = op
        .stock_changes
        .iter()
        .find(|d| d.delta < 0.0)
        .ok_or_else(|| error("Food raw debit absent"))?;
    let output = op
        .stock_changes
        .iter()
        .find(|d| d.delta > 0.0)
        .ok_or_else(|| error("Food output absent"))?;
    if input.stock_id == output.stock_id || input.create.is_some() {
        return Err(error("Food input/output stock identity invalid"));
    }
    let raw = s
        .stock(&input.stock_id)
        .ok_or_else(|| error("Food raw stock absent"))?;
    let good = raw.goods.external_id();
    let (product, coefficient) = match good.as_str() {
        Some("grain") => (ExternalId::from("flour"), 1.03),
        Some("livestock") => (ExternalId::from("meat"), 5.0),
        _ => return Err(error("Unknown food conversion recipe")),
    };
    let stage = op
        .meta
        .get("stage")
        .and_then(Value::as_str)
        .ok_or_else(|| error("Food conversion stage absent"))?;
    let period = match (request.stage, stage) {
        (MarketStage::Maintenance, "early") => format!("{}:maintenance", request.period),
        (MarketStage::EarlyFood, "early") | (MarketStage::ClosingMarket, "late" | "surplus") => {
            request.period.clone()
        }
        _ => return Err(error("Food conversion stage does not belong to request")),
    };
    let identity = json!([s.base.campaign().campaign_id, period, stage, input.stock_id]);
    if op.event_id != ExternalId::from(format!("food-conversion:{identity}")) {
        return Err(error("Food conversion event identity mismatch"));
    }
    let quantity = number_meta(op, "output_quantity")?;
    if quantity <= 0.0
        || !close(output.delta, quantity)
        || !close(input.delta, -quantity * coefficient)
        || !close(number_meta(op, "input_quantity")?, quantity * coefficient)
        || number_meta(op, "coefficient")? != coefficient
        || id_meta(op, "input_goods_id")? != good
        || id_meta(op, "goods_id")? != product
        || id_meta(op, "owner_id")? != raw.owner_id
        || kingmaker_core::ProvinceId::from_external(&id_meta(op, "province_id")?)
            .map_err(OperationError)?
            != raw.province_id
    {
        return Err(error(
            "Food raw debit/output does not implement authored recipe",
        ));
    }
    let generated;
    let policy = if let Some(policy) = s.purchase_policy.as_ref() {
        policy
    } else {
        generated = MarketPolicy::from_session(s)?;
        &generated
    };
    if quantity * coefficient > policy.free(raw) + 1e-9 {
        return Err(error(
            "Food conversion consumed held/protected/raw shortage",
        ));
    }
    let expected_id = if stage == "surplus" {
        format!("converted-surplus:{}", json!([input.stock_id, product]))
    } else {
        format!("conversion-offer:{identity}")
    };
    if output.stock_id != ExternalId::from(expected_id) {
        return Err(error("Food output stock ID mismatch"));
    }
    let c = output
        .create
        .as_ref()
        .ok_or_else(|| error("Food output creation metadata absent"))?;
    let raw_unit = raw
        .unit
        .as_ref()
        .filter(|u| !u.is_null() && !crate::needs::undefined(u))
        .ok_or_else(|| error("Unknown raw food commodity unit"))?;
    let catalog_unit = |id: &ExternalId| {
        s.base.production_catalog()
            .as_ref()
            .and_then(|cat| cat.goods.iter().find(|g| &g.goods_id == id))
            .and_then(|g| g.extra.get("unit"))
            .filter(|v| !v.is_null() && !crate::needs::undefined(v))
    };
    if catalog_unit(&good).is_some_and(|u| u != raw_unit) {
        return Err(error("Raw food catalogue unit mismatch"));
    }
    let unit = catalog_unit(&product).unwrap_or(raw_unit);
    let facility = if stage == "surplus" {
        raw.facility_id.clone()
    } else {
        None
    };
    let origin = if raw.inventory_origin.as_deref() == Some("produced") {
        "produced"
    } else {
        "food_conversion"
    };
    if c.owner_id != raw.owner_id
        || c.province_id != raw.province_id
        || c.goods_id != product
        || &c.unit != unit
        || c.facility_id != facility
        || c.currency_id != raw.currency_id
        || c.method_kind.is_some()
        || c.inventory_origin != origin
        || c.extra.get("conversion_source_stock_id") != Some(&input.stock_id.json())
    {
        return Err(error(
            "Food conversion changed source title/unit/currency/origin",
        ));
    }
    if let Some(existing) = s.stock(&output.stock_id) {
        if existing.owner_id != c.owner_id
            || existing.province_id != c.province_id
            || existing.goods.external_id() != c.goods_id
            || existing.unit.as_ref() != Some(&c.unit)
            || existing.currency_id != c.currency_id
            || existing.facility_id != c.facility_id
        {
            return Err(error("Food output ID points at another owned stock"));
        }
    }
    if stage == "surplus"
        && !s.storage_reservations().any(|r| {
            r.id == op.event_id
                && r.owner_id == raw.owner_id
                && r.goods_id == product
                && Some(&r.facility_id) == raw.facility_id.as_ref()
                && r.status == "held"
                && r.quantity + 1e-9 >= quantity
        })
    {
        return Err(error(
            "Surplus food conversion lacks real warehouse reservation",
        ));
    }
    Ok(())
}

#[cfg(test)]
#[path = "../tests/common/market.rs"]
mod fixture;
#[cfg(test)]
mod tests {
    use super::*;
    use fixture::*;
    use kingmaker_core::market::*;
    use serde_json::json;
    fn request() -> MarketRequest {
        MarketRequest {
            expected_revision: 20,
            period: "w".into(),
            stage: MarketStage::ClosingMarket,
            profile: Default::default(),
            priorities: None,
        }
    }
    fn purchase() -> EconomicOperation {
        serde_json::from_value(json!({"event_id":"market:market-fixture:w:phase1:o","type":"purchase","order_id":"o","pop_id":"a","province_id":1,"goods_id":"grain","quantity":3,"requested_quantity":3,"consumed":true,"paid_minor":"300","currency_id":"COIN","priority":"basic","need_id":"food","maintenance":false,"debits":[{"owner_id":"a","currency_id":"COIN","purpose":"personal_savings","amount_minor":"300"}],"credits":[{"owner_id":"seller","currency_id":"COIN","purpose":"employer_operating_capital","amount_minor":"300"}],"stock_changes":[{"stock_id":"grain:2","delta":-3}]})).unwrap()
    }
    #[test]
    fn valid_purchase_credits_actual_seller_and_stale_read_set_rejects() {
        let mut w = market_world();
        let mut s = OperationSession::new(&w).unwrap();
        s.configure_market(&request()).unwrap();
        let op = purchase();
        let proof = PurchaseEvidence {
            budget_minor: Money(300),
            allocations: vec![SaleAllocation {
                stock_id: "grain:2".into(),
                quantity: 3.0,
            }],
        };
        s.authorize_purchase(&op, &order("o", "a", "grain", 3.0, "basic", true), &proof)
            .unwrap();
        s.stage(&op).unwrap();
        let prepared = s.finish().unwrap();
        let before = w.clone();
        w.market_data
            .price_calibration
            .as_mut()
            .unwrap()
            .prices_by_currency
            .get_mut("COIN")
            .unwrap()
            .insert("grain".into(), json!(2));
        let changed = w.clone();
        assert!(crate::operation_session::apply_prepared(&mut w, prepared).is_err());
        assert_eq!(w, changed);
        assert_eq!(w.accounts, before.accounts);
    }
    #[test]
    fn valid_purchase_publishes_conserved_cash_and_real_consumption() {
        let mut w = market_world();
        let mut s = OperationSession::new(&w).unwrap();
        s.configure_market(&request()).unwrap();
        let op = purchase();
        let proof = PurchaseEvidence {
            budget_minor: Money(300),
            allocations: vec![SaleAllocation {
                stock_id: "grain:2".into(),
                quantity: 3.0,
            }],
        };
        s.authorize_purchase(&op, &order("o", "a", "grain", 3.0, "basic", true), &proof)
            .unwrap();
        s.stage(&op).unwrap();
        let prepared = s.finish().unwrap();
        crate::operation_session::apply_prepared(&mut w, prepared).unwrap();
        assert_eq!(w.stockpiles[0].quantity, 17.0);
        assert_eq!(
            w.accounts
                .iter()
                .find(|a| a.owner_id.as_str() == Some("seller"))
                .unwrap()
                .balance,
            Money(300)
        );
        assert_eq!(w.accounts.iter().map(|a| a.balance.0).sum::<i64>(), 20000);
    }
    fn conversion() -> EconomicOperation {
        serde_json::from_value(json!({"event_id":"food-conversion:[\"market-fixture\",\"w\",\"late\",\"grain:2\"]","type":"food_conversion","province_id":2,"owner_id":"seller","input_goods_id":"grain","goods_id":"flour","input_quantity":1.03,"output_quantity":1,"coefficient":1.03,"stage":"late","stock_changes":[{"stock_id":"grain:2","delta":-1.03},{"stock_id":"conversion-offer:[\"market-fixture\",\"w\",\"late\",\"grain:2\"]","delta":1,"create":{"owner_id":"seller","province_id":2,"goods_id":"flour","unit":"t","currency_id":"COIN","inventory_origin":"food_conversion","conversion_source_stock_id":"grain:2"}}]})).unwrap()
    }
    #[test]
    fn actual_food_conversion_uses_real_raw_preserves_owner_and_has_no_factory() {
        let mut w = market_world();
        let mut s = OperationSession::new(&w).unwrap();
        s.configure_market(&request()).unwrap();
        let op = conversion();
        s.authorize_food_conversion(&op).unwrap();
        s.stage(&op).unwrap();
        let prepared = s.finish().unwrap();
        crate::operation_session::apply_prepared(&mut w, prepared).unwrap();
        assert_close(w.stockpiles[0].quantity, 18.97, 1e-9);
        let output = w
            .stockpiles
            .iter()
            .find(|s| s.goods == kingmaker_core::Goods::Flour)
            .unwrap();
        assert_eq!(output.quantity, 1.0);
        assert!(output.facility_id.is_none());
        assert!(output.method_kind.is_none());
        assert_eq!(output.owner_id, ExternalId::from("seller"));
    }
    #[test]
    fn wrong_food_recipe_poisoned_phase_has_no_side_effects() {
        let w = market_world();
        let mut s = OperationSession::new(&w).unwrap();
        s.configure_market(&request()).unwrap();
        let mut op = conversion();
        op.stock_changes[0].delta = -1.0;
        assert!(s.authorize_food_conversion(&op).is_err());
        assert!(s.finish().is_err());
    }
    #[test]
    fn stored_food_reserve_retains_beneficiary_and_does_not_increase_foreign_stock() {
        let mut w = market_world();
        let mut s = OperationSession::new(&w).unwrap();
        s.configure_market(&request()).unwrap();
        let mut buyer = order("o", "a", "grain", 3.0, "basic", false);
        buyer.purpose = "personal_savings".into();
        buyer.pop_id = Some("a".into());
        buyer.need_id = Some("food".into());
        buyer
            .extra
            .insert("inventory_purpose".into(), json!("food_reserve"));
        let mut batch = crate::storage::StorageBatch::new(&s).unwrap();
        crate::storage::reserve_storage(
            &mut s,
            &mut batch,
            &crate::storage::StorageRequest {
                id: "market-storage:w:o".into(),
                owner_id: "a".into(),
                facility_id: "wh:a".into(),
                goods_id: "grain".into(),
                unit: json!("t"),
                quantity: 3.0,
            },
            "w",
        )
        .unwrap();
        let mut op = purchase();
        op.meta.insert("consumed".into(), json!(false));
        op.stock_changes.push(serde_json::from_value(json!({"stock_id":"purchased:[\"a\",\"1\",\"grain\",\"COIN\",\"wh:a\",\"food_reserve\",\"a\"]","delta":3,"create":{"owner_id":"a","province_id":1,"goods_id":"grain","unit":"t","facility_id":"wh:a","currency_id":"COIN","inventory_origin":"purchased","inventory_purpose":"food_reserve","beneficiary_pop_id":"a"}})).unwrap());
        let proof = PurchaseEvidence {
            budget_minor: Money(300),
            allocations: vec![SaleAllocation {
                stock_id: "grain:2".into(),
                quantity: 3.0,
            }],
        };
        s.authorize_purchase(&op, &buyer, &proof).unwrap();
        s.stage(&op).unwrap();
        let prepared = s.finish().unwrap();
        crate::operation_session::apply_prepared(&mut w, prepared).unwrap();
        let received = w
            .stockpiles
            .iter()
            .find(|s| s.owner_id.as_str() == Some("a"))
            .unwrap();
        assert_eq!(received.purpose, "food_reserve");
        assert_eq!(received.beneficiary_pop_id, Some("a".into()));
    }
    #[test]
    fn metadata_shadow_budget_and_held_or_personal_stock_cannot_authorize_purchase() {
        for bad in ["budget", "held", "personal", "unit", "invoice"] {
            let mut w = market_world();
            if bad == "held" {
                w.production_data.goods_reservations.push(
                    serde_json::from_value(
                        json!({"id":"hold","stock_id":"grain:2","quantity":19,"status":"held"}),
                    )
                    .unwrap(),
                );
            }
            if bad == "personal" {
                w.stockpiles[0].owner_id = "b".into();
            }
            if bad == "unit" {
                w.stockpiles[0].unit = Some(json!("kg"));
            }
            let mut s = OperationSession::new(&w).unwrap();
            s.configure_market(&request()).unwrap();
            let mut op = purchase();
            if bad == "invoice" {
                op.meta.insert("paid_minor".into(), json!("1"));
            }
            let proof = PurchaseEvidence {
                budget_minor: Money(if bad == "budget" { 1 } else { 300 }),
                allocations: vec![SaleAllocation {
                    stock_id: "grain:2".into(),
                    quantity: 3.0,
                }],
            };
            assert!(
                s.authorize_purchase(&op, &order("o", "a", "grain", 3.0, "basic", true), &proof)
                    .is_err(),
                "accepted {bad}"
            );
            assert!(s.finish().is_err());
        }
    }
    #[test]
    fn added_currency_or_stock_invalidates_prepared_market() {
        let mut w = market_world();
        let mut s = OperationSession::new(&w).unwrap();
        s.configure_market(&request()).unwrap();
        let op = purchase();
        let proof = PurchaseEvidence {
            budget_minor: Money(300),
            allocations: vec![SaleAllocation {
                stock_id: "grain:2".into(),
                quantity: 3.0,
            }],
        };
        s.authorize_purchase(&op, &order("o", "a", "grain", 3.0, "basic", true), &proof)
            .unwrap();
        s.stage(&op).unwrap();
        let prepared = s.finish().unwrap();
        let mut stock = w.stockpiles[0].clone();
        stock.id = 2;
        stock.external_id = "new eligible stock".into();
        w.stockpiles.push(stock);
        let before = w.clone();
        assert!(crate::operation_session::apply_prepared(&mut w, prepared).is_err());
        assert_eq!(w, before);
    }
    #[test]
    fn nested_creation_metadata_cannot_shadow_title_or_headers() {
        let w = market_world();
        let mut s = OperationSession::new(&w).unwrap();
        s.configure_market(&request()).unwrap();
        let mut op = conversion();
        op.stock_changes[1]
            .create
            .as_mut()
            .unwrap()
            .extra
            .insert("owner_id".into(), json!("someone else"));
        assert!(s.authorize_food_conversion(&op).is_err());
        assert!(s.finish().is_err());
    }
}
