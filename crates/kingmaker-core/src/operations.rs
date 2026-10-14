use crate::production::{optional_value, Extra};
use crate::{ExternalId, Money};
use serde::{Deserialize, Serialize};
use serde_json::Value;
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MoneyReservation {
    #[serde(default, deserialize_with = "optional_value")]
    pub reservation_id: Option<ExternalId>,
    #[serde(default, deserialize_with = "optional_value")]
    pub id: Option<ExternalId>,
    pub owner_id: ExternalId,
    pub currency_id: ExternalId,
    pub purpose: String,
    pub amount_minor: Money,
    #[serde(flatten)]
    pub extra: Extra,
}
/// Compact independent evidence; complete canonical bodies remain in committed history.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OperationEvidence {
    pub event_id: ExternalId,
    pub sha256: String,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Obligation {
    pub obligation_id: ExternalId,
    pub payer_id: ExternalId,
    pub payee_id: ExternalId,
    pub currency_id: ExternalId,
    pub purpose: String,
    pub amount_minor: Money,
    #[serde(default, deserialize_with = "optional_value")]
    pub payee_purpose: Option<String>,
    #[serde(default, deserialize_with = "optional_value")]
    pub tax_rate: Option<f64>,
    #[serde(default, deserialize_with = "optional_value")]
    pub tax_owner_id: Option<ExternalId>,
    #[serde(default, deserialize_with = "optional_value")]
    pub category: Option<String>,
    #[serde(default, deserialize_with = "optional_value")]
    pub pop_id: Option<ExternalId>,
    #[serde(default, deserialize_with = "optional_value")]
    pub employment_id: Option<ExternalId>,
    #[serde(flatten)]
    pub extra: Extra,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CommittedOperation {
    pub event_id: ExternalId,
    pub value: Value,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationKind {
    Transfer,
    CapitalistInvestment,
    ConstructionStaffTransfer,
    ConstructionWait,
    ConstructionProgress,
    ConstructionMaterials,
    ConstructionContractFee,
    FactoryWorkingCapital,
    ProjectRefund,
    ConstructionCancelled,
    ConstructionPhaseBoundary,
    EnterpriseIncome,
    IncomePhaseBoundary,
    EnterpriseDividend,
    ArtisanRetooling,
    ArtisanRetoolingComplete,
    ArtisanCapitalFunding,
    ArtisanCapitalRelease,
    ShipmentArrival,
    ShipmentDispatch,
    ShipmentInterrupted,
    ShipmentRecovery,
    ShipmentReturnDispatch,
    ShipmentReturnArrival,
    SupplyPhaseBoundary,
    StateProcurementPeriod,
    MarketPhaseBoundary,
    AdventurerIncome,
    StateBudgetFunding,
    BudgetFunding,
    BudgetPhaseBoundary,
    NativeWeekBoundary,
    LaborHiringFunding,
    PopCohortSplit,
    PopCohortClaim,
    PopMigrationArrival,
    Payroll,
    DebtPayment,
    StorageRent,
    StorageRentRefund,
    Production,
    Purchase,
    FoodConversion,
    Blocked,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MoneyLeg {
    pub owner_id: ExternalId,
    pub currency_id: ExternalId,
    pub purpose: String,
    pub amount_minor: Money,
}
impl MoneyLeg {
    pub fn address(&self) -> crate::AccountAddress {
        crate::AccountAddress {
            owner_id: self.owner_id.clone(),
            currency_id: self.currency_id.clone(),
            purpose: self.purpose.clone(),
        }
    }
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StockCreate {
    pub owner_id: ExternalId,
    #[serde(deserialize_with = "crate::production::province_id")]
    pub province_id: crate::ProvinceId,
    pub goods_id: ExternalId,
    pub unit: Value,
    #[serde(
        default,
        deserialize_with = "optional_value",
        skip_serializing_if = "Option::is_none"
    )]
    pub facility_id: Option<ExternalId>,
    #[serde(
        default,
        deserialize_with = "optional_value",
        skip_serializing_if = "Option::is_none"
    )]
    pub currency_id: Option<ExternalId>,
    pub inventory_origin: String,
    #[serde(
        default,
        deserialize_with = "optional_value",
        skip_serializing_if = "Option::is_none"
    )]
    pub method_kind: Option<String>,
    #[serde(flatten)]
    pub extra: Extra,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StockDelta {
    pub stock_id: ExternalId,
    pub delta: f64,
    pub create: Option<StockCreate>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EconomicOperation {
    pub event_id: ExternalId,
    #[serde(rename = "type")]
    pub kind: OperationKind,
    #[serde(default)]
    pub debits: Vec<MoneyLeg>,
    #[serde(default)]
    pub credits: Vec<MoneyLeg>,
    #[serde(default)]
    pub stock_changes: Vec<StockDelta>,
    pub obligation: Option<Obligation>,
    #[serde(flatten)]
    pub meta: Extra,
}
impl EconomicOperation {
    pub fn transfer(
        event_id: ExternalId,
        from: crate::AccountAddress,
        to: crate::AccountAddress,
        amount: Money,
    ) -> Self {
        let leg = |a: crate::AccountAddress| MoneyLeg {
            owner_id: a.owner_id,
            currency_id: a.currency_id,
            purpose: a.purpose,
            amount_minor: amount,
        };
        Self {
            event_id,
            kind: OperationKind::Transfer,
            debits: vec![leg(from)],
            credits: vec![leg(to)],
            stock_changes: vec![],
            obligation: None,
            meta: Extra::new(),
        }
    }
}
