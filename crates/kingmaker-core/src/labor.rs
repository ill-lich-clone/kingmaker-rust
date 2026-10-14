//! Additional authored terms; jobs/employment/contracts remain in their existing tables.
use crate::production::optional_value;
use serde::{Deserialize, Serialize};
use serde_json::Value;
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct LaborData {
    #[serde(default)]
    pub phase_receipts: Vec<LaborPhaseReceipt>,
    #[serde(default, deserialize_with = "optional_value")]
    pub institution_rules: Option<Value>,
    #[serde(default)]
    pub profiles: Vec<Value>,
    #[serde(default)]
    pub species: Vec<Value>,
    #[serde(default, deserialize_with = "optional_value")]
    pub pop_mobility: Option<Value>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LaborRequest {
    pub expected_revision: u64,
    pub period: String,
    pub policies: crate::production::ProductionPolicies,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LaborPhaseReceipt {
    pub campaign_id: String,
    pub request: LaborRequest,
    pub released_fte: f64,
    pub hired_fte: f64,
    pub released: Vec<Value>,
    pub hires: Vec<Value>,
    pub blocked: Vec<Value>,
    pub operations: Vec<crate::operations::EconomicOperation>,
}
