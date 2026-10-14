use kingmaker_core::*;
use kingmaker_sim::{
    pipeline::{advance_week_date, WeekError},
    simulate_week,
};
use std::collections::HashMap;
#[test]
fn full_week_refusal_is_atomic_even_with_real_recipes() {
    let mut w = WorldState::default();
    w.campaign = CampaignHeader {
        campaign_id: "fixture".into(),
        revision: 20,
        year: 306,
        month: 1,
        day: 1,
        ..Default::default()
    };
    let before = w.clone();
    let result = simulate_week(&mut w, &HashMap::new());
    assert!(matches!(result, Err(WeekError::IncompleteEngine { .. })));
    assert_eq!(w, before);
    let recipes = HashMap::from([(
        "farm".into(),
        kingmaker_sim::ProductionRecipe {
            inputs: vec![],
            outputs: vec![(Goods::Grain, 100.0)],
            base_labor: 1,
        },
    )]);
    assert!(simulate_week(&mut w, &recipes).is_err());
    assert_eq!(w, before);
}
#[test]
fn calendar_crosses_year_without_advancing_any_world() {
    assert_eq!(advance_week_date(306, 12, 27).unwrap(), (307, 1, 4));
    assert_eq!(advance_week_date(306, 1, 25).unwrap(), (306, 2, 2));
    assert!(advance_week_date(306, 0, 1).is_err());
    assert!(advance_week_date(i32::MAX, 12, 27).is_err());
}
