//! Autonomous supply derives its requests from the authenticated native market, never raw orders.
use kingmaker_core::world_read::WorldRead;
use crate::{
    budget_funding::error,
    market_policy::MarketScope,
    operation_history::operation_evidence,
    operation_session::{canonical_operation, OpResult},
    shipment_dispatch::{DispatchSession, ShipmentOrder},
    transport_validation::{actor, province, RouteValidation},
};
use kingmaker_core::{calendar::GameDate, market::*, operations::*, transport::*, *};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
type PipelineKey = (
    ExternalId,
    ExternalId,
    ProvinceId,
    ExternalId,
    String,
    Option<ExternalId>,
);
pub struct PreparedPhysicalSupply {
    reads: Option<crate::world_transaction::PrepareProof>,
    state: Option<crate::world_transaction::PreparedWorldDelta>,
    receipt: TransportPhaseReceipt,
}
fn digest(v: &Value) -> OpResult<String> {
    Ok(Sha256::digest(serde_json::to_vec(v)?)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}
fn boundary(w: &dyn WorldRead, r: &TransportRequest) -> String {
    format!(
        "supply-phase:{}:{}:dispatch",
        w.campaign().campaign_id, r.period
    )
}
pub(crate) fn inputs(
    w: &dyn WorldRead,
    r: &TransportRequest,
) -> OpResult<(
    Vec<MarketOrder>,
    Vec<EconomicOperation>,
    Vec<OperationEvidence>,
)> {
    let arrivals = crate::shipment_arrivals::replay(
        w,
        &TransportRequest {
            stage: TransportStage::Arrivals,
            ..r.clone()
        },
    )?
    .ok_or_else(|| error("Supply requires same-period native arrivals"))?;
    let markets = w.all_market_phase_receipts().select(|p| p.request.period == r.period)?;
    if markets
        .iter()
        .filter(|p| p.request.stage == MarketStage::ClosingMarket)
        .count()
        != 1
    {
        return Err(error("Supply requires one closing market receipt"));
    }
    let mut orders = vec![];
    let mut operations = vec![];
    let mut seen = HashSet::new();
    let mut evidence = arrivals.operation_evidence;
    for p in markets {
        if p.campaign_id != w.campaign().campaign_id
            || p.request.expected_revision != r.expected_revision
            || !seen.insert(p.request.stage as u8)
            || !p.invariants.production_required_passed()
        {
            return Err(error("Conflicting supply market prerequisites"));
        }
        crate::market_cycle::verify_history(w, &p)?;
        if matches!(
            p.request.stage,
            MarketStage::Maintenance | MarketStage::ClosingMarket
        ) {
            orders.extend(p.orders.clone());
        }
        operations.extend(p.operations.clone());
        evidence.extend(
            p.operations
                .iter()
                .map(operation_evidence)
                .collect::<OpResult<Vec<_>>>()?,
        );
    }
    Ok((orders, operations, evidence))
}
fn supply_inputs(
    w: &dyn WorldRead,
    r: &TransportRequest,
) -> OpResult<(
    Vec<MarketOrder>,
    Vec<EconomicOperation>,
    Vec<OperationEvidence>,
)> {
    let mut result = inputs(w, r)?;
    let reserve = crate::local_food_reserves::replay(
        w,
        &TransportRequest {
            stage: TransportStage::LocalFoodReserves,
            ..r.clone()
        },
    )?
    .ok_or_else(|| error("Physical supply requires same-period local food reserves"))?;
    result.2.extend(reserve.operation_evidence);
    Ok(result)
}
pub(crate) fn replay(
    w: &dyn WorldRead,
    r: &TransportRequest,
) -> OpResult<Option<TransportPhaseReceipt>> {
    let found = w.all_transport_phase_receipts().select(|p| p.request.stage == TransportStage::Dispatch && p.request.period == r.period)?;
    if found.is_empty() {
        return Ok(None);
    }
    if found.len() != 1
        || found[0].request != *r
        || found[0].campaign_id != w.campaign().campaign_id
        || !found[0].invariants.all_required_passed()
    {
        return Err(error("Conflicting supply phase"));
    }
    let p = &*found[0];
    if p.input_evidence != supply_inputs(w, r)?.2 {
        return Err(error("Supply prerequisite evidence altered"));
    }
    let history = crate::operation_history::CheckedHistory::new(w)?;
    let proof = p
        .operations
        .iter()
        .map(operation_evidence)
        .collect::<OpResult<Vec<_>>>()?;
    let mut ids = HashSet::new();
    if proof != p.operation_evidence
        || p.operations.iter().any(|o| {
            !ids.insert(&o.event_id)
                || !history.matches(o)
        })
    {
        return Err(error("Missing or altered supply history"));
    }
    let marker = p
        .operations
        .last()
        .ok_or_else(|| error("Supply boundary lost"))?;
    if marker.kind != OperationKind::SupplyPhaseBoundary
        || marker.event_id.json() != json!(boundary(w, r))
        || marker.meta.get("registry_sha256") != Some(&json!(digest(&p.registry_changes)?))
    {
        return Err(error("Supply registry history altered"));
    }
    Ok(Some(p.clone()))
}
#[derive(Clone)]
struct Choice {
    offer: SaleOffer,
    route: Value,
    sequence: usize,
    price: Money,
}
fn choices(
    w: &dyn WorldRead,
    order: &MarketOrder,
    routes: &[Value],
    validation: &mut RouteValidation,
) -> OpResult<Vec<Choice>> {
    let mut result = vec![];
    for offer in w.sale_offers().values() {
        if offer.goods_id != order.goods_id
            || offer.extra.get("currency_id")
                != order.currency_id.as_ref().map(ExternalId::json).as_ref()
        {
            continue;
        }
        let Some(stock) = w.stockpiles().values()
            .find(|s| s.external_id == offer.stock_id)
        else {
            continue;
        };
        let mut candidates = routes
            .iter()
            .filter(|r| {
                province(&r["from_province_id"]).ok() == Some(stock.province_id)
                    && province(&r["to_province_id"]).ok() == Some(order.province_id)
            })
            .cloned()
            .collect::<Vec<_>>();
        if candidates.is_empty()
            && stock.province_id != order.province_id
            && w.automatic_land_paths()
                .as_ref()
                .is_some_and(crate::needs::js_truthy)
        {
            if let Some(template) = routes.iter().find(|r| {
                r["mode"] == json!("land")
                    && province(&r["from_province_id"]).ok() == Some(stock.province_id)
            }) {
                if let Some(route) =
                    validation.adapt_land(template, stock.province_id, order.province_id)?
                {
                    candidates.push(route);
                }
            }
        }
        for route in candidates {
            let price = offer
                .extra
                .get("price_minor_per_unit")
                .filter(|v| !v.is_null())
                .map(|v| serde_json::from_value::<Money>(v.clone()))
                .transpose()?
                .unwrap_or(Money::ZERO);
            result.push(Choice {
                offer: offer.clone(),
                route,
                sequence: result.len(),
                price,
            });
        }
    }
    let collator = crate::ordering::collator();
    result.sort_by(|a, b| {
        a.route["travel_days"]
            .as_f64()
            .unwrap_or(0.)
            .partial_cmp(&b.route["travel_days"].as_f64().unwrap_or(0.))
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.price.cmp(&b.price))
            .then_with(|| {
                collator.compare(
                    &crate::needs::js_string(&a.route["id"]),
                    &crate::needs::js_string(&b.route["id"]),
                )
            })
            .then(a.sequence.cmp(&b.sequence))
    });
    Ok(result)
}
pub fn prepare_physical_supply(
    w: &dyn WorldRead,
    r: &TransportRequest,
) -> OpResult<PreparedPhysicalSupply> {
    prepare_physical_supply_guarded(w,r,None)
}
fn prepare_physical_supply_guarded(
    w: &dyn WorldRead,
    r: &TransportRequest,
    local:Option<&crate::world_transaction::WorldTransaction<'_>>,
) -> OpResult<PreparedPhysicalSupply> {
    if let Some(receipt) = replay(w, r)? {
        return Ok(PreparedPhysicalSupply {
            reads: None,
            state: None,
            receipt,
        });
    }
    if r.stage != TransportStage::Dispatch
        || r.expected_revision != w.campaign().revision
        || r.period.is_empty()
    {
        return Err(error("Invalid physical supply request"));
    }
    let (orders, market, input_evidence) = supply_inputs(w, r)?;
    if w.income_phase_receipts().iter().any(|p| {
        p.request.period == r.period
            && p.stage == kingmaker_core::income::IncomeStage::ClosingIncome
    }) {
        return Err(error("Supply must precede closing income"));
    }
    let prefix = format!(
        "supply:{}:auto-supply:{}:",
        w.campaign().campaign_id, r.period
    );
    if w.committed_operations().iter().any(|o| {
        o.event_id.json() == json!(boundary(w, r))
            || crate::needs::js_string(&o.event_id.json()).starts_with(&prefix)
    }) {
        return Err(error("Orphan physical supply history"));
    }
    let mut fulfilled = HashMap::<String, f64>::new();
    for op in &market {
        if op.kind == OperationKind::Purchase {
            if let (Some(id), Some(q)) = (
                op.meta.get("order_id").and_then(Value::as_str),
                op.meta.get("quantity").and_then(Value::as_f64),
            ) {
                *fulfilled.entry(id.into()).or_default() += q;
            }
        }
    }
    let mut incoming = HashMap::<PipelineKey, f64>::new();
    for s in &w.shipments() {
        let q = s["remaining_quantity"]
            .as_f64()
            .filter(|q| q.is_finite() && *q >= 0.)
            .ok_or_else(|| error("Invalid incoming cargo quantity"))?;
        if q > 0.
            && !matches!(
                s["status"].as_str(),
                Some("returned" | "returning" | "cancelled" | "lost")
            )
        {
            let Ok(facility) = actor(&s["facility_id"]) else {
                continue;
            };
            *incoming
                .entry((
                    actor(&s["buyer_id"])?,
                    actor(&s["goods_id"])?,
                    province(&s["to_province_id"])?,
                    facility,
                    s["inventory_purpose"]
                        .as_str()
                        .unwrap_or("commercial")
                        .into(),
                    s.get("beneficiary_pop_id")
                        .filter(|v| !v.is_null())
                        .map(actor)
                        .transpose()?,
                ))
                .or_default() += q;
        }
    }
    let scope = MarketScope::from_world(w)?;
    let mut food_forecast = crate::food_supply_forecast::FoodSupplyForecast::new(w, &orders)?;
    let mut session = DispatchSession::new(w, &r.period)?;
    let mut validation = RouteValidation::new(w)?;
    let mut routes = w.supply_routes().iter().cloned().collect::<Vec<_>>();
    let mut diagnostics = vec![];
    let mut decisions = vec![];
    for o in orders
        .iter()
        .filter(|o| !crate::food_supply_forecast::food(o))
        .chain(
            orders
                .iter()
                .filter(|o| crate::food_supply_forecast::food(o) && o.quantity > 0.),
        )
    {
        let is_food = crate::food_supply_forecast::food(o);
        let facility = o.facility_id.clone().or_else(|| {
            w.consumer_facility_by_province()
                .as_ref()
                .and_then(|m| m.get(o.province_id.0.to_string()))
                .and_then(|v| actor(v).ok())
        });
        let Some(facility) = facility else {
            diagnostics.push(json!({"id":o.id,"reason":"consumer_storage_not_configured"}));
            continue;
        };
        let purpose = o.extra.get("inventory_purpose").and_then(Value::as_str);
        let key = (
            o.owner_id.clone(),
            o.goods_id.clone(),
            o.province_id,
            facility.clone(),
            purpose.unwrap_or("commercial").into(),
            purpose.and(o.pop_id.clone()),
        );
        let mut remaining = if is_food {
            o.quantity
        } else {
            let demand = (o.quantity - fulfilled.get(&o.id).copied().unwrap_or(0.)).max(0.);
            let available = incoming.entry(key).or_default();
            let allocated = demand.min(*available);
            *available -= allocated;
            demand - allocated
        };
        if !remaining.is_finite() {
            return Err(error("Nonfinite unmet supply need"));
        }
        if remaining <= 1e-9 {
            continue;
        }
        let candidates = choices(w, o, &routes, &mut validation)?;
        let mut attempted = false;
        for choice in candidates {
            if !is_food && remaining <= 1e-9 {
                break;
            }
            let Some(stock) = session.session.stock(&choice.offer.stock_id) else {
                continue;
            };
            if scope.same(stock.province_id, o.province_id) {
                continue;
            }
            if is_food {
                let Some(days) = choice.route["travel_days"].as_f64().filter(|n| {
                    n.is_finite() && *n >= 0. && n.fract() == 0. && *n <= 9_007_199_254_740_991.
                }) else {
                    continue;
                };
                remaining = food_forecast.forecast_for(o, days as i64)?.quantity;
                if remaining <= 1e-9 {
                    continue;
                }
            }
            let Some(offer_id) = choice.offer.id.clone() else {
                continue;
            };
            let route_id = actor(&choice.route["id"])?;
            if !routes.iter().any(|r| r["id"] == route_id.json()) {
                session.add_route(choice.route.clone())?;
                routes.push(choice.route.clone());
            }
            let id = ExternalId::from(format!(
                "auto-supply:{}:{}:{}:{}",
                r.period,
                o.id,
                crate::needs::js_string(&offer_id.json()),
                crate::needs::js_string(&route_id.json())
            ));
            attempted = true;
            let out = session.dispatch(&ShipmentOrder {
                id: id.clone(),
                buyer_id: o.owner_id.clone(),
                purpose: o.purpose.clone(),
                offer_id,
                route_id,
                destination_facility_id: facility.clone(),
                quantity: remaining,
                inventory_purpose: if is_food {
                    Some("food_reserve".into())
                } else {
                    o.extra
                        .get("inventory_purpose")
                        .and_then(Value::as_str)
                        .map(str::to_owned)
                },
                pop_id: o.pop_id.clone(),
            })?;
            if let Some(reason) = out.reason {
                diagnostics.push(json!({"id":id,"order_id":o.id,"reason":reason}));
            }
            if out.quantity > 0. {
                if is_food {
                    let days = choice.route["travel_days"]
                        .as_f64()
                        .ok_or_else(|| error("Food route date absent"))?
                        as i64;
                    food_forecast.register_dispatch(
                        o,
                        out.quantity,
                        crate::calendar::add_days(GameDate::from(w.campaign()), days)?,
                    )?;
                }
                remaining = (remaining - out.quantity).max(0.);
                decisions.push(json!({"order_id":o.id,"shipment_id":id,"quantity":out.quantity,"replayed":out.replayed}));
            }
        }
        if remaining > 1e-9 {
            diagnostics.push(json!({"id":o.id,"unfulfilled_quantity":remaining,"reason":if attempted{"supply_unfulfilled"}else{"supply_route_or_offer_unavailable"}}));
        }
    }
    diagnostics.push(json!({"food_forecast":food_forecast.diagnostics()}));
    let plan = session.finish()?;
    let mut state=local.map_or_else(||crate::world_transaction::WorldTransaction::new(w),|parent|parent.child());
    state.apply_phase(plan.changes)?;
    state.replace_shipments(plan.shipments)?;
    state.replace_route_reservations(plan.route_reservations)?;
    state.replace_supply_routes(plan.routes)?;
    let old = w.shipments().len();
    let registry_changes = json!({"shipments_added":&state.shipments().iter().skip(old).collect::<Vec<_>>(),"route_reservations_added":&state.route_reservations().iter().skip(w.route_reservations().len()).collect::<Vec<_>>(),"routes_added":&state.supply_routes().iter().skip(w.supply_routes().len()).collect::<Vec<_>>()});
    let op: EconomicOperation = serde_json::from_value(
        json!({"event_id":boundary(w,r),"type":"supply_phase_boundary","period":r.period,"stage":"dispatch","date":GameDate::from(w.campaign()),"registry_sha256":digest(&registry_changes)?}),
    )?;
    let mut boundary_session = crate::operation_session::OperationSession::new(&state)?;
    crate::shipping_operations::stage_shipping_operations(
        &mut boundary_session,
        std::slice::from_ref(&op),
    )?;
    let changes = boundary_session.finish()?;
    state.apply_phase(changes)?;
    let mut operations = plan.operations;
    operations.push(op);
    let invariants = crate::shipping_invariants::verify_arrivals(w, &state, &operations)?;
    let receipt = TransportPhaseReceipt {
        campaign_id: w.campaign().campaign_id.clone(),
        request: r.clone(),
        date: GameDate::from(w.campaign()),
        operation_evidence: operations
            .iter()
            .map(operation_evidence)
            .collect::<OpResult<_>>()?,
        input_evidence,
        operations,
        registry_changes,
        diagnostics,
        decisions,
        invariants,
    };
    let key=state.transport_phase_receipts().next_key().map_err(crate::operation_session::OperationError)?;
    state.put_transport_phase_receipts(key,receipt.clone())?;
    Ok(PreparedPhysicalSupply {
        reads: Some(crate::world_transaction::PrepareProof::capture(w,local.map(|parent|parent.local_prepare_token()))?),
        state: Some(state.finish()?),
        receipt,
    })
}
pub fn apply_physical_supply(
    w: &mut WorldState,
    p: PreparedPhysicalSupply,
) -> OpResult<TransportPhaseReceipt> {
    crate::world_transaction::execute_atomic(w,|w|apply_physical_supply_in(w,p))
}
pub(crate) fn apply_physical_supply_in(
    w: &mut crate::world_transaction::WorldTransaction<'_>,
    p: PreparedPhysicalSupply,
) -> OpResult<TransportPhaseReceipt> {
    if let Some(reads) = p.reads {
        reads.validate(w)?;
        w.apply_delta(p
            .state
            .ok_or_else(|| error("Physical supply state missing"))?)?;
    } else if replay(w, &p.receipt.request)?.as_ref() != Some(&p.receipt) {
        return Err(error("Prepared supply replay altered"));
    }
    Ok(p.receipt)
}
pub fn execute_physical_supply(
    w: &mut WorldState,
    r: &TransportRequest,
) -> OpResult<TransportPhaseReceipt> {
    crate::world_transaction::execute_atomic(w,|w|execute_physical_supply_in(w,r))
}
pub(crate) fn execute_physical_supply_in(
    w: &mut crate::world_transaction::WorldTransaction<'_>,
    r: &TransportRequest,
) -> OpResult<TransportPhaseReceipt> {
    let p = prepare_physical_supply_guarded(w, r, Some(w))?;
    apply_physical_supply_in(w, p)
}

