use kingmaker_core::calendar::GameDate;
use kingmaker_sim::{
    calendar::{add_days, serial_date},
    pipeline::advance_week_date,
};
fn date(year: i32, month: u8, day: u8) -> GameDate {
    GameDate { year, month, day }
}
#[test]
fn real_thirty_day_month_rollover_and_year_boundary_match_engine() {
    assert_eq!(add_days(date(306, 1, 29), 7).unwrap(), date(306, 2, 6));
    assert_eq!(add_days(date(306, 12, 30), 7).unwrap(), date(307, 1, 7));
    assert_eq!(
        serial_date(date(307, 1, 7)).unwrap() - serial_date(date(306, 12, 30)).unwrap(),
        7
    );
    assert_eq!(advance_week_date(306, 12, 30).unwrap(), (307, 1, 7));
}
#[test]
fn invalid_negative_year_and_overflow_do_not_form_dates() {
    for d in [date(-1, 1, 1), date(306, 0, 1), date(306, 1, 31)] {
        assert!(serial_date(d).is_err());
    }
    assert!(add_days(date(i32::MAX, 12, 30), 7).is_err());
    assert!(add_days(date(0, 1, 1), -1).is_err());
    assert!(advance_week_date(-1, 1, 1).is_err());
}
