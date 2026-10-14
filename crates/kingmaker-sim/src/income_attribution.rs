//! Each physical sale is recognized once; pooled output retains actual producer quantities.
use kingmaker_core::world_read::WorldRead;
use crate::{
    budget_funding::error, income_basis::IncomeOpeningBasis, market_operations::invoice,
    money_math::multiply_money, operation_session::OpResult, transport_validation::actor,
};
use kingmaker_core::{
    income::StockContribution,
    operations::{EconomicOperation, OperationKind},
    ExternalId, Money, StockpileRecord,
};
use num_bigint::BigInt;
use serde_json::{json, Value};
use std::collections::HashMap;

#[derive(Default)]
pub(crate) struct ArtisanAmounts {
    pub revenue: BigInt,
    pub opening_revenue: BigInt,
    pub cost: BigInt,
    pub spent: BigInt,
    pub unresolved: bool,
}
pub(crate) struct Attribution {
    pub amounts: Vec<ArtisanAmounts>,
    pub contributions: Vec<StockContribution>,
    pub diagnostics: Vec<Value>,
}
struct Lot {
    quantity: f64,
    parts: Vec<(ExternalId, f64)>,
}

fn candidates(w: &dyn WorldRead, stock: &StockpileRecord) -> Vec<usize> {
    w.artisans().values()
        .enumerate()
        .filter_map(|(i, e)| {
            let m = w.production_catalog()
                .as_ref()?
                .methods
                .iter()
                .find(|m| m.method_id == e.base.method_id)?;
            (e.base.owner_id.as_ref() == Some(&stock.owner_id)
                && e.base.province_id == stock.province_id
                && m.output_goods_id == stock.goods.external_id()
                && (stock.facility_id.is_none()
                    || e.base.facility_id.as_ref() == stock.facility_id.as_ref()))
            .then_some(i)
        })
        .collect()
}
fn split(amount: Money, weights: &[f64], total: f64) -> OpResult<Vec<BigInt>> {
    if total <= 0. || !total.is_finite() || weights.iter().any(|x| !x.is_finite() || *x < 0.) {
        return Err(error("Invalid producer attribution quantities"));
    }
    let known: f64 = weights.iter().sum();
    if known > total + 1e-8 * total.max(1.) {
        return Err(error("Producer contributions exceed stock"));
    }
    let mut left = amount.0;
    let mut result = vec![];
    for (i, weight) in weights.iter().enumerate() {
        let part = if i + 1 == weights.len() && (known - total).abs() <= 1e-8 * total.max(1.) {
            Money(left)
        } else {
            multiply_money(amount, *weight / total)?
        };
        if part.0 < 0 || part.0 > left {
            return Err(error("Attribution exceeds real payment"));
        }
        left -= part.0;
        result.push(BigInt::from(part.0));
    }
    Ok(result)
}
pub(crate) fn attribute_artisan_operations(
    w: &dyn WorldRead,
    basis: &IncomeOpeningBasis,
    operations: &[EconomicOperation],
) -> OpResult<Attribution> {
    let ids: HashMap<_, _> = w.artisans().values()
        .enumerate()
        .map(|(i, e)| (e.base.id.clone(), i))
        .collect();
    let mut reservation_subjects = HashMap::new();
    for receipt in w.production_phase_receipts().values() {
        for activity in &receipt.procurement_activities {
            if let Some(i) = ids.get(&activity.subject_id) {
                let reservation: ExternalId =
                    format!("output:{}:{}", receipt.request.period, activity.activity_id).into();
                if reservation_subjects.insert(reservation, *i).is_some() {
                    return Err(error("Ambiguous artisan output reservation"));
                }
            }
        }
    }
    let stocks: HashMap<_, _> = basis
        .stocks
        .iter()
        .chain(&w.stockpiles())
        .map(|s| (s.external_id.clone(), s))
        .collect();
    let mut result = Attribution {
        amounts: (0..ids.len()).map(|_| ArtisanAmounts::default()).collect(),
        contributions: vec![],
        diagnostics: vec![],
    };
    let mut lots: HashMap<_, _> = basis
        .stocks
        .iter()
        .map(|s| {
            (
                s.external_id.clone(),
                Lot {
                    quantity: s.quantity,
                    parts: vec![],
                },
            )
        })
        .collect();
    for c in &basis.stock_contributions {
        let lot = lots
            .get_mut(&c.stock_id)
            .ok_or_else(|| error("Contribution stock missing"))?;
        if !ids.contains_key(&c.producer_id)
            || !c.quantity.is_finite()
            || c.quantity < 0.
            || lot.parts.iter().any(|p| p.0 == c.producer_id)
        {
            return Err(error("Invalid producer contribution"));
        }
        lot.parts.push((c.producer_id.clone(), c.quantity));
    }
    for s in &basis.stocks {
        let lot = lots.get_mut(&s.external_id).unwrap();
        if lot.parts.iter().map(|p| p.1).sum::<f64>() > lot.quantity + 1e-8 * lot.quantity.max(1.) {
            return Err(error("Producer contributions exceed opening inventory"));
        }
    }
    for op in operations {
        let subject = op.meta.get("subject_id").map(actor).transpose()?;
        let direct = subject.as_ref().and_then(|id| ids.get(id)).copied();
        if op.kind == OperationKind::ShipmentArrival {
            let cargo_id = actor(&op.meta["shipment_id"])?;
            let cargo = basis
                .shipments
                .iter()
                .find(|s| s["id"] == cargo_id.json())
                .ok_or_else(|| error("Artisan arrival requires frozen opening cargo"))?;
            for credit in &op.credits {
                let owners = w.artisans().values()
                    .enumerate()
                    .filter(|(_, e)| e.base.owner_id.as_ref() == Some(&credit.owner_id))
                    .map(|(i, _)| i)
                    .collect::<Vec<_>>();
                if owners.is_empty() {
                    continue;
                }
                if cargo["goods_origin"] == json!("produced") {
                    let parts = cargo["producer_contributions"]
                        .as_array()
                        .cloned()
                        .unwrap_or_default();
                    let mut creator_ids = vec![];
                    let mut quantities = vec![];
                    for p in parts {
                        let id = actor(&p["producer_id"])?;
                        let i = *ids
                            .get(&id)
                            .ok_or_else(|| error("Cargo artisan creator missing"))?;
                        if !owners.contains(&i) {
                            return Err(error("Cargo artisan seller title mismatch"));
                        }
                        let q = p["quantity"]
                            .as_f64()
                            .filter(|q| q.is_finite() && *q >= 0.)
                            .ok_or_else(|| error("Invalid artisan cargo attribution"))?;
                        creator_ids.push(i);
                        quantities.push(q);
                    }
                    let quantity = cargo["remaining_quantity"]
                        .as_f64()
                        .filter(|q| q.is_finite() && *q > 0.)
                        .ok_or_else(|| error("Invalid frozen artisan cargo quantity"))?;
                    let amounts = split(credit.amount_minor, &quantities, quantity)?;
                    let recognized = amounts.iter().sum::<BigInt>();
                    for (i, amount) in creator_ids.into_iter().zip(amounts) {
                        result.amounts[i].revenue += amount;
                    }
                    if recognized != BigInt::from(credit.amount_minor.0) {
                        for i in owners {
                            result.amounts[i].unresolved = true;
                        }
                        result.diagnostics.push(json!({"shipment_id":cargo_id,"reason":"artisan_arrival_provenance_unresolved","measurement":"нет измерения"}));
                    }
                } else {
                    let stock_id = actor(&cargo["stock_id"])?;
                    let possible = stocks
                        .get(&stock_id)
                        .map(|s| candidates(w, s))
                        .unwrap_or_default();
                    if possible.len() == 1 {
                        result.amounts[possible[0]].opening_revenue +=
                            BigInt::from(credit.amount_minor.0);
                    } else {
                        result.diagnostics.push(json!({"shipment_id":cargo_id,"reason":"artisan_opening_arrival_producer_unresolved","measurement":"нет измерения"}));
                    }
                }
            }
        }
        if op.kind == OperationKind::Purchase {
            for credit in &op.credits {
                let sold: Vec<_> = op
                    .stock_changes
                    .iter()
                    .filter(|c| {
                        c.delta < 0.
                            && stocks
                                .get(&c.stock_id)
                                .is_some_and(|s| s.owner_id == credit.owner_id)
                    })
                    .collect();
                let total: f64 = sold.iter().map(|c| -c.delta).sum();
                if total <= 0. {
                    continue;
                }
                let stock_amounts = split(
                    credit.amount_minor,
                    &sold.iter().map(|c| -c.delta).collect::<Vec<_>>(),
                    total,
                )?;
                for (change, amount) in sold.iter().zip(stock_amounts) {
                    let stock = stocks[&change.stock_id];
                    let owners: Vec<_> = w.artisans().values()
                        .enumerate()
                        .filter(|(_, e)| e.base.owner_id.as_ref() == Some(&stock.owner_id))
                        .map(|(i, _)| i)
                        .collect();
                    if owners.is_empty() {
                        continue;
                    }
                    let amount = Money(
                        num_traits::ToPrimitive::to_i64(&amount)
                            .ok_or_else(|| error("Income allocation overflow"))?,
                    );
                    if stock.inventory_origin.as_deref() != Some("produced") {
                        let possible = candidates(w, stock);
                        if possible.len() == 1 {
                            result.amounts[possible[0]].opening_revenue += BigInt::from(amount.0);
                        } else {
                            result.diagnostics.push(json!({"stock_id":stock.external_id,"reason":"opening_sale_producer_unresolved","measurement":"нет измерения"}));
                        }
                        continue;
                    }
                    let lot = lots
                        .get(&change.stock_id)
                        .ok_or_else(|| error("Sold produced lot missing"))?;
                    let parts = split(
                        amount,
                        &lot.parts.iter().map(|p| p.1).collect::<Vec<_>>(),
                        lot.quantity,
                    )?;
                    let attributed = parts.iter().sum::<BigInt>();
                    for ((id, _), part) in lot.parts.iter().zip(parts) {
                        result.amounts[ids[id]].revenue += part;
                    }
                    if attributed != BigInt::from(amount.0) {
                        for i in owners {
                            result.amounts[i].unresolved = true;
                        }
                        result.diagnostics.push(json!({"stock_id":stock.external_id,"reason":"produced_sale_provenance_unresolved","measurement":"нет измерения"}));
                    }
                }
            }
        }
        if op.kind == OperationKind::Production {
            if let Some(i) = direct {
                let e = w.artisans().at(i).expect("validated row position");
                for change in op.stock_changes.iter().filter(|c| c.delta < 0.) {
                    let stock = stocks
                        .get(&change.stock_id)
                        .ok_or_else(|| error("Artisan input stock missing"))?;
                    let currency = w.accounts().values()
                        .find(|a| {
                            e.base.owner_id.as_ref() == Some(&a.owner_id)
                                && a.purpose
                                    == e.base
                                        .payment_purpose
                                        .as_deref()
                                        .unwrap_or("operating_capital")
                        })
                        .map(|a| a.currency_id.clone())
                        .or_else(|| {
                            e.working_capital
                                .as_ref()?
                                .get("currency_id")
                                .and_then(|v| actor(v).ok())
                        });
                    let price = currency
                        .as_ref()
                        .and_then(|cur| {
                            w.price_calibration()
                                .as_ref()?
                                .prices_by_currency
                                .get(&crate::needs::js_string(&cur.json()))?
                                .get(&crate::needs::js_string(&stock.goods.external_id().json()))?
                                .as_f64()
                        })
                        .filter(|x| x.is_finite() && *x > 0.);
                    if let Some(price) = price {
                        result.amounts[i].cost += invoice(-change.delta, price)?;
                    } else {
                        result.amounts[i].unresolved = true;
                        result.diagnostics.push(json!({"id":e.base.id,"stock_id":stock.external_id,"reason":"artisan_input_cost_basis_unresolved","measurement":"нет измерения"}));
                    }
                }
            }
        }
        for debit in &op.debits {
            let owners: Vec<_> = w.artisans().values()
                .enumerate()
                .filter(|(_, e)| e.base.owner_id.as_ref() == Some(&debit.owner_id))
                .map(|(i, _)| i)
                .collect();
            if owners.is_empty() {
                continue;
            }
            let storage_subject = if op.kind == OperationKind::StorageRent {
                op.meta
                    .get("reservation_id")
                    .map(actor)
                    .transpose()?
                    .as_ref()
                    .and_then(|id| reservation_subjects.get(id))
                    .copied()
            } else {
                None
            };
            let target = direct
                .or(storage_subject)
                .filter(|i| owners.contains(i))
                .or_else(|| (owners.len() == 1).then(|| owners[0]));
            let cost = matches!(
                op.kind,
                OperationKind::StorageRent
                    | OperationKind::ArtisanRetooling
                    | OperationKind::ShipmentDispatch
                    | OperationKind::ShipmentRecovery
                    | OperationKind::ShipmentReturnDispatch
            );
            let spent = debit.purpose == "operating_capital"
                && op.kind != OperationKind::EnterpriseIncome
                && op.kind != OperationKind::ArtisanCapitalRelease
                && !(op.kind == OperationKind::ArtisanRetooling
                    && crate::needs::js_string(&op.event_id.json())
                        .starts_with("artisan-opening-choice:"));
            if cost || spent {
                if let Some(i) = target {
                    if cost {
                        result.amounts[i].cost += BigInt::from(debit.amount_minor.0);
                    }
                    if spent {
                        result.amounts[i].spent += BigInt::from(debit.amount_minor.0);
                    }
                } else {
                    for i in owners {
                        result.amounts[i].unresolved = true;
                    }
                    result.diagnostics.push(json!({"event_id":op.event_id,"reason":"shared_artisan_debit_producer_unresolved","measurement":"нет измерения"}));
                }
            }
        }
        for change in &op.stock_changes {
            let lot = lots.entry(change.stock_id.clone()).or_insert(Lot {
                quantity: 0.,
                parts: vec![],
            });
            if !change.delta.is_finite() || lot.quantity + change.delta < -1e-8 {
                return Err(error("Invalid contribution inventory delta"));
            }
            if change.delta < 0. {
                let fraction = if lot.quantity > 0. {
                    (lot.quantity + change.delta).max(0.) / lot.quantity
                } else {
                    0.
                };
                for part in &mut lot.parts {
                    part.1 *= fraction;
                }
            } else if change.delta > 0. && op.kind == OperationKind::Production {
                if let Some(id) = subject.as_ref().filter(|id| ids.contains_key(*id)) {
                    if let Some(part) = lot.parts.iter_mut().find(|p| &p.0 == id) {
                        part.1 += change.delta;
                    } else {
                        lot.parts.push((id.clone(), change.delta));
                    }
                }
            }
            lot.quantity = (lot.quantity + change.delta).max(0.);
        }
    }
    result.contributions = crate::stock_provenance::next_stock_provenance_from_cargo(
        w,
        &basis.stocks,
        &basis.stock_contributions,
        operations,
        &basis.shipments,
    )?;
    Ok(result)
}
