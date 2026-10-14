//! Authoritative transport terms reused by forecasts and later physical execution.
use crate::production::optional_value;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct TransportData {
    #[serde(default)]
    pub phase_receipts: Vec<TransportPhaseReceipt>,
    #[serde(default)]
    pub transport_assets: Vec<Value>,
    #[serde(default)]
    pub transport_terminals: Vec<Value>,
    #[serde(default)]
    pub supply_routes: Vec<Value>,
    #[serde(default)]
    pub route_reservations: Vec<Value>,
    #[serde(default)]
    pub navigation_geography: Vec<Value>,
    #[serde(default)]
    pub shipment_recovery_plans: Vec<Value>,
    #[serde(default, deserialize_with = "optional_value")]
    pub automatic_land_paths: Option<Value>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransportStage {
    Arrivals,
    LocalFoodReserves,
    Dispatch,
    StateProcurement,
    Interruption,
    Recovery,
    Return,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TransportRequest {
    pub expected_revision: u64,
    pub period: String,
    pub stage: TransportStage,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TransportPhaseReceipt {
    pub campaign_id: String,
    pub request: TransportRequest,
    pub date: crate::calendar::GameDate,
    pub operations: Vec<crate::operations::EconomicOperation>,
    pub operation_evidence: Vec<crate::operations::OperationEvidence>,
    pub input_evidence: Vec<crate::operations::OperationEvidence>,
    pub registry_changes: Value,
    pub diagnostics: Vec<Value>,
    pub decisions: Vec<Value>,
    pub invariants: crate::audit::InvariantReport,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ShipmentAction {
    Interrupt { reason: String },
    Resume { recovery_id: crate::ExternalId },
    Return { recovery_id: crate::ExternalId },
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ShipmentActionRequest {
    pub expected_revision: u64,
    pub event_id: crate::ExternalId,
    pub shipment_id: crate::ExternalId,
    pub actor_id: Option<crate::ExternalId>,
    pub action: ShipmentAction,
}
