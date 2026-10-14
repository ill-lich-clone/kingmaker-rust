use crate::{
    operations::EconomicOperation,
    production::{optional_value, Extra, MoneyRate, ProductionPolicies},
    ExternalId, Money,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Employment {
    pub employment_id: ExternalId,
    pub pop_id: ExternalId,
    pub contract_id: ExternalId,
    pub fte: f64,
    #[serde(flatten)]
    pub extra: Extra,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AdventurerRate {
    pub pop_id: ExternalId,
    pub currency_id: ExternalId,
    #[serde(flatten)]
    pub rate: MoneyRate,
    #[serde(flatten)]
    pub extra: Extra,
}
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct BudgetData {
    #[serde(default)]
    pub profile_id: Option<String>,
    pub employment: Vec<Employment>,
    pub jobs: Vec<Value>,
    pub adventurer_income_rates: Vec<AdventurerRate>,
    #[serde(default, deserialize_with = "optional_value")]
    pub foundation_budget_policy: Option<Value>,
    pub phase_receipts: Vec<BudgetPhaseReceipt>,
    #[serde(default)]
    pub opening_receipts: Vec<BudgetPhaseReceipt>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BudgetRequest {
    pub expected_revision: u64,
    pub period: String,
    #[serde(default)]
    pub policies: ProductionPolicies,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BudgetPhaseReceipt {
    pub campaign_id: String,
    pub request: BudgetRequest,
    pub operations: Vec<EconomicOperation>,
    pub issuance_by_currency: Vec<(ExternalId, Money)>,
    pub requirements: Vec<Value>,
}
