use crate::{
    artisan_capital::CapitalProtectionResult,
    invariants::InvariantReport,
    production::{ProductionRecipe, ProductionResult},
    state_procurement::ProcurementSummary,
};
use kingmaker_core::WorldState;
use std::collections::HashMap;
use thiserror::Error;

pub const MISSING_NATIVE_PHASES: &[&str] = &[
    "full_world_checkpoint_and_restore",
    "autonomous_measured_runner",
];
#[derive(Debug, Error)]
pub enum WeekError {
    #[error("Полный мир ещё не допущен к запуску; нужны проверки: {missing_phases:?}. Detached recipes не являются native-неделей")]
    IncompleteEngine { missing_phases: Vec<&'static str> },
    #[error("Invalid 30-day calendar date or year overflow")]
    InvalidDate,
}
#[derive(Debug)]
pub struct WeekSummary {
    pub revision: u64,
    pub production: ProductionResult,
    pub capital_protection: CapitalProtectionResult,
    pub state_procurement: ProcurementSummary,
    pub invariants: InvariantReport,
    pub elapsed_ms: u64,
}
/// Refuse before cloning/mutating any state. Partial helpers are not a game week.
pub fn simulate_week(
    _world: &mut WorldState,
    _recipes: &HashMap<String, ProductionRecipe>,
) -> Result<WeekSummary, WeekError> {
    Err(WeekError::IncompleteEngine {
        missing_phases: MISSING_NATIVE_PHASES.to_vec(),
    })
}
pub fn advance_week_date(year: i32, month: u8, day: u8) -> Result<(i32, u8, u8), WeekError> {
    let date =
        crate::calendar::add_days(kingmaker_core::calendar::GameDate { year, month, day }, 7)
            .map_err(|_| WeekError::InvalidDate)?;
    Ok((date.year, date.month, date.day))
}
