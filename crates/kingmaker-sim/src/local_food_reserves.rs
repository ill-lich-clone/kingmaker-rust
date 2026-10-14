//! Real paid local food buffers derived from authenticated weekly market orders.
use kingmaker_core::world_read::WorldRead;
use crate::{
    budget_funding::error,
    food_supply_forecast::{food, food_key, FoodSupplyForecast},
    operation_history::operation_evidence,
    operation_session::{canonical_operation, OpResult, OperationSession},
    transport_validation::actor,
};
use kingmaker_core::{calendar::GameDate, market::*, operations::*, transport::*, *};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
pub struct PreparedLocalFoodReserves {
    reads: Option<crate::world_transaction::PrepareProof>,
    state: Option<crate::world_transaction::PreparedWorldDelta>,
    receipt: TransportPhaseReceipt,
}
fn hash(v: &Value) -> OpResult<String> {
    Ok(Sha256::digest(serde_json::to_vec(v)?)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}
fn boundary(w: &dyn WorldRead, r: &TransportRequest) -> String {
    format!(
        "supply-phase:{}:{}:local-food-reserves",
        w.campaign().campaign_id, r.period
    )
}
pub(crate) fn replay(
    w: &dyn WorldRead,
    r: &TransportRequest,
) -> OpResult<Option<TransportPhaseReceipt>> {
    let found = w.all_transport_phase_receipts().select(|p| {
            p.request.stage == TransportStage::LocalFoodReserves && p.request.period == r.period
        })?;
    if found.is_empty() {
        return Ok(None);
    }
    if found.len() != 1
        || found[0].request != *r
        || found[0].campaign_id != w.campaign().campaign_id
        || !found[0].invariants.production_required_passed()
    {
        return Err(error("Conflicting local food reserve receipt"));
    }
    let p = &*found[0];
    if p.input_evidence != crate::physical_supply::inputs(w, r)?.2 {
        return Err(error("Food reserve prerequisite evidence altered"));
    }
    let history = crate::operation_history::CheckedHistory::new(w)?;
    let mut seen = HashSet::new();
    if p.operations
            .iter()
            .map(operation_evidence)
            .collect::<OpResult<Vec<_>>>()?
            != p.operation_evidence
        || p.operations.iter().any(|o| {
            !seen.insert(&o.event_id)
                || !history.matches(o)
        })
    {
        return Err(error("Food reserve history missing or altered"));
    }
    let marker = p
        .operations
        .last()
        .ok_or_else(|| error("Food reserve boundary absent"))?;
    if marker.kind != OperationKind::SupplyPhaseBoundary
        || marker.event_id.json() != json!(boundary(w, r))
        || marker.meta.get("registry_sha256") != Some(&json!(hash(&p.registry_changes)?))
        || marker.meta.get("decisions_sha256") != Some(&json!(hash(&json!(p.decisions))?))
    {
        return Err(error("Food reserve state evidence altered"));
    }
    Ok(Some(p.clone()))
}
pub fn prepare_local_food_reserves(
    w: &dyn WorldRead,
    r: &TransportRequest,
) -> OpResult<PreparedLocalFoodReserves> {
    prepare_local_food_reserves_guarded(w,r,None)
}
fn prepare_local_food_reserves_guarded(
    w: &dyn WorldRead,
    r: &TransportRequest,
    local:Option<&crate::world_transaction::WorldTransaction<'_>>,
) -> OpResult<PreparedLocalFoodReserves> {
    if let Some(receipt) = replay(w, r)? {
        return Ok(PreparedLocalFoodReserves {
            reads: None,
            state: None,
            receipt,
        });
    }
    if r.stage != TransportStage::LocalFoodReserves
        || r.expected_revision != w.campaign().revision
        || r.period.is_empty()
    {
        return Err(error("Invalid local food reserve request"));
    }
    let (orders, _, input_evidence) = crate::physical_supply::inputs(w, r)?;
    if w.transport_phase_receipts().iter().any(|p| {
        p.request.period == r.period
            && matches!(
                p.request.stage,
                TransportStage::Dispatch | TransportStage::StateProcurement
            )
    }) || w.income_phase_receipts().iter().any(|p| {
        p.request.period == r.period
            && p.stage == kingmaker_core::income::IncomeStage::ClosingIncome
    }) {
        return Err(error(
            "Food reserves must precede outgoing supply and income",
        ));
    }
    let prefix = format!(
        "market:{}:{}:food-reserve:",
        w.campaign().campaign_id, r.period
    );
    if w.committed_operations().iter().any(|o| {
        o.event_id.json() == json!(boundary(w, r))
            || crate::needs::js_string(&o.event_id.json()).starts_with(&prefix)
    }) {
        return Err(error("Orphan food reserve history"));
    }
    let mut forecast = FoodSupplyForecast::new(w, &orders)?;
    let mut seen = HashSet::new();
    let mut reserve_orders = vec![];
    let mut diagnostics = vec![];
    if let Some(mapping) = &w.consumer_facility_by_province() {
        for o in orders.iter().filter(|o| food(o)) {
            if !seen.insert(food_key(o)?) {
                continue;
            }
            let n = forecast.forecast_for(o, 0)?;
            if n.quantity > 1e-9 {
                let mut order = o.clone();
                order.id = format!("food-reserve:{}:{}", r.period, o.id);
                order.quantity = n.quantity;
                order.consume = false;
                order
                    .extra
                    .insert("inventory_purpose".into(), json!("food_reserve"));
                order.facility_id = o.facility_id.clone().or_else(|| {
                    mapping
                        .get(o.province_id.0.to_string())
                        .and_then(|v| actor(v).ok())
                });
                reserve_orders.push(order);
            }
        }
    } else {
        diagnostics.push(json!({"status":"consumer_storage_not_configured"}));
    }
    let decisions = vec![json!({"reserve_orders":reserve_orders})];
    let mut session = OperationSession::new(w)?;
    session.configure_market(&MarketRequest {
        expected_revision: r.expected_revision,
        period: format!("{}:food-reserve", r.period),
        stage: MarketStage::ClosingMarket,
        profile: Default::default(),
        priorities: Some(vec!["basic".into()]),
    })?;
    crate::market_clearing::plan_foundation_market(
        &mut session,
        &format!("{}:food-reserve", r.period),
        &reserve_orders,
        &["basic".into()],
    )?;
    let registry_changes = serde_json::to_value(session.registry_changes())?;
    let op: EconomicOperation = serde_json::from_value(
        json!({"event_id":boundary(w,r),"type":"supply_phase_boundary","period":r.period,"stage":"local_food_reserves","date":GameDate::from(w.campaign()),"registry_sha256":hash(&registry_changes)?,"decisions_sha256":hash(&json!(decisions))?}),
    )?;
    crate::shipping_operations::stage_shipping_operations(&mut session, std::slice::from_ref(&op))?;
    let market_receipt = MarketPhaseReceipt {
        campaign_id: w.campaign().campaign_id.clone(),
        request: MarketRequest {
            expected_revision: r.expected_revision,
            period: format!("{}:food-reserve", r.period),
            stage: MarketStage::ClosingMarket,
            profile: Default::default(),
            priorities: Some(vec!["basic".into()]),
        },
        orders: reserve_orders,
        effective_priorities: vec!["basic".into()],
        frozen_needs: vec![],
        operations: session.operations().to_vec(),
        maintenance_receipts: vec![],
        consumption: vec![],
        diagnostics: vec![],
        registry_changes: session.registry_changes(),
        invariants: Default::default(),
    };
    let invariants =
        crate::market_invariants::check_session_market_operations(&session, &market_receipt);
    if !invariants.production_required_passed() {
        return Err(error("Food reserve market audit failed"));
    }
    let operations = session.operations().to_vec();
    let changes = session.finish()?;
    let mut state=local.map_or_else(||crate::world_transaction::WorldTransaction::new(w),|parent|parent.child());
    state.apply_phase(changes)?;
    diagnostics.push(json!({"food_forecast":forecast.diagnostics()}));
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
    Ok(PreparedLocalFoodReserves {
        reads: Some(crate::world_transaction::PrepareProof::capture(w,local.map(|parent|parent.local_prepare_token()))?),
        state: Some(state.finish()?),
        receipt,
    })
}
pub fn apply_local_food_reserves(
    w: &mut WorldState,
    p: PreparedLocalFoodReserves,
) -> OpResult<TransportPhaseReceipt> {
    crate::world_transaction::execute_atomic(w,|w|apply_local_food_reserves_in(w,p))
}
pub(crate) fn apply_local_food_reserves_in(
    w: &mut crate::world_transaction::WorldTransaction<'_>,
    p: PreparedLocalFoodReserves,
) -> OpResult<TransportPhaseReceipt> {
    if let Some(reads) = p.reads {
        reads.validate(w)?;
        w.apply_delta(p.state.ok_or_else(|| error("Food reserve state absent"))?)?;
    } else if replay(w, &p.receipt.request)?.as_ref() != Some(&p.receipt) {
        return Err(error("Food reserve replay altered"));
    }
    Ok(p.receipt)
}
pub fn execute_local_food_reserves(
    w: &mut WorldState,
    r: &TransportRequest,
) -> OpResult<TransportPhaseReceipt> {
    crate::world_transaction::execute_atomic(w,|w|execute_local_food_reserves_in(w,r))
}
pub(crate) fn execute_local_food_reserves_in(
    w: &mut crate::world_transaction::WorldTransaction<'_>,
    r: &TransportRequest,
) -> OpResult<TransportPhaseReceipt> {
    let p = prepare_local_food_reserves_guarded(w, r, Some(w))?;
    apply_local_food_reserves_in(w, p)
}

