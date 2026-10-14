//! Quantities follow real stock operations, independently of ownership or cash flows.
use kingmaker_core::row_store::ReadSequence;
use kingmaker_core::world_read::WorldRead;
use crate::{budget_funding::error, operation_session::OpResult, transport_validation::actor};
use kingmaker_core::{
    income::StockContribution,
    operations::{EconomicOperation, OperationKind},
    ExternalId, ProvinceId,
};
use std::collections::{HashMap, HashSet};
struct Lot {
    quantity: f64,
    goods: ExternalId,
    parts: HashMap<ExternalId, f64>,
}
fn initial(
    w: &dyn WorldRead,
    stocks: &(impl ReadSequence<kingmaker_core::StockpileRecord> + ?Sized),
    prior: &(impl ReadSequence<StockContribution> + ?Sized),
) -> OpResult<HashMap<ExternalId, Lot>> {
    let creators: HashSet<_> = w.artisans().values()
        .map(|e| &e.base.id)
        .collect();
    let mut lots: HashMap<_, _> = stocks
        .iter()
        .map(|s| {
            (
                s.external_id.clone(),
                Lot {
                    quantity: s.quantity,
                    goods: s.goods.external_id(),
                    parts: HashMap::new(),
                },
            )
        })
        .collect();
    if lots.len() != stocks.len() {
        return Err(error("Duplicate provenance stock identity"));
    }
    for c in prior.iter() {
        if !c.quantity.is_finite() || c.quantity < 0. || !creators.contains(&c.producer_id) {
            return Err(error("Invalid stock producer provenance"));
        }
        let lot = lots
            .get_mut(&c.stock_id)
            .ok_or_else(|| error("Producer provenance stock missing"))?;
        if lot
            .parts
            .insert(c.producer_id.clone(), c.quantity)
            .is_some()
        {
            return Err(error("Duplicate stock producer provenance"));
        }
    }
    let methods: HashMap<_, _> = w.production_catalog()
        .iter()
        .flat_map(|c| &c.methods)
        .map(|m| (&m.method_id, m))
        .collect();
    let mut candidates: HashMap<
        (ExternalId, ProvinceId, ExternalId),
        Vec<&kingmaker_core::production::Producer>,
    > = HashMap::new();
    for e in w.artisans().values() {
        if let (Some(owner), Some(m)) = (&e.base.owner_id, methods.get(&e.base.method_id)) {
            candidates
                .entry((owner.clone(), e.base.province_id, m.output_goods_id.clone()))
                .or_default()
                .push(&e.base);
        }
    }
    for stock in stocks.iter() {
        let lot = lots.get_mut(&stock.external_id).unwrap();
        if !lot.quantity.is_finite()
            || lot.quantity < 0.
            || lot.parts.values().sum::<f64>() > lot.quantity + 1e-8 * lot.quantity.max(1.)
        {
            return Err(error("Stock provenance exceeds real quantity"));
        }
        if lot.parts.is_empty()
            && stock.inventory_origin.as_deref() == Some("produced")
            && stock.method_kind.as_deref().is_none_or(|k| k == "artisan")
        {
            if let Some(rows) = candidates.get(&(
                stock.owner_id.clone(),
                stock.province_id,
                stock.goods.external_id(),
            )) {
                let rows: Vec<_> = rows
                    .iter()
                    .filter(|e| stock.facility_id.is_none() || e.facility_id == stock.facility_id)
                    .collect();
                if rows.len() == 1 {
                    lot.parts.insert(rows[0].id.clone(), stock.quantity);
                }
            }
        }
    }
    Ok(lots)
}
fn rows(lots: HashMap<ExternalId, Lot>) -> Vec<StockContribution> {
    let mut rows: Vec<_> =
        lots.into_iter()
            .flat_map(|(stock_id, lot)| {
                lot.parts.into_iter().filter(|(_, q)| *q > 0.).map(
                    move |(producer_id, quantity)| StockContribution {
                        stock_id: stock_id.clone(),
                        producer_id,
                        quantity,
                    },
                )
            })
            .collect();
    rows.sort_by_key(|c| {
        (
            c.stock_id.json().to_string(),
            c.producer_id.json().to_string(),
        )
    });
    rows
}
pub(crate) fn capture_stock_provenance(w: &dyn WorldRead) -> OpResult<Vec<StockContribution>> {
    Ok(rows(initial(
        w,
        &w.stockpiles(),
        &w.stock_contributions(),
    )?))
}
pub(crate) fn next_stock_provenance(
    w: &dyn WorldRead,
    operations: &(impl ReadSequence<EconomicOperation> + ?Sized),
) -> OpResult<Vec<StockContribution>> {
    next_stock_provenance_from(
        w,
        &w.stockpiles(),
        &w.stock_contributions(),
        operations,
    )
}
pub(crate) fn next_stock_provenance_from(
    w: &dyn WorldRead,
    stocks: &(impl ReadSequence<kingmaker_core::StockpileRecord> + ?Sized),
    prior: &(impl ReadSequence<StockContribution> + ?Sized),
    operations: &(impl ReadSequence<EconomicOperation> + ?Sized),
) -> OpResult<Vec<StockContribution>> {
    next_stock_provenance_from_cargo(w, stocks, prior, operations, &w.shipments())
}
pub(crate) fn next_stock_provenance_from_cargo(
    w: &dyn WorldRead,
    stocks: &(impl ReadSequence<kingmaker_core::StockpileRecord> + ?Sized),
    prior: &(impl ReadSequence<StockContribution> + ?Sized),
    operations: &(impl ReadSequence<EconomicOperation> + ?Sized),
    shipments: &(impl ReadSequence<serde_json::Value> + ?Sized),
) -> OpResult<Vec<StockContribution>> {
    let mut lots = initial(w, stocks, prior)?;
    let creators: HashSet<_> = w.artisans().values()
        .map(|e| &e.base.id)
        .collect();
    let mut cargo_used = HashMap::<ExternalId, f64>::new();
    for op in operations.iter() {
        let mut removed: HashMap<ExternalId, (f64, HashMap<ExternalId, f64>)> = HashMap::new();
        for change in op.stock_changes.iter().filter(|c| c.delta < 0.) {
            let lot = lots
                .get_mut(&change.stock_id)
                .ok_or_else(|| error("Provenance debit stock missing"))?;
            let q = -change.delta;
            if !q.is_finite() || q > lot.quantity + 1e-8 * lot.quantity.max(1.) {
                return Err(error("Invalid provenance debit quantity"));
            }
            let fraction = if lot.quantity > 0. {
                (q / lot.quantity).min(1.)
            } else {
                0.
            };
            let moved = removed.entry(lot.goods.clone()).or_default();
            moved.0 += q;
            for (id, part) in &mut lot.parts {
                let quantity = *part * fraction;
                *part -= quantity;
                *moved.1.entry(id.clone()).or_default() += quantity;
            }
            lot.quantity = (lot.quantity - q).max(0.);
        }
        if matches!(
            op.kind,
            OperationKind::ShipmentArrival | OperationKind::ShipmentReturnArrival
        ) {
            let shipment_id = actor(
                op.meta
                    .get("shipment_id")
                    .ok_or_else(|| error("Arrival cargo identity missing"))?,
            )?;
            let shipment = shipments
                .iter()
                .find(|s| s["id"] == shipment_id.json())
                .ok_or_else(|| error("Arrival provenance requires real cargo"))?;
            let goods = actor(&shipment["goods_id"])?;
            let remaining = shipment["remaining_quantity"]
                .as_f64()
                .filter(|n| n.is_finite() && *n >= 0.)
                .ok_or_else(|| error("Invalid cargo provenance quantity"))?;
            let quantity = op
                .stock_changes
                .iter()
                .filter(|c| c.delta > 0.)
                .map(|c| c.delta)
                .sum::<f64>();
            let used = cargo_used.entry(shipment_id).or_default();
            if !quantity.is_finite()
                || quantity <= 0.
                || *used + quantity > remaining + 1e-8 * remaining.max(1.)
            {
                return Err(error("Arrival exceeds real cargo provenance"));
            }
            *used += quantity;
            let mut parts = HashMap::new();
            if let Some(contributions) = shipment
                .get("producer_contributions")
                .and_then(serde_json::Value::as_array)
            {
                for part in contributions {
                    let id = actor(&part["producer_id"])?;
                    let q = part["quantity"]
                        .as_f64()
                        .filter(|n| n.is_finite() && *n >= 0.)
                        .ok_or_else(|| error("Invalid cargo producer contribution"))?;
                    if !creators.contains(&id)
                        || parts.insert(id, q * quantity / remaining).is_some()
                    {
                        return Err(error("Invalid cargo producer rights"));
                    }
                }
                if contributions
                    .iter()
                    .filter_map(|p| p["quantity"].as_f64())
                    .sum::<f64>()
                    > remaining + 1e-8 * remaining.max(1.)
                {
                    return Err(error("Cargo producer provenance exceeds quantity"));
                }
            }
            removed.insert(goods, (quantity, parts));
        }
        let transfer = matches!(
            op.kind,
            OperationKind::Purchase
                | OperationKind::PopCohortSplit
                | OperationKind::PopCohortClaim
                | OperationKind::ShipmentArrival
                | OperationKind::ShipmentDispatch
                | OperationKind::ShipmentRecovery
                | OperationKind::ShipmentReturnDispatch
                | OperationKind::ShipmentReturnArrival
        );
        let mut destinations: HashMap<ExternalId, f64> = HashMap::new();
        for change in op.stock_changes.iter().filter(|c| c.delta > 0.) {
            let goods = lots
                .get(&change.stock_id)
                .map(|l| l.goods.clone())
                .or_else(|| change.create.as_ref().map(|c| c.goods_id.clone()))
                .ok_or_else(|| error("Provenance destination goods missing"))?;
            *destinations.entry(goods).or_default() += change.delta;
        }
        if transfer
            && destinations.iter().any(|(good, q)| {
                removed
                    .get(good)
                    .is_none_or(|(source, _)| *q > *source + 1e-8 * source.max(1.))
            })
        {
            return Err(error("Stock transfer created unexplained provenance goods"));
        }
        let subject = if op.kind == OperationKind::Production {
            op.meta
                .get("subject_id")
                .map(actor)
                .transpose()?
                .filter(|id| creators.contains(id))
        } else {
            None
        };
        for change in op.stock_changes.iter().filter(|c| c.delta > 0.) {
            if !change.delta.is_finite() {
                return Err(error("Invalid provenance credit quantity"));
            }
            if !lots.contains_key(&change.stock_id) {
                let create = change
                    .create
                    .as_ref()
                    .ok_or_else(|| error("Provenance new stock metadata missing"))?;
                lots.insert(
                    change.stock_id.clone(),
                    Lot {
                        quantity: 0.,
                        goods: create.goods_id.clone(),
                        parts: HashMap::new(),
                    },
                );
            }
            let lot = lots.get_mut(&change.stock_id).unwrap();
            if let Some(id) = &subject {
                *lot.parts.entry(id.clone()).or_default() += change.delta;
            } else if transfer {
                if let Some((quantity, parts)) = removed.get(&lot.goods).filter(|(q, _)| *q > 0.) {
                    for (id, q) in parts {
                        *lot.parts.entry(id.clone()).or_default() += q * change.delta / quantity;
                    }
                }
            }
            lot.quantity += change.delta;
            if !lot.quantity.is_finite() {
                return Err(error("Stock provenance quantity overflow"));
            }
        }
    }
    Ok(rows(lots))
}
