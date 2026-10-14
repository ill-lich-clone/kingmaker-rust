//! Finite shared space, owned access and saved lease terms. No inventory is created here.
use kingmaker_core::world_read::WorldRead;
use crate::money_math::multiply_money;
use crate::operation_session::*;
use kingmaker_core::operations::*;
use kingmaker_core::storage::*;
use kingmaker_core::{ExternalId, Money};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
fn error(s: &str) -> OperationError {
    OperationError(s.into())
}
pub fn truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_some_and(|n| n != 0.0),
        Value::String(s) => !s.is_empty(),
        _ => true,
    }
}
pub fn approved(status: Option<&str>, source: Option<&Value>, reference: Option<&Value>) -> bool {
    matches!(status, Some("source" | "approved"))
        && source.is_some_and(truthy)
        && (status != Some("approved") || reference.is_some_and(truthy))
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageRequest {
    pub id: ExternalId,
    pub owner_id: ExternalId,
    pub facility_id: ExternalId,
    pub goods_id: ExternalId,
    pub unit: Value,
    pub quantity: f64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageOutcome {
    pub reservation: Option<StorageReservation>,
    pub reason: Option<String>,
    pub partial: bool,
    pub replayed: bool,
}
#[derive(Debug, Clone)]
pub struct StorageAvailability {
    pub free: Option<f64>,
    pub free_space: Option<f64>,
    pub status: &'static str,
    pub unit: Option<Value>,
}
pub struct StorageBatch<'w> {
    pub facilities: HashMap<ExternalId, &'w StorageFacility>,
    remaining: HashMap<ExternalId, f64>,
    invalid: HashSet<ExternalId>,
}
pub struct FacilityCheckpoint {
    id: ExternalId,
    remaining: f64,
    invalid: bool,
}
impl<'w> StorageBatch<'w> {
    pub fn new(s: &OperationSession<'w>) -> OpResult<Self> {
        let mut facilities = HashMap::new();
        for f in s.base.storage_facilities().values() {
            if facilities.insert(f.id.clone(), f).is_some() {
                return Err(error("Duplicate warehouse"));
            }
        }
        let mut used: HashMap<ExternalId, f64> = HashMap::new();
        let mut invalid = HashSet::new();
        let terms = s
            .stocks()
            .filter_map(|r| {
                r.facility_id.as_ref().map(|id| {
                    (
                        id.clone(),
                        r.goods.external_id(),
                        r.unit.clone(),
                        r.quantity,
                    )
                })
            })
            .chain(
                s.storage_reservations()
                    .filter(|r| r.status == "held")
                    .map(|r| {
                        (
                            r.facility_id.clone(),
                            r.goods_id.clone(),
                            Some(r.unit.clone()),
                            r.quantity,
                        )
                    }),
            );
        for (id, good, unit, quantity) in terms {
            if let Some(mapping) = facilities
                .get(&id)
                .and_then(|f| f.goods.get(&external_text(&good)))
            {
                if unit.as_ref() != Some(&mapping.unit)
                    || !quantity.is_finite()
                    || quantity < 0.0
                    || !mapping.space_per_unit.is_finite()
                    || mapping.space_per_unit <= 0.0
                {
                    invalid.insert(id);
                } else {
                    *used.entry(id).or_default() += quantity * mapping.space_per_unit;
                }
            } else {
                invalid.insert(id);
            }
        }
        let remaining = facilities
            .iter()
            .filter_map(|(id, f)| {
                f.capacity
                    .map(|c| (id.clone(), c - used.get(id).copied().unwrap_or(0.0)))
            })
            .collect();
        Ok(Self {
            facilities,
            remaining,
            invalid,
        })
    }
    pub fn availability(&self, facility: &ExternalId, good: &ExternalId) -> StorageAvailability {
        let unknown = || StorageAvailability {
            free: None,
            free_space: None,
            status: "unknown",
            unit: self
                .facilities
                .get(facility)
                .and_then(|f| f.goods.get(&external_text(good)))
                .map(|m| m.unit.clone()),
        };
        let Some(f) = self.facilities.get(facility) else {
            return unknown();
        };
        let Some(m) = f.goods.get(&external_text(good)) else {
            return unknown();
        };
        if !approved(
            f.status.as_deref(),
            f.source.as_ref(),
            f.approval_ref.as_ref(),
        ) || f.capacity.is_none_or(|c| !c.is_finite() || c < 0.0)
            || !f.unit.as_ref().is_some_and(truthy)
            || !truthy(&m.unit)
            || !m.space_per_unit.is_finite()
            || m.space_per_unit <= 0.0
            || self.invalid.contains(facility)
        {
            return unknown();
        }
        let rem = self.remaining[facility];
        StorageAvailability {
            free: Some(rem.max(0.0) / m.space_per_unit),
            free_space: Some(rem.max(0.0)),
            status: if rem < -1e-9 {
                "over_capacity"
            } else {
                "known"
            },
            unit: Some(m.unit.clone()),
        }
    }
    pub fn adjust(&mut self, facility: &ExternalId, space_delta: f64) -> OpResult<()> {
        if !space_delta.is_finite() {
            return Err(error("Invalid space delta"));
        }
        if let Some(r) = self.remaining.get_mut(facility) {
            *r -= space_delta;
        }
        Ok(())
    }
    pub fn checkpoint_facility(&self, id: &ExternalId) -> OpResult<FacilityCheckpoint> {
        Ok(FacilityCheckpoint {
            id: id.clone(),
            remaining: *self.remaining.get(id).ok_or_else(|| error("Unknown warehouse capacity"))?,
            invalid: self.invalid.contains(id),
        })
    }
    pub fn restore_facility(&mut self, checkpoint: FacilityCheckpoint) {
        self.remaining.insert(checkpoint.id.clone(), checkpoint.remaining);
        if checkpoint.invalid { self.invalid.insert(checkpoint.id); }
        else { self.invalid.remove(&checkpoint.id); }
    }
}
pub fn reserve_storage(
    s: &mut OperationSession,
    b: &mut StorageBatch,
    r: &StorageRequest,
    period: &str,
) -> OpResult<StorageOutcome> {
    let result = reserve(s, b, r, period);
    if result.is_err() {
        s.poison();
    }
    result
}
fn reserve(
    s: &mut OperationSession,
    b: &mut StorageBatch,
    r: &StorageRequest,
    period: &str,
) -> OpResult<StorageOutcome> {
    let identity = json!([
        period,
        r.owner_id,
        r.facility_id,
        r.goods_id,
        r.unit,
        crate::needs::js_quantity_value(r.quantity)
    ])
    .to_string();
    if let Some(old) = s.storage_reservation(&r.id) {
        if old.extra.get("request_identity") != Some(&json!(identity)) {
            return Err(error("Storage reservation identity mismatch"));
        }
        return Ok(StorageOutcome {
            reservation: Some(old.clone()),
            reason: None,
            partial: false,
            replayed: true,
        });
    }
    let blocked = |reason: &str| {
        Ok(StorageOutcome {
            reservation: None,
            reason: Some(reason.into()),
            partial: false,
            replayed: false,
        })
    };
    if !r.quantity.is_finite()
        || r.quantity <= 0.0
        || r.id.as_str() == Some("")
        || r.owner_id.as_str() == Some("")
    {
        return blocked("invalid_storage_request");
    }
    let availability = b.availability(&r.facility_id, &r.goods_id);
    let Some(free) = availability.free else {
        return blocked("storage_parameters_unknown");
    };
    if availability.unit.as_ref() != Some(&r.unit) {
        return blocked("storage_unit_mismatch");
    }
    let f = &b.facilities[&r.facility_id];
    let contract = f.access_contracts.iter().find(|c| {
        c.owner_id.as_ref() == Some(&r.owner_id)
            && approved(
                c.status.as_deref(),
                c.source.as_ref(),
                c.approval_ref.as_ref(),
            )
    });
    if f.owner_id != r.owner_id && contract.is_none() {
        return blocked("storage_access_denied");
    }
    let mut quantity = r.quantity.min(free);
    let mut fee = Money::ZERO;
    let space = f.goods[&external_text(&r.goods_id)].space_per_unit;
    if let Some(c) = contract {
        let (Some(price), Some(currency), Some(purpose)) = (
            c.price_minor_per_space,
            c.currency_id.as_ref(),
            c.purpose.as_ref(),
        ) else {
            return blocked("storage_lease_parameters_unknown");
        };
        if price.0 < 0 {
            return Err(error("Negative storage rent"));
        }
        if price.0 > 0
            && !matches!(
                c.cancellation_policy.as_deref(),
                Some("nonrefundable" | "refund_unused")
            )
        {
            return blocked("storage_refund_terms_unknown");
        }
        let cash = s.cash(&r.owner_id, currency, purpose);
        fee = multiply_money(price, quantity * space)?;
        if fee > cash {
            quantity *= cash.0 as f64 / fee.0 as f64;
            fee = multiply_money(price, quantity * space)?;
            for _ in 0..8 {
                if fee <= cash {
                    break;
                }
                quantity *= 1.0 - f64::EPSILON;
                fee = multiply_money(price, quantity * space)?;
            }
            if fee > cash {
                return Err(error("Storage affordability rounding failed"));
            }
        }
    }
    if quantity <= 0.0 {
        return blocked(if free <= 0.0 {
            "storage_full"
        } else {
            "storage_rent_unfunded"
        });
    }
    let row: StorageReservation = serde_json::from_value(
        json!({"id":r.id,"owner_id":r.owner_id,"facility_id":r.facility_id,"goods_id":r.goods_id,"unit":r.unit,"quantity":quantity,"requested_quantity":r.quantity,"accepted_quantity":quantity,"status":"held","period":period,"request_identity":identity,"fee_paid_minor":fee,"fee_currency_id":contract.and_then(|c|c.currency_id.clone()),"lease_terms":contract}),
    )?;
    let lease = row.lease_terms.clone();
    let provider = f.owner_id.clone();
    s.storage_rows.insert(row.id.clone(), row.clone());
    if fee.0 > 0 {
        let c = lease.unwrap();
        let op: EconomicOperation = serde_json::from_value(
            json!({"event_id":format!("storage-rent:{}",external_text(&r.id)),"type":"storage_rent","reservation_id":r.id,"debits":[{"owner_id":r.owner_id,"currency_id":c.currency_id,"purpose":c.purpose,"amount_minor":fee}],"credits":[{"owner_id":provider,"currency_id":c.currency_id,"purpose":"operating_capital","amount_minor":fee}]}),
        )?;
        s.stage(&op)?;
    }
    b.adjust(&r.facility_id, quantity * space)?;
    Ok(StorageOutcome {
        reservation: Some(row),
        reason: None,
        partial: quantity < r.quantity - 1e-9,
        replayed: false,
    })
}
pub fn consume_storage(
    s: &mut OperationSession,
    b: &mut StorageBatch,
    id: &ExternalId,
    quantity: f64,
) -> OpResult<()> {
    let result = (|| {
        let mut row = s
            .storage_reservation(id)
            .ok_or_else(|| error("Unknown output reserve"))?
            .clone();
        if row.status != "held"
            || !quantity.is_finite()
            || quantity < 0.0
            || quantity > row.quantity + 1e-9
        {
            return Err(error("Output exceeds held storage"));
        }
        row.quantity = (row.quantity - quantity).max(0.0);
        row.consumed_quantity = Some(row.consumed_quantity.unwrap_or(0.0) + quantity);
        if row.quantity <= 1e-9 {
            row.status = "consumed".into();
            row.quantity = 0.0;
        }
        b.adjust(
            &row.facility_id,
            -quantity
                * b.facilities[&row.facility_id].goods[&external_text(&row.goods_id)]
                    .space_per_unit,
        )?;
        s.storage_rows.insert(id.clone(), row);
        Ok(())
    })();
    if result.is_err() {
        s.poison();
    }
    result
}
pub fn release_storage(
    s: &mut OperationSession,
    b: &mut StorageBatch,
    id: &ExternalId,
    event_id: &ExternalId,
) -> OpResult<()> {
    let result = (|| {
        let old = s
            .storage_reservation(id)
            .ok_or_else(|| error("Unknown release reserve"))?
            .clone();
        if old.release_event_id.as_ref() == Some(event_id) {
            return Ok(());
        }
        if old.status != "held" || event_id.as_str() == Some("") {
            return Err(error("Storage reserve not held"));
        }
        if old.fee_paid_minor.unwrap_or(Money::ZERO).0 > 0
            && old
                .lease_terms
                .as_ref()
                .and_then(|t| t.cancellation_policy.as_deref())
                == Some("refund_unused")
        {
            let terms = old.lease_terms.as_ref().unwrap();
            let provider = &b.facilities[&old.facility_id].owner_id;
            let currency = terms
                .currency_id
                .as_ref()
                .ok_or_else(|| error("Missing rent currency"))?;
            let purpose = terms
                .purpose
                .as_ref()
                .ok_or_else(|| error("Missing rent purpose"))?;
            let due = multiply_money(
                old.fee_paid_minor.unwrap(),
                old.quantity
                    / old
                        .accepted_quantity
                        .ok_or_else(|| error("Missing accepted quantity"))?,
            )?;
            let paid = due.min(s.cash(provider, currency, "operating_capital"));
            let mut op: EconomicOperation = serde_json::from_value(
                json!({"event_id":format!("{}:refund",external_text(event_id)),"type":"storage_rent_refund","reservation_id":id,"debits":[],"credits":[]}),
            )?;
            if paid.0 > 0 {
                op.debits.push(MoneyLeg {
                    owner_id: provider.clone(),
                    currency_id: currency.clone(),
                    purpose: "operating_capital".into(),
                    amount_minor: paid,
                });
                op.credits.push(MoneyLeg {
                    owner_id: old.owner_id.clone(),
                    currency_id: currency.clone(),
                    purpose: purpose.clone(),
                    amount_minor: paid,
                });
            }
            if due > paid {
                op.obligation = Some(serde_json::from_value(
                    json!({"obligation_id":format!("debt:{}",external_text(event_id)),"amount_minor":Money(due.0-paid.0),"payer_id":provider,"payee_id":old.owner_id,"payee_purpose":purpose,"currency_id":currency,"purpose":"operating_capital","tax_rate":0,"tax_owner_id":null,"category":"lease_refund"}),
                )?);
            }
            s.stage(&op)?;
        }
        let mut row = old.clone();
        row.status = "released".into();
        row.release_event_id = Some(event_id.clone());
        row.released_quantity = Some(old.quantity);
        row.quantity = 0.0;
        b.adjust(
            &old.facility_id,
            -old.quantity
                * b.facilities[&old.facility_id].goods[&external_text(&old.goods_id)]
                    .space_per_unit,
        )?;
        s.storage_rows.insert(id.clone(), row);
        Ok(())
    })();
    if result.is_err() {
        s.poison();
    }
    result
}
