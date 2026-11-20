//! Exact scalar observations, independent of conversion reports and casts.
use arrow_array::{
    Array, ArrayRef, BooleanArray, Float32Array, Float64Array, Int16Array, Int32Array, Int64Array,
    Int8Array, StringArray, UInt16Array, UInt32Array, UInt64Array, UInt8Array,
};

#[derive(Debug, PartialEq, Eq)]
enum Scalar {
    Null,
    Boolean(bool),
    Text(String),
    Number {
        negative: bool,
        mantissa: u128,
        exponent: i32,
    },
    NonFinite(u64),
}

const fn number(negative: bool, mut mantissa: u128, mut exponent: i32) -> Scalar {
    if mantissa != 0 {
        while mantissa & 1 == 0 {
            mantissa >>= 1;
            exponent += 1;
        }
    } else {
        exponent = 0;
    }
    Scalar::Number {
        negative,
        mantissa,
        exponent,
    }
}

fn floating(value: f64) -> Scalar {
    let bits = value.to_bits();
    if !value.is_finite() {
        return Scalar::NonFinite(bits);
    }
    let power = i32::try_from((bits >> 52) & 0x7ff).unwrap();
    let fraction = bits & ((1_u64 << 52) - 1);
    let mantissa = if power == 0 {
        fraction
    } else {
        fraction | (1_u64 << 52)
    };
    number(
        bits >> 63 != 0,
        u128::from(mantissa),
        power.max(1) - 1023 - 52,
    )
}

fn scalar(array: &ArrayRef, row: usize) -> Result<Scalar, &'static str> {
    if array.is_null(row) {
        return Ok(Scalar::Null);
    }
    macro_rules! signed {
        ($($array:ty),+) => { $(
            if let Some(a) = array.as_any().downcast_ref::<$array>() {
                let v = i128::from(a.value(row));
                return Ok(number(v < 0, v.unsigned_abs(), 0));
            }
        )+ };
    }
    macro_rules! unsigned {
        ($($array:ty),+) => { $(
            if let Some(a) = array.as_any().downcast_ref::<$array>() {
                return Ok(number(false, u128::from(a.value(row)), 0));
            }
        )+ };
    }
    signed!(Int8Array, Int16Array, Int32Array, Int64Array);
    unsigned!(UInt8Array, UInt16Array, UInt32Array, UInt64Array);
    if let Some(a) = array.as_any().downcast_ref::<Float32Array>() {
        return Ok(floating(f64::from(a.value(row))));
    }
    if let Some(a) = array.as_any().downcast_ref::<Float64Array>() {
        return Ok(floating(a.value(row)));
    }
    if let Some(a) = array.as_any().downcast_ref::<BooleanArray>() {
        return Ok(Scalar::Boolean(a.value(row)));
    }
    if let Some(a) = array.as_any().downcast_ref::<arrow_array::Date32Array>() {
        let date = arrow_array::temporal_conversions::date32_to_datetime(a.value(row))
            .ok_or("date outside representable calendar")?;
        return Ok(Scalar::Text(date.format("%Y-%m-%d").to_string()));
    }
    if let Some(a) = array.as_any().downcast_ref::<StringArray>() {
        return Ok(Scalar::Text(a.value(row).to_owned()));
    }
    Err("cross-type scalar semantics not implemented")
}

fn text_scalar(text: &str) -> Option<Scalar> {
    if let Ok(v) = text.parse::<i128>() {
        return Some(number(v < 0, v.unsigned_abs(), 0));
    }
    if let Ok(v) = text.parse::<bool>() {
        return Some(Scalar::Boolean(v));
    }
    text.parse::<f64>().ok().map(floating)
}

pub(super) fn equal(before: &ArrayRef, after: &ArrayRef) -> Result<bool, &'static str> {
    if before.len() != after.len() {
        return Ok(false);
    }
    // Arrow equality covers nested offsets, validity and payload for equal
    // nonnumeric types; numeric values below retain the sign of zero.
    if before.data_type() == after.data_type()
        && !matches!(
            before.data_type(),
            arrow_schema::DataType::Float32 | arrow_schema::DataType::Float64
        )
    {
        return Ok(before.eq(after));
    }
    for row in 0..before.len() {
        let mut a = scalar(before, row)?;
        let mut b = scalar(after, row)?;
        if let Scalar::Text(text) = &a {
            if !matches!(b, Scalar::Text(_)) {
                a = text_scalar(text).unwrap_or(a);
            }
        }
        if let Scalar::Text(text) = &b {
            if !matches!(a, Scalar::Text(_)) {
                b = text_scalar(text).unwrap_or(b);
            }
        }
        if let Scalar::Boolean(value) = a {
            if matches!(b, Scalar::Number { .. }) {
                a = number(false, u128::from(value), 0);
            }
        }
        if let Scalar::Boolean(value) = b {
            if matches!(a, Scalar::Number { .. }) {
                b = number(false, u128::from(value), 0);
            }
        }
        if a != b {
            return Ok(false);
        }
    }
    Ok(true)
}

#[test]
fn numeric_coercion_does_not_hide_rounding_or_signed_zero() {
    use std::sync::Arc;
    let integer: ArrayRef = Arc::new(Int64Array::from(vec![9_007_199_254_740_993]));
    let rounded: ArrayRef = Arc::new(Float64Array::from(vec![9_007_199_254_740_992.0]));
    assert_eq!(equal(&integer, &rounded), Ok(false));
    let exact: ArrayRef = Arc::new(Int64Array::from(vec![9_007_199_254_740_992]));
    assert_eq!(equal(&exact, &rounded), Ok(true));
    let negative: ArrayRef = Arc::new(Float64Array::from(vec![-0.0]));
    let positive: ArrayRef = Arc::new(Float64Array::from(vec![0.0]));
    assert_eq!(equal(&negative, &positive), Ok(false));
}
