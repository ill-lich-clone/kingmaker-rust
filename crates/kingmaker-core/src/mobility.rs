//! Additional authoritative rights; existing labor, stock, claim and need rows are reused.
use crate::production::optional_value;
use serde::{Deserialize, Serialize};
use serde_json::Value;
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct MobilityData {
    #[serde(default)]
    pub history_receipts: Vec<LaborHistoryReceipt>,
    #[serde(default)]
    pub opening_receipts: Vec<MobilityOpeningReceipt>,
    #[serde(default)]
    pub migration_receipts: Vec<MigrationPhaseReceipt>,
    #[serde(default)]
    pub profession_receipts: Vec<ProfessionPhaseReceipt>,
    #[serde(default)]
    pub cohort_receipts: Vec<CohortSplitReceipt>,
    #[serde(default, deserialize_with = "optional_value")]
    pub reference: Option<Value>,
    #[serde(default)]
    pub capitalist_entities: Vec<Value>,
    #[serde(default)]
    pub project_investments: Vec<Value>,
    #[serde(default)]
    pub production_beneficiaries: Vec<Value>,
    #[serde(default)]
    pub factory_beneficiaries: Vec<Value>,
    #[serde(default)]
    pub shipments: Vec<Value>,
}
#[derive(Debug,Clone,PartialEq,Serialize,Deserialize)]
pub struct LaborHistoryReceipt{pub campaign_id:String,pub request:crate::labor::LaborRequest,pub payroll_events:Vec<crate::ExternalId>,pub pop_mobility:Value}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MobilityOpeningReceipt {
    pub campaign_id: String,
    pub request: crate::labor::LaborRequest,
    pub arrivals: MigrationPhaseReceipt,
    pub local: crate::labor::LaborPhaseReceipt,
    pub professions: ProfessionPhaseReceipt,
    pub departures: MigrationPhaseReceipt,
    pub operations: Vec<crate::operations::EconomicOperation>,
    pub invariants: crate::audit::InvariantReport,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MigrationPhase {
    Arrivals,
    Departures,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MigrationPhaseReceipt {
    pub campaign_id: String,
    pub request: crate::labor::LaborRequest,
    pub phase: MigrationPhase,
    pub workforce: u64,
    pub movements: Vec<Value>,
    pub blocked: Vec<Value>,
    pub operations: Vec<crate::operations::EconomicOperation>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProfessionPhaseReceipt {
    pub campaign_id: String,
    pub request: crate::labor::LaborRequest,
    pub changed_workforce: u64,
    pub changes: Vec<Value>,
    pub blocked: Vec<Value>,
    pub operations: Vec<crate::operations::EconomicOperation>,
}
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SplitTarget {
    #[serde(default)]
    pub pop_type_id: Option<crate::ExternalId>,
    #[serde(default)]
    pub province_id: Option<crate::ProvinceId>,
    #[serde(default)]
    pub state_id: Option<crate::ExternalId>,
    #[serde(default)]
    pub currency_id: Option<crate::ExternalId>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CohortSplitRequest {
    pub expected_revision: u64,
    pub event_id: String,
    pub pop_id: crate::ExternalId,
    pub workforce_count: u64,
    #[serde(default)]
    pub target: SplitTarget,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CohortSplitReceipt {
    pub campaign_id: String,
    pub request: CohortSplitRequest,
    pub child_id: Option<crate::ExternalId>,
    pub people_moved: u32,
    pub workforce_moved: u64,
    pub blocked_reason: Option<String>,
    pub operations: Vec<crate::operations::EconomicOperation>,
}
