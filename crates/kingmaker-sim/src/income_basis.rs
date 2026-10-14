//! Frozen native opening metadata, never an unverified caller-provided sales ledger.
use kingmaker_core::world_read::WorldRead;
use crate::{budget_funding::error, operation_session::OpResult};
use kingmaker_core::{income::IncomeRequest, CampaignHeader, StockpileRecord};
use serde::Serialize;

/// Public callers may capture/inspect a basis, but cannot construct or deserialize one.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct IncomeOpeningBasis {
    pub(crate) campaign: CampaignHeader,
    pub(crate) period: String,
    pub(crate) stocks: Vec<StockpileRecord>,
    pub(crate) stock_contributions: Vec<kingmaker_core::income::StockContribution>,
    pub(crate) shipments: Vec<serde_json::Value>,
}

pub fn capture_income_opening(w: &dyn WorldRead, r: &IncomeRequest) -> OpResult<IncomeOpeningBasis> {
    if r.period.is_empty() || r.expected_revision != w.campaign().revision {
        return Err(error("Invalid income opening revision/period"));
    }
    if w.budget_phase_receipts().values()
        .any(|p| p.request.period == r.period)
        || w.budget_opening_receipts().values().any(|p|p.request.period==r.period)
        || w.production_phase_receipts().values()
            .any(|p| p.request.period == r.period)
        || w.transport_phase_receipts().values()
            .any(|p| p.request.period == r.period)
        || w.market_phase_receipts().values()
            .any(|p| p.request.period == r.period)
    {
        return Err(error("Income opening basis must precede economic phases"));
    }
    Ok(IncomeOpeningBasis {
        campaign: w.campaign().clone(),
        period: r.period.clone(),
        stocks: w.stockpiles().iter().map(|s|StockpileRecord {
            id:s.id,external_id:s.external_id.clone(),province_id:s.province_id,goods:s.goods.clone(),quantity:s.quantity,
            purpose:s.purpose.clone(),owner_id:s.owner_id.clone(),unit:s.unit.clone(),facility_id:s.facility_id.clone(),currency_id:s.currency_id.clone(),beneficiary_pop_id:s.beneficiary_pop_id.clone(),inventory_origin:s.inventory_origin.clone(),method_kind:s.method_kind.clone(),extra:Default::default(),
        }).collect(),
        stock_contributions: crate::stock_provenance::capture_stock_provenance(w)?,
        shipments: w.shipments().iter().map(|s| {
            let mut row=serde_json::Map::new();
            for key in ["id","goods_id","remaining_quantity","producer_contributions","goods_origin","stock_id"] {
                if let Some(value)=s.get(key) {row.insert(key.into(),value.clone());}
            }
            serde_json::Value::Object(row)
        }).collect(),
    })
}

pub fn validate_income_basis(
    w: &dyn WorldRead,
    r: &IncomeRequest,
    basis: &IncomeOpeningBasis,
) -> OpResult<()> {
    if r.period.is_empty()
        || r.expected_revision != w.campaign().revision
        || &basis.campaign != w.campaign()
        || basis.period != r.period
    {
        return Err(error(
            "Income opening basis campaign/revision/date/period mismatch",
        ));
    }
    Ok(())
}

