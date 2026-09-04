//! Mattoni condivisi dai driver testuali/tabellari (geojson, csv, …):
//! inferenza tipi per colonna JSON↔Arrow, campo geometria `geoarrow.wkb`,
//! conversione array Arrow → JSON. Nessuna logica di formato qui.
#![forbid(unsafe_code)]

pub mod prevalida_arrow;
pub mod wkt_lossless;
// L'analisi progressiva del WKT: usata da `wkt_lossless`, non esposta.
// Il confine pubblico resta `parse_wkt_bounded`, che aggiunge il tetto in byte
// e la verifica di esprimibilita'.
mod wkt_progressivo;

use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::Arc;

use arrow_array::builder::{BooleanBuilder, Float64Builder, Int64Builder, StringBuilder};
use arrow_array::{
    Array, ArrayRef, BooleanArray, Date32Array, Date64Array, Float32Array, Float64Array,
    Int16Array, Int32Array, Int64Array, Int8Array, StringArray, Time32MillisecondArray,
    Time32SecondArray, Time64MicrosecondArray, Time64NanosecondArray, TimestampMicrosecondArray,
    TimestampMillisecondArray, TimestampNanosecondArray, TimestampSecondArray, UInt16Array,
    UInt32Array, UInt64Array, UInt8Array,
};
use arrow_schema::{DataType, Field, Schema, TimeUnit};
use serde_json::{Number, Value as JsonValue};

use plenora_io_model::geometry::{
    is_geometry_field, ARROW_EXTENSION_NAME_KEY, GEOARROW_WKB_EXTENSION, GEO_CRS_KEY,
};
use plenora_io_model::PublicMessage;
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
    /// Nome statico del tipo inferito, per i messaggi pubblici.
    ///
    /// Prende il posto di un `{:?}`: `Debug` non e' un formato che qualcuno
    /// abbia promesso di tenere stabile, e stamparlo in un errore pubblico
    /// impegna a non rinominare mai la variante senza averlo mai scritto.
    #[must_use]
    pub const fn nome(self) -> &'static str {
        match self {
            Self::Integer => "integer",
            Self::Number => "number",
            Self::Boolean => "boolean",
            Self::Text => "text",
        }
    }

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
        PlenoraIoError::schema_redatto(&PublicMessage::CuratedPair(
            "valore non nullo incompatibile con il tipo inferito:",
            column_type.nome(),
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
            return Err(PlenoraIoError::schema_redatto(&PublicMessage::Curated(
                "numero non finito non rappresentabile",
            )));
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
                    return Err(PlenoraIoError::schema_redatto(&PublicMessage::Curated(
                        "numero CSV non finito non rappresentabile",
                    )));
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
                    return Err(PlenoraIoError::schema_redatto(&PublicMessage::Curated(
                        "numero non finito non rappresentabile",
                    )));
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

// --- i temporali, resi come testo -------------------------------------------
//
// # Perche' esistono queste righe
//
// `SCALAR_TYPES` ammette `ArrowTypeClass::Temporal`, e i formati testuali che
// lo dichiarano promettono `TypeCoercionPolicy::ExplicitText`: una
// serializzazione testuale deterministica. Il renderer pero' non sapeva
// scrivere un temporale e rifiutava, quindi la capability prometteva una cosa
// e il codice ne faceva un'altra -- e a scoprirlo era chi convertiva, a meta'
// di una conversione dichiarata possibile.
//
// # Che cosa la rappresentazione promette
//
// Di essere **deterministica** e di non dipendere dalla macchina: nessun fuso
// locale, nessuna localizzazione, nessuna cifra che compare o sparisce col
// valore. La frazione di secondo si scrive sempre con le cifre dell'unita'
// dichiarata dal tipo -- tre per i millisecondi, sei per i microsecondi, nove
// per i nanosecondi, nessuna per i secondi -- cosi' due valori dello stesso
// tipo hanno sempre la stessa forma.
//
// | tipo Arrow | testo |
// |---|---|
// | `Date32`, `Date64` | `AAAA-MM-GG` |
// | `Time32`, `Time64` | `HH:MM:SS[.fff...]` |
// | `Timestamp` senza fuso | `AAAA-MM-GGTHH:MM:SS[.fff...]` |
// | `Timestamp` con fuso | lo stesso, piu' `Z` |
//
// Il `Z` non e' decorativo e non e' una conversione: Arrow conserva un
// timestamp con fuso come istante **UTC**, e il fuso e' metadato di
// presentazione. Scriverlo senza `Z` direbbe «ora locale di chissa' dove»;
// riproiettarlo nel fuso dichiarato sarebbe una trasformazione, e questo
// prodotto non trasforma.
//
// # Che cosa resta fuori, e non per dimenticanza
//
// `Duration` e `Interval` stanno nella stessa classe `Temporal` e **non**
// vengono scritti. Non sono istanti: sono quantita', e la loro forma testuale
// -- una durata ISO 8601, oppure la tripla mesi/giorni/nanosecondi di
// `Interval` -- e' una scelta di rappresentazione a se', che nessuna
// conversione del catalogo chiede oggi. Restano il rifiuto che erano, e il
// rifiuto nomina la classe.

/// La scala dell'unita': quante frazioni entrano in un secondo, e con quante
/// cifre si scrivono.
const fn scala(unita: TimeUnit) -> (i64, usize) {
    match unita {
        TimeUnit::Second => (1, 0),
        TimeUnit::Millisecond => (1_000, 3),
        TimeUnit::Microsecond => (1_000_000, 6),
        TimeUnit::Nanosecond => (1_000_000_000, 9),
    }
}

/// Anno, mese e giorno dai giorni dall'epoca Unix.
///
/// E' `civil_from_days` di Howard Hinnant, calendario proletticamente
/// gregoriano. Scritto qui invece di prendere una libreria di date: sarebbe una
/// dipendenza in piu' nel perimetro spedito, e in `Cargo.lock`, per quindici
/// righe di aritmetica che non hanno versioni.
const fn data_civile(dall_epoca: i64) -> (i64, i64, i64) {
    let spostati = dall_epoca + 719_468;
    let era = if spostati >= 0 {
        spostati
    } else {
        spostati - 146_096
    } / 146_097;
    let dall_era = spostati - era * 146_097;
    let anni_interi = (dall_era - dall_era / 1460 + dall_era / 36_524 - dall_era / 146_096) / 365;
    let anno = anni_interi + era * 400;
    let dall_anno = dall_era - (365 * anni_interi + anni_interi / 4 - anni_interi / 100);
    let indice = (5 * dall_anno + 2) / 153;
    let giorno = dall_anno - (153 * indice + 2) / 5 + 1;
    let mese = if indice < 10 { indice + 3 } else { indice - 9 };
    (if mese <= 2 { anno + 1 } else { anno }, mese, giorno)
}

fn testo_data(dall_epoca: i64) -> String {
    let (anno, mese, giorno) = data_civile(dall_epoca);
    format!("{anno:04}-{mese:02}-{giorno:02}")
}

/// L'orario, dalle frazioni trascorse dalla mezzanotte.
fn testo_orario(frazioni: i64, per_secondo: i64, cifre: usize) -> String {
    let interi = frazioni.div_euclid(per_secondo);
    let resto = frazioni.rem_euclid(per_secondo);
    let (ore, minuti, secondi) = (interi / 3600, (interi / 60) % 60, interi % 60);
    let base = format!("{ore:02}:{minuti:02}:{secondi:02}");
    if cifre == 0 {
        base
    } else {
        format!("{base}.{resto:0cifre$}")
    }
}

/// Un istante, dalle frazioni dall'epoca.
fn testo_istante(valore: i64, per_secondo: i64, cifre: usize, utc: bool) -> String {
    let per_giorno = per_secondo * 86_400;
    let giorni = valore.div_euclid(per_giorno);
    let nel_giorno = valore.rem_euclid(per_giorno);
    let zulu = if utc { "Z" } else { "" };
    format!(
        "{}T{}{zulu}",
        testo_data(giorni),
        testo_orario(nel_giorno, per_secondo, cifre)
    )
}

/// Il testo di una cella temporale, o `None` se il tipo non e' un istante.
fn testo_temporale(array: &ArrayRef, row: usize) -> Option<String> {
    let a = array.as_any();
    match array.data_type() {
        DataType::Date32 => a
            .downcast_ref::<Date32Array>()
            .map(|x| testo_data(i64::from(x.value(row)))),
        // `Date64` sono millisecondi, e la specifica Arrow li vuole multipli
        // di un giorno: si scrive la data, non l'istante.
        DataType::Date64 => a
            .downcast_ref::<Date64Array>()
            .map(|x| testo_data(x.value(row).div_euclid(86_400_000))),
        DataType::Time32(unita) => {
            let (per_secondo, cifre) = scala(*unita);
            match unita {
                TimeUnit::Second => a
                    .downcast_ref::<Time32SecondArray>()
                    .map(|x| testo_orario(i64::from(x.value(row)), per_secondo, cifre)),
                TimeUnit::Millisecond => a
                    .downcast_ref::<Time32MillisecondArray>()
                    .map(|x| testo_orario(i64::from(x.value(row)), per_secondo, cifre)),
                // `Time32` porta solo secondi e millisecondi: le altre due
                // unita' non sono costruibili con questo tipo.
                TimeUnit::Microsecond | TimeUnit::Nanosecond => None,
            }
        }
        DataType::Time64(unita) => {
            let (per_secondo, cifre) = scala(*unita);
            match unita {
                TimeUnit::Microsecond => a
                    .downcast_ref::<Time64MicrosecondArray>()
                    .map(|x| testo_orario(x.value(row), per_secondo, cifre)),
                TimeUnit::Nanosecond => a
                    .downcast_ref::<Time64NanosecondArray>()
                    .map(|x| testo_orario(x.value(row), per_secondo, cifre)),
                TimeUnit::Second | TimeUnit::Millisecond => None,
            }
        }
        DataType::Timestamp(unita, fuso) => {
            let (per_secondo, cifre) = scala(*unita);
            let utc = fuso.is_some();
            match unita {
                TimeUnit::Second => a
                    .downcast_ref::<TimestampSecondArray>()
                    .map(|x| testo_istante(x.value(row), per_secondo, cifre, utc)),
                TimeUnit::Millisecond => a
                    .downcast_ref::<TimestampMillisecondArray>()
                    .map(|x| testo_istante(x.value(row), per_secondo, cifre, utc)),
                TimeUnit::Microsecond => a
                    .downcast_ref::<TimestampMicrosecondArray>()
                    .map(|x| testo_istante(x.value(row), per_secondo, cifre, utc)),
                TimeUnit::Nanosecond => a
                    .downcast_ref::<TimestampNanosecondArray>()
                    .map(|x| testo_istante(x.value(row), per_secondo, cifre, utc)),
            }
        }
        _ => None,
    }
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
            .ok_or_else(|| {
                PlenoraIoError::schema_redatto(&PublicMessage::Curated("Float32 non finito"))
            });
    }
    if let Some(x) = a.downcast_ref::<Float64Array>() {
        return Number::from_f64(x.value(row))
            .map(JsonValue::Number)
            .ok_or_else(|| {
                PlenoraIoError::schema_redatto(&PublicMessage::Curated("Float64 non finito"))
            });
    }
    if let Some(x) = a.downcast_ref::<BooleanArray>() {
        return Ok(JsonValue::Bool(x.value(row)));
    }
    if let Some(x) = a.downcast_ref::<StringArray>() {
        return Ok(JsonValue::String(x.value(row).to_owned()));
    }
    if let Some(testo) = testo_temporale(array, row) {
        return Ok(JsonValue::String(testo));
    }
    // Il `Debug` di `DataType` e' testo di una dipendenza: puo' contenere
    // nomi di campo e metadati letti dal file, e non e' un formato che arrow
    // abbia promesso di tenere stabile. Al suo posto la classe, che e' un
    // vocabolario nostro.
    Err(PlenoraIoError::non_supportato_redatto(
        &PublicMessage::CuratedPair(
            "tipo Arrow non convertibile in JSON senza perdita, classe:",
            classe_arrow(array.data_type()),
        ),
    ))
}

/// Conteggio saturante da `usize` a `u64`, per i numeri strutturali.
///
/// Non e' un fallback: su ogni piattaforma che ci interessa `usize` sta in
/// `u64`, e il ramo saturante esiste solo perche' il compilatore non lo sa. La
/// forma esplicita evita che ogni driver migrato aggiunga il proprio
/// `unwrap_or(u64::MAX)` al registro dei fallback, dove somiglierebbe a una
/// decisione presa in mancanza di meglio invece che a una conversione totale.
#[must_use]
pub const fn saturating_u64(value: usize) -> u64 {
    if value as u128 > u64::MAX as u128 {
        u64::MAX
    } else {
        value as u64
    }
}

/// Classe statica di un tipo Arrow, per i messaggi pubblici.
///
/// `driver-common` non dipende da `plenora-io-core` e non vede
/// `ArrowTypeClass`: questo e' lo stesso vocabolario, dichiarato dove serve.
/// Non e' una tassonomia completa dei tipi Arrow — e' l'insieme delle classi
/// che questo bordo distingue, e `altro` copre per costruzione tutto il resto.
#[must_use]
pub const fn classe_arrow(tipo: &DataType) -> &'static str {
    match tipo {
        DataType::Boolean => "boolean",
        DataType::Int8 | DataType::Int16 | DataType::Int32 | DataType::Int64 => "signed_integer",
        DataType::UInt8 | DataType::UInt16 | DataType::UInt32 | DataType::UInt64 => {
            "unsigned_integer"
        }
        DataType::Float16 | DataType::Float32 | DataType::Float64 => "floating",
        DataType::Utf8 | DataType::LargeUtf8 => "utf8",
        DataType::Binary | DataType::LargeBinary | DataType::FixedSizeBinary(_) => "binary",
        DataType::Date32
        | DataType::Date64
        | DataType::Time32(_)
        | DataType::Time64(_)
        | DataType::Timestamp(_, _)
        | DataType::Duration(_)
        | DataType::Interval(_) => "temporal",
        DataType::Decimal128(_, _) | DataType::Decimal256(_, _) => "decimal",
        DataType::List(_)
        | DataType::LargeList(_)
        | DataType::FixedSizeList(_, _)
        | DataType::Struct(_)
        | DataType::Union(_, _)
        | DataType::Map(_, _) => "nested",
        _ => "altro",
    }
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

    /// I vocabolari statici sono distinti, non vuoti e coprono le famiglie.
    ///
    /// `classe_arrow` e `ColType::nome` sono `match` esaustivi: il compilatore
    /// garantisce che ogni variante abbia un nome, non che i nomi siano
    /// distinti ne' che la mappa sia quella intesa. La misura di copertura
    /// differenziale del checkpoint su `effc4ab` li ha trovati **mai
    /// esercitati**, ed e' la ragione per cui questo test esiste: un
    /// vocabolario che nessuno attraversa e' una tabella di traduzione di cui
    /// nessuno ha mai letto una riga.
    #[test]
    fn i_vocabolari_statici_sono_distinti_e_coprono_le_famiglie() {
        use arrow_schema::{DataType, Field, TimeUnit};
        use std::sync::Arc;

        let campioni: Vec<(DataType, &str)> = vec![
            (DataType::Boolean, "boolean"),
            (DataType::Int8, "signed_integer"),
            (DataType::Int64, "signed_integer"),
            (DataType::UInt8, "unsigned_integer"),
            (DataType::UInt64, "unsigned_integer"),
            (DataType::Float16, "floating"),
            (DataType::Float64, "floating"),
            (DataType::Utf8, "utf8"),
            (DataType::LargeUtf8, "utf8"),
            (DataType::Binary, "binary"),
            (DataType::FixedSizeBinary(4), "binary"),
            (DataType::Date32, "temporal"),
            (DataType::Time64(TimeUnit::Nanosecond), "temporal"),
            (DataType::Timestamp(TimeUnit::Second, None), "temporal"),
            (DataType::Duration(TimeUnit::Second), "temporal"),
            (DataType::Decimal128(10, 2), "decimal"),
            (DataType::Decimal256(40, 2), "decimal"),
            (
                DataType::List(Arc::new(Field::new("item", DataType::Int32, true))),
                "nested",
            ),
            (
                DataType::Struct(vec![Field::new("a", DataType::Int32, true)].into()),
                "nested",
            ),
            // Il ramo di riserva esiste e va raggiunto: se sparisse, un tipo
            // nuovo di arrow non avrebbe piu' nome e il match non
            // compilerebbe — ma finche' c'e', va provato che risponde.
            (DataType::Null, "altro"),
        ];
        for (tipo, atteso) in campioni {
            assert_eq!(classe_arrow(&tipo), atteso, "classe di {tipo:?}");
        }

        let colonne = [
            (ColType::Integer, "integer"),
            (ColType::Number, "number"),
            (ColType::Boolean, "boolean"),
            (ColType::Text, "text"),
        ];
        let mut visti = std::collections::BTreeSet::new();
        for (tipo, atteso) in colonne {
            assert_eq!(tipo.nome(), atteso);
            assert!(visti.insert(tipo.nome()), "nome duplicato: {}", tipo.nome());
        }
        assert_eq!(visti.len(), colonne.len());
    }

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

    // --- la rappresentazione testuale dei temporali ------------------------

    fn testo<A: Array + 'static>(colonna: A) -> String {
        let array: ArrayRef = Arc::new(colonna);
        match json_from_array(&array, 0).expect("la cella temporale si scrive") {
            JsonValue::String(testo) => testo,
            altro => panic!("atteso testo, arrivato {altro:?}"),
        }
    }

    /// Le date, compresa una **prima** dell'epoca.
    ///
    /// Il giorno negativo non e' un caso di scuola: `div_euclid` e il
    /// calendario di Hinnant esistono per lui, e un'implementazione che
    /// dividesse per difetto sbagliata darebbe una data plausibile e falsa --
    /// il modo peggiore di sbagliare, perche' nessuno la guarda due volte.
    #[test]
    fn le_date_si_scrivono_in_iso_anche_prima_dell_epoca() {
        assert_eq!(testo(Date32Array::from(vec![0])), "1970-01-01");
        assert_eq!(testo(Date32Array::from(vec![20_468])), "2026-01-15");
        assert_eq!(testo(Date32Array::from(vec![-1])), "1969-12-31");
        assert_eq!(testo(Date32Array::from(vec![-719_162])), "0001-01-01");
        // Un anno bisestile secolare: il 2000 lo e', il 1900 no, ed e' li' che
        // le implementazioni approssimate si dividono.
        assert_eq!(testo(Date32Array::from(vec![11_016])), "2000-02-29");
        // `Date64` sono millisecondi, e si scrive la data.
        assert_eq!(
            testo(Date64Array::from(vec![1_768_435_200_000])),
            "2026-01-15"
        );
    }

    /// Le cifre della frazione vengono dall'**unita' dichiarata**, non dal
    /// valore.
    ///
    /// E' la parte che rende la rappresentazione deterministica: se le cifre
    /// dipendessero dal valore, `12:00:00` e `12:00:00.500` sarebbero due forme
    /// della stessa colonna, e due file identici nel contenuto avrebbero
    /// larghezze diverse.
    #[test]
    fn le_cifre_della_frazione_vengono_dall_unita_e_non_dal_valore() {
        assert_eq!(testo(Time32SecondArray::from(vec![45_296])), "12:34:56");
        assert_eq!(
            testo(Time32MillisecondArray::from(vec![45_296_000])),
            "12:34:56.000"
        );
        assert_eq!(
            testo(Time32MillisecondArray::from(vec![45_296_500])),
            "12:34:56.500"
        );
        assert_eq!(
            testo(Time64MicrosecondArray::from(vec![45_296_000_007])),
            "12:34:56.000007"
        );
        assert_eq!(
            testo(Time64NanosecondArray::from(vec![45_296_000_000_009])),
            "12:34:56.000000009"
        );
    }

    /// Il fuso non riproietta: aggiunge la `Z`.
    ///
    /// Arrow conserva un timestamp con fuso come istante UTC, e il fuso e'
    /// metadato di presentazione. Riproiettarlo sarebbe una trasformazione, e
    /// questo prodotto non trasforma; ometterlo direbbe «ora locale di chissa'
    /// dove». Le due colonne portano lo **stesso** istante, e si distinguono
    /// per la sola `Z`.
    #[test]
    fn il_timestamp_con_fuso_si_scrive_in_utc_e_lo_dichiara() {
        let senza = TimestampSecondArray::from(vec![1_768_480_496]);
        let con: TimestampSecondArray =
            TimestampSecondArray::from(vec![1_768_480_496]).with_timezone("Europe/Rome");
        assert_eq!(testo(senza), "2026-01-15T12:34:56");
        assert_eq!(testo(con), "2026-01-15T12:34:56Z");
    }

    /// `Duration` e `Interval` restano un rifiuto, e il rifiuto nomina la
    /// classe.
    ///
    /// Sono nella stessa classe `Temporal` degli istanti e **non** sono
    /// istanti: la loro forma testuale e' una scelta di rappresentazione a se'.
    /// La sonda esiste perche' il confine sia dichiarato invece che scoperto:
    /// il giorno in cui una conversione la chiede, e' questa prova a doversi
    /// muovere.
    #[test]
    fn le_durate_non_sono_istanti_e_restano_un_rifiuto() {
        let durata: ArrayRef = Arc::new(arrow_array::DurationSecondArray::from(vec![90]));
        let Err(errore) = json_from_array(&durata, 0) else {
            panic!("una durata non ha una forma testuale decisa: deve essere rifiutata");
        };
        assert!(
            errore.message.contains("temporal"),
            "il rifiuto deve nominare la classe, arrivato «{}»",
            errore.message
        );
    }
}
