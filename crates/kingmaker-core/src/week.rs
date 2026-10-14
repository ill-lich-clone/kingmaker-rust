//! Canonical native game-week history; wall-clock timings are technical diagnostics.
use crate::{
    audit::InvariantReport,
    calendar::GameDate,
    market::MarketProfile,
    operations::{EconomicOperation, OperationEvidence},
    production::{ProductionPolicies, ProductionProfile},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct NativeWeekRequest {
    pub expected_revision: u64,
    pub period: String,
    #[serde(default)]
    pub policies: ProductionPolicies,
    #[serde(default)]
    pub market_profile: MarketProfile,
    #[serde(default)]
    pub market_priorities: Option<Vec<String>>,
    #[serde(default)]
    pub production_profile: ProductionProfile,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NativeWeekReceipt {
    pub campaign_id: String,
    pub request: NativeWeekRequest,
    pub start_date: GameDate,
    pub end_date: GameDate,
    pub end_revision: u64,
    pub phase_order: Vec<String>,
    pub operation_evidence: Vec<OperationEvidence>,
    pub registry_evidence: Vec<(String, String)>,
    pub diagnostics: Vec<Value>,
    pub timings: Vec<(String, u64)>,
    pub invariants: InvariantReport,
    pub boundary: EconomicOperation,
}
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct WeekData {
    #[serde(default)]
    pub phase_receipts: Vec<NativeWeekReceipt>,
}
