#[path = "common/labor.rs"]
mod common;
use kingmaker_core::production::ProductionPolicies;
use kingmaker_sim::{
    labor_offers::build_job_offers, labor_roster::build_labor_roster,
    local_hiring::DestinationFunding,
};
#[test]
fn forecast_uses_actual_parent_rate_and_two_week_money_without_spending_it() {
    for cash in [100000, 1540, 0] {
        let w = common::world(cash);
        let before = serde_json::to_value(&w).unwrap();
        let roster = build_labor_roster(&w).unwrap();
        let offer = build_job_offers(&w, &roster).unwrap().offers.remove(0);
        let policies = ProductionPolicies::default();
        let mut funding = DestinationFunding::new(&w, &policies, &roster).unwrap();
        let result = funding.capacity(&w.pops[0], &offer).unwrap();
        assert_eq!(
            result,
            if cash == 100000 {
                600
            } else if cash == 1540 {
                300
            } else {
                0
            }
        );
        assert_eq!(serde_json::to_value(&w).unwrap(), before);
        assert_eq!(funding.capacity(&w.pops[0], &offer).unwrap(), result);
    }
}
#[test]
fn missing_own_currency_is_unmeasured_and_cannot_invent_a_wage() {
    let mut w = common::world(100000);
    w.pops[0].currency_id = None;
    let roster = build_labor_roster(&w).unwrap();
    let offer = build_job_offers(&w, &roster).unwrap().offers.remove(0);
    let policies = ProductionPolicies::default();
    let mut funding = DestinationFunding::new(&w, &policies, &roster).unwrap();
    assert!(funding.capacity(&w.pops[0], &offer).is_err());
}
