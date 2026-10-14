use kingmaker_core::production::MoneyRate;
#[test]
fn budget_remainder_uses_js_utf16_id_order() {
    use kingmaker_core::Money;
    use kingmaker_sim::money_math::*;
    let rows = vec![
        BudgetRequest {
            id: "\u{e000}".into(),
            amount: Money(1),
        },
        BudgetRequest {
            id: "𐀀".into(),
            amount: Money(1),
        },
    ];
    let result = allocate_budget(&rows, Money(1)).unwrap();
    assert_eq!(
        result
            .iter()
            .find(|r| r.id.as_str() == Some("𐀀"))
            .unwrap()
            .paid,
        Money(1)
    );
}
use kingmaker_core::{ExternalId, Money};
use kingmaker_sim::money_math::*;
use serde_json::json;
use std::process::Command;

#[test]
fn rounding_and_budget_remainders_match_js() {
    assert_eq!(round_ratio(&1.into(), &2.into()).unwrap().to_string(), "1");
    assert_eq!(
        round_ratio(&(-1).into(), &2.into()).unwrap().to_string(),
        "-1"
    );
    assert_eq!(decimal_ratio("2.5e-2").unwrap().0.to_string(), "25");
    assert_eq!(decimal_ratio("2.5e-2").unwrap().1.to_string(), "1000");
    let rate = MoneyRate {
        numerator: "5".into(),
        denominator: "2".into(),
    };
    assert_eq!(rate_pay(&rate, 1.0, 1.0).unwrap(), Money(3));
    assert_eq!(rate_pay(&rate, 1.0, 0.0).unwrap(), Money(0));
    let requests = vec![
        BudgetRequest {
            id: "b".into(),
            amount: Money(5),
        },
        BudgetRequest {
            id: "a".into(),
            amount: Money(5),
        },
    ];
    let a = allocate_budget(&requests, Money(3)).unwrap();
    assert_eq!(
        a.iter().map(|a| (a.id.clone(), a.paid)).collect::<Vec<_>>(),
        vec![("a".into(), Money(2)), ("b".into(), Money(1))]
    );
    assert_eq!(
        a,
        allocate_budget(&requests.into_iter().rev().collect::<Vec<_>>(), Money(3)).unwrap()
    );
}
#[test]
fn unsupported_values_reject_without_float_cash() {
    assert!(decimal_ratio("NaN").is_err());
    assert!(decimal_ratio("Infinity").is_err());
    assert!(decimal_ratio("1e-9223372036854775807").is_err());
    assert!(round_ratio(&1.into(), &0.into()).is_err());
    assert!(multiply_money(Money(1), f64::INFINITY).is_err());
    assert!(multiply_money(Money(1), -1.0).is_err());
    assert!(multiply_money(Money(i64::MAX), 2.0).is_err());
    assert!(allocate_budget(&[], Money(-1)).is_err());
    assert!(allocate_budget(
        &[BudgetRequest {
            id: "a".into(),
            amount: Money(-1)
        }],
        Money(1)
    )
    .is_err());
    let rate = MoneyRate {
        numerator: "100000000000000000000".into(),
        denominator: "100000000000000000000".into(),
    };
    assert_eq!(
        rate_pay(&rate, 1.0, 1.0).unwrap(),
        Money(1),
        "Large intermediate can have a legal final sum"
    );
}
#[test]
fn budget_allocation_preserves_external_id_types() {
    let r = vec![
        BudgetRequest {
            id: ExternalId::from(7u32),
            amount: Money(4),
        },
        BudgetRequest {
            id: ExternalId::from("7"),
            amount: Money(4),
        },
    ];
    let paid = allocate_budget(&r, Money(4)).unwrap();
    assert_eq!(paid[0].id, r[0].id);
    assert_eq!(paid[1].id, r[1].id);
    assert_eq!(paid.iter().map(|a| a.paid.units()).sum::<i64>(), 4);
}
#[test]
fn float_decimal_and_money_rounding_agree_with_real_js() {
    let values: Vec<f64> = vec![
        0.0,
        0.1,
        0.3,
        0.3333333333333333,
        1e-7,
        1e20,
        1e23,
        1000000000000000100.0,
        0.000001,
        2.2250738585072014e-308,
    ];
    let script = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../scripts/tests/money-math-oracle.cjs");
    let output = Command::new("node")
        .arg(script)
        .arg(json!(values).to_string())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let expected: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    for (i, value) in values.iter().enumerate() {
        let (n, d) = decimal_ratio(&value.to_string()).unwrap();
        // Exponent spelling can differ; compare the actual rational value by cross multiplication.
        let (en, ed) = decimal_ratio(expected[i]["text"].as_str().unwrap()).unwrap();
        assert_eq!(&n * &ed, &en * &d, "Float {value}");
        if value.abs() < 1e10 {
            assert_eq!(
                multiply_money(Money(101), *value)
                    .unwrap()
                    .units()
                    .to_string(),
                expected[i]["minor"].as_str().unwrap()
            );
        }
    }
}
