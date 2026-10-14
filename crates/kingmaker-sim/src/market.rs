use kingmaker_core::{Goods, ProvinceId};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum OrderPriority {
    Basic = 0,
    Everyday = 1,
    Luxury = 2,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MarketOrder {
    pub order_id: u64,
    pub buyer_province: ProvinceId,
    pub goods: Goods,
    pub requested_qty: f64,
    pub priority: OrderPriority,
    pub allocated_qty: f64,
}

/// Detached orders cannot establish paid supply, ownership or an authoritative demand.
pub fn clear_market_orders(
    _orders: &mut [MarketOrder],
    _available_supply: f64,
) -> crate::operation_session::OpResult<f64> {
    Err(crate::operation_session::OperationError("Use execute_market_phase with authoritative WorldState/request; detached orders cannot execute an economy".into()))
}
