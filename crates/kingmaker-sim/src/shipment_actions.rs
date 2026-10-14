//! Explicit transport commands preserve the cargo and use approved owner/participant terms.
use kingmaker_core::world_read::WorldRead;
use crate::{
    budget_funding::error,
    calendar::add_days,
    operation_history::operation_evidence,
    operation_session::{canonical_operation, OpResult, OperationSession},
    shipping_operations::stage_shipping_operations,
    storage::{approved, release_storage, reserve_storage, StorageBatch, StorageRequest},
    transport_validation::{actor, province},
};
use kingmaker_core::{calendar::GameDate, operations::*, transport::*, *};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
pub struct PreparedShipmentAction {
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
fn money(v: &Value) -> OpResult<Money> {
    let m: Money = serde_json::from_value(v.clone())?;
    if m.0 < 0 {
        return Err(error("Negative recovery money"));
    }
    Ok(m)
}
fn request(r: &ShipmentActionRequest) -> TransportRequest {
    TransportRequest {
        expected_revision: r.expected_revision,
        period: crate::needs::js_string(&r.event_id.json()),
        stage: match r.action {
            ShipmentAction::Interrupt { .. } => TransportStage::Interruption,
            ShipmentAction::Resume { .. } => TransportStage::Recovery,
            ShipmentAction::Return { .. } => TransportStage::Return,
        },
    }
}
fn boundary(w: &dyn WorldRead, r: &ShipmentActionRequest) -> String {
    format!(
        "shipment-action:{}:{}",
        w.campaign().campaign_id,
        crate::needs::js_string(&r.event_id.json())
    )
}
pub(crate) fn replay(
    w: &dyn WorldRead,
    r: &ShipmentActionRequest,
) -> OpResult<Option<TransportPhaseReceipt>> {
    let req = request(r);
    let rows = w.all_transport_phase_receipts().select(|p| {
            matches!(
                p.request.stage,
                TransportStage::Interruption | TransportStage::Recovery | TransportStage::Return
            ) && p.request.period == req.period
        })?;
    if rows.is_empty() {
        return Ok(None);
    }
    if rows.len() != 1
        || rows[0].request != req
        || rows[0].decisions != vec![json!(r)]
        || rows[0].campaign_id != w.campaign().campaign_id
        || !rows[0].invariants.all_required_passed()
    {
        return Err(error("Conflicting shipment action identity"));
    }
    let p = &*rows[0];
    let history = crate::operation_history::CheckedHistory::new(w)?;
    let mut ids = HashSet::new();
    let proof = p
        .operations
        .iter()
        .map(operation_evidence)
        .collect::<OpResult<Vec<_>>>()?;
    if proof != p.operation_evidence
        || p.operations.iter().any(|o| {
            !ids.insert(&o.event_id)
                || !history.matches(o)
        })
    {
        return Err(error("Missing or altered shipment action history"));
    }
    for evidence in &p.input_evidence {
        let value = history
            .get(&evidence.event_id)?
            .ok_or_else(|| error("Action prerequisite history missing"))?;
        let op: EconomicOperation = serde_json::from_value((*value).clone())?;
        if operation_evidence(&op)? != *evidence {
            return Err(error("Action prerequisite history altered"));
        }
    }
    let marker = p
        .operations
        .last()
        .ok_or_else(|| error("Shipment action boundary lost"))?;
    if marker.kind != OperationKind::SupplyPhaseBoundary
        || marker.event_id.json() != json!(boundary(w, r))
        || marker.meta.get("registry_sha256") != Some(&json!(digest(&p.registry_changes)?))
        || marker.meta.get("date") != Some(&json!(p.date))
        || marker.meta.get("economic_revision") != Some(&json!(p.request.expected_revision))
    {
        return Err(error("Shipment action registry altered"));
    }
    Ok(Some(p.clone()))
}
pub fn prepare_shipment_action(
    w: &dyn WorldRead,
    r: &ShipmentActionRequest,
) -> OpResult<PreparedShipmentAction> {
    prepare_shipment_action_guarded(w,r,None)
}
fn prepare_shipment_action_guarded(
    w: &dyn WorldRead,
    r: &ShipmentActionRequest,
    local:Option<&crate::world_transaction::WorldTransaction<'_>>,
) -> OpResult<PreparedShipmentAction> {
    if let Some(receipt) = replay(w, r)? {
        return Ok(PreparedShipmentAction {
            reads: None,
            state: None,
            receipt,
        });
    }
    if r.expected_revision != w.campaign().revision
        || crate::needs::js_string(&r.event_id.json()).is_empty()
    {
        return Err(error("Invalid shipment action event/revision"));
    }
    if w.committed_operations()
        .iter()
        .any(|o| o.event_id == r.event_id || o.event_id.json() == json!(boundary(w, r)))
    {
        return Err(error("Orphan shipment action history"));
    }
    let positions = w.shipments()
        .iter()
        .enumerate()
        .filter(|(_, s)| s["id"] == r.shipment_id.json())
        .map(|(i, _)| i)
        .collect::<Vec<_>>();
    if positions.len() != 1 {
        return Err(error("Unknown or duplicate action cargo"));
    }
    let index = positions[0];
    let before = w.shipments().at(index).expect("validated row position").clone();
    let mut cargo = before.clone();
    let date = GameDate::from(w.campaign());
    let mut session = OperationSession::new(w)?;
    let mut storage = StorageBatch::new(&session)?;
    let mut input_evidence = vec![];
    let op = match &r.action {
        ShipmentAction::Interrupt { reason } => {
            if reason.is_empty()
                || !matches!(
                    cargo["status"].as_str(),
                    Some("in_transit" | "partial_arrived" | "awaiting_storage")
                )
            {
                return Err(error("Shipment cannot be interrupted"));
            }
            cargo["status"] = json!("interrupted");
            cargo["interruption"] = json!({"event_id":r.event_id,"reason":reason});
            cargo["recovery_required"] = json!(true);
            json!({"event_id":r.event_id,"type":"shipment_interrupted","shipment_id":r.shipment_id})
        }
        ShipmentAction::Resume { recovery_id } | ShipmentAction::Return { recovery_id } => {
            let returning = matches!(r.action, ShipmentAction::Return { .. });
            let buyer = actor(&cargo["buyer_id"])?;
            if r.actor_id.as_ref() != Some(&buyer) {
                return Err(error("Verified cargo owner required"));
            }
            if cargo["status"] != json!("interrupted") {
                return Err(error("Only interrupted cargo accepts recovery/return"));
            }
            if let Some(event) = cargo.get("interruption").and_then(|v| v.get("event_id")) {
                let history = w.committed_operations()
                    .iter()
                    .find(|o| o.event_id.json() == *event)
                    .ok_or_else(|| error("Interruption history missing"))?;
                let op: EconomicOperation = serde_json::from_value(history.value.clone())?;
                if op.kind != OperationKind::ShipmentInterrupted
                    || op.meta.get("shipment_id") != Some(&r.shipment_id.json())
                {
                    return Err(error("Interruption history mismatch"));
                }
                input_evidence.push(operation_evidence(&op)?);
            }
            let plans = w.shipment_recovery_plans()
                .iter()
                .filter(|p| {
                    p["id"] == recovery_id.json() && p["shipment_id"] == r.shipment_id.json()
                })
                .collect::<Vec<_>>();
            if plans.len() != 1 {
                return Err(error("Approved recovery terms required"));
            }
            let plan = plans[0];
            let days = plan["remaining_travel_days"]
                .as_i64()
                .filter(|n| *n > 0 && *n <= 9007199254740991)
                .ok_or_else(|| error("Invalid physical recovery duration"))?;
            let currency = actor(&cargo["currency_id"])?;
            if !approved(
                plan["status"].as_str(),
                plan.get("source"),
                plan.get("approval_ref"),
            ) || plan["action"] != json!(if returning { "return" } else { "resume" })
                || plan["accepted_buyer_id"] != cargo["buyer_id"]
                || plan["accepted_carrier_id"] != cargo["carrier_id"]
                || (returning && plan["accepted_seller_id"] != cargo["seller_id"])
                || plan["currency_id"] != cargo["currency_id"]
            {
                return Err(error(
                    "Approved physical recovery participant/currency terms required",
                ));
            }
            let fee = money(&plan["extra_freight_minor"])?;
            let purpose = cargo["buyer_purpose"]
                .as_str()
                .ok_or_else(|| error("Cargo buyer purpose missing"))?
                .to_owned();
            if session.cash(&buyer, &currency, &purpose) < fee {
                return Err(error("Unfunded recovery freight"));
            }
            if returning {
                let seller = actor(&cargo["seller_id"])?;
                let stock_id = actor(&cargo["stock_id"])?;
                let facility = actor(&plan["source_facility_id"])?;
                let stock = session
                    .stock(&stock_id)
                    .ok_or_else(|| error("Return source stock missing"))?;
                let warehouse = storage
                    .facilities
                    .get(&facility)
                    .ok_or_else(|| error("Return source warehouse missing"))?;
                if stock.owner_id != seller
                    || stock.facility_id.as_ref() != Some(&facility)
                    || warehouse.province_id != province(&cargo["from_province_id"])?
                    || stock.goods.external_id() != actor(&cargo["goods_id"])?
                {
                    return Err(error("Return source storage rights mismatch"));
                }
                let q = cargo["remaining_quantity"]
                    .as_f64()
                    .filter(|n| n.is_finite() && *n > 0.)
                    .ok_or_else(|| error("Return cargo quantity invalid"))?;
                let reserved = reserve_storage(
                    &mut session,
                    &mut storage,
                    &StorageRequest {
                        id: format!(
                            "return-storage:{}",
                            crate::needs::js_string(&r.event_id.json())
                        )
                        .into(),
                        owner_id: seller,
                        facility_id: facility.clone(),
                        goods_id: actor(&cargo["goods_id"])?,
                        unit: cargo["unit"].clone(),
                        quantity: q,
                    },
                    &crate::needs::js_string(&r.event_id.json()),
                )?;
                if reserved.partial || reserved.reservation.is_none() {
                    return Err(error("Full return storage must be reserved"));
                }
                let reserved = reserved.reservation.unwrap();
                release_storage(
                    &mut session,
                    &mut storage,
                    &actor(&cargo["storage_reservation_id"])?,
                    &format!(
                        "{}:destination-release",
                        crate::needs::js_string(&r.event_id.json())
                    )
                    .into(),
                )?;
                cargo["status"] = json!("returning");
                cargo["return_receipt"] = json!({"event_id":r.event_id,"recovery_id":recovery_id});
                cargo["return_arrival_date"] = json!(add_days(date, days)?);
                cargo["return_initial_quantity"] = json!(q);
                cargo["returned_quantity"] = json!(0);
                cargo["refunded_minor"] = json!("0");
                cargo["return_facility_id"] = facility.json();
                cargo["return_storage_reservation_id"] = reserved.id.json();
                cargo["carrier_release_date"] = json!(add_days(date, days)?);
            } else {
                let returns = cargo["return_days"]
                    .as_i64()
                    .filter(|n| *n >= 0)
                    .ok_or_else(|| error("Invalid carrier return duration"))?;
                cargo["status"] = json!("in_transit");
                cargo["recovery_required"] = json!(false);
                cargo["arrival_date"] = json!(add_days(date, days)?);
                cargo["carrier_release_date"] = json!(add_days(
                    date,
                    days.checked_add(returns)
                        .ok_or_else(|| error("Recovery date overflow"))?
                )?);
                let receipts = cargo
                    .as_object_mut()
                    .unwrap()
                    .entry("recovery_receipts")
                    .or_insert(json!([]))
                    .as_array_mut()
                    .ok_or_else(|| error("Invalid recovery receipt list"))?;
                receipts.push(json!({"event_id":r.event_id,"recovery_id":recovery_id}));
            }
            cargo["freight_paid_minor"] = json!(money(&cargo["freight_paid_minor"])?
                .0
                .checked_add(fee.0)
                .ok_or_else(|| error("Recovery freight overflow"))?
                .to_string());
            let mut op = json!({"event_id":r.event_id,"type":if returning{"shipment_return_dispatch"}else{"shipment_recovery"},"shipment_id":r.shipment_id,"debits":[],"credits":[]});
            if fee.0 > 0 {
                op["debits"] = json!([{"owner_id":buyer,"currency_id":currency,"purpose":purpose,"amount_minor":fee.0.to_string()}]);
                op["credits"] = json!([{"owner_id":cargo["carrier_id"],"currency_id":currency,"purpose":"operating_capital","amount_minor":fee.0.to_string()}]);
            }
            op
        }
    };
    let op: EconomicOperation = serde_json::from_value(op)?;
    stage_shipping_operations(&mut session, std::slice::from_ref(&op))?;
    let registry_changes = json!({"shipment":{"before":before,"after":cargo},"storage":session.registry_changes().storage});
    let marker: EconomicOperation = serde_json::from_value(
        json!({"event_id":boundary(w,r),"type":"supply_phase_boundary","stage":request(r).stage,"date":date,"economic_revision":r.expected_revision,"registry_sha256":digest(&registry_changes)?}),
    )?;
    stage_shipping_operations(&mut session, std::slice::from_ref(&marker))?;
    let operations = session.operations().to_vec();
    let changes = session.finish()?;
    let mut state=local.map_or_else(||crate::world_transaction::WorldTransaction::new(w),|parent|parent.child());
    state.apply_phase(changes)?;
    let key=state.shipments().keyed().nth(index).ok_or_else(||error("Shipment disappeared"))?.0;
    state.put_shipments(key,cargo)?;
    let invariants = crate::shipping_invariants::verify_arrivals(w, &state, &operations)?;
    let receipt = TransportPhaseReceipt {
        campaign_id: w.campaign().campaign_id.clone(),
        request: request(r),
        date,
        operation_evidence: operations
            .iter()
            .map(operation_evidence)
            .collect::<OpResult<_>>()?,
        input_evidence,
        operations,
        registry_changes,
        diagnostics: vec![],
        decisions: vec![json!(r)],
        invariants,
    };
    let key=state.transport_phase_receipts().next_key().map_err(crate::operation_session::OperationError)?;
    state.put_transport_phase_receipts(key,receipt.clone())?;
    Ok(PreparedShipmentAction {
        reads: Some(crate::world_transaction::PrepareProof::capture(w,local.map(|parent|parent.local_prepare_token()))?),
        state: Some(state.finish()?),
        receipt,
    })
}
pub fn apply_shipment_action(
    w: &mut WorldState,
    p: PreparedShipmentAction,
) -> OpResult<TransportPhaseReceipt> {
    crate::world_transaction::execute_atomic(w,|w|apply_shipment_action_in(w,p))
}
pub(crate) fn apply_shipment_action_in(
    w: &mut crate::world_transaction::WorldTransaction<'_>,
    p: PreparedShipmentAction,
) -> OpResult<TransportPhaseReceipt> {
    if let Some(reads) = p.reads {
        reads.validate(w)?;
        w.apply_delta(p
            .state
            .ok_or_else(|| error("Shipment action state missing"))?)?;
    } else {
        let r: ShipmentActionRequest = serde_json::from_value(p.receipt.decisions[0].clone())?;
        if replay(w, &r)?.as_ref() != Some(&p.receipt) {
            return Err(error("Prepared shipment action replay altered"));
        }
    }
    Ok(p.receipt)
}
pub fn execute_shipment_action(
    w: &mut WorldState,
    r: &ShipmentActionRequest,
) -> OpResult<TransportPhaseReceipt> {
    crate::world_transaction::execute_atomic(w,|w|execute_shipment_action_in(w,r))
}
pub(crate) fn execute_shipment_action_in(
    w: &mut crate::world_transaction::WorldTransaction<'_>,
    r: &ShipmentActionRequest,
) -> OpResult<TransportPhaseReceipt> {
    let p = prepare_shipment_action_guarded(w, r, Some(w))?;
    apply_shipment_action_in(w, p)
}

