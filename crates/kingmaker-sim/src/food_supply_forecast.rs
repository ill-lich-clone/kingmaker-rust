//! Readonly forecast of actual owned food and dated cargo, using the authored weekly need.
use kingmaker_core::world_read::WorldRead;
use crate::{
    budget_funding::error,
    calendar::{add_days, serial_date},
    operation_session::OpResult,
    transport_validation::{actor, province},
};
use kingmaker_core::{calendar::GameDate, market::MarketOrder, *};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
type Base = (ExternalId, ProvinceId, ExternalId);
type Key = (ExternalId, ProvinceId, ExternalId, ExternalId);
pub(crate) fn food(o: &MarketOrder) -> bool {
    o.need_id.as_ref().and_then(ExternalId::as_str) == Some("food")
}
pub(crate) fn food_key(o: &MarketOrder) -> OpResult<Key> {
    let beneficiary = o
        .extra
        .get("beneficiary_pop_id")
        .filter(|v| !v.is_null())
        .map(actor)
        .transpose()?
        .or_else(|| o.pop_id.clone())
        .unwrap_or_else(|| o.owner_id.clone());
    Ok((
        o.owner_id.clone(),
        o.province_id,
        o.goods_id.clone(),
        beneficiary,
    ))
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FoodForecastResult {
    pub order_id: String,
    pub target_quantity: f64,
    pub on_hand: f64,
    pub incoming_quantity: f64,
    pub quantity: f64,
    pub uncovered_before_arrival: f64,
    pub arrival_date: GameDate,
    pub horizon_days: i64,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FoodForecastDiagnostic {
    #[serde(flatten)]
    pub forecast: FoodForecastResult,
    pub ordered_quantity: f64,
}
#[derive(Clone)]
struct Event {
    at: i64,
    quantity: f64,
    delayed: bool,
}
pub struct FoodSupplyForecast {
    date: GameDate,
    today: i64,
    weekly: HashMap<Key, f64>,
    owned: HashMap<Key, f64>,
    incoming: HashMap<Key, Vec<Event>>,
    diagnostics: Vec<FoodForecastDiagnostic>,
    by_order: HashMap<String, usize>,
}
fn distribute(
    weekly: &HashMap<Key, f64>,
    groups: &HashMap<Base, Vec<Key>>,
    base: Base,
    beneficiary: Option<ExternalId>,
    quantity: f64,
) -> Vec<(Key, f64)> {
    let specific = (
        base.0.clone(),
        base.1,
        base.2.clone(),
        beneficiary.clone().unwrap_or_else(|| base.0.clone()),
    );
    if beneficiary.is_some() || weekly.contains_key(&specific) {
        return if weekly.contains_key(&specific) {
            vec![(specific, quantity)]
        } else {
            vec![]
        };
    }
    let Some(keys) = groups.get(&base) else {
        return vec![];
    };
    let total = keys.iter().map(|k| weekly[k]).sum::<f64>();
    if total <= 0. {
        return vec![];
    }
    keys.iter()
        .map(|k| (k.clone(), quantity * weekly[k] / total))
        .collect()
}
impl FoodSupplyForecast {
    pub fn new(w: &dyn WorldRead, orders: &[MarketOrder]) -> OpResult<Self> {
        let date = GameDate::from(w.campaign());
        let today = serial_date(date)?;
        let mut weekly = HashMap::<Key, f64>::new();
        let mut groups = HashMap::<Base, Vec<Key>>::new();
        let mut seen = HashSet::new();
        for o in orders.iter().filter(|o| food(o)) {
            if !seen.insert(&o.id) {
                continue;
            }
            if !o.quantity.is_finite() || o.quantity < 0. {
                return Err(error("Invalid weekly food quantity"));
            }
            let key = food_key(o)?;
            *weekly.entry(key.clone()).or_default() += o.quantity;
            let keys = groups
                .entry((key.0.clone(), key.1, key.2.clone()))
                .or_default();
            if !keys.contains(&key) {
                keys.push(key);
            }
        }
        let mut held = HashMap::<ExternalId, f64>::new();
        for r in w.goods_reservations().values() {
            if r.status == "held" {
                *held.entry(r.stock_id.clone()).or_default() += r.quantity;
            }
        }
        for (id, q) in crate::market_policy::protected_artisan_inputs(w)? {
            *held.entry(id).or_default() += q;
        }
        let mut owned = HashMap::<Key, f64>::new();
        let mut incoming = HashMap::<Key, Vec<Event>>::new();
        for s in w.stockpiles().values() {
            let q = (s.quantity - held.get(&s.external_id).copied().unwrap_or(0.)).max(0.);
            if !q.is_finite() {
                return Err(error("Invalid owned food quantity"));
            }
            for (k, q) in distribute(
                &weekly,
                &groups,
                (s.owner_id.clone(), s.province_id, s.goods.external_id()),
                s.beneficiary_pop_id.clone(),
                q,
            ) {
                *owned.entry(k).or_default() += q;
            }
        }
        for s in w.shipments().values() {
            let Some(q) = s["remaining_quantity"].as_f64().filter(|q| *q > 0.) else {
                continue;
            };
            if !matches!(
                s["status"].as_str(),
                Some("in_transit" | "partial_arrived" | "awaiting_storage")
            ) || s["arrival_date"].is_null()
            {
                continue;
            }
            if !q.is_finite() {
                return Err(error("Invalid incoming food quantity"));
            }
            let due = serial_date(serde_json::from_value(s["arrival_date"].clone())?)?;
            let delayed = s["status"] != "in_transit" || due <= today;
            let at = if delayed {
                today
                    .checked_add(7)
                    .ok_or_else(|| error("Food date overflow"))?
            } else {
                due
            };
            let beneficiary = s
                .get("beneficiary_pop_id")
                .filter(|v| !v.is_null())
                .map(actor)
                .transpose()?;
            for (k, quantity) in distribute(
                &weekly,
                &groups,
                (
                    actor(&s["buyer_id"])?,
                    province(&s["to_province_id"])?,
                    actor(&s["goods_id"])?,
                ),
                beneficiary,
                q,
            ) {
                incoming.entry(k).or_default().push(Event {
                    at,
                    quantity,
                    delayed,
                });
            }
        }
        Ok(Self {
            date,
            today,
            weekly,
            owned,
            incoming,
            diagnostics: vec![],
            by_order: HashMap::new(),
        })
    }
    pub fn forecast_for(
        &mut self,
        o: &MarketOrder,
        travel_days: i64,
    ) -> OpResult<FoodForecastResult> {
        if travel_days < 0 {
            return Err(error("Invalid food route duration"));
        }
        let key = food_key(o)?;
        let rate = self.weekly.get(&key).copied().unwrap_or(0.) / 7.;
        let horizon = travel_days
            .checked_add(7)
            .ok_or_else(|| error("Food horizon overflow"))?;
        let end = self
            .today
            .checked_add(horizon)
            .ok_or_else(|| error("Food date overflow"))?;
        let arrival = self
            .today
            .checked_add(travel_days)
            .ok_or_else(|| error("Food date overflow"))?;
        let mut events = self
            .incoming
            .get(&key)
            .into_iter()
            .flatten()
            .filter(|e| e.at <= end)
            .cloned()
            .collect::<Vec<_>>();
        events.sort_by_key(|e| e.at);
        let incoming_quantity = events.iter().map(|e| e.quantity).sum::<f64>();
        let on_hand = self.owned.get(&key).copied().unwrap_or(0.);
        let mut stock = on_hand;
        let mut at = self.today;
        let mut shortage = 0.;
        for e in events.iter().filter(|e| !e.delayed && e.at <= arrival) {
            let demand = rate * (e.at - at).max(0) as f64;
            shortage += (demand - stock).max(0.);
            stock = (stock - demand).max(0.) + e.quantity;
            at = at.max(e.at);
        }
        shortage += (rate * (arrival - at).max(0) as f64 - stock).max(0.);
        let target = rate * horizon as f64;
        if [target, on_hand, incoming_quantity, shortage]
            .iter()
            .any(|x| !x.is_finite())
        {
            return Err(error("Nonfinite food forecast"));
        }
        let result = FoodForecastResult {
            order_id: o.id.clone(),
            target_quantity: target,
            on_hand,
            incoming_quantity,
            quantity: (target - on_hand - incoming_quantity).max(0.),
            uncovered_before_arrival: shortage,
            arrival_date: add_days(self.date, travel_days)?,
            horizon_days: horizon,
        };
        if let Some(&i) = self.by_order.get(&o.id) {
            self.diagnostics[i].forecast = result.clone();
        } else {
            self.by_order.insert(o.id.clone(), self.diagnostics.len());
            self.diagnostics.push(FoodForecastDiagnostic {
                forecast: result.clone(),
                ordered_quantity: 0.,
            });
        }
        Ok(result)
    }
    pub fn register_dispatch(
        &mut self,
        o: &MarketOrder,
        quantity: f64,
        date: GameDate,
    ) -> OpResult<()> {
        if !quantity.is_finite() {
            return Err(error("Invalid dispatched food quantity"));
        }
        if quantity <= 0. {
            return Ok(());
        }
        self.incoming.entry(food_key(o)?).or_default().push(Event {
            at: serial_date(date)?,
            quantity,
            delayed: false,
        });
        if let Some(&i) = self.by_order.get(&o.id) {
            self.diagnostics[i].ordered_quantity += quantity;
        }
        Ok(())
    }
    pub fn diagnostics(&self) -> &[FoodForecastDiagnostic] {
        &self.diagnostics
    }
}
