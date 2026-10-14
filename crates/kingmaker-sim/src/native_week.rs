//! Complete source-ordered native week, calculated privately and published exactly once.
use kingmaker_core::diagnostics as profile;
use kingmaker_core::world_read::WorldRead;
use crate::{
    budget_funding::error,
    operation_history::operation_evidence,
    operation_session::{OpResult, OperationSession},
    week_history::*,
};
use kingmaker_core::{calendar::GameDate, operations::*, week::*, *};
use serde_json::json;
use std::time::Instant;
pub struct PreparedNativeWeek {
    reads: Option<Vec<crate::read_guard::Fingerprint>>,
    state: Option<crate::world_transaction::PreparedWorldDelta>,
    receipt: NativeWeekReceipt,
}
impl PreparedNativeWeek {
    /// Read-only measurement before publication; callers cannot alter or manufacture authority.
    pub fn preview<'a>(&'a self,base:&'a dyn WorldRead)->Option<(crate::world_transaction::PreparedWorldView<'a>,&'a NativeWeekReceipt)> {
        self.state.as_ref().map(|delta|(delta.view(base),&self.receipt))
    }
}
pub fn prepare_native_week(
    w: &WorldState,
    r: &NativeWeekRequest,
    mut progress: impl FnMut(&str),
) -> OpResult<PreparedNativeWeek> {
    try_prepare_native_week(w, r, |phase| {
        progress(phase);
        Ok(())
    })
}
pub fn try_prepare_native_week(
    w: &WorldState,
    r: &NativeWeekRequest,
    mut progress: impl FnMut(&str) -> OpResult<()>,
) -> OpResult<PreparedNativeWeek> {
kingmaker_core::diagnostics::measure("week.prepare", kingmaker_core::diagnostics::Visibility::Boundary, || {
    let indexed=kingmaker_core::world_read::BorrowedWorld::new(w).map_err(crate::operation_session::OperationError)?;
    let w:&dyn WorldRead=&indexed;
    if let Some(receipt) = profile::measure("week.history", profile::Visibility::Boundary, || validate_week_history(w, r))? {
        return Ok(PreparedNativeWeek {
            reads: None,
            state: None,
            receipt,
        });
    }
    if r.period.is_empty() || r.expected_revision != w.campaign().revision {
        return Err(error("Invalid native week period/revision"));
    }
    let start_date = GameDate::from(w.campaign());
    let end_date = crate::calendar::add_days(start_date, 7)?;
    let end_revision = r
        .expected_revision
        .checked_add(1)
        .ok_or_else(|| error("Native week revision overflow"))?;
    crate::production_validation::validate_production_catalog(w)?;
    let mut policies = w.production_policies().clone();
    policies.taxes.extend(r.policies.taxes.clone());
    policies.wages.extend(r.policies.wages.clone());
    policies.extra.extend(r.policies.extra.clone());
    if !r.policies.priorities.is_empty() {
        policies.priorities = r.policies.priorities.clone();
    }
    let labor = kingmaker_core::labor::LaborRequest {
        expected_revision: r.expected_revision,
        period: r.period.clone(),
        policies: policies.clone(),
    };
    let budget = kingmaker_core::budget::BudgetRequest {
        expected_revision: r.expected_revision,
        period: r.period.clone(),
        policies: policies.clone(),
    };
    let income = kingmaker_core::income::IncomeRequest {
        expected_revision: r.expected_revision,
        period: r.period.clone(),
        policies: policies.clone(),
    };
    let construction = kingmaker_core::construction::ConstructionRequest {
        expected_revision: r.expected_revision,
        period: r.period.clone(),
        policies: policies.clone(),
    };
    let transport = |stage| kingmaker_core::transport::TransportRequest {
        expected_revision: r.expected_revision,
        period: r.period.clone(),
        stage,
    };
    let market = |stage| kingmaker_core::market::MarketRequest {
        expected_revision: r.expected_revision,
        period: r.period.clone(),
        stage,
        profile: r.market_profile.clone(),
        priorities: r
            .market_priorities
            .clone()
            .or_else(|| (!policies.priorities.is_empty()).then(|| policies.priorities.clone())),
    };
    let basis = profile::measure("week.income_basis", profile::Visibility::Boundary, || crate::income_basis::capture_income_opening(w, &income))?;
    let transaction_scope=profile::scope("week.transaction",profile::Visibility::Detail);
    let mut state=crate::world_transaction::WorldTransaction::from_borrowed(&indexed);
    transaction_scope.finish(profile::Outcome::Ok);
    let first_operation = state.committed_operations().len();
    let mut phase_order = vec![];
    let mut timings = vec![];
    let mut diagnostics = vec![];
    macro_rules! phase {
        ($name:literal,$body:expr) => {{
            crate::run_control::check($name)?;
            progress($name).map_err(|e| {
                error(&format!(
                    "Native week cancelled before phase {}: {e}",
                    $name
                ))
            })?;
            let clock = Instant::now();
            let out = profile::measure(concat!("phase.",$name), profile::Visibility::Boundary, || $body);
            let out = out.map_err(|e| error(&format!("Native week phase {}: {e}", $name)))?;
            phase_order.push($name.into());
            timings.push((
                $name.into(),
                clock.elapsed().as_millis().min(u128::from(u64::MAX)) as u64,
            ));
            out
        }};
    }
    state.replace_storage_facilities(phase!(
        "storage_capacity",
        crate::warehouse_capacity::derive_storage_capacities(&state)
    ))?;
    use kingmaker_core::transport::TransportStage;
    phase!(
        "arrivals",
        crate::shipment_arrivals::execute_shipment_arrivals_in(
            &mut state,
            &transport(TransportStage::Arrivals)
        )
    );
    phase!(
        "mobility",
        crate::mobility_cycle::execute_mobility_opening_in(&mut state, &labor)
    );
    phase!(
        "budget_opening",
        crate::budget_cycle::execute_budget_opening_in(&mut state, &budget)
    );
    phase!(
        "investments",
        crate::investments::execute_automatic_investments_in(&mut state, &construction)
    );
    phase!(
        "artisan_opening",
        crate::income_cycle::execute_artisan_opening_with_profile_in(
            &mut state,
            &income,
            &r.market_profile
        )
    );
    phase!(
        "service_payroll",
        crate::budget_cycle::execute_budget_services_in(&mut state, &budget)
    );
    use kingmaker_core::market::MarketStage;
    let maintenance = phase!(
        "maintenance",
        crate::market_cycle::execute_market_phase_in(&mut state, &market(MarketStage::Maintenance))
    );
    if state.market_parameters()
        .as_ref()
        .and_then(|p| p.instant_food)
        == Some(true)
    {
        phase!(
            "early_food",
            crate::market_cycle::execute_market_phase_in(&mut state, &market(MarketStage::EarlyFood))
        );
    }
    phase!(
        "builder_payroll",
        crate::construction_payroll::execute_construction_payroll_in(&mut state, &construction)
    );
    let production = kingmaker_core::production::ProductionRequest {
        expected_revision: r.expected_revision,
        period: r.period.clone(),
        policies: policies.clone(),
        profile: r.production_profile.clone(),
        maintenance_receipts: maintenance.maintenance_receipts,
    };
    phase!(
        "production",
        crate::production::execute_production_cycle_in(&mut state, &production)
    );
    phase!(
        "construction",
        crate::construction_cycle::execute_construction_in(&mut state, &construction)
    );
    state.replace_sale_offers(phase!(
        "market_offers",
        crate::trade_offers::plan_trade_offers(&state, &r.period)
    ))?;
    let consumed = phase!(
        "market",
        crate::market_cycle::execute_market_phase_in(&mut state, &market(MarketStage::ClosingMarket))
    );
    diagnostics.extend(consumed.diagnostics);
    state.replace_sale_offers(phase!(
        "supply_offers",
        crate::trade_offers::plan_trade_offers(&state, &r.period)
    ))?;
    phase!(
        "food_reserves",
        crate::local_food_reserves::execute_local_food_reserves_in(
            &mut state,
            &transport(TransportStage::LocalFoodReserves)
        )
    );
    let supply = phase!(
        "supply",
        crate::physical_supply::execute_physical_supply_in(
            &mut state,
            &transport(TransportStage::Dispatch)
        )
    );
    diagnostics.extend(supply.diagnostics);
    let procurement = phase!(
        "state_procurement",
        crate::raw_procurement::execute_raw_procurement_in(&mut state, &budget)
    );
    diagnostics.extend(procurement.diagnostics);
    let closing = phase!(
        "income",
        crate::income_cycle::execute_income_closing_in(&mut state, &income, &basis)
    );
    diagnostics.extend(closing.diagnostics);
    phase!(
        "labor_history",
        crate::labor_history_cycle::execute_labor_history_in(&mut state, &labor)
    );
    profile::measure("week.finalize",profile::Visibility::Boundary,|| {
    let operations = state.committed_operations().iter().skip(first_operation)
        .map(|o| {
            serde_json::from_value::<EconomicOperation>(o.value.clone())
                .map_err(crate::operation_session::OperationError::from)
        })
        .collect::<OpResult<Vec<_>>>()?;
    let invariants = phase!(
        "closing",
        crate::week_invariants::verify_week(w, &state, &operations)
    );
    let placeholder =
        serde_json::from_value(json!({"event_id":boundary_id(w,r),"type":"native_week_boundary"}))?;
    let mut receipt = NativeWeekReceipt {
        campaign_id: w.campaign().campaign_id.clone(),
        request: r.clone(),
        start_date,
        end_date,
        end_revision,
        phase_order,
        operation_evidence: operations
            .iter()
            .map(operation_evidence)
            .collect::<OpResult<_>>()?,
        registry_evidence: registry_evidence(&state, r)?,
        diagnostics,
        timings,
        invariants,
        boundary: placeholder,
    };
    receipt.boundary = serde_json::from_value(
        json!({"event_id":boundary_id(w,r),"type":"native_week_boundary","period":r.period,"receipt_sha256":receipt_digest(&receipt)?}),
    )?;
    let mut session = OperationSession::new(&state)?;
    session.stage_week_boundary(&receipt.boundary)?;
    let changes = session.finish()?;
    state.apply_phase(changes)?;
    let mut campaign=state.campaign().clone();
    campaign.year=end_date.year;campaign.month=end_date.month;campaign.day=end_date.day;campaign.revision=end_revision;
    state.set_campaign(campaign)?;
    let key=state.week_phase_receipts().next_key().map_err(crate::operation_session::OperationError)?;
    state.put_week_phase_receipts(key,receipt.clone())?;
    if validate_week_history(&state, r)?.as_ref() != Some(&receipt) {
        return Err(error("Native week publication evidence mismatch"));
    }
    Ok(PreparedNativeWeek {
        reads: Some(crate::read_guard::world_fingerprint(w)?),
        state: Some(state.finish()?),
        receipt,
    })

})
})
}
pub fn apply_native_week(w: &mut WorldState, p: PreparedNativeWeek) -> OpResult<NativeWeekReceipt> {
kingmaker_core::diagnostics::measure("week.apply", kingmaker_core::diagnostics::Visibility::Boundary, || {
    if let Some(before) = p.reads {
        if before != crate::read_guard::world_fingerprint(w)? {
            return Err(error("Prepared native week consulted world changed"));
        }
        crate::world_transaction::apply_world_delta(w,p.state.ok_or_else(||error("Native week state missing"))?)?;
    } else if validate_week_history(w, &p.receipt.request)?.as_ref() != Some(&p.receipt) {
        return Err(error("Prepared native week replay changed"));
    }
    Ok(p.receipt)

})
}
pub fn execute_native_week(
    w: &mut WorldState,
    r: &NativeWeekRequest,
    progress: impl FnMut(&str),
) -> OpResult<NativeWeekReceipt> {
    let p = prepare_native_week(w, r, progress)?;
    apply_native_week(w, p)
}

pub fn try_prepare_native_week_controlled(w:&WorldState,r:&NativeWeekRequest,control:&crate::run_control::RunControl,progress:impl FnMut(&str)->OpResult<()>)->OpResult<PreparedNativeWeek> {
    control.run(||try_prepare_native_week(w,r,progress))
}
