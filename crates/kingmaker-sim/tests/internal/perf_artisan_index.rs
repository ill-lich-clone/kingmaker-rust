//! The forecast index is a lookup accelerator, never an iteration-order authority.
use super::*;
use crate::income_capital::{CapitalEntity, CapitalForecast, CapitalGroup};

fn entity(id: ExternalId, amount: i64) -> CapitalEntity {
    CapitalEntity {
        id, owner_id: "owner".into(), currency_id: "coin".into(),
        minimum_minor: Money(amount), requested_minor: Money(amount),
        possible: true, active: true, feasible_output: 1.0,
    }
}
fn group(owner: ExternalId, currency: ExternalId, amount: i64) -> CapitalGroup {
    CapitalGroup {
        owner_id: owner, currency_id: currency, minimum_minor: Money(amount),
        ids: vec![], active: true, personal_funded_minor: Money(0), retained_profit_minor: Money(0),
    }
}
#[test]
fn perf_forecast_index_preserves_first_matching_record() {
    let f = CapitalForecast {
        entities: vec![entity("z".into(), 1), entity("a".into(), 2), entity("z".into(), 3)],
        groups: vec![group("owner".into(), "coin".into(), 4), group("owner".into(), "coin".into(), 5)],
    };
    let index = ForecastIndex::new(&f);
    let key: ExternalId = "z".into();
    let actual = index.entities.get(&key).copied().unwrap();
    let expected = f.entities.iter().find(|e| e.id == key).unwrap();
    assert!(std::ptr::eq(actual, expected));
    let group_key = (ExternalId::from("owner"), ExternalId::from("coin"));
    assert!(std::ptr::eq(index.groups.get(&group_key).copied().unwrap(), &f.groups[0]));
    assert!(!index.entities.contains_key(&ExternalId::from("absent")));
}
#[test]
fn perf_forecast_index_distinguishes_numeric_ids_and_text() {
    let number = ExternalId::from_json(json!(7)).unwrap();
    let text = ExternalId::from_json(json!("7")).unwrap();
    let f = CapitalForecast {
        entities: vec![entity(number.clone(), 1), entity(text.clone(), 2)],
        groups: vec![group(number.clone(), "coin".into(), 3), group(text.clone(), "coin".into(), 4)],
    };
    let index = ForecastIndex::new(&f);
    assert_eq!(index.entities.get(&number).unwrap().minimum_minor, Money(1));
    assert_eq!(index.entities.get(&text).unwrap().minimum_minor, Money(2));
    assert_eq!(index.groups.get(&(number, "coin".into())).unwrap().minimum_minor, Money(3));
    assert_eq!(index.groups.get(&(text, "coin".into())).unwrap().minimum_minor, Money(4));
}
#[test]
fn perf_new_forecast_gets_new_index_after_state_change() {
    let before = CapitalForecast { entities: vec![entity("e".into(), 1)], groups: vec![] };
    let after = CapitalForecast { entities: vec![entity("e".into(), 9)], groups: vec![] };
    let a = ForecastIndex::new(&before);
    let b = ForecastIndex::new(&after);
    let key: ExternalId = "e".into();
    assert_eq!(a.entities.get(&key).unwrap().minimum_minor, Money(1));
    assert_eq!(b.entities.get(&key).unwrap().minimum_minor, Money(9));
}
