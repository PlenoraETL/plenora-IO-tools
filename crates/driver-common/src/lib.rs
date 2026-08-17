//! Mattoni condivisi dai driver testuali/tabellari (geojson, csv, …):
//! inferenza tipi per colonna JSON↔Arrow, campo geometria `geoarrow.wkb`,
//! conversione array Arrow → JSON. Nessuna logica di formato qui.
#![forbid(unsafe_code)]

pub mod prevalida_arrow;
pub mod wkt_lossless;

use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::Arc;

use arrow_array::builder::{BooleanBuilder, Float64Builder, Int64Builder, StringBuilder};
use arrow_array::{
    Array, ArrayRef, BooleanArray, Float32Array, Float64Array, Int16Array, Int32Array, Int64Array,
    Int8Array, StringArray, UInt16Array, UInt32Array, UInt64Array, UInt8Array,
};
use arrow_schema::{DataType, Field, Schema};
use serde_json::{Number, Value as JsonValue};

use plenora_io_model::geometry::{
    is_geometry_field, ARROW_EXTENSION_NAME_KEY, GEOARROW_WKB_EXTENSION, GEO_CRS_KEY,
};
use plenora_io_model::{PlenoraIoError, Result};

/// CRS di default per i formati WGS84 per specifica (`GeoJSON`, KML).
pub const OGC_CRS84: &str = "OGC:CRS84";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ColType {
    Integer,
    Number,
    Boolean,
    Text,
}

impl ColType {
    #[must_use]
    pub const fn arrow_data_type(self) -> DataType {
        match self {
            Self::Integer => DataType::Int64,
            Self::Number => DataType::Float64,
            Self::Boolean => DataType::Boolean,
            Self::Text => DataType::Utf8,
        }
    }
}

/// Classe osservata durante l'inferenza incrementale di una colonna.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ObservedValueClass {
    Null,
    Integer,
    /// Intero rappresentabile da `i64`, ma non esattamente da `f64`.
    ///
    /// Rimane `Int64` finché la colonna contiene soltanto interi; se compare
    /// anche un numero frazionario la colonna viene promossa a testo, evitando
    /// una conversione silenziosamente lossy.
    WideInteger,
    Number,
    Boolean,
    Text,
}

const MAX_EXACT_F64_INTEGER: i64 = 1_i64 << 53;

#[must_use]
pub const fn classify_i64(value: i64) -> ObservedValueClass {
    if value >= -MAX_EXACT_F64_INTEGER && value <= MAX_EXACT_F64_INTEGER {
        ObservedValueClass::Integer
    } else {
        ObservedValueClass::WideInteger
    }
}

pub fn classify_u64(value: u64) -> ObservedValueClass {
    i64::try_from(value).map_or(ObservedValueClass::Text, classify_i64)
}

/// Stato comune dell'inferenza. La promozione è monotona: una colonna non può
/// tornare a un tipo più ristretto dopo avere osservato un valore incompatibile.
#[derive(Clone, Copy, Debug)]
// I flag sono lo stato dell'automa di promozione, non opzioni di
// configurazione: raggrupparli in un enum cambierebbe la logica monotona.
#[allow(clippy::struct_excessive_bools)]
pub struct TypeAccumulator {
    any: bool,
    all_int: bool,
    all_num: bool,
    all_bool: bool,
    has_fractional_number: bool,
    has_wide_integer: bool,
}

impl Default for TypeAccumulator {
    fn default() -> Self {
        Self {
            any: false,
            all_int: true,
            all_num: true,
            all_bool: true,
            has_fractional_number: false,
            has_wide_integer: false,
        }
    }
}

impl TypeAccumulator {
    pub fn observe(&mut self, class: ObservedValueClass) {
        match class {
            ObservedValueClass::Null => {}
            ObservedValueClass::Integer | ObservedValueClass::WideInteger => {
                self.any = true;
                self.all_bool = false;
                self.has_wide_integer |= class == ObservedValueClass::WideInteger;
            }
            ObservedValueClass::Number => {
                self.any = true;
                self.all_int = false;
                self.all_bool = false;
                self.has_fractional_number = true;
            }
            ObservedValueClass::Boolean => {
                self.any = true;
                self.all_int = false;
                self.all_num = false;
            }
            ObservedValueClass::Text => {
                self.any = true;
                self.all_int = false;
                self.all_num = false;
                self.all_bool = false;
            }
        }
    }

    #[must_use]
    pub const fn column_type(self) -> ColType {
        if !self.any {
            ColType::Text
        } else if self.all_int {
            ColType::Integer
        } else if self.all_bool {
            ColType::Boolean
        } else if self.all_num && !(self.has_fractional_number && self.has_wide_integer) {
            ColType::Number
        } else {
            ColType::Text
        }
    }
}

/// Builder Arrow comune alle colonne inferite dei driver tabellari.
pub struct InferredColumnBuilder {
    inner: InferredColumnBuilderInner,
}

enum InferredColumnBuilderInner {
    Integer(Int64Builder),
    Number(Float64Builder),
    Boolean(BooleanBuilder),
    Text(StringBuilder),
}

impl InferredColumnBuilder {
    #[must_use]
    pub fn new(column_type: ColType) -> Self {
        let inner = match column_type {
            ColType::Integer => InferredColumnBuilderInner::Integer(Int64Builder::new()),
            ColType::Number => InferredColumnBuilderInner::Number(Float64Builder::new()),
            ColType::Boolean => InferredColumnBuilderInner::Boolean(BooleanBuilder::new()),
            ColType::Text => InferredColumnBuilderInner::Text(StringBuilder::new()),
        };
        Self { inner }
    }

    pub fn append_null(&mut self) {
        match &mut self.inner {
            InferredColumnBuilderInner::Integer(builder) => builder.append_null(),
            InferredColumnBuilderInner::Number(builder) => builder.append_null(),
            InferredColumnBuilderInner::Boolean(builder) => builder.append_null(),
            InferredColumnBuilderInner::Text(builder) => builder.append_null(),
        }
    }

    fn incompatible_value(column_type: ColType) -> PlenoraIoError {
        PlenoraIoError::Schema(format!(
            "valore non nullo incompatibile con il tipo inferito {column_type:?}"
        ))
    }

    const fn column_type(&self) -> ColType {
        match self.inner {
            InferredColumnBuilderInner::Integer(_) => ColType::Integer,
            InferredColumnBuilderInner::Number(_) => ColType::Number,
            InferredColumnBuilderInner::Boolean(_) => ColType::Boolean,
            InferredColumnBuilderInner::Text(_) => ColType::Text,
        }
    }

    /// Accoda un valore JSON alla colonna, rispettando il tipo inferito.
    ///
    /// # Errors
    ///
    /// Restituisce [`PlenoraIoError::Schema`] se il valore non nullo non è
    /// compatibile con il tipo inferito della colonna o se è un numero non
    /// finito.
    pub fn append_json(&mut self, value: Option<&JsonValue>) -> Result<()> {
        let column_type = self.column_type();
        match value {
            None | Some(JsonValue::Null) => self.append_null(),
            Some(JsonValue::Number(number)) => {
                if let Some(value) = number.as_i64() {
                    return self.append_i64(value);
                }
                if let Some(value) = number.as_u64() {
                    return self.append_u64(value);
                }
                if let Some(value) = number.as_f64() {
                    return self.append_f64(value);
                }
                return Err(Self::incompatible_value(column_type));
            }
            Some(JsonValue::Bool(value)) => return self.append_bool(*value),
            Some(JsonValue::String(value)) => return self.append_str(value),
            Some(other) => match &mut self.inner {
                InferredColumnBuilderInner::Text(builder) => {
                    builder.append_value(other.to_string());
                }
                _ => return Err(Self::incompatible_value(column_type)),
            },
        }
        Ok(())
    }

    /// Accoda un intero con segno alla colonna.
    ///
    /// # Errors
    ///
    /// Restituisce [`PlenoraIoError::Schema`] se la colonna è booleana, cioè
    /// incompatibile con un intero.
    pub fn append_i64(&mut self, value: i64) -> Result<()> {
        let column_type = self.column_type();
        match &mut self.inner {
            InferredColumnBuilderInner::Integer(builder) => builder.append_value(value),
            // La conversione i64 -> f64 e' la semantica voluta della colonna
            // Number; l'inferenza promuove a testo gli interi fuori da 2^53
            // (`WideInteger`), quindi qui non si perde una cifra significativa.
            // Un cast controllato cambierebbe il valore prodotto.
            #[allow(clippy::cast_precision_loss)]
            InferredColumnBuilderInner::Number(builder) => builder.append_value(value as f64),
            InferredColumnBuilderInner::Boolean(_) => {
                return Err(Self::incompatible_value(column_type));
            }
            InferredColumnBuilderInner::Text(builder) => builder.append_value(value.to_string()),
        }
        Ok(())
    }

    /// Accoda un intero senza segno alla colonna.
    ///
    /// # Errors
    ///
    /// Restituisce [`PlenoraIoError::Schema`] se il valore eccede `i64` in una
    /// colonna intera o se la colonna è booleana.
    pub fn append_u64(&mut self, value: u64) -> Result<()> {
        let column_type = self.column_type();
        match &mut self.inner {
            InferredColumnBuilderInner::Integer(builder) => builder.append_value(
                i64::try_from(value).map_err(|_| Self::incompatible_value(column_type))?,
            ),
            // Stessa motivazione di `append_i64`: la conversione u64 -> f64 e'
            // la semantica voluta della colonna Number e l'inferenza esclude
            // gli interi fuori da 2^53.
            #[allow(clippy::cast_precision_loss)]
            InferredColumnBuilderInner::Number(builder) => builder.append_value(value as f64),
            InferredColumnBuilderInner::Boolean(_) => {
                return Err(Self::incompatible_value(column_type));
            }
            InferredColumnBuilderInner::Text(builder) => builder.append_value(value.to_string()),
        }
        Ok(())
    }

    /// Accoda un numero in virgola mobile alla colonna.
    ///
    /// # Errors
    ///
    /// Restituisce [`PlenoraIoError::Schema`] se il valore non è finito oppure
    /// se la colonna è intera o booleana.
    pub fn append_f64(&mut self, value: f64) -> Result<()> {
        if !value.is_finite() {
            return Err(PlenoraIoError::Schema(
                "numero non finito non rappresentabile".to_owned(),
            ));
        }
        let column_type = self.column_type();
        match &mut self.inner {
            InferredColumnBuilderInner::Number(builder) => builder.append_value(value),
            InferredColumnBuilderInner::Integer(_) | InferredColumnBuilderInner::Boolean(_) => {
                return Err(Self::incompatible_value(column_type));
            }
            InferredColumnBuilderInner::Text(builder) => builder.append_value(value.to_string()),
        }
        Ok(())
    }

    /// Accoda un booleano alla colonna.
    ///
    /// # Errors
    ///
    /// Restituisce [`PlenoraIoError::Schema`] se la colonna è numerica.
    pub fn append_bool(&mut self, value: bool) -> Result<()> {
        let column_type = self.column_type();
        match &mut self.inner {
            InferredColumnBuilderInner::Integer(_) | InferredColumnBuilderInner::Number(_) => {
                return Err(Self::incompatible_value(column_type));
            }
            InferredColumnBuilderInner::Boolean(builder) => builder.append_value(value),
            InferredColumnBuilderInner::Text(builder) => builder.append_value(value.to_string()),
        }
        Ok(())
    }

    /// Accoda una stringa alla colonna, senza reinterpretarla.
    ///
    /// # Errors
    ///
    /// Restituisce [`PlenoraIoError::Schema`] se la colonna non è testuale.
    pub fn append_str(&mut self, value: &str) -> Result<()> {
        let column_type = self.column_type();
        match &mut self.inner {
            InferredColumnBuilderInner::Integer(_)
            | InferredColumnBuilderInner::Number(_)
            | InferredColumnBuilderInner::Boolean(_) => {
                return Err(Self::incompatible_value(column_type));
            }
            InferredColumnBuilderInner::Text(builder) => builder.append_value(value),
        }
        Ok(())
    }

    /// Accoda una cella CSV grezza, applicando il parsing del tipo inferito.
    ///
    /// La cella con soli spazi vale null; la colonna testuale conserva la
    /// cella originale senza `trim`.
    ///
    /// # Errors
    ///
    /// Restituisce [`PlenoraIoError::Schema`] se la cella non è parsabile nel
    /// tipo inferito o se il numero risultante non è finito.
    pub fn append_csv_cell(&mut self, cell: &str) -> Result<()> {
        let trimmed = cell.trim();
        if trimmed.is_empty() {
            self.append_null();
            return Ok(());
        }
        let column_type = self.column_type();
        match &mut self.inner {
            InferredColumnBuilderInner::Integer(builder) => builder.append_value(
                trimmed
                    .parse::<i64>()
                    .map_err(|_| Self::incompatible_value(column_type))?,
            ),
            InferredColumnBuilderInner::Number(builder) => {
                let value = trimmed
                    .parse::<f64>()
                    .map_err(|_| Self::incompatible_value(column_type))?;
                if !value.is_finite() {
                    return Err(PlenoraIoError::Schema(
                        "numero CSV non finito non rappresentabile".to_owned(),
                    ));
                }
                builder.append_value(value);
            }
            InferredColumnBuilderInner::Boolean(builder) => {
                let value = if trimmed.eq_ignore_ascii_case("true") {
                    true
                } else if trimmed.eq_ignore_ascii_case("false") {
                    false
                } else {
                    return Err(Self::incompatible_value(column_type));
                };
                builder.append_value(value);
            }
            InferredColumnBuilderInner::Text(builder) => builder.append_value(cell),
        }
        Ok(())
    }

    /// Accoda un valore di origine convertendolo con l'estrattore adatto al
    /// tipo inferito della colonna.
    ///
    /// # Errors
    ///
    /// Restituisce [`PlenoraIoError::Schema`] se l'estrattore corrispondente
    /// restituisce `None` o se il numero convertito non è finito.
    pub fn append_converted<'a, T, I, N, B, S>(
        &mut self,
        value: Option<&'a T>,
        integer: I,
        number: N,
        boolean: B,
        text: S,
    ) -> Result<()>
    where
        I: FnOnce(&'a T) -> Option<i64>,
        N: FnOnce(&'a T) -> Option<f64>,
        B: FnOnce(&'a T) -> Option<bool>,
        S: FnOnce(&'a T) -> Option<Cow<'a, str>>,
    {
        let Some(value) = value else {
            self.append_null();
            return Ok(());
        };
        let column_type = self.column_type();
        match &mut self.inner {
            InferredColumnBuilderInner::Integer(builder) => {
                builder.append_value(
                    integer(value).ok_or_else(|| Self::incompatible_value(column_type))?,
                );
            }
            InferredColumnBuilderInner::Number(builder) => {
                let converted =
                    number(value).ok_or_else(|| Self::incompatible_value(column_type))?;
                if !converted.is_finite() {
                    return Err(PlenoraIoError::Schema(
                        "numero non finito non rappresentabile".to_owned(),
                    ));
                }
                builder.append_value(converted);
            }
            InferredColumnBuilderInner::Boolean(builder) => {
                builder.append_value(
                    boolean(value).ok_or_else(|| Self::incompatible_value(column_type))?,
                );
            }
            InferredColumnBuilderInner::Text(builder) => {
                let converted = text(value).ok_or_else(|| Self::incompatible_value(column_type))?;
                builder.append_value(converted.as_ref());
            }
        }
        Ok(())
    }

    pub fn finish(&mut self) -> ArrayRef {
        match &mut self.inner {
            InferredColumnBuilderInner::Integer(builder) => Arc::new(builder.finish()),
            InferredColumnBuilderInner::Number(builder) => Arc::new(builder.finish()),
            InferredColumnBuilderInner::Boolean(builder) => Arc::new(builder.finish()),
            InferredColumnBuilderInner::Text(builder) => Arc::new(builder.finish()),
        }
    }
}

/// Inferisce il tipo di una colonna dai suoi valori JSON non nulli.
pub fn infer_column<'a>(values: impl Iterator<Item = &'a JsonValue>) -> ColType {
    let mut accumulator = TypeAccumulator::default();
    for v in values {
        accumulator.observe(match v {
            JsonValue::Null => ObservedValueClass::Null,
            JsonValue::Number(number) if number.is_i64() => number
                .as_i64()
                .map_or(ObservedValueClass::Text, classify_i64),
            JsonValue::Number(number) if number.is_u64() => number
                .as_u64()
                .map_or(ObservedValueClass::Text, classify_u64),
            JsonValue::Number(_) => ObservedValueClass::Number,
            JsonValue::Bool(_) => ObservedValueClass::Boolean,
            _ => ObservedValueClass::Text,
        });
    }
    accumulator.column_type()
}

/// Costruisce l'array Arrow tipizzato di una colonna proprietà.
#[must_use]
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
#[must_use]
pub fn geometry_field(name: &str, crs_id: &str) -> Field {
    let mut md = HashMap::new();
    md.insert(
        ARROW_EXTENSION_NAME_KEY.to_owned(),
        GEOARROW_WKB_EXTENSION.to_owned(),
    );
    md.insert(GEO_CRS_KEY.to_owned(), crs_id.to_owned());
    Field::new(name, DataType::Binary, true).with_metadata(md)
}

/// Indice della prima colonna dichiarata come geometria `GeoArrow` WKB.
#[must_use]
pub fn geometry_index(schema: &Schema) -> Option<usize> {
    schema
        .fields()
        .iter()
        .position(|field| is_geometry_field(field))
}

/// Valore JSON di una cella Arrow (per la scrittura verso formati testuali).
///
/// # Errors
///
/// Restituisce [`PlenoraIoError::Schema`] per un `Float32`/`Float64` non
/// finito e [`PlenoraIoError::Unsupported`] per un tipo Arrow non
/// convertibile in JSON senza perdita.
pub fn json_from_array(array: &ArrayRef, row: usize) -> Result<JsonValue> {
    if array.is_null(row) {
        return Ok(JsonValue::Null);
    }
    let a = array.as_any();
    macro_rules! as_i64 {
        ($t:ty) => {
            if let Some(x) = a.downcast_ref::<$t>() {
                return Ok(JsonValue::from(i64::from(x.value(row))));
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
        return Ok(JsonValue::from(x.value(row)));
    }
    if let Some(x) = a.downcast_ref::<Float32Array>() {
        return Number::from_f64(f64::from(x.value(row)))
            .map(JsonValue::Number)
            .ok_or_else(|| PlenoraIoError::Schema("Float32 non finito".to_owned()));
    }
    if let Some(x) = a.downcast_ref::<Float64Array>() {
        return Number::from_f64(x.value(row))
            .map(JsonValue::Number)
            .ok_or_else(|| PlenoraIoError::Schema("Float64 non finito".to_owned()));
    }
    if let Some(x) = a.downcast_ref::<BooleanArray>() {
        return Ok(JsonValue::Bool(x.value(row)));
    }
    if let Some(x) = a.downcast_ref::<StringArray>() {
        return Ok(JsonValue::String(x.value(row).to_owned()));
    }
    Err(PlenoraIoError::Unsupported(format!(
        "tipo Arrow {:?} non convertibile in JSON senza perdita",
        array.data_type()
    )))
}

/// Rappresentazione testuale lossless di una cella Arrow, con `None` per null.
///
/// # Errors
///
/// Propaga gli errori di [`json_from_array`].
pub fn cell_string(array: &ArrayRef, row: usize) -> Result<Option<String>> {
    Ok(match json_from_array(array, row)? {
        JsonValue::Null => None,
        JsonValue::String(value) => Some(value),
        other => Some(other.to_string()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn incremental_inference_is_monotonic_and_matches_batch_inference() {
        let values = [JsonValue::Null, JsonValue::from(1), JsonValue::from(2.5)];
        let mut accumulator = TypeAccumulator::default();
        accumulator.observe(ObservedValueClass::Null);
        accumulator.observe(ObservedValueClass::Integer);
        assert_eq!(accumulator.column_type(), ColType::Integer);
        accumulator.observe(ObservedValueClass::Number);

        assert_eq!(accumulator.column_type(), ColType::Number);
        assert_eq!(infer_column(values.iter()), ColType::Number);

        accumulator.observe(ObservedValueClass::Boolean);
        assert_eq!(accumulator.column_type(), ColType::Text);
        accumulator.observe(ObservedValueClass::Integer);
        assert_eq!(accumulator.column_type(), ColType::Text);
    }

    #[test]
    fn inferred_builder_preserves_csv_text_and_null_semantics() {
        let mut builder = InferredColumnBuilder::new(ColType::Text);
        builder.append_csv_cell("  value  ").unwrap();
        builder.append_csv_cell("   ").unwrap();
        let array = builder.finish();
        let strings = array.as_any().downcast_ref::<StringArray>().unwrap();

        assert_eq!(strings.value(0), "  value  ");
        assert!(strings.is_null(1));
    }

    #[test]
    fn inference_never_routes_unrepresentable_integer_through_float() {
        let out_of_i64 = [JsonValue::from(u64::MAX)];
        assert_eq!(infer_column(out_of_i64.iter()), ColType::Text);

        let mixed = [JsonValue::from(i64::MAX), JsonValue::from(0.5)];
        assert_eq!(infer_column(mixed.iter()), ColType::Text);

        let integral = [JsonValue::from(i64::MAX), JsonValue::from(i64::MIN)];
        assert_eq!(infer_column(integral.iter()), ColType::Integer);
    }

    #[test]
    fn inferred_builder_rejects_incompatible_non_null_value() {
        let mut builder = InferredColumnBuilder::new(ColType::Integer);
        assert!(matches!(
            builder.append_str("not-an-integer"),
            Err(error) if error.code == plenora_io_model::IoErrorCode::Schema
        ));
    }

    #[test]
    fn json_conversion_never_conflates_unsupported_or_non_finite_with_null() {
        let unsupported: ArrayRef = Arc::new(arrow_array::BinaryArray::from(vec![Some(
            b"payload".as_slice(),
        )]));
        let non_finite: ArrayRef = Arc::new(Float64Array::from(vec![f64::NAN]));
        let absent: ArrayRef = Arc::new(StringArray::from(vec![None::<&str>]));

        assert!(matches!(
            json_from_array(&unsupported, 0),
            Err(error) if error.code == plenora_io_model::IoErrorCode::Unsupported
        ));
        assert!(matches!(
            json_from_array(&non_finite, 0),
            Err(error) if error.code == plenora_io_model::IoErrorCode::Schema
        ));
        assert_eq!(json_from_array(&absent, 0).unwrap(), JsonValue::Null);
    }

    #[test]
    fn cell_string_renders_wide_int64_textually_exact() {
        let wide = (1_i64 << 53) + 1;
        let array: ArrayRef = Arc::new(Int64Array::from(vec![wide]));

        assert_eq!(
            cell_string(&array, 0).unwrap().as_deref(),
            Some("9007199254740993")
        );
    }

    #[test]
    fn cell_string_maps_null_to_none() {
        let array: ArrayRef = Arc::new(Int64Array::from(vec![None::<i64>]));

        assert_eq!(cell_string(&array, 0).unwrap(), None);
    }

    #[test]
    fn cell_string_passes_through_utf8_and_bool() {
        let strings: ArrayRef = Arc::new(StringArray::from(vec!["testo"]));
        let bools: ArrayRef = Arc::new(BooleanArray::from(vec![true]));

        assert_eq!(cell_string(&strings, 0).unwrap().as_deref(), Some("testo"));
        assert_eq!(cell_string(&bools, 0).unwrap().as_deref(), Some("true"));
    }

    #[test]
    fn cell_string_rejects_non_finite_float64() {
        let nan: ArrayRef = Arc::new(Float64Array::from(vec![f64::NAN]));
        let infinite: ArrayRef = Arc::new(Float64Array::from(vec![f64::INFINITY]));

        assert!(cell_string(&nan, 0).is_err());
        assert!(cell_string(&infinite, 0).is_err());
    }

    #[test]
    fn geometry_index_returns_first_of_two_geoarrow_wkb_fields() {
        let schema = Schema::new(vec![
            Field::new("name", DataType::Utf8, true),
            geometry_field("geom_a", OGC_CRS84),
            geometry_field("geom_b", OGC_CRS84),
        ]);

        assert_eq!(geometry_index(&schema), Some(1));
    }

    #[test]
    fn geometry_index_is_none_without_geometry_metadata() {
        let schema = Schema::new(vec![
            Field::new("a", DataType::Int64, true),
            Field::new("b", DataType::Binary, true),
        ]);

        assert_eq!(geometry_index(&schema), None);
    }

    #[test]
    fn geometry_index_ignores_bare_geometry_name_without_extension() {
        let schema = Schema::new(vec![Field::new("geometry", DataType::Binary, true)]);

        assert_eq!(geometry_index(&schema), None);
    }
}
