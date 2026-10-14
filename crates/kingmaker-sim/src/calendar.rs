//! The authored calendar has twelve 30-day months, not Gregorian dates.
use crate::{budget_funding::error, operation_session::OpResult};
use kingmaker_core::calendar::GameDate;
pub fn serial_date(date: GameDate) -> OpResult<i64> {
    if date.year < 0 || !(1..=12).contains(&date.month) || !(1..=30).contains(&date.day) {
        return Err(error("Invalid 30-day calendar date"));
    }
    Ok(i64::from(date.year) * 360 + (i64::from(date.month) - 1) * 30 + i64::from(date.day) - 1)
}
pub fn add_days(date: GameDate, days: i64) -> OpResult<GameDate> {
    let n = serial_date(date)?
        .checked_add(days)
        .filter(|n| *n >= 0)
        .ok_or_else(|| error("Calendar date overflow"))?;
    Ok(GameDate {
        year: i32::try_from(n / 360).map_err(|_| error("Calendar year overflow"))?,
        month: (n % 360 / 30 + 1) as u8,
        day: (n % 30 + 1) as u8,
    })
}
