use kingmaker_core::{
    AccountBalance, ExternalId, Goods, Money, ProvinceId, ProvinceRecord, StockpileRecord,
};
use std::collections::HashMap;

#[derive(Debug, Default)]
pub struct ProcurementSummary {
    pub procured_lots: u32,
    pub total_cost_minor: i64,
}

pub fn execute_state_raw_procurement(
    provinces: &[ProvinceRecord],
    stockpiles: &mut [StockpileRecord],
    accounts: &mut [AccountBalance],
) -> ProcurementSummary {
    let mut state_by_province: HashMap<ProvinceId, ExternalId> =
        HashMap::with_capacity(provinces.len());
    for p in provinces {
        if let Some(state) = &p.owner_state_id {
            state_by_province.insert(p.id, state.clone());
        }
    }

    let mut treasury_indices: HashMap<ExternalId, usize> = HashMap::new();
    for (idx, acc) in accounts.iter().enumerate() {
        if let Some(state) = acc
            .owner_id
            .as_str()
            .and_then(|s| s.strip_prefix("treasury:"))
        {
            if acc.purpose == "operating_capital" {
                treasury_indices.insert(ExternalId::from(state), idx);
            }
        }
    }

    let mut procured_lots = 0;
    let mut total_cost = 0;

    for stock in stockpiles.iter_mut() {
        if stock.purpose != "commercial" || stock.quantity <= 0.0 {
            continue;
        }

        let is_raw = matches!(
            stock.goods,
            Goods::Iron | Goods::Coal | Goods::Timber | Goods::Grain
        );
        if !is_raw {
            continue;
        }

        if let Some(state_id) = state_by_province.get(&stock.province_id) {
            if let Some(&t_idx) = treasury_indices.get(state_id) {
                let treasury_cash = accounts[t_idx].balance.units();
                let price_per_unit = match stock.goods {
                    Goods::Iron => 50,
                    Goods::Coal => 30,
                    Goods::Timber => 20,
                    Goods::Grain => 10,
                    _ => 10,
                };
                let lot_cost = (stock.quantity * (price_per_unit as f64)) as i64;
                if treasury_cash >= lot_cost && lot_cost > 0 {
                    accounts[t_idx].balance = Money::from_units(treasury_cash - lot_cost);
                    stock.purpose = "state_raw".to_string();
                    procured_lots += 1;
                    total_cost += lot_cost;
                }
            }
        }
    }

    ProcurementSummary {
        procured_lots,
        total_cost_minor: total_cost,
    }
}
