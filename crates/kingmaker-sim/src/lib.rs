pub mod artisan_capital;
pub mod artisan_choices;
pub mod artisan_income;
mod artisan_working_capital;
pub mod budget_cycle;
mod budget_funding;
mod budget_invariants;
pub mod calendar;
mod cohort_assets;
pub mod cohort_cycle;
pub mod cohort_scope;
mod construction;
pub mod construction_cancellation;
pub mod construction_cycle;
mod construction_invariants;
pub mod construction_labor;
pub mod construction_limits;
mod construction_operations;
#[cfg(test)]
#[path = "../tests/internal/construction_operations.rs"]
mod construction_operations_tests;
pub mod construction_payroll;
pub mod construction_rights;
pub mod food_conversion;
#[cfg(test)]
#[path = "../tests/internal/food_conversion_native.rs"]
mod food_conversion_native_tests;
pub mod food_supply_forecast;
mod income_attribution;
pub mod income_basis;
pub mod income_capital;
pub mod income_cycle;
mod income_invariants;
#[cfg(test)]
#[path = "../tests/internal/income_operations.rs"]
mod income_operation_native_tests;
mod income_operations;
pub mod income_recognition;
#[cfg(test)]
#[path = "../tests/internal/income_shipping_recognition.rs"]
mod income_shipping_recognition_tests;
pub mod invariants;
pub mod investment_assessment;
mod investment_invariants;
mod investment_staff;
pub mod investments;
pub mod labor_capacity;
pub mod labor_cycle;
pub mod labor_forecast;
pub mod labor_history;
pub mod labor_history_cycle;
mod labor_identity;
pub mod labor_money;
pub mod labor_offers;
pub mod labor_release;
pub mod labor_roster;
pub mod land_paths;
pub mod world_transaction;
mod read_guard;
pub mod ledger;
pub mod local_food_reserves;
pub mod local_hiring;
pub mod market;
pub mod market_clearing;
pub mod market_cycle;
pub mod market_inputs;
pub mod market_index;
pub mod market_invariants;
mod market_operations;
pub mod market_policy;
#[cfg(test)]
#[path = "../tests/internal/market_policy_storage.rs"]
mod market_policy_storage_tests;
pub mod migration_cycle;
pub mod migration_path;
pub mod mobility_cycle;
pub mod mobility_invariants;
pub mod money_math;
pub mod needs;
mod operation_history;
pub mod operation_session;
pub mod ordering;
pub mod physical_supply;
pub mod pipeline;
#[cfg(test)]
#[path = "../tests/internal/private_income_context.rs"]
mod private_income_context_tests;
pub mod producer_income;
mod producer_reserves;
pub mod production;
pub mod production_activities;
pub mod production_feasibility;
pub mod production_invariants;
pub mod production_payroll;
pub mod production_validation;
pub mod profession_cycle;
mod profession_scope;
mod profit_rights;
pub mod raw_procurement;
mod service_payroll;
pub mod shipment_actions;
pub mod shipment_arrivals;
mod shipment_dispatch;
#[cfg(test)]
#[path = "../tests/internal/shipment_dispatch.rs"]
mod shipment_dispatch_tests;
mod shipping_invariants;
#[cfg(test)]
#[path = "../tests/internal/shipping_invariants.rs"]
mod shipping_invariants_tests;
mod shipping_operations;
#[cfg(test)]
#[path = "../tests/internal/shipping_operations.rs"]
mod shipping_operations_tests;
pub mod state_procurement;
mod stock_provenance;
pub mod storage;
pub mod supply;
pub mod transport_validation;
pub mod trade_offers;
pub mod warehouse_capacity;
pub mod world_audit;
pub mod week_metrics;
pub mod native_week;
pub mod week_history;
mod week_invariants;
#[cfg(test)]
#[path="../tests/internal/week_invariants.rs"]
mod week_invariants_tests;

pub use artisan_capital::{protect_artisan_working_capital, CapitalProtectionResult};
pub use invariants::{
    check_conservation, check_with_operations, CheckStatus, InvariantReport, OperationEvidence,
};
pub use ledger::{LedgerError, LedgerSession, Transaction};
pub use market::{clear_market_orders, MarketOrder, OrderPriority};
/// Authoritative writes derive their own demand and payer from WorldState.
/// Executable internal planners must not grant arbitrary-order write capabilities.
/// ```compile_fail
/// use kingmaker_sim::market_clearing::plan_foundation_market;
/// ```
/// ```compile_fail
/// use kingmaker_sim::market_clearing::apply_market_result;
/// ```
/// ```compile_fail
/// use kingmaker_sim::food_conversion::plan_food_conversion;
/// ```
pub use market_cycle::execute_market_phase;
pub use needs::{calculate_pop_demands, calculate_pop_demands_with_reserves, PopDemand};
pub use pipeline::{simulate_week, WeekSummary};
pub use production::{execute_world_production, ProductionRecipe, ProductionResult};
pub use state_procurement::{execute_state_raw_procurement, ProcurementSummary};
pub use supply::{find_supply_route, SupplyNetwork, SupplyRoute};

pub mod run_control;

#[cfg(test)]
#[path = "../tests/common/production.rs"]
mod production_invariants_fixture;

mod pop_invariant_projection;

#[cfg(test)]
#[path="../tests/internal/construction_invariants.rs"]
mod construction_invariants_tests;

#[cfg(test)]
#[path="../tests/internal/investment_staff_cases.rs"]
mod investment_staff_cases;
