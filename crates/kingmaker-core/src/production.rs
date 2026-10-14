//! Authoritative production terms. Extra authored fields stay attached to their row.
use crate::operations::{CommittedOperation, MoneyReservation, Obligation};
use crate::storage::{GoodsReservation, StorageFacility, StorageReservation};
use crate::{ExternalId, Money, ProvinceId};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
pub use crate::extra::Extra;
pub fn optional_value<'de, D, T>(d: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: serde::de::DeserializeOwned,
{
    let value = Value::deserialize(d)?;
    if value.is_null()
        || value.as_object().is_some_and(|o| {
            o.len() == 1
                && o.get("__kingmaker_codec_v2__").and_then(Value::as_str) == Some("undefined")
        })
    {
        Ok(None)
    } else {
        serde_json::from_value(value)
            .map(Some)
            .map_err(serde::de::Error::custom)
    }
}

pub fn province_id<'de, D: serde::Deserializer<'de>>(d: D) -> Result<ProvinceId, D::Error> {
    ProvinceId::from_external(&ExternalId::deserialize(d)?).map_err(serde::de::Error::custom)
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RecipeInput {
    pub goods_id: ExternalId,
    pub amount: f64,
    #[serde(flatten)]
    pub extra: Extra,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CatalogueGood {
    pub goods_id: ExternalId,
    pub kind: String,
    #[serde(flatten)]
    pub extra: Extra,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProductionMethod {
    pub method_id: ExternalId,
    pub kind: String,
    pub output_goods_id: ExternalId,
    pub output_unit: Value,
    pub status: String,
    #[serde(default, deserialize_with = "optional_value")]
    pub weekly_productivity: Option<f64>,
    pub inputs: Vec<RecipeInput>,
    pub staffing: Vec<ExternalId>,
    #[serde(default)]
    pub technology: Vec<ExternalId>,
    #[serde(flatten)]
    pub extra: Extra,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProductionCatalog {
    pub version: String,
    pub goods: Vec<CatalogueGood>,
    pub methods: Vec<ProductionMethod>,
    #[serde(flatten)]
    pub extra: Extra,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LaborAssignment {
    pub pop_id: ExternalId,
    pub workers_count: f64,
    pub time_fraction: f64,
    #[serde(default, deserialize_with = "optional_value")]
    pub fte: Option<f64>,
    #[serde(default, deserialize_with = "optional_value")]
    pub employment_id: Option<ExternalId>,
    #[serde(default, deserialize_with = "optional_value")]
    pub contract_id: Option<ExternalId>,
    #[serde(default, deserialize_with = "optional_value")]
    pub worker_group_id: Option<ExternalId>,
    #[serde(default, deserialize_with = "optional_value")]
    pub productivity_multiplier: Option<f64>,
    #[serde(default, deserialize_with = "optional_value")]
    pub productivity_status: Option<String>,
    #[serde(flatten)]
    pub extra: Extra,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Producer {
    pub id: ExternalId,
    #[serde(default, deserialize_with = "optional_value")]
    pub owner_id: Option<ExternalId>,
    #[serde(deserialize_with = "province_id")]
    pub province_id: ProvinceId,
    pub method_id: ExternalId,
    #[serde(default, deserialize_with = "optional_value")]
    pub facility_id: Option<ExternalId>,
    #[serde(default, deserialize_with = "optional_value")]
    pub payment_purpose: Option<String>,
    #[serde(default)]
    pub assignments: Vec<LaborAssignment>,
    #[serde(flatten)]
    pub extra: Extra,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RgoTechnology {
    pub id: ExternalId,
    pub status: String,
    pub source: Value,
    #[serde(default, deserialize_with = "optional_value")]
    pub approval_ref: Option<Value>,
    pub places_delta: f64,
    #[serde(flatten)]
    pub extra: Extra,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResourceOperation {
    #[serde(flatten)]
    pub base: Producer,
    pub resource_id: ExternalId,
    #[serde(default, deserialize_with = "optional_value")]
    pub resource_status: Option<String>,
    #[serde(default, deserialize_with = "optional_value")]
    pub approval_ref: Option<Value>,
    #[serde(default, deserialize_with = "optional_value")]
    pub land_access: Option<bool>,
    #[serde(default, deserialize_with = "optional_value")]
    pub initial_multiplier: Option<f64>,
    #[serde(default)]
    pub technology: Vec<RgoTechnology>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FactoryParameters {
    #[serde(default, deserialize_with = "optional_value")]
    pub status: Option<String>,
    #[serde(default, deserialize_with = "optional_value")]
    pub source: Option<Value>,
    #[serde(default, deserialize_with = "optional_value")]
    pub approval_ref: Option<Value>,
    #[serde(rename = "clerk_Bmax")]
    #[serde(default, deserialize_with = "optional_value")]
    pub clerk_bmax: Option<f64>,
    #[serde(default, deserialize_with = "optional_value")]
    pub clerk_ratio: Option<f64>,
    #[serde(rename = "tenure_Bmax")]
    #[serde(default, deserialize_with = "optional_value")]
    pub tenure_bmax: Option<f64>,
    #[serde(rename = "tenure_Tsat")]
    #[serde(default, deserialize_with = "optional_value")]
    pub tenure_tsat: Option<f64>,
    #[serde(flatten)]
    pub extra: Extra,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FactoryEnterprise {
    #[serde(flatten)]
    pub base: Producer,
    #[serde(default, deserialize_with = "optional_value")]
    pub region_id: Option<ExternalId>,
    #[serde(default, deserialize_with = "optional_value")]
    pub status: Option<String>,
    #[serde(default, deserialize_with = "optional_value")]
    pub people_slots: Option<f64>,
    #[serde(default, deserialize_with = "optional_value")]
    pub capacity_source: Option<Value>,
    #[serde(default, deserialize_with = "optional_value")]
    pub equipment_capacity_weekly: Option<f64>,
    #[serde(default, deserialize_with = "optional_value")]
    pub equipment_source: Option<Value>,
    #[serde(default, deserialize_with = "optional_value")]
    pub staff_count: Option<f64>,
    #[serde(default, deserialize_with = "optional_value")]
    pub tenure_days: Option<f64>,
    #[serde(default, deserialize_with = "optional_value")]
    pub parameters: Option<FactoryParameters>,
    #[serde(default)]
    pub implemented_technologies: Vec<ExternalId>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ArtisanEnterprise {
    #[serde(flatten)]
    pub base: Producer,
    #[serde(default, deserialize_with = "optional_value")]
    pub people_slots: Option<f64>,
    #[serde(default, deserialize_with = "optional_value")]
    pub pending_retooling: Option<Value>,
    #[serde(default)]
    pub implemented_technologies: Vec<ExternalId>,
    #[serde(default, deserialize_with = "optional_value")]
    pub working_capital: Option<Value>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RgoExpansion {
    pub id: ExternalId,
    pub rgo_id: ExternalId,
    pub status: String,
    pub kind: String,
    pub completed_levels: f64,
    pub places_per_level: f64,
    #[serde(flatten)]
    pub extra: Extra,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CompletedConstruction {
    pub status: String,
    #[serde(default, deserialize_with = "optional_value")]
    pub province_id: Option<ExternalId>,
    #[serde(default, deserialize_with = "optional_value")]
    pub asset: Option<Value>,
    #[serde(default, deserialize_with = "optional_value")]
    pub level: Option<f64>,
    #[serde(default)]
    pub effects: Extra,
    #[serde(flatten)]
    pub extra: Extra,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MoneyRate {
    pub numerator: String,
    pub denominator: String,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LaborContract {
    pub contract_id: ExternalId,
    #[serde(default, deserialize_with = "optional_value")]
    pub employer_entity_id: Option<ExternalId>,
    #[serde(default, deserialize_with = "optional_value")]
    pub currency_id: Option<ExternalId>,
    #[serde(default, deserialize_with = "optional_value")]
    pub weekly_wage_rate: Option<MoneyRate>,
    #[serde(default, deserialize_with = "optional_value")]
    pub wage_weekly_per_worker: Option<f64>,
    #[serde(default, deserialize_with = "optional_value")]
    pub guaranteed_idle_weekly_rate: Option<MoneyRate>,
    #[serde(default, deserialize_with = "optional_value")]
    pub payment_purpose: Option<String>,
    #[serde(rename = "expectedPayroll")]
    #[serde(default, deserialize_with = "optional_value")]
    pub expected_payroll: Option<Value>,
    #[serde(flatten)]
    pub extra: Extra,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConstructionProjectLabor {
    pub id: ExternalId,
    pub status: String,
    #[serde(default)]
    pub assignments: Vec<LaborAssignment>,
    #[serde(flatten)]
    pub extra: Extra,
}
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct ProductionPolicies {
    #[serde(default)]
    pub taxes: BTreeMap<String, f64>,
    #[serde(default)]
    pub wages: BTreeMap<String, f64>,
    #[serde(default)]
    pub priorities: Vec<String>,
    #[serde(flatten)]
    pub extra: Extra,
}
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct ProductionProfile {
    #[serde(default, deserialize_with = "optional_value")]
    pub productivity_multiplier: Option<f64>,
    #[serde(flatten)]
    pub extra: Extra,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MaintenanceReceipt {
    pub pop_id: ExternalId,
    #[serde(default, deserialize_with = "optional_value")]
    pub period: Option<String>,
    pub fraction: f64,
    pub event_ids: Vec<ExternalId>,
    #[serde(flatten)]
    pub extra: Extra,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProductionRequest {
    pub expected_revision: u64,
    pub period: String,
    #[serde(default)]
    pub policies: ProductionPolicies,
    #[serde(default)]
    pub profile: ProductionProfile,
    #[serde(default)]
    pub maintenance_receipts: Vec<MaintenanceReceipt>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct ProductionData {
    #[serde(default)]
    pub policies: ProductionPolicies,
    #[serde(default, deserialize_with = "optional_value")]
    pub catalog: Option<ProductionCatalog>,
    pub resource_operations: Vec<ResourceOperation>,
    pub factories: Vec<FactoryEnterprise>,
    pub artisans: Vec<ArtisanEnterprise>,
    pub contracts: Vec<LaborContract>,
    pub expansions: Vec<RgoExpansion>,
    pub construction_projects: Vec<ConstructionProjectLabor>,
    pub completed_construction: Vec<CompletedConstruction>,
    pub storage_facilities: Vec<StorageFacility>,
    pub storage_reservations: Vec<StorageReservation>,
    pub goods_reservations: Vec<GoodsReservation>,
    pub money_reservations: Vec<MoneyReservation>,
    pub obligations: Vec<Obligation>,
    // Rows, rather than JSON object keys, preserve numeric/string event IDs in serialization.
    pub committed_operations: Vec<CommittedOperation>,
    pub phase_receipts: Vec<ProductionPhaseReceipt>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProductionOutputReceipt {
    pub assignments: Vec<LaborAssignment>,
    pub activity_id: String,
    pub subject_id: ExternalId,
    pub method_kind: String,
    pub goods_id: ExternalId,
    pub quantity: f64,
    pub fte: f64,
    pub blocked_reason: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProductionPhaseReceipt {
    #[serde(default)]
    pub procurement_activities: Vec<ProcurementActivity>,
    pub planned_outputs: Vec<ProductionOutputReceipt>,
    pub registry_changes: ProductionRegistryChanges,
    pub invariants: crate::audit::InvariantReport,
    pub campaign_id: String,
    pub request: ProductionRequest,
    pub effective_policies: ProductionPolicies,
    pub outputs: Vec<ProductionOutputReceipt>,
    pub payroll: Vec<PayrollReceipt>,
    pub event_ids: Vec<ExternalId>,
    #[serde(default)]
    pub operation_evidence: Vec<crate::operations::OperationEvidence>,
    pub purchase_priorities: Vec<String>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProcurementActivity {
    pub activity_id: String,
    pub subject_id: ExternalId,
    pub method_id: ExternalId,
    pub potential_output: Option<f64>,
    pub labor_output: Option<f64>,
    pub output: f64,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RecordTransition<T> {
    pub before: Option<T>,
    pub after: T,
}
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct ProductionRegistryChanges {
    pub storage: Vec<RecordTransition<StorageReservation>>,
    pub factories: Vec<RecordTransition<FactoryEnterprise>>,
    pub obligations: Vec<RecordTransition<Obligation>>,
    pub money_reservations: Vec<RecordTransition<MoneyReservation>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PayrollReceipt {
    pub event_id: ExternalId,
    pub period: String,
    pub employment_id: ExternalId,
    pub pop_id: ExternalId,
    pub employer_id: ExternalId,
    pub currency_id: ExternalId,
    pub gross_minor: Money,
    pub paid_gross_minor: Money,
    pub tax_minor: Money,
    pub net_minor: Money,
    pub planned_fte: f64,
    pub actual_fte: f64,
    pub idle_guarantee_minor: Money,
    pub tax_rate: f64,
    #[serde(default, deserialize_with = "optional_value")]
    pub tax_owner_id: Option<ExternalId>,
    #[serde(flatten)]
    pub extra: Extra,
}
