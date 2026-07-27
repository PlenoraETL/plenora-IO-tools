//! Mattoni condivisi dai driver testuali/tabellari (geojson, csv, …):
//! inferenza tipi per colonna JSON↔Arrow, campo geometria `geoarrow.wkb`,
//! conversione array Arrow → JSON. Nessuna logica di formato qui.
#![forbid(unsafe_code)]

pub mod wkt_lossless;

use std::collections::HashMap;
use std::sync::Arc;

use arrow_array::{
    Array, ArrayRef, BooleanArray, Float32Array, Float64Array, Int16Array, Int32Array, Int64Array,
    Int8Array, StringArray, UInt16Array, UInt32Array, UInt64Array, UInt8Array,
};
use arrow_schema::{DataType, Field};
use serde_json::{Number, Value as JsonValue};

use plenora_core::geometry::{ARROW_EXTENSION_NAME_KEY, GEOARROW_WKB_EXTENSION, GEO_CRS_KEY};

/// CRS di default per i formati WGS84 per specifica (GeoJSON, KML).
pub const OGC_CRS84: &str = "OGC:CRS84";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ColType {
    Integer,
    Number,
    Boolean,
    Text,
}

/// Inferisce il tipo di una colonna dai suoi valori JSON non nulli.
pub fn infer_column<'a>(values: impl Iterator<Item = &'a JsonValue>) -> ColType {
    let mut any = false;
    let (mut all_int, mut all_num, mut all_bool) = (true, true, true);
    for v in values {
        match v {
            JsonValue::Null => continue,
            JsonValue::Number(n) => {
                any = true;
                all_bool = false;
                if !n.is_i64() && !n.is_u64() {
                    all_int = false;
                }
            }
            JsonValue::Bool(_) => {
                any = true;
                all_int = false;
                all_num = false;
            }
            _ => {
                any = true;
                all_int = false;
                all_num = false;
                all_bool = false;
            }
        }
    }
    if !any {
        ColType::Text
    } else if all_int {
        ColType::Integer
    } else if all_bool {
        ColType::Boolean
    } else if all_num {
        ColType::Number
    } else {
        ColType::Text
    }
}

/// Costruisce l'array Arrow tipizzato di una colonna proprietà.
pub fn build_property_array(col: ColType, values: &[Option<JsonValue>]) -> (DataType, ArrayRef) {
    match col {
        ColType::Integer => (
            DataType::Int64,
            Arc::new(Int64Array::from(
                values
                    .iter()
                    .map(|v| v.as_ref().and_then(JsonValue::as_i64))
                    .collect::<Vec<_>>(),
            )),
        ),
        ColType::Number => (
            DataType::Float64,
            Arc::new(Float64Array::from(
                values
                    .iter()
                    .map(|v| v.as_ref().and_then(JsonValue::as_f64))
                    .collect::<Vec<_>>(),
            )),
        ),
        ColType::Boolean => (
            DataType::Boolean,
            Arc::new(BooleanArray::from(
                values
                    .iter()
                    .map(|v| v.as_ref().and_then(JsonValue::as_bool))
                    .collect::<Vec<_>>(),
            )),
        ),
        ColType::Text => (
            DataType::Utf8,
            Arc::new(StringArray::from(
                values
                    .iter()
                    .map(|v| match v {
                        None | Some(JsonValue::Null) => None,
                        Some(JsonValue::String(s)) => Some(s.clone()),
                        Some(other) => Some(other.to_string()),
                    })
                    .collect::<Vec<_>>(),
            )),
        ),
    }
}

/// Campo Arrow per la colonna geometria (`geoarrow.wkb` + `crs`).
pub fn geometry_field(name: &str, crs_id: &str) -> Field {
    let mut md = HashMap::new();
    md.insert(
        ARROW_EXTENSION_NAME_KEY.to_owned(),
        GEOARROW_WKB_EXTENSION.to_owned(),
    );
    md.insert(GEO_CRS_KEY.to_owned(), crs_id.to_owned());
    Field::new(name, DataType::Binary, true).with_metadata(md)
}

/// Valore JSON di una cella Arrow (per la scrittura verso formati testuali).
pub fn json_from_array(array: &ArrayRef, row: usize) -> JsonValue {
    if array.is_null(row) {
        return JsonValue::Null;
    }
    let a = array.as_any();
    macro_rules! as_i64 {
        ($t:ty) => {
            if let Some(x) = a.downcast_ref::<$t>() {
                return JsonValue::from(x.value(row) as i64);
            }
        };
    }
    as_i64!(Int8Array);
    as_i64!(Int16Array);
    as_i64!(Int32Array);
    as_i64!(Int64Array);
    as_i64!(UInt8Array);
    as_i64!(UInt16Array);
    as_i64!(UInt32Array);
    if let Some(x) = a.downcast_ref::<UInt64Array>() {
        return JsonValue::from(x.value(row));
    }
    if let Some(x) = a.downcast_ref::<Float32Array>() {
        return Number::from_f64(x.value(row) as f64).map_or(JsonValue::Null, JsonValue::Number);
    }
    if let Some(x) = a.downcast_ref::<Float64Array>() {
        return Number::from_f64(x.value(row)).map_or(JsonValue::Null, JsonValue::Number);
    }
    if let Some(x) = a.downcast_ref::<BooleanArray>() {
        return JsonValue::Bool(x.value(row));
    }
    if let Some(x) = a.downcast_ref::<StringArray>() {
        return JsonValue::String(x.value(row).to_owned());
    }
    JsonValue::Null
}
