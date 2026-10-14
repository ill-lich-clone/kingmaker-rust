//! Three isolated, named, atomic source-market steps. They do not advance a week.
use kingmaker_core::world_read::WorldRead;
use crate::{
    food_conversion::*, market_clearing::*, market_inputs::build_foundation_input_orders,
    needs::build_need_orders, operation_session::*,
};
use kingmaker_core::{market::*, operations::*, *};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
fn error(s: &str) -> OperationError {
    OperationError(s.into())
}
pub struct PreparedMarketPhase {
    changes: Option<PreparedChanges>,
    receipt: MarketPhaseReceipt,
}
fn matching<'a>(w: &'a dyn WorldRead, r: &MarketRequest) -> Option<&'a MarketPhaseReceipt> {
    w.market_phase_receipts().values().find(|old| {
        old.campaign_id == w.campaign().campaign_id
            && old.request.period == r.period
            && old.request.stage == r.stage
    })
}
fn priorities(w: &dyn WorldRead, r: &MarketRequest) -> OpResult<Vec<String>> {
    if r.stage != MarketStage::ClosingMarket {
        return Ok(vec!["basic".into()]);
    }
    if let Some(p) = &r.priorities {
        return Ok(p.clone());
    }
    let p = w.production_phase_receipts().values()
        .find(|p| {
            p.campaign_id == w.campaign().campaign_id
                && p.request.period == r.period
                && p.request.expected_revision == r.expected_revision
        })
        .ok_or_else(|| error("Closing market requires same-period native production"))?;
    Ok(p.purchase_priorities.clone())
}
fn order_digest(
    request: &MarketRequest,
    orders: &[MarketOrder],
    needs: &[ResolvedNeed],
    priorities: &[String],
) -> OpResult<String> {
    Ok(Sha256::digest(serde_json::to_vec(
        &json!({"request":request,"orders":orders,"frozen_needs":needs,"priorities":priorities}),
    )?)
    .iter()
    .map(|b| format!("{b:02x}"))
    .collect())
}
fn phase_boundary(w: &dyn WorldRead, r: &MarketRequest) -> String {
    format!(
        "market-phase:{}:{}:{}",
        w.campaign().campaign_id,
        r.period,
        crate::needs::js_string(&json!(r.stage))
    )
}
pub(crate) fn verify_history(w: &dyn WorldRead, r: &MarketPhaseReceipt) -> OpResult<()> {
    let rows = w.committed_operations().values()
        .map(|o| (&o.event_id, &o.value))
        .collect::<HashMap<_, _>>();
    let mut ids = HashSet::new();
    if r.operations.iter().any(|o| {
        !ids.insert(&o.event_id)
            || canonical_operation(o).is_err()
            || canonical_operation(o).ok().as_ref() != rows.get(&o.event_id).copied()
    }) {
        return Err(error("Market phase receipt history missing/conflicting"));
    }
    let boundary = r
        .operations
        .last()
        .ok_or_else(|| error("Market boundary lost"))?;
    if boundary.kind != OperationKind::MarketPhaseBoundary
        || boundary.event_id.json() != json!(phase_boundary(w, &r.request))
        || boundary.meta.get("orders_sha256")
            != Some(&json!(order_digest(
                &r.request,
                &r.orders,
                &r.frozen_needs,
                &r.effective_priorities
            )?))
        || rows.len() != w.committed_operations().len()
    {
        return Err(error("Market orders/needs or required boundary altered"));
    }
    Ok(())
}
fn belongs(w: &dyn WorldRead, request: &MarketRequest, op: &Value) -> bool {
    if op["type"] == json!("market_phase_boundary") {
        return op["event_id"] == json!(phase_boundary(w, request));
    }
    let event = op.get("event_id").and_then(Value::as_str).unwrap_or("");
    let (period, food_stage) = match request.stage {
        MarketStage::Maintenance => (format!("{}:maintenance", request.period), "early"),
        MarketStage::EarlyFood => (request.period.clone(), "early"),
        MarketStage::ClosingMarket => (request.period.clone(), "late"),
    };
    if op.get("type").and_then(Value::as_str) == Some("food_conversion") {
        let stage = op.get("stage").and_then(Value::as_str);
        return (stage == Some(food_stage)
            || request.stage == MarketStage::ClosingMarket && stage == Some("surplus"))
            && event.starts_with(&format!(
                "food-conversion:[{},{}",
                json!(w.campaign().campaign_id),
                json!(period)
            ));
    }
    let prefix = format!("market:{}:{period}:", w.campaign().campaign_id);
    match request.stage {
        MarketStage::Maintenance => {
            event.starts_with(&prefix)
                || event.starts_with(&format!("market-blocked:{period}:"))
                || event.starts_with(&format!("storage-rent:market-storage:{period}:"))
        }
        MarketStage::EarlyFood => event.starts_with(&format!("{prefix}food:early:")),
        MarketStage::ClosingMarket => {
            event.starts_with(&format!("{prefix}phase"))
                || event.starts_with(&format!("{prefix}food:late:"))
                || event.starts_with(&format!("market-blocked:{period}:"))
                || event.starts_with(&format!("storage-rent:market-storage:{period}:"))
        }
    }
}
pub fn prepare_market_phase(w: &dyn WorldRead, r: &MarketRequest) -> OpResult<PreparedMarketPhase> {
    if r.expected_revision != w.campaign().revision
        || r.period.is_empty()
        || r.profile
            .food_norm_multiplier
            .is_some_and(|m| !m.is_finite() || m < 0.0)
    {
        return Err(error("Invalid market request revision/period/profile"));
    }
    let effective = priorities(w, r)?;
    if let Some(old) = matching(w, r) {
        if old.request != *r || old.effective_priorities != effective {
            return Err(error("Conflicting market phase replay"));
        }
        verify_history(w, old)?;
        return Ok(PreparedMarketPhase {
            changes: None,
            receipt: old.clone(),
        });
    }
    let has_production = w.production_phase_receipts().values()
        .any(|p| p.campaign_id == w.campaign().campaign_id && p.request.period == r.period);
    let later = w.market_phase_receipts().values().any(|p| {
        p.campaign_id == w.campaign().campaign_id
            && p.request.period == r.period
            && match r.stage {
                MarketStage::Maintenance => p.request.stage != MarketStage::Maintenance,
                MarketStage::EarlyFood => p.request.stage == MarketStage::ClosingMarket,
                MarketStage::ClosingMarket => false,
            }
    });
    if r.stage != MarketStage::ClosingMarket && (has_production || later) {
        return Err(error(
            "Opening market stage cannot run after a later same-period stage",
        ));
    }
    if w.committed_operations().values()
        .any(|o| belongs(w, r, &o.value))
    {
        return Err(error(
            "Market operations exist without complete named receipt",
        ));
    }
    let built = build_need_orders(w, &r.profile)?;
    let slave_ids = w.pops().values()
        .filter(|p| p.pop_type_id.as_str() == Some("slaves"))
        .map(|p| p.external_id.clone())
        .collect::<HashSet<_>>();
    let mut orders = built
        .orders
        .iter()
        .filter(|o| {
            if r.stage == MarketStage::Maintenance {
                o.maintenance
            } else {
                !o.maintenance
            }
        })
        .cloned()
        .collect::<Vec<_>>();
    let frozen_needs = built
        .rows
        .into_iter()
        .filter(|row| {
            row.extra
                .get("pop_id")
                .and_then(|v| ExternalId::from_json(v.clone()).ok())
                .is_some_and(|id| slave_ids.contains(&id) == (r.stage == MarketStage::Maintenance))
        })
        .collect::<Vec<_>>();
    let instant = w.market_parameters()
        .as_ref()
        .and_then(|p| p.instant_food)
        == Some(true);
    let mut s = OperationSession::new(w)?;
    s.configure_market(r)?;
    let mut diagnostics = built.diagnostics;
    match r.stage {
        MarketStage::Maintenance => {
            let period = format!("{}:maintenance", r.period);
            if instant {
                orders = food_orders(&orders)?;
                let food = plan_food_conversion(&mut s, &period, &orders, FoodStage::Early)?;
                diagnostics.extend(food.blocked);
            } else {
                plan_foundation_market(&mut s, &period, &orders, &["basic".into()])?;
            }
        }
        MarketStage::EarlyFood => {
            if !instant {
                return Err(error("EarlyFood requires enabled instant_food rule"));
            }
            let food = plan_food_conversion(&mut s, &r.period, &orders, FoodStage::Early)?;
            diagnostics.extend(food.blocked);
        }
        MarketStage::ClosingMarket => {
            let production = w.production_phase_receipts().values()
                .find(|p| {
                    p.campaign_id == w.campaign().campaign_id
                        && p.request.period == r.period
                        && p.request.expected_revision == r.expected_revision
                })
                .ok_or_else(|| error("Closing market requires same-period native production"))?;
            if !production.invariants.production_required_passed() {
                return Err(error("Native production receipt audit failed"));
            }
            if instant {
                orders = food_orders(&orders)?;
            }
            orders.extend(build_foundation_input_orders(w, production)?);
            let mut pending = orders.clone();
            if instant {
                let early = matching(
                    w,
                    &MarketRequest {
                        stage: MarketStage::EarlyFood,
                        ..r.clone()
                    },
                );
                if let Some(early) = early {
                    verify_history(w, early)?;
                    pending = remaining_food_orders(&pending, &early.operations)?;
                }
                let late = plan_food_conversion(&mut s, &r.period, &pending, FoodStage::Late)?;
                diagnostics.extend(late.blocked);
                pending = remaining_food_orders(&pending, &late.operations)?;
            }
            let market = plan_foundation_market(&mut s, &r.period, &pending, &effective)?;
            if instant {
                let mut fulfilled = HashMap::new();
                for o in &market.operations {
                    if o.kind == OperationKind::Purchase {
                        if let (Some(id), Some(q)) = (
                            o.meta.get("order_id").and_then(Value::as_str),
                            o.meta.get("quantity").and_then(Value::as_f64),
                        ) {
                            *fulfilled.entry(id.to_string()).or_insert(0.0) += q;
                        }
                    }
                }
                let residual = pending
                    .iter()
                    .map(|o| {
                        let mut o = o.clone();
                        o.quantity =
                            (o.quantity - fulfilled.get(&o.id).copied().unwrap_or(0.0)).max(0.0);
                        o
                    })
                    .filter(|o| o.quantity > 1e-10)
                    .collect::<Vec<_>>();
                let surplus =
                    plan_food_conversion(&mut s, &r.period, &residual, FoodStage::Surplus)?;
                diagnostics.extend(surplus.blocked);
            }
        }
    }
    let boundary: EconomicOperation = serde_json::from_value(
        json!({"event_id":phase_boundary(w,r),"type":"market_phase_boundary","period":r.period,"stage":r.stage,"orders_sha256":order_digest(r,&orders,&frozen_needs,&effective)?}),
    )?;
    s.stage_market_boundary(&boundary)?;
    let operations = s.operations().to_vec();
    let mut receipt = MarketPhaseReceipt {
        campaign_id: w.campaign().campaign_id.clone(),
        request: r.clone(),
        effective_priorities: effective,
        frozen_needs,
        orders,
        operations,
        maintenance_receipts: vec![],
        consumption: vec![],
        diagnostics,
        registry_changes: s.registry_changes(),
        invariants: Default::default(),
    };
    summarize(w, &mut receipt)?;
    receipt.invariants = crate::market_invariants::check_session_market_operations(&s, &receipt);
    if !receipt.invariants.production_required_passed() {
        return Err(OperationError(format!(
            "Market audit rejected: {:?}",
            receipt.invariants
        )));
    }
    Ok(PreparedMarketPhase {
        changes: Some(s.finish()?),
        receipt,
    })
}
fn summarize(w: &dyn WorldRead, r: &mut MarketPhaseReceipt) -> OpResult<()> {
    let mut demand: HashMap<(ExternalId, ExternalId), f64> = HashMap::new();
    for o in r.orders.iter().filter(|o| o.consume) {
        if let (Some(pop), Some(need)) = (&o.pop_id, &o.need_id) {
            *demand.entry((pop.clone(), need.clone())).or_default() += o.quantity;
        }
    }
    let mut actual: HashMap<(ExternalId, ExternalId), f64> = HashMap::new();
    let mut events: HashMap<ExternalId, Vec<ExternalId>> = HashMap::new();
    let early = if r.request.stage == MarketStage::ClosingMarket {
        matching(
            w,
            &MarketRequest {
                stage: MarketStage::EarlyFood,
                ..r.request.clone()
            },
        )
    } else {
        None
    };
    let all_consumption = r
        .operations
        .iter()
        .chain(early.into_iter().flat_map(|r| r.operations.iter()))
        .collect::<Vec<_>>();
    for o in all_consumption.iter().copied().filter(|o| {
        o.kind == OperationKind::Purchase && o.meta.get("consumed") == Some(&json!(true))
    }) {
        if let (Some(pop), Some(need), Some(q)) = (
            o.meta.get("pop_id"),
            o.meta.get("need_id"),
            o.meta.get("quantity").and_then(Value::as_f64),
        ) {
            let pop = ExternalId::from_json(pop.clone()).map_err(OperationError)?;
            let need = ExternalId::from_json(need.clone()).map_err(OperationError)?;
            *actual.entry((pop.clone(), need)).or_default() += q;
            if o.meta.get("maintenance") == Some(&json!(true)) {
                events.entry(pop).or_default().push(o.event_id.clone());
            }
        }
    }
    let supply_by_pop = r
        .orders
        .iter()
        .filter(|o| o.goods_id.as_str() == Some("military_supply"))
        .map(|o| (o.pop_id.clone(), o))
        .collect::<HashMap<_, _>>();
    let mut substitution_by_pop = HashMap::new();
    for o in &r.orders {
        if o.need_id.as_ref().and_then(ExternalId::as_str) == Some("food") {
            substitution_by_pop
                .entry(o.pop_id.clone())
                .or_insert(o.military_food_substitution.unwrap_or(0.0));
        }
    }
    let mut actual_by_order = HashMap::new();
    for o in &all_consumption {
        if let (Some(id), Some(q)) = (
            o.meta.get("order_id").and_then(Value::as_str),
            o.meta.get("quantity").and_then(Value::as_f64),
        ) {
            *actual_by_order.entry(id).or_insert(0.0) += q;
        }
    }
    for row in &r.frozen_needs {
        let pop = row
            .extra
            .get("pop_id")
            .ok_or_else(|| error("Frozen demand POP missing"))?;
        let pop_id = ExternalId::from_json(pop.clone()).map_err(OperationError)?;
        let key = (pop_id, row.need_id.clone());
        let q = demand.get(&key).copied().or_else(|| {
            row.quantity
                .filter(|q| *q == 0.0 && row.extra.get("consumable") != Some(&json!(false)))
        });
        let eaten = q.map(|_| actual.get(&key).copied().unwrap_or(0.0));
        let gross = if row.need_id.as_str() == Some("food") {
            row.gross_quantity
                .map(|q| q * r.request.profile.food_norm_multiplier.unwrap_or(1.0))
        } else {
            None
        };
        let supply = supply_by_pop.get(&Some(key.0.clone())).copied();
        let covered = if let Some(supply) = supply.filter(|o| o.quantity > 0.0) {
            let actual = actual_by_order
                .get(supply.id.as_str())
                .copied()
                .unwrap_or(0.0);
            substitution_by_pop
                .get(&supply.pop_id)
                .copied()
                .unwrap_or(0.0)
                * (actual / supply.quantity).min(1.0)
        } else {
            0.0
        };
        let fraction = q.zip(eaten).map(|(q, a)| {
            let required = gross.unwrap_or(q);
            let actual = a + if gross.is_some() { covered } else { 0.0 };
            if required > 0.0 {
                (actual / required).min(1.0)
            } else {
                1.0
            }
        });
        r.consumption.push(json!({"pop_id":pop,"need_id":row.need_id,"requested_quantity":q,"gross_requested_quantity":gross,"actual_quantity":eaten,"military_food_coverage":if gross.is_some(){Some(covered)}else{None},"fraction":fraction,"scope":if r.request.stage==MarketStage::ClosingMarket{"period_to_closing"}else{"current_phase"},"status":if q.is_some(){"measured"}else{"not_measured"}}));
    }
    if r.request.stage == MarketStage::Maintenance {
        let mut by_pop: HashMap<ExternalId, Vec<Value>> = HashMap::new();
        for ((pop, need), q) in &demand {
            let a = actual
                .get(&(pop.clone(), need.clone()))
                .copied()
                .unwrap_or(0.0);
            by_pop.entry(pop.clone()).or_default().push(json!({"need_id":need,"requested_quantity":q,"actual_quantity":a,"fraction":if *q>0.0{(a/q).min(1.0)}else{1.0}}));
        }
        for p in w.pops().values()
            .filter(|p| p.pop_type_id.as_str() == Some("slaves"))
        {
            let mut rows = by_pop.remove(&p.external_id).unwrap_or_default();
            rows.sort_by_key(|r| r["need_id"].to_string());
            let fraction = if rows.is_empty() {
                0.0
            } else {
                rows.iter()
                    .map(|r| r["fraction"].as_f64().unwrap())
                    .fold(1.0, f64::min)
            };
            r.maintenance_receipts.push(serde_json::from_value(json!({"pop_id":p.external_id,"period":r.request.period,"fraction":fraction,"event_ids":events.remove(&p.external_id).unwrap_or_default(),"demands":rows,"source":"native_complete_market"}))?);
        }
    }
    Ok(())
}
pub fn apply_market_phase(world:&mut WorldState,prepared:PreparedMarketPhase)->OpResult<MarketPhaseReceipt> {
    crate::world_transaction::execute_atomic(world,|world|apply_market_phase_in(world,prepared))
}
pub(crate) fn apply_market_phase_in(
    w: &mut crate::world_transaction::WorldTransaction<'_>,
    prepared: PreparedMarketPhase,
) -> OpResult<MarketPhaseReceipt> {
    let r = prepared.receipt;
    if let Some(old) = matching(w, &r.request) {
        if old != &r {
            return Err(error(
                "Prepared market receipt conflicts with intervening phase",
            ));
        }
        verify_history(w, old)?;
        return Ok(old.clone());
    }
    let Some(changes) = prepared.changes else {
        return Err(error("Replay receipt disappeared before apply"));
    };
    let event_ids = r
        .operations
        .iter()
        .map(|o| &o.event_id)
        .collect::<HashSet<_>>();
    if w.committed_operations()
        .iter()
        .any(|o| event_ids.contains(&o.event_id))
    {
        return Err(error(
            "Prepared market history applied without named receipt",
        ));
    }
    w.apply_phase(changes)?;
    let key=w.market_phase_receipts().next_key().map_err(OperationError)?;
    w.put_market_phase_receipts(key,r.clone())?;
    Ok(r)
}
pub fn execute_market_phase(
    world: &mut WorldState,
    request: &MarketRequest,
) -> OpResult<MarketPhaseReceipt> {
    crate::world_transaction::execute_atomic(world,|world|execute_market_phase_in(world,request))
}

pub(crate) fn execute_market_phase_in(world:&mut crate::world_transaction::WorldTransaction<'_>,request:&MarketRequest)->OpResult<MarketPhaseReceipt> {
    let prepared=prepare_market_phase(world,request)?;
    apply_market_phase_in(world,prepared)
}
