use crate::market_policy::{MarketLocation, MarketScope};
use kingmaker_core::{market::MarketOrder, ExternalId};
use std::collections::HashMap;

/// Positions retain authored order; quantities and cash are read by the caller.
pub struct OrderIndex {
    raw: HashMap<MarketLocation, HashMap<ExternalId, Vec<usize>>>,
    food: HashMap<Option<ExternalId>, HashMap<ExternalId, Vec<usize>>>,
}
impl OrderIndex {
    pub fn new(orders: &[MarketOrder], scope: &MarketScope) -> Self {
        let mut index = Self { raw: HashMap::new(), food: HashMap::new() };
        for (i, o) in orders.iter().enumerate() {
            if !(o.quantity > 0.0) { continue; }
            index.raw.entry(scope.key(o.province_id)).or_default()
                .entry(o.goods_id.clone()).or_default().push(i);
            if o.need_id.as_ref().and_then(ExternalId::as_str) == Some("food") {
                index.food.entry(o.currency_id.clone()).or_default()
                    .entry(o.goods_id.clone()).or_default().push(i);
            }
        }
        index
    }
    pub fn raw_orders(&self, location: &MarketLocation, goods: &ExternalId) -> &[usize] {
        self.raw.get(location).and_then(|r| r.get(goods)).map_or(&[], Vec::as_slice)
    }
    pub fn food_orders(&self, currency: &Option<ExternalId>, goods: &ExternalId) -> &[usize] {
        self.food.get(currency).and_then(|r| r.get(goods)).map_or(&[], Vec::as_slice)
    }
}
