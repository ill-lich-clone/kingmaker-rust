#![allow(dead_code)]
use kingmaker_core::{budget::*, *};
use serde_json::json;
pub fn world(cash: i64) -> WorldState {
    let mut w = WorldState::default();
    w.campaign = CampaignHeader {
        campaign_id: "budget".into(),
        revision: 20,
        year: 306,
        month: 1,
        day: 1,
        ..Default::default()
    };
    w.provinces.push(ProvinceRecord {
        id: ProvinceId(1),
        owner_state_id: Some("s".into()),
        ..Default::default()
    });
    for (i, (id, sector)) in [("teacher", "education"), ("medic", "medicine")]
        .into_iter()
        .enumerate()
    {
        w.pops.push(PopRecord {
            id: PopId(i as u32 + 1),
            external_id: id.into(),
            province_id: ProvinceId(1),
            size: 1,
            workforce: 1.,
            employed: 1.,
            state_id: Some("s".into()),
            currency_id: Some("X".into()),
            pop_type_id: "clergymen".into(),
            pop_type: "clergymen".into(),
            ..Default::default()
        });
        w.budget_data.employment.push(serde_json::from_value(json!({"employment_id":id,"pop_id":id,"contract_id":id,"job_id":id,"workers_count":1,"time_fraction":1,"fte":1})).unwrap());
        w.budget_data
            .jobs
            .push(json!({"job_id":id,"slots_total":1}));
        w.production_data.contracts.push(serde_json::from_value(json!({"contract_id":id,"employer_entity_id":"s","currency_id":"X","capacity_pool_id":id,"weekly_wage_rate":{"numerator":"1000","denominator":"1"},"budget_state_id":"s","budget_sector":sector,"payment_purpose":format!("state_service:{sector}")})).unwrap());
    }
    w.accounts
        .push(account(10, "treasury:s", "X", "operating_capital", cash));
    w.production_data.policies.taxes.insert("s".into(), 0.1);
    w.rebuild_account_index().unwrap();
    w
}
pub fn account(id: u32, owner: &str, currency: &str, purpose: &str, cash: i64) -> AccountBalance {
    AccountBalance {
        id: AccountId(id),
        external_id: format!("cash:{id}").into(),
        owner_id: owner.into(),
        currency_id: currency.into(),
        purpose: purpose.into(),
        balance: Money(cash),
    }
}
pub fn request(w: &WorldState) -> BudgetRequest {
    BudgetRequest {
        expected_revision: w.campaign.revision,
        period: "w".into(),
        policies: w.production_data.policies.clone(),
    }
}
pub fn funding(w: &mut WorldState, education: f64, medicine: f64) {
    w.production_data.policies.extra.insert(
        "state_budgets".into(),
        json!({"s":{"education":education,"medicine":medicine}}),
    );
}
pub fn adv(w: &mut WorldState) {
    w.pops.push(PopRecord {
        id: PopId(3),
        external_id: "a".into(),
        province_id: ProvinceId(1),
        size: 10,
        workforce: 6.,
        state_id: Some("s".into()),
        currency_id: Some("X".into()),
        pop_type_id: "adventurers".into(),
        pop_type: "adventurers".into(),
        ..Default::default()
    });
    w.budget_data.adventurer_income_rates.push(serde_json::from_value(json!({"id":"a","pop_id":"a","currency_id":"X","numerator":"200","denominator":"1","source":"temporary_adventurer_issuance"})).unwrap());
}
pub fn cash(w: &WorldState, owner: &str, purpose: &str) -> i64 {
    w.account_by_address(&AccountAddress {
        owner_id: owner.into(),
        currency_id: "X".into(),
        purpose: purpose.into(),
    })
    .map(|a| a.balance.0)
    .unwrap_or(0)
}
