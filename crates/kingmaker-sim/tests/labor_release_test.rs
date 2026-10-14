#[path = "common/budget.rs"]
mod budget;
#[path = "common/production.rs"]
mod production;
use kingmaker_core::*;
use kingmaker_sim::labor_release::{apply_staff_change, release_employment};
use serde_json::json;

#[test]
fn active_builders_are_protected_after_two_unpaid_weeks() {
    let mut w = budget::world(0);
    w.production_data.construction_projects.push(serde_json::from_value(json!({"id":"project","status":"building","assignments":[{"employment_id":"teacher","pop_id":"teacher","contract_id":"teacher","workers_count":1,"time_fraction":1}]})).unwrap());
    w.labor_data.pop_mobility = Some(json!({"unpaid_weeks":{"teacher":2}}));
    let changes = release_employment(&w, "w").unwrap();
    assert_eq!(changes.released_fte, 0.);
    assert_eq!(
        changes.blocked[0]["reason"],
        json!("protected_construction")
    );
}
#[test]
fn partial_release_and_replay_do_not_remove_extra_people_or_claims() {
    let mut w = budget::world(0);
    w.pops[0].workforce = 2.;
    w.budget_data.employment[0].fte = 2.;
    w.budget_data.employment[0]
        .extra
        .insert("workers_count".into(), json!(2));
    let changes = release_employment(&w, "w").unwrap();
    assert_eq!(changes.released_fte, 1.);
    changes.apply(&mut w).unwrap();
    assert_eq!(w.pops[0].employed, 1.);
    assert_eq!(w.pops[0].unemployed, 1.);
    let after = w.clone();
    let changes = release_employment(&w, "w").unwrap();
    assert_eq!(changes.released_fte, 0.);
    changes.apply(&mut w).unwrap();
    assert_eq!(w, after);
}
#[test]
fn changed_profession_and_stale_workplace_release_without_losing_pay_claims() {
    for stale in [false, true] {
        let mut w = budget::world(0);
        if stale {
            w.production_data.contracts[0]
                .extra
                .insert("building_id".into(), json!("retired-factory"));
            w.budget_data.jobs[0]["building_id"] = json!("active-rgo");
        } else {
            w.production_data.contracts[0]
                .extra
                .insert("pop_type_id".into(), json!("clerks"));
            w.pops[0].pop_type_id = "artisans".into();
        }
        w.production_data.obligations.push(serde_json::from_value(json!({"obligation_id":"old","payer_id":"s","payee_id":"teacher","currency_id":"X","purpose":"state_service:education","amount_minor":"900"})).unwrap());
        let old_accounts = w.accounts.clone();
        let old_debts = w.production_data.obligations.clone();
        let result = release_employment(&w, "w").unwrap();
        assert_eq!(result.released_fte, 1.);
        let mut next = w.clone();
        result.apply(&mut next).unwrap();
        assert_eq!(next.pops[0].unemployed, 1.);
        assert_eq!(next.accounts, old_accounts);
        assert_eq!(next.production_data.obligations, old_debts);
        assert!(!next
            .budget_data
            .employment
            .iter()
            .any(|e| e.pop_id == ExternalId::from("teacher")));
    }
}
#[test]
fn capacity_shrink_two_unpaid_weeks_and_closed_places_release() {
    for reason in 0..3 {
        let mut w = budget::world(0);
        match reason {
            0 => w.budget_data.jobs[0]["slots_total"] = json!(0),
            1 => w.labor_data.pop_mobility = Some(json!({"unpaid_weeks":{"teacher":2}})),
            _ => {
                w.budget_data.employment[0]
                    .extra
                    .insert("building_id".into(), json!("school"));
                w.market_data
                    .institutions
                    .push(json!({"building_id":"school","operational_status":"closed"}));
            }
        }
        let changes = release_employment(&w, "w").unwrap();
        assert_eq!(changes.released_fte, 1.);
    }
}
#[test]
fn factory_staff_changes_use_author_formula_and_validate_counts() {
    let mut f = production::world().production_data.factories.remove(0);
    f.staff_count = Some(10.);
    f.tenure_days = Some(30.);
    let next = apply_staff_change(&f, 2, 4).unwrap();
    assert_eq!(next.staff_count, Some(12.));
    assert_eq!(next.tenure_days, Some(20.));
    assert_eq!(f.staff_count, Some(10.));
    assert!(apply_staff_change(&f, 11, 0).is_err());
}
#[test]
fn unchanged_zero_fte_artisan_assignment_survives_release() {
    let mut w = production::world();
    w.pops[0].pop_type_id = "artisans".into();
    let mut a = serde_json::to_value(&w.production_data.factories[0]).unwrap();
    a["assignments"][0]["time_fraction"] = serde_json::json!(0);
    a["people_slots"] = serde_json::json!(10);
    w.production_data
        .artisans
        .push(serde_json::from_value(a).unwrap());
    w.production_data.factories.clear();
    let changes = kingmaker_sim::labor_release::release_employment(&w, "w").unwrap();
    changes.apply(&mut w).unwrap();
    assert_eq!(w.production_data.artisans[0].base.assignments.len(), 1);
}
