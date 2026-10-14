//! Borrowed container views for phase audits; no duplicated author or registry payload.
use crate::row_store::RowView;
#[derive(Clone, PartialEq)]
pub struct ProductionView<'a> {
    pub resource_operations: RowView<'a,crate::ExternalId,crate::production::ResourceOperation>,
    pub factories: RowView<'a,crate::ExternalId,crate::production::FactoryEnterprise>,
    pub artisans: RowView<'a,crate::ExternalId,crate::production::ArtisanEnterprise>,
    pub contracts: RowView<'a,crate::ExternalId,crate::production::LaborContract>,
    pub expansions: RowView<'a,crate::ExternalId,crate::production::RgoExpansion>,
    pub construction_projects: RowView<'a,crate::ExternalId,crate::production::ConstructionProjectLabor>,
    pub completed_construction: RowView<'a,u64,crate::production::CompletedConstruction>,
    pub storage_facilities: RowView<'a,crate::ExternalId,crate::storage::StorageFacility>,
    pub storage_reservations: RowView<'a,crate::ExternalId,crate::storage::StorageReservation>,
    pub goods_reservations: RowView<'a,u64,crate::storage::GoodsReservation>,
    pub money_reservations: RowView<'a,u64,crate::operations::MoneyReservation>,
    pub obligations: RowView<'a,crate::ExternalId,crate::operations::Obligation>,
    pub committed_operations: RowView<'a,crate::ExternalId,crate::operations::CommittedOperation>,
    pub phase_receipts: RowView<'a,u64,crate::production::ProductionPhaseReceipt>,
    pub policies: &'a crate::production::ProductionPolicies,
    pub catalog: &'a Option<crate::production::ProductionCatalog>,
}
impl ProductionView<'_> {
    pub fn to_owned(&self)->crate::production::ProductionData {
        crate::production::ProductionData {
            resource_operations:self.resource_operations.values().cloned().collect(),
            factories:self.factories.values().cloned().collect(),
            artisans:self.artisans.values().cloned().collect(),
            contracts:self.contracts.values().cloned().collect(),
            expansions:self.expansions.values().cloned().collect(),
            construction_projects:self.construction_projects.values().cloned().collect(),
            completed_construction:self.completed_construction.values().cloned().collect(),
            storage_facilities:self.storage_facilities.values().cloned().collect(),
            storage_reservations:self.storage_reservations.values().cloned().collect(),
            goods_reservations:self.goods_reservations.values().cloned().collect(),
            money_reservations:self.money_reservations.values().cloned().collect(),
            obligations:self.obligations.values().cloned().collect(),
            committed_operations:self.committed_operations.values().cloned().collect(),
            phase_receipts:self.phase_receipts.values().cloned().collect(),
            policies:self.policies.clone(),
            catalog:self.catalog.clone(),
        }
    }
}
#[derive(Clone, PartialEq)]
pub struct MarketView<'a> {
    pub territory: RowView<'a,u64,crate::market::MarketTerritory>,
    pub maintenance_agreements: RowView<'a,u64,crate::market::MaintenanceAgreement>,
    pub sale_offers: RowView<'a,u64,crate::market::SaleOffer>,
    pub institutions: RowView<'a,u64,serde_json::Value>,
    pub construction_catalog: RowView<'a,u64,serde_json::Value>,
    pub phase_receipts: RowView<'a,u64,crate::market::MarketPhaseReceipt>,
    pub needs: &'a Option<crate::market::NeedsData>,
    pub price_calibration: &'a Option<crate::market::PriceCalibration>,
    pub parameters: &'a Option<crate::market::FoundationParameters>,
    pub consumer_facility_by_province: &'a Option<serde_json::Value>,
}
impl MarketView<'_> {
    pub fn to_owned(&self)->crate::market::MarketData {
        crate::market::MarketData {
            territory:self.territory.values().cloned().collect(),
            maintenance_agreements:self.maintenance_agreements.values().cloned().collect(),
            sale_offers:self.sale_offers.values().cloned().collect(),
            institutions:self.institutions.values().cloned().collect(),
            construction_catalog:self.construction_catalog.values().cloned().collect(),
            phase_receipts:self.phase_receipts.values().cloned().collect(),
            needs:self.needs.clone(),
            price_calibration:self.price_calibration.clone(),
            parameters:self.parameters.clone(),
            consumer_facility_by_province:self.consumer_facility_by_province.clone(),
        }
    }
}
#[derive(Clone, PartialEq)]
pub struct BudgetView<'a> {
    pub employment: RowView<'a,crate::ExternalId,crate::budget::Employment>,
    pub adventurer_income_rates: RowView<'a,u64,crate::budget::AdventurerRate>,
    pub jobs: RowView<'a,u64,serde_json::Value>,
    pub phase_receipts: RowView<'a,u64,crate::budget::BudgetPhaseReceipt>,
    pub opening_receipts: RowView<'a,u64,crate::budget::BudgetPhaseReceipt>,
    pub profile_id: &'a Option<String>,
    pub foundation_budget_policy: &'a Option<serde_json::Value>,
}
impl BudgetView<'_> {
    pub fn to_owned(&self)->crate::budget::BudgetData {
        crate::budget::BudgetData {
            employment:self.employment.values().cloned().collect(),
            adventurer_income_rates:self.adventurer_income_rates.values().cloned().collect(),
            jobs:self.jobs.values().cloned().collect(),
            phase_receipts:self.phase_receipts.values().cloned().collect(),
            opening_receipts:self.opening_receipts.values().cloned().collect(),
            profile_id:self.profile_id.clone(),
            foundation_budget_policy:self.foundation_budget_policy.clone(),
        }
    }
}
#[derive(Clone, PartialEq)]
pub struct LaborView<'a> {
    pub profiles: RowView<'a,u64,serde_json::Value>,
    pub species: RowView<'a,u64,serde_json::Value>,
    pub phase_receipts: RowView<'a,u64,crate::labor::LaborPhaseReceipt>,
    pub institution_rules: &'a Option<serde_json::Value>,
    pub pop_mobility: &'a Option<serde_json::Value>,
}
impl LaborView<'_> {
    pub fn to_owned(&self)->crate::labor::LaborData {
        crate::labor::LaborData {
            profiles:self.profiles.values().cloned().collect(),
            species:self.species.values().cloned().collect(),
            phase_receipts:self.phase_receipts.values().cloned().collect(),
            institution_rules:self.institution_rules.clone(),
            pop_mobility:self.pop_mobility.clone(),
        }
    }
}
#[derive(Clone, PartialEq)]
pub struct MobilityView<'a> {
    pub capitalist_entities: RowView<'a,u64,serde_json::Value>,
    pub project_investments: RowView<'a,u64,serde_json::Value>,
    pub production_beneficiaries: RowView<'a,u64,serde_json::Value>,
    pub factory_beneficiaries: RowView<'a,u64,serde_json::Value>,
    pub shipments: RowView<'a,u64,serde_json::Value>,
    pub history_receipts: RowView<'a,u64,crate::mobility::LaborHistoryReceipt>,
    pub opening_receipts: RowView<'a,u64,crate::mobility::MobilityOpeningReceipt>,
    pub migration_receipts: RowView<'a,u64,crate::mobility::MigrationPhaseReceipt>,
    pub profession_receipts: RowView<'a,u64,crate::mobility::ProfessionPhaseReceipt>,
    pub cohort_receipts: RowView<'a,u64,crate::mobility::CohortSplitReceipt>,
    pub reference: &'a Option<serde_json::Value>,
}
impl MobilityView<'_> {
    pub fn to_owned(&self)->crate::mobility::MobilityData {
        crate::mobility::MobilityData {
            capitalist_entities:self.capitalist_entities.values().cloned().collect(),
            project_investments:self.project_investments.values().cloned().collect(),
            production_beneficiaries:self.production_beneficiaries.values().cloned().collect(),
            factory_beneficiaries:self.factory_beneficiaries.values().cloned().collect(),
            shipments:self.shipments.values().cloned().collect(),
            history_receipts:self.history_receipts.values().cloned().collect(),
            opening_receipts:self.opening_receipts.values().cloned().collect(),
            migration_receipts:self.migration_receipts.values().cloned().collect(),
            profession_receipts:self.profession_receipts.values().cloned().collect(),
            cohort_receipts:self.cohort_receipts.values().cloned().collect(),
            reference:self.reference.clone(),
        }
    }
}
#[derive(Clone, PartialEq)]
pub struct TransportView<'a> {
    pub transport_assets: RowView<'a,u64,serde_json::Value>,
    pub transport_terminals: RowView<'a,u64,serde_json::Value>,
    pub supply_routes: RowView<'a,u64,serde_json::Value>,
    pub route_reservations: RowView<'a,u64,serde_json::Value>,
    pub navigation_geography: RowView<'a,u64,serde_json::Value>,
    pub shipment_recovery_plans: RowView<'a,u64,serde_json::Value>,
    pub phase_receipts: RowView<'a,u64,crate::transport::TransportPhaseReceipt>,
    pub automatic_land_paths: &'a Option<serde_json::Value>,
}
impl TransportView<'_> {
    pub fn to_owned(&self)->crate::transport::TransportData {
        crate::transport::TransportData {
            transport_assets:self.transport_assets.values().cloned().collect(),
            transport_terminals:self.transport_terminals.values().cloned().collect(),
            supply_routes:self.supply_routes.values().cloned().collect(),
            route_reservations:self.route_reservations.values().cloned().collect(),
            navigation_geography:self.navigation_geography.values().cloned().collect(),
            shipment_recovery_plans:self.shipment_recovery_plans.values().cloned().collect(),
            phase_receipts:self.phase_receipts.values().cloned().collect(),
            automatic_land_paths:self.automatic_land_paths.clone(),
        }
    }
}
#[derive(Clone, PartialEq)]
pub struct IncomeView<'a> {
    pub stock_contributions: RowView<'a,u64,crate::income::StockContribution>,
    pub producer_income: RowView<'a,u64,serde_json::Value>,
    pub phase_receipts: RowView<'a,u64,crate::income::IncomePhaseReceipt>,
}
impl IncomeView<'_> {
    pub fn to_owned(&self)->crate::income::IncomeData {
        crate::income::IncomeData {
            stock_contributions:self.stock_contributions.values().cloned().collect(),
            producer_income:self.producer_income.values().cloned().collect(),
            phase_receipts:self.phase_receipts.values().cloned().collect(),
        }
    }
}
#[derive(Clone, PartialEq)]
pub struct ConstructionView<'a> {
    pub investment_opportunities: RowView<'a,u64,serde_json::Value>,
    pub construction_catalog: RowView<'a,u64,serde_json::Value>,
    pub building_site_grants: RowView<'a,u64,serde_json::Value>,
    pub phase_receipts: RowView<'a,u64,crate::construction::ConstructionPhaseReceipt>,
    pub construction_catalog_by_currency: &'a Option<serde_json::Value>,
}
impl ConstructionView<'_> {
    pub fn to_owned(&self)->crate::construction::ConstructionData {
        crate::construction::ConstructionData {
            investment_opportunities:self.investment_opportunities.values().cloned().collect(),
            construction_catalog:self.construction_catalog.values().cloned().collect(),
            building_site_grants:self.building_site_grants.values().cloned().collect(),
            phase_receipts:self.phase_receipts.values().cloned().collect(),
            construction_catalog_by_currency:self.construction_catalog_by_currency.clone(),
        }
    }
}
#[derive(Clone, PartialEq)]
pub struct WeekView<'a> {
    pub phase_receipts: RowView<'a,u64,crate::week::NativeWeekReceipt>,
}
impl WeekView<'_> {
    pub fn to_owned(&self)->crate::week::WeekData {
        crate::week::WeekData {
            phase_receipts:self.phase_receipts.values().cloned().collect(),
        }
    }
}
