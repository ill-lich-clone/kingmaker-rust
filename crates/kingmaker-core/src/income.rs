//! Mutable native income history; authored unknown terms remain intact.
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct IncomeData {
    #[serde(default)]
    pub producer_income: Vec<Value>,
    /// Derived quantities in pooled produced stocks, never authored ownership shares.
    #[serde(default)]
    pub stock_contributions: Vec<StockContribution>,
    #[serde(default)]
    pub phase_receipts: Vec<IncomePhaseReceipt>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StockContribution {
    pub stock_id: crate::ExternalId,
    pub producer_id: crate::ExternalId,
    pub quantity: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IncomeRequest {
    pub expected_revision: u64,
    pub period: String,
    #[serde(default)]
    pub policies: crate::production::ProductionPolicies,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IncomeStage {
    ArtisanOpening,
    ClosingIncome,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IncomeRegistryChanges {
    pub producer_income: Vec<Value>,
    pub artisans: Vec<crate::production::ArtisanEnterprise>,
    pub stock_contributions: Vec<StockContribution>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IncomePhaseReceipt {
    pub campaign_id: String,
    pub request: IncomeRequest,
    pub stage: IncomeStage,
    #[serde(default)]
    pub opening_profile: Option<crate::market::MarketProfile>,
    pub effective_policies: crate::production::ProductionPolicies,
    pub operations: Vec<crate::operations::EconomicOperation>,
    pub boundary: crate::operations::EconomicOperation,
    pub operation_evidence: Vec<crate::operations::OperationEvidence>,
    pub input_evidence: Vec<crate::operations::OperationEvidence>,
    pub registry_changes: IncomeRegistryChanges,
    pub diagnostics: Vec<Value>,
    pub invariants: crate::audit::InvariantReport,
}
