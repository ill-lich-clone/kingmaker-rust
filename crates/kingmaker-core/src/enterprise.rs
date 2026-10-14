use crate::enums::Goods;
use crate::ids::{AccountId, EnterpriseId, ProvinceId};
use crate::ExternalId;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct EnterpriseRecord {
    pub id: EnterpriseId,
    pub external_id: ExternalId,
    pub owner_id: ExternalId,
    pub province_id: ProvinceId,
    pub recipe_id: Option<ExternalId>,
    pub level: u32,
    /// Present only when the source defines an unambiguous account binding.
    pub account_id: Option<AccountId>,
    pub account_ids: Vec<AccountId>,
    pub payment_purpose: Option<String>,
    pub employees: Option<f64>,
    pub is_artisan: bool,
    /// Source working_capital.minimum_minor; not a cash balance or executed protection.
    pub operating_capital: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StockpileRecord {
    /// Author extensions retained for title-preserving inventory transfers.
    #[serde(default)]
    pub extra:crate::production::Extra,
    pub inventory_origin: Option<String>,
    pub method_kind: Option<String>,
    pub id: u32,
    pub external_id: ExternalId,
    pub province_id: ProvinceId,
    pub goods: Goods,
    pub quantity: f64,
    pub purpose: String,
    pub owner_id: ExternalId,
    pub unit: Option<serde_json::Value>,
    pub facility_id: Option<ExternalId>,
    pub currency_id: Option<ExternalId>,
    pub beneficiary_pop_id: Option<ExternalId>,
}

impl Default for StockpileRecord {
    fn default() -> Self {
        Self {
            extra:Default::default(),
            inventory_origin: None,
            method_kind: None,
            id: 0,
            external_id: ExternalId::default(),
            province_id: ProvinceId(0),
            goods: Goods::Grain,
            quantity: 0.0,
            purpose: "commercial".to_string(),
            owner_id: ExternalId::default(),
            unit: None,
            facility_id: None,
            currency_id: None,
            beneficiary_pop_id: None,
        }
    }
}
