//! Current authored market terms. Quantitative resolver fields retain their JSON representation.
use crate::production::{optional_value, province_id, Extra};
use crate::{ExternalId, ProvinceId};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct MarketData {
    #[serde(default)]
    pub phase_receipts: Vec<MarketPhaseReceipt>,
    #[serde(default)]
    pub needs: Option<NeedsData>,
    #[serde(default)]
    pub price_calibration: Option<PriceCalibration>,
    #[serde(default)]
    pub parameters: Option<FoundationParameters>,
    #[serde(default)]
    pub territory: Vec<MarketTerritory>,
    #[serde(default)]
    pub maintenance_agreements: Vec<MaintenanceAgreement>,
    #[serde(default)]
    pub sale_offers: Vec<SaleOffer>,
    #[serde(default)]
    pub institutions: Vec<Value>,
    #[serde(default)]
    pub construction_catalog: Vec<Value>,
    #[serde(default)]
    pub consumer_facility_by_province: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MarketPhaseReceipt {
    pub campaign_id: String,
    pub request: MarketRequest,
    pub effective_priorities: Vec<String>,
    pub frozen_needs: Vec<ResolvedNeed>,
    pub orders: Vec<MarketOrder>,
    pub operations: Vec<crate::operations::EconomicOperation>,
    pub maintenance_receipts: Vec<crate::production::MaintenanceReceipt>,
    pub consumption: Vec<serde_json::Value>,
    pub diagnostics: Vec<Value>,
    pub registry_changes: crate::production::ProductionRegistryChanges,
    pub invariants: crate::audit::InvariantReport,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct NeedsData {
    #[serde(default, deserialize_with = "optional_value")]
    pub schema_version: Option<String>,
    #[serde(default, rename = "profileRefs")]
    pub profile_refs: Vec<NeedProfileRef>,
    #[serde(default)]
    pub profiles: Vec<NeedProfile>,
    #[serde(default)]
    pub resolver: NeedsResolver,
    #[serde(flatten)]
    pub extra: Extra,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NeedProfileRef {
    pub pop_id: ExternalId,
    pub need_profile_id: ExternalId,
    // JS finiteOrNull intentionally accepts authored decimal strings and nulls.
    #[serde(flatten)]
    pub extra: Extra,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NeedProfile {
    pub need_profile_id: ExternalId,
    #[serde(default)]
    pub norms: Vec<NeedNorm>,
    #[serde(flatten)]
    pub extra: Extra,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct NeedsResolver {
    #[serde(flatten)]
    pub extra: Extra,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NeedNorm {
    pub need_id: ExternalId,
    #[serde(flatten)]
    pub extra: Extra,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct PriceCalibration {
    #[serde(default, rename = "pricesByCurrency")]
    pub prices_by_currency: BTreeMap<String, BTreeMap<String, Value>>,
    #[serde(flatten)]
    pub extra: Extra,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct FoundationParameters {
    #[serde(default, deserialize_with = "optional_value")]
    pub instant_food: Option<bool>,
    #[serde(default, deserialize_with = "optional_value")]
    pub regional_market: Option<bool>,
    #[serde(flatten)]
    pub extra: Extra,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MarketTerritory {
    #[serde(deserialize_with = "province_id")]
    pub province_id: ProvinceId,
    #[serde(default, deserialize_with = "optional_value")]
    pub region_id: Option<ExternalId>,
    #[serde(flatten)]
    pub extra: Extra,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MaintenanceAgreement {
    pub pop_id: ExternalId,
    pub payer_id: ExternalId,
    pub currency_id: ExternalId,
    #[serde(flatten)]
    pub extra: Extra,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SaleOffer {
    #[serde(default, deserialize_with = "optional_value")]
    pub id: Option<ExternalId>,
    pub stock_id: ExternalId,
    pub seller_id: ExternalId,
    pub goods_id: ExternalId,
    #[serde(default, deserialize_with = "optional_value")]
    pub seller_purpose: Option<String>,
    #[serde(flatten)]
    pub extra: Extra,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct MarketProfile {
    #[serde(default, deserialize_with = "optional_value")]
    pub food_norm_multiplier: Option<f64>,
    #[serde(flatten)]
    pub extra: Extra,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResolvedNeed {
    pub need_id: ExternalId,
    pub quantity: Option<f64>,
    pub gross_quantity: Option<f64>,
    #[serde(flatten)]
    pub extra: Extra,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MarketOrder {
    pub id: String,
    pub owner_id: ExternalId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pop_id: Option<ExternalId>,
    #[serde(deserialize_with = "province_id")]
    pub province_id: ProvinceId,
    #[serde(
        default,
        deserialize_with = "optional_value",
        skip_serializing_if = "Option::is_none"
    )]
    pub currency_id: Option<ExternalId>,
    pub purpose: String,
    pub goods_id: ExternalId,
    pub quantity: f64,
    pub priority: String,
    pub consume: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub need_id: Option<ExternalId>,
    #[serde(default)]
    pub maintenance: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub facility_id: Option<ExternalId>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub food_alternatives: Vec<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub food_gross_quantity: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub military_food_substitution: Option<f64>,
    #[serde(flatten)]
    pub extra: Extra,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct NeedBuildResult {
    pub rows: Vec<ResolvedNeed>,
    pub orders: Vec<MarketOrder>,
    pub diagnostics: Vec<Value>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MarketStage {
    Maintenance,
    EarlyFood,
    ClosingMarket,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MarketRequest {
    pub expected_revision: u64,
    pub period: String,
    pub stage: MarketStage,
    #[serde(default)]
    pub profile: MarketProfile,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub priorities: Option<Vec<String>>,
}
