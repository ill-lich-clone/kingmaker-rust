use kingmaker_core::world_read::WorldRead;
use crate::{
    land_paths::{LandPathIndex, PHASE_PATH_CACHE_BYTES}, market_index::OrderIndex,
    market_clearing::*, market_policy::*, operation_session::*,
    storage::*,
};
use kingmaker_core::{market::*, operations::*, *};
use serde_json::{json, Value};
use std::collections::HashMap;
#[derive(Debug, Clone, Copy)]
pub enum FoodStage {
    Early,
    Late,
    Surplus,
}
#[derive(Debug, Default)]
pub struct FoodResult {
    pub operations: Vec<EconomicOperation>,
    pub receipts: Vec<EconomicOperation>,
    pub converted: Vec<Value>,
    pub blocked: Vec<Value>,
}
fn error(s: &str) -> OperationError {
    OperationError(s.into())
}
fn is_food(o: &MarketOrder) -> bool {
    o.need_id.as_ref().and_then(ExternalId::as_str) == Some("food")
}
pub(crate) fn recipe(g: &ExternalId) -> Option<(&'static str, f64)> {
    match g.as_str() {
        Some("grain") => Some(("flour", 1.03)),
        Some("livestock") => Some(("meat", 5.0)),
        _ => None,
    }
}
fn num(v: Option<&Value>) -> f64 {
    crate::needs::finite_or_null(v).unwrap_or(0.0)
}
fn text(id: &ExternalId) -> String {
    crate::needs::js_string(&id.json())
}
impl FoodStage {
    fn name(self) -> &'static str {
        match self {
            Self::Early => "early",
            Self::Late => "late",
            Self::Surplus => "surplus",
        }
    }
}
pub fn food_orders(orders: &[MarketOrder]) -> OpResult<Vec<MarketOrder>> {
    let mut rows = orders.to_vec();
    for o in &mut rows {
        if !is_food(o) {
            continue;
        }
        if o.goods_id.as_str() == Some("grain") {
            o.goods_id = "flour".into();
        }
        let mut alternatives: Vec<Value> = vec![];
        for a in &o.food_alternatives {
            let mut a = a.clone();
            if a.get("goods_id").and_then(Value::as_str) == Some("grain") {
                a["goods_id"] = json!("flour");
            }
            if let Some(old) = alternatives
                .iter_mut()
                .find(|old| old.get("goods_id") == a.get("goods_id"))
            {
                old["share"] = json!(num(old.get("share")) + num(a.get("share")));
                if old.get("H").is_some_and(|v| !v.is_null())
                    && a.get("H").is_some_and(|v| !v.is_null())
                {
                    old["H"] = json!(num(old.get("H")) + num(a.get("H")));
                }
            } else {
                alternatives.push(a);
            }
        }
        o.food_alternatives = alternatives;
    }
    Ok(rows)
}
pub fn remaining_food_orders(
    orders: &[MarketOrder],
    purchases: &[EconomicOperation],
) -> OpResult<Vec<MarketOrder>> {
    let mut eaten = HashMap::new();
    let mut fulfilled = HashMap::new();
    let mut groups = HashMap::new();
    let supply = orders
        .iter()
        .filter(|o| o.goods_id.as_str() == Some("military_supply"))
        .map(|o| (o.pop_id.clone(), o))
        .collect::<HashMap<_, _>>();
    for op in purchases {
        if op.kind != OperationKind::Purchase {
            continue;
        }
        if let Some(id) = op.meta.get("order_id").and_then(Value::as_str) {
            *fulfilled.entry(id.to_string()).or_insert(0.0) += num(op.meta.get("quantity"));
        }
        if op.meta.get("consumed") == Some(&json!(true))
            && op.meta.get("need_id").and_then(Value::as_str) == Some("food")
        {
            let pop = op
                .meta
                .get("pop_id")
                .filter(|v| !v.is_null())
                .map(|v| ExternalId::from_json(v.clone()).map_err(OperationError))
                .transpose()?;
            *eaten.entry(pop).or_insert(0.0) += num(op.meta.get("quantity"));
        }
    }
    for o in orders.iter().filter(|o| is_food(o)) {
        *groups.entry(o.pop_id.clone()).or_insert(0.0) += o.quantity;
    }
    let mut rows = food_orders(orders)?;
    for o in &mut rows {
        if !is_food(o) {
            o.quantity = (o.quantity - fulfilled.get(&o.id).copied().unwrap_or(0.0)).max(0.0);
            continue;
        }
        let total = groups.get(&o.pop_id).copied().unwrap_or(0.0);
        let done = eaten.get(&o.pop_id).copied().unwrap_or(0.0);
        o.quantity = if total > 0.0 {
            o.quantity * (total - done).max(0.0) / total
        } else {
            0.0
        };
        let substitution = o.military_food_substitution.unwrap_or(0.0);
        let covered = substitution
            * supply
                .get(&o.pop_id)
                .filter(|s| s.quantity > 0.0)
                .map(|s| (fulfilled.get(&s.id).copied().unwrap_or(0.0) / s.quantity).min(1.0))
                .unwrap_or(0.0);
        o.food_gross_quantity =
            Some((o.food_gross_quantity.unwrap_or(total + substitution) - done - covered).max(0.0));
        o.military_food_substitution = Some((substitution - covered).max(0.0));
    }
    rows.retain(|o| {
        o.quantity > 1e-10 || is_food(o) && o.food_gross_quantity.unwrap_or(0.0) > 1e-10
    });
    Ok(rows)
}
fn conversion(
    w: &dyn WorldRead,
    period: &str,
    stage: FoodStage,
    raw: &StockpileRecord,
    output: &ExternalId,
    q: f64,
) -> OpResult<EconomicOperation> {
    let (good, k) = recipe(&raw.goods.external_id()).ok_or_else(|| error("Unknown food recipe"))?;
    let unit = w.production_catalog()
        .as_ref()
        .and_then(|c| c.goods.iter().find(|g| g.goods_id.as_str() == Some(good)))
        .and_then(|g| g.extra.get("unit"))
        .filter(|v| !v.is_null())
        .cloned()
        .or_else(|| raw.unit.clone())
        .ok_or_else(|| error("Food output unit missing"))?;
    Ok(serde_json::from_value(
        json!({"event_id":format!("food-conversion:{}",json!([w.campaign().campaign_id,period,stage.name(),raw.external_id])),"type":"food_conversion","province_id":raw.province_id.0,"owner_id":raw.owner_id,"input_goods_id":raw.goods.external_id(),"goods_id":good,"input_quantity":q*k,"output_quantity":q,"coefficient":k,"stage":stage.name(),"stock_changes":[{"stock_id":raw.external_id,"delta":-q*k},{"stock_id":output,"delta":q,"create":{"owner_id":raw.owner_id,"province_id":raw.province_id.0,"goods_id":good,"unit":unit,"currency_id":raw.currency_id,"facility_id":if matches!(stage,FoodStage::Surplus){raw.facility_id.clone()}else{None},"inventory_origin":if raw.inventory_origin.as_deref()==Some("produced"){"produced"}else{"food_conversion"},"conversion_source_stock_id":raw.external_id}}],"causes":["instant authored food conversion","real unreserved raw stock","unchanged seller ownership"]}),
    )?)
}
fn actual_stage(s: &mut OperationSession, op: &EconomicOperation) -> OpResult<()> {
    s.authorize_food_conversion(op)?;
    s.stage(op)?;
    Ok(())
}
fn result_from(s: &OperationSession, start: usize, blocked: Vec<Value>) -> FoodResult {
    let ops = s.operations()[start..].to_vec();
    FoodResult{receipts:ops.iter().filter(|o|o.kind==OperationKind::Purchase).cloned().collect(),converted:ops.iter().filter(|o|o.kind==OperationKind::FoodConversion).map(|o|json!({"input_goods_id":o.meta["input_goods_id"],"input_quantity":o.meta["input_quantity"],"goods_id":o.meta["goods_id"],"output_quantity":o.meta["output_quantity"]})).collect(),operations:ops,blocked}
}
pub(crate) fn plan_food_conversion(
    s: &mut OperationSession,
    period: &str,
    orders: &[MarketOrder],
    stage: FoodStage,
) -> OpResult<FoodResult> {
    let result = (|| {
        if s.base.market_parameters()
            .as_ref()
            .and_then(|p| p.instant_food)
            != Some(true)
        {
            return Err(error("Instant food rule is not enabled"));
        }
        let start = s.operations().len();
        if matches!(stage, FoodStage::Surplus) {
            return surplus(s, period, orders, start);
        }
        let requested = food_orders(
            &orders
                .iter()
                .filter(|o| {
                    is_food(o)
                        || o.goods_id.as_str() == Some("military_supply")
                        || o.maintenance && o.priority == "basic"
                })
                .cloned()
                .collect::<Vec<_>>(),
        )?;
        let ready = plan_foundation_market(
            s,
            &format!("{period}:food:{}:ready", stage.name()),
            &requested,
            &["basic".into()],
        )?;
        let pending = remaining_food_orders(&requested, &ready.operations)?;
        let mut view = MarketView::from_session(s)?;
        let mut sources = HashMap::new();
        let raw = view.stocks.clone();
        for raw in raw {
            crate::run_control::work("food_conversion")?;
            let Some((good, k)) = recipe(&raw.goods.external_id()) else {
                continue;
            };
            let free = view.policy.free(&raw);
            if free <= 1e-10 {
                continue;
            }
            let identity = json!([
                s.base.campaign().campaign_id,
                period,
                stage.name(),
                raw.external_id
            ]);
            let event: ExternalId = format!("food-conversion:{identity}").into();
            if s.committed(&event).is_some() {
                continue;
            }
            let mut out = raw.clone();
            out.external_id = format!("conversion-offer:{identity}").into();
            out.quantity = free / k;
            out.goods = Goods::from_external(good.into());
            out.facility_id = None;
            view.add_virtual_food(&raw, out.clone())?;
            sources.insert(out.external_id, raw);
        }
        let market = plan_local_orders(
            &view,
            &format!("{period}:food:{}:converted", stage.name()),
            &pending,
            &["basic".into()],
        )?;
        let mut used: Vec<(ExternalId, f64)> = vec![];
        for op in &market.operations {
            if op.kind == OperationKind::Purchase {
                for d in &op.stock_changes {
                    if d.delta < 0.0 && sources.contains_key(&d.stock_id) {
                        if let Some((_, q)) = used.iter_mut().find(|(id, _)| id == &d.stock_id) {
                            *q -= d.delta;
                        } else {
                            used.push((d.stock_id.clone(), -d.delta));
                        }
                    }
                }
            }
        }
        s.purchase_policy = Some(view.policy.clone());
        for (id, q) in &used {
            let op = conversion(s.base, period, stage, &sources[id], id, *q)?;
            actual_stage(s, &op)?;
        }
        apply_market_result(s, &market)?;
        s.purchase_policy = None;
        Ok(result_from(s, start, vec![]))
    })();
    if result.is_err() {
        s.poison();
    }
    result
}
fn surplus(
    s: &mut OperationSession,
    period: &str,
    orders: &[MarketOrder],
    start: usize,
) -> OpResult<FoodResult> {
    let policy = MarketPolicy::from_session(s)?;
    let order_index = OrderIndex::new(orders, &policy.scope);
    let mut paths = None;
    let raw_ids = s.stocks()
        .filter(|r| recipe(&r.goods.external_id()).is_some())
        .map(|r| r.external_id.clone()).collect::<Vec<_>>();
    // Lazily constructed to retain the no-eligible-stock path's validation behavior.
    let mut storage = None;
    let mut blocked = vec![];
    for raw_id in raw_ids {
        crate::run_control::work("food_surplus")?;
        let raw = s.stock(&raw_id).ok_or_else(|| error("Surplus source stock missing"))?.clone();
        let Some((good, k)) = recipe(&raw.goods.external_id()) else {
            continue;
        };
        let event: ExternalId = format!(
            "food-conversion:{}",
            json!([
                s.base.campaign().campaign_id,
                period,
                "surplus",
                raw.external_id
            ])
        )
        .into();
        if s.committed(&event).is_some() {
            continue;
        }
        let free = policy.free(&raw);
        if free <= 0.0 {
            continue;
        }
        let mut raw_demand = 0.0;
        for &i in order_index.raw_orders(&policy.scope.key(raw.province_id), &raw.goods.external_id()) {
            let o = &orders[i];
            raw_demand += if o.owner_id == raw.owner_id {
                o.quantity
            } else {
                MarketPolicy::quote(s.base, o)
                    .map(|price| {
                        o.quantity.min(
                            o.currency_id
                                .as_ref()
                                .map(|c| {
                                    s.cash(&o.owner_id, c, &o.purpose).0 as f64 / (price * 100.0)
                                })
                                .unwrap_or(0.0),
                        )
                    })
                    .unwrap_or(0.0)
            };
        }
        let mut food_wanted = f64::INFINITY;
        if raw_demand > 0.0 {
            food_wanted = 0.0;
            for &i in order_index.food_orders(&raw.currency_id, &good.into()) {
                let o = &orders[i];
                let path_length = if policy.scope.same(raw.province_id, o.province_id) {
                    Some(1)
                } else {
                    if paths.is_none() {
                        paths = Some(LandPathIndex::from_territory(s.base.territory().values(), PHASE_PATH_CACHE_BYTES)?);
                    }
                    paths.as_mut().expect("initialized paths")
                        .path(raw.province_id, o.province_id)?.map(|p| p.len())
                };
                if let (Some(path_length), Some(price), Some(currency)) =
                    (path_length, MarketPolicy::quote(s.base, o), &o.currency_id)
                {
                    food_wanted += (o.quantity * path_length as f64)
                        .min(s.cash(&o.owner_id, currency, &o.purpose).0 as f64 / (price * 100.0));
                }
            }
            if food_wanted <= 0.0 {
                blocked
                    .push(json!({"stock_id":raw.external_id,"reason":"other_funded_raw_demand"}));
                continue;
            }
        }
        let Some(facility) = &raw.facility_id else {
            blocked.push(json!({"stock_id":raw.external_id,"reason":"conversion_storage_unknown"}));
            continue;
        };
        if storage.is_none() { storage = Some(StorageBatch::new(s)?); }
        let batch = storage.as_mut().expect("initialized storage batch");
        let product: ExternalId = good.into();
        let availability = batch.availability(facility, &product);
        let raw_space = batch
            .facilities
            .get(facility)
            .and_then(|f| f.goods.get(&text(&raw.goods.external_id())))
            .map(|m| m.space_per_unit);
        let out_space = batch
            .facilities
            .get(facility)
            .and_then(|f| f.goods.get(good))
            .map(|m| m.space_per_unit);
        let (Some(raw_space), Some(out_space), Some(space)) =
            (raw_space, out_space, availability.free_space)
        else {
            blocked.push(json!({"stock_id":raw.external_id,"reason":"conversion_storage_unknown"}));
            continue;
        };
        let net = out_space - k * raw_space;
        let max = ((free - raw_demand).max(0.0) / k).min(food_wanted);
        let q = if net > 0.0 { max.min(space / net) } else { max };
        if q <= 1e-10 {
            blocked.push(json!({"stock_id":raw.external_id,"reason":"storage_full"}));
            continue;
        }
        let checkpoint = batch.checkpoint_facility(facility)?;
        batch.adjust(facility, -q * k * raw_space)?;
        let outcome = reserve_storage(
            s,
            batch,
            &StorageRequest {
                id: event.clone(),
                owner_id: raw.owner_id.clone(),
                facility_id: facility.clone(),
                goods_id: product,
                unit: availability.unit.unwrap_or(Value::Null),
                quantity: q,
            },
            period,
        )?;
        batch.restore_facility(checkpoint);
        let Some(row) = outcome.reservation else {
            blocked.push(json!({"id":event,"reason":outcome.reason}));
            continue;
        };
        // Rent may fund less than q. Only actually consumed raw frees space.
        batch.adjust(facility, row.quantity * net)?;
        let id: ExternalId = format!("converted-surplus:{}", json!([raw.external_id, good])).into();
        s.purchase_policy = Some(policy.clone());
        let op = conversion(s.base, period, FoodStage::Surplus, &raw, &id, row.quantity)?;
        actual_stage(s, &op)?;
        s.purchase_policy = None;
        let mut row = row;
        row.consumed_quantity = Some(row.quantity);
        row.quantity = 0.0;
        row.status = "released".into();
        s.storage_rows.insert(row.id.clone(), row);
    }
    Ok(result_from(s, start, blocked))
}
