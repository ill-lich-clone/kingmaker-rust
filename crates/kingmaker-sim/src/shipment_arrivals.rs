//! Real calendar arrivals: private readset, actual cargo/storage/escrow and canonical replay.
use kingmaker_core::world_read::WorldRead;
use crate::{
    budget_funding::error,
    calendar::{add_days, serial_date},
    money_math::multiply_money,
    operation_history::operation_evidence,
    operation_session::{canonical_operation, OpResult, OperationSession},
    shipping_operations::stage_shipping_operations,
    storage::StorageBatch,
    transport_validation::actor,
};
use kingmaker_core::{calendar::GameDate, operations::*, transport::*, *};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
pub struct PreparedShipmentArrivals {
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
        return Err(error("Negative shipment money"));
    }
    Ok(m)
}
fn quantity(v: &Value) -> OpResult<f64> {
    v.as_f64()
        .filter(|n| n.is_finite() && *n >= 0.)
        .ok_or_else(|| error("Invalid shipment quantity"))
}
fn date(v: &Value) -> OpResult<GameDate> {
    let d: GameDate = serde_json::from_value(v.clone())?;
    serial_date(d)?;
    Ok(d)
}
fn boundary(w: &dyn WorldRead, r: &TransportRequest) -> String {
    format!(
        "supply-phase:{}:{}:arrivals",
        w.campaign().campaign_id, r.period
    )
}
pub(crate) fn replay(
    w: &dyn WorldRead,
    r: &TransportRequest,
) -> OpResult<Option<TransportPhaseReceipt>> {
    let found = w.all_transport_phase_receipts().select(|p| p.request.stage == TransportStage::Arrivals && p.request.period == r.period)?;
    if found.is_empty() {
        return Ok(None);
    }
    if found.len() != 1
        || found[0].request != *r
        || found[0].campaign_id != w.campaign().campaign_id
        || !found[0].invariants.all_required_passed()
    {
        return Err(error("Conflicting arrival receipt"));
    }
    let p = &*found[0];
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
        return Err(error("Missing or altered arrival history"));
    }
    let op = p
        .operations
        .last()
        .ok_or_else(|| error("Arrival boundary missing"))?;
    if op.kind != OperationKind::SupplyPhaseBoundary
        || op.event_id.json() != json!(boundary(w, r))
        || op.meta.get("registry_sha256") != Some(&json!(digest(&p.registry_changes)?))
        || op.meta.get("date") != Some(&json!(p.date))
    {
        return Err(error("Arrival registry/date history altered"));
    }
    Ok(Some(p.clone()))
}
pub fn prepare_shipment_arrivals(
    w: &dyn WorldRead,
    r: &TransportRequest,
) -> OpResult<PreparedShipmentArrivals> {
    prepare_shipment_arrivals_guarded(w,r,None)
}
fn prepare_shipment_arrivals_guarded(
    w: &dyn WorldRead,
    r: &TransportRequest,
    local:Option<&crate::world_transaction::WorldTransaction<'_>>,
) -> OpResult<PreparedShipmentArrivals> {
    if let Some(receipt) = replay(w, r)? {
        return Ok(PreparedShipmentArrivals {
            reads: None,
            state: None,
            receipt,
        });
    }
    if r.expected_revision != w.campaign().revision
        || r.period.is_empty()
        || r.stage != TransportStage::Arrivals
    {
        return Err(error("Invalid arrival request"));
    }
    if w.budget_phase_receipts()
        .iter()
        .any(|p| p.request.period == r.period)
        || w.production_phase_receipts()
            .iter()
            .any(|p| p.request.period == r.period)
        || w.market_phase_receipts()
            .iter()
            .any(|p| p.request.period == r.period)
        || w.labor_phase_receipts()
            .iter()
            .any(|p| p.request.period == r.period)
    {
        return Err(error("Arrivals must precede weekly labor and economy"));
    }
    let event_prefix = format!("arrival:{}:{}:", w.campaign().campaign_id, r.period);
    let return_prefix = format!("return-arrival:{}:{}:", w.campaign().campaign_id, r.period);
    if w.committed_operations().iter().any(|o| {
        o.event_id.json() == json!(boundary(w, r))
            || crate::needs::js_string(&o.event_id.json()).starts_with(&event_prefix)
            || crate::needs::js_string(&o.event_id.json()).starts_with(&return_prefix)
    }) {
        return Err(error("Orphan arrival history"));
    }
    let current = GameDate::from(w.campaign());
    let today = serial_date(current)?;
    let mut session = OperationSession::new(w)?;
    let mut storage = StorageBatch::new(&session)?;
    let mut shipments = w.shipments().iter().cloned().collect::<Vec<_>>();
    let mut reservations = w.storage_reservations().iter().cloned().collect::<Vec<_>>();
    let positions = reservations
        .iter()
        .enumerate()
        .map(|(i, r)| (r.id.clone(), i))
        .collect::<HashMap<_, _>>();
    if positions.len() != reservations.len() {
        return Err(error("Duplicate shipment storage reservation"));
    }
    let mut seen = HashSet::new();
    let mut diagnostics = vec![];
    let mut operations = vec![];
    let mut ship_changes = vec![];
    let mut reserve_changes = vec![];
    for cargo in &mut shipments {
        let id = actor(&cargo["id"])?;
        if !seen.insert(id.clone()) {
            return Err(error("Duplicate cargo identity"));
        }
        let remaining = quantity(&cargo["remaining_quantity"])?;
        let status = cargo["status"].as_str().unwrap_or("");
        let returning = matches!(status, "returning" | "awaiting_return_storage");
        if remaining == 0.
            || (!returning
                && !matches!(
                    status,
                    "in_transit" | "awaiting_storage" | "partial_arrived"
                ))
        {
            continue;
        }
        let due = date(
            &cargo[if returning {
                "return_arrival_date"
            } else {
                "arrival_date"
            }],
        )?;
        if serial_date(due)? > today {
            continue;
        }
        let before = cargo.clone();
        let reserve_id = actor(
            &cargo[if returning {
                "return_storage_reservation_id"
            } else {
                "storage_reservation_id"
            }],
        )?;
        let position = *positions
            .get(&reserve_id)
            .ok_or_else(|| error("Shipment destination reservation missing"))?;
        let reservation = &mut reservations[position];
        let old_reservation = reservation.clone();
        let owner = actor(&cargo[if returning { "seller_id" } else { "buyer_id" }])?;
        let facility = actor(
            &cargo[if returning {
                "return_facility_id"
            } else {
                "facility_id"
            }],
        )?;
        let good = actor(&cargo["goods_id"])?;
        if reservation.status != "held"
            || reservation.owner_id != owner
            || reservation.facility_id != facility
            || reservation.goods_id != good
            || reservation.unit != cargo["unit"]
        {
            return Err(error("Shipment destination reservation rights mismatch"));
        }
        let available = storage.availability(&facility, &good);
        if available.status == "unknown" {
            diagnostics.push(json!({"shipment_id":id,"reason":if returning{"return_storage_unavailable"}else{"destination_storage_unknown"}}));
            continue;
        }
        let warehouse = &storage.facilities[&facility];
        if warehouse.owner_id != owner {
            let lease = reservation
                .lease_terms
                .as_ref()
                .ok_or_else(|| error("Shipment destination tenancy missing"))?;
            if lease.owner_id.as_ref() != Some(&owner)
                || lease.currency_id.as_ref() != Some(&actor(&cargo["currency_id"])?)
                || lease.purpose.as_deref()
                    != cargo[if returning {
                        "seller_purpose"
                    } else {
                        "buyer_purpose"
                    }]
                    .as_str()
                || !crate::storage::approved(
                    lease.status.as_deref(),
                    lease.source.as_ref(),
                    lease.approval_ref.as_ref(),
                )
                || lease.price_minor_per_space.is_none_or(|m| m.0 < 0)
            {
                return Err(error("Shipment destination tenancy rights mismatch"));
            }
        }
        let space = storage.facilities[&facility].goods[&crate::needs::js_string(&good.json())]
            .space_per_unit;
        let old = reservation.quantity;
        if !old.is_finite() || old < 0. {
            return Err(error("Invalid shipment storage quantity"));
        }
        storage.adjust(&facility, -old * space)?;
        let free = storage.availability(&facility, &good).free.unwrap_or(0.);
        storage.adjust(&facility, old * space)?;
        let q = remaining.min(old).min(free);
        if q <= 0. {
            cargo["status"] = json!(if returning {
                "awaiting_return_storage"
            } else {
                "awaiting_storage"
            });
            if !returning {
                cargo["storage_delayed"] = json!(true);
            }
            diagnostics.push(json!({"shipment_id":id,"reason":if returning{"return_storage_unavailable"}else{"destination_storage_full"}}));
            ship_changes.push(json!({"id":id,"before":before,"after":cargo}));
            continue;
        }
        let price = money(&cargo["goods_price_minor"])?;
        let currency = actor(&cargo["currency_id"])?;
        let (event, total, paid, stock, create, credit_owner, credit_purpose) = if returning {
            let received = cargo
                .get("returned_quantity")
                .map(quantity)
                .transpose()?
                .unwrap_or(0.)
                + q;
            let initial = quantity(&cargo["return_initial_quantity"])?;
            if initial <= 0. || received > initial + 1e-8 {
                return Err(error("Return quantity exceeds real shipment"));
            }
            let refundable = price.0 - money(&cargo["paid_to_seller_minor"])?.0;
            if refundable < 0 {
                return Err(error("Invalid refundable cargo escrow"));
            }
            let total = multiply_money(Money(refundable), received / initial)?;
            let paid = total.0 - money(&cargo["refunded_minor"])?.0;
            let stock = actor(&cargo["stock_id"])?;
            let row = session
                .stock(&stock)
                .ok_or_else(|| error("Return original stock missing"))?;
            if row.owner_id != owner
                || row.goods.external_id() != good
                || row.facility_id.as_ref() != Some(&facility)
            {
                return Err(error("Return original stock rights mismatch"));
            }
            cargo["returned_quantity"] = json!(received);
            cargo["refunded_minor"] = json!(total.0.to_string());
            cargo["remaining_quantity"] = json!((remaining - q).max(0.));
            cargo["status"] = json!(if remaining > q {
                "awaiting_return_storage"
            } else {
                "returned"
            });
            cargo["carrier_release_date"] = json!(current);
            (
                format!("{return_prefix}{}", crate::needs::js_string(&id.json())),
                total,
                paid,
                stock,
                None,
                actor(&cargo["buyer_id"])?,
                cargo["buyer_purpose"].clone(),
            )
        } else {
            let received = quantity(&cargo["arrived_quantity"])? + q;
            let shipped = quantity(&cargo["shipped_quantity"])?;
            if shipped <= 0. || received > shipped + 1e-8 {
                return Err(error("Arrival quantity exceeds real shipment"));
            }
            let total = multiply_money(price, received / shipped)?;
            let paid = total.0 - money(&cargo["paid_to_seller_minor"])?.0;
            let stock = cargo
                .get("destination_stock_id")
                .map(actor)
                .transpose()?
                .unwrap_or_else(|| {
                    ExternalId::from(format!(
                        "shipment:{}:destination",
                        crate::needs::js_string(&id.json())
                    ))
                });
            if let Some(row) = session.stock(&stock) {
                if row.owner_id != owner
                    || row.goods.external_id() != good
                    || row.facility_id.as_ref() != Some(&facility)
                {
                    return Err(error("Shipment destination stock identity mismatch"));
                }
            }
            let mut create = json!({"owner_id":owner,"province_id":cargo["to_province_id"],"goods_id":good,"unit":cargo["unit"],"facility_id":facility,"currency_id":currency,"inventory_origin":"purchased","source_inventory_origin":cargo["goods_origin"]});
            if let Some(purpose) = cargo.get("inventory_purpose") {
                create["inventory_purpose"] = purpose.clone();
                create["beneficiary_pop_id"] = cargo["beneficiary_pop_id"].clone();
            }
            cargo["arrived_quantity"] = json!(received);
            cargo["remaining_quantity"] = json!((shipped - received).max(0.));
            cargo["paid_to_seller_minor"] = json!(total.0.to_string());
            cargo["status"] = json!(if received < shipped {
                "partial_arrived"
            } else {
                "arrived"
            });
            if received < shipped {
                cargo["storage_delayed"] = json!(true);
            } else if cargo["storage_delayed"] == true {
                let days = cargo["return_days"]
                    .as_i64()
                    .filter(|n| *n >= 0)
                    .ok_or_else(|| error("Invalid carrier return duration"))?;
                cargo["carrier_release_date"] = json!(add_days(current, days)?);
            }
            (
                format!("{event_prefix}{}", crate::needs::js_string(&id.json())),
                total,
                paid,
                stock,
                Some(create),
                actor(&cargo["seller_id"])?,
                cargo["seller_purpose"].clone(),
            )
        };
        if paid < 0 {
            return Err(error("Accumulated cargo payment decreased"));
        }
        let _ = total;
        let mut op = json!({"event_id":event,"type":if returning{"shipment_return_arrival"}else{"shipment_arrival"},"shipment_id":id,"quantity":q,"debits":[],"credits":[],"stock_changes":[{"stock_id":stock,"delta":q}]});
        if let Some(create) = create {
            op["stock_changes"][0]["create"] = create;
            op["goods_origin"] = cargo["goods_origin"].clone();
        }
        if paid > 0 {
            op["debits"] = json!([{"owner_id":format!("shipment:{}",crate::needs::js_string(&id.json())),"currency_id":currency,"purpose":"trade_escrow","amount_minor":paid.to_string()}]);
            op["credits"] = json!([{"owner_id":credit_owner,"currency_id":currency,"purpose":credit_purpose,"amount_minor":paid.to_string()}]);
        }
        let op: EconomicOperation = serde_json::from_value(op)?;
        stage_shipping_operations(&mut session, std::slice::from_ref(&op))?;
        operations.push(op);
        reservation.quantity = (old - q).max(0.);
        if reservation.quantity == 0. {
            reservation.status = "consumed".into();
        }
        if !returning {
            let receipts = cargo
                .as_object_mut()
                .unwrap()
                .entry("arrival_receipts")
                .or_insert(json!([]))
                .as_array_mut()
                .ok_or_else(|| error("Invalid arrival receipt list"))?;
            receipts.push(json!({"event_id":event,"quantity":q,"date":current}));
        }
        if let Some(parts) = cargo
            .get_mut("producer_contributions")
            .and_then(Value::as_array_mut)
        {
            for part in parts {
                part["quantity"] =
                    json!(quantity(&part["quantity"])? * (remaining - q).max(0.) / remaining);
            }
        }
        ship_changes.push(json!({"id":id,"before":before,"after":cargo}));
        reserve_changes
            .push(json!({"id":reservation.id,"before":old_reservation,"after":reservation}));
    }
    let registry_changes = json!({"shipments":ship_changes,"storage_reservations":reserve_changes});
    let op: EconomicOperation = serde_json::from_value(
        json!({"event_id":boundary(w,r),"type":"supply_phase_boundary","period":r.period,"stage":"arrivals","date":current,"registry_sha256":digest(&registry_changes)?}),
    )?;
    stage_shipping_operations(&mut session, std::slice::from_ref(&op))?;
    operations.push(op);
    let changes = session.finish()?;
    let mut state=local.map_or_else(||crate::world_transaction::WorldTransaction::new(w),|parent|parent.child());
    state.apply_phase(changes)?;
    state.replace_shipments(shipments)?;
    state.replace_storage_reservations(reservations)?;
    let invariants = crate::shipping_invariants::verify_arrivals(w, &state, &operations)?;
    let receipt = TransportPhaseReceipt {
        campaign_id: w.campaign().campaign_id.clone(),
        request: r.clone(),
        date: current,
        operation_evidence: operations
            .iter()
            .map(operation_evidence)
            .collect::<OpResult<_>>()?,
        input_evidence: vec![],
        operations,
        registry_changes,
        diagnostics,
        decisions: vec![],
        invariants,
    };
    let key=state.transport_phase_receipts().next_key().map_err(crate::operation_session::OperationError)?;
    state.put_transport_phase_receipts(key,receipt.clone())?;
    Ok(PreparedShipmentArrivals {
        reads: Some(crate::world_transaction::PrepareProof::capture(w,local.map(|parent|parent.local_prepare_token()))?),
        state: Some(state.finish()?),
        receipt,
    })
}
pub fn apply_shipment_arrivals(
    w: &mut WorldState,
    p: PreparedShipmentArrivals,
) -> OpResult<TransportPhaseReceipt> {
    crate::world_transaction::execute_atomic(w,|w|apply_shipment_arrivals_in(w,p))
}
pub(crate) fn apply_shipment_arrivals_in(
    w: &mut crate::world_transaction::WorldTransaction<'_>,
    p: PreparedShipmentArrivals,
) -> OpResult<TransportPhaseReceipt> {
    if let Some(reads) = p.reads {
        reads.validate(w)?;
        w.apply_delta(p.state.ok_or_else(|| error("Arrival state missing"))?)?;
    } else if replay(w, &p.receipt.request)?.as_ref() != Some(&p.receipt) {
        return Err(error("Prepared arrival replay altered"));
    }
    Ok(p.receipt)
}
pub fn execute_shipment_arrivals(
    w: &mut WorldState,
    r: &TransportRequest,
) -> OpResult<TransportPhaseReceipt> {
    crate::world_transaction::execute_atomic(w,|w|execute_shipment_arrivals_in(w,r))
}
pub(crate) fn execute_shipment_arrivals_in(
    w: &mut crate::world_transaction::WorldTransaction<'_>,
    r: &TransportRequest,
) -> OpResult<TransportPhaseReceipt> {
    let p = prepare_shipment_arrivals_guarded(w, r, Some(w))?;
    apply_shipment_arrivals_in(w, p)
}

