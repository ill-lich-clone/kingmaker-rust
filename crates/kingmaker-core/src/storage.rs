use crate::production::{optional_value, province_id, Extra};
use crate::{ExternalId, Money, ProvinceId};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StorageGoods {
    pub unit: Value,
    pub space_per_unit: f64,
    #[serde(flatten)]
    pub extra: Extra,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LeaseTerms {
    #[serde(default, deserialize_with = "optional_value")]
    pub owner_id: Option<ExternalId>,
    #[serde(default, deserialize_with = "optional_value")]
    pub status: Option<String>,
    #[serde(default, deserialize_with = "optional_value")]
    pub source: Option<Value>,
    #[serde(default, deserialize_with = "optional_value")]
    pub approval_ref: Option<Value>,
    #[serde(default, deserialize_with = "optional_value")]
    pub price_minor_per_space: Option<Money>,
    #[serde(default, deserialize_with = "optional_value")]
    pub currency_id: Option<ExternalId>,
    #[serde(default, deserialize_with = "optional_value")]
    pub purpose: Option<String>,
    #[serde(default, deserialize_with = "optional_value")]
    pub cancellation_policy: Option<String>,
    #[serde(flatten)]
    pub extra: Extra,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StorageFacility {
    pub id: ExternalId,
    pub owner_id: ExternalId,
    #[serde(deserialize_with = "province_id")]
    pub province_id: ProvinceId,
    #[serde(default, deserialize_with = "optional_value")]
    pub capacity: Option<f64>,
    #[serde(default, deserialize_with = "optional_value")]
    pub unit: Option<Value>,
    #[serde(default, deserialize_with = "optional_value")]
    pub status: Option<String>,
    #[serde(default, deserialize_with = "optional_value")]
    pub source: Option<Value>,
    #[serde(default, deserialize_with = "optional_value")]
    pub approval_ref: Option<Value>,
    #[serde(default)]
    pub goods: BTreeMap<String, StorageGoods>,
    #[serde(default)]
    pub access_contracts: Vec<LeaseTerms>,
    #[serde(flatten)]
    pub extra: Extra,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StorageReservation {
    pub id: ExternalId,
    pub owner_id: ExternalId,
    pub facility_id: ExternalId,
    pub goods_id: ExternalId,
    pub unit: Value,
    pub status: String,
    pub quantity: f64,
    #[serde(default, deserialize_with = "optional_value")]
    pub accepted_quantity: Option<f64>,
    #[serde(default, deserialize_with = "optional_value")]
    pub consumed_quantity: Option<f64>,
    #[serde(default, deserialize_with = "optional_value")]
    pub released_quantity: Option<f64>,
    #[serde(default, deserialize_with = "optional_value")]
    pub fee_paid_minor: Option<Money>,
    #[serde(default, deserialize_with = "optional_value")]
    pub lease_terms: Option<LeaseTerms>,
    #[serde(default, deserialize_with = "optional_value")]
    pub period: Option<String>,
    #[serde(default, deserialize_with = "optional_value")]
    pub release_event_id: Option<ExternalId>,
    #[serde(flatten)]
    pub extra: Extra,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GoodsReservation {
    #[serde(default, deserialize_with = "optional_value")]
    pub id: Option<ExternalId>,
    pub stock_id: ExternalId,
    pub quantity: f64,
    pub status: String,
    #[serde(flatten)]
    pub extra: Extra,
}
