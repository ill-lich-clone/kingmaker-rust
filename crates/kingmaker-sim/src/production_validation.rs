//! Foundation boundary checks. Invalid ready data is never converted into free work.
use kingmaker_core::world_read::WorldRead;
use crate::{operation_session::*, storage::truthy};
use kingmaker_core::production::*;
use kingmaker_core::*;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
fn err(s: &str) -> OperationError {
    OperationError(s.into())
}
pub fn goods_unit(w: &dyn WorldRead, id: &ExternalId) -> OpResult<Value> {
    let c = w.production_catalog()
        .as_ref()
        .ok_or_else(|| err("Missing catalogue"))?;
    let g = c
        .goods
        .iter()
        .find(|g| &g.goods_id == id)
        .ok_or_else(|| err("Unknown recipe goods"))?;
    if let Some(u) = g
        .extra
        .get("unit")
        .or_else(|| g.extra.get("Единица"))
        .filter(|u| truthy(u) && u.get("__kingmaker_codec_v2__").is_none())
    {
        return Ok(u.clone());
    }
    let units: Vec<_> = c
        .methods
        .iter()
        .filter(|m| &m.output_goods_id == id && truthy(&m.output_unit))
        .map(|m| &m.output_unit)
        .collect();
    let first = units
        .first()
        .ok_or_else(|| err("Authoritative commodity unit unknown"))?;
    if units.iter().any(|u| u != first) {
        return Err(err("Commodity method units disagree"));
    }
    Ok((*first).clone())
}
pub fn validate_production_catalog(w: &dyn WorldRead) -> OpResult<()> {
    let c = w.production_catalog()
        .as_ref()
        .ok_or_else(|| err("Missing production catalogue"))?;
    let goods: HashMap<_, _> = c.goods.iter().map(|g| (&g.goods_id, g)).collect();
    if goods.len() != c.goods.len() {
        return Err(err("Duplicate catalogue goods"));
    }
    let mut ids = HashSet::new();
    for m in &c.methods {
        if !ids.insert(&m.method_id)
            || !matches!(m.kind.as_str(), "rgo" | "factory" | "artisan")
            || !goods.contains_key(&m.output_goods_id)
        {
            return Err(err("Invalid or duplicate production method"));
        }
        if !truthy(&m.output_unit) {
            return Err(err("Output unit required"));
        }
        if m.status == "ready"
            && m.weekly_productivity
                .is_none_or(|n| !n.is_finite() || n <= 0.0)
        {
            return Err(err("Positive productivity required"));
        }
        if m.inputs
            .iter()
            .any(|i| !goods.contains_key(&i.goods_id) || !i.amount.is_finite() || i.amount <= 0.0)
        {
            return Err(err("Invalid production inputs"));
        }
        if m.kind == "rgo" && !m.inputs.is_empty() {
            return Err(err("Raw inputs forbidden"));
        }
        if m.kind != "rgo" && m.status == "ready" && m.inputs.is_empty() {
            return Err(err("Processed inputs required"));
        }
        if m.output_goods_id.as_str() == Some("steel")
            && m.kind != "rgo"
            && (m.inputs.len() != 2
                || !m
                    .inputs
                    .iter()
                    .any(|i| i.goods_id.as_str() == Some("iron") && i.amount == 1.1)
                || !m
                    .inputs
                    .iter()
                    .any(|i| i.goods_id.as_str() == Some("coal") && i.amount == 0.4))
        {
            return Err(err("Authored steel recipe required"));
        }
        let staff = match m.kind.as_str() {
            "artisan" => vec!["artisans"],
            "factory" => vec!["craftsmen", "clerks"],
            _ => vec!["farmers", "laborers", "slaves"],
        };
        if m.staffing.is_empty()
            || m.staffing
                .iter()
                .any(|s| s.as_str().is_none_or(|s| !staff.contains(&s)))
        {
            return Err(err("Invalid method staff roles"));
        }
        if let Some(capacity) = m.extra.get("capacity").filter(|v| truthy(v)) {
            if capacity
                .get("base_people")
                .and_then(Value::as_f64)
                .is_none_or(|n| !n.is_finite() || n <= 0.0 || n.fract() != 0.0)
            {
                return Err(err("Invalid authored method capacity"));
            }
        }
        let constructions = m.extra.get("construction").into_iter().chain(
            m.extra
                .get("construction_by_currency")
                .and_then(Value::as_object)
                .into_iter()
                .flat_map(|v| v.values()),
        );
        for terms in
            constructions.filter(|v| v.get("status").and_then(Value::as_str) == Some("ready"))
        {
            let money = terms
                .get("cost_minor")
                .ok_or_else(|| err("Missing construction price"))?;
            let text = money
                .as_str()
                .map(str::to_owned)
                .or_else(|| {
                    money
                        .get("value")
                        .and_then(Value::as_str)
                        .map(str::to_owned)
                })
                .unwrap_or_else(|| money.to_string());
            let price: num_bigint::BigInt = text
                .parse()
                .map_err(|_| err("Invalid construction price"))?;
            if price <= 0.into() {
                return Err(err("Positive construction price required"));
            }
            if !terms.get("currency_id").is_some_and(truthy)
                || terms
                    .get("duration_days")
                    .and_then(Value::as_f64)
                    .is_none_or(|d| d <= 0.0 || d.fract() != 0.0)
                || terms
                    .get("materials")
                    .and_then(Value::as_array)
                    .is_none_or(|rows| {
                        rows.iter().any(|r| {
                            r.get("goods_id")
                                .cloned()
                                .and_then(|v| ExternalId::from_json(v).ok())
                                .is_none_or(|id| !goods.contains_key(&id))
                                || r.get("amount")
                                    .and_then(Value::as_f64)
                                    .is_none_or(|n| !n.is_finite() || n <= 0.0)
                        })
                    })
            {
                return Err(err("Invalid construction terms"));
            }
        }
    }
    Ok(())
}
pub fn maintenance_verified(
    s: &OperationSession,
    request: &ProductionRequest,
    receipt: &MaintenanceReceipt,
) -> bool {
    if request.expected_revision != s.base.campaign().revision
        || receipt.period.as_deref() != Some(&request.period)
        || !receipt.fraction.is_finite()
        || receipt.fraction <= 0.0
        || receipt.fraction > 1.0
        || receipt.event_ids.is_empty()
    {
        return false;
    }
    // Once authored needs exist, successful food events alone cannot prove full maintenance.
    if s.base.needs().is_some() {
        return s
            .native_maintenance
            .get_or_init(|| native_maintenance_evidence(s, request))
            .as_ref()
            .is_some_and(|e| {
                e.period == request.period
                    && e.revision == request.expected_revision
                    && e.receipts.get(&receipt.pop_id) == Some(receipt)
            });
    }
    let mut ids = HashSet::new();
    let mut orders = HashSet::new();
    let mut needs: HashMap<ExternalId, (f64, f64)> = HashMap::new();
    let prefix = format!(
        "market:{}:{}:maintenance:",
        s.base.campaign().campaign_id, request.period
    );
    for id in &receipt.event_ids {
        if !ids.insert(id) {
            return false;
        }
        let Some(op) = s.committed(id) else {
            return false;
        };
        if op.get("type").and_then(Value::as_str) != Some("purchase")
            || op.get("maintenance") != Some(&Value::Bool(true))
            || op.get("consumed") != Some(&Value::Bool(true))
            || op.get("pop_id") != Some(&receipt.pop_id.json())
            || !id.as_str().is_some_and(|id| id.starts_with(&prefix))
        {
            return false;
        }
        if op
            .get("period")
            .and_then(Value::as_str)
            .is_some_and(|p| p != request.period && p != format!("{}:maintenance", request.period))
        {
            return false;
        }
        let (Some(q), Some(required), Some(need), Some(order)) = (
            op.get("quantity").and_then(Value::as_f64),
            op.get("requested_quantity").and_then(Value::as_f64),
            op.get("need_id")
                .cloned()
                .and_then(|v| ExternalId::from_json(v).ok()),
            op.get("order_id")
                .cloned()
                .and_then(|v| ExternalId::from_json(v).ok()),
        ) else {
            return false;
        };
        if !q.is_finite()
            || q <= 0.0
            || !required.is_finite()
            || required <= 0.0
            || q > required + 1e-9
            || !orders.insert(order)
        {
            return false;
        }
        let Some(changes) = op.get("stock_changes").and_then(Value::as_array) else {
            return false;
        };
        if changes.is_empty()
            || changes.iter().any(|d| {
                d.get("delta")
                    .and_then(Value::as_f64)
                    .is_none_or(|v| !v.is_finite() || v >= 0.0)
            })
        {
            return false;
        }
        let totals = needs.entry(need).or_default();
        totals.0 += q;
        totals.1 += required;
    }
    !needs.is_empty()
        && needs
            .values()
            .all(|(actual, required)| receipt.fraction <= (*actual / *required).min(1.0) + 1e-9)
}

pub(crate) struct NativeMaintenanceEvidence {
    period: String,
    revision: u64,
    receipts: HashMap<ExternalId, MaintenanceReceipt>,
}
fn native_maintenance_evidence(
    s: &OperationSession,
    request: &ProductionRequest,
) -> Option<NativeMaintenanceEvidence> {
    let phase = s.base.market_phase_receipts().values().find(|p| {
        p.request.stage == kingmaker_core::market::MarketStage::Maintenance
            && p.campaign_id == s.base.campaign().campaign_id
            && p.request.period == request.period
            && p.request.expected_revision == request.expected_revision
    })?;
    if !phase.invariants.production_required_passed() {
        return None;
    }
    let current = crate::needs::build_need_orders(s.base, &phase.request.profile).ok()?;
    let slaves = s
        .base.pops().values()
        .filter(|p| p.pop_type_id.as_str() == Some("slaves"))
        .map(|p| p.external_id.clone())
        .collect::<HashSet<_>>();
    let rows = current
        .rows
        .into_iter()
        .filter(|r| {
            r.extra
                .get("pop_id")
                .and_then(|v| ExternalId::from_json(v.clone()).ok())
                .is_some_and(|id| slaves.contains(&id))
        })
        .collect::<Vec<_>>();
    if rows != phase.frozen_needs {
        return None;
    }
    let mut events = HashSet::new();
    if !phase.operations.iter().all(|op| {
        events.insert(&op.event_id)
            && crate::operation_session::canonical_operation(op)
                .is_ok_and(|value| s.committed(&op.event_id).as_deref() == Some(&value))
    }) {
        return None;
    }
    let mut receipts = HashMap::new();
    for r in &phase.maintenance_receipts {
        if receipts.insert(r.pop_id.clone(), r.clone()).is_some() {
            return None;
        }
    }
    Some(NativeMaintenanceEvidence {
        period: request.period.clone(),
        revision: request.expected_revision,
        receipts,
    })
}
/// Ordinary string-only phases keep JS names; typed phases use injective JSON identities.
pub fn financial_event_id(
    s: &OperationSession,
    kind: &str,
    period: &str,
    id: &ExternalId,
    typed: bool,
) -> ExternalId {
    let identity = if typed {
        serde_json::json!([
            if matches!(id, ExternalId::Number(_)) {
                "number"
            } else {
                "string"
            },
            id
        ])
        .to_string()
    } else {
        external_text(id)
    };
    format!("{kind}:{}:{period}:{identity}", s.base.campaign().campaign_id).into()
}
