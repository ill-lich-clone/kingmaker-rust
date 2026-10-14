//! Authoritative authored opportunities and building terms; no runtime replacements.
use serde::{Deserialize, Serialize};
use serde_json::Value;
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct ConstructionData {
    #[serde(default)]
    pub investment_opportunities: Vec<Value>,
    #[serde(default)]
    pub construction_catalog: Vec<Value>,
    #[serde(default)]
    pub construction_catalog_by_currency: Option<Value>,
    #[serde(default)]
    pub building_site_grants: Vec<Value>,
    #[serde(default)]
    pub phase_receipts: Vec<ConstructionPhaseReceipt>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConstructionRequest {
    pub expected_revision: u64,
    pub period: String,
    #[serde(default)]
    pub policies: crate::production::ProductionPolicies,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConstructionCancellationRequest {
    pub expected_revision: u64,
    pub event_id: crate::ExternalId,
    pub project_id: crate::ExternalId,
    pub reason: String,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConstructionStage {
    Investments,
    Payroll,
    Progress,
    Cancellation,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConstructionPhaseReceipt {
    pub campaign_id: String,
    pub request: ConstructionRequest,
    pub stage: ConstructionStage,
    pub operations: Vec<crate::operations::EconomicOperation>,
    pub operation_evidence: Vec<crate::operations::OperationEvidence>,
    pub input_evidence: Vec<crate::operations::OperationEvidence>,
    pub registry_changes: Value,
    pub decisions: Vec<Value>,
    pub diagnostics: Vec<Value>,
    pub invariants: crate::audit::InvariantReport,
}
