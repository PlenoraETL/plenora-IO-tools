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

use super::*;
use crate::record::Field;
#[test]
fn temporal_record_boundaries() {
    for v in [i32::MIN, i32::MAX] {
        assert_eq!(Field::Date(v).to_string(), INVALID);
        assert_eq!(Field::Date(v).to_json_value(), serde_json::json!(INVALID));
    }
    for v in [i64::MIN, i64::MAX] {
        assert_eq!(Field::TimestampMillis(v).to_string(), INVALID);
        assert_eq!(
            Field::TimestampMillis(v).to_json_value(),
            serde_json::json!(INVALID)
        );
        assert_eq!(Field::TimestampMicros(v).to_string(), INVALID);
        assert_eq!(
            Field::TimestampMicros(v).to_json_value(),
            serde_json::json!(INVALID)
        );
    }
    for v in [i32::MIN, -1, 86_400_000, i32::MAX] {
        assert_eq!(Field::TimeMillis(v).to_string(), INVALID);
        assert_eq!(
            Field::TimeMillis(v).to_json_value(),
            serde_json::json!(INVALID)
        );
    }
    for v in [i64::MIN, -1, 86_400_000_000, i64::MAX] {
        assert_eq!(Field::TimeMicros(v).to_string(), INVALID);
        assert_eq!(
            Field::TimeMicros(v).to_json_value(),
            serde_json::json!(INVALID)
        );
    }
    assert_eq!(convert_time_millis_to_string(86_399_999), "23:59:59.999");
    assert_eq!(
        convert_time_micros_to_string(86_399_999_999),
        "23:59:59.999999"
    );
    assert_eq!(convert_date_to_string(-719162), "0001-01-01");
    assert_eq!(convert_date_to_string(2932896), "9999-12-31");
}

#[test]
fn temporal_record_calendar_edges_and_zero() {
    assert_eq!(convert_time_millis_to_string(0), "00:00:00.000");
    assert_eq!(convert_time_micros_to_string(0), "00:00:00.000000");
    let min = chrono::DateTime::<Utc>::MIN_UTC.timestamp_millis();
    let max = chrono::DateTime::<Utc>::MAX_UTC.timestamp_millis();
    assert_ne!(convert_timestamp_millis_to_string(min), INVALID);
    assert_ne!(convert_timestamp_millis_to_string(max), INVALID);
    assert_eq!(convert_timestamp_millis_to_string(min - 1), INVALID);
    assert_eq!(convert_timestamp_millis_to_string(max + 1), INVALID);
    let min = chrono::DateTime::<Utc>::MIN_UTC.timestamp_micros();
    let max = chrono::DateTime::<Utc>::MAX_UTC.timestamp_micros();
    assert_ne!(convert_timestamp_micros_to_string(min), INVALID);
    assert_ne!(convert_timestamp_micros_to_string(max), INVALID);
    assert_eq!(convert_timestamp_micros_to_string(min - 1), INVALID);
    assert_eq!(convert_timestamp_micros_to_string(max + 1), INVALID);
}
