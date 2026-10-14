use kingmaker_core::{ExternalId, Money};
use kingmaker_sim::money_math::allocate_big_budget;
use num_bigint::BigInt;
#[test]
fn huge_requested_invoice_does_not_overflow_or_distort_small_actual_budget() {
    let large = BigInt::parse_bytes(b"10000000000000000000000000", 10).unwrap();
    let rows = vec![
        (ExternalId::from("b"), large.clone()),
        (ExternalId::from("a"), large),
    ];
    let actual = allocate_big_budget(&rows, Money(5)).unwrap();
    assert_eq!(
        actual
            .iter()
            .map(|a| (a.id.clone(), a.paid))
            .collect::<Vec<_>>(),
        vec![("a".into(), Money(3)), ("b".into(), Money(2))]
    );
}
