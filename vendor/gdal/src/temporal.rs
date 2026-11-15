//! OGR integer-second fields accept years 1..9999 and quarter-hour offsets.
use chrono::{DateTime, Datelike, FixedOffset, Timelike};
#[derive(Debug)]
pub(crate) struct InvalidDateTime;
pub(crate) fn datetime(value: DateTime<FixedOffset>) -> Result<[i32; 7], InvalidDateTime> {
    let invalid = || InvalidDateTime;
    let offset = value.offset().local_minus_utc();
    if offset % 900 != 0 || value.timestamp_subsec_nanos() != 0 {
        return Err(invalid());
    }
    let local = value
        .naive_utc()
        .checked_add_offset(*value.offset())
        .ok_or_else(invalid)?;
    if !(1..=9999).contains(&local.year()) {
        return Err(invalid());
    }
    Ok([
        local.year(),
        i32::try_from(local.month()).map_err(|_| InvalidDateTime)?,
        i32::try_from(local.day()).map_err(|_| InvalidDateTime)?,
        i32::try_from(local.hour()).map_err(|_| InvalidDateTime)?,
        i32::try_from(local.minute()).map_err(|_| InvalidDateTime)?,
        i32::try_from(local.second()).map_err(|_| InvalidDateTime)?,
        100 + offset / 900,
    ])
}
#[cfg(test)]
mod tests;
