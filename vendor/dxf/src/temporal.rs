//! DXF dates are civil Julian days (midnight), not astronomical noon days.
//! Domain: Gregorian years 0001..9999. Zero is rejected, not an epoch sentinel.
//! Dates round to the nearest second; elapsed days round to the nearest ns.
use crate::{DxfError, DxfResult};
use chrono::{DateTime, Datelike, Local, NaiveDate, Offset, TimeZone, Utc};
use std::time::Duration;

fn date<T: TimeZone>(zone: &T, value: f64) -> DxfResult<DateTime<T>> {
    if !value.is_finite() || !(1_721_426.0..5_373_485.0).contains(&value) {
        return Err(DxfError::InvalidTemporalValue);
    }
    let day = value.floor();
    let seconds = ((value - day) * 86_400.0).round() as i64;
    let civil = NaiveDate::from_num_days_from_ce_opt((day - 1_721_425.0) as i32)
        .and_then(|d| d.and_hms_opt(0, 0, 0))
        .and_then(|d| chrono::TimeDelta::try_seconds(seconds).and_then(|t| d.checked_add_signed(t)))
        .filter(|d| (1..=9999).contains(&d.year()))
        .ok_or(DxfError::InvalidTemporalValue)?;
    // Ambiguous or nonexistent local times are errors, never guessed offsets.
    zone.from_local_datetime(&civil)
        .single()
        .ok_or(DxfError::InvalidTemporalValue)
}
pub(crate) fn as_datetime_local(value: f64) -> DxfResult<DateTime<Local>> {
    date(&Local, value)
}
pub(crate) fn as_datetime_utc(value: f64) -> DxfResult<DateTime<Utc>> {
    date(&Utc, value)
}
pub(crate) fn sunstudy(day: i32, seconds: i32) -> DxfResult<DateTime<Local>> {
    if !(0..86_400).contains(&seconds) {
        return Err(DxfError::InvalidTemporalValue);
    }
    date(&Local, f64::from(day) + f64::from(seconds) / 86_400.0)
}
pub(crate) fn as_duration(days: f64) -> DxfResult<Duration> {
    Duration::try_from_secs_f64(days * 86_400.0).map_err(|_| DxfError::InvalidTemporalValue)
}
pub(crate) fn duration_as_double(value: Duration) -> f64 {
    value.as_secs_f64() / 86_400.0
}
fn as_double<T: TimeZone>(value: DateTime<T>) -> f64 {
    // UTC seconds plus an offset fit i128 even for externally constructed
    // chrono extremes. Avoid DateTime's infallible local calendar accessors.
    let local_seconds =
        i128::from(value.timestamp()) + i128::from(value.offset().fix().local_minus_utc());
    local_seconds.div_euclid(86_400) as f64
        + 2_440_588.0
        + (local_seconds.rem_euclid(86_400) as f64
            + f64::from(value.timestamp_subsec_nanos()) / 1e9)
            / 86_400.0
}
pub(crate) fn as_double_local(value: DateTime<Local>) -> f64 {
    as_double(value)
}
pub(crate) fn as_double_utc(value: DateTime<Utc>) -> f64 {
    as_double(value)
}
#[cfg(test)]
mod tests;
