// Licensed to the Apache Software Foundation (ASF) under one
// or more contributor license agreements.  See the NOTICE file
// distributed with this work for additional information
// regarding copyright ownership.  The ASF licenses this file
// to you under the Apache License, Version 2.0 (the
// "License"); you may not use this file except in compliance
// with the License.  You may obtain a copy of the License at
//
//   http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing,
// software distributed under the License is distributed on an
// "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY
// KIND, either express or implied.  See the License for the
// specific language governing permissions and limitations
// under the License.

//! Checked record rendering. Display/JSON use an explicit diagnostic string
//! for invalid fields: never panic, null, epoch or an unsigned reinterpretation.
use chrono::{DateTime, Utc};
const INVALID: &str = "<invalid temporal value>";
pub(crate) fn convert_date_to_string(value: i32) -> String {
    DateTime::from_timestamp(i64::from(value) * 86_400, 0)
        .map(|d| d.format("%Y-%m-%d").to_string())
        .unwrap_or_else(|| INVALID.into())
}
pub(crate) fn convert_timestamp_millis_to_string(value: i64) -> String {
    DateTime::<Utc>::from_timestamp_millis(value)
        .map(|d| d.format("%Y-%m-%d %H:%M:%S%.3f %:z").to_string())
        .unwrap_or_else(|| INVALID.into())
}
pub(crate) fn convert_timestamp_micros_to_string(value: i64) -> String {
    DateTime::<Utc>::from_timestamp_micros(value)
        .map(|d| d.format("%Y-%m-%d %H:%M:%S%.6f %:z").to_string())
        .unwrap_or_else(|| INVALID.into())
}
fn time(value: i64, scale: i64, digits: usize) -> String {
    if !(0..86_400 * scale).contains(&value) {
        return INVALID.into();
    }
    let seconds = value / scale;
    format!(
        "{:02}:{:02}:{:02}.{:0digits$}",
        seconds / 3600,
        seconds / 60 % 60,
        seconds % 60,
        value % scale
    )
}
pub(crate) fn convert_time_millis_to_string(value: i32) -> String {
    time(i64::from(value), 1000, 3)
}
pub(crate) fn convert_time_micros_to_string(value: i64) -> String {
    time(value, 1_000_000, 6)
}
#[cfg(test)]
mod tests;
