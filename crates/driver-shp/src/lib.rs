//! driver-shp — Shapefile ⇄ `RecordBatch`. Le shape XY/M/Z diventano WKB
//! `geoarrow.wkb` XY/XYM/XYZ/XYZM senza passare da `geo-types`; il dbf fornisce
//! gli attributi e il `.prj` (o `assume_crs`) il CRS.
//!
//! Scrittura (Fase 2B): capability-check fail-closed (`ENGINEERING.md § Pipeline di scrittura (capability-check`)) — nomi campo dbf
//! ≤10 char (imposto da `FieldName`), tipo geometria unico per file (imposto da
//! shapefile). Il publish **multi-file** espone entrambe le modalità di `ENGINEERING.md § Pipeline di scrittura:`
//! `*.shp.d` è uno `ShapefileDirectoryDataset` pubblicato con un unico rename
//! atomico; `*.shp` è un `LooseShapefileSet` compatibile, pubblicato con rename
//! ordinati e `.shp` per ultimo. `.prj` è scritto se c'è una definizione WKT o
//! per WGS84; nessuna riproiezione (`PRODUCT.md § CRS`).
#![forbid(unsafe_code)]

use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs::File;
use std::io::{BufReader, BufWriter, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use arrow_array::builder::BinaryBuilder;
use arrow_array::{Array, ArrayRef, BinaryArray, RecordBatch, RecordBatchOptions};
#[cfg(test)]
use arrow_schema::DataType;
use arrow_schema::{Field, Schema, SchemaRef};
use serde_json::Value as JsonValue;
use shapefile::dbase::{FieldValue, Record, TableWriterBuilder};
use shapefile::{
    Multipoint, MultipointM, MultipointZ, Point, PointM, PointZ, Polygon, PolygonM, PolygonRing,
    PolygonZ, Polyline, PolylineM, PolylineZ, Shape, ShapeReader, ShapeType, Writer, NO_DATA,
};

use driver_common::{
    classify_i64, geometry_field, geometry_index, json_from_array, ColType, InferredColumnBuilder,
    ObservedValueClass, TypeAccumulator,
};
use plenora_io_core::descriptor::{
    CrsHandling, Direction, Fidelity, FormatDescriptor, ReadMode, ReaderConcurrency, Runtime,
    WriteMode,
};
use plenora_io_core::driver::{
    spawn_batch_reader, BatchEmitter, FormatDriver, FormatWriter, LayerReader, OpenDatasetHandle,
    Published, ReadOptions, Sink, Source, WriteOptions,
};
use plenora_io_core::loss::{LossExample, LossReport, Posizione};
use plenora_io_core::publish::{
    create_staged_dir, publish_dir_atomic, publish_files_ordered_limited,
};
use plenora_io_core::request::{BatchTarget, ProjectionMode, ReadRequest, ReadScope};
use plenora_io_core::{
    validate_write, with_write_validation, write_row_rejection, AttributeWriteSupport,
    CrsDerivation, CrsRepresentationCapabilities, CrsRepresentationState, CrsWriteSupport,
    FormatWriteCapabilities, NullabilitySupport, TypeCoercionPolicy, WritePlan, DBF_FIELD_NAMES,
    SCALAR_TYPES, WKB_SINGLE_TYPE_ALL_DIMENSIONS_GEOMETRY,
};
use plenora_io_model::contract::{
    CoordinateDimensions, DataContract, FieldId, GeometryColumnContract, GeometryType,
    LayerContract, LayerId,
};
use plenora_io_model::crs::{CrsKind, RawCrs, ResolvedCrs};
use plenora_io_model::geometry::{with_geometry_contract_metadata, GEO_CRS_KEY};
use plenora_io_model::limits::WkbLimits;
use plenora_io_model::wkb::{
    decode_wkb, encode_wkb, WkbCoordinate, WkbFlavor, WkbGeometry, WkbValue,
};
use plenora_io_model::{
    NumeroStrutturale, PlenoraIoError, PublicMessage, Result, RowDiagnosticExample,
    RowDiagnosticKey, RowDiagnosticKeyState, RowDiagnosticKeyValue, RowDiagnosticScope,
    RowDiagnostics, RowDiagnosticsCompleteness, ROW_DIAGNOSTICS_CONTRACT,
    ROW_DIAGNOSTICS_INDEX_BASIS,
};

const GEOMETRY: &str = "geometry";
const DIRECTORY_DATASET_SUFFIX: &str = ".shp.d";
const DIRECTORY_DATASET_MODE: &str = "shapefile_directory_dataset";
const LOOSE_SET_MODE: &str = "loose_shapefile_set";
const DBF_NUMERIC_INTEGER_PRECISION_UNVERIFIABLE: &str =
    "dbf_numeric_integer_precision_unverifiable";
const FIRST_F64_INTEGER_WITHOUT_UNIT_PRECISION: f64 = 9_007_199_254_740_992.0;
const DBF_HEADER_SIZE: usize = 32;
const DBF_FIELD_DESCRIPTOR_SIZE: usize = 32;
const DBF_HEADER_TERMINATOR_SIZE: usize = 1;
/// Il byte di flag di cancellazione che apre ogni record.
const DBF_DELETION_FLAG_SIZE: usize = 1;
/// Il valore di quel byte quando la riga e' cancellata.
const DBF_RECORD_CANCELLATO: u8 = b'*';
/// Il byte che chiude l'elenco dei descrittori. `dbase` lo pretende con un
/// `debug_assert_eq!`, che sparisce in release: da noi e' un rifiuto, cosi'
/// l'esito non dipende dalla configurazione di build.
const DBF_HEADER_TERMINATOR: u8 = 0x0d;
/// L'intestazione DBF come posizione di ricerca: i descrittori cominciano li'.
const SHP_HEADER_SIZE_DBF: u64 = 32;
#[cfg(test)]
const DBF_FIELD_NAME_SIZE: usize = 11;
const DBF_VISUAL_FOXPRO_VERSION: u8 = 0x30;
/// I byte di versione che **`dbase`** tratta come Visual `FoxPro`.
///
/// Sono tre valori e non un intervallo: e' la tabella di `Version::from(u8)`
/// della crate esterna, e derivarla da una regola significherebbe indovinare
/// che cosa un'altra libreria fara' di un byte. `DBF_VISUAL_FOXPRO_VERSION`
/// resta il solo `0x30` perche' descrive il file che **noi** scriviamo.
const DBF_VERSIONI_VISUAL_FOXPRO: [u8; 3] = [0x30, 0x31, 0x32];
const SHP_HEADER_SIZE: usize = 100;
/// Le stesse misure in `i64`, perche' l'aritmetica di validazione ci lavora
/// dentro: convertirle a ogni uso aggiungerebbe conversioni fallibili a valori
/// che sono costanti di formato.
const SHP_HEADER_BYTE: i64 = 100;
const SHP_HEADER_PAROLE: i64 = 50;
const SHP_RECORD_HEADER_BYTE: i64 = 8;
const SHX_RECORD_BYTE: i64 = 8;
/// Le lunghezze dello Shapefile sono in **parole da 16 bit**, e `shapefile` le
/// riporta in byte moltiplicandole per due dentro un `i32`. Oltre questa soglia
/// il prodotto trabocca: e' il panico che il target ha trovato sull'indice.
const SHP_MAX_PAROLE: i64 = (i32::MAX as i64) / 2;
/// Il contenuto minimo di un record: il solo tag di tipo, quattro byte.
const SHP_MIN_PAROLE_DI_RECORD: i64 = 2;
/// I byte piu' pochi che un elemento dichiarato puo' occupare nel record:
/// quattro per un indice di parte, sedici per un punto XY. Servono a limitare
/// **cio' che viene prenotato** a cio' che il record puo' contenere.
const SHP_BYTE_PER_PARTE: i64 = 4;
const SHP_BYTE_PER_PUNTO: i64 = 16;
const DBF_VISUAL_FOXPRO_BACKLINK_SIZE: usize = 263;
const DEFAULT_ROW_DIAGNOSTICS_EXAMPLES_LIMIT: u64 = 64;
const MAX_ROW_DIAGNOSTICS_EXAMPLES_LIMIT: u64 = 64;
const INNER_RING_WITHOUT_OUTER_CAUSE: &str = "shapefile.inner_ring_without_outer";
const POLYGON_WITHOUT_OUTER_CAUSE: &str = "shapefile.polygon_without_outer";
const UNCLOSED_RING_CAUSE: &str = "shapefile.unclosed_ring";
const DEGENERATE_RING_CAUSE: &str = "shapefile.degenerate_ring";
const ATTRIBUTE_NUMERIC_INVALID_CAUSE: &str = "shapefile.attribute_numeric_invalid";

/// Gli identificatori per cui il writer **sintetizza** una definizione, e
/// quindi scrive il `.prj` anche senza WKT in ingresso.
///
/// La lista e' una sola, e la usano due posti: `wkt_for_id`, che scrive, e la
/// capability `crs_id`, che dichiara. Tenerne due copie vorrebbe dire che il
/// giorno in cui una cresce l'altra mente, e mentirebbe proprio sulla domanda
/// «l'identificatore e' ricavabile dal file?».
pub const CRS_CON_DEFINIZIONE_SINTETIZZATA: &[&str] = &["EPSG:4326", "OGC:CRS84"];

/// WKT standard per WGS84 (accettato da GDAL), usato per il `.prj` quando la
/// sorgente dà solo il codice autorità e non una definizione WKT.
const WGS84_WKT: &str = "GEOGCS[\"WGS 84\",DATUM[\"WGS_1984\",SPHEROID[\"WGS 84\",6378137,298.257223563]],PRIMEM[\"Greenwich\",0],UNIT[\"degree\",0.0174532925199433]]";

fn err(reason: &PublicMessage) -> PlenoraIoError {
    PlenoraIoError::formato_redatto("shp", reason)
}

#[derive(Clone, Copy)]
enum DiagnosticKeyPolicy {
    /// Espone il valore lessicale DBF soltanto quando `key_field` e' stato
    /// configurato esplicitamente e il valore e' attestabile.
    Emit,
    /// Espone esclusivamente nome campo e stato `redacted`, mai il valore DBF.
    Redact,
}

#[derive(Clone)]
struct DiagnosticKeyConfig {
    field: String,
    policy: DiagnosticKeyPolicy,
    raw_numeric_field_index: Option<usize>,
}

#[derive(Clone)]
struct ShpRowDiagnosticsConfig {
    examples_limit: u64,
    /// `None` significa che gli esempi non contengono alcun oggetto `key`;
    /// non esiste una policy implicita.
    key: Option<DiagnosticKeyConfig>,
}

impl ShpRowDiagnosticsConfig {
    fn from_options(
        options: &BTreeMap<String, String>,
        columns: &[ShpColumn],
        dbf_layout: &DbfLayout,
    ) -> Result<Self> {
        let examples_limit = options.get("row_diagnostics.examples_limit").map_or(
            Ok(DEFAULT_ROW_DIAGNOSTICS_EXAMPLES_LIMIT),
            |value| {
                value.parse::<u64>().map_err(|_| {
                    PlenoraIoError::redatto(
                        plenora_io_model::IoErrorCode::Generic,
                        plenora_io_model::ErrorCategory::InvalidConfiguration,
                        plenora_io_model::ErrorPhase::Validate,
                        plenora_io_model::RemoteEffect::None,
                        plenora_io_model::RetryDisposition::Never,
                        &PublicMessage::Curated(
                            "row_diagnostics.examples_limit deve essere un intero",
                        ),
                    )
                })
            },
        )?;
        if !(1..=MAX_ROW_DIAGNOSTICS_EXAMPLES_LIMIT).contains(&examples_limit) {
            return Err(PlenoraIoError::redatto(
                plenora_io_model::IoErrorCode::Generic,
                plenora_io_model::ErrorCategory::InvalidConfiguration,
                plenora_io_model::ErrorPhase::Validate,
                plenora_io_model::RemoteEffect::None,
                plenora_io_model::RetryDisposition::Never,
                &PublicMessage::CuratedWith(
                    "row_diagnostics.examples_limit deve essere compreso fra 1 e",
                    NumeroStrutturale::Limite(MAX_ROW_DIAGNOSTICS_EXAMPLES_LIMIT),
                ),
            ));
        }

        let key = match options.get("row_diagnostics.key_field") {
            None => {
                if options.contains_key("row_diagnostics.key_policy") {
                    return Err(PlenoraIoError::redatto(
                        plenora_io_model::IoErrorCode::Generic,
                        plenora_io_model::ErrorCategory::InvalidConfiguration,
                        plenora_io_model::ErrorPhase::Validate,
                        plenora_io_model::RemoteEffect::None,
                        plenora_io_model::RetryDisposition::Never,
                        &PublicMessage::Curated(
                            "row_diagnostics.key_policy richiede row_diagnostics.key_field",
                        ),
                    ));
                }
                None
            }
            Some(field) => {
                let _column = columns
                    .iter()
                    .find(|column| column.name == *field)
                    .ok_or_else(|| {
                        PlenoraIoError::redatto(
                            plenora_io_model::IoErrorCode::Generic,
                            plenora_io_model::ErrorCategory::InvalidConfiguration,
                            plenora_io_model::ErrorPhase::Validate,
                            plenora_io_model::RemoteEffect::None,
                            plenora_io_model::RetryDisposition::Never,
                            &PublicMessage::Curated(
                                "row_diagnostics.key_field non esiste nello schema DBF",
                            ),
                        )
                    })?;
                let policy = match options
                    .get("row_diagnostics.key_policy")
                    .map(String::as_str)
                {
                    Some("emit") => DiagnosticKeyPolicy::Emit,
                    Some("redact") => DiagnosticKeyPolicy::Redact,
                    _ => {
                        return Err(PlenoraIoError::redatto(
                            plenora_io_model::IoErrorCode::Generic,
                            plenora_io_model::ErrorCategory::InvalidConfiguration,
                            plenora_io_model::ErrorPhase::Validate,
                            plenora_io_model::RemoteEffect::None,
                            plenora_io_model::RetryDisposition::Never,
                            &PublicMessage::Curated(
                                "row_diagnostics.key_policy deve essere 'emit' o 'redact'",
                            ),
                        ))
                    }
                };
                Some(DiagnosticKeyConfig {
                    field: field.clone(),
                    policy,
                    raw_numeric_field_index: dbf_layout.fields.iter().position(|layout| {
                        layout.name == *field && matches!(layout.field_type, b'N' | b'F')
                    }),
                })
            }
        };
        Ok(Self {
            examples_limit,
            key,
        })
    }
}

struct ShpRowDiagnostics {
    config: ShpRowDiagnosticsConfig,
    counts: BTreeMap<String, u64>,
    observed_total: u64,
    examples: Vec<RowDiagnosticExample>,
}

impl ShpRowDiagnostics {
    const fn new(config: ShpRowDiagnosticsConfig) -> Self {
        Self {
            config,
            counts: BTreeMap::new(),
            observed_total: 0,
            examples: Vec::new(),
        }
    }

    const fn is_empty(&self) -> bool {
        self.observed_total == 0
    }

    fn record(
        &mut self,
        source_index: u64,
        cause: &'static str,
        record: Option<&Record>,
        raw_numeric_key: Option<&str>,
    ) {
        self.observed_total += 1;
        *self.counts.entry(cause.to_owned()).or_default() += 1;
        if self.examples.len() as u64 >= self.config.examples_limit {
            return;
        }
        let key = self.config.key.as_ref().map(|config| match config.policy {
            DiagnosticKeyPolicy::Redact => RowDiagnosticKey {
                field: config.field.clone(),
                state: RowDiagnosticKeyState::Redacted,
                value: None,
            },
            DiagnosticKeyPolicy::Emit => {
                let decoded = config
                    .raw_numeric_field_index
                    .is_none()
                    .then(|| {
                        record
                            .and_then(|row| row.get(&config.field))
                            .and_then(fv_string)
                    })
                    .flatten();
                let value = raw_numeric_key.map(str::to_owned).or(decoded);
                match value {
                    Some(value) if value.len() <= 1024 => RowDiagnosticKey {
                        field: config.field.clone(),
                        state: RowDiagnosticKeyState::Value,
                        value: Some(RowDiagnosticKeyValue::String(value)),
                    },
                    _ => RowDiagnosticKey {
                        field: config.field.clone(),
                        state: RowDiagnosticKeyState::Unavailable,
                        value: None,
                    },
                }
            }
        });
        self.examples.push(RowDiagnosticExample {
            source_index,
            cause: cause.to_owned(),
            column: None,
            key,
            write_state: None,
        });
    }

    fn into_report(self) -> RowDiagnostics {
        let total = self.observed_total;
        self.into_report_with(RowDiagnosticsCompleteness::Complete, None, Some(total))
    }

    fn into_partial_report(self, knowledge_limit: &str) -> RowDiagnostics {
        self.into_report_with(
            RowDiagnosticsCompleteness::Partial,
            Some(vec![knowledge_limit.to_owned()]),
            None,
        )
    }

    fn into_partial_error(self, error: PlenoraIoError, knowledge_limit: &str) -> PlenoraIoError {
        if self.is_empty() {
            error
        } else {
            error.with_row_diagnostics(self.into_partial_report(knowledge_limit))
        }
    }

    fn into_report_with(
        self,
        completeness: RowDiagnosticsCompleteness,
        knowledge_limits: Option<Vec<String>>,
        total: Option<u64>,
    ) -> RowDiagnostics {
        let examples_truncated = self.observed_total > self.examples.len() as u64;
        RowDiagnostics {
            contract: ROW_DIAGNOSTICS_CONTRACT.to_owned(),
            scope: RowDiagnosticScope::Read,
            index_basis: ROW_DIAGNOSTICS_INDEX_BASIS.to_owned(),
            completeness,
            knowledge_limits,
            observed_total: self.observed_total,
            total,
            input_total: None,
            counts: self.counts,
            examples_limit: self.config.examples_limit,
            examples_truncated,
            examples: self.examples,
            diagnostic_state_counts: None,
            write_outcome: None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ShapefilePublishMode {
    DirectoryDataset,
    LooseSet,
}

fn is_directory_dataset_path(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| {
            name.to_ascii_lowercase()
                .ends_with(DIRECTORY_DATASET_SUFFIX)
        })
}

/// Il set loose non si deduce: si chiede.
///
/// Un'estensione non e' un consenso. Fino a questa revisione una destinazione
/// `*.shp` faceva dedurre il set loose, e chi la scriveva otteneva senza saperlo
/// una pubblicazione **non crash-atomic**: quattro rename in sequenza, con
/// rollback best-effort e, quando il rollback fallisce, companion visibili
/// accanto a nessun `.shp`. Il rischio e' reale ma governabile; sceglierlo per
/// distrazione non lo e'.
///
/// La regola nuova ha due meta' asimmetriche, e l'asimmetria e' voluta:
///
/// * `*.shp.d` **deduce** il directory-dataset, perche' e' la forma con la
///   garanzia piu' forte — un rename solo, atomico — e non c'e' niente da
///   accettare;
/// * `*.shp` **non deduce nulla** e pretende `publish_mode=loose_shapefile_set`.
///   Senza quell'opzione la scrittura e' rifiutata prima di creare lo staging.
///
/// Un `publish_mode` che contraddice il suffisso resta un errore in entrambi i
/// versi: l'opzione dichiara la forma, non la sceglie contro la destinazione.
///
/// La procedura di recovery per un set loose interrotto sta in
/// `PRODUCT.md § Publish` e in `ENGINEERING.md § Pipeline di scrittura`.
fn publish_mode(path: &Path, opts: &WriteOptions) -> Result<ShapefilePublishMode> {
    let inferred = if is_directory_dataset_path(path) {
        Some(ShapefilePublishMode::DirectoryDataset)
    } else if path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("shp"))
    {
        // Nessuna deduzione: la scelta della forma debole appartiene a chi
        // scrive, non all'estensione che ha battuto.
        None
    } else {
        return Err(PlenoraIoError::non_supportato_redatto(&PublicMessage::Curated("l'output Shapefile deve terminare con .shp (loose set) o .shp.d (directory dataset)")));
    };
    let requested = match opts.format_options.get("publish_mode").map(String::as_str) {
        None => None,
        Some(DIRECTORY_DATASET_MODE) => Some(ShapefilePublishMode::DirectoryDataset),
        Some(LOOSE_SET_MODE) => Some(ShapefilePublishMode::LooseSet),
        Some(_) => {
            // Il valore non esce: lo schema dichiara `publish_mode` come
            // `Enumerato`, quindi un valore diverso e' gia' stato respinto da
            // `valida_opzioni` con il suo token. Questo ramo e' difensivo.
            return Err(PlenoraIoError::non_supportato_redatto(
                &PublicMessage::Curated(
                    "publish_mode Shapefile non valido; usare directory-dataset o loose-set",
                ),
            ));
        }
    };
    match (inferred, requested) {
        (Some(dedotto), None) => Ok(dedotto),
        (Some(dedotto), Some(chiesto)) if chiesto == dedotto => Ok(dedotto),
        (None, Some(ShapefilePublishMode::LooseSet)) => Ok(ShapefilePublishMode::LooseSet),
        (_, Some(chiesto)) => Err(PlenoraIoError::non_supportato_redatto(
            // Entrambi sono `&'static str` del nostro enum, non testo runtime.
            &PublicMessage::CuratedPair(
                "publish_mode richiede una destinazione con suffisso",
                chiesto.destination_suffix(),
            ),
        )),
        (None, None) => Err(opt_in_loose_mancante()),
    }
}

/// Il rifiuto che chiede il consenso, e dice che cosa si stava per accettare.
///
/// La categoria e' `InvalidConfiguration` e non `Unsupported`, per la regola
/// dichiarata in [`plenora_io_model::ErrorCategory`]: il prodotto **sa** fare
/// questa scrittura, e' la richiesta a essere incompleta. Davanti a
/// `Unsupported` chi automatizza cambia driver o formato, che qui sarebbe la
/// reazione sbagliata: cio' che serve e' una riga in piu' nella richiesta.
///
/// Stesso codice e stessa fase dello scarto di `valida_opzioni`, perche' e' la
/// stessa specie di rifiuto: un'opzione di scrittura che manca.
fn opt_in_loose_mancante() -> PlenoraIoError {
    PlenoraIoError::redatto(
        plenora_io_model::IoErrorCode::Unsupported,
        plenora_io_model::ErrorCategory::InvalidConfiguration,
        plenora_io_model::ErrorPhase::Validate,
        plenora_io_model::RemoteEffect::None,
        plenora_io_model::RetryDisposition::Never,
        &PublicMessage::Curated(
            "una destinazione .shp pubblica un set di file non crash-atomic: \
             va accettata con publish_mode=loose_shapefile_set, oppure sostituita \
             da una destinazione .shp.d, che pubblica con un solo rename atomico",
        ),
    )
}

impl ShapefilePublishMode {
    const fn destination_suffix(self) -> &'static str {
        match self {
            Self::DirectoryDataset => "*.shp.d",
            Self::LooseSet => "*.shp",
        }
    }
}

fn shapefile_source_path(path: PathBuf) -> Result<PathBuf> {
    if !path.is_dir() {
        return Ok(path);
    }
    if !is_directory_dataset_path(&path) {
        return Err(PlenoraIoError::non_supportato_redatto(
            &PublicMessage::Curated("directory Shapefile non riconosciuta (atteso *.shp.d)"),
        ));
    }
    let source = path.join("data.shp");
    if !source.is_file() {
        return Err(err(&PublicMessage::Curated(
            "directory dataset senza data.shp",
        )));
    }
    Ok(source)
}

use plenora_io_model::format_options::{
    FaseOpzione, OpzioneFormato, SchemaOpzioniFormato, ValoreAmmesso,
};

/// Le `format_options` interpretate dal driver Shapefile (L0.7, S6).
///
/// Gli estremi di `row_diagnostics.examples_limit` sono gli stessi che il
/// driver applica: dichiararli qui non sposta il controllo, lo rende leggibile
/// prima di aprire il file.
const SCHEMA_OPZIONI: SchemaOpzioniFormato = SchemaOpzioniFormato::nuovo(&[
    OpzioneFormato {
        chiave: "publish_mode",
        fase: FaseOpzione::Scrittura,
        valore: ValoreAmmesso::Enumerato(&[DIRECTORY_DATASET_MODE, LOOSE_SET_MODE]),
        predefinito: None,
        descrizione: "forma di pubblicazione; .shp.d si deduce, .shp va accettata",
    },
    OpzioneFormato {
        chiave: "row_diagnostics.examples_limit",
        fase: FaseOpzione::Lettura,
        valore: ValoreAmmesso::Intero {
            minimo: 1,
            massimo: MAX_ROW_DIAGNOSTICS_EXAMPLES_LIMIT,
        },
        predefinito: Some("64"),
        descrizione: "numero massimo di righe di esempio per diagnostica",
    },
    OpzioneFormato {
        chiave: "row_diagnostics.key_field",
        fase: FaseOpzione::Lettura,
        valore: ValoreAmmesso::Testo,
        predefinito: None,
        descrizione: "campo DBF usato come chiave nelle diagnostiche di riga",
    },
    OpzioneFormato {
        chiave: "row_diagnostics.key_policy",
        fase: FaseOpzione::Lettura,
        valore: ValoreAmmesso::Enumerato(&["emit", "redact"]),
        predefinito: None,
        descrizione: "se emettere o redigere la chiave; richiede row_diagnostics.key_field",
    },
]);

static DESCRIPTOR: FormatDescriptor = FormatDescriptor::const_new(
    "shp",
    Direction::Bidirectional,
    ReadMode::StreamingSequential,
    // INV-7: una sola `seek` per saltare l'header, poi sequenziale.
    plenora_io_core::NativeReadMode::StreamingSequential,
    // Il drenaggio e lo spool sono dell'adapter comune, non di
    // questo driver: `BudgetedReader` li impone a tutti.
    plenora_io_core::DeliverySemantics::OperationAtomic,
    plenora_io_core::BufferingStrategy::AdaptiveMemoryThenDisk,
    plenora_io_core::DeterminismLevel::Semantic,
    Some(WriteMode::Streaming),
    Some(plenora_io_core::DeterminismLevel::Semantic),
    false,
    true, // .shp/.shx/.dbf/.prj
    ReaderConcurrency::MultipleIndependentReaders,
    plenora_io_core::ProjectionSupport::Exact,
    plenora_io_core::PredicatePruningSupport::None,
    plenora_io_core::SpatialPruningSupport::None,
    CrsHandling::Embedded,
    Fidelity::Conditional,
    Runtime::PureRust,
    // `hostile_input_hardened`: non dichiarato: l'input e' binario e ha la sua prevalidazione
    // strutturale, che e' un'altra garanzia e non questa.
    false,
    // `spec_version_supported`: il formato non si versiona in un modo che
    // il driver possa dichiarare per intero.
    None,
    Some(FormatWriteCapabilities {
        field_names: DBF_FIELD_NAMES,
        allowed_types: SCALAR_TYPES,
        type_coercion: TypeCoercionPolicy::ExplicitText,
        attributes: AttributeWriteSupport::All,
        geometry: WKB_SINGLE_TYPE_ALL_DIMENSIONS_GEOMETRY,
        crs: CrsWriteSupport::Embedded,
        crs_representations: CrsRepresentationCapabilities::new(
            // L'identificatore si rilegge dal `.prj`, e il `.prj` c'e' se la
            // sorgente porta un WKT oppure se l'identificatore e' uno di
            // quelli che il writer sa sintetizzare. Fuori da questi due casi
            // non viene scritto niente da cui ricavarlo.
            CrsRepresentationState::Derived(CrsDerivation::FromDefinition {
                synthesized_for: CRS_CON_DEFINIZIONE_SINTETIZZATA,
            }),
            CrsRepresentationState::Absent,
            CrsRepresentationState::Preserved,
        ),
        nullability: NullabilitySupport::FormatDefined,
        multi_layer: false,
    }),
    SCHEMA_OPZIONI,
    1,
    9,
    9,
);

pub struct ShpDriver;

impl FormatDriver for ShpDriver {
    fn descriptor(&self) -> &FormatDescriptor {
        &DESCRIPTOR
    }

    fn open(&self, source: Source, mut opts: ReadOptions) -> Result<Box<dyn OpenDatasetHandle>> {
        let path = shapefile_source_path(plenora_io_core::preflight_source(
            self.descriptor(),
            source,
            &mut opts,
        )?)?;
        let crs = resolve_crs(&path, &opts)?;
        // Pass 1: inferenza schema (nomi + tipi) dai record, a RAM O(ncol).
        let ShpInference {
            cols,
            dbf_layout,
            geometry_info,
            active_row_count,
            loss,
        } = infer_shp_schema(&path)?;
        let row_diagnostics =
            ShpRowDiagnosticsConfig::from_options(&opts.format_options, &cols, &dbf_layout)?;
        let mut geometry_contract =
            GeometryColumnContract::wkb_xy(FieldId(0), GEOMETRY, crs.clone(), true);
        geometry_contract.dimensions = geometry_info.dimensions;
        geometry_contract.set_exact_geometry_types(geometry_info.geometry_types);
        if let Some(shape_type) = geometry_info.shape_type {
            geometry_contract
                .native_metadata
                .insert("shp.shape_type".to_owned(), shape_type.to_owned());
        }
        if matches!(
            geometry_contract.dimensions,
            CoordinateDimensions::Xym | CoordinateDimensions::Xyzm
        ) {
            geometry_contract
                .native_metadata
                .insert("shp.measure_no_data".to_owned(), NO_DATA.to_string());
        }
        let crs_id = resolved_crs_id(&crs)?;
        let geometry_field =
            with_geometry_contract_metadata(&geometry_field(GEOMETRY, crs_id), &geometry_contract);
        let mut fields = vec![geometry_field];
        for column in &cols {
            fields.push(Field::new(
                &column.name,
                column.column_type.arrow_data_type(),
                true,
            ));
        }
        let schema: SchemaRef = Arc::new(Schema::new(fields));
        let contract = DataContract::new(schema, Some(geometry_contract.clone()));
        let name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("layer")
            .to_owned();
        Ok(plenora_io_core::with_read_budget(
            Box::new(ShpDataset {
                path,
                cols,
                dbf_layout,
                dimensions: geometry_contract.dimensions,
                shape_type: geometry_info.shape_type,
                active_row_count,
                loss,
                row_diagnostics,
                layers: vec![LayerContract {
                    id: LayerId(0),
                    name,
                    contract,
                }],
            }),
            &opts,
            false,
        ))
    }

    fn create(
        &self,
        sink: Sink,
        plan: &WritePlan,
        opts: &WriteOptions,
    ) -> Result<Box<dyn FormatWriter>> {
        validate_write(
            self.descriptor(),
            plan,
            opts.max_columns(),
            &opts.format_options,
        )?;
        let Sink::Path(dest) = sink;
        let publish_mode = publish_mode(&dest, opts)?;
        if plan.layers.len() != 1 {
            return Err(PlenoraIoError::non_supportato_redatto(
                &PublicMessage::Curated("Shapefile: un solo layer per file"),
            ));
        }
        match publish_mode {
            ShapefilePublishMode::DirectoryDataset => {
                if dest.exists() {
                    return Err(PlenoraIoError::destinazione_esistente());
                }
            }
            ShapefilePublishMode::LooseSet => {
                // no-clobber sull'intero set.
                for ext in ["shp", "shx", "dbf", "prj"] {
                    let sibling = dest.with_extension(ext);
                    if sibling.exists() {
                        return Err(PlenoraIoError::destinazione_esistente());
                    }
                }
            }
        }

        let layer = &plan.layers[0];
        let schema = &layer.contract.schema;
        let geom_idx = geometry_index(schema).ok_or_else(|| {
            err(&PublicMessage::Curated(
                "il contratto non ha una colonna geometria geoarrow.wkb",
            ))
        })?;

        // Capability-check (`ENGINEERING.md § Pipeline di scrittura (capability-check`)): costruisce il dbf, fail-closed sui nomi.
        let mut table = TableWriterBuilder::new();
        let mut attrs: Vec<(usize, String, DbfKind)> = Vec::new();
        for (i, f) in schema.fields().iter().enumerate() {
            if i == geom_idx {
                continue;
            }
            let fname = shapefile::dbase::FieldName::try_from(f.name().as_str()).map_err(|_| {
                // Il nome viene dal piano, e chi legge l'errore ha il piano.
                PlenoraIoError::non_supportato_redatto(&PublicMessage::CuratedWith(
                    "nome campo non valido per dbf (max 10 caratteri ASCII), indice",
                    NumeroStrutturale::Indice(driver_common::saturating_u64(i)),
                ))
            })?;
            let kind = DbfKind::from(f.data_type());
            table = match kind {
                DbfKind::Char => table.add_character_field(fname, 254),
                DbfKind::Int => table.add_numeric_field(fname, 18, 0),
                DbfKind::Float => table.add_numeric_field(fname, 20, 8),
                DbfKind::Logical => table.add_logical_field(fname),
            };
            attrs.push((i, f.name().clone(), kind));
        }

        let staging = create_staged_dir(&dest)?;
        let shp_path = staging.path().join("data.shp");
        let writer = Writer::from_path(&shp_path, table)
            .map_err(|_| err(&PublicMessage::Curated("creazione dello shapefile fallita")))?;

        with_write_validation(
            Box::new(ShpWriter {
                staging: Some(staging),
                writer: Some(writer),
                dest,
                durable: opts.durable,
                publish_mode,
                attrs,
                geom_idx,
                prj: resolve_prj(layer, schema, geom_idx),
                shape_type: None,
                rows: 0,
                input_total: None,
                wkb_limits: opts.wkb_limits(),
                max_output_bytes: opts.max_output_bytes(),
            }),
            self.descriptor(),
            plan,
            opts,
        )
    }
}

// --- lettura streaming -----------------------------------------------------

struct ShpDataset {
    path: PathBuf,
    cols: Vec<ShpColumn>,
    dbf_layout: DbfLayout,
    dimensions: CoordinateDimensions,
    shape_type: Option<&'static str>,
    active_row_count: u64,
    loss: LossReport,
    row_diagnostics: ShpRowDiagnosticsConfig,
    layers: Vec<LayerContract>,
}

impl OpenDatasetHandle for ShpDataset {
    fn layers(&self) -> &[LayerContract] {
        &self.layers
    }
    fn fidelity_assessment(&self) -> plenora_io_core::FidelityAssessment {
        plenora_io_core::FidelityAssessment::for_format(
            DESCRIPTOR.id(),
            DESCRIPTOR.fidelity_class(),
        )
        .with_loss_report(&self.loss)
    }
    fn open_layer_reader(&self, request: &ReadRequest) -> Result<Box<dyn LayerReader>> {
        plenora_io_core::validate_read_projection(&DESCRIPTOR, request)?;
        let (indices, layer) = plenora_io_core::project_layer_contract(&self.layers[0], request)?;
        let include_geometry = indices.binary_search(&0).is_ok();
        let cols = indices
            .iter()
            .filter_map(|&index| {
                index
                    .checked_sub(1)
                    .and_then(|column_index| self.cols.get(column_index))
                    .cloned()
            })
            .collect();
        let batch_sizer = plenora_io_core::AdaptiveBatchSizer::new(
            layer.contract.schema.as_ref(),
            request.batch_target,
        );
        let reader = spawn_parser(ShpParserInput {
            path: self.path.clone(),
            schema: layer.contract.schema.clone(),
            cols,
            dbf_layout: self.dbf_layout.clone(),
            dimensions: self.dimensions,
            expected_shape_type: self.shape_type,
            expected_active_rows: self.active_row_count,
            include_geometry,
            batch_sizer,
            layer,
            loss: self.loss.clone(),
            row_diagnostics: self.row_diagnostics.clone(),
            scope: request.scope,
            cancellation: request.cancellation.clone(),
        })?;
        Ok(plenora_io_core::with_cancellation(
            reader,
            request.cancellation.clone(),
        ))
    }
}

// --- scrittura -------------------------------------------------------------

#[derive(Clone, Copy)]
enum DbfKind {
    Char,
    Int,
    Float,
    Logical,
}

impl DbfKind {
    const fn from(dt: &arrow_schema::DataType) -> Self {
        use arrow_schema::DataType as D;
        match dt {
            D::Int8
            | D::Int16
            | D::Int32
            | D::Int64
            | D::UInt8
            | D::UInt16
            | D::UInt32
            | D::UInt64 => Self::Int,
            D::Float16 | D::Float32 | D::Float64 => Self::Float,
            D::Boolean => Self::Logical,
            _ => Self::Char,
        }
    }
}

struct ShpWriter {
    staging: Option<tempfile::TempDir>,
    writer: Option<Writer<BufWriter<File>>>,
    dest: PathBuf,
    durable: bool,
    publish_mode: ShapefilePublishMode,
    attrs: Vec<(usize, String, DbfKind)>,
    geom_idx: usize,
    prj: Option<String>,
    shape_type: Option<&'static str>,
    rows: u64,
    input_total: Option<u64>,
    wkb_limits: WkbLimits,
    max_output_bytes: u64,
}

impl FormatWriter for ShpWriter {
    fn declare_input_total(&mut self, layer: LayerId, total: u64) -> Result<()> {
        if layer.0 != 0 {
            return Err(PlenoraIoError::non_supportato_redatto(
                &PublicMessage::Curated("Shapefile supporta un solo layer"),
            ));
        }
        self.input_total = Some(total);
        Ok(())
    }

    fn write(&mut self, batch: &RecordBatch) -> Result<()> {
        let geom_col = batch
            .column(self.geom_idx)
            .as_any()
            .downcast_ref::<BinaryArray>()
            .ok_or_else(|| err(&PublicMessage::Curated("colonna geometria non binaria")))?;
        let limits = self.wkb_limits;
        let mut st = self.shape_type;
        let mut prepared = Vec::with_capacity(batch.num_rows());
        let mut rejections = Vec::new();
        for row in 0..batch.num_rows() {
            if geom_col.is_null(row) {
                rejections.push((row, "shapefile.null_geometry_unsupported", GEOMETRY));
                continue;
            }
            let Ok(geometry) = decode_wkb(geom_col.value(row), &limits) else {
                rejections.push((row, "shapefile.invalid_geometry", GEOMETRY));
                continue;
            };
            let Ok(shape) = shape_from_wkb(geometry) else {
                rejections.push((row, "shapefile.geometry_not_representable", GEOMETRY));
                continue;
            };
            // Capability-check (`ENGINEERING.md § Pipeline di scrittura (capability-check`)): un unico tipo di geometria per file.
            let tag = shape_tag(&shape);
            if tag == "unsupported" {
                rejections.push((row, "shapefile.geometry_type_unsupported", GEOMETRY));
                continue;
            }
            if !tag.is_empty() && st.is_some_and(|existing| existing != tag) {
                rejections.push((row, "shapefile.mixed_geometry_type", GEOMETRY));
                continue;
            }
            let mut rec = Record::default();
            let mut valid_record = true;
            for (col, name, kind) in &self.attrs {
                let Ok(value) = cell_to_field(batch.column(*col), row, *kind) else {
                    rejections.push((row, "shapefile.cell_not_representable", name.as_str()));
                    valid_record = false;
                    break;
                };
                rec.insert(name.clone(), value);
            }
            if valid_record {
                if !tag.is_empty() && st.is_none() {
                    st = Some(tag);
                }
                prepared.push((shape, rec));
            }
        }
        if !rejections.is_empty() {
            return Err(write_row_rejection(
                "shp",
                self.rows,
                batch.num_rows(),
                &rejections,
                self.input_total,
            ));
        }
        // Il conteggio **prima** della scrittura: un lotto che farebbe
        // traboccare il contatore viene rifiutato con il writer ancora intatto,
        // invece di essere constatato dopo che le shape sono gia' sul disco.
        let righe = righe_dopo_il_lotto(self.rows, batch.num_rows())?;
        let w = self
            .writer
            .as_mut()
            .ok_or_else(|| err(&PublicMessage::Curated("writer chiuso")))?;
        for (shape, rec) in prepared {
            write_shape(w, shape, &rec)?;
        }
        self.shape_type = st;
        self.rows = righe;
        Ok(())
    }

    fn finish(mut self: Box<Self>) -> Result<Published> {
        // Finalizza .shp/.shx/.dbf (header + bounding box) rilasciando il writer.
        let w = self
            .writer
            .take()
            .ok_or_else(|| err(&PublicMessage::Curated("writer già chiuso")))?;
        drop(w);
        let staging = self
            .staging
            .take()
            .ok_or_else(|| err(&PublicMessage::Curated("staging mancante")))?;

        if let Some(wkt) = &self.prj {
            std::fs::write(staging.path().join("data.prj"), wkt)?;
        }

        let dimensioni = ["dbf", "shx", "prj", "shp"]
            .into_iter()
            .map(|ext| staging.path().join(format!("data.{ext}")))
            .filter(|path| path.exists())
            .map(|path| std::fs::metadata(path).map(|dati| dati.len()))
            .collect::<std::io::Result<Vec<_>>>()?;
        let staged_bytes = byte_dello_staging(dimensioni)?;
        if staged_bytes > self.max_output_bytes {
            return Err(PlenoraIoError::limite_redatto(
                &PublicMessage::CuratedBetween(
                    "output Shapefile da",
                    NumeroStrutturale::Conteggio(staged_bytes),
                    "byte oltre il limite di",
                    NumeroStrutturale::Limite(self.max_output_bytes),
                ),
            ));
        }

        let (bytes, outcome) = match self.publish_mode {
            ShapefilePublishMode::DirectoryDataset => {
                let outcome = publish_dir_atomic(staging.path(), &self.dest, self.durable)?;
                (staged_bytes, outcome)
            }
            ShapefilePublishMode::LooseSet => {
                // Companion prima, .shp marker per ultimo.
                let files = ["dbf", "shx", "prj", "shp"]
                    .into_iter()
                    .map(|extension| {
                        (
                            staging.path().join(format!("data.{extension}")),
                            self.dest.with_extension(extension),
                        )
                    })
                    .filter(|(source, _)| source.exists())
                    .collect::<Vec<_>>();
                publish_files_ordered_limited(&files, self.durable, self.max_output_bytes)?
            }
        };
        Ok(Published {
            bytes,
            loss: LossReport::default(),
            fidelity: plenora_io_core::FidelityAssessment::lossless(),
            outcome,
        })
    }
}

enum ShpTopology {
    Point(WkbCoordinate),
    Multipoint(Vec<WkbCoordinate>),
    Polyline(Vec<Vec<WkbCoordinate>>),
    /// `true` marks an exterior ring, `false` an interior ring.
    Polygon(Vec<(bool, Vec<WkbCoordinate>)>),
}

fn take_child(
    child: WkbGeometry,
    parent_dimensions: CoordinateDimensions,
    expected: GeometryType,
) -> Result<WkbValue> {
    if child.srid.is_some()
        || child.dimensions != parent_dimensions
        || child.geometry_type() != expected
    {
        return Err(err(&PublicMessage::Curated(
            "geometria WKB annidata incoerente per Shapefile",
        )));
    }
    Ok(child.value)
}

fn polygon_rings(
    rings: Vec<Vec<WkbCoordinate>>,
    destination: &mut Vec<(bool, Vec<WkbCoordinate>)>,
) -> Result<()> {
    if rings.is_empty() {
        return Err(err(&PublicMessage::Curated(
            "poligono vuoto non rappresentabile in Shapefile",
        )));
    }
    for (index, ring) in rings.into_iter().enumerate() {
        if ring.len() < 4 || ring.first() != ring.last() {
            return Err(err(&PublicMessage::Curated(
                "anello WKB non chiuso o con meno di quattro coordinate",
            )));
        }
        destination.push((index == 0, ring));
    }
    Ok(())
}

fn topology_from_wkb(geometry: WkbGeometry) -> Result<ShpTopology> {
    if geometry.srid.is_some() {
        return Err(err(&PublicMessage::Curated(
            "SRID embedded non rappresentabile nel payload Shapefile; usare il CRS del layer",
        )));
    }
    let dimensions = geometry.dimensions;
    match geometry.value {
        WkbValue::Point(coordinate) => Ok(ShpTopology::Point(coordinate)),
        WkbValue::MultiPoint(children) => {
            if children.is_empty() {
                return Err(err(&PublicMessage::Curated(
                    "MultiPoint vuoto non rappresentabile in Shapefile",
                )));
            }
            let mut coordinates = Vec::with_capacity(children.len());
            for child in children {
                match take_child(child, dimensions, GeometryType::Point)? {
                    WkbValue::Point(coordinate) => coordinates.push(coordinate),
                    _ => {
                        return Err(err(&PublicMessage::Curated(
                            "MultiPoint con membro non-Point",
                        )))
                    }
                }
            }
            Ok(ShpTopology::Multipoint(coordinates))
        }
        WkbValue::LineString(coordinates) => {
            if coordinates.len() < 2 {
                return Err(err(&PublicMessage::Curated(
                    "LineString con meno di due coordinate non rappresentabile in Shapefile",
                )));
            }
            Ok(ShpTopology::Polyline(vec![coordinates]))
        }
        WkbValue::MultiLineString(children) => {
            if children.is_empty() {
                return Err(err(&PublicMessage::Curated(
                    "MultiLineString vuoto non rappresentabile in Shapefile",
                )));
            }
            let mut parts = Vec::with_capacity(children.len());
            for child in children {
                match take_child(child, dimensions, GeometryType::LineString)? {
                    WkbValue::LineString(coordinates) if coordinates.len() >= 2 => {
                        parts.push(coordinates);
                    }
                    WkbValue::LineString(_) => {
                        return Err(err(&PublicMessage::Curated(
                            "parte LineString con meno di due coordinate in Shapefile",
                        )))
                    }
                    _ => {
                        return Err(err(&PublicMessage::Curated(
                            "MultiLineString con membro non-LineString",
                        )))
                    }
                }
            }
            Ok(ShpTopology::Polyline(parts))
        }
        WkbValue::Polygon(rings) => {
            let mut destination = Vec::with_capacity(rings.len());
            polygon_rings(rings, &mut destination)?;
            Ok(ShpTopology::Polygon(destination))
        }
        WkbValue::MultiPolygon(children) => {
            if children.is_empty() {
                return Err(err(&PublicMessage::Curated(
                    "MultiPolygon vuoto non rappresentabile in Shapefile",
                )));
            }
            let mut destination = Vec::new();
            for child in children {
                match take_child(child, dimensions, GeometryType::Polygon)? {
                    WkbValue::Polygon(rings) => polygon_rings(rings, &mut destination)?,
                    _ => {
                        return Err(err(&PublicMessage::Curated(
                            "MultiPolygon con membro non-Polygon",
                        )))
                    }
                }
            }
            Ok(ShpTopology::Polygon(destination))
        }
        WkbValue::GeometryCollection(_) => Err(err(&PublicMessage::Curated(
            "GeometryCollection non rappresentabile in Shapefile",
        ))),
        WkbValue::CircularString(_)
        | WkbValue::CompoundCurve(_)
        | WkbValue::CurvePolygon(_)
        | WkbValue::MultiCurve(_)
        | WkbValue::MultiSurface(_)
        | WkbValue::PolyhedralSurface(_)
        | WkbValue::Tin(_)
        | WkbValue::Triangle(_) => Err(err(&PublicMessage::Curated(
            "tipo WKB esteso non rappresentabile in Shapefile senza normalizzazione",
        ))),
    }
}

fn point_m(coordinate: WkbCoordinate) -> Result<PointM> {
    let measure = coordinate
        .m
        .ok_or_else(|| err(&PublicMessage::Curated("coordinata XYM senza ordinata M")))?;
    Ok(PointM::new(coordinate.x, coordinate.y, measure))
}

fn point_z(coordinate: WkbCoordinate, require_measure: bool) -> Result<PointZ> {
    let z = coordinate
        .z
        .ok_or_else(|| err(&PublicMessage::Curated("coordinata XYZ senza ordinata Z")))?;
    let measure = if require_measure {
        coordinate
            .m
            .ok_or_else(|| err(&PublicMessage::Curated("coordinata XYZM senza ordinata M")))?
    } else {
        NO_DATA
    };
    Ok(PointZ::new(coordinate.x, coordinate.y, z, measure))
}

fn convert_parts<T, F>(parts: Vec<Vec<WkbCoordinate>>, convert: F) -> Result<Vec<Vec<T>>>
where
    F: Fn(WkbCoordinate) -> Result<T> + Copy,
{
    parts
        .into_iter()
        .map(|part| part.into_iter().map(convert).collect())
        .collect()
}

fn convert_rings<T, F>(
    rings: Vec<(bool, Vec<WkbCoordinate>)>,
    convert: F,
) -> Result<Vec<PolygonRing<T>>>
where
    F: Fn(WkbCoordinate) -> Result<T> + Copy,
{
    rings
        .into_iter()
        .map(|(outer, ring)| {
            let points = ring.into_iter().map(convert).collect::<Result<Vec<_>>>()?;
            Ok(if outer {
                PolygonRing::Outer(points)
            } else {
                PolygonRing::Inner(points)
            })
        })
        .collect()
}

fn shape_from_wkb(geometry: WkbGeometry) -> Result<Shape> {
    let dimensions = geometry.dimensions;
    let topology = topology_from_wkb(geometry)?;
    match (dimensions, topology) {
        (CoordinateDimensions::Xy, ShpTopology::Point(c)) => Ok(Shape::Point(Point::new(c.x, c.y))),
        (CoordinateDimensions::Xym, ShpTopology::Point(c)) => Ok(Shape::PointM(point_m(c)?)),
        (CoordinateDimensions::Xyz, ShpTopology::Point(c)) => Ok(Shape::PointZ(point_z(c, false)?)),
        (CoordinateDimensions::Xyzm, ShpTopology::Point(c)) => Ok(Shape::PointZ(point_z(c, true)?)),
        (CoordinateDimensions::Xy, ShpTopology::Multipoint(coordinates)) => {
            Ok(Shape::Multipoint(Multipoint::new(
                coordinates
                    .into_iter()
                    .map(|c| Point::new(c.x, c.y))
                    .collect(),
            )))
        }
        (CoordinateDimensions::Xym, ShpTopology::Multipoint(coordinates)) => {
            let points = coordinates
                .into_iter()
                .map(point_m)
                .collect::<Result<Vec<_>>>()?;
            Ok(Shape::MultipointM(MultipointM::new(points)))
        }
        (CoordinateDimensions::Xyz, ShpTopology::Multipoint(coordinates)) => {
            let points = coordinates
                .into_iter()
                .map(|coordinate| point_z(coordinate, false))
                .collect::<Result<Vec<_>>>()?;
            Ok(Shape::MultipointZ(MultipointZ::new(points)))
        }
        (CoordinateDimensions::Xyzm, ShpTopology::Multipoint(coordinates)) => {
            let points = coordinates
                .into_iter()
                .map(|coordinate| point_z(coordinate, true))
                .collect::<Result<Vec<_>>>()?;
            Ok(Shape::MultipointZ(MultipointZ::new(points)))
        }
        (CoordinateDimensions::Xy, ShpTopology::Polyline(parts)) => {
            Ok(Shape::Polyline(Polyline::with_parts(
                parts
                    .into_iter()
                    .map(|part| part.into_iter().map(|c| Point::new(c.x, c.y)).collect())
                    .collect(),
            )))
        }
        (CoordinateDimensions::Xym, ShpTopology::Polyline(parts)) => Ok(Shape::PolylineM(
            PolylineM::with_parts(convert_parts(parts, point_m)?),
        )),
        (CoordinateDimensions::Xyz, ShpTopology::Polyline(parts)) => Ok(Shape::PolylineZ(
            PolylineZ::with_parts(convert_parts(parts, |coordinate| {
                point_z(coordinate, false)
            })?),
        )),
        (CoordinateDimensions::Xyzm, ShpTopology::Polyline(parts)) => Ok(Shape::PolylineZ(
            PolylineZ::with_parts(convert_parts(parts, |coordinate| {
                point_z(coordinate, true)
            })?),
        )),
        (CoordinateDimensions::Xy, ShpTopology::Polygon(rings)) => {
            Ok(Shape::Polygon(Polygon::with_rings(
                rings
                    .into_iter()
                    .map(|(outer, ring)| {
                        let points = ring.into_iter().map(|c| Point::new(c.x, c.y)).collect();
                        if outer {
                            PolygonRing::Outer(points)
                        } else {
                            PolygonRing::Inner(points)
                        }
                    })
                    .collect(),
            )))
        }
        (CoordinateDimensions::Xym, ShpTopology::Polygon(rings)) => Ok(Shape::PolygonM(
            PolygonM::with_rings(convert_rings(rings, point_m)?),
        )),
        (CoordinateDimensions::Xyz, ShpTopology::Polygon(rings)) => Ok(Shape::PolygonZ(
            PolygonZ::with_rings(convert_rings(rings, |coordinate| {
                point_z(coordinate, false)
            })?),
        )),
        (CoordinateDimensions::Xyzm, ShpTopology::Polygon(rings)) => Ok(Shape::PolygonZ(
            PolygonZ::with_rings(convert_rings(rings, |coordinate| {
                point_z(coordinate, true)
            })?),
        )),
        (CoordinateDimensions::Unknown, _) => Err(err(&PublicMessage::Curated(
            "dimensionalità WKB ignota non scrivibile in Shapefile",
        ))),
    }
}

const fn shape_tag(s: &Shape) -> &'static str {
    match s {
        Shape::Point(_) => "point-xy",
        Shape::PointM(_) => "point-m",
        Shape::PointZ(_) => "point-z",
        Shape::Polyline(_) => "polyline-xy",
        Shape::PolylineM(_) => "polyline-m",
        Shape::PolylineZ(_) => "polyline-z",
        Shape::Polygon(_) => "polygon-xy",
        Shape::PolygonM(_) => "polygon-m",
        Shape::PolygonZ(_) => "polygon-z",
        Shape::Multipoint(_) => "multipoint-xy",
        Shape::MultipointM(_) => "multipoint-m",
        Shape::MultipointZ(_) => "multipoint-z",
        Shape::NullShape => "",
        Shape::Multipatch(_) => "unsupported",
    }
}

/// Le righe scritte dopo un lotto, o il rifiuto se il conteggio non ci sta.
///
/// Estratta da `ShpWriter::write` per due ragioni, e la seconda conta piu'
/// della prima. La prima: dove stava non era provabile senza un lotto da piu'
/// di `u64::MAX` righe, mentre qui il conteggio e' un argomento. La seconda: il
/// calcolo era **dopo** il ciclo che scrive le shape, quindi un rifiuto sarebbe
/// arrivato a scrittura fatta. Lo staging viene comunque buttato, e nessun dato
/// esce; ma un limite che si constata invece di fermare e' un limite piu'
/// debole, e adesso il conteggio precede la scrittura.
///
/// # Errors
///
/// [`PlenoraIoError`] con categoria `ResourceLimit` se la cardinalita' del
/// lotto non entra in `u64`, o se la somma con le righe gia' scritte trabocca.
fn righe_dopo_il_lotto(gia_scritte: u64, nel_lotto: usize) -> Result<u64> {
    let troppe =
        || PlenoraIoError::limite_redatto(&PublicMessage::Curated("troppe righe Shapefile"));
    let nel_lotto = u64::try_from(nel_lotto).map_err(|_| troppe())?;
    gia_scritte.checked_add(nel_lotto).ok_or_else(troppe)
}

/// I byte dello staging, sommati senza traboccare.
///
/// Estratta da `ShpWriter::finish` per la stessa ragione: dove stava, la somma
/// prendeva le dimensioni dal filesystem, e provarne l'overflow avrebbe
/// richiesto uno staging da piu' di `u64::MAX` byte. Qui le dimensioni sono
/// l'argomento, e l'aritmetica si prova con dei numeri.
///
/// # Errors
///
/// [`PlenoraIoError`] con categoria `ResourceLimit` se la somma trabocca.
fn byte_dello_staging(dimensioni: impl IntoIterator<Item = u64>) -> Result<u64> {
    dimensioni.into_iter().try_fold(0_u64, |totale, byte| {
        totale.checked_add(byte).ok_or_else(|| {
            PlenoraIoError::limite_redatto(&PublicMessage::Curated(
                "overflow nel conteggio dell'output Shapefile",
            ))
        })
    })
}

/// Scrive la shape come tipo ESRI concreto (l'enum `Shape` non è `EsriShape`).
fn write_shape(w: &mut Writer<BufWriter<File>>, shape: Shape, rec: &Record) -> Result<()> {
    let me = |_| {
        err(&PublicMessage::Curated(
            "scrittura di un record shapefile fallita",
        ))
    };
    match shape {
        Shape::Point(s) => w.write_shape_and_record(&s, rec).map_err(me),
        Shape::PointM(s) => w.write_shape_and_record(&s, rec).map_err(me),
        Shape::PointZ(s) => w.write_shape_and_record(&s, rec).map_err(me),
        Shape::Polyline(s) => w.write_shape_and_record(&s, rec).map_err(me),
        Shape::PolylineM(s) => w.write_shape_and_record(&s, rec).map_err(me),
        Shape::PolylineZ(s) => w.write_shape_and_record(&s, rec).map_err(me),
        Shape::Polygon(s) => w.write_shape_and_record(&s, rec).map_err(me),
        Shape::PolygonM(s) => w.write_shape_and_record(&s, rec).map_err(me),
        Shape::PolygonZ(s) => w.write_shape_and_record(&s, rec).map_err(me),
        Shape::Multipoint(s) => w.write_shape_and_record(&s, rec).map_err(me),
        Shape::MultipointM(s) => w.write_shape_and_record(&s, rec).map_err(me),
        Shape::MultipointZ(s) => w.write_shape_and_record(&s, rec).map_err(me),
        Shape::NullShape => Err(err(&PublicMessage::Curated(
            "geometria nulla non supportata in scrittura Shapefile",
        ))),
        Shape::Multipatch(_) => Err(err(&PublicMessage::Curated(
            "Multipatch non supportato in scrittura Shapefile",
        ))),
    }
}

fn cell_to_field(array: &ArrayRef, row: usize, kind: DbfKind) -> Result<FieldValue> {
    let v = json_from_array(array, row)?;
    Ok(match kind {
        DbfKind::Char => FieldValue::Character(match v {
            JsonValue::Null => None,
            JsonValue::String(s) => Some(s),
            other => Some(other.to_string()),
        }),
        DbfKind::Int | DbfKind::Float => FieldValue::Numeric(v.as_f64()),
        DbfKind::Logical => FieldValue::Logical(v.as_bool()),
    })
}

fn wkt_for_id(id: Option<&str>) -> Option<String> {
    id.filter(|id| CRS_CON_DEFINIZIONE_SINTETIZZATA.contains(id))
        .map(|_| WGS84_WKT.to_owned())
}

fn resolve_prj(
    layer: &plenora_io_core::WriteLayer,
    schema: &Schema,
    geom_idx: usize,
) -> Option<String> {
    if let Some(g) = &layer.contract.geometry {
        if let Some(def) = g.crs.definition() {
            return Some(def.to_owned());
        }
        if let Some(wkt) = wkt_for_id(g.crs.id()) {
            return Some(wkt);
        }
    }
    let id = schema
        .field(geom_idx)
        .metadata()
        .get(GEO_CRS_KEY)
        .map(String::as_str);
    wkt_for_id(id)
}

// --- lettura: helpers ------------------------------------------------------

fn resolve_crs(path: &Path, opts: &ReadOptions) -> Result<ResolvedCrs> {
    let prj = path.with_extension("prj");
    if let Ok(wkt) = std::fs::read_to_string(&prj) {
        let id = opts
            .assume_crs
            .clone()
            .or_else(|| authority_id_from_wkt(&wkt));
        let Some(id) = id else {
            let raw = RawCrs::new(wkt, None);
            return Err(PlenoraIoError::crs_non_risolto_redatto("shp", &raw));
        };
        let kind = crs_kind(&id, Some(&wkt));
        return Ok(ResolvedCrs::new(Some(id), kind, Some(wkt)));
    }
    opts.assume_crs.as_ref().map_or_else(
        || {
            Err(PlenoraIoError::crs_redatto(&PublicMessage::Curated(
                "Shapefile senza .prj: fornire --assume-crs",
            )))
        },
        |id| Ok(ResolvedCrs::new(Some(id.clone()), crs_kind(id, None), None)),
    )
}

fn resolved_crs_id(crs: &ResolvedCrs) -> Result<&str> {
    crs.id.as_deref().ok_or_else(|| {
        PlenoraIoError::crs_redatto(&PublicMessage::Curated(
            "Shapefile: CRS risolto senza identificatore; vietato inventare un'etichetta Arrow",
        ))
    })
}

fn authority_id_from_wkt(wkt: &str) -> Option<String> {
    let upper = wkt.to_ascii_uppercase();
    if upper.trim() == "OGC:CRS84" {
        return Some("OGC:CRS84".to_owned());
    }
    // Il writer Shapefile emette questa forma ESRI WKT1 canonica, che non
    // contiene AUTHORITY ma identifica senza ambiguità WGS 84.
    if upper.contains("GEOGCS[\"WGS 84\"") && upper.contains("DATUM[\"WGS_1984\"") {
        return Some("EPSG:4326".to_owned());
    }
    let epsg = upper.rfind("\"EPSG\"")?;
    let tail = &upper[epsg + "\"EPSG\"".len()..];
    let start = tail.find(char::is_numeric)?;
    let code: String = tail[start..]
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    (!code.is_empty()).then(|| format!("EPSG:{code}"))
}

fn crs_kind(id: &str, definition: Option<&str>) -> CrsKind {
    let definition = definition.unwrap_or_default().to_ascii_uppercase();
    if definition.contains("PROJCS[")
        || definition.contains("PROJCRS[")
        || id.eq_ignore_ascii_case("EPSG:3857")
    {
        CrsKind::Projected
    } else if id.eq_ignore_ascii_case("OGC:CRS84")
        || id.eq_ignore_ascii_case("EPSG:4326")
        || definition.contains("GEOGCS[")
        || definition.contains("GEOGCRS[")
    {
        CrsKind::Geographic
    } else {
        CrsKind::Unknown
    }
}

fn dbf_numeric_integer_precision_unverifiable(value: &FieldValue) -> bool {
    matches!(
        value,
        FieldValue::Numeric(Some(number))
            if number.is_finite()
                && number.fract() == 0.0
                && number.abs() >= FIRST_F64_INTEGER_WITHOUT_UNIT_PRECISION
    )
}

/// Classe dbf per l'inferenza (Numeric/Double/Float=numero, Integer=int).
const fn classify(v: &FieldValue) -> ObservedValueClass {
    match v {
        FieldValue::Integer(_) => ObservedValueClass::Integer,
        FieldValue::Numeric(Some(_)) | FieldValue::Double(_) | FieldValue::Float(Some(_)) => {
            ObservedValueClass::Number
        }
        FieldValue::Logical(Some(_)) => ObservedValueClass::Boolean,
        FieldValue::Character(Some(_)) | FieldValue::Date(Some(_)) => ObservedValueClass::Text,
        _ => ObservedValueClass::Null,
    }
}

#[derive(Clone, Debug)]
struct DbfFieldLayout {
    name: String,
    field_type: u8,
    offset: usize,
    width: usize,
    exact_integer_slot: Option<usize>,
}

#[derive(Clone, Debug)]
struct DbfLayout {
    header_length: usize,
    record_length: usize,
    record_count: u32,
    fields: Vec<DbfFieldLayout>,
    exact_integer_count: usize,
}

/// Legge la parte strutturale del DBF che `dbase::Record` non espone.
///
/// Due proprieta' dipendono dai descrittori originali: i nomi duplicati devono
/// essere respinti prima che `Record` li comprima in una `HashMap`, e un campo
/// Numeric largo, senza decimali, deve essere letto dal testo ASCII originale
/// anziche' dal `f64` gia' arrotondato dalla dipendenza.
/// Legge i descrittori di campo del DBF e ne verifica nomi e larghezze.
///
/// I nomi dei campi **vengono dal file**, quindi non entrano nei messaggi:
/// escono gli indici, che sono prodotti da questa enumerazione. Un nome vuoto,
/// un duplicato dopo la normalizzazione ASCII o una larghezza zero fanno
/// fallire la lettura invece di far perdere silenziosamente una colonna.
fn leggi_descrittori_dbf(
    reader: &mut impl Read,
    decoded_names: Vec<String>,
    field_count: usize,
) -> Result<(Vec<DbfFieldLayout>, usize, usize)> {
    let mut fields = Vec::with_capacity(field_count);
    let mut seen = BTreeSet::new();
    let mut offset = 1_usize; // deletion flag
    let mut exact_integer_count = 0_usize;
    for (index, decoded_name) in decoded_names.into_iter().enumerate() {
        let mut descriptor = [0_u8; DBF_FIELD_DESCRIPTOR_SIZE];
        reader.read_exact(&mut descriptor).map_err(|_| {
            err(&PublicMessage::Curated(
                "descrittore di campo DBF incompleto",
            ))
        })?;
        let name = decoded_name;
        if name.is_empty() {
            return Err(err(&PublicMessage::CuratedWith(
                "nome campo DBF vuoto, indice",
                NumeroStrutturale::Indice(driver_common::saturating_u64(index)),
            )));
        }
        let normalized = name.to_ascii_uppercase();
        if !seen.insert(normalized) {
            // Il nome non esce: e' letto dal DBF. Esce l'indice, che e'
            // prodotto dalla nostra enumerazione dei descrittori.
            return Err(err(&PublicMessage::CuratedWith(
                "nomi campo DBF duplicati; il file e' rifiutato per non perdere una colonna, \
                 secondo indice",
                NumeroStrutturale::Indice(driver_common::saturating_u64(index)),
            )));
        }
        let width = usize::from(descriptor[16]);
        if width == 0 {
            return Err(err(&PublicMessage::CuratedWith(
                "campo DBF con larghezza zero, indice",
                NumeroStrutturale::Indice(driver_common::saturating_u64(index)),
            )));
        }
        let exact_integer_slot = (descriptor[11] == b'N' && descriptor[17] == 0 && width >= 10)
            .then(|| {
                let slot = exact_integer_count;
                exact_integer_count += 1;
                slot
            });
        fields.push(DbfFieldLayout {
            name,
            field_type: descriptor[11],
            offset,
            width,
            exact_integer_slot,
        });
        offset = offset.checked_add(width).ok_or_else(|| {
            err(&PublicMessage::Curated(
                "overflow nella lunghezza record DBF",
            ))
        })?;
    }
    Ok((fields, offset, exact_integer_count))
}

/// I valori dichiarati dentro un record, e quanto spazio pretendono.
///
/// Il layout Shapefile ha tre sole forme: nessun conteggio (i punti singoli),
/// il solo numero di punti (i multipunto), il numero di parti **e** quello di
/// punti (polilinee, poligoni, multipatch). I conteggi stanno sempre agli
/// stessi scostamenti, dopo il tag e il riquadro, e questo permette di
/// verificarli senza riscrivere il decoder di quattordici tipi.
enum ConteggiDelRecord {
    Nessuno,
    SoloPunti,
    PartiEPunti,
}

/// Lo scostamento del primo conteggio: un `i32` di tipo piu' quattro `f64` di
/// riquadro.
const SHP_SCOSTAMENTO_CONTEGGI: usize = 4 + 8 * 4;

/// Quanto basta leggere di un record per verificarne i conteggi: il tag, il
/// riquadro e i due conteggi. Il resto lo legge il decoder.
const SHP_TESTA_DEL_CONTENUTO: usize = SHP_SCOSTAMENTO_CONTEGGI + 8;
const SHP_TESTA_DEL_CONTENUTO_BYTE: i64 = 44;
/// Lo stesso scostamento in `i64`: la testa fissa che precede i conteggi.
const SHP_SCOSTAMENTO_CONTEGGI_BYTE: i64 = 36;

const fn conteggi_attesi(tag: i32) -> ConteggiDelRecord {
    match tag {
        // Multipoint, MultipointZ, MultipointM.
        8 | 18 | 28 => ConteggiDelRecord::SoloPunti,
        // Polyline, Polygon e le loro varianti Z e M, piu' Multipatch.
        3 | 5 | 13 | 15 | 23 | 25 | 31 => ConteggiDelRecord::PartiEPunti,
        // Due casi con la stessa risposta, e vale la pena dire quali sono.
        // NullShape, Point, PointM e PointZ hanno dimensione fissa e non
        // dichiarano conteggi. Un tag che non conosciamo lo rifiuta il decoder
        // con un errore tipizzato: pretendere qui di sapere che cosa contenga
        // sarebbe indovinare, e rifiutarlo di qui toglierebbe al fuzzer un ramo
        // di errore legittimo.
        _ => ConteggiDelRecord::Nessuno,
    }
}

/// Il record dichiara piu' elementi di quanti ne stiano nel record stesso?
///
/// `shapefile` prenota `Vec::with_capacity(num_points as usize)` **prima** di
/// leggere i punti, e non lega quel numero alla dimensione del record: un
/// record da cento byte che ne dichiara due miliardi fa tentare una
/// prenotazione da decine di gigabyte, e il processo muore per allocazione
/// fallita invece di rifiutare il file. Un conteggio negativo diventa poi, via
/// `as usize`, un numero enorme.
///
/// Il limite qui e' una condizione **necessaria**, non la dimensione esatta:
/// ogni parte occupa almeno quattro byte e ogni punto almeno sedici. Basta a
/// legare cio' che viene prenotato a cio' che il file contiene davvero, e non
/// richiede di conoscere il layout dei quattordici tipi.
fn conteggi_del_record(contenuto: &[u8], byte_del_record: i64) -> Result<Option<(i64, i64)>> {
    let Some(tag) = contenuto.get(..4) else {
        return Ok(None);
    };
    let tag = i32::from_le_bytes([tag[0], tag[1], tag[2], tag[3]]);

    let leggi = |scostamento: usize| -> Option<i64> {
        let campo = contenuto.get(scostamento..scostamento + 4)?;
        Some(i64::from(i32::from_le_bytes([
            campo[0], campo[1], campo[2], campo[3],
        ])))
    };
    // La testa fissa fa parte del record, e la sua dimensione dipende da quanti
    // conteggi il tipo dichiara: quaranta byte per un multipunto, quarantaquattro
    // per una polilinea. Contarla e' la differenza fra «gli elementi ci stanno»
    // e «gli elementi ci stanno **dopo** cio' che li precede»: senza, un record
    // da quarantaquattro byte che dichiara una parte passava, e la lettura
    // dell'indice delle parti usciva dal record.
    let (testa, parti, punti) = match conteggi_attesi(tag) {
        ConteggiDelRecord::Nessuno => return Ok(None),
        ConteggiDelRecord::SoloPunti => (
            SHP_SCOSTAMENTO_CONTEGGI_BYTE + 4,
            Some(0),
            leggi(SHP_SCOSTAMENTO_CONTEGGI),
        ),
        ConteggiDelRecord::PartiEPunti => (
            SHP_SCOSTAMENTO_CONTEGGI_BYTE + 8,
            leggi(SHP_SCOSTAMENTO_CONTEGGI),
            leggi(SHP_SCOSTAMENTO_CONTEGGI + 4),
        ),
    };
    // Un record troppo corto per portare i propri conteggi va **rifiutato**, non
    // lasciato passare.
    //
    // `read_shape_content` riceve la dimensione del record ma legge dal flusso:
    // se i conteggi non stanno dentro il record, li legge dai byte che seguono,
    // cioe' dal record successivo o da quel che c'e'. Un `Vec::with_capacity`
    // grande quanto quel numero e' l'unica cosa che poi succede -- e' cosi' che
    // una campagna ha chiesto quattro gigabyte per un file da trecento byte.
    let (Some(parti), Some(punti)) = (parti, punti) else {
        return Err(err(&PublicMessage::Curated(
            "record Shapefile troppo corto per il tipo che dichiara",
        )));
    };

    if parti < 0 || punti < 0 {
        return Err(err(&PublicMessage::Curated(
            "conteggio negativo dichiarato in un record Shapefile",
        )));
    }
    let richiesti = punti
        .checked_mul(SHP_BYTE_PER_PUNTO)
        .and_then(|byte| byte.checked_add(parti.checked_mul(SHP_BYTE_PER_PARTE)?))
        .and_then(|byte| byte.checked_add(testa));
    match richiesti {
        Some(byte) if byte <= byte_del_record => Ok(Some((parti, punti))),
        _ => Err(err(&PublicMessage::Curated(
            "record Shapefile che dichiara piu' elementi di quanti ne contenga",
        ))),
    }
}

/// L'indice delle parti: dove comincia ciascuna, in numeri di punto.
///
/// `PartIndexIter` prende la differenza fra due voci consecutive e la passa a
/// `read_xy_in_vec_of` come numero di punti da leggere. Un indice che scende
/// rende quella differenza negativa -- e c'e' un `debug_assert!` che lo dice,
/// cioe' un panico sotto il fuzzer e niente in release, dove il numero negativo
/// diventa enorme passando da `as usize`. Un indice che sale oltre il numero di
/// punti dichiarato produce lo stesso effetto senza nemmeno l'asserzione.
///
/// La spec vuole l'indice non decrescente e dentro il numero di punti; qui si
/// pretende esattamente quello.
fn valida_indice_delle_parti(parti: &[u8], punti: i64) -> Result<()> {
    let mut precedente = 0_i64;
    for voce in parti.chunks_exact(4) {
        let inizio = i64::from(i32::from_le_bytes([voce[0], voce[1], voce[2], voce[3]]));
        if inizio < precedente || inizio > punti {
            return Err(err(&PublicMessage::Curated(
                "indice delle parti Shapefile che esce dai punti dichiarati",
            )));
        }
        precedente = inizio;
    }
    Ok(())
}

/// Legge dal record quel tanto che serve a verificarlo, e dice quanti byte ha
/// consumato: il chiamante salta il resto, che e' cio' che legge il decoder.
fn valida_contenuto_del_record(lettore: &mut BufReader<File>, byte_del_record: i64) -> Result<i64> {
    let da_leggere = byte_del_record.min(SHP_TESTA_DEL_CONTENUTO_BYTE);
    // La stessa limatura in `usize`. Non e' un ripiego: il valore e' gia'
    // limitato a quarantaquattro byte, e su una piattaforma dove `i64` non ci
    // stesse la risposta giusta resterebbe quel massimo.
    let quanti = usize::try_from(da_leggere).map_or(SHP_TESTA_DEL_CONTENUTO, |byte| {
        byte.min(SHP_TESTA_DEL_CONTENUTO)
    });
    let mut testa = [0_u8; SHP_TESTA_DEL_CONTENUTO];
    lettore.read_exact(&mut testa[..quanti]).map_err(|_| {
        err(&PublicMessage::Curated(
            "record Shapefile troncato dentro il proprio contenuto",
        ))
    })?;

    let Some((parti, punti)) = conteggi_del_record(&testa[..quanti], byte_del_record)? else {
        return Ok(da_leggere);
    };
    if parti == 0 {
        return Ok(da_leggere);
    }

    // `parti * 4` non supera la dimensione del record: lo ha appena verificato
    // `conteggi_del_record`, ed e' cio' che rende questa lettura limitata dal
    // file invece che da un numero dichiarato.
    let byte_delle_parti = parti * SHP_BYTE_PER_PARTE;
    let Ok(dimensione_indice) = usize::try_from(byte_delle_parti) else {
        return Err(err(&PublicMessage::Curated(
            "record Shapefile che dichiara piu' elementi di quanti ne contenga",
        )));
    };
    let mut indice = vec![0_u8; dimensione_indice];
    lettore.read_exact(&mut indice).map_err(|_| {
        err(&PublicMessage::Curated(
            "record Shapefile troncato dentro il proprio contenuto",
        ))
    })?;
    valida_indice_delle_parti(&indice, punti)?;
    Ok(da_leggere + byte_delle_parti)
}

/// I byte utili dichiarati dall'header di un `.shp` o di un `.shx`.
fn byte_utili_dichiarati(intestazione: &[u8; SHP_HEADER_SIZE], dimensione: u64) -> Result<i64> {
    let dichiarate = i64::from(i32::from_be_bytes([
        intestazione[24],
        intestazione[25],
        intestazione[26],
        intestazione[27],
    ]));
    if !(SHP_HEADER_PAROLE..=SHP_MAX_PAROLE).contains(&dichiarate) {
        return Err(err(&PublicMessage::Curated(
            "lunghezza dichiarata nell'header Shapefile fuori intervallo",
        )));
    }
    let byte = dichiarate * 2;
    // Un file piu' grande di `i64::MAX` byte non esiste su nessun filesystem
    // che ci interessa; se esistesse, sarebbe comunque piu' grande di qualunque
    // lunghezza dichiarabile in `i32`, quindi il confronto passerebbe.
    if let Ok(reale) = i64::try_from(dimensione) {
        if byte > reale {
            return Err(err(&PublicMessage::Curated(
                "header Shapefile che dichiara piu' byte di quanti il file ne abbia",
            )));
        }
    }
    Ok(byte)
}

/// La catena dei record del `.shp`, e i conteggi che ciascuno dichiara.
fn valida_geometrie_shp(path: &Path) -> Result<i64> {
    let file =
        File::open(path).map_err(|_| err(&PublicMessage::Curated("shapefile non valido")))?;
    let dimensione = file
        .metadata()
        .map_err(|_| err(&PublicMessage::Curated("shapefile non valido")))?
        .len();
    let mut lettore = BufReader::new(file);
    let mut intestazione = [0_u8; SHP_HEADER_SIZE];
    lettore
        .read_exact(&mut intestazione)
        .map_err(|_| err(&PublicMessage::Curated("header Shapefile incompleto")))?;
    let byte_utili = byte_utili_dichiarati(&intestazione, dimensione)?;

    let mut posizione = SHP_HEADER_BYTE;
    while posizione < byte_utili {
        let mut testa_del_record = [0_u8; 8];
        lettore.read_exact(&mut testa_del_record).map_err(|_| {
            err(&PublicMessage::Curated(
                "record Shapefile troncato prima della propria testa",
            ))
        })?;
        let parole = i64::from(i32::from_be_bytes([
            testa_del_record[4],
            testa_del_record[5],
            testa_del_record[6],
            testa_del_record[7],
        ]));
        if !(SHP_MIN_PAROLE_DI_RECORD..=SHP_MAX_PAROLE).contains(&parole) {
            return Err(err(&PublicMessage::Curated(
                "lunghezza di un record Shapefile fuori intervallo",
            )));
        }
        let byte_del_record = parole * 2;
        if posizione + SHP_RECORD_HEADER_BYTE + byte_del_record > byte_utili {
            return Err(err(&PublicMessage::Curated(
                "record Shapefile che esce dai byte dichiarati nell'header",
            )));
        }

        // Solo la testa del contenuto: tag, riquadro e conteggi. Il resto lo
        // legge il decoder, ed e' cio' che questa verifica non deve rifare.
        let letti = valida_contenuto_del_record(&mut lettore, byte_del_record)?;
        let saltati = byte_del_record - letti;
        if saltati > 0 {
            lettore
                .seek_relative(saltati)
                .map_err(|_| err(&PublicMessage::Curated("shapefile non valido")))?;
        }
        posizione += SHP_RECORD_HEADER_BYTE + byte_del_record;
    }
    Ok(byte_utili)
}

/// La testa di un record, letta **dove l'indice manda**.
///
/// Verificare la catena sequenziale non basta quando c'e' un `.shx`: il lettore
/// non la percorre, cerca. Uno scostamento che regge il raddoppio e sta dentro
/// il file puo' comunque puntare in mezzo al contenuto di un altro record, dove
/// otto byte qualunque diventano una testa di record e la lunghezza che ne
/// esce torna a traboccare al raddoppio. E' il difetto che il target ha trovato
/// dopo la prima correzione, ed e' la ragione per cui la verifica dell'indice
/// legge il `.shp`.
fn valida_record_indicizzato(
    lettore: &mut BufReader<File>,
    scostamento: i64,
    parole_dichiarate: i64,
) -> Result<()> {
    let Ok(posizione) = u64::try_from(scostamento * 2) else {
        return Err(err(&PublicMessage::Curated(
            "voce dell'indice Shapefile fuori intervallo",
        )));
    };
    lettore
        .seek(SeekFrom::Start(posizione))
        .map_err(|_| err(&PublicMessage::Curated("shapefile non valido")))?;

    let mut testa = [0_u8; 8];
    lettore.read_exact(&mut testa).map_err(|_| {
        err(&PublicMessage::Curated(
            "record Shapefile troncato prima della propria testa",
        ))
    })?;
    let parole = i64::from(i32::from_be_bytes([testa[4], testa[5], testa[6], testa[7]]));
    if parole != parole_dichiarate {
        // La lunghezza sta scritta due volte, nell'indice e nel record. Quando
        // divergono, il lettore ne usa una per cercare e l'altra per leggere.
        return Err(err(&PublicMessage::Curated(
            "lunghezza del record diversa da quella dichiarata nell'indice",
        )));
    }

    valida_contenuto_del_record(lettore, parole * 2).map(|_| ())
}

/// L'indice e' facoltativo: senza, `shapefile` legge i record in sequenza. Con,
/// ne usa gli scostamenti per cercare -- e li raddoppia dentro un `i32`.
fn valida_indice_shx(path: &Path, shp_path: &Path, byte_utili_shp: i64) -> Result<()> {
    let Ok(file) = File::open(path) else {
        return Ok(());
    };
    let dimensione = file
        .metadata()
        .map_err(|_| err(&PublicMessage::Curated("indice Shapefile non valido")))?
        .len();
    let mut lettore = BufReader::new(file);
    let mut intestazione = [0_u8; SHP_HEADER_SIZE];
    lettore
        .read_exact(&mut intestazione)
        .map_err(|_| err(&PublicMessage::Curated("header dell'indice incompleto")))?;
    let byte_utili = byte_utili_dichiarati(&intestazione, dimensione)?;

    if (byte_utili - SHP_HEADER_BYTE) % SHX_RECORD_BYTE != 0 {
        return Err(err(&PublicMessage::Curated(
            "indice Shapefile di lunghezza non multipla di una voce",
        )));
    }

    // Il `.shp` si apre una volta sola: ogni voce dell'indice ci fa una ricerca,
    // e riaprirlo a ogni voce trasformerebbe una verifica in un costo.
    let mut geometrie = BufReader::new(
        File::open(shp_path).map_err(|_| err(&PublicMessage::Curated("shapefile non valido")))?,
    );

    let mut posizione = SHP_HEADER_BYTE;
    while posizione < byte_utili {
        let mut grezza = [0_u8; 8];
        lettore
            .read_exact(&mut grezza)
            .map_err(|_| err(&PublicMessage::Curated("voce dell'indice troncata")))?;
        let scostamento = i64::from(i32::from_be_bytes([
            grezza[0], grezza[1], grezza[2], grezza[3],
        ]));
        let parole = i64::from(i32::from_be_bytes([
            grezza[4], grezza[5], grezza[6], grezza[7],
        ]));
        if !(SHP_HEADER_PAROLE..=SHP_MAX_PAROLE).contains(&scostamento)
            || !(SHP_MIN_PAROLE_DI_RECORD..=SHP_MAX_PAROLE).contains(&parole)
        {
            return Err(err(&PublicMessage::Curated(
                "voce dell'indice Shapefile fuori intervallo",
            )));
        }
        // Lo scostamento e' in parole e punta alla **testa** del record: il
        // record occupa quattro parole di testa piu' la propria lunghezza.
        if (scostamento + 4 + parole) * 2 > byte_utili_shp {
            return Err(err(&PublicMessage::Curated(
                "voce dell'indice Shapefile che punta fuori dal .shp",
            )));
        }
        valida_record_indicizzato(&mut geometrie, scostamento, parole)?;
        posizione += SHX_RECORD_BYTE;
    }
    Ok(())
}

/// La struttura di `.shp` e `.shx`, verificata **prima** di consegnarli a
/// `shapefile`.
///
/// La crate tratta i valori dichiarati nel file come se venissero da un file
/// che ha scritto lei: moltiplica per due lunghezze `i32` senza controllo,
/// prenota vettori grandi quanto un conteggio dichiarato, e usa gli
/// scostamenti dell'indice per cercare dentro il `.shp`. Su un file ostile
/// l'esito e' un panico o un'allocazione fallita -- e sotto `libfuzzer-sys` un
/// panico e' un `abort()` che nessun `catch_unwind` vede.
///
/// Non e' una convalida semantica: un file che passa di qui puo' ancora essere
/// rifiutato dal decoder, ed e' giusto cosi'. Serve a garantire che il rifiuto
/// sia un `Err`.
fn valida_struttura_shp(shp_path: &Path) -> Result<()> {
    let byte_utili = valida_geometrie_shp(shp_path)?;
    valida_indice_shx(&shp_path.with_extension("shx"), shp_path, byte_utili)
}

/// La lunghezza minima che `dbase` legge da un campo, per tipo.
///
/// La crate dichiara queste dimensioni in `FieldType::size()` e **non le
/// verifica**: legge comunque `field_bytes[0]` da un logico, o
/// `field_bytes[..4]` da un intero. Un descrittore che dichiara meno byte fa
/// uscire l'indice dalla fetta, e la fetta panica.
const fn lunghezza_minima_del_campo(tipo: u8) -> Option<usize> {
    match tipo {
        b'L' => Some(1),
        b'I' => Some(4),
        // Date: otto cifre. Currency, Double e DateTime: otto byte.
        b'D' | b'Y' | b'B' | b'T' => Some(8),
        _ => None,
    }
}

/// La parte "utile" di un campo, come la ritaglia `dbase`: si ferma al primo
/// NUL e scarta gli spazi ai due capi.
fn parte_utile_del_campo(byte: &[u8]) -> &[u8] {
    let fino_al_nul = byte.iter().position(|b| *b == 0).unwrap_or(byte.len());
    let dati = &byte[..fino_al_nul];
    let inizio = dati.iter().position(|b| *b != b' ').unwrap_or(dati.len());
    let fine = dati
        .iter()
        .rposition(|b| *b != b' ')
        .map_or(inizio, |i| i + 1);
    &dati[inizio..fine]
}

/// Un campo `D` che `dbase` non riesce a interpretare **senza panicare**.
///
/// `Date::from_str` affetta la stringa a byte -- `s[0..4]`, `s[4..6]`,
/// `s[6..8]` -- senza guardare ne' la lunghezza ne' i confini di carattere. Un
/// campo che porta meno di otto byte utili esce dall'intervallo; uno che porta
/// un carattere multibyte cade dentro di esso. Entrambi sono panici, e nessuno
/// dei due e' un errore di parsing che la crate restituirebbe.
fn data_non_interpretabile(byte: &[u8]) -> bool {
    let utile = parte_utile_del_campo(byte);
    if utile.is_empty() {
        // Tutto spazi: `dbase` la legge come data assente.
        return false;
    }
    !utile.is_ascii() || utile.len() < 8
}

/// Il giorno giuliano piu' grande che `dbase` sa convertire senza traboccare.
///
/// `julian_day_number_to_gregorian_date` lavora in `i32` e comincia con
/// `4 * jdn + 274_277`: bastano numeri ben piu' piccoli di `i32::MAX` per far
/// traboccare quel prodotto. Invece di ricostruire la soglia esatta di
/// un'aritmetica altrui, si pretende un giorno che esista: `5_373_484` e' il
/// 31 dicembre 9999, e oltre non c'e' data da leggere.
const DBF_MASSIMO_GIORNO_GIULIANO: i32 = 5_373_484;
/// I millisecondi di un giorno. `Time::from_word` divide e rimoltiplica il
/// parola-tempo passando da `u32`: un valore negativo diventa enorme e il
/// prodotto trabocca.
const DBF_MILLISECONDI_DEL_GIORNO: i32 = 86_400_000;

/// Un campo `T` che `dbase` non riesce a convertire **senza traboccare**.
fn data_e_ora_non_convertibili(byte: &[u8]) -> bool {
    let Some(grezzo) = byte.get(..8) else {
        // Il descrittore ne pretende otto: se non ci sono, a rifiutare e' il
        // controllo sulla larghezza del campo, non questo.
        return false;
    };
    let giorno = i32::from_le_bytes([grezzo[0], grezzo[1], grezzo[2], grezzo[3]]);
    let ora = i32::from_le_bytes([grezzo[4], grezzo[5], grezzo[6], grezzo[7]]);
    !(0..=DBF_MASSIMO_GIORNO_GIULIANO).contains(&giorno)
        || !(0..DBF_MILLISECONDI_DEL_GIORNO).contains(&ora)
}

/// I panici di `dbase`, chiusi **prima** di arrivarci.
///
/// `dbase` ricava il numero di campi dall'intestazione con
/// `(offset_to_first_record - 32 - 1) / 32`, e per i file Visual `FoxPro` sottrae
/// prima i 263 byte di backlink. Nessuna delle due sottrazioni e' controllata:
///
/// * un `offset_to_first_record` sotto la soglia fa `attempt to subtract with
///   overflow` dove le `debug_assertions` sono attive -- cioe' sotto il fuzzer
///   -- e senza di esse produce per wrap un numero di campi assurdo;
/// * un file dichiarato Visual `FoxPro` con offset sotto i 263 byte incontra un
///   `panic!("Invalid file")` scritto a mano nella crate.
///
/// `read_dbf_layout` faceva gia' entrambi i controlli, con `checked_sub`, ma
/// **dopo** aver costruito il `Reader`: il panico arrivava prima. L'ordine e'
/// tutto, ed e' la ragione per cui questa verifica e' una funzione a se' e non
/// una riga in piu' -- `scripts/check_prevalidazione_decoder.py` pretende che
/// compaia in ogni funzione che apre un `dbase::Reader`, e prima della
/// chiamata.
///
/// E' la stessa famiglia di `valida_file_ipc` e
/// `valida_schema_arrow_incorporato`: un decoder esterno che si fida di un
/// campo del file, e un driver che deve controllarlo prima di consegnarglielo.
/// Trovato dal target `shp_reader` alla sua prima campagna.
fn valida_intestazione_dbf(path: &Path) -> Result<()> {
    let mut file =
        File::open(path).map_err(|_| err(&PublicMessage::Curated("apertura del DBF fallita")))?;
    let mut intestazione = [0_u8; DBF_HEADER_SIZE];
    file.read_exact(&mut intestazione)
        .map_err(|_| err(&PublicMessage::Curated("header DBF incompleto")))?;

    let dichiarato = usize::from(u16::from_le_bytes([intestazione[8], intestazione[9]]));
    let utile = if DBF_VERSIONI_VISUAL_FOXPRO.contains(&intestazione[0]) {
        dichiarato
            .checked_sub(DBF_VISUAL_FOXPRO_BACKLINK_SIZE)
            .ok_or_else(|| {
                err(&PublicMessage::Curated(
                    "header Visual FoxPro piu' corto del backlink",
                ))
            })?
    } else {
        dichiarato
    };
    if utile < DBF_HEADER_SIZE + DBF_HEADER_TERMINATOR_SIZE {
        // Trentatre' byte: l'intestazione piu' il suo terminatore. Sotto questa
        // soglia non c'e' spazio nemmeno per zero descrittori, e la divisione
        // che li conta lavora su una sottrazione negativa.
        return Err(err(&PublicMessage::Curated(
            "offset del primo record DBF piu' corto dell'intestazione",
        )));
    }

    // Il terzo punto in cui `dbase` si ferma invece di tornare: dopo i
    // descrittori pretende il terminatore con un `debug_assert_eq!`. Con le
    // `debug_assertions` attive -- cioe' sotto il fuzzer -- un byte diverso e'
    // un panico; senza, l'asserzione sparisce e il file viene letto come se il
    // terminatore ci fosse. Rifiutarlo qui chiude il panico **e** rende
    // l'esito lo stesso nelle due configurazioni.
    //
    // La posizione si calcola con l'aritmetica di `dbase`, non con la nostra:
    // la sua divisione tronca, e `read_dbf_layout` invece rifiuta i resti. Qui
    // interessa sapere che cosa leggera' **lui**.
    let descrittori =
        (utile - DBF_HEADER_SIZE - DBF_HEADER_TERMINATOR_SIZE) / DBF_FIELD_DESCRIPTOR_SIZE;
    let posizione = DBF_HEADER_SIZE + descrittori * DBF_FIELD_DESCRIPTOR_SIZE;
    file.seek(SeekFrom::Start(driver_common::saturating_u64(posizione)))
        .map_err(|_| err(&PublicMessage::Curated("header DBF incompleto")))?;
    let mut terminatore = [0_u8; DBF_HEADER_TERMINATOR_SIZE];
    file.read_exact(&mut terminatore)
        .map_err(|_| err(&PublicMessage::Curated("header DBF incompleto")))?;
    if terminatore[0] != DBF_HEADER_TERMINATOR {
        return Err(err(&PublicMessage::Curated(
            "terminatore header DBF non valido",
        )));
    }

    let descrittori =
        (utile - DBF_HEADER_SIZE - DBF_HEADER_TERMINATOR_SIZE) / DBF_FIELD_DESCRIPTOR_SIZE;
    let campi = leggi_campi_da_validare(&mut file, descrittori)?;

    // Il passo fra due record e' **calcolato**, non letto: `dbase::File::open`
    // sovrascrive `size_of_record` dell'header con la somma delle larghezze piu'
    // il flag di cancellazione, perche' -- dice il suo commento -- certi
    // produttori non contano quel byte. Leggere qui il valore dichiarato
    // significherebbe scorrere i record con un passo diverso dal suo, e
    // guardare i byte sbagliati.
    //
    // Che il valore dichiarato sia coerente con quello calcolato lo verifica
    // `read_dbf_layout`; qui interessa solo dove si trovano i valori.
    let record = DBF_DELETION_FLAG_SIZE + campi.iter().map(|campo| campo.lunghezza).sum::<usize>();

    if campi.iter().any(|campo| matches!(campo.tipo, b'D' | b'T')) {
        // Il costo di questa scansione lo paga solo chi ha campi temporali:
        // sono gli unici tipi il cui **valore** puo' fermare la crate. Gli
        // altri si fermano al descrittore -- una stringa che non e' un numero
        // e' un errore di parsing, e la crate lo restituisce.
        valida_i_valori_temporali(&mut file, &intestazione, record, &campi)?;
    }
    Ok(())
}

/// Il poco che serve sapere di un campo per dire se `dbase` ci panichera'.
struct CampoDaValidare {
    tipo: u8,
    lunghezza: usize,
    scostamento: usize,
}

fn leggi_campi_da_validare(file: &mut File, quanti: usize) -> Result<Vec<CampoDaValidare>> {
    file.seek(SeekFrom::Start(SHP_HEADER_SIZE_DBF))
        .map_err(|_| err(&PublicMessage::Curated("header DBF incompleto")))?;

    let mut campi = Vec::with_capacity(quanti);
    let mut scostamento = DBF_DELETION_FLAG_SIZE;
    for _ in 0..quanti {
        let mut grezzo = [0_u8; DBF_FIELD_DESCRIPTOR_SIZE];
        file.read_exact(&mut grezzo)
            .map_err(|_| err(&PublicMessage::Curated("descrittori DBF incompleti")))?;
        let tipo = grezzo[11];
        let lunghezza = usize::from(grezzo[16]);
        if let Some(minima) = lunghezza_minima_del_campo(tipo) {
            if lunghezza < minima {
                return Err(err(&PublicMessage::Curated(
                    "campo DBF piu' corto di quanto il suo tipo pretenda",
                )));
            }
        }
        campi.push(CampoDaValidare {
            tipo,
            lunghezza,
            scostamento,
        });
        scostamento += lunghezza;
    }
    Ok(campi)
}

fn valida_i_valori_temporali(
    file: &mut File,
    intestazione: &[u8; DBF_HEADER_SIZE],
    lunghezza_record: usize,
    campi: &[CampoDaValidare],
) -> Result<()> {
    let primo = u64::from(u16::from_le_bytes([intestazione[8], intestazione[9]]));
    let quanti = u32::from_le_bytes([
        intestazione[4],
        intestazione[5],
        intestazione[6],
        intestazione[7],
    ]);
    if lunghezza_record == 0 {
        // Nessun byte per record: non c'e' nessun valore da leggere, e la
        // divisione che seguirebbe non avrebbe senso.
        return Ok(());
    }
    file.seek(SeekFrom::Start(primo))
        .map_err(|_| err(&PublicMessage::Curated("header DBF incompleto")))?;

    let mut lettore = BufReader::new(file);
    let mut record = vec![0_u8; lunghezza_record];
    for _ in 0..quanti {
        if lettore.read_exact(&mut record).is_err() {
            // Il file finisce prima dei record dichiarati: e' un errore che la
            // crate restituisce leggendo, non un panico. Non tocca a questa
            // verifica trasformarlo in un rifiuto diverso.
            return Ok(());
        }
        // I record cancellati **non** si saltano.
        //
        // Qui c'era un `continue`, e sopra il perche': «un record cancellato non
        // viene letto, `dbase` salta i suoi byte senza decodificarne un solo
        // campo». Non e' vero, e non era mai stato verificato. La fuzz smoke ha
        // trovato un `.dbf` il cui unico record e' marcato `*` e il cui campo `D`
        // fa panicare `Date::from_str` -- `end byte index 4 is out of bounds for
        // string of length 1` -- attraversando l'apertura del driver e il
        // drenaggio, cioe' la strada vera e non un ramo del fuzzing.
        //
        // Il resto di quel commento cadeva con la premessa: temeva che validare
        // i cancellati facesse fallire un dataset «che oggi si legge». Un
        // dataset con una data malformata in una riga cancellata oggi non si
        // legge -- panica -- e un rifiuto tipizzato al posto di un panico e' cio'
        // che questa funzione esiste per dare.
        //
        // Resta vero che il lettore fisico del driver riconosce il marcatore e
        // non consegna la riga. Riconoscerlo **dopo** averne decodificato i campi
        // e' un'altra cosa dal non leggerli, ed e' la distinzione che il commento
        // precedente aveva perso.
        for temporale in campi
            .iter()
            .filter(|campo| matches!(campo.tipo, b'D' | b'T'))
        {
            let ultimo = temporale.scostamento + temporale.lunghezza;
            let Some(byte) = record.get(temporale.scostamento..ultimo) else {
                continue;
            };
            if temporale.tipo == b'D' && data_non_interpretabile(byte) {
                return Err(err(&PublicMessage::Curated(
                    "campo data DBF che il lettore non puo' interpretare",
                )));
            }
            if temporale.tipo == b'T' && data_e_ora_non_convertibili(byte) {
                return Err(err(&PublicMessage::Curated(
                    "campo data-e-ora DBF fuori dall'intervallo convertibile",
                )));
            }
        }
    }
    Ok(())
}

/// I due conteggi di descrittori devono coincidere prima di leggerli.
///
/// Uno viene dalla nostra aritmetica sull'header, l'altro da quanti campi
/// `dbase` ha decodificato. Non e' un controllo ridondante: `leggi_descrittori_dbf`
/// scorre **un descrittore per nome decodificato**, quindi se i due numeri
/// divergessero il lettore si fermerebbe a meta' dei trentadue byte di un
/// descrittore, e il controllo sul terminatore che segue leggerebbe un byte
/// qualunque. Il rifiuto qui tiene allineate le due letture prima che si
/// disallineino.
///
/// Estratta dal chiamante perche' la divergenza non e' producibile da un file:
/// dipende da come `dbase` interpreta l'header, cioe' da un contratto con la
/// dipendenza e non da un input. Qui i due conteggi sono argomenti, e la logica
/// del rifiuto si prova senza doverla far accadere.
///
/// # Errors
///
/// [`PlenoraIoError`] se i due conteggi non coincidono. Il numero che esce e'
/// quello **decodificato**, che e' nostro; quello dell'header viene dal file.
fn descrittori_concordi(dichiarati: usize, decodificati: usize) -> Result<()> {
    if dichiarati == decodificati {
        return Ok(());
    }
    Err(err(&PublicMessage::CuratedWith(
        "numero di descrittori DBF incoerente con l'header, descrittori decodificati",
        NumeroStrutturale::Conteggio(driver_common::saturating_u64(decodificati)),
    )))
}

fn read_dbf_layout(shp_path: &Path) -> Result<DbfLayout> {
    let path = shp_path.with_extension("dbf");
    valida_intestazione_dbf(&path)?;
    let decoded_names = shapefile::dbase::Reader::from_path(&path)
        .map_err(|_| err(&PublicMessage::Curated("apertura dello schema DBF fallita")))?
        .fields()
        .iter()
        .map(|field| field.name().to_owned())
        .collect::<Vec<_>>();
    let mut reader = BufReader::new(
        File::open(&path).map_err(|_| err(&PublicMessage::Curated("apertura del DBF fallita")))?,
    );
    let mut header = [0_u8; DBF_HEADER_SIZE];
    reader
        .read_exact(&mut header)
        .map_err(|_| err(&PublicMessage::Curated("header DBF incompleto")))?;
    let record_count = u32::from_le_bytes([header[4], header[5], header[6], header[7]]);
    let header_length = usize::from(u16::from_le_bytes([header[8], header[9]]));
    let declared_record_length = usize::from(u16::from_le_bytes([header[10], header[11]]));
    let descriptor_end = if header[0] == DBF_VISUAL_FOXPRO_VERSION {
        header_length
            .checked_sub(DBF_VISUAL_FOXPRO_BACKLINK_SIZE)
            .ok_or_else(|| {
                err(&PublicMessage::Curated(
                    "header Visual FoxPro piu' corto del backlink",
                ))
            })?
    } else {
        header_length
    };
    let descriptor_bytes = descriptor_end
        .checked_sub(DBF_HEADER_SIZE + DBF_HEADER_TERMINATOR_SIZE)
        .ok_or_else(|| err(&PublicMessage::Curated("lunghezza header DBF non valida")))?;
    if descriptor_bytes % DBF_FIELD_DESCRIPTOR_SIZE != 0 {
        return Err(err(&PublicMessage::Curated(
            "lunghezza descrittori DBF non valida",
        )));
    }

    let field_count = descriptor_bytes / DBF_FIELD_DESCRIPTOR_SIZE;
    descrittori_concordi(field_count, decoded_names.len())?;
    let (fields, offset, exact_integer_count) =
        leggi_descrittori_dbf(&mut reader, decoded_names, field_count)?;
    let mut terminator = [0_u8; 1];
    reader.read_exact(&mut terminator).map_err(|_| {
        err(&PublicMessage::Curated(
            "terminatore dell'header DBF mancante",
        ))
    })?;
    if terminator[0] != DBF_HEADER_TERMINATOR {
        return Err(err(&PublicMessage::Curated(
            "terminatore header DBF non valido",
        )));
    }
    if declared_record_length != offset && declared_record_length.checked_add(1) != Some(offset) {
        // La lunghezza dichiarata viene dal file; quella richiesta dai campi
        // la calcoliamo noi.
        return Err(err(&PublicMessage::CuratedWith(
            "lunghezza di record DBF dichiarata incoerente con i campi, byte richiesti",
            NumeroStrutturale::Conteggio(driver_common::saturating_u64(offset)),
        )));
    }
    Ok(DbfLayout {
        header_length,
        // `dbase` adotta la lunghezza calcolata quando un produttore omette il
        // deletion flag dalla lunghezza dichiarata; il lettore raw deve restare
        // allineato allo stesso comportamento.
        record_length: offset,
        record_count,
        fields,
        exact_integer_count,
    })
}

struct DbfExactIntegerRows {
    reader: BufReader<File>,
    layout: DbfLayout,
    records_read: u32,
    buffer: Vec<u8>,
}

enum DbfPhysicalRow {
    Deleted,
    Active {
        exact_values: Vec<Option<i64>>,
        raw_numeric_key: Option<String>,
        rejection_cause: Option<&'static str>,
    },
}

impl DbfExactIntegerRows {
    fn open(shp_path: &Path, layout: &DbfLayout) -> Result<Self> {
        let mut reader = BufReader::new(
            File::open(shp_path.with_extension("dbf"))
                .map_err(|_| err(&PublicMessage::Curated("apertura del DBF fallita")))?,
        );
        reader
            .seek(SeekFrom::Start(layout.header_length as u64))
            .map_err(|_| {
                err(&PublicMessage::Curated(
                    "posizionamento sui record DBF fallito",
                ))
            })?;
        Ok(Self {
            reader,
            layout: layout.clone(),
            records_read: 0,
            buffer: vec![0_u8; layout.record_length],
        })
    }

    fn next_physical(
        &mut self,
        raw_numeric_field_index: Option<usize>,
    ) -> Result<Option<DbfPhysicalRow>> {
        if self.records_read >= self.layout.record_count {
            return Ok(None);
        }
        self.reader
            .read_exact(&mut self.buffer)
            .map_err(|_| err(&PublicMessage::Curated("record DBF incompleto")))?;
        self.records_read += 1;
        match self.buffer[0] {
            DBF_RECORD_CANCELLATO => return Ok(Some(DbfPhysicalRow::Deleted)),
            b' ' => {}
            _ => {
                // Il byte non esce: e' letto dal payload.
                return Err(err(&PublicMessage::Curated(
                    "marcatore di record DBF non valido",
                )));
            }
        }
        let mut values = vec![None; self.layout.exact_integer_count];
        let mut rejection_cause = None;
        for field in &self.layout.fields {
            let Some(slot) = field.exact_integer_slot else {
                continue;
            };
            let end = field.offset.checked_add(field.width).ok_or_else(|| {
                err(&PublicMessage::Curated(
                    "overflow nell'offset del campo DBF",
                ))
            })?;
            let raw = self
                .buffer
                .get(field.offset..end)
                .ok_or_else(|| err(&PublicMessage::Curated("campo DBF fuori dal record")))?;
            let Ok(text) = std::str::from_utf8(raw) else {
                rejection_cause = Some(ATTRIBUTE_NUMERIC_INVALID_CAUSE);
                continue;
            };
            let text = text.trim();
            if !text.is_empty() {
                match text.parse::<i64>() {
                    Ok(value) => values[slot] = Some(value),
                    Err(_) => rejection_cause = Some(ATTRIBUTE_NUMERIC_INVALID_CAUSE),
                }
            }
        }
        let raw_numeric_key = if let Some(index) = raw_numeric_field_index {
            let field = self.layout.fields.get(index).ok_or_else(|| {
                err(&PublicMessage::Curated(
                    "indice campo chiave DBF fuori schema",
                ))
            })?;
            let end = field.offset.checked_add(field.width).ok_or_else(|| {
                err(&PublicMessage::Curated(
                    "overflow nell'offset della chiave DBF",
                ))
            })?;
            let raw = self
                .buffer
                .get(field.offset..end)
                .ok_or_else(|| err(&PublicMessage::Curated("chiave DBF fuori record")))?;
            std::str::from_utf8(raw).map_or_else(
                |_| {
                    rejection_cause = Some(ATTRIBUTE_NUMERIC_INVALID_CAUSE);
                    None
                },
                |text| {
                    let text = text.trim();
                    (!text.is_empty()).then(|| text.to_owned())
                },
            )
        } else {
            None
        };
        Ok(Some(DbfPhysicalRow::Active {
            exact_values: values,
            raw_numeric_key,
            rejection_cause,
        }))
    }
}

#[derive(Clone)]
struct ShpColumn {
    name: String,
    column_type: ColType,
    exact_integer_slot: Option<usize>,
}

struct ShpGeometryInfo {
    dimensions: CoordinateDimensions,
    geometry_types: Vec<GeometryType>,
    shape_type: Option<&'static str>,
}

struct ShpInference {
    cols: Vec<ShpColumn>,
    dbf_layout: DbfLayout,
    geometry_info: ShpGeometryInfo,
    active_row_count: u64,
    loss: LossReport,
}

trait NativePoint {
    fn x(&self) -> f64;
    fn y(&self) -> f64;
    fn z(&self) -> Option<f64>;
    fn m(&self) -> Option<f64>;
}

impl NativePoint for Point {
    fn x(&self) -> f64 {
        self.x
    }
    fn y(&self) -> f64 {
        self.y
    }
    fn z(&self) -> Option<f64> {
        None
    }
    fn m(&self) -> Option<f64> {
        None
    }
}

impl NativePoint for PointM {
    fn x(&self) -> f64 {
        self.x
    }
    fn y(&self) -> f64 {
        self.y
    }
    fn z(&self) -> Option<f64> {
        None
    }
    fn m(&self) -> Option<f64> {
        Some(self.m)
    }
}

impl NativePoint for PointZ {
    fn x(&self) -> f64 {
        self.x
    }
    fn y(&self) -> f64 {
        self.y
    }
    fn z(&self) -> Option<f64> {
        Some(self.z)
    }
    fn m(&self) -> Option<f64> {
        Some(self.m)
    }
}

fn native_coordinate<P: NativePoint>(
    point: &P,
    dimensions: CoordinateDimensions,
) -> Result<WkbCoordinate> {
    let (z, m) =
        match dimensions {
            CoordinateDimensions::Xy if point.z().is_none() && point.m().is_none() => (None, None),
            CoordinateDimensions::Xym if point.z().is_none() => (
                None,
                Some(point.m().ok_or_else(|| {
                    err(&PublicMessage::Curated("coordinata ShapeM senza misura"))
                })?),
            ),
            CoordinateDimensions::Xyz => {
                let z = point
                    .z()
                    .ok_or_else(|| err(&PublicMessage::Curated("coordinata ShapeZ senza quota")))?;
                if point.m().is_some_and(|measure| {
                    !matches!(
                        measure.partial_cmp(&NO_DATA),
                        Some(std::cmp::Ordering::Less | std::cmp::Ordering::Equal)
                    )
                }) {
                    return Err(err(&PublicMessage::Curated(
                        "misura valida trovata in un dataset ShapeZ dichiarato XYZ",
                    )));
                }
                (Some(z), None)
            }
            CoordinateDimensions::Xyzm => (
                Some(point.z().ok_or_else(|| {
                    err(&PublicMessage::Curated("coordinata ShapeZ senza quota"))
                })?),
                Some(point.m().ok_or_else(|| {
                    err(&PublicMessage::Curated(
                        "coordinata ShapeZ senza misura nativa",
                    ))
                })?),
            ),
            CoordinateDimensions::Unknown => {
                return Err(err(&PublicMessage::Curated(
                    "dimensionalità Shapefile non determinata",
                )))
            }
            _ => {
                return Err(err(&PublicMessage::Curated(
                    "variante Shape incoerente con la dimensionalità del layer",
                )))
            }
        };
    Ok(WkbCoordinate {
        x: point.x(),
        y: point.y(),
        z,
        m,
    })
}

fn native_coordinates<P: NativePoint>(
    points: &[P],
    dimensions: CoordinateDimensions,
) -> Result<Vec<WkbCoordinate>> {
    points
        .iter()
        .map(|point| native_coordinate(point, dimensions))
        .collect()
}

fn polyline_wkb<P: NativePoint>(
    parts: &[Vec<P>],
    dimensions: CoordinateDimensions,
) -> Result<WkbGeometry> {
    let children = parts
        .iter()
        .map(|part| {
            Ok(WkbGeometry {
                value: WkbValue::LineString(native_coordinates(part, dimensions)?),
                dimensions,
                srid: None,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(WkbGeometry {
        value: WkbValue::MultiLineString(children),
        dimensions,
        srid: None,
    })
}

fn polygon_wkb<P: NativePoint>(
    rings: &[PolygonRing<P>],
    dimensions: CoordinateDimensions,
) -> Result<WkbGeometry> {
    let mut polygons = Vec::<WkbGeometry>::new();
    let mut current = None::<Vec<Vec<WkbCoordinate>>>;
    for ring in rings {
        match ring {
            PolygonRing::Outer(points) => {
                if let Some(rings) = current.take() {
                    polygons.push(WkbGeometry {
                        value: WkbValue::Polygon(rings),
                        dimensions,
                        srid: None,
                    });
                }
                current = Some(vec![native_coordinates(points, dimensions)?]);
            }
            PolygonRing::Inner(points) => {
                let current = current.as_mut().ok_or_else(|| {
                    err(&PublicMessage::Curated(
                        "anello interno Shapefile senza anello esterno",
                    ))
                })?;
                current.push(native_coordinates(points, dimensions)?);
            }
        }
    }
    if let Some(rings) = current {
        polygons.push(WkbGeometry {
            value: WkbValue::Polygon(rings),
            dimensions,
            srid: None,
        });
    }
    if polygons.is_empty() {
        return Err(err(&PublicMessage::Curated(
            "Polygon Shapefile senza anelli esterni",
        )));
    }
    Ok(WkbGeometry {
        value: WkbValue::MultiPolygon(polygons),
        dimensions,
        srid: None,
    })
}

// Un anello Shapefile e' chiuso se e solo se primo e ultimo vertice coincidono
// bit a bit: il confronto esatto e' la definizione del formato, una tolleranza
// accetterebbe come chiusi anelli che GDAL e il corpus reale considerano aperti.
#[allow(clippy::float_cmp)]
fn polygon_rejection_cause<P: NativePoint>(rings: &[PolygonRing<P>]) -> Option<&'static str> {
    if rings.is_empty() {
        return Some(POLYGON_WITHOUT_OUTER_CAUSE);
    }
    if rings.iter().any(|ring| {
        let points = ring.points();
        !matches!(
            (points.first(), points.last()),
            (Some(first), Some(last)) if first.x() == last.x() && first.y() == last.y()
        )
    }) {
        return Some(UNCLOSED_RING_CAUSE);
    }
    if rings.iter().any(|ring| {
        let points = ring.points();
        if points.len() < 4 {
            return true;
        }
        // Niente mul_add/FMA: la fusione cambia l'arrotondamento IEEE e
        // romperebbe il determinismo bit-esatto della somma dell'area doppia.
        #[allow(clippy::suboptimal_flops)]
        let twice_area = points.windows(2).fold(0.0, |area, edge| {
            area + edge[0].x() * edge[1].y() - edge[1].x() * edge[0].y()
        });
        !twice_area.is_finite() || twice_area == 0.0
    }) {
        return Some(DEGENERATE_RING_CAUSE);
    }
    let mut has_outer = false;
    for ring in rings {
        match ring {
            PolygonRing::Outer(_) => has_outer = true,
            PolygonRing::Inner(_) if !has_outer => return Some(INNER_RING_WITHOUT_OUTER_CAUSE),
            PolygonRing::Inner(_) => {}
        }
    }
    if !has_outer {
        return Some(POLYGON_WITHOUT_OUTER_CAUSE);
    }
    None
}

fn shape_rejection_cause(shape: &Shape) -> Option<&'static str> {
    match shape {
        Shape::Polygon(polygon) => polygon_rejection_cause(polygon.rings()),
        Shape::PolygonM(polygon) => polygon_rejection_cause(polygon.rings()),
        Shape::PolygonZ(polygon) => polygon_rejection_cause(polygon.rings()),
        _ => None,
    }
}

fn multipoint_wkb<P: NativePoint>(
    points: &[P],
    dimensions: CoordinateDimensions,
) -> Result<WkbGeometry> {
    let children = points
        .iter()
        .map(|point| {
            Ok(WkbGeometry {
                value: WkbValue::Point(native_coordinate(point, dimensions)?),
                dimensions,
                srid: None,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(WkbGeometry {
        value: WkbValue::MultiPoint(children),
        dimensions,
        srid: None,
    })
}

fn shape_to_wkb(shape: &Shape, dimensions: CoordinateDimensions) -> Result<Option<WkbGeometry>> {
    let geometry = match shape {
        Shape::NullShape => return Ok(None),
        Shape::Point(point) => WkbGeometry {
            value: WkbValue::Point(native_coordinate(point, dimensions)?),
            dimensions,
            srid: None,
        },
        Shape::PointM(point) => WkbGeometry {
            value: WkbValue::Point(native_coordinate(point, dimensions)?),
            dimensions,
            srid: None,
        },
        Shape::PointZ(point) => WkbGeometry {
            value: WkbValue::Point(native_coordinate(point, dimensions)?),
            dimensions,
            srid: None,
        },
        Shape::Polyline(polyline) => polyline_wkb(polyline.parts(), dimensions)?,
        Shape::PolylineM(polyline) => polyline_wkb(polyline.parts(), dimensions)?,
        Shape::PolylineZ(polyline) => polyline_wkb(polyline.parts(), dimensions)?,
        Shape::Polygon(polygon) => polygon_wkb(polygon.rings(), dimensions)?,
        Shape::PolygonM(polygon) => polygon_wkb(polygon.rings(), dimensions)?,
        Shape::PolygonZ(polygon) => polygon_wkb(polygon.rings(), dimensions)?,
        Shape::Multipoint(multipoint) => multipoint_wkb(multipoint.points(), dimensions)?,
        Shape::MultipointM(multipoint) => multipoint_wkb(multipoint.points(), dimensions)?,
        Shape::MultipointZ(multipoint) => multipoint_wkb(multipoint.points(), dimensions)?,
        Shape::Multipatch(_) => {
            return Err(err(&PublicMessage::Curated(
                "Multipatch non ha una conversione WKB univoca ed è rifiutato",
            )))
        }
    };
    Ok(Some(geometry))
}

fn shape_has_valid_measure(shape: &Shape) -> bool {
    let valid = |measure: f64| {
        !matches!(
            measure.partial_cmp(&NO_DATA),
            Some(std::cmp::Ordering::Less | std::cmp::Ordering::Equal)
        )
    };
    match shape {
        Shape::PointZ(point) => valid(point.m),
        Shape::PolylineZ(polyline) => polyline
            .parts()
            .iter()
            .flatten()
            .any(|point| valid(point.m)),
        Shape::PolygonZ(polygon) => polygon
            .rings()
            .iter()
            .flat_map(PolygonRing::points)
            .any(|point| valid(point.m)),
        Shape::MultipointZ(multipoint) => multipoint.points().iter().any(|point| valid(point.m)),
        _ => false,
    }
}

fn dimensions_for_shape_tag(shape_type: Option<&str>, z_has_measure: bool) -> CoordinateDimensions {
    match shape_type {
        Some(tag) if tag.ends_with("-xy") => CoordinateDimensions::Xy,
        Some(tag) if tag.ends_with("-m") => CoordinateDimensions::Xym,
        Some(tag) if tag.ends_with("-z") && z_has_measure => CoordinateDimensions::Xyzm,
        Some(tag) if tag.ends_with("-z") => CoordinateDimensions::Xyz,
        _ => CoordinateDimensions::Unknown,
    }
}

fn header_geometry(shape_type: ShapeType) -> Result<(Option<&'static str>, Vec<GeometryType>)> {
    let value = match shape_type {
        ShapeType::NullShape => (None, Vec::new()),
        ShapeType::Point => (Some("point-xy"), vec![GeometryType::Point]),
        ShapeType::PointM => (Some("point-m"), vec![GeometryType::Point]),
        ShapeType::PointZ => (Some("point-z"), vec![GeometryType::Point]),
        ShapeType::Polyline => (Some("polyline-xy"), vec![GeometryType::MultiLineString]),
        ShapeType::PolylineM => (Some("polyline-m"), vec![GeometryType::MultiLineString]),
        ShapeType::PolylineZ => (Some("polyline-z"), vec![GeometryType::MultiLineString]),
        ShapeType::Polygon => (Some("polygon-xy"), vec![GeometryType::MultiPolygon]),
        ShapeType::PolygonM => (Some("polygon-m"), vec![GeometryType::MultiPolygon]),
        ShapeType::PolygonZ => (Some("polygon-z"), vec![GeometryType::MultiPolygon]),
        ShapeType::Multipoint => (Some("multipoint-xy"), vec![GeometryType::MultiPoint]),
        ShapeType::MultipointM => (Some("multipoint-m"), vec![GeometryType::MultiPoint]),
        ShapeType::MultipointZ => (Some("multipoint-z"), vec![GeometryType::MultiPoint]),
        ShapeType::Multipatch => {
            return Err(err(&PublicMessage::Curated(
                "Multipatch Shapefile non supportato",
            )))
        }
    };
    Ok(value)
}

fn shape_type_label(shape_type: Option<&str>) -> &str {
    shape_type.unwrap_or("null")
}

/// Il tipo geometrico e' una proprieta' dell'header Shapefile. Per i tipi Z
/// soltanto, M e' opzionale record per record e richiede una scansione; i
/// comuni percorsi XY/M non devono decodificare tutte le geometrie durante
/// l'apertura per poi decodificarle di nuovo durante la lettura.
fn infer_geometry_info(path: &Path, dbf_record_count: u32) -> Result<ShpGeometryInfo> {
    valida_struttura_shp(path)?;
    let mut reader = ShapeReader::from_path(path).map_err(|_| {
        err(&PublicMessage::Curated(
            "apertura delle geometrie Shapefile fallita",
        ))
    })?;
    let native_type = reader.header().shape_type;
    if let Ok(shape_count) = reader.shape_count() {
        if shape_count != dbf_record_count as usize {
            // Entrambi i conteggi vengono dal file: resta la condizione.
            return Err(err(&PublicMessage::Curated(
                "numero di geometrie diverso dal numero di record DBF",
            )));
        }
    }
    let (shape_type, geometry_types) = header_geometry(native_type)?;
    let mut z_has_measure = false;
    if native_type.has_z() {
        for shape in reader.iter_shapes() {
            let shape = shape.map_err(|_| {
                err(&PublicMessage::Curated(
                    "record geometrico Shapefile non leggibile",
                ))
            })?;
            let tag = shape_tag(&shape);
            if !tag.is_empty() && Some(tag) != shape_type {
                // Il tag del record viene dal file. L'etichetta dell'header
                // e' un nostro `&'static str`, e resta.
                return Err(err(&PublicMessage::CuratedPair(
                    "tipo Shape del record incoerente con quello dell'header:",
                    shape_type_label(shape_type),
                )));
            }
            z_has_measure |= shape_has_valid_measure(&shape);
        }
    }
    Ok(ShpGeometryInfo {
        dimensions: dimensions_for_shape_tag(shape_type, z_has_measure),
        geometry_types,
        shape_type,
    })
}

/// Pass 1: nomi campo, tipo DBF e contratto geometrico nativo, a RAM O(ncol).
// Passata unica sul DBF: layout, accumulatori di tipo, righe cancellate e
// rischio di precisione condividono lo stesso scorrimento dei record. Spezzarla
// significherebbe rileggere il file e perdere la garanzia O(ncol).
#[allow(clippy::too_many_lines)]
fn infer_shp_schema(path: &Path) -> Result<ShpInference> {
    let dbf_layout = read_dbf_layout(path)?;
    let mut exact_rows = DbfExactIntegerRows::open(path, &dbf_layout)?;
    let geometry_info = infer_geometry_info(path, dbf_layout.record_count)?;
    // `read_dbf_layout` ha gia' validato lo stesso file poche righe sopra, e la
    // ripetizione e' voluta: la garanzia deve valere per **questa** apertura,
    // non per una che le sta accanto oggi.
    //
    // Il costo non e' sempre lo stesso: trentadue byte piu' i descrittori quando
    // non ci sono campi temporali, una scansione dei record quando ce ne sono --
    // perche' li' il valore, non il descrittore, e' cio' che puo' fermare il
    // lettore.
    valida_intestazione_dbf(&path.with_extension("dbf"))?;
    let mut reader = shapefile::dbase::Reader::from_path(path.with_extension("dbf"))
        .map_err(|_| err(&PublicMessage::Curated("apertura del DBF fallita")))?;
    let mut accs: HashMap<String, TypeAccumulator> = dbf_layout
        .fields
        .iter()
        .map(|field| {
            let mut accumulator = TypeAccumulator::default();
            if field.exact_integer_slot.is_some() {
                // Il tipo e' dichiarato dal descrittore N(width>=10, decimals=0),
                // anche quando tutti i valori sono nulli.
                accumulator.observe(ObservedValueClass::Integer);
            } else {
                accumulator.observe(match field.field_type {
                    b'N' | b'F' => ObservedValueClass::Number,
                    b'L' => ObservedValueClass::Boolean,
                    _ => ObservedValueClass::Text,
                });
            }
            (field.name.clone(), accumulator)
        })
        .collect();
    let mut loss = LossReport::default();
    let mut precision_risk_fields = BTreeSet::new();
    let mut active_row_count = 0_u64;
    let mut records = reader.iter_records();
    while let Some(physical_row) = exact_rows.next_physical(None)? {
        let exact_values = match physical_row {
            DbfPhysicalRow::Deleted => continue,
            DbfPhysicalRow::Active { exact_values, .. } => exact_values,
        };
        active_row_count = active_row_count.checked_add(1).ok_or_else(|| {
            err(&PublicMessage::Curated(
                "numero di record DBF fuori intervallo u64",
            ))
        })?;
        let record = match records.next() {
            Some(Ok(record)) => record,
            Some(Err(_)) => continue,
            None => {
                return Err(err(&PublicMessage::Curated(
                    "numero di record DBF incoerente con l'header",
                )))
            }
        };
        for field in &dbf_layout.fields {
            let accumulator = accs.get_mut(&field.name).ok_or_else(|| {
                err(&PublicMessage::Curated(
                    "schema DBF senza accumulatore per un campo dichiarato",
                ))
            })?;
            if let Some(slot) = field.exact_integer_slot {
                accumulator
                    .observe(exact_values[slot].map_or(ObservedValueClass::Null, classify_i64));
                continue;
            }
            let value = record.get(&field.name);
            if value.is_some_and(dbf_numeric_integer_precision_unverifiable) {
                loss.record(DBF_NUMERIC_INTEGER_PRECISION_UNVERIFIABLE, 1);
                precision_risk_fields.insert(field.name.clone());
            }
            accumulator.observe(value.map_or(ObservedValueClass::Null, classify));
        }
    }
    if records.next().is_some() {
        return Err(err(&PublicMessage::Curated(
            "numero di record DBF incoerente con l'header",
        )));
    }
    let columns = dbf_layout
        .fields
        .iter()
        .map(|field| field.name.clone())
        .map(|name| {
            let column_type = accs
                .get(&name)
                .ok_or_else(|| {
                    err(&PublicMessage::Curated(
                        "schema DBF senza accumulatore per un campo dichiarato",
                    ))
                })?
                .column_type();
            let exact_integer_slot = dbf_layout
                .fields
                .iter()
                .find(|field| field.name == name)
                .and_then(|field| field.exact_integer_slot);
            Ok(ShpColumn {
                name,
                column_type,
                exact_integer_slot,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    for name in precision_risk_fields {
        loss.add_example(LossExample {
            category: DBF_NUMERIC_INTEGER_PRECISION_UNVERIFIABLE.to_owned(),
            posizione: Posizione {
                layer_index: None,
                // L'indice in `schema.fields()`, non in `cols`: lo schema mette
                // la geometria **prima** delle colonne DBF, quindi la colonna
                // `i`-esima di `cols` e' la `i+1`-esima dello schema. Il numero
                // deve essere confrontabile con gli altri `field_index`, che
                // quella sequenza indicizzano. L'accoppiamento e' verificato
                // dalla sonda `la_geometria_e_il_primo_campo_dello_schema`:
                // se un giorno lo schema cambiasse ordine, questo `+ 1`
                // sarebbe sbagliato e nessuno se ne accorgerebbe.
                field_index: columns
                    .iter()
                    .position(|column| column.name == name)
                    // `saturating_add` prima della conversione: sommare e poi
                    // convertire potrebbe traboccare prima di saturare.
                    .map(|i| plenora_io_core::driver::saturating_u64(i.saturating_add(1))),
                type_class: None,
            },
            context: "DBF Numeric già decodificato come f64 senza precisione intera unitaria"
                .to_owned(),
        });
    }
    Ok(ShpInference {
        cols: columns,
        dbf_layout,
        geometry_info,
        active_row_count,
        loss,
    })
}

struct ShpParserInput {
    path: PathBuf,
    schema: SchemaRef,
    cols: Vec<ShpColumn>,
    dbf_layout: DbfLayout,
    dimensions: CoordinateDimensions,
    expected_shape_type: Option<&'static str>,
    expected_active_rows: u64,
    include_geometry: bool,
    batch_sizer: plenora_io_core::AdaptiveBatchSizer,
    layer: LayerContract,
    loss: LossReport,
    row_diagnostics: ShpRowDiagnosticsConfig,
    scope: ReadScope,
    cancellation: plenora_io_model::CancellationToken,
}

/// Pass 2: thread che scorre i record e produce batch da `batch_size` righe.
// Corpo unico del thread di parsing: lo stato del ciclo (reader shp, reader
// dbf, builder, diagnostica, scope) e' condiviso da tutte le fasi e spezzarlo
// richiederebbe di esporre quello stato in strutture ausiliarie.
#[allow(clippy::too_many_lines)]
fn spawn_parser(input: ShpParserInput) -> Result<Box<dyn LayerReader>> {
    let ShpParserInput {
        path,
        schema,
        cols,
        dbf_layout,
        dimensions,
        expected_shape_type,
        expected_active_rows,
        include_geometry,
        mut batch_sizer,
        layer,
        loss,
        row_diagnostics,
        scope,
        cancellation,
    } = input;
    // Le validazioni stanno **fuori** dalla chiusura, e prima che il thread
    // parta: un file che farebbe panicare `shapefile` o `dbase` viene rifiutato
    // dal chiamante come errore tipizzato, invece che dentro un thread appena
    // creato -- dove il panico diventa un abort che nessun `catch_unwind` vede.
    valida_struttura_shp(&path)?;
    valida_intestazione_dbf(&path.with_extension("dbf"))?;
    let reader = spawn_batch_reader(DESCRIPTOR.id(), layer, 2, move |emitter: BatchEmitter| {
        if scope == ReadScope::AcceptedRows(0) {
            return Ok(());
        }
        let mut shape_reader = shapefile::ShapeReader::from_path(&path)
            .map_err(|_| err(&PublicMessage::Curated("shapefile non valido")))?;
        let mut dbf_reader = shapefile::dbase::Reader::from_path(path.with_extension("dbf"))
            .map_err(|_| err(&PublicMessage::Curated("apertura del DBF fallita")))?;
        let mut shapes = shape_reader.iter_shapes();
        let mut records = dbf_reader.iter_records();
        let mut exact_rows = DbfExactIntegerRows::open(&path, &dbf_layout)?;
        let mut geom = include_geometry.then(BinaryBuilder::new);
        let mut builders: Vec<InferredColumnBuilder> = cols
            .iter()
            .map(|column| InferredColumnBuilder::new(column.column_type))
            .collect();
        let mut n = 0usize;
        let mut source_rows_seen = 0_u64;
        let mut active_rows_seen = 0_u64;
        let raw_numeric_field_index = row_diagnostics
            .key
            .as_ref()
            .and_then(|key| key.raw_numeric_field_index);
        let mut diagnostics = ShpRowDiagnostics::new(row_diagnostics);
        loop {
            if !diagnostics.is_empty()
                && matches!(scope, ReadScope::AcceptedRows(limit) if active_rows_seen >= limit)
            {
                return Err(diagnostics.into_partial_error(
                    err(&PublicMessage::Curated(
                        "limite di righe richiesto raggiunto durante la diagnostica Shapefile",
                    )),
                    "read_scope_row_limit_reached",
                ));
            }
            if !diagnostics.is_empty()
                && source_rows_seen.is_multiple_of(1_024)
                && !emitter.is_receiver_alive()
            {
                return Ok(());
            }
            if let Err(error) =
                plenora_io_core::check_cancelled(&cancellation, plenora_io_model::ErrorPhase::Read)
            {
                return Err(diagnostics.into_partial_error(error, "shapefile.scan_cancelled"));
            }
            let physical_row = match exact_rows.next_physical(raw_numeric_field_index) {
                Ok(Some(row)) => row,
                Ok(None) => break,
                Err(error) => {
                    return Err(diagnostics
                        .into_partial_error(error, "shapefile.dbf_exact_scan_interrupted"));
                }
            };
            let source_index = source_rows_seen;
            source_rows_seen = source_rows_seen.checked_add(1).ok_or_else(|| {
                err(&PublicMessage::Curated(
                    "numero di record Shapefile fuori intervallo u64",
                ))
            })?;
            let shape = match shapes.next() {
                Some(Ok(shape)) => shape,
                Some(Err(_)) => {
                    return Err(diagnostics.into_partial_error(
                        err(&PublicMessage::Curated("record shapefile non valido")),
                        "shapefile.scan_interrupted",
                    ));
                }
                None => {
                    return Err(diagnostics.into_partial_error(
                        err(&PublicMessage::Curated(
                            "numero di geometrie incoerente con i record DBF",
                        )),
                        "shapefile.scan_interrupted",
                    ));
                }
            };
            let (exact_values, raw_numeric_key, physical_rejection) = match physical_row {
                DbfPhysicalRow::Deleted => continue,
                DbfPhysicalRow::Active {
                    exact_values,
                    raw_numeric_key,
                    rejection_cause,
                } => (exact_values, raw_numeric_key, rejection_cause),
            };
            active_rows_seen = active_rows_seen.checked_add(1).ok_or_else(|| {
                err(&PublicMessage::Curated(
                    "numero di record DBF attivi fuori intervallo u64",
                ))
            })?;
            let record = match records.next() {
                Some(Ok(record)) => record,
                Some(Err(_)) => {
                    let cause = physical_rejection
                        .map_or("shapefile.attribute_decode_failed", |cause| cause);
                    diagnostics.record(source_index, cause, None, raw_numeric_key.as_deref());
                    continue;
                }
                None => {
                    return Err(diagnostics.into_partial_error(
                        err(&PublicMessage::Curated(
                            "numero di record DBF attivi incoerente con le geometrie",
                        )),
                        "shapefile.scan_interrupted",
                    ));
                }
            };
            if let Some(cause) = physical_rejection {
                diagnostics.record(
                    source_index,
                    cause,
                    Some(&record),
                    raw_numeric_key.as_deref(),
                );
                continue;
            }
            let tag = shape_tag(&shape);
            if !tag.is_empty() && Some(tag) != expected_shape_type {
                diagnostics.record(
                    source_index,
                    "shapefile.shape_type_mismatch",
                    Some(&record),
                    raw_numeric_key.as_deref(),
                );
                continue;
            }
            if let Some(cause) = shape_rejection_cause(&shape) {
                diagnostics.record(
                    source_index,
                    cause,
                    Some(&record),
                    raw_numeric_key.as_deref(),
                );
                continue;
            }
            let Ok(converted_geometry) = shape_to_wkb(&shape, dimensions) else {
                diagnostics.record(
                    source_index,
                    "shapefile.geometry_conversion_failed",
                    Some(&record),
                    raw_numeric_key.as_deref(),
                );
                continue;
            };
            let encoded_geometry = match converted_geometry {
                Some(geometry) if include_geometry => {
                    let Ok(bytes) = encode_wkb(&geometry, WkbFlavor::Iso) else {
                        diagnostics.record(
                            source_index,
                            "shapefile.geometry_encoding_failed",
                            Some(&record),
                            raw_numeric_key.as_deref(),
                        );
                        continue;
                    };
                    Some(bytes)
                }
                _ => None,
            };
            if !diagnostics.is_empty() {
                // Dopo il primo rifiuto la scansione continua soltanto per
                // completare conteggi/esempi; nessun altro batch viene emesso.
                continue;
            }
            if let Some(builder) = &mut geom {
                match encoded_geometry {
                    Some(bytes) => builder.append_value(bytes),
                    None => builder.append_null(),
                }
            }
            // Lookup per nome (l'ordine di iterazione del Record non è garantito).
            for (k, column) in cols.iter().enumerate() {
                if let Some(slot) = column.exact_integer_slot {
                    match exact_values[slot] {
                        Some(value) => {
                            if let Err(error) = builders[k].append_i64(value) {
                                diagnostics.record(
                                    source_index,
                                    "shapefile.attribute_conversion_failed",
                                    Some(&record),
                                    raw_numeric_key.as_deref(),
                                );
                                return Err(diagnostics.into_partial_error(
                                    error,
                                    "shapefile.attribute_scan_interrupted",
                                ));
                            }
                        }
                        None => builders[k].append_null(),
                    }
                    continue;
                }
                let value = record
                    .get(&column.name)
                    .filter(|value| classify(value) != ObservedValueClass::Null);
                if let Err(error) =
                    builders[k].append_converted(value, fv_i64, fv_f64, fv_bool, |value| {
                        fv_string(value).map(Cow::Owned)
                    })
                {
                    diagnostics.record(
                        source_index,
                        "shapefile.attribute_conversion_failed",
                        Some(&record),
                        raw_numeric_key.as_deref(),
                    );
                    return Err(diagnostics
                        .into_partial_error(error, "shapefile.attribute_scan_interrupted"));
                }
            }
            n += 1;
            if n >= batch_sizer.rows() {
                let batch = finish_batch(&schema, &mut geom, &mut builders, n)?;
                batch_sizer.observe(&batch);
                if !emitter.send_cancellable(
                    batch,
                    &cancellation,
                    plenora_io_model::ErrorPhase::Read,
                )? {
                    return Ok(());
                }
                n = 0;
            }
        }
        if source_rows_seen != u64::from(dbf_layout.record_count)
            || active_rows_seen != expected_active_rows
            || shapes.next().is_some()
            || records.next().is_some()
        {
            return Err(diagnostics.into_partial_error(
                err(&PublicMessage::Curated(
                    "cardinalita' Shapefile cambiata durante la lettura",
                )),
                "shapefile.scan_interrupted",
            ));
        }
        if !diagnostics.is_empty() {
            let rejected = diagnostics.observed_total;
            return Err(err(&PublicMessage::CuratedWith(
                "righe Shapefile non valide; consultare row_diagnostics, conteggio",
                NumeroStrutturale::Conteggio(rejected),
            ))
            .with_row_diagnostics(diagnostics.into_report()));
        }
        if n > 0 {
            let batch = finish_batch(&schema, &mut geom, &mut builders, n)?;
            if !emitter.send_cancellable(
                batch,
                &cancellation,
                plenora_io_model::ErrorPhase::Read,
            )? {
                return Ok(());
            }
        }
        Ok(())
    })?;
    Ok(Box::new(ShpLossReader {
        inner: reader,
        loss,
    }))
}

struct ShpLossReader {
    inner: Box<dyn LayerReader>,
    loss: LossReport,
}

impl LayerReader for ShpLossReader {
    fn contract(&self) -> &LayerContract {
        self.inner.contract()
    }

    fn next_batch(&mut self) -> Result<Option<RecordBatch>> {
        self.inner.next_batch()
    }

    fn loss_report(&self) -> LossReport {
        let mut loss = self.inner.loss_report();
        loss.merge(&self.loss);
        loss
    }
}

fn finish_batch(
    schema: &SchemaRef,
    geom: &mut Option<BinaryBuilder>,
    builders: &mut [InferredColumnBuilder],
    row_count: usize,
) -> Result<RecordBatch> {
    let mut arrays: Vec<ArrayRef> =
        Vec::with_capacity(usize::from(geom.is_some()) + builders.len());
    if let Some(builder) = geom {
        arrays.push(Arc::new(builder.finish()));
    }
    for b in builders.iter_mut() {
        arrays.push(b.finish());
    }
    let options = RecordBatchOptions::new().with_row_count(Some(row_count));
    RecordBatch::try_new_with_options(schema.clone(), arrays, &options).map_err(|_| {
        err(&PublicMessage::Curated(
            "costruzione del RecordBatch fallita",
        ))
    })
}

fn fv_i64(v: &FieldValue) -> Option<i64> {
    match v {
        FieldValue::Integer(i) => Some(i64::from(*i)),
        // I campi DBF Numeric/Double/Float sono decodificati come virgola
        // mobile dal parser dbase: la conversione a intero tronca verso zero e
        // satura, esattamente come prima. Il caso davvero esatto (N con
        // width>=10 e decimals=0) non passa da qui ma dallo slot
        // `exact_integer_slot`, che legge i byte ASCII del record.
        #[allow(clippy::cast_possible_truncation)]
        FieldValue::Numeric(Some(n)) => Some(*n as i64),
        #[allow(clippy::cast_possible_truncation)]
        FieldValue::Double(d) => Some(*d as i64),
        #[allow(clippy::cast_possible_truncation)]
        FieldValue::Float(Some(f)) => Some(*f as i64),
        _ => None,
    }
}

fn fv_f64(v: &FieldValue) -> Option<f64> {
    match v {
        FieldValue::Numeric(Some(n)) => Some(*n),
        FieldValue::Double(d) => Some(*d),
        FieldValue::Float(Some(f)) => Some(f64::from(*f)),
        FieldValue::Integer(i) => Some(f64::from(*i)),
        _ => None,
    }
}

const fn fv_bool(v: &FieldValue) -> Option<bool> {
    match v {
        FieldValue::Logical(Some(b)) => Some(*b),
        _ => None,
    }
}

fn fv_string(v: &FieldValue) -> Option<String> {
    match v {
        FieldValue::Character(Some(s)) => Some(s.clone()),
        FieldValue::Date(Some(d)) => {
            Some(format!("{:04}-{:02}-{:02}", d.year(), d.month(), d.day()))
        }
        FieldValue::Integer(i) => Some(i.to_string()),
        FieldValue::Numeric(Some(n)) => Some(n.to_string()),
        FieldValue::Double(d) => Some(d.to_string()),
        FieldValue::Float(Some(f)) => Some(f.to_string()),
        FieldValue::Logical(Some(b)) => Some(b.to_string()),
        _ => None,
    }
}

/// Entry point non stabile per libFuzzer: decodifica WKB dimensionale,
/// conversione nella shape ESRI concreta e ritorno a WKB.
#[doc(hidden)]
pub fn __fuzz_wkb_roundtrip(bytes: &[u8]) -> Result<usize> {
    let geometry = decode_wkb(bytes, &WkbLimits::default())?;
    let dimensions = geometry.dimensions;
    let shape = shape_from_wkb(geometry)?;
    let round_trip = shape_to_wkb(&shape, dimensions)?.ok_or_else(|| {
        err(&PublicMessage::Curated(
            "la conversione di una geometria ha prodotto NullShape",
        ))
    })?;
    Ok(encode_wkb(&round_trip, WkbFlavor::Iso)?.len())
}

/// Le quattro parti di un bundle, con un nome ciascuna.
///
/// Quattro fette dello stesso tipo in una tupla si scambiano senza che il
/// compilatore se ne accorga, e uno scambio fra `.shx` e `.dbf` produrrebbe un
/// target che sembra funzionare e copre l'errore invece del formato.
#[doc(hidden)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PartiDelBundle<'a> {
    pub shp: &'a [u8],
    pub shx: &'a [u8],
    pub dbf: &'a [u8],
    pub prj: &'a [u8],
}

/// Entry point non stabile per libFuzzer: la divisione di un bundle Shapefile.
///
/// Uno Shapefile non e' un file. Il driver riceve il percorso del `.shp` e
/// risale ai fratelli cambiando estensione, quindi un target che materializzi
/// il solo `.shp` rimbalza sull'apertura del `.dbf` senza mai raggiungere il
/// parsing. Il fuzzer consegna pero' **un** blob, e qualcuno deve dividerlo.
///
/// ```text
/// byte 0..2   lunghezza del .shp, big-endian
/// byte 2..4   lunghezza del .shx, big-endian
/// byte 4..6   lunghezza del .dbf, big-endian
/// byte 6..    .shp | .shx | .dbf | .prj   (il resto e' il .prj, anche vuoto)
/// ```
///
/// Il `.shx` e' nel bundle perche' senza di esso `shape_count()` non risponde,
/// e il confronto anticipato fra numero di forme e numero di record DBF non
/// viene mai eseguito: un ramo del reader resterebbe irraggiungibile da questo
/// target, e la copertura direbbe che il formato e' esercitato mentre una delle
/// sue difese non lo e'.
///
/// Le lunghezze dichiarate si **saturano** su cio' che resta invece di far
/// scartare l'input: un mutante che allunga un campo di lunghezza non smette di
/// essere un caso di prova, e rifiutarlo toglierebbe al fuzzer proprio le
/// mutazioni sull'intestazione.
///
/// La divisione e' fail-closed per costruzione: `usize::from(u16)` non puo'
/// traboccare, `min` con la lunghezza residua non puo' superarla, e nessuna
/// allocazione deriva dai valori dichiarati — si restituiscono sottofette di
/// cio' che il chiamante gia' possiede. Una lunghezza di 65 535 su un corpo di
/// dieci byte produce dieci byte, non un tentativo di riservarne 65 535.
///
/// `None` = input piu' corto dell'intestazione, cioe' non un bundle.
#[doc(hidden)]
#[must_use]
pub fn __fuzz_dividi_bundle(dati: &[u8]) -> Option<PartiDelBundle<'_>> {
    const INTESTAZIONE: usize = 6;
    if dati.len() < INTESTAZIONE {
        return None;
    }
    let dichiarate = [
        usize::from(u16::from_be_bytes([dati[0], dati[1]])),
        usize::from(u16::from_be_bytes([dati[2], dati[3]])),
        usize::from(u16::from_be_bytes([dati[4], dati[5]])),
    ];
    let resto = &dati[INTESTAZIONE..];

    let (shp, resto) = resto.split_at(dichiarate[0].min(resto.len()));
    let (shx, resto) = resto.split_at(dichiarate[1].min(resto.len()));
    let (dbf, prj) = resto.split_at(dichiarate[2].min(resto.len()));
    Some(PartiDelBundle { shp, shx, dbf, prj })
}

/// Un errore dell'ambiente non e' un difetto del file letto.
///
/// Un filesystem pieno, una directory non creabile o una scrittura fallita
/// diventano un errore tipizzato e non un panico: un panico dell'harness
/// verrebbe archiviato dal fuzzer come finding del reader, e la campagna
/// misurerebbe il proprio scaffolding.
fn errore_di_ambiente(_: std::io::Error) -> PlenoraIoError {
    err(&PublicMessage::Curated(
        "materializzazione del bundle fallita: e' l'ambiente, non il file letto",
    ))
}

/// Scrive le parti del bundle dentro una directory **gia' esistente**.
///
/// Separata da `__fuzz_leggi_bundle` per una ragione sola: cosi' una sonda puo'
/// passarle una radice inesistente e osservare l'errore. Forzare il fallimento
/// mutando `TMPDIR` renderebbe il difetto visibile agli altri test in
/// parallelo, e il fallimento sarebbe intermittente invece che riproducibile.
///
/// I nomi sono **letterali**: nessun percorso deriva dal payload.
fn materializza_bundle(radice: &Path, parti: &PartiDelBundle<'_>) -> Result<PathBuf> {
    let PartiDelBundle { shp, shx, dbf, prj } = *parti;
    let principale = radice.join("input.shp");
    std::fs::write(&principale, shp).map_err(errore_di_ambiente)?;
    std::fs::write(radice.join("input.dbf"), dbf).map_err(errore_di_ambiente)?;
    if !shx.is_empty() {
        // Senza `.shx` il driver non conta le forme in anticipo: e' un percorso
        // legittimo, e va lasciato raggiungibile quanto quello con l'indice.
        std::fs::write(radice.join("input.shx"), shx).map_err(errore_di_ambiente)?;
    }
    if !prj.is_empty() {
        // Il `.prj` si scrive solo se c'e': la sua assenza e' un percorso del
        // driver — `resolve_crs` ripiega su `assume_crs` — e va esercitata
        // quanto la sua presenza.
        std::fs::write(radice.join("input.prj"), prj).map_err(errore_di_ambiente)?;
    }
    Ok(principale)
}

#[doc(hidden)]
pub fn __fuzz_leggi_bundle(dati: &[u8], opts: ReadOptions) -> Result<usize> {
    let Some(parti) = __fuzz_dividi_bundle(dati) else {
        return Err(err(&PublicMessage::Curated(
            "bundle piu' corto della propria intestazione",
        )));
    };

    let directory = tempfile::Builder::new()
        .prefix("plenora-fuzz-shp-")
        .tempdir()
        .map_err(errore_di_ambiente)?;
    let principale = materializza_bundle(directory.path(), &parti)?;

    let dataset = ShpDriver.open(Source::Path(principale), opts)?;
    let layers: Vec<LayerId> = dataset.layers().iter().map(|layer| layer.id).collect();
    let mut righe = 0_usize;
    for layer in layers {
        let mut reader = dataset.open_layer_reader(&ReadRequest {
            layer,
            projected_fields: None,
            projection_mode: ProjectionMode::BestEffort,
            pruning_predicate: None,
            spatial_pruning_hint: None,
            scope: ReadScope::Complete,
            batch_target: BatchTarget::default(),
            cancellation: plenora_io_model::CancellationToken::default(),
        })?;
        while let Some(batch) = reader.next_batch()? {
            righe = righe.saturating_add(batch.num_rows());
        }
        let _ = reader.loss_report();
    }
    Ok(righe)
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- il bundle del fuzzer, e i semi che lo alimentano -----------------
    //
    // Una build che compila e un replay senza crash non dimostrano che i semi
    // raggiungano il parsing: un bundle rifiutato all'apertura non fa crashare
    // niente ed e' indistinguibile, da fuori, da uno letto per intero. Queste
    // sonde chiamano lo **stesso** entry point del target sui **semi
    // committati**, e guardano che cosa ne esce.

    /// I semi vivono accanto al target che li usa.
    fn seme(nome: &str) -> Vec<u8> {
        let percorso = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fuzz/seeds/shp_reader")
            .join(nome);
        std::fs::read(&percorso)
            .unwrap_or_else(|errore| panic!("seme {} non leggibile: {errore}", percorso.display()))
    }

    /// Limiti dello stesso ordine di quelli della campagna.
    ///
    /// Non sono gli **stessi**: `harness::limits()` vive nella crate di fuzzing,
    /// e un driver non puo' dipenderne. Quel che conta e' che ci siano tetti --
    /// una sonda che leggesse senza limiti percorrerebbe una strada che il
    /// fuzzer non percorre mai -- e che siano stretti abbastanza da rendere
    /// osservabile un input che li supera.
    fn opzioni_di_campagna() -> ReadOptions {
        let limiti = plenora_io_model::budget::PipelineLimits::default()
            .with_max_input_bytes(1_048_576)
            .with_max_rows(100_000)
            .with_memory_bytes(64 * 1024 * 1024);
        let bundle = plenora_io_model::budget::PipelineBudget::builder()
            .limits(limiti)
            .build()
            .expect("limiti della campagna validi");
        ReadOptions::from_read_parts(bundle.into_read_parts())
    }

    #[test]
    fn il_seme_di_punti_arriva_alle_geometrie_e_agli_attributi() {
        let righe = __fuzz_leggi_bundle(
            &seme("punti-con-attributi.bundle"),
            opzioni_di_campagna().with_assume_crs("EPSG:4326"),
        )
        .expect("il seme deve essere letto: se no il target non copre il parsing");
        assert_eq!(
            righe, 2,
            "due punti e due record DBF: un conteggio diverso vuol dire che il \
             seme non attraversa piu' il drenaggio"
        );
    }

    /// La terza famiglia di geometria del formato.
    ///
    /// Un multipunto dichiara il numero di punti e **non** l'indice delle
    /// parti: nel driver percorre un ramo di prevalidazione che ne' il punto --
    /// che non dichiara conteggi -- ne' la polilinea raggiungono. Senza un seme
    /// che lo porti, quel ramo resterebbe scoperto e una delle tre famiglie del
    /// formato non sarebbe esercitata dal target.
    #[test]
    fn il_seme_di_multipunto_arriva_al_drenaggio() {
        let righe = __fuzz_leggi_bundle(
            &seme("multipunto.bundle"),
            opzioni_di_campagna().with_assume_crs("EPSG:4326"),
        )
        .expect("il seme deve essere letto");
        assert_eq!(
            righe, 1,
            "una geometria multipunto e un record DBF: il conteggio dice che il \
             seme attraversa il drenaggio, non solo l'apertura"
        );

        // Il ramo della prevalidazione, chiamato direttamente: `SoloPunti` e'
        // una risposta diversa da `PartiEPunti`, e confondere i due tipi
        // renderebbe il limite sugli elementi sbagliato per entrambi.
        assert!(matches!(conteggi_attesi(8), ConteggiDelRecord::SoloPunti));
        for tag in [18, 28] {
            assert!(
                matches!(conteggi_attesi(tag), ConteggiDelRecord::SoloPunti),
                "il multipunto Z e M dichiarano gli stessi conteggi: {tag}"
            );
        }
        assert!(matches!(conteggi_attesi(3), ConteggiDelRecord::PartiEPunti));
        assert!(matches!(conteggi_attesi(1), ConteggiDelRecord::Nessuno));
    }

    /// Un valore ostile ferma la lettura **anche** in una riga cancellata.
    ///
    /// Questa prova diceva il contrario, e la coppia di semi serviva a
    /// dimostrarlo: stesso valore, un solo byte di differenza -- il marcatore --
    /// ed esiti opposti. Reggeva su una premessa scritta e mai verificata,
    /// «`dbase` salta i byte di una riga cancellata senza decodificarne un
    /// campo». Non li salta: la fuzz smoke ha trovato una riga cancellata il cui
    /// campo `D` fa panicare `Date::from_str`, attraversando l'apertura del
    /// driver.
    ///
    /// # Perche' uniforme, e che cosa costa
    ///
    /// Panica il taglio fuori dai confini -- meno di otto byte utili -- e panica
    /// un confine di carattere che cade dentro un multibyte. Il secondo dipende
    /// da **dove** cade, e dove cada dipende da come `dbase` decodifica i byte
    /// in stringa, cioe' da una scelta di codifica che non e' nostra. Il seme
    /// multibyte qui sotto oggi non panica; un altro con l'accento spostato di
    /// un byte lo farebbe.
    ///
    /// Una regola giusta solo per le posizioni che abbiamo campionato non e' una
    /// regola. La prevalidazione rifiuta quindi allo stesso modo nelle due
    /// righe, e il costo va detto: un dataset con una data malformata in una
    /// riga cancellata -- che prima veniva letto saltandola -- ora viene
    /// rifiutato. E' il verso in cui si sbaglia meglio, perche' l'altro verso
    /// non e' «leggere di piu'», e' un panico.
    #[test]
    fn una_data_ostile_ferma_la_lettura_anche_se_la_riga_e_cancellata() {
        for nome in [
            "dbf-data-multibyte.bundle",
            "dbf-data-multibyte-cancellata.bundle",
        ] {
            let errore = __fuzz_leggi_bundle(
                &seme(nome),
                opzioni_di_campagna().with_assume_crs("EPSG:4326"),
            )
            .expect_err("il marcatore di cancellazione non esenta il campo");
            assert_eq!(
                errore.message, "campo data DBF che il lettore non puo' interpretare",
                "{nome}"
            );
        }
    }

    #[test]
    fn il_seme_di_polilinea_arriva_al_drenaggio() {
        let righe = __fuzz_leggi_bundle(
            &seme("polilinea.bundle"),
            opzioni_di_campagna().with_assume_crs("EPSG:4326"),
        )
        .expect("il seme deve essere letto");
        assert_eq!(righe, 1);
    }

    /// Conteggi disallineati **con** indice: il `.shx` risponde a
    /// `shape_count()`, e il driver rifiuta all'apertura senza decodificare una
    /// sola geometria. E' la difesa piu' economica del reader, e senza un seme
    /// che porti l'indice sarebbe irraggiungibile da questo target.
    #[test]
    fn i_conteggi_disallineati_sono_rifiutati_all_apertura_quando_c_e_l_indice() {
        let errore = __fuzz_leggi_bundle(
            &seme("disallineati-con-indice.bundle"),
            opzioni_di_campagna().with_assume_crs("EPSG:4326"),
        )
        .expect_err("due forme e un solo record DBF non sono uno Shapefile coerente");
        assert_eq!(
            errore.message, "numero di geometrie diverso dal numero di record DBF",
            "il rifiuto deve venire dal confronto anticipato dei conteggi: se              cambia messaggio, il seme sta coprendo un altro ramo"
        );
    }

    /// Lo stesso disallineamento **senza** indice: `shape_count()` non risponde,
    /// il confronto anticipato non avviene, e l'incoerenza emerge durante il
    /// drenaggio. Due rami diversi dello stesso difetto del file — questo
    /// attraversa il parsing delle geometrie, l'altro no.
    #[test]
    fn i_conteggi_disallineati_emergono_nel_drenaggio_senza_indice() {
        let errore = __fuzz_leggi_bundle(
            &seme("disallineati-senza-indice.bundle"),
            opzioni_di_campagna().with_assume_crs("EPSG:4326"),
        )
        .expect_err("l'incoerenza va rilevata anche senza `.shx`");
        assert_eq!(
            errore.message, "cardinalita' Shapefile cambiata durante la lettura",
            "senza indice il rifiuto deve arrivare dalla lettura, non              dall'apertura: {errore:?}"
        );
    }

    /// I due punti di arresto di `shapefile`, e la loro regressione.
    ///
    /// La crate tratta i valori dichiarati nel file come se li avesse scritti
    /// lei: raddoppia gli scostamenti dell'indice dentro un `i32`, e prenota un
    /// vettore grande quanto il conteggio di punti dichiarato in un record,
    /// senza legarlo alla dimensione del record. Il primo e' un panico, il
    /// secondo un'allocazione che il processo non sopravvive -- e nessuno dei
    /// due e' un `Err`.
    #[test]
    fn uno_scostamento_dell_indice_che_non_regge_il_raddoppio_e_un_errore() {
        let errore = __fuzz_leggi_bundle(
            &seme("shx-scostamento-traboccante.bundle"),
            opzioni_di_campagna().with_assume_crs("EPSG:4326"),
        )
        .expect_err("uno scostamento oltre meta' di i32::MAX non e' un indice");
        assert_eq!(
            errore.message,
            "voce dell'indice Shapefile fuori intervallo"
        );

        // Il secondo ramo: uno scostamento che regge il raddoppio e punta
        // comunque oltre la fine del `.shp`. Sono due difese diverse, e un seme
        // solo ne proverebbe una.
        let errore = __fuzz_leggi_bundle(
            &seme("shx-scostamento-fuori-dal-shp.bundle"),
            opzioni_di_campagna().with_assume_crs("EPSG:4326"),
        )
        .expect_err("un record che comincia oltre la fine del file non esiste");
        assert_eq!(
            errore.message,
            "voce dell'indice Shapefile che punta fuori dal .shp"
        );

        // Il terzo ramo, e il piu' insidioso: lo scostamento sta dentro il file
        // ma cade in mezzo al contenuto di un record, dove otto byte qualunque
        // diventano una testa. La catena sequenziale non lo vede, perche' il
        // lettore con indice non la percorre.
        let errore = __fuzz_leggi_bundle(
            &seme("shx-scostamento-dentro-un-record.bundle"),
            opzioni_di_campagna().with_assume_crs("EPSG:4326"),
        )
        .expect_err("un indice che punta dentro un record non e' un indice");
        assert_eq!(
            errore.message,
            "lunghezza del record diversa da quella dichiarata nell'indice"
        );
    }

    #[test]
    fn un_record_che_dichiara_piu_punti_di_quanti_ne_contenga_e_un_errore() {
        for nome in ["shp-punti-assurdi.bundle", "shp-punti-negativi.bundle"] {
            let errore = __fuzz_leggi_bundle(
                &seme(nome),
                opzioni_di_campagna().with_assume_crs("EPSG:4326"),
            )
            .expect_err("un conteggio slegato dal record non e' leggibile");
            assert!(
                errore.message.contains("conteggio negativo")
                    || errore
                        .message
                        .contains("piu' elementi di quanti ne contenga"),
                "{nome}: {errore:?}"
            );
        }
    }

    /// Un record che dichiara un tipo con conteggi e non ha spazio per
    /// portarli.
    ///
    /// `read_shape_content` riceve la dimensione del record ma legge dal
    /// flusso: i conteggi finiscono per venire dai byte che seguono, e il
    /// vettore prenotato e' grande quanto quel numero. Non e' un panico ma una
    /// richiesta di memoria che il processo non sopravvive -- e per il fuzzer
    /// e' un finding come gli altri.
    #[test]
    fn un_record_troppo_corto_per_il_proprio_tipo_e_un_errore() {
        let errore = __fuzz_leggi_bundle(
            &seme("shp-record-troppo-corto.bundle"),
            opzioni_di_campagna().with_assume_crs("EPSG:4326"),
        )
        .expect_err("una polilinea di quattro byte non porta i propri conteggi");
        assert_eq!(
            errore.message,
            "record Shapefile troppo corto per il tipo che dichiara"
        );
    }

    /// L'indice delle parti, nei due modi in cui esce dai punti dichiarati.
    ///
    /// Il lettore prende la differenza fra due voci consecutive come numero di
    /// punti da leggere: una voce che scende la rende negativa, una che sale
    /// oltre il numero di punti la gonfia. Nel primo caso c'e' un
    /// `debug_assert!` -- panico sotto il fuzzer, niente in release -- nel
    /// secondo nemmeno quello.
    #[test]
    fn un_indice_delle_parti_fuori_dai_punti_e_un_errore() {
        for nome in [
            "shp-parti-che-scendono.bundle",
            "shp-parti-oltre-i-punti.bundle",
        ] {
            let errore = __fuzz_leggi_bundle(
                &seme(nome),
                opzioni_di_campagna().with_assume_crs("EPSG:4326"),
            )
            .expect_err("una parte che comincia fuori dai punti non esiste");
            assert_eq!(
                errore.message, "indice delle parti Shapefile che esce dai punti dichiarati",
                "{nome}"
            );
        }
    }

    /// Il campo `T`, che porta due interi binari invece di otto cifre.
    ///
    /// `julian_day_number_to_gregorian_date` lavora in `i32` e comincia con
    /// `4 * jdn + 274_277`; `Time::from_word` divide e rimoltiplica passando da
    /// `u32`, dove un parola-tempo negativo diventa enorme. Due traboccamenti
    /// distinti, e due semi.
    #[test]
    fn un_campo_data_e_ora_fuori_intervallo_e_un_errore() {
        for nome in [
            "dbf-giorno-giuliano-enorme.bundle",
            "dbf-parola-tempo-negativa.bundle",
        ] {
            let errore = __fuzz_leggi_bundle(
                &seme(nome),
                opzioni_di_campagna().with_assume_crs("EPSG:4326"),
            )
            .expect_err("un istante che il lettore non sa convertire non e' leggibile");
            assert_eq!(
                errore.message, "campo data-e-ora DBF fuori dall'intervallo convertibile",
                "{nome}"
            );
        }
    }

    /// Il rovescio: un istante reale passa, e cosi' i due estremi ammessi.
    #[test]
    fn un_istante_reale_passa() {
        let istante = |giorno: i32, ora: i32| {
            let mut byte = [0_u8; 8];
            byte[..4].copy_from_slice(&giorno.to_le_bytes());
            byte[4..].copy_from_slice(&ora.to_le_bytes());
            data_e_ora_non_convertibili(&byte)
        };
        assert!(
            !istante(2_458_685, 43_200_000),
            "mezzogiorno del 2019-07-11"
        );
        assert!(!istante(0, 0));
        assert!(!istante(
            DBF_MASSIMO_GIORNO_GIULIANO,
            DBF_MILLISECONDI_DEL_GIORNO - 1
        ));

        assert!(istante(DBF_MASSIMO_GIORNO_GIULIANO + 1, 0));
        assert!(istante(-1, 0));
        assert!(istante(0, DBF_MILLISECONDI_DEL_GIORNO));
        assert!(istante(0, -1));
    }

    /// Un record lungo **esattamente** la propria testa, che dichiara una
    /// parte.
    ///
    /// E' il caso che ha mostrato un difetto in questa stessa verifica: il
    /// confronto guardava solo gli elementi, non la testa che li precede, e i
    /// quattro byte dell'indice delle parti venivano letti **oltre** il record.
    /// La posizione nel file restava indietro, e da li' in poi la catena dei
    /// record veniva letta sfasata.
    #[test]
    fn un_record_lungo_quanto_la_propria_testa_non_puo_dichiarare_parti() {
        let errore = __fuzz_leggi_bundle(
            &seme("shp-parti-oltre-la-testa.bundle"),
            opzioni_di_campagna().with_assume_crs("EPSG:4326"),
        )
        .expect_err("quarantaquattro byte non portano testa e indice delle parti");
        assert_eq!(
            errore.message,
            "record Shapefile che dichiara piu' elementi di quanti ne contenga"
        );
    }

    /// Un descrittore che dichiara meno byte di quanti il suo tipo ne legga.
    ///
    /// `dbase` dichiara la dimensione fissa dei propri tipi e non la verifica:
    /// affetta comunque, e la fetta esce dal campo. Sono quattro tipi -- `L`,
    /// `I`, `Y`, `B`, `T` -- e la sonda ne prova uno per il seme e tutti per la
    /// funzione, perche' un elenco incompleto qui sarebbe indistinguibile da
    /// uno completo.
    #[test]
    fn un_campo_piu_corto_del_proprio_tipo_e_un_errore() {
        let errore = __fuzz_leggi_bundle(
            &seme("dbf-campo-corto.bundle"),
            opzioni_di_campagna().with_assume_crs("EPSG:4326"),
        )
        .expect_err("un intero largo due byte non e' un intero");
        assert_eq!(
            errore.message,
            "campo DBF piu' corto di quanto il suo tipo pretenda"
        );

        for (tipo, minima) in [
            (b'L', 1),
            (b'I', 4),
            (b'D', 8),
            (b'Y', 8),
            (b'B', 8),
            (b'T', 8),
        ] {
            assert_eq!(
                lunghezza_minima_del_campo(tipo),
                Some(minima),
                "tipo {}",
                char::from(tipo)
            );
        }
        // I tipi testuali non hanno una larghezza imposta: `C` e `N` la
        // dichiarano, e il loro contenuto e' un errore di parsing, non un
        // panico.
        assert_eq!(lunghezza_minima_del_campo(b'C'), None);
        assert_eq!(lunghezza_minima_del_campo(b'N'), None);
    }

    /// La verifica strutturale non deve rifiutare cio' che il decoder
    /// accetterebbe: e' la meta' che una prevalidazione sbaglia piu' spesso, e
    /// che nessun seme ostile mostrerebbe.
    #[test]
    fn i_semi_validi_passano_la_verifica_strutturale() {
        let temporanea = tempfile::tempdir().expect("directory temporanea");
        for nome in [
            "punti-con-attributi.bundle",
            "punti-con-prj.bundle",
            "polilinea.bundle",
        ] {
            let dati = seme(nome);
            let parti = __fuzz_dividi_bundle(&dati).expect("il seme e' un bundle");
            let radice = temporanea.path().join(nome);
            std::fs::create_dir(&radice).expect("directory del seme");
            let principale = materializza_bundle(&radice, &parti).expect("materializzazione");
            assert_eq!(valida_struttura_shp(&principale), Ok(()), "{nome}");
        }
    }

    /// Il primo finding del target, e la sua regressione.
    ///
    /// `dbase::File::open` ricavava il numero di campi da
    /// `offset_to_first_record` con una sottrazione non controllata: sotto la
    /// soglia il processo **panicava** invece di restituire un errore. Un
    /// panico attraversa il confine della libreria, e sotto `libfuzzer-sys`
    /// diventa un abort che nessun `catch_unwind` vede.
    ///
    /// Le due sonde tengono i due rami distinti: l'offset corto e il file
    /// dichiarato Visual `FoxPro` con meno byte del backlink. Chiuderne uno solo
    /// avrebbe lasciato l'altro raggiungibile.
    #[test]
    fn un_offset_del_primo_record_troppo_corto_e_un_errore_non_un_panico() {
        let errore = __fuzz_leggi_bundle(
            &seme("dbf-offset-corto.bundle"),
            opzioni_di_campagna().with_assume_crs("EPSG:4326"),
        )
        .expect_err("un offset sotto l'intestazione non e' un DBF leggibile");
        assert_eq!(
            errore.message,
            "offset del primo record DBF piu' corto dell'intestazione"
        );
    }

    #[test]
    fn un_visual_foxpro_piu_corto_del_backlink_e_un_errore_non_un_panico() {
        let errore = __fuzz_leggi_bundle(
            &seme("dbf-visual-foxpro-corto.bundle"),
            opzioni_di_campagna().with_assume_crs("EPSG:4326"),
        )
        .expect_err("263 byte di backlink non stanno in 100");
        assert_eq!(
            errore.message, "header Visual FoxPro piu' corto del backlink",
            "il rifiuto deve venire dal ramo Visual FoxPro: {errore:?}"
        );
    }

    /// Il terzo punto di arresto: il terminatore dei descrittori.
    ///
    /// `dbase` lo pretende con un `debug_assert_eq!`, quindi panica sotto il
    /// fuzzer e **non** in release, dove il file verrebbe letto come se il
    /// terminatore ci fosse. Rifiutarlo rende l'esito lo stesso nelle due
    /// configurazioni, che e' meta' del valore della correzione.
    #[test]
    fn un_terminatore_dell_header_non_valido_e_un_errore_non_un_panico() {
        let errore = __fuzz_leggi_bundle(
            &seme("dbf-terminatore-non-valido.bundle"),
            opzioni_di_campagna().with_assume_crs("EPSG:4326"),
        )
        .expect_err("un DBF senza terminatore non e' leggibile");
        assert_eq!(errore.message, "terminatore header DBF non valido");
    }

    /// Il valore di un campo data: l'unico contenuto -- non descrittore -- che
    /// puo' far panicare il lettore.
    ///
    /// `Date::from_str` affetta la stringa a byte senza guardare ne' la
    /// lunghezza ne' i confini di carattere. Sono due modi di uscirne, e
    /// servono due semi: un valore multibyte e uno piu' corto di otto byte
    /// utili.
    #[test]
    fn un_campo_data_non_interpretabile_e_un_errore_non_un_panico() {
        for nome in ["dbf-data-multibyte.bundle", "dbf-data-corta.bundle"] {
            let errore = __fuzz_leggi_bundle(
                &seme(nome),
                opzioni_di_campagna().with_assume_crs("EPSG:4326"),
            )
            .expect_err("una data che il lettore non sa affettare non e' leggibile");
            assert_eq!(
                errore.message, "campo data DBF che il lettore non puo' interpretare",
                "{nome}"
            );
        }
    }

    /// Una riga **cancellata** porta gli stessi panici di una viva.
    ///
    /// La prevalidazione le saltava, sulla premessa scritta che «un record
    /// cancellato non viene letto: `dbase` salta i suoi byte senza decodificarne
    /// un solo campo». La premessa era falsa e non era mai stata verificata: la
    /// fuzz smoke ha trovato un `.dbf` il cui unico record e' marcato `*`, e il
    /// panico arriva lo stesso attraversando l'apertura del driver.
    ///
    /// Il seme differisce da `dbf-data-corta.bundle` per **un byte** -- lo
    /// spazio iniziale del record diventa `*` -- e cosi' la prova dice quale
    /// proprieta' sta misurando: non «una data corta e' rifiutata», che si sa
    /// gia', ma «il marcatore di cancellazione non compra l'esenzione».
    #[test]
    fn una_data_malformata_in_una_riga_cancellata_e_un_errore_non_un_panico() {
        let errore = __fuzz_leggi_bundle(
            &seme("dbf-data-corta-in-riga-cancellata.bundle"),
            opzioni_di_campagna().with_assume_crs("EPSG:4326"),
        )
        .expect_err("una riga cancellata non esenta il campo dal panico");
        assert_eq!(
            errore.message,
            "campo data DBF che il lettore non puo' interpretare"
        );
    }

    /// Il rovescio: le date valide restano valide, e un campo tutto spazi e'
    /// una data assente, non un rifiuto. E' la meta' che una prevalidazione
    /// sbaglia piu' spesso, e che nessun seme ostile mostrerebbe.
    #[test]
    fn una_data_valida_e_un_campo_vuoto_passano() {
        assert!(!data_non_interpretabile(b"20260101"));
        assert!(!data_non_interpretabile(b"        "));
        assert!(!data_non_interpretabile(&[0; 8]));
        assert!(!data_non_interpretabile(b" 20260101 "));

        assert!(data_non_interpretabile(b"2026    "));
        // Una `e` accentata in UTF-8: due byte, e il taglio a `s[4..6]` cade
        // dentro il secondo.
        assert!(data_non_interpretabile("2026\u{e8}01".as_bytes()));
    }

    /// La prevalidazione vale per **tutte e tre** le versioni che `dbase`
    /// tratta come Visual `FoxPro`, non solo per quella che scriviamo noi.
    ///
    /// `DBF_VISUAL_FOXPRO_VERSION` e' `0x30` perche' descrive i nostri file;
    /// `Version::from(u8)` della crate esterna accetta anche `0x31` e `0x32`, e
    /// un seme con quei byte raggiungerebbe lo stesso `panic!`.
    #[test]
    fn le_tre_versioni_visual_foxpro_sono_tutte_prevalidate() {
        let temporanea = tempfile::tempdir().expect("directory temporanea");
        for versione in DBF_VERSIONI_VISUAL_FOXPRO {
            let percorso = temporanea.path().join(format!("v{versione:02x}.dbf"));
            let mut intestazione = [0_u8; DBF_HEADER_SIZE];
            intestazione[0] = versione;
            // Sotto i 263 byte del backlink, e sopra i 33 dell'intestazione:
            // cosi' a rifiutare puo' essere **solo** il ramo Visual FoxPro.
            intestazione[8..10].copy_from_slice(&100_u16.to_le_bytes());
            std::fs::write(&percorso, intestazione).expect("scrittura del DBF");

            let errore = valida_intestazione_dbf(&percorso)
                .expect_err("i 263 byte di backlink non stanno in 100");
            assert_eq!(
                errore.message, "header Visual FoxPro piu' corto del backlink",
                "versione {versione:#04x}"
            );
        }
    }

    /// **Isolamento fra invocazioni**, provato dal `.prj`.
    ///
    /// Senza `assume_crs` il driver accetta solo se il `.prj` c'e'. La prima
    /// lettura ne scrive uno; la seconda usa un bundle che non ne ha. Se le due
    /// invocazioni condividessero la directory, la seconda troverebbe il `.prj`
    /// della prima e **riuscirebbe**: il fallimento e' la prova che ogni input
    /// ha la propria directory e che i fratelli della mutazione precedente non
    /// sopravvivono.
    #[test]
    fn ogni_invocazione_ha_la_propria_directory() {
        let con_prj = __fuzz_leggi_bundle(&seme("punti-con-prj.bundle"), opzioni_di_campagna());
        assert!(
            con_prj.is_ok(),
            "il `.prj` del bundle deve bastare a risolvere il CRS: {con_prj:?}"
        );

        let senza_prj =
            __fuzz_leggi_bundle(&seme("punti-con-attributi.bundle"), opzioni_di_campagna());
        let errore = senza_prj.expect_err(
            "senza `.prj` e senza `assume_crs` l'apertura deve fallire; se riesce, \
             il `.prj` della lettura precedente e' sopravvissuto",
        );
        assert!(errore.message.contains("assume-crs"), "{errore:?}");
    }

    /// **Fail-closed della divisione**: nessun trabocco, nessuna allocazione
    /// derivata dai valori dichiarati, nessun percorso costruito dal payload.
    #[test]
    fn la_divisione_del_bundle_satura_invece_di_fidarsi() {
        let lunghezze = |dati: &[u8]| {
            let p = __fuzz_dividi_bundle(dati).expect("l'intestazione c'e'");
            (p.shp.len(), p.shx.len(), p.dbf.len(), p.prj.len())
        };

        // Lunghezze massime su un corpo di tre byte: le fette restano dentro il
        // corpo, e non viene riservato niente per i 65 535 dichiarati.
        assert_eq!(
            lunghezze(&[0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 1, 2, 3]),
            (3, 0, 0, 0)
        );

        // Ogni campo dichiara piu' di quanto resti dopo il precedente.
        assert_eq!(
            lunghezze(&[0, 2, 0xFF, 0xFF, 0xFF, 0xFF, 1, 2, 3, 4]),
            (2, 2, 0, 0)
        );

        // Divisione esatta, con un `.prj` che e' il resto. Il confronto e' sui
        // byte e non sulle lunghezze: cosi' uno scambio fra due parti si vede.
        assert_eq!(
            __fuzz_dividi_bundle(&[0, 1, 0, 1, 0, 1, 9, 8, 7, 6, 6]),
            Some(PartiDelBundle {
                shp: &[9],
                shx: &[8],
                dbf: &[7],
                prj: &[6, 6],
            })
        );

        // Un input piu' corto dell'intestazione non e' un bundle.
        for corto in [&b""[..], &b"a"[..], &b"abcde"[..]] {
            assert!(__fuzz_dividi_bundle(corto).is_none(), "{corto:?}");
        }
        // Un bundle senza corpo e' un bundle vuoto, non un errore di divisione.
        assert_eq!(
            __fuzz_dividi_bundle(&[0, 0, 0, 0, 0, 0]),
            Some(PartiDelBundle {
                shp: b"",
                shx: b"",
                dbf: b"",
                prj: b"",
            })
        );
    }

    /// Un bundle degenere non deve panicare: deve tornare `Err`.
    #[test]
    fn un_bundle_degenere_e_un_errore_non_un_panico() {
        for degenere in [vec![], vec![0, 0, 0], vec![0, 0, 0, 0], vec![0xFF; 64]] {
            let esito = __fuzz_leggi_bundle(
                &degenere,
                opzioni_di_campagna().with_assume_crs("EPSG:4326"),
            );
            assert!(esito.is_err(), "{degenere:?} non e' uno Shapefile");
        }
    }

    /// **Errore d'ambiente e non finding**: una radice che non esiste produce
    /// un errore tipizzato, non un panico. Provata sulla funzione che scrive,
    /// perche' forzare il fallimento mutando `TMPDIR` renderebbe il difetto
    /// visibile agli altri test in parallelo.
    #[test]
    fn una_radice_inesistente_e_un_errore_di_ambiente() {
        let temporanea = tempfile::tempdir().expect("directory temporanea");
        let inesistente = temporanea.path().join("mai-creata");

        let parti = PartiDelBundle {
            shp: b"shp",
            shx: b"",
            dbf: b"dbf",
            prj: b"",
        };
        let errore = materializza_bundle(&inesistente, &parti)
            .expect_err("scrivere in una directory che non esiste deve fallire");
        assert!(
            errore.message.contains("ambiente"),
            "un errore d'ambiente non va confuso con un difetto del file letto: {errore:?}"
        );
    }

    /// Opzioni di lettura sul modello unificato.
    ///
    /// Da S4.d il percorso di lettura vive interamente li': la memoria dei
    /// batch e' una `InternalMemoryLease`, che esiste solo dentro un
    /// `PipelineContext`. `opzioni_lettura()` costruisce ancora il ramo
    /// legacy — sparira' in S4.e — e con quello `open` fallisce chiuso.
    /// Opzioni di scrittura sul modello unificato.
    ///
    /// `opzioni_scrittura()` non esiste piu' (S4.e): le opzioni portano un
    /// `OperationBudget`, che nasce da una costruzione che puo' fallire.
    fn opzioni_scrittura() -> WriteOptions {
        match plenora_io_model::budget::PipelineBudget::builder().build() {
            Ok(bundle) => WriteOptions::from_write_parts(bundle.into_write_parts()),
            Err(error) => unreachable!("bundle di test non costruibile: {error:?}"),
        }
    }

    /// Opzioni di scrittura con i limiti dati.
    ///
    /// I tetti stanno nel budget, non nelle opzioni: passarli da qui e' l'unico
    /// modo di provare il rifiuto sul tetto **e** l'accettazione sotto di esso
    /// con lo stesso lotto.
    fn opzioni_scrittura_con(limits: plenora_io_model::budget::PipelineLimits) -> WriteOptions {
        match plenora_io_model::budget::PipelineBudget::builder()
            .limits(limits)
            .build()
        {
            Ok(bundle) => WriteOptions::from_write_parts(bundle.into_write_parts()),
            Err(error) => unreachable!("bundle di test non costruibile: {error:?}"),
        }
    }

    /// Opzioni di scrittura che **accettano** il set di file sciolti.
    ///
    /// Le prove che pubblicano su `*.shp` la usano al posto di
    /// `opzioni_scrittura`: da questa revisione una destinazione `*.shp` non
    /// deduce piu' la forma debole, la pretende dichiarata. Che le prove
    /// debbano dichiararla e' il segno che il rifiuto funziona -- se potessero
    /// continuare come prima, non funzionerebbe.
    fn opzioni_scrittura_loose() -> WriteOptions {
        opzioni_scrittura().with_format_option("publish_mode", LOOSE_SET_MODE)
    }

    fn opzioni_lettura() -> ReadOptions {
        match plenora_io_model::budget::PipelineBudget::builder().build() {
            Ok(bundle) => ReadOptions::from_read_parts(bundle.into_read_parts()),
            Err(error) => unreachable!("bundle di test non costruibile: {error:?}"),
        }
    }

    use std::io::Write as _;

    use plenora_io_core::request::{BatchTarget, ProjectionMode};
    use plenora_io_core::WriteLayer;
    use plenora_io_model::wkb::to_wkb;
    use plenora_io_model::CancellationToken;

    const EPSG_3003_WKT: &str = include_str!("../tests/fixtures/epsg3003.prj");

    fn read_opts() -> ReadOptions {
        opzioni_lettura().with_assume_crs("EPSG:4326")
    }

    fn req() -> ReadRequest {
        ReadRequest {
            layer: LayerId(0),
            projected_fields: None,
            projection_mode: ProjectionMode::BestEffort,
            pruning_predicate: None,
            spatial_pruning_hint: None,
            scope: ReadScope::default(),
            batch_target: BatchTarget::default(),
            cancellation: CancellationToken::default(),
        }
    }

    fn make_polygon_ring_unclosed(path: &Path, target_record: usize) {
        let mut file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)
            .unwrap();
        let mut record_offset = 100_u64;
        for record_index in 0..=target_record {
            file.seek(SeekFrom::Start(record_offset)).unwrap();
            let mut record_header = [0_u8; 8];
            file.read_exact(&mut record_header).unwrap();
            let content_bytes =
                u64::from(u32::from_be_bytes(record_header[4..8].try_into().unwrap())) * 2;
            let body_offset = record_offset + 8;
            if record_index == target_record {
                file.seek(SeekFrom::Start(body_offset + 36)).unwrap();
                let mut counts = [0_u8; 8];
                file.read_exact(&mut counts).unwrap();
                let part_count = u64::from(u32::from_le_bytes(counts[0..4].try_into().unwrap()));
                let point_count = u64::from(u32::from_le_bytes(counts[4..8].try_into().unwrap()));
                assert!(part_count > 0 && point_count > 1);
                let points_offset = body_offset + 44 + part_count * 4;
                let last_x_offset = points_offset + (point_count - 1) * 16;
                file.seek(SeekFrom::Start(last_x_offset)).unwrap();
                file.write_all(&1.0_f64.to_le_bytes()).unwrap();
                return;
            }
            record_offset += 8 + content_bytes;
        }
        panic!("record Shapefile {target_record} inesistente");
    }

    fn truncate_dbf_mid_record(path: &Path, complete_records: u64) {
        let dbf_path = path.with_extension("dbf");
        let header = std::fs::read(&dbf_path).unwrap();
        let header_length = u64::from(u16::from_le_bytes(header[8..10].try_into().unwrap()));
        let record_length = u64::from(u16::from_le_bytes(header[10..12].try_into().unwrap()));
        assert!(record_length > 1);
        let truncated_length = header_length + complete_records * record_length + record_length / 2;
        std::fs::OpenOptions::new()
            .write(true)
            .open(dbf_path)
            .unwrap()
            .set_len(truncated_length)
            .unwrap();
    }

    fn mark_dbf_record_deleted(path: &Path, source_index: u64) {
        let dbf_path = path.with_extension("dbf");
        let header = std::fs::read(&dbf_path).unwrap();
        let header_length = u64::from(u16::from_le_bytes(header[8..10].try_into().unwrap()));
        let record_length = u64::from(u16::from_le_bytes(header[10..12].try_into().unwrap()));
        let marker_offset = header_length + source_index * record_length;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .open(dbf_path)
            .unwrap();
        file.seek(SeekFrom::Start(marker_offset)).unwrap();
        file.write_all(b"*").unwrap();
    }

    fn overwrite_dbf_ascii_field(path: &Path, source_index: u64, field_name: &str, value: &str) {
        let dbf_path = path.with_extension("dbf");
        let mut file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(dbf_path)
            .unwrap();
        let mut header = [0_u8; 32];
        file.read_exact(&mut header).unwrap();
        let header_length = u64::from(u16::from_le_bytes(header[8..10].try_into().unwrap()));
        let record_length = u64::from(u16::from_le_bytes(header[10..12].try_into().unwrap()));
        let mut descriptor_offset = 32_u64;
        let mut field_offset = 1_u64;
        loop {
            file.seek(SeekFrom::Start(descriptor_offset)).unwrap();
            let mut descriptor = [0_u8; 32];
            file.read_exact(&mut descriptor).unwrap();
            assert_ne!(descriptor[0], 0x0d, "campo DBF non trovato");
            let name_end = descriptor[..11]
                .iter()
                .position(|byte| *byte == 0)
                .map_or(11, |position| position);
            let name = std::str::from_utf8(&descriptor[..name_end]).unwrap();
            let width = usize::from(descriptor[16]);
            if name == field_name {
                assert!(value.len() <= width);
                let mut encoded = vec![b' '; width];
                encoded[width - value.len()..].copy_from_slice(value.as_bytes());
                let record_offset = header_length + source_index * record_length + field_offset;
                file.seek(SeekFrom::Start(record_offset)).unwrap();
                file.write_all(&encoded).unwrap();
                return;
            }
            field_offset += width as u64;
            descriptor_offset += 32;
        }
    }

    fn consume_until_error(reader: &mut dyn LayerReader) -> (usize, PlenoraIoError) {
        let mut emitted_rows = 0;
        loop {
            match reader.next_batch() {
                Ok(Some(batch)) => emitted_rows += batch.num_rows(),
                Ok(None) => panic!("atteso rifiuto row-scoped"),
                Err(error) => return (emitted_rows, error),
            }
        }
    }

    #[test]
    fn degenerate_polygon_rings_have_a_stable_rejection_cause() {
        let repeated = Point::new(1.0, 1.0);
        let rings = vec![PolygonRing::Outer(vec![
            repeated, repeated, repeated, repeated,
        ])];

        assert_eq!(polygon_rejection_cause(&rings), Some(DEGENERATE_RING_CAUSE));
    }

    // Una sola fixture copre scrittura, corruzioni mirate e le varianti di
    // configurazione della diagnostica: separarle duplicherebbe la costruzione
    // dello shapefile e ne perderebbe la sequenza.
    #[allow(clippy::too_many_lines)]
    #[test]
    fn invalid_polygon_rows_return_complete_bounded_diagnostics() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("invalid-polygons.shp");
        let key_name = shapefile::dbase::FieldName::try_from("ID_PART").unwrap();
        let numeric_key_name = shapefile::dbase::FieldName::try_from("NUM_KEY").unwrap();
        let integer_value_name = shapefile::dbase::FieldName::try_from("INT_VALUE").unwrap();
        let table = TableWriterBuilder::new()
            .add_character_field(key_name, 32)
            .add_numeric_field(numeric_key_name, 20, 2)
            .add_numeric_field(integer_value_name, 18, 0);
        let mut writer = Writer::from_path(&path, table).unwrap();

        let key_base = 9_007_199_254_740_992_u64;
        for source_index in 0..128 {
            let points = vec![
                Point::new(0.0, 0.0),
                Point::new(0.0, 5.0),
                Point::new(5.0, 5.0),
                Point::new(5.0, 0.0),
                Point::new(0.0, 0.0),
            ];
            let rings = if matches!(source_index, 17 | 113) {
                vec![PolygonRing::Inner(points)]
            } else {
                vec![PolygonRing::Outer(points)]
            };
            let polygon = Polygon::with_rings(rings);
            // source_index < 128: la conversione in f64 e' esatta.
            #[allow(clippy::cast_precision_loss)]
            let numeric_value = source_index as f64;
            let mut record = Record::default();
            record.insert(
                "ID_PART".to_owned(),
                FieldValue::Character(Some((key_base + source_index).to_string())),
            );
            record.insert(
                "NUM_KEY".to_owned(),
                FieldValue::Numeric(Some(numeric_value)),
            );
            record.insert(
                "INT_VALUE".to_owned(),
                FieldValue::Numeric(Some(numeric_value)),
            );
            writer.write_shape_and_record(&polygon, &record).unwrap();
        }
        drop(writer);
        make_polygon_ring_unclosed(&path, 89);
        mark_dbf_record_deleted(&path, 20);
        for source_index in [17_u64, 89, 113] {
            overwrite_dbf_ascii_field(
                &path,
                source_index,
                "NUM_KEY",
                &format!("{}.25", key_base + source_index),
            );
        }
        std::fs::write(path.with_extension("prj"), EPSG_3003_WKT).unwrap();
        let malformed_directory = tempfile::tempdir().unwrap();
        let malformed_path = malformed_directory.path().join("invalid-attribute.shp");
        for extension in ["shp", "shx", "dbf", "prj"] {
            std::fs::copy(
                path.with_extension(extension),
                malformed_path.with_extension(extension),
            )
            .unwrap();
        }

        let mut options = read_opts();
        options
            .format_options
            .insert("row_diagnostics.examples_limit".to_owned(), "2".to_owned());
        options
            .format_options
            .insert("row_diagnostics.key_field".to_owned(), "ID_PART".to_owned());
        options
            .format_options
            .insert("row_diagnostics.key_policy".to_owned(), "emit".to_owned());
        let dataset = ShpDriver.open(Source::Path(path.clone()), options).unwrap();
        let request = ReadRequest {
            batch_target: BatchTarget {
                target_bytes: 8 * 1024 * 1024,
                max_rows: 8,
            },
            ..req()
        };
        let mut reader = dataset.open_layer_reader(&request).unwrap();
        let (emitted_rows, error) = consume_until_error(reader.as_mut());
        assert_eq!(emitted_rows, 0);
        let diagnostics = error
            .row_diagnostics
            .expect("diagnostica row-scoped mancante");
        assert_eq!(diagnostics.observed_total, 3);
        assert_eq!(diagnostics.total, Some(3));
        assert_eq!(
            diagnostics.counts.get("shapefile.inner_ring_without_outer"),
            Some(&2)
        );
        assert_eq!(diagnostics.counts.get("shapefile.unclosed_ring"), Some(&1));
        assert_eq!(diagnostics.examples_limit, 2);
        assert!(diagnostics.examples_truncated);
        assert_eq!(diagnostics.examples.len(), 2);
        assert_eq!(diagnostics.examples[0].source_index, 17);
        assert_eq!(diagnostics.examples[1].source_index, 89);
        assert_eq!(
            diagnostics.examples[0]
                .key
                .as_ref()
                .and_then(|key| key.value.as_ref()),
            Some(&plenora_io_model::RowDiagnosticKeyValue::String(
                (key_base + 17).to_string()
            ))
        );
        assert_eq!(
            diagnostics.examples[1]
                .key
                .as_ref()
                .and_then(|key| key.value.as_ref()),
            Some(&plenora_io_model::RowDiagnosticKeyValue::String(
                (key_base + 89).to_string()
            ))
        );

        let mut attribute_only_request = req();
        attribute_only_request.projected_fields = Some(vec![FieldId(1)]);
        attribute_only_request.batch_target = BatchTarget {
            target_bytes: 8 * 1024 * 1024,
            max_rows: 8,
        };
        let attribute_only_dataset = ShpDriver
            .open(Source::Path(path.clone()), read_opts())
            .unwrap();
        let mut attribute_only_reader = attribute_only_dataset
            .open_layer_reader(&attribute_only_request)
            .unwrap();
        let (attribute_rows, attribute_error) = consume_until_error(attribute_only_reader.as_mut());
        assert_eq!(attribute_rows, 0);
        assert_eq!(attribute_error.row_diagnostics.unwrap().observed_total, 3);

        let mut numeric_key_options = read_opts();
        numeric_key_options
            .format_options
            .insert("row_diagnostics.examples_limit".to_owned(), "2".to_owned());
        numeric_key_options
            .format_options
            .insert("row_diagnostics.key_field".to_owned(), "NUM_KEY".to_owned());
        numeric_key_options
            .format_options
            .insert("row_diagnostics.key_policy".to_owned(), "emit".to_owned());
        let numeric_key_dataset = ShpDriver
            .open(Source::Path(path.clone()), numeric_key_options)
            .unwrap();
        let mut numeric_key_reader = numeric_key_dataset.open_layer_reader(&request).unwrap();
        let (_, numeric_key_error) = consume_until_error(numeric_key_reader.as_mut());
        let numeric_examples = numeric_key_error.row_diagnostics.unwrap().examples;
        for (example, source_index) in numeric_examples.iter().zip([17_u64, 89]) {
            assert_eq!(
                example.key.as_ref().and_then(|key| key.value.as_ref()),
                Some(&plenora_io_model::RowDiagnosticKeyValue::String(format!(
                    "{}.25",
                    key_base + source_index
                )))
            );
        }

        let dataset_without_key = ShpDriver
            .open(Source::Path(path.clone()), read_opts())
            .unwrap();
        let mut reader_without_key = dataset_without_key.open_layer_reader(&request).unwrap();
        let (_, error_without_key) = consume_until_error(reader_without_key.as_mut());
        assert!(error_without_key
            .row_diagnostics
            .unwrap()
            .examples
            .iter()
            .all(|example| example.key.is_none()));

        let mut redacted_options = read_opts();
        redacted_options
            .format_options
            .insert("row_diagnostics.key_field".to_owned(), "ID_PART".to_owned());
        redacted_options
            .format_options
            .insert("row_diagnostics.key_policy".to_owned(), "redact".to_owned());
        let redacted_dataset = ShpDriver
            .open(Source::Path(path.clone()), redacted_options)
            .unwrap();
        let mut redacted_reader = redacted_dataset.open_layer_reader(&request).unwrap();
        let (_, redacted_error) = consume_until_error(redacted_reader.as_mut());
        let redacted_key = redacted_error.row_diagnostics.unwrap().examples[0]
            .key
            .clone()
            .unwrap();
        assert_eq!(redacted_key.state, RowDiagnosticKeyState::Redacted);
        assert!(redacted_key.value.is_none());

        let mut missing_policy = read_opts();
        missing_policy
            .format_options
            .insert("row_diagnostics.key_field".to_owned(), "ID_PART".to_owned());
        let Err(missing_policy_error) = ShpDriver.open(Source::Path(path.clone()), missing_policy)
        else {
            panic!("key_field senza policy deve essere rifiutato")
        };
        assert_eq!(
            missing_policy_error.category,
            plenora_io_model::ErrorCategory::InvalidConfiguration
        );

        let mut zero_limit = read_opts();
        zero_limit
            .format_options
            .insert("row_diagnostics.examples_limit".to_owned(), "0".to_owned());
        let Err(zero_limit_error) = ShpDriver.open(Source::Path(path.clone()), zero_limit) else {
            panic!("examples_limit zero deve essere rifiutato")
        };
        assert_eq!(
            zero_limit_error.category,
            plenora_io_model::ErrorCategory::InvalidConfiguration
        );

        let mut cancelled_diagnostics = ShpRowDiagnostics::new(ShpRowDiagnosticsConfig {
            examples_limit: 1,
            key: None,
        });
        cancelled_diagnostics.record(
            17,
            INNER_RING_WITHOUT_OUTER_CAUSE,
            Some(&Record::default()),
            None,
        );
        let cancellation = plenora_io_model::CancellationToken::default();
        cancellation.cancel();
        let cancellation_error =
            plenora_io_core::check_cancelled(&cancellation, plenora_io_model::ErrorPhase::Read)
                .expect_err("la cancellation richiesta deve essere osservata");
        let cancelled = cancelled_diagnostics
            .into_partial_error(cancellation_error, "shapefile.scan_cancelled")
            .row_diagnostics
            .unwrap();
        assert_eq!(cancelled.completeness, RowDiagnosticsCompleteness::Partial);
        assert_eq!(
            cancelled.knowledge_limits,
            Some(vec!["shapefile.scan_cancelled".to_owned()])
        );
        assert!(cancelled.total.is_none());

        let partial_dataset = ShpDriver
            .open(Source::Path(path.clone()), read_opts())
            .unwrap();
        truncate_dbf_mid_record(&path, 50);
        let mut partial_reader = partial_dataset.open_layer_reader(&request).unwrap();
        let (_, partial_error) = consume_until_error(partial_reader.as_mut());
        let partial = partial_error.row_diagnostics.unwrap();
        assert_eq!(partial.completeness, RowDiagnosticsCompleteness::Partial);
        assert_eq!(partial.total, None);
        assert_eq!(partial.observed_total, 1);
        assert_eq!(
            partial.knowledge_limits,
            Some(vec!["shapefile.dbf_exact_scan_interrupted".to_owned()])
        );
        assert_eq!(
            partial.counts.get("shapefile.inner_ring_without_outer"),
            Some(&1)
        );
        assert_eq!(partial.examples.len(), 1);
        assert!(!partial.examples_truncated);

        overwrite_dbf_ascii_field(&malformed_path, 42, "INT_VALUE", "not-an-integer");
        let malformed_dataset = ShpDriver
            .open(Source::Path(malformed_path), read_opts())
            .unwrap();
        let mut malformed_reader = malformed_dataset.open_layer_reader(&request).unwrap();
        let (_, malformed_error) = consume_until_error(malformed_reader.as_mut());
        let malformed = malformed_error.row_diagnostics.unwrap();
        assert_eq!(malformed.completeness, RowDiagnosticsCompleteness::Complete);
        assert_eq!(malformed.observed_total, 4);
        assert_eq!(
            malformed.counts.get(ATTRIBUTE_NUMERIC_INVALID_CAUSE),
            Some(&1)
        );
        assert!(malformed.examples.iter().any(|example| {
            example.source_index == 42 && example.cause == ATTRIBUTE_NUMERIC_INVALID_CAUSE
        }));
    }

    #[test]
    fn accepted_rows_stops_invalid_shapefile_scan_at_active_row_limit() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("bounded-invalid.shp");
        let id = shapefile::dbase::FieldName::try_from("ID").unwrap();
        let table = TableWriterBuilder::new().add_numeric_field(id, 9, 0);
        let mut writer = Writer::from_path(&path, table).unwrap();
        for source_index in 0..4_096 {
            let points = vec![
                Point::new(0.0, 0.0),
                Point::new(0.0, 5.0),
                Point::new(5.0, 5.0),
                Point::new(5.0, 0.0),
                Point::new(0.0, 0.0),
            ];
            let rings = if matches!(source_index, 17 | 89 | 3_000) {
                vec![PolygonRing::Inner(points)]
            } else {
                vec![PolygonRing::Outer(points)]
            };
            let mut record = Record::default();
            record.insert(
                "ID".to_owned(),
                FieldValue::Numeric(Some(f64::from(source_index))),
            );
            writer
                .write_shape_and_record(&Polygon::with_rings(rings), &record)
                .unwrap();
        }
        drop(writer);
        mark_dbf_record_deleted(&path, 20);
        std::fs::write(path.with_extension("prj"), EPSG_3003_WKT).unwrap();

        // Fault-tail deterministico: il dataset e' inferito quando integro, poi
        // la coda oltre il prefisso richiesto diventa illeggibile.
        let dataset = ShpDriver
            .open(Source::Path(path.clone()), read_opts())
            .unwrap();
        truncate_dbf_mid_record(&path, 200);
        let request = ReadRequest {
            scope: plenora_io_core::ReadScope::AcceptedRows(32),
            batch_target: BatchTarget {
                target_bytes: 8 * 1024 * 1024,
                max_rows: 8,
            },
            ..req()
        };
        let mut reader = dataset.open_layer_reader(&request).unwrap();
        let (emitted_rows, error) = consume_until_error(reader.as_mut());
        assert_eq!(emitted_rows, 0);
        let diagnostics = error.row_diagnostics.as_deref().unwrap();
        assert_eq!(
            diagnostics.completeness,
            RowDiagnosticsCompleteness::Partial
        );
        assert_eq!(diagnostics.total, None);
        assert_eq!(diagnostics.observed_total, 1);
        assert_eq!(diagnostics.counts[INNER_RING_WITHOUT_OUTER_CAUSE], 1);
        assert_eq!(diagnostics.examples[0].source_index, 17);
        assert_eq!(
            diagnostics.knowledge_limits.as_deref(),
            Some(["read_scope_row_limit_reached".to_owned()].as_slice())
        );
        assert!(diagnostics.validate().is_ok());
    }

    #[test]
    fn accepted_rows_preserves_valid_shapefile_batch_overshoot_and_skips_late_invalidity() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("late-invalid.shp");
        let id = shapefile::dbase::FieldName::try_from("ID").unwrap();
        let table = TableWriterBuilder::new().add_numeric_field(id, 9, 0);
        let mut writer = Writer::from_path(&path, table).unwrap();
        for source_index in 0..25 {
            let points = vec![
                Point::new(0.0, 0.0),
                Point::new(0.0, 5.0),
                Point::new(5.0, 5.0),
                Point::new(5.0, 0.0),
                Point::new(0.0, 0.0),
            ];
            let rings = if source_index == 20 {
                vec![PolygonRing::Inner(points)]
            } else {
                vec![PolygonRing::Outer(points)]
            };
            let mut record = Record::default();
            record.insert(
                "ID".to_owned(),
                FieldValue::Numeric(Some(f64::from(source_index))),
            );
            writer
                .write_shape_and_record(&Polygon::with_rings(rings), &record)
                .unwrap();
        }
        drop(writer);
        std::fs::write(path.with_extension("prj"), EPSG_3003_WKT).unwrap();

        let dataset = ShpDriver
            .open(Source::Path(path.clone()), read_opts())
            .unwrap();
        let request = ReadRequest {
            scope: ReadScope::AcceptedRows(10),
            batch_target: BatchTarget {
                target_bytes: 8 * 1024 * 1024,
                max_rows: 8,
            },
            ..req()
        };
        let mut reader = dataset.open_layer_reader(&request).unwrap();
        let mut rows = Vec::new();
        while let Some(batch) = reader.next_batch().unwrap() {
            rows.push(batch.num_rows());
        }
        assert_eq!(rows, vec![8, 8]);

        let complete_dataset = ShpDriver.open(Source::Path(path), read_opts()).unwrap();
        // L'accoppiamento su cui poggia il `field_index` degli esempi: lo schema
        // mette la geometria **davanti** alle colonne DBF, e l'indice pubblicato
        // e' `posizione in cols + 1`. Se l'ordine cambiasse, quegli indici
        // indicherebbero la colonna sbagliata e nessuno se ne accorgerebbe.
        {
            let contratto = &complete_dataset.layers()[0];
            assert_eq!(
                contratto.contract.schema.field(0).name(),
                &contratto
                    .contract
                    .geometry
                    .as_ref()
                    .expect("uno shapefile ha una geometria")
                    .name,
                "la geometria deve essere il primo campo dello schema"
            );
        }
        let mut complete_request = request;
        complete_request.scope = ReadScope::Complete;
        let mut complete = complete_dataset
            .open_layer_reader(&complete_request)
            .unwrap();
        let (published_rows, error) = consume_until_error(complete.as_mut());
        assert_eq!(published_rows, 0);
        let diagnostics = error.row_diagnostics.as_deref().unwrap();
        assert_eq!(
            diagnostics.completeness,
            RowDiagnosticsCompleteness::Complete
        );
        assert_eq!(diagnostics.examples[0].source_index, 20);
        assert_eq!(diagnostics.total, Some(1));
    }

    #[test]
    fn prj_authority_is_resolved_and_keeps_epsg_axis_order() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("roads.shp");
        std::fs::write(
            path.with_extension("prj"),
            "GEOGCS[\"WGS 84\",AUTHORITY[\"EPSG\",\"4326\"]]",
        )
        .unwrap();

        let crs = resolve_crs(&path, &opzioni_lettura()).unwrap();
        assert_eq!(crs.id.as_deref(), Some("EPSG:4326"));
        assert_eq!(
            crs.axis_order,
            plenora_io_model::crs::AxisOrder::LatitudeLongitude
        );
    }

    #[test]
    fn projected_prj_with_nested_geogcs_keeps_projected_kind_and_axis_order() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("parcels.shp");
        std::fs::write(path.with_extension("prj"), EPSG_3003_WKT).unwrap();

        let crs = resolve_crs(&path, &opzioni_lettura()).unwrap();
        assert_eq!(crs.id.as_deref(), Some("EPSG:3003"));
        assert_eq!(crs.kind, CrsKind::Projected);
        assert_eq!(
            crs.axis_order,
            plenora_io_model::crs::AxisOrder::EastingNorthing
        );
    }

    #[test]
    fn wide_zero_decimal_dbf_numeric_is_read_exactly_as_i64() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("parcels.shp");
        let field_name = shapefile::dbase::FieldName::try_from("parcel_id").unwrap();
        let table = TableWriterBuilder::new().add_numeric_field(field_name, 18, 0);
        let mut writer = Writer::from_path(&path, table).unwrap();
        for coordinate in [0.0, 1.0] {
            let mut record = Record::default();
            record.insert("parcel_id".to_owned(), FieldValue::Numeric(Some(0.0)));
            writer
                .write_shape_and_record(&Point::new(coordinate, coordinate), &record)
                .unwrap();
        }
        drop(writer);
        std::fs::write(path.with_extension("prj"), EPSG_3003_WKT).unwrap();

        // Il writer dbase accetta già f64. Si sostituiscono i byte del campo
        // con due interi ASCII distinti per riprodurre un DBF patrimoniale
        // reale prima che dbase 0.5.0 li converta nello stesso f64.
        let mut dbf = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(path.with_extension("dbf"))
            .unwrap();
        let mut header = [0_u8; 32];
        dbf.read_exact(&mut header).unwrap();
        let header_length = u64::from(u16::from_le_bytes([header[8], header[9]]));
        let record_length = u64::from(u16::from_le_bytes([header[10], header[11]]));
        for (row, value) in ["9007199254740992", "9007199254740993"]
            .into_iter()
            .enumerate()
        {
            dbf.seek(SeekFrom::Start(
                header_length + (row as u64 * record_length) + 1,
            ))
            .unwrap();
            dbf.write_all(format!("{value:>18}").as_bytes()).unwrap();
        }
        drop(dbf);

        let dataset = ShpDriver
            .open(Source::Path(path), opzioni_lettura())
            .unwrap();
        let assessment = dataset.fidelity_assessment();
        assert_eq!(assessment.level, Fidelity::Conditional);

        let mut reader = dataset.open_layer_reader(&req()).unwrap();
        let loss = reader.loss_report();
        assert!(!loss
            .counts
            .contains_key(DBF_NUMERIC_INTEGER_PRECISION_UNVERIFIABLE));
        let batch = reader.next_batch().unwrap().unwrap();
        let ids = batch
            .column(1)
            .as_any()
            .downcast_ref::<arrow_array::Int64Array>()
            .unwrap();
        assert_eq!(ids.value(0), 9_007_199_254_740_992);
        assert_eq!(ids.value(1), 9_007_199_254_740_993);
    }

    #[test]
    fn narrow_or_decimal_dbf_numeric_keeps_float_mapping() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("numeric-shapes.shp");
        let narrow = shapefile::dbase::FieldName::try_from("narrow").unwrap();
        let decimal = shapefile::dbase::FieldName::try_from("decimal").unwrap();
        let table = TableWriterBuilder::new()
            .add_numeric_field(narrow, 9, 0)
            .add_numeric_field(decimal, 18, 2);
        let mut writer = Writer::from_path(&path, table).unwrap();
        let mut record = Record::default();
        record.insert("narrow".to_owned(), FieldValue::Numeric(Some(123.0)));
        record.insert("decimal".to_owned(), FieldValue::Numeric(Some(12.5)));
        writer
            .write_shape_and_record(&Point::new(0.0, 0.0), &record)
            .unwrap();
        drop(writer);
        std::fs::write(path.with_extension("prj"), EPSG_3003_WKT).unwrap();

        let dataset = ShpDriver
            .open(Source::Path(path), opzioni_lettura())
            .unwrap();
        let schema = &dataset.layers()[0].contract.schema;
        assert_eq!(schema.field(1).data_type(), &DataType::Float64);
        assert_eq!(schema.field(2).data_type(), &DataType::Float64);
    }

    #[test]
    fn duplicate_dbf_field_names_are_rejected_before_record_map_collapse() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("duplicates.shp");
        let first = shapefile::dbase::FieldName::try_from("first").unwrap();
        let second = shapefile::dbase::FieldName::try_from("second").unwrap();
        let table = TableWriterBuilder::new()
            .add_character_field(first, 16)
            .add_character_field(second, 16);
        let mut writer = Writer::from_path(&path, table).unwrap();
        let mut record = Record::default();
        record.insert(
            "first".to_owned(),
            FieldValue::Character(Some("a".to_owned())),
        );
        record.insert(
            "second".to_owned(),
            FieldValue::Character(Some("b".to_owned())),
        );
        writer
            .write_shape_and_record(&Point::new(0.0, 0.0), &record)
            .unwrap();
        drop(writer);
        std::fs::write(path.with_extension("prj"), EPSG_3003_WKT).unwrap();

        let mut dbf = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(path.with_extension("dbf"))
            .unwrap();
        dbf.seek(SeekFrom::Start(
            (DBF_HEADER_SIZE + DBF_FIELD_DESCRIPTOR_SIZE) as u64,
        ))
        .unwrap();
        let mut duplicate = [0_u8; DBF_FIELD_NAME_SIZE];
        duplicate[..5].copy_from_slice(b"first");
        dbf.write_all(&duplicate).unwrap();
        drop(dbf);

        let error = ShpDriver
            .open(Source::Path(path), opzioni_lettura())
            .err()
            .expect("il DBF con nomi duplicati deve essere rifiutato");
        assert!(error.to_string().contains("nomi campo DBF duplicati"));
    }

    #[test]
    fn unresolved_prj_is_preserved_in_typed_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("local.shp");
        let definition = "LOCAL_CS[\"survey-grid-secret\"]";
        std::fs::write(path.with_extension("prj"), definition).unwrap();

        let error = resolve_crs(&path, &opzioni_lettura()).unwrap_err();
        assert_eq!(error.code, plenora_io_model::IoErrorCode::CrsUnresolved);
        assert_eq!(error.driver.as_deref(), Some("shp"));
        assert!(!error.to_string().contains("survey-grid-secret"));
    }

    #[test]
    fn assumed_unknown_epsg_does_not_invent_an_axis_order() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("no-prj.shp");
        let crs = resolve_crs(&path, &opzioni_lettura().with_assume_crs("EPSG:4258")).unwrap();
        assert_eq!(crs.kind, CrsKind::Unknown);
        assert_eq!(crs.axis_order, plenora_io_model::crs::AxisOrder::Unknown);
    }

    #[test]
    fn resolved_crs_without_id_cannot_be_relabelled_as_unknown() {
        let crs = ResolvedCrs::new(
            None,
            CrsKind::Unknown,
            Some("LOCAL_CS[\"private\"]".to_owned()),
        );

        assert!(matches!(
            resolved_crs_id(&crs),
            Err(error) if error.code == plenora_io_model::IoErrorCode::Crs
        ));
    }

    #[test]
    fn write_then_read_round_trip() {
        use arrow_array::{Int64Array, StringArray};
        use arrow_schema::DataType;

        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("pts.shp");

        let wkb1 = to_wkb(&geo_types::Geometry::Point(geo_types::Point::new(
            12.5, 45.9,
        )))
        .unwrap();
        let wkb2 = to_wkb(&geo_types::Geometry::Point(geo_types::Point::new(
            9.19, 45.46,
        )))
        .unwrap();
        let schema: SchemaRef = Arc::new(Schema::new(vec![
            geometry_field(GEOMETRY, "EPSG:4326"),
            Field::new("nome", DataType::Utf8, true),
            Field::new("pop", DataType::Int64, true),
        ]));
        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![
                Arc::new(BinaryArray::from(vec![
                    Some(wkb1.as_slice()),
                    Some(wkb2.as_slice()),
                ])),
                Arc::new(StringArray::from(vec!["Roma", "Milano"])),
                Arc::new(Int64Array::from(vec![2_800_000i64, 1_400_000])),
            ],
        )
        .unwrap();

        let driver = ShpDriver;
        let plan = WritePlan {
            layers: vec![WriteLayer {
                name: "l".to_owned(),
                contract: DataContract {
                    schema,
                    geometry: None,
                },
            }],
        };
        let mut w = driver
            .create(Sink::Path(out.clone()), &plan, &opzioni_scrittura_loose())
            .unwrap();
        w.write(&batch).unwrap();
        w.finish().unwrap();

        // il set è stato pubblicato
        assert!(out.exists());
        assert!(out.with_extension("dbf").exists());
        assert!(out.with_extension("prj").exists());

        // rilettura
        let ds = driver.open(Source::Path(out), read_opts()).unwrap();
        let mut r = ds.open_layer_reader(&req()).unwrap();
        let rb = r.next_batch().unwrap().unwrap();
        assert_eq!(rb.num_rows(), 2);
        let nome = rb
            .column_by_name("nome")
            .unwrap()
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap();
        assert_eq!(nome.value(0), "Roma");
    }

    #[test]
    fn writer_adapter_attributes_mixed_geometry_and_prevents_publish() {
        let dir = tempfile::tempdir().unwrap();
        let output = dir.path().join("mixed.shp");
        let point = to_wkb(&geo_types::Geometry::Point(geo_types::Point::new(1.0, 2.0))).unwrap();
        let line = to_wkb(&geo_types::Geometry::LineString(
            geo_types::LineString::from(vec![(0.0, 0.0), (1.0, 1.0)]),
        ))
        .unwrap();
        let schema: SchemaRef = Arc::new(Schema::new(vec![geometry_field(GEOMETRY, "EPSG:4326")]));
        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![Arc::new(BinaryArray::from(vec![
                Some(point.as_slice()),
                Some(line.as_slice()),
            ]))],
        )
        .unwrap();
        let plan = WritePlan {
            layers: vec![WriteLayer {
                name: "mixed".to_owned(),
                contract: DataContract {
                    schema,
                    geometry: None,
                },
            }],
        };
        let mut writer = ShpDriver
            .create(
                Sink::Path(output.clone()),
                &plan,
                &opzioni_scrittura_loose(),
            )
            .unwrap();
        writer.declare_input_total(LayerId(0), 2).unwrap();

        let error = writer.write(&batch).unwrap_err();
        let diagnostics = error.row_diagnostics.as_deref().unwrap();
        assert_eq!(diagnostics.input_total, Some(2));
        assert_eq!(diagnostics.examples[0].source_index, 1);
        assert_eq!(diagnostics.counts["shapefile.mixed_geometry_type"], 1);
        assert!(diagnostics.validate().is_ok());
        assert!(writer.finish().is_err());
        assert!(!output.exists());
        assert!(!output.with_extension("dbf").exists());
    }

    #[test]
    fn directory_dataset_round_trip_uses_atomic_directory_unit() {
        use arrow_array::Int64Array;
        use arrow_schema::DataType;

        let root = tempfile::tempdir().unwrap();
        let output = root.path().join("points.shp.d");
        let wkb = to_wkb(&geo_types::Geometry::Point(geo_types::Point::new(
            12.5, 45.9,
        )))
        .unwrap();
        let schema: SchemaRef = Arc::new(Schema::new(vec![
            geometry_field(GEOMETRY, "EPSG:4326"),
            Field::new("id", DataType::Int64, false),
        ]));
        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![
                Arc::new(BinaryArray::from(vec![Some(wkb.as_slice())])),
                Arc::new(Int64Array::from(vec![1_i64])),
            ],
        )
        .unwrap();
        let plan = WritePlan {
            layers: vec![WriteLayer {
                name: "points".to_owned(),
                contract: DataContract {
                    schema,
                    geometry: None,
                },
            }],
        };
        let options = opzioni_scrittura()
            .with_durable(true)
            .with_format_option("publish_mode", DIRECTORY_DATASET_MODE);

        let driver = ShpDriver;
        let mut writer = driver
            .create(Sink::Path(output.clone()), &plan, &options)
            .unwrap();
        writer.write(&batch).unwrap();
        assert!(
            !output.exists(),
            "la directory dataset è diventata visibile prima di finish"
        );
        let published = writer.finish().unwrap();

        let expected_outcome = if cfg!(unix) {
            plenora_io_core::PublishOutcome::Published
        } else {
            plenora_io_core::PublishOutcome::PublishedButDurabilityUnconfirmed
        };
        assert_eq!(published.outcome, expected_outcome);
        assert!(output.is_dir());
        assert!(output.join("data.shp").is_file());
        assert!(output.join("data.shx").is_file());
        assert!(output.join("data.dbf").is_file());
        assert!(output.join("data.prj").is_file());

        let dataset = driver.open(Source::Path(output), read_opts()).unwrap();
        let mut reader = dataset.open_layer_reader(&req()).unwrap();
        assert_eq!(reader.next_batch().unwrap().unwrap().num_rows(), 1);
    }

    #[test]
    fn directory_dataset_abort_removes_staging() {
        let root = tempfile::tempdir().unwrap();
        let output = root.path().join("aborted.shp.d");
        let schema: SchemaRef = Arc::new(Schema::new(vec![geometry_field(GEOMETRY, "EPSG:4326")]));
        let plan = WritePlan {
            layers: vec![WriteLayer {
                name: "points".to_owned(),
                contract: DataContract {
                    schema,
                    geometry: None,
                },
            }],
        };

        let writer = ShpDriver
            .create(Sink::Path(output.clone()), &plan, &opzioni_scrittura())
            .unwrap();
        drop(writer);

        assert!(!output.exists());
        assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
    }

    // --- ASSURANCE-N1: i rami negativi della conversione WKB -> topologia ---
    //
    // Tre gruppi del censimento, chiusi insieme perche' sono la stessa
    // superficie: `topology_from_wkb` decide, e delega a `take_child` e
    // `polygon_rings`. Il replay del fuzzer li raggiungeva gia' -- reachability
    // e panic-safety erano provate -- ma nessuna prova diceva **quale** rifiuto
    // arriva per **quale** input, che e' il contratto.

    /// Una coordinata XY, che e' l'unica forma che queste sonde usano.
    fn xy(x: f64, y: f64) -> WkbCoordinate {
        WkbCoordinate {
            x,
            y,
            z: None,
            m: None,
        }
    }

    /// Un anello quadrato e chiuso: quattro coordinate, la prima uguale
    /// all'ultima.
    fn anello_valido() -> Vec<WkbCoordinate> {
        vec![xy(0.0, 0.0), xy(1.0, 0.0), xy(1.0, 1.0), xy(0.0, 0.0)]
    }

    fn geometria(value: WkbValue) -> WkbGeometry {
        WkbGeometry {
            value,
            dimensions: CoordinateDimensions::Xy,
            srid: None,
        }
    }

    /// Ogni rifiuto di `topology_from_wkb`, con l'input che lo produce.
    ///
    /// La sonda non verifica «fallisce»: verifica **quale** messaggio arriva.
    /// Un rifiuto giusto per la ragione sbagliata manda chi legge l'errore a
    /// correggere la cosa sbagliata, ed e' indistinguibile da quello giusto se
    /// la sonda si accontenta di `is_err()`.
    // La lunghezza e' nel numero di casi, non in complessita' logica: la
    // funzione e' una tabella, e spezzarla in due meta' arbitrarie renderebbe
    // piu' difficile vedere che i casi coprono ogni rifiuto.
    #[allow(clippy::too_many_lines)]
    #[test]
    fn n1_topology_from_wkb_rifiuta_ogni_forma_non_rappresentabile() {
        let casi: Vec<(&str, WkbGeometry, &str)> = vec![
            (
                "SRID incorporato",
                WkbGeometry {
                    value: WkbValue::Point(xy(1.0, 2.0)),
                    dimensions: CoordinateDimensions::Xy,
                    srid: Some(4326),
                },
                "SRID embedded",
            ),
            (
                "MultiPoint vuoto",
                geometria(WkbValue::MultiPoint(Vec::new())),
                "MultiPoint vuoto",
            ),
            (
                "LineString con una sola coordinata",
                geometria(WkbValue::LineString(vec![xy(0.0, 0.0)])),
                "LineString con meno di due coordinate",
            ),
            (
                "MultiLineString vuoto",
                geometria(WkbValue::MultiLineString(Vec::new())),
                "MultiLineString vuoto",
            ),
            (
                "parte di MultiLineString con una sola coordinata",
                geometria(WkbValue::MultiLineString(vec![geometria(
                    WkbValue::LineString(vec![xy(0.0, 0.0)]),
                )])),
                "parte LineString con meno di due coordinate",
            ),
            (
                "MultiPolygon vuoto",
                geometria(WkbValue::MultiPolygon(Vec::new())),
                "MultiPolygon vuoto",
            ),
            (
                "GeometryCollection",
                geometria(WkbValue::GeometryCollection(Vec::new())),
                "GeometryCollection non rappresentabile",
            ),
            (
                "tipo esteso: CircularString",
                geometria(WkbValue::CircularString(Vec::new())),
                "tipo WKB esteso",
            ),
            (
                "tipo esteso: Triangle",
                geometria(WkbValue::Triangle(Vec::new())),
                "tipo WKB esteso",
            ),
            (
                "poligono senza anelli",
                geometria(WkbValue::Polygon(Vec::new())),
                "poligono vuoto",
            ),
            (
                "anello con meno di quattro coordinate",
                geometria(WkbValue::Polygon(vec![vec![
                    xy(0.0, 0.0),
                    xy(1.0, 0.0),
                    xy(0.0, 0.0),
                ]])),
                "anello WKB non chiuso o con meno di quattro coordinate",
            ),
            (
                "anello non chiuso",
                geometria(WkbValue::Polygon(vec![vec![
                    xy(0.0, 0.0),
                    xy(1.0, 0.0),
                    xy(1.0, 1.0),
                    xy(0.0, 1.0),
                ]])),
                "anello WKB non chiuso o con meno di quattro coordinate",
            ),
            (
                "membro di MultiPoint con SRID proprio",
                geometria(WkbValue::MultiPoint(vec![WkbGeometry {
                    value: WkbValue::Point(xy(1.0, 2.0)),
                    dimensions: CoordinateDimensions::Xy,
                    srid: Some(4326),
                }])),
                "geometria WKB annidata incoerente",
            ),
            (
                "membro di MultiPoint con dimensioni diverse dal padre",
                geometria(WkbValue::MultiPoint(vec![WkbGeometry {
                    value: WkbValue::Point(xy(1.0, 2.0)),
                    dimensions: CoordinateDimensions::Xyz,
                    srid: None,
                }])),
                "geometria WKB annidata incoerente",
            ),
            (
                "membro di MultiPoint che non e' un Point",
                geometria(WkbValue::MultiPoint(vec![geometria(WkbValue::LineString(
                    vec![xy(0.0, 0.0), xy(1.0, 1.0)],
                ))])),
                "geometria WKB annidata incoerente",
            ),
            (
                "membro di MultiLineString che non e' una LineString",
                geometria(WkbValue::MultiLineString(vec![geometria(WkbValue::Point(
                    xy(0.0, 0.0),
                ))])),
                "geometria WKB annidata incoerente",
            ),
            (
                "membro di MultiPolygon che non e' un Polygon",
                geometria(WkbValue::MultiPolygon(vec![geometria(WkbValue::Point(
                    xy(0.0, 0.0),
                ))])),
                "geometria WKB annidata incoerente",
            ),
            (
                "anello non chiuso dentro un MultiPolygon",
                geometria(WkbValue::MultiPolygon(vec![geometria(WkbValue::Polygon(
                    vec![vec![xy(0.0, 0.0), xy(1.0, 0.0), xy(1.0, 1.0), xy(0.0, 1.0)]],
                ))])),
                "anello WKB non chiuso",
            ),
        ];

        for (nome, ingresso, atteso) in casi {
            // `let ... else` e non `unwrap_or_else(|| panic!(...))`: la seconda
            // forma **sembra** un fallback -- il registro di H-01 conta
            // `unwrap_or*` per sintassi -- mentre qui non c'e' nessun valore di
            // ripiego, solo un'asserzione che diverge.
            let Err(errore) = topology_from_wkb(ingresso) else {
                panic!("«{nome}» doveva essere rifiutato e non lo e' stato");
            };
            assert!(
                errore.message.contains(atteso),
                "«{nome}»: atteso un rifiuto che nomina «{atteso}», arrivato «{}»",
                errore.message
            );
        }
    }

    /// Le forme che **devono** passare, accanto a quelle che non devono.
    ///
    /// Senza questo verso, un `topology_from_wkb` che rifiutasse tutto
    /// supererebbe la sonda dei rifiuti: e' il modo piu' rapido di far passare
    /// una tabella di casi negativi.
    #[test]
    fn n1_topology_from_wkb_accetta_le_forme_rappresentabili() {
        let casi: Vec<(&str, WkbValue)> = vec![
            ("punto", WkbValue::Point(xy(1.0, 2.0))),
            (
                "multipunto con un membro",
                WkbValue::MultiPoint(vec![geometria(WkbValue::Point(xy(1.0, 2.0)))]),
            ),
            (
                "polilinea di due coordinate",
                WkbValue::LineString(vec![xy(0.0, 0.0), xy(1.0, 1.0)]),
            ),
            (
                "multipolilinea con una parte",
                WkbValue::MultiLineString(vec![geometria(WkbValue::LineString(vec![
                    xy(0.0, 0.0),
                    xy(1.0, 1.0),
                ]))]),
            ),
            (
                "poligono con un anello chiuso",
                WkbValue::Polygon(vec![anello_valido()]),
            ),
            (
                "multipoligono con un membro",
                WkbValue::MultiPolygon(vec![geometria(WkbValue::Polygon(vec![anello_valido()]))]),
            ),
        ];

        for (nome, valore) in casi {
            assert!(
                topology_from_wkb(geometria(valore)).is_ok(),
                "«{nome}» doveva essere accettato"
            );
        }
    }

    /// Il primo anello e' l'esterno, gli altri sono interni.
    ///
    /// E' l'unica affermazione **positiva** di `polygon_rings`, e senza di lei
    /// la funzione potrebbe marcare tutti gli anelli allo stesso modo senza che
    /// nessuna sonda se ne accorga: i rifiuti resterebbero tutti verdi.
    #[test]
    fn n1_polygon_rings_marca_esterno_solo_il_primo_anello() {
        let mut destinazione = Vec::new();
        polygon_rings(
            vec![anello_valido(), anello_valido(), anello_valido()],
            &mut destinazione,
        )
        .expect("tre anelli chiusi sono validi");

        let esterni: Vec<bool> = destinazione.iter().map(|(esterno, _)| *esterno).collect();
        assert_eq!(esterni, vec![true, false, false]);
    }

    /// `publish_mode`: ogni combinazione di destinazione e opzione, e il suo esito.
    ///
    /// Il gruppo era censito come «ramo semantico negativo mai eseguito da
    /// nulla». Da allora la forma sciolta e' diventata un opt-in esplicito, e
    /// tre sonde ne provano i rifiuti passando dalla `create`; questa tabella
    /// chiude il gruppo alla sua origine, chiamando la funzione direttamente.
    ///
    /// Otto casi: le **sei** combinazioni fra i due suffissi supportati e i tre
    /// stati dell'opzione, piu' **due** classi di destinazione invalida --
    /// un'estensione estranea e un percorso senza estensione. Non e' «ogni
    /// coppia possibile», che sarebbe piu' ampio di cio' che la tabella
    /// enumera.
    #[test]
    fn n1_publish_mode_decide_per_ogni_coppia_di_destinazione_e_opzione() {
        let casi: Vec<(&str, &str, Option<&str>, Option<ShapefilePublishMode>)> = vec![
            (
                "*.shp.d senza opzione deduce il directory-dataset",
                "dati.shp.d",
                None,
                Some(ShapefilePublishMode::DirectoryDataset),
            ),
            (
                "*.shp.d con l'opzione che lo conferma",
                "dati.shp.d",
                Some(DIRECTORY_DATASET_MODE),
                Some(ShapefilePublishMode::DirectoryDataset),
            ),
            (
                "*.shp.d con l'opzione del set sciolto: contraddizione",
                "dati.shp.d",
                Some(LOOSE_SET_MODE),
                None,
            ),
            (
                "*.shp senza opzione: il set sciolto non si deduce",
                "dati.shp",
                None,
                None,
            ),
            (
                "*.shp con l'opt-in esplicito",
                "dati.shp",
                Some(LOOSE_SET_MODE),
                Some(ShapefilePublishMode::LooseSet),
            ),
            (
                "*.shp con l'opzione del directory-dataset: contraddizione",
                "dati.shp",
                Some(DIRECTORY_DATASET_MODE),
                None,
            ),
            (
                "un'estensione che non e' ne' l'una ne' l'altra",
                "dati.dbf",
                None,
                None,
            ),
            ("nessuna estensione", "dati", Some(LOOSE_SET_MODE), None),
        ];

        for (nome, destinazione, opzione, atteso) in casi {
            let mut opzioni = opzioni_scrittura();
            if let Some(valore) = opzione {
                opzioni = opzioni.with_format_option("publish_mode", valore);
            }
            let esito = publish_mode(Path::new(destinazione), &opzioni);
            match (esito, atteso) {
                (Ok(ottenuto), Some(voluto)) => assert_eq!(ottenuto, voluto, "«{nome}»"),
                (Err(_), None) => {}
                (Ok(ottenuto), None) => {
                    panic!("«{nome}» doveva essere rifiutato, ha dato {ottenuto:?}")
                }
                (Err(errore), Some(_)) => panic!("«{nome}» doveva passare: {}", errore.message),
            }
        }
    }

    /// `shapefile_source_path`: i due rifiuti di una sorgente che e' una directory.
    ///
    /// Un file qualunque passa senza domande -- e' il caso comune, e la
    /// funzione non lo tocca. Una **directory** invece deve chiamarsi `*.shp.d`
    /// e contenere `data.shp`, e i due rifiuti sono distinti: il primo dice che
    /// la directory non e' un dataset, il secondo che lo e' ma e' incompleta.
    #[test]
    fn n1_shapefile_source_path_distingue_i_due_rifiuti_di_una_directory() {
        let radice = tempfile::tempdir().unwrap();

        let file = radice.path().join("dati.shp");
        std::fs::write(&file, b"non importa").unwrap();
        assert_eq!(
            shapefile_source_path(file.clone()).unwrap(),
            file,
            "un file non e' una directory: passa cosi' com'e'"
        );

        let inesistente = radice.path().join("assente.shp");
        assert_eq!(
            shapefile_source_path(inesistente.clone()).unwrap(),
            inesistente,
            "nemmeno un percorso inesistente e' una directory"
        );

        let non_dataset = radice.path().join("una-directory");
        std::fs::create_dir(&non_dataset).unwrap();
        let errore = shapefile_source_path(non_dataset)
            .expect_err("una directory senza il suffisso non e' un dataset");
        assert!(errore
            .message
            .contains("directory Shapefile non riconosciuta"));

        let vuoto = radice.path().join("vuoto.shp.d");
        std::fs::create_dir(&vuoto).unwrap();
        let errore = shapefile_source_path(vuoto.clone())
            .expect_err("un dataset senza data.shp e' incompleto");
        assert!(errore.message.contains("directory dataset senza data.shp"));

        std::fs::write(vuoto.join("data.shp"), b"non importa").unwrap();
        assert_eq!(
            shapefile_source_path(vuoto.clone()).unwrap(),
            vuoto.join("data.shp"),
            "con data.shp il dataset risolve al proprio marker"
        );
    }

    /// `declare_input_total`: il rifiuto del driver e' **irraggiungibile**, e
    /// questa sonda dice da chi.
    ///
    /// Il censimento lo dava «chiudibile con un test parametrico sulla classe
    /// di equivalenza della sua precondizione». Scritto quel test, il messaggio
    /// che arriva non e' quello di `ShpWriter` -- «Shapefile supporta un solo
    /// layer» -- ma quello del wrapper comune, che confronta il layer con il
    /// `WritePlan` **prima** di delegare.
    ///
    /// Non e' un dettaglio da nota a pie' di pagina: significa che quelle due
    /// righe del driver non sono raggiungibili dall'API pubblica, e che un test
    /// che si accontentasse di `is_err()` le avrebbe dichiarate coperte
    /// asserendo il rifiuto di **qualcun altro**. La sonda assevera percio' la
    /// firma di chi rifiuta, non la sola presenza di un errore.
    ///
    /// Il verso positivo sta accanto perche' senza di lui un
    /// `declare_input_total` che rifiutasse **ogni** layer supererebbe la sonda
    /// del rifiuto.
    #[test]
    fn n1_declare_input_total_e_fermato_dal_piano_prima_del_driver() {
        let radice = tempfile::tempdir().unwrap();
        let uscita = radice.path().join("punti.shp");
        let piano = piano_di_publish();
        let mut writer = ShpDriver
            .create(Sink::Path(uscita), &piano, &opzioni_scrittura_loose())
            .unwrap();

        writer
            .declare_input_total(LayerId(0), 3)
            .expect("il layer zero e' nel piano, e va accettato");

        for indice in [1_u32, 2, u32::MAX] {
            let Err(errore) = writer.declare_input_total(LayerId(indice), 3) else {
                panic!("il layer {indice} doveva essere rifiutato");
            };
            assert!(
                errore.message.contains("fuori dal WritePlan"),
                "layer {indice}: a rifiutare doveva essere il piano, non il driver;                  messaggio «{}»",
                errore.message
            );
            assert!(
                !errore.message.contains("Shapefile supporta un solo layer"),
                "layer {indice}: e' arrivato il rifiuto del driver, quindi la guardia                  del piano non lo precede piu' e il gruppo va rivisto"
            );
        }
    }

    /// `header_geometry`: ogni `ShapeType` che l'header puo' dichiarare.
    ///
    /// Tredici forme si traducono, una sola no. La tabella le enumera **tutte**
    /// invece di provare solo il rifiuto: un `header_geometry` che sbagliasse la
    /// traduzione di `PolygonZ` -- dicendo `MultiLineString` invece di
    /// `MultiPolygon` -- lascerebbe verde una sonda che guarda il solo
    /// Multipatch, e produrrebbe un contratto di layer sbagliato su ogni
    /// poligono 3D letto.
    #[test]
    fn n1_header_geometry_traduce_ogni_shape_type_e_rifiuta_multipatch() {
        let casi: Vec<(ShapeType, Option<&str>, Vec<GeometryType>)> = vec![
            (ShapeType::NullShape, None, Vec::new()),
            (
                ShapeType::Point,
                Some("point-xy"),
                vec![GeometryType::Point],
            ),
            (
                ShapeType::PointM,
                Some("point-m"),
                vec![GeometryType::Point],
            ),
            (
                ShapeType::PointZ,
                Some("point-z"),
                vec![GeometryType::Point],
            ),
            (
                ShapeType::Polyline,
                Some("polyline-xy"),
                vec![GeometryType::MultiLineString],
            ),
            (
                ShapeType::PolylineM,
                Some("polyline-m"),
                vec![GeometryType::MultiLineString],
            ),
            (
                ShapeType::PolylineZ,
                Some("polyline-z"),
                vec![GeometryType::MultiLineString],
            ),
            (
                ShapeType::Polygon,
                Some("polygon-xy"),
                vec![GeometryType::MultiPolygon],
            ),
            (
                ShapeType::PolygonM,
                Some("polygon-m"),
                vec![GeometryType::MultiPolygon],
            ),
            (
                ShapeType::PolygonZ,
                Some("polygon-z"),
                vec![GeometryType::MultiPolygon],
            ),
            (
                ShapeType::Multipoint,
                Some("multipoint-xy"),
                vec![GeometryType::MultiPoint],
            ),
            (
                ShapeType::MultipointM,
                Some("multipoint-m"),
                vec![GeometryType::MultiPoint],
            ),
            (
                ShapeType::MultipointZ,
                Some("multipoint-z"),
                vec![GeometryType::MultiPoint],
            ),
        ];

        for (shape_type, etichetta, tipi) in casi {
            let Ok((etichetta_letta, tipi_letti)) = header_geometry(shape_type) else {
                panic!("{shape_type:?} e' rappresentabile e non doveva essere rifiutato");
            };
            assert_eq!(etichetta_letta, etichetta, "etichetta di {shape_type:?}");
            assert_eq!(tipi_letti, tipi, "tipi geometrici di {shape_type:?}");
        }

        let Err(errore) = header_geometry(ShapeType::Multipatch) else {
            panic!("Multipatch non e' rappresentabile e deve essere rifiutato");
        };
        assert!(errore.message.contains("Multipatch"));
    }

    /// `resolve_crs`: le quattro vie di un CRS, e le due che finiscono in errore.
    ///
    /// La funzione decide su due assi -- il `.prj` c'e' o no, `--assume-crs` c'e'
    /// o no -- e i quattro incroci non danno quattro esiti uguali: con il `.prj`
    /// l'identificatore puo' arrivare dall'opzione **o** dal WKT, e se non
    /// arriva da nessuno dei due il rifiuto e' `CrsUnresolved`, che e' diverso
    /// dal rifiuto senza `.prj`.
    #[test]
    fn n1_resolve_crs_copre_i_quattro_incroci_di_prj_e_assume_crs() {
        let radice = tempfile::tempdir().unwrap();
        let shp = radice.path().join("dati.shp");
        std::fs::write(&shp, b"non letto da resolve_crs").unwrap();
        let prj = radice.path().join("dati.prj");

        // 1. niente `.prj`, niente `--assume-crs`: rifiuto esplicito.
        let Err(errore) = resolve_crs(&shp, &opzioni_lettura()) else {
            panic!("senza .prj e senza --assume-crs il CRS non e' deducibile");
        };
        assert!(errore.message.contains("senza .prj"));

        // 2. niente `.prj`, ma `--assume-crs`: l'opzione decide.
        let mut con_opzione = opzioni_lettura();
        con_opzione.assume_crs = Some("EPSG:4326".to_owned());
        let risolto = resolve_crs(&shp, &con_opzione).expect("l'opzione risolve da sola");
        assert_eq!(risolto.id.as_deref(), Some("EPSG:4326"));
        assert!(
            risolto.definition.is_none(),
            "senza .prj non c'e' WKT da conservare"
        );

        // 3. `.prj` con un WKT da cui l'autorita' si ricava.
        std::fs::write(&prj, "GEOGCS[\"WGS 84\",AUTHORITY[\"EPSG\",\"4326\"]]").unwrap();
        let risolto = resolve_crs(&shp, &opzioni_lettura()).expect("l'autorita' e' nel WKT");
        assert_eq!(risolto.id.as_deref(), Some("EPSG:4326"));
        assert!(
            risolto.definition.is_some(),
            "il WKT del .prj va conservato: e' la fonte"
        );

        // 4. `.prj` con un WKT senza autorita' e senza opzione: non risolto.
        std::fs::write(&prj, "LOCAL_CS[\"senza autorita\"]").unwrap();
        let Err(errore) = resolve_crs(&shp, &opzioni_lettura()) else {
            panic!("un WKT senza autorita' e senza opzione non e' risolvibile");
        };
        assert_eq!(errore.code, plenora_io_model::IoErrorCode::CrsUnresolved);

        // 5. lo stesso WKT, ma con `--assume-crs`: l'opzione vince sul silenzio
        //    del WKT, e il WKT resta conservato accanto.
        let risolto = resolve_crs(&shp, &con_opzione).expect("l'opzione copre il WKT muto");
        assert_eq!(risolto.id.as_deref(), Some("EPSG:4326"));
        assert!(risolto.definition.is_some());
    }

    /// Il piano minimo delle prove sul publish: una sola colonna geometria.
    fn piano_di_publish() -> WritePlan {
        let schema: SchemaRef = Arc::new(Schema::new(vec![geometry_field(GEOMETRY, "EPSG:4326")]));
        WritePlan {
            layers: vec![WriteLayer {
                name: "points".to_owned(),
                contract: DataContract {
                    schema,
                    geometry: None,
                },
            }],
        }
    }

    /// Una destinazione `*.shp` non deduce piu' la forma debole: la pretende.
    ///
    /// La sonda guarda due cose insieme, e la seconda conta quanto la prima: il
    /// rifiuto arriva **prima** che qualunque cosa tocchi il disco. Un rifiuto
    /// che lasciasse dietro di se' uno staging avrebbe gia' fatto il danno che
    /// esiste per evitare.
    #[test]
    fn una_destinazione_shp_senza_opt_in_e_rifiutata_prima_dello_staging() {
        let root = tempfile::tempdir().unwrap();

        let errore = ShpDriver
            .create(
                Sink::Path(root.path().join("points.shp")),
                &piano_di_publish(),
                &opzioni_scrittura(),
            )
            .map(|_| ())
            .unwrap_err();

        // `InvalidConfiguration` e non `Unsupported`: il prodotto sa fare questa
        // scrittura, e' la richiesta a essere incompleta. Chi automatizza deve
        // aggiungere un'opzione, non cambiare driver.
        assert_eq!(
            errore.category,
            plenora_io_model::ErrorCategory::InvalidConfiguration
        );
        assert_eq!(errore.phase, plenora_io_model::ErrorPhase::Validate);
        // Il messaggio nomina entrambe le uscite: quella che accetta il rischio
        // e quella che non lo corre.
        assert!(errore.message.contains(LOOSE_SET_MODE));
        assert!(errore.message.contains(".shp.d"));
        assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
    }

    /// Accettata l'opzione, il set sciolto si pubblica come prima.
    ///
    /// Il rifiuto e' una domanda, non un divieto: chi risponde ottiene i quattro
    /// file dove li ha chiesti.
    #[test]
    fn una_destinazione_shp_con_opt_in_pubblica_il_set_sciolto() {
        let root = tempfile::tempdir().unwrap();
        let uscita = root.path().join("points.shp");
        let piano = piano_di_publish();
        let batch = RecordBatch::new_empty(piano.layers[0].contract.schema.clone());

        let mut writer = ShpDriver
            .create(
                Sink::Path(uscita.clone()),
                &piano,
                &opzioni_scrittura_loose(),
            )
            .unwrap();
        writer.write(&batch).unwrap();
        writer.finish().unwrap();

        assert!(uscita.is_file());
        assert!(uscita.with_extension("shx").is_file());
        assert!(uscita.with_extension("dbf").is_file());
        // Nessuna directory di staging sopravvive al publish.
        assert!(std::fs::read_dir(root.path())
            .unwrap()
            .all(|voce| voce.unwrap().path().is_file()));
    }

    /// La contraddizione e' un errore anche nel verso opposto.
    ///
    /// Chiedere il set sciolto su una destinazione `*.shp.d` non e' «accettare il
    /// rischio»: e' descrivere male la destinazione, e vale il rifiuto come il
    /// verso gia' coperto.
    #[test]
    fn il_set_sciolto_chiesto_su_una_directory_dataset_e_rifiutato() {
        let root = tempfile::tempdir().unwrap();

        let errore = ShpDriver
            .create(
                Sink::Path(root.path().join("points.shp.d")),
                &piano_di_publish(),
                &opzioni_scrittura_loose(),
            )
            .map(|_| ())
            .unwrap_err();

        assert_eq!(errore.code, plenora_io_model::IoErrorCode::Unsupported);
        // Il messaggio nomina il suffisso che il modo **chiesto** pretende, e
        // deve nominare quello: `contains("*.shp")` da solo sarebbe vero anche
        // per `*.shp.d`, cioe' non distinguerebbe i due versi del rifiuto.
        assert!(errore.message.ends_with("*.shp"), "{}", errore.message);
        assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
    }

    #[test]
    fn publish_mode_must_match_destination_shape() {
        let root = tempfile::tempdir().unwrap();
        let schema: SchemaRef = Arc::new(Schema::new(vec![geometry_field(GEOMETRY, "EPSG:4326")]));
        let plan = WritePlan {
            layers: vec![WriteLayer {
                name: "points".to_owned(),
                contract: DataContract {
                    schema,
                    geometry: None,
                },
            }],
        };
        let mut options = opzioni_scrittura();
        options
            .format_options
            .insert("publish_mode".to_owned(), DIRECTORY_DATASET_MODE.to_owned());

        let result = ShpDriver
            .create(Sink::Path(root.path().join("points.shp")), &plan, &options)
            .map(|_| ());

        assert!(matches!(
            result,
            Err(error) if error.code == plenora_io_model::IoErrorCode::Unsupported
        ));
        assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
    }

    #[test]
    fn rejects_long_field_name() {
        use arrow_schema::DataType;
        let schema: SchemaRef = Arc::new(Schema::new(vec![
            geometry_field(GEOMETRY, "EPSG:4326"),
            Field::new("nome_campo_troppo_lungo", DataType::Utf8, true),
        ]));
        let driver = ShpDriver;
        let plan = WritePlan {
            layers: vec![WriteLayer {
                name: "l".to_owned(),
                contract: DataContract {
                    schema,
                    geometry: None,
                },
            }],
        };
        let dir = tempfile::tempdir().unwrap();
        let e = driver
            .create(
                Sink::Path(dir.path().join("x.shp")),
                &plan,
                &opzioni_scrittura(),
            )
            .map(|_| ())
            .unwrap_err();
        assert_eq!(
            e.capability_reason,
            Some(plenora_io_model::CapabilityReason::FieldNameTooLong)
        );
    }

    #[test]
    fn streams_multiple_batches() {
        use arrow_array::Int64Array;
        use arrow_schema::DataType;

        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("many.shp");
        let wkb: Vec<Vec<u8>> = (0..10)
            .map(|i| {
                to_wkb(&geo_types::Geometry::Point(geo_types::Point::new(
                    f64::from(i),
                    f64::from(i),
                )))
                .unwrap()
            })
            .collect();
        let schema: SchemaRef = Arc::new(Schema::new(vec![
            geometry_field(GEOMETRY, "EPSG:4326"),
            Field::new("id", DataType::Int64, true),
        ]));
        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![
                Arc::new(BinaryArray::from(
                    wkb.iter().map(|w| Some(w.as_slice())).collect::<Vec<_>>(),
                )),
                Arc::new(Int64Array::from((0..10i64).collect::<Vec<_>>())),
            ],
        )
        .unwrap();

        let driver = ShpDriver;
        let plan = WritePlan {
            layers: vec![WriteLayer {
                name: "l".to_owned(),
                contract: DataContract {
                    schema,
                    geometry: None,
                },
            }],
        };
        let mut w = driver
            .create(Sink::Path(out.clone()), &plan, &opzioni_scrittura_loose())
            .unwrap();
        w.write(&batch).unwrap();
        w.finish().unwrap();

        let ds = driver.open(Source::Path(out), read_opts()).unwrap();
        let req = ReadRequest {
            layer: LayerId(0),
            projected_fields: None,
            projection_mode: ProjectionMode::BestEffort,
            pruning_predicate: None,
            spatial_pruning_hint: None,
            scope: ReadScope::default(),
            batch_target: BatchTarget {
                target_bytes: 8 * 1024 * 1024,
                max_rows: 4,
            },
            cancellation: CancellationToken::default(),
        };
        let mut r = ds.open_layer_reader(&req).unwrap();
        let (mut total, mut batches) = (0, 0);
        while let Some(b) = r.next_batch().unwrap() {
            total += b.num_rows();
            batches += 1;
        }
        assert_eq!(total, 10);
        assert!(
            batches >= 3,
            "atteso streaming multi-batch, avuti {batches}"
        );
    }

    fn dimensional_point(
        dimensions: CoordinateDimensions,
        z: Option<f64>,
        m: Option<f64>,
    ) -> WkbGeometry {
        WkbGeometry {
            value: WkbValue::Point(WkbCoordinate {
                x: 12.5,
                y: 45.9,
                z,
                m,
            }),
            dimensions,
            srid: None,
        }
    }

    fn round_trip_dimensional_point(
        dimensions: CoordinateDimensions,
        geometry: &WkbGeometry,
    ) -> WkbGeometry {
        use arrow_array::Int64Array;

        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join(format!("{dimensions:?}.shp"));
        let bytes = encode_wkb(geometry, WkbFlavor::Iso).unwrap();
        let mut geometry_contract =
            GeometryColumnContract::wkb_xy(FieldId(0), GEOMETRY, ResolvedCrs::wgs84(), false);
        geometry_contract.dimensions = dimensions;
        geometry_contract.set_exact_geometry_types(vec![GeometryType::Point]);
        let schema: SchemaRef = Arc::new(Schema::new(vec![
            with_geometry_contract_metadata(
                &geometry_field(GEOMETRY, "EPSG:4326"),
                &geometry_contract,
            ),
            Field::new("id", DataType::Int64, false),
        ]));
        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![
                Arc::new(BinaryArray::from(vec![Some(bytes.as_slice())])),
                Arc::new(Int64Array::from(vec![1_i64])),
            ],
        )
        .unwrap();
        let plan = WritePlan {
            layers: vec![WriteLayer {
                name: "points".to_owned(),
                contract: DataContract {
                    schema,
                    geometry: Some(geometry_contract),
                },
            }],
        };

        let driver = ShpDriver;
        let mut writer = driver
            .create(Sink::Path(out.clone()), &plan, &opzioni_scrittura_loose())
            .unwrap();
        writer.write(&batch).unwrap();
        writer.finish().unwrap();

        let mut native = shapefile::Reader::from_path(&out).unwrap();
        let (shape, _) = native.iter_shapes_and_records().next().unwrap().unwrap();
        match dimensions {
            CoordinateDimensions::Xym => assert!(matches!(shape, Shape::PointM(_))),
            CoordinateDimensions::Xyz | CoordinateDimensions::Xyzm => {
                assert!(matches!(shape, Shape::PointZ(_)));
            }
            _ => unreachable!("test solo dimensionale"),
        }

        let dataset = driver.open(Source::Path(out), read_opts()).unwrap();
        let layer = &dataset.layers()[0];
        let output_contract = layer.contract.geometry.as_ref().unwrap();
        assert_eq!(output_contract.dimensions, dimensions);
        assert_eq!(output_contract.geometry_types, vec![GeometryType::Point]);
        assert!(output_contract
            .native_metadata
            .contains_key("shp.shape_type"));
        let mut reader = dataset.open_layer_reader(&req()).unwrap();
        let batch = reader.next_batch().unwrap().unwrap();
        let geometry = batch
            .column(0)
            .as_any()
            .downcast_ref::<BinaryArray>()
            .unwrap();
        decode_wkb(geometry.value(0), &WkbLimits::default()).unwrap()
    }

    #[test]
    fn round_trip_preserves_xyz_xym_and_xyzm_points() {
        let cases = [
            dimensional_point(CoordinateDimensions::Xyz, Some(123.25), None),
            dimensional_point(CoordinateDimensions::Xym, None, Some(7.5)),
            dimensional_point(CoordinateDimensions::Xyzm, Some(123.25), Some(7.5)),
        ];
        for expected in cases {
            let actual = round_trip_dimensional_point(expected.dimensions, &expected);
            assert_eq!(actual, expected);
        }
    }

    #[test]
    fn direct_conversion_preserves_xyzm_multiline_and_no_data_measure() {
        let dimensions = CoordinateDimensions::Xyzm;
        let line = WkbGeometry {
            value: WkbValue::MultiLineString(vec![WkbGeometry {
                value: WkbValue::LineString(vec![
                    WkbCoordinate {
                        x: 0.0,
                        y: 1.0,
                        z: Some(2.0),
                        m: Some(NO_DATA),
                    },
                    WkbCoordinate {
                        x: 3.0,
                        y: 4.0,
                        z: Some(5.0),
                        m: Some(6.0),
                    },
                ]),
                dimensions,
                srid: None,
            }]),
            dimensions,
            srid: None,
        };
        let shape = shape_from_wkb(line.clone()).unwrap();
        assert!(matches!(shape, Shape::PolylineZ(_)));
        let decoded = shape_to_wkb(&shape, dimensions).unwrap().unwrap();
        assert_eq!(decoded, line);
    }

    #[test]
    fn direct_conversion_preserves_xyzm_multipolygon_rings() {
        let dimensions = CoordinateDimensions::Xyzm;
        let coordinate = |x, y, z, m| WkbCoordinate {
            x,
            y,
            z: Some(z),
            m: Some(m),
        };
        let exterior = vec![
            coordinate(0.0, 0.0, 1.0, 10.0),
            coordinate(0.0, 5.0, 2.0, 11.0),
            coordinate(5.0, 5.0, 3.0, 12.0),
            coordinate(5.0, 0.0, 4.0, 13.0),
            coordinate(0.0, 0.0, 1.0, 10.0),
        ];
        let interior = vec![
            coordinate(1.0, 1.0, 5.0, 14.0),
            coordinate(4.0, 1.0, 6.0, 15.0),
            coordinate(4.0, 4.0, 7.0, 16.0),
            coordinate(1.0, 4.0, 8.0, 17.0),
            coordinate(1.0, 1.0, 5.0, 14.0),
        ];
        let polygon = WkbGeometry {
            value: WkbValue::MultiPolygon(vec![WkbGeometry {
                value: WkbValue::Polygon(vec![exterior, interior]),
                dimensions,
                srid: None,
            }]),
            dimensions,
            srid: None,
        };
        let shape = shape_from_wkb(polygon.clone()).unwrap();
        assert!(matches!(shape, Shape::PolygonZ(_)));
        let decoded = shape_to_wkb(&shape, dimensions).unwrap().unwrap();
        assert_eq!(decoded, polygon);
    }

    #[test]
    fn geometry_collection_is_rejected_without_xy_normalization() {
        let geometry = WkbGeometry {
            value: WkbValue::GeometryCollection(Vec::new()),
            dimensions: CoordinateDimensions::Xyz,
            srid: None,
        };
        assert!(shape_from_wkb(geometry).is_err());
    }

    #[test]
    fn declared_dimensions_without_required_ordinates_are_rejected() {
        let missing_z = WkbGeometry {
            value: WkbValue::Point(WkbCoordinate {
                x: 1.0,
                y: 2.0,
                z: None,
                m: None,
            }),
            dimensions: CoordinateDimensions::Xyz,
            srid: None,
        };
        let missing_m = WkbGeometry {
            value: WkbValue::Point(WkbCoordinate {
                x: 1.0,
                y: 2.0,
                z: Some(3.0),
                m: None,
            }),
            dimensions: CoordinateDimensions::Xyzm,
            srid: None,
        };

        assert!(shape_from_wkb(missing_z).is_err());
        assert!(shape_from_wkb(missing_m).is_err());
    }

    // --- ASSURANCE-N1: le funzioni pure della conversione -----------------

    /// Un vertice nativo costruito a mano, per provare `native_coordinate`
    /// sulle combinazioni che i tipi di `shapefile` non permettono di formare.
    ///
    /// `Point`, `PointM` e `PointZ` hanno ciascuno le proprie ordinate fissate
    /// dal tipo: da loro non si ottiene uno `ShapeZ` senza quota, che e' proprio
    /// il caso che i rifiuti esistono per prendere. Il tratto e' privato del
    /// modulo, quindi la sonda lo implementa dove vive.
    #[derive(Clone, Copy)]
    struct VerticeFinto {
        z: Option<f64>,
        m: Option<f64>,
    }

    impl NativePoint for VerticeFinto {
        fn x(&self) -> f64 {
            1.0
        }
        fn y(&self) -> f64 {
            2.0
        }
        fn z(&self) -> Option<f64> {
            self.z
        }
        fn m(&self) -> Option<f64> {
            self.m
        }
    }

    const fn vertice(z: Option<f64>, m: Option<f64>) -> VerticeFinto {
        VerticeFinto { z, m }
    }

    /// `native_coordinate` esige le ordinate che la dimensionalita' dichiara, e
    /// nomina quale manca.
    ///
    /// La dimensionalita' non e' una proprieta' del singolo vertice: viene dal
    /// tipo di shape dichiarato nell'header, e vale per tutto il layer. Un
    /// vertice che non la rispetta e' un file incoerente con la propria
    /// intestazione, e i cinque messaggi distinti dicono **quale** ordinata
    /// manca invece di «geometria non valida»: chi ripara il dato deve sapere
    /// se aggiungere la quota o la misura.
    ///
    /// Il caso piu' sottile e' l'ultimo dei rifiuti: un dataset `ShapeZ`
    /// dichiarato XYZ che porta una misura **valida**. Non e' un dato assente
    /// ma un dato in piu', e accettarlo lo butterebbe via in silenzio; il
    /// confronto e' con `NO_DATA`, perche' nel formato la misura assente e' un
    /// valore, non l'assenza del campo.
    // Nove rifiuti e cinque accettazioni in una tabella sola: separarle in
    // due test spezzerebbe la coppia che le rende interpretabili -- una
    // tabella di soli negativi la passa anche una funzione che rifiuta tutto.
    #[allow(clippy::too_many_lines)]
    #[test]
    fn n1_native_coordinate_esige_le_ordinate_che_la_dimensionalita_dichiara() {
        for (caso, punto, dimensioni, atteso) in [
            (
                "XYM senza misura",
                vertice(None, None),
                CoordinateDimensions::Xym,
                "coordinata ShapeM senza misura",
            ),
            (
                "XYZ senza quota",
                vertice(None, Some(NO_DATA)),
                CoordinateDimensions::Xyz,
                "coordinata ShapeZ senza quota",
            ),
            (
                "XYZ con una misura valida",
                vertice(Some(3.0), Some(7.0)),
                CoordinateDimensions::Xyz,
                "misura valida trovata in un dataset ShapeZ dichiarato XYZ",
            ),
            (
                "XYZM senza quota",
                vertice(None, Some(7.0)),
                CoordinateDimensions::Xyzm,
                "coordinata ShapeZ senza quota",
            ),
            (
                "XYZM senza misura",
                vertice(Some(3.0), None),
                CoordinateDimensions::Xyzm,
                "coordinata ShapeZ senza misura nativa",
            ),
            (
                "dimensionalita' non determinata",
                vertice(None, None),
                CoordinateDimensions::Unknown,
                "dimensionalità Shapefile non determinata",
            ),
            (
                "XY con una quota che non dovrebbe esserci",
                vertice(Some(3.0), None),
                CoordinateDimensions::Xy,
                "variante Shape incoerente con la dimensionalità del layer",
            ),
            (
                "XY con una misura che non dovrebbe esserci",
                vertice(None, Some(7.0)),
                CoordinateDimensions::Xy,
                "variante Shape incoerente con la dimensionalità del layer",
            ),
            (
                "XYM con una quota che non dovrebbe esserci",
                vertice(Some(3.0), Some(7.0)),
                CoordinateDimensions::Xym,
                "variante Shape incoerente con la dimensionalità del layer",
            ),
        ] {
            let Err(errore) = native_coordinate(&punto, dimensioni) else {
                panic!("{caso}: doveva essere rifiutato");
            };
            assert_eq!(errore.message, atteso, "{caso}: messaggio sbagliato");
        }

        // Le accettazioni, e cio' che ciascuna porta con se': senza, una
        // funzione che rifiutasse tutto supererebbe la tabella dei negativi, e
        // una che scartasse le ordinate passerebbe una tabella che guardasse
        // solo `is_ok`.
        for (caso, punto, dimensioni, z, m) in [
            (
                "XY puro",
                vertice(None, None),
                CoordinateDimensions::Xy,
                None,
                None,
            ),
            (
                "XYM con misura",
                vertice(None, Some(7.0)),
                CoordinateDimensions::Xym,
                None,
                Some(7.0),
            ),
            (
                "XYZ con quota e misura assente per convenzione",
                vertice(Some(3.0), Some(NO_DATA)),
                CoordinateDimensions::Xyz,
                Some(3.0),
                None,
            ),
            (
                "XYZ con quota e nessuna misura",
                vertice(Some(3.0), None),
                CoordinateDimensions::Xyz,
                Some(3.0),
                None,
            ),
            (
                "XYZM completo",
                vertice(Some(3.0), Some(7.0)),
                CoordinateDimensions::Xyzm,
                Some(3.0),
                Some(7.0),
            ),
        ] {
            let coordinata = match native_coordinate(&punto, dimensioni) {
                Ok(coordinata) => coordinata,
                Err(errore) => panic!("{caso}: doveva essere accettato: {errore:?}"),
            };
            assert_eq!(
                (coordinata.z, coordinata.m),
                (z, m),
                "{caso}: ordinate perse"
            );
            assert!(
                (coordinata.x - 1.0).abs() < f64::EPSILON
                    && (coordinata.y - 2.0).abs() < f64::EPSILON,
                "{caso}: X e Y devono passare invariate"
            );
        }
    }

    /// `polygon_wkb` esige un anello esterno prima di ogni interno, e almeno
    /// uno in tutto.
    ///
    /// Nel formato Shapefile gli anelli sono una sequenza piatta, e la
    /// gerarchia esiste solo nell'ordine: un anello interno appartiene
    /// all'ultimo esterno visto. Un file che apre con un interno non descrive
    /// un buco in niente, e la funzione non ha modo di indovinare a chi
    /// appartenga -- accettarlo significherebbe scegliere un contenitore a
    /// caso.
    ///
    /// Le due accettazioni fissano proprio la gerarchia: un esterno con un
    /// interno diventa un poligono di due anelli, due esterni diventano due
    /// poligoni. Senza, uno scambio fra i due rami non romperebbe nulla.
    #[test]
    fn n1_polygon_wkb_esige_un_anello_esterno_prima_di_ogni_interno() {
        let anello = |chiuso: bool| {
            let z = if chiuso { None } else { Some(3.0) };
            vec![vertice(z, None), vertice(z, None), vertice(z, None)]
        };

        let Err(errore) = polygon_wkb(
            &[PolygonRing::Inner(anello(true))],
            CoordinateDimensions::Xy,
        ) else {
            panic!("un anello interno senza esterno non appartiene a niente");
        };
        assert_eq!(
            errore.message,
            "anello interno Shapefile senza anello esterno"
        );

        let vuoti: [PolygonRing<VerticeFinto>; 0] = [];
        let Err(errore) = polygon_wkb(&vuoti, CoordinateDimensions::Xy) else {
            panic!("un poligono senza anelli non e' un poligono");
        };
        assert_eq!(errore.message, "Polygon Shapefile senza anelli esterni");

        // Un esterno con il suo interno: un poligono, due anelli.
        let uno = polygon_wkb(
            &[
                PolygonRing::Outer(anello(true)),
                PolygonRing::Inner(anello(true)),
            ],
            CoordinateDimensions::Xy,
        )
        .expect("un esterno con il suo interno e' un poligono con un buco");
        let WkbValue::MultiPolygon(poligoni) = uno.value else {
            panic!("il risultato e' sempre un MultiPolygon");
        };
        assert_eq!(poligoni.len(), 1, "un solo esterno, un solo poligono");
        let WkbValue::Polygon(corona) = &poligoni[0].value else {
            panic!("il membro e' un poligono");
        };
        assert_eq!(corona.len(), 2, "l'interno deve stare dentro l'esterno");

        // Due esterni: due poligoni, non un poligono con due anelli.
        let due = polygon_wkb(
            &[
                PolygonRing::Outer(anello(true)),
                PolygonRing::Outer(anello(true)),
            ],
            CoordinateDimensions::Xy,
        )
        .expect("due esterni sono due poligoni");
        let WkbValue::MultiPolygon(poligoni) = due.value else {
            panic!("il risultato e' sempre un MultiPolygon");
        };
        assert_eq!(poligoni.len(), 2, "due esterni non si fondono in uno");

        // La propagazione da `native_coordinates`: un vertice incoerente con
        // la dimensionalita' ferma la conversione invece di perdere l'ordinata.
        let Err(errore) = polygon_wkb(
            &[PolygonRing::Outer(anello(false))],
            CoordinateDimensions::Xy,
        ) else {
            panic!("un vertice con una quota in un layer XY e' incoerente");
        };
        assert_eq!(
            errore.message,
            "variante Shape incoerente con la dimensionalità del layer"
        );
    }

    /// `shape_to_wkb` traduce l'assenza in `None` e rifiuta il Multipatch.
    ///
    /// `NullShape` e' l'assenza di geometria dichiarata dal formato, non un
    /// errore: diventa `Ok(None)`, che a valle e' una riga senza geometria.
    /// Il Multipatch invece e' una forma che WKB non sa rappresentare in modo
    /// univoco -- le sue parti hanno una semantica di superficie che il modello
    /// piatto non porta -- e tradurlo comunque significherebbe scegliere una
    /// delle letture possibili e non dirlo.
    #[test]
    fn n1_shape_to_wkb_rende_nulla_la_nullshape_e_rifiuta_il_multipatch() {
        assert!(
            shape_to_wkb(&Shape::NullShape, CoordinateDimensions::Xy)
                .expect("la NullShape non e' un errore")
                .is_none(),
            "la NullShape e' assenza di geometria, non un fallimento"
        );

        let multipatch = Shape::Multipatch(shapefile::Multipatch::new(
            shapefile::Patch::TriangleStrip(vec![
                shapefile::PointZ::new(0.0, 0.0, 0.0, NO_DATA),
                shapefile::PointZ::new(1.0, 0.0, 0.0, NO_DATA),
                shapefile::PointZ::new(0.0, 1.0, 0.0, NO_DATA),
            ]),
        ));
        let Err(errore) = shape_to_wkb(&multipatch, CoordinateDimensions::Xyz) else {
            panic!("il Multipatch non ha una traduzione WKB univoca");
        };
        assert_eq!(
            errore.message,
            "Multipatch non ha una conversione WKB univoca ed è rifiutato"
        );

        // Il controllo positivo: una forma traducibile passa e conserva la
        // dimensionalita' dichiarata.
        let punto = shape_to_wkb(
            &Shape::Point(shapefile::Point::new(1.0, 2.0)),
            CoordinateDimensions::Xy,
        )
        .expect("un punto XY si traduce")
        .expect("un punto non e' un'assenza");
        assert_eq!(punto.dimensions, CoordinateDimensions::Xy);
    }

    /// `shape_from_wkb` rifiuta la dimensionalita' ignota, ed e' la ragione per
    /// cui `__fuzz_wkb_roundtrip` non puo' vedere una `NullShape`.
    ///
    /// La funzione decide su una coppia (dimensionalita', topologia), e ogni
    /// braccio costruisce una shape **concreta**: nessuno produce `NullShape`.
    /// Percio' il rifiuto «la conversione di una geometria ha prodotto
    /// `NullShape`» dentro il target del fuzzer non ha input -- `shape_to_wkb`
    /// restituisce `None` solo per `NullShape`, e li' non ci puo' arrivare.
    ///
    /// La sonda esegue quella precondizione su ogni combinazione che il target
    /// puo' formare, invece di argomentarla: se un braccio futuro restituisse
    /// `NullShape`, diventa rossa.
    #[test]
    fn n1_shape_from_wkb_non_produce_mai_nullshape_e_rifiuta_la_dimensionalita_ignota() {
        let coordinata = |z: Option<f64>, m: Option<f64>| WkbCoordinate {
            x: 1.0,
            y: 2.0,
            z,
            m,
        };
        let ordinate = |dimensioni: CoordinateDimensions| match dimensioni {
            CoordinateDimensions::Xy => coordinata(None, None),
            CoordinateDimensions::Xym => coordinata(None, Some(7.0)),
            CoordinateDimensions::Xyz => coordinata(Some(3.0), None),
            _ => coordinata(Some(3.0), Some(7.0)),
        };

        for dimensioni in [
            CoordinateDimensions::Xy,
            CoordinateDimensions::Xym,
            CoordinateDimensions::Xyz,
            CoordinateDimensions::Xyzm,
        ] {
            let vertice = ordinate(dimensioni);
            // I membri di un multi- sono geometrie, non coordinate: il modello
            // porta la dimensionalita' su ciascuna, e costruirli a mano e'
            // l'unico modo di formare le sedici coppie senza passare da un file.
            let membro = |valore| WkbGeometry {
                value: valore,
                dimensions: dimensioni,
                srid: None,
            };
            for (caso, valore) in [
                ("punto", WkbValue::Point(vertice)),
                (
                    "multipunto",
                    WkbValue::MultiPoint(vec![membro(WkbValue::Point(vertice))]),
                ),
                (
                    "linea",
                    WkbValue::MultiLineString(vec![membro(WkbValue::LineString(vec![
                        vertice, vertice,
                    ]))]),
                ),
                (
                    "poligono",
                    WkbValue::MultiPolygon(vec![membro(WkbValue::Polygon(vec![vec![
                        vertice, vertice, vertice, vertice,
                    ]]))]),
                ),
            ] {
                let shape = match shape_from_wkb(WkbGeometry {
                    value: valore,
                    dimensions: dimensioni,
                    srid: None,
                }) {
                    Ok(shape) => shape,
                    Err(errore) => panic!("{caso} {dimensioni:?}: doveva convertirsi: {errore:?}"),
                };
                assert!(
                    !matches!(shape, Shape::NullShape),
                    "{caso} {dimensioni:?}: se questo fallisse, il rifiuto sulla NullShape \
                     dentro __fuzz_wkb_roundtrip diventerebbe raggiungibile"
                );
            }
        }

        // La dimensionalita' ignota: l'unico rifiuto proprio della funzione.
        let Err(errore) = shape_from_wkb(WkbGeometry {
            value: WkbValue::Point(coordinata(None, None)),
            dimensions: CoordinateDimensions::Unknown,
            srid: None,
        }) else {
            panic!("una dimensionalita' ignota non si scrive in Shapefile");
        };
        assert_eq!(
            errore.message,
            "dimensionalità WKB ignota non scrivibile in Shapefile"
        );
    }

    /// Un bundle in cui il `.dbf` conta piu' righe di quante geometrie abbia il
    /// `.shp`.
    ///
    /// Nessun produttore conforme lo scrive, e all'apertura non passerebbe:
    /// `infer_geometry_info` confronta i due conteggi e rifiuta. Serve percio'
    /// a raggiungere il parser **saltando** quel confronto, che e' l'unico modo
    /// di eseguire la difesa che il parser ha per lo stesso caso -- e che esiste
    /// perche' fra l'apertura e la lettura il file puo' essere cambiato.
    ///
    /// Le due meta' vengono da due bundle scritti dal writer vero: il `.shp` e
    /// lo `.shx` da quello corto, il `.dbf` da quello lungo. Nessun byte e'
    /// costruito a mano, quindi l'unica incoerenza e' quella voluta.
    fn bundle_disallineato(dir: &Path, nome: &str, geometrie: usize, record: usize) -> PathBuf {
        let corto = bundle_di_righe(dir, &format!("corto-{nome}"), &["NOME"], geometrie, |_| {});
        let lungo = bundle_di_righe(dir, &format!("lungo-{nome}"), &["NOME"], record, |_| {});
        std::fs::copy(lungo.with_extension("dbf"), corto.with_extension("dbf"))
            .expect("il dbf lungo prende il posto di quello corto");
        corto
    }

    /// `spawn_parser` si ferma quando le geometrie finiscono prima dei record,
    /// e i due contatori non possono traboccare.
    ///
    /// # Perche' passa dalla costruzione diretta dell'ingresso
    ///
    /// Dal driver questa difesa non ha input: `infer_geometry_info` confronta
    /// geometrie e record all'apertura e rifiuta prima. Resta nel parser perche'
    /// fra l'apertura e la lettura il file puo' cambiare, e allora la difesa e'
    /// l'ultima cosa che sta fra un troncamento e una riga con gli attributi di
    /// un'altra. `ShpParserInput` e' privato del modulo, quindi la sonda lo
    /// costruisce dove vive.
    ///
    /// # I due contatori
    ///
    /// «numero di record Shapefile fuori intervallo u64» e «numero di record DBF
    /// attivi fuori intervallo u64» non hanno input, e non serve una guardia per
    /// dirlo: il ciclo gira al piu' `record_count` volte, e `record_count` e' un
    /// `u32` letto dall'header. Un contatore `u64` che parte da zero e cresce di
    /// uno per giro arriva al massimo a `u32::MAX`.
    ///
    /// La difesa simmetrica -- i record che finiscono prima delle geometrie --
    /// e' chiusa altrove: pretenderebbe che l'iteratore di `dbase` si esaurisse
    /// prima del lettore raw, e le due strade per farlo sono percorse e chiuse
    /// in `n1_le_due_letture_del_dbf_non_possono_divergere`.
    #[test]
    fn n1_spawn_parser_si_ferma_quando_le_geometrie_finiscono_prima_dei_record() {
        let dir = tempfile::tempdir().unwrap();
        let bundle = bundle_disallineato(dir.path(), "misto.shp", 1, 3);
        let layout = read_dbf_layout(&bundle).expect("il dbf lungo e' ben formato");
        assert_eq!(layout.record_count, 3, "il dbf porta tre record");

        let schema: SchemaRef = Arc::new(Schema::new(vec![geometry_field(GEOMETRY, "EPSG:4326")]));
        let layer = LayerContract {
            id: LayerId(0),
            name: "misto".to_owned(),
            contract: DataContract::new(schema.clone(), None),
        };
        let ingresso = ShpParserInput {
            path: bundle,
            schema: schema.clone(),
            cols: Vec::new(),
            dbf_layout: layout.clone(),
            dimensions: CoordinateDimensions::Xy,
            expected_shape_type: Some("point-xy"),
            expected_active_rows: u64::from(layout.record_count),
            include_geometry: true,
            batch_sizer: plenora_io_core::AdaptiveBatchSizer::new(&schema, BatchTarget::default()),
            layer,
            loss: LossReport::default(),
            row_diagnostics: ShpRowDiagnosticsConfig::from_options(&BTreeMap::new(), &[], &layout)
                .expect("nessuna opzione di diagnostica"),
            scope: ReadScope::Complete,
            cancellation: plenora_io_model::CancellationToken::new(),
        };

        let mut lettore = spawn_parser(ingresso).expect("le validazioni pre-thread passano");
        let mut errore = None;
        loop {
            match lettore.next_batch() {
                Ok(Some(_)) => {}
                Ok(None) => break,
                Err(e) => {
                    errore = Some(e);
                    break;
                }
            }
        }
        let Some(errore) = errore else {
            panic!("tre record contro una geometria non sono un allineamento");
        };
        assert_eq!(
            errore.message, "numero di geometrie incoerente con i record DBF",
            "il messaggio deve dire quale dei due file e' finito prima"
        );

        // I due contatori: il ciclo gira al piu' `record_count` volte, e quello
        // e' un `u32`. Un `u64` che cresce di uno per giro non ci arriva.
        assert!(
            u64::from(u32::MAX).checked_add(1).is_some(),
            "un contatore u64 che sale fino a u32::MAX non trabocca: e' la ragione per cui \
             i due «fuori intervallo u64» non hanno input"
        );

        // Il controllo positivo, con lo stesso parser: un bundle allineato si
        // legge fino in fondo. Senza, «fallisce sempre» supererebbe la prova.
        let allineato = bundle_di_righe(dir.path(), "allineato.shp", &["NOME"], 3, |_| {});
        let layout = read_dbf_layout(&allineato).expect("il bundle e' conforme");
        let ingresso = ShpParserInput {
            path: allineato,
            schema: schema.clone(),
            cols: Vec::new(),
            dbf_layout: layout.clone(),
            dimensions: CoordinateDimensions::Xy,
            expected_shape_type: Some("point-xy"),
            expected_active_rows: u64::from(layout.record_count),
            include_geometry: true,
            batch_sizer: plenora_io_core::AdaptiveBatchSizer::new(&schema, BatchTarget::default()),
            layer: LayerContract {
                id: LayerId(0),
                name: "allineato".to_owned(),
                contract: DataContract::new(schema, None),
            },
            loss: LossReport::default(),
            row_diagnostics: ShpRowDiagnosticsConfig::from_options(&BTreeMap::new(), &[], &layout)
                .expect("nessuna opzione di diagnostica"),
            scope: ReadScope::Complete,
            cancellation: plenora_io_model::CancellationToken::new(),
        };
        let mut lettore = spawn_parser(ingresso).expect("le validazioni pre-thread passano");
        let mut righe = 0_usize;
        while let Some(lotto) = lettore.next_batch().expect("un bundle allineato si legge") {
            righe += lotto.num_rows();
        }
        assert_eq!(
            righe, 3,
            "tre record allineati a tre geometrie fanno tre righe"
        );
    }

    /// `descrittori_concordi` rifiuta la divergenza fra le due letture
    /// dell'header, e fa uscire il conteggio che e' nostro.
    ///
    /// Il controllo non e' ridondante, ed e' la parte che vale la pena fissare:
    /// `leggi_descrittori_dbf` scorre **un descrittore per nome decodificato**,
    /// quindi con due numeri diversi il lettore si fermerebbe a meta' dei
    /// trentadue byte di un descrittore, e il controllo sul terminatore che
    /// segue leggerebbe un byte qualunque. Il rifiuto tiene allineate le due
    /// letture prima che si disallineino.
    ///
    /// Che il messaggio porti il conteggio **decodificato** e non quello
    /// dell'header e' l'altra meta': il primo lo produciamo noi, il secondo
    /// viene dal file, e nessun numero letto dal payload esce dai messaggi.
    #[test]
    fn n1_descrittori_concordi_rifiuta_la_divergenza_e_fa_uscire_il_numero_nostro() {
        descrittori_concordi(3, 3).expect("due conteggi uguali non sono una divergenza");
        descrittori_concordi(0, 0).expect("zero descrittori concordano con zero");

        for (caso, dichiarati, decodificati) in [
            ("l'header ne dichiara uno in piu'", 2, 1),
            ("il decoder ne trova uno in piu'", 1, 2),
        ] {
            let Err(errore) = descrittori_concordi(dichiarati, decodificati) else {
                panic!("{caso}: due conteggi diversi non possono allineare due letture");
            };
            assert!(
                errore
                    .message
                    .starts_with("numero di descrittori DBF incoerente con l'header"),
                "{caso}: arrivato «{}»",
                errore.message
            );
            assert!(
                errore.message.ends_with(&decodificati.to_string()),
                "{caso}: deve uscire il conteggio decodificato ({decodificati}), \
                 non quello letto dall'header: «{}»",
                errore.message
            );
        }
    }

    /// `righe_dopo_il_lotto` ferma il contatore che traboccherebbe, e lo fa
    /// **prima** che il lotto raggiunga il disco.
    ///
    /// Il contatore serve a due cose diverse -- il totale dichiarato e la
    /// posizione delle righe nelle diagnostiche -- e un valore che ricomincia
    /// da zero dopo un giro completo le rovinerebbe entrambe in silenzio.
    ///
    /// La conversione della cardinalita' del lotto resta senza input: e' una
    /// `usize` verso `u64`, e su ogni bersaglio supportato non fallisce. Cio'
    /// che si prova qui e' la somma, che dipende da uno stato accumulato e non
    /// dal bersaglio.
    #[test]
    fn n1_righe_dopo_il_lotto_ferma_il_contatore_che_traboccherebbe() {
        assert_eq!(
            righe_dopo_il_lotto(7, 3).expect("dieci righe stanno in un u64"),
            10
        );
        assert_eq!(
            righe_dopo_il_lotto(u64::MAX, 0).expect("un lotto vuoto non aggiunge niente"),
            u64::MAX,
            "il confine e' incluso: e' l'ultimo valore rappresentabile, non il primo rifiutato"
        );

        let Err(errore) = righe_dopo_il_lotto(u64::MAX, 1) else {
            panic!("una riga oltre l'ultimo valore rappresentabile non si conta");
        };
        assert_eq!(errore.message, "troppe righe Shapefile");
        assert_eq!(
            errore.category,
            plenora_io_model::ErrorCategory::ResourceLimit
        );

        // La conversione infallibile: che questa riga compili e' la prova che
        // l'altro ramo dell'aiutante non ha input su questo bersaglio.
        let _: u64 = u64::try_from(usize::MAX).expect("usize sta in u64 sui bersagli supportati");
    }

    /// Il rifiuto sul contatore precede la scrittura, e lo staging resta come
    /// era.
    ///
    /// E' la meta' che l'aritmetica da sola non prova: che il conteggio corra
    /// **prima** del ciclo che scrive le shape. Un contatore verificato dopo
    /// avrebbe gia' lasciato le geometrie nel file di staging, e il rifiuto
    /// sarebbe una constatazione.
    #[test]
    fn n1_il_contatore_che_trabocca_ferma_il_lotto_prima_di_scriverlo() {
        let dir = tempfile::tempdir().unwrap();
        let punto = to_wkb(&geo_types::Geometry::Point(geo_types::Point::new(1.0, 2.0))).unwrap();
        let schema: SchemaRef = Arc::new(Schema::new(vec![geometry_field(GEOMETRY, "EPSG:4326")]));
        let lotto = RecordBatch::try_new(
            schema,
            vec![Arc::new(BinaryArray::from(vec![Some(punto.as_slice())]))],
        )
        .unwrap();

        // Lo stato si costruisce direttamente: `create` restituisce un
        // `Box<dyn FormatWriter>`, e da li' il contatore non e' raggiungibile.
        let destinazione = dir.path().join("contatore.shp");
        let staging = create_staged_dir(&destinazione).expect("lo staging si crea");
        let percorso_shp = staging.path().join("data.shp");
        let writer = Writer::from_path(&percorso_shp, TableWriterBuilder::new())
            .expect("il writer dello staging si apre");
        let mut stato = ShpWriter {
            staging: Some(staging),
            writer: Some(writer),
            dest: destinazione,
            durable: false,
            publish_mode: ShapefilePublishMode::LooseSet,
            attrs: Vec::new(),
            geom_idx: 0,
            prj: None,
            shape_type: None,
            rows: u64::MAX,
            input_total: None,
            wkb_limits: WkbLimits::default(),
            max_output_bytes: u64::MAX,
        };

        let prima = std::fs::metadata(&percorso_shp)
            .expect("il file di staging esiste")
            .len();
        let Err(errore) = stato.write(&lotto) else {
            panic!("un lotto che fa traboccare il contatore non si scrive");
        };
        assert_eq!(errore.message, "troppe righe Shapefile");
        assert_eq!(
            std::fs::metadata(&percorso_shp)
                .expect("il file di staging esiste ancora")
                .len(),
            prima,
            "il rifiuto deve precedere la scrittura: lo staging non deve essere cresciuto"
        );
    }

    /// `byte_dello_staging` somma le parti senza traboccare in silenzio.
    ///
    /// Il tetto sull'output vale sull'**insieme** delle quattro parti, e un
    /// totale che ricominciasse da zero lo farebbe passare: il set verrebbe
    /// pubblicato dichiarando meno byte di quanti ne occupa, che e' peggio di
    /// un rifiuto.
    #[test]
    fn n1_byte_dello_staging_non_trabocca_in_silenzio() {
        assert_eq!(
            byte_dello_staging([1, 2, 3]).expect("tre parti si sommano"),
            6
        );
        assert_eq!(
            byte_dello_staging([]).expect("nessuna parte fa zero byte"),
            0,
            "un set senza parti presenti e' zero, non un errore"
        );
        assert_eq!(
            byte_dello_staging([u64::MAX, 0]).expect("il confine e' incluso"),
            u64::MAX
        );

        let Err(errore) = byte_dello_staging([u64::MAX, 1]) else {
            panic!("una somma oltre u64::MAX non e' un conteggio");
        };
        assert_eq!(
            errore.message,
            "overflow nel conteggio dell'output Shapefile"
        );
        assert_eq!(
            errore.category,
            plenora_io_model::ErrorCategory::ResourceLimit
        );
    }

    /// `next_physical` rifiuta il campo che non sta nel record, e nessun layout
    /// letto da un file glielo puo' presentare.
    ///
    /// Il metodo indicizza il buffer del record con gli offset del layout, e la
    /// difesa esiste perche' quei due valori arrivano da posti diversi: il
    /// buffer e' lungo `record_length`, gli offset vengono dai descrittori.
    /// `read_dbf_layout` li rende coerenti per costruzione -- `record_length`
    /// **e'** l'offset finale calcolato dai descrittori, non quello dichiarato
    /// nell'header -- quindi da un file la difesa non ha input.
    ///
    /// Il tipo e' privato del modulo, e la sonda costruisce il layout
    /// incoerente direttamente: e' l'unico modo di eseguire quella riga senza
    /// fingere che un file possa produrla. Accanto sta la prova che un layout
    /// **letto davvero** e' coerente, che e' la meta' che rende interpretabile
    /// la prima.
    #[test]
    fn n1_next_physical_rifiuta_il_campo_che_non_sta_nel_record() {
        let dir = tempfile::tempdir().unwrap();
        let bundle = bundle_con_dbf_ritoccato(dir.path(), "coerente.shp", &["CODICE"], |_| {});

        // La meta' che vale come controllo: il layout che il lettore produce ha
        // ogni campo dentro il record.
        let letto = read_dbf_layout(&bundle).expect("il bundle e' conforme");
        for campo in &letto.fields {
            assert!(
                campo.offset + campo.width <= letto.record_length,
                "`read_dbf_layout` non produce campi che escono dal record: \
                 offset {} + larghezza {} contro {}",
                campo.offset,
                campo.width,
                letto.record_length
            );
        }

        // Il layout incoerente: un campo che comincia dentro il record e finisce
        // fuori. `exact_integer_slot` deve essere pieno, altrimenti il ciclo
        // salta il campo e la riga non viene mai raggiunta.
        let incoerente = DbfLayout {
            header_length: letto.header_length,
            record_length: letto.record_length,
            record_count: 1,
            fields: vec![DbfFieldLayout {
                name: "CODICE".to_owned(),
                field_type: b'N',
                offset: 1,
                width: letto.record_length + 10,
                exact_integer_slot: Some(0),
            }],
            exact_integer_count: 1,
        };
        let mut righe = DbfExactIntegerRows::open(&bundle, &incoerente)
            .expect("l'apertura non guarda gli offset dei campi");
        let Err(errore) = righe.next_physical(None) else {
            panic!("un campo che finisce fuori dal record non e' leggibile");
        };
        assert_eq!(errore.message, "campo DBF fuori dal record");
    }

    /// `finish_batch` rifiuta la combinazione che Arrow non accetta.
    ///
    /// La funzione mette insieme tre cose che arrivano da posti diversi: lo
    /// schema del contratto, i costruttori delle colonne e il numero di righe
    /// osservate. Arrow pretende che concordino, e la difesa esiste perche' un
    /// disaccordo qui produrrebbe un batch che il chiamante crede conforme al
    /// contratto e non lo e'.
    ///
    /// Il caso provato e' il piu' semplice dei tre disaccordi possibili: uno
    /// schema che dichiara una colonna in piu' di quelle costruite.
    #[test]
    fn n1_finish_batch_rifiuta_lo_schema_che_non_torna_con_i_costruttori() {
        let schema: SchemaRef = Arc::new(Schema::new(vec![
            geometry_field(GEOMETRY, "EPSG:4326"),
            Field::new("MANCANTE", arrow_schema::DataType::Utf8, true),
        ]));

        // Il controllo positivo: con il costruttore che lo schema dichiara, il
        // batch si forma e porta le righe dichiarate.
        let mut geometria = Some(BinaryBuilder::new());
        if let Some(builder) = geometria.as_mut() {
            builder.append_value(b"wkb");
        }
        let mut costruttori = vec![InferredColumnBuilder::new(ColType::Text)];
        costruttori[0]
            .append_str("uno")
            .expect("un testo entra in un costruttore di testo");
        let lotto = finish_batch(&schema, &mut geometria, &mut costruttori, 1)
            .expect("schema e costruttori concordano");
        assert_eq!(lotto.num_rows(), 1);
        assert_eq!(lotto.num_columns(), 2);

        // Lo stesso schema senza il costruttore della seconda colonna.
        let mut geometria = Some(BinaryBuilder::new());
        if let Some(builder) = geometria.as_mut() {
            builder.append_value(b"wkb");
        }
        let Err(errore) = finish_batch(&schema, &mut geometria, &mut [], 1) else {
            panic!("uno schema con una colonna in piu' dei costruttori non forma un batch");
        };
        assert_eq!(errore.message, "costruzione del RecordBatch fallita");
    }

    /// `open` propaga i rifiuti della catena e prende il nome del layer dal
    /// nome del file.
    ///
    /// L'entry point non decide quasi niente da se': la sua parte e' mettere in
    /// fila le validazioni e comporre il contratto. Le due cose che vale la
    /// pena fissare sono percio' che i rifiuti **escano** con il proprio
    /// messaggio invece di essere riscritti, e che il nome del layer sia il
    /// gambo del file -- un dettaglio che nessun errore esprime e che chi legge
    /// il contratto vede per primo.
    #[test]
    fn n1_open_propaga_i_rifiuti_della_catena_e_nomina_il_layer_dal_file() {
        let dir = tempfile::tempdir().unwrap();
        let bundle = bundle_con_dbf_ritoccato(dir.path(), "comuni.shp", &["NOME"], |_| {});

        // Il nome del layer viene dal gambo del file, non da un letterale.
        let aperto = ShpDriver
            .open(
                Source::Path(bundle.clone()),
                opzioni_lettura().with_assume_crs("EPSG:4326"),
            )
            .expect("un bundle conforme si apre");
        assert_eq!(
            aperto.layers()[0].name,
            "comuni",
            "il nome del layer e' il gambo del file"
        );

        // La propagazione dalle opzioni di diagnostica: il messaggio e' quello
        // di `from_options`, non una riscrittura.
        let Err(errore) = ShpDriver.open(
            Source::Path(bundle),
            opzioni_lettura()
                .with_assume_crs("EPSG:4326")
                .with_format_option("row_diagnostics.key_policy", "emit"),
        ) else {
            panic!("una policy senza campo non e' una configurazione valida");
        };
        assert_eq!(
            errore.message,
            "row_diagnostics.key_policy richiede row_diagnostics.key_field"
        );

        // La propagazione dall'inferenza dello schema: un `.dbf` con l'header
        // ritoccato ferma l'apertura, e il messaggio resta quello del lettore.
        let rotto = bundle_con_dbf_ritoccato(dir.path(), "rotto.shp", &["NOME"], |byte| {
            byte[10..12].copy_from_slice(&99_u16.to_le_bytes());
        });
        let Err(errore) = ShpDriver.open(
            Source::Path(rotto),
            opzioni_lettura().with_assume_crs("EPSG:4326"),
        ) else {
            panic!("un header DBF incoerente non produce uno schema");
        };
        assert_eq!(
            errore.message,
            "lunghezza di record DBF dichiarata incoerente con i campi, byte richiesti 11",
            "il messaggio del lettore deve uscire da `open` invariato"
        );
    }

    /// Il conteggio dell'header e `dbase` non possono divergere: due guardie
    /// diverse lo impediscono, e questa sonda le esegue entrambe.
    ///
    /// `infer_shp_schema` scorre i record due volte in parallelo -- il lettore
    /// raw degli interi esatti e l'iteratore di `dbase` -- e ha una difesa per
    /// il caso in cui uno dei due finisca prima dell'altro. Entrambi sono
    /// guidati dallo stesso conteggio nell'header, quindi per farli divergere
    /// bisognerebbe far fermare `dbase` prima, e `dbase` si ferma in due modi
    /// soli: alla fine fisica del file, o sul marcatore di fine record.
    ///
    /// **Le due strade sono chiuse, e la sonda le percorre invece di
    /// argomentarle.** Un `.dbf` troncato non arriva alla divergenza: il
    /// lettore raw pretende i byte del record e rifiuta con «record DBF
    /// incompleto». Un marcatore di fine file messo dove `dbase` lo
    /// leggerebbe come tale cade sul flag di cancellazione, e il lettore raw
    /// ammette solo lo spazio e l'asterisco: rifiuta con «marcatore di record
    /// DBF non valido».
    ///
    /// La seconda difesa -- `dbase` che ha **piu'** record del conteggio -- e'
    /// chiusa da un'altra parte: `infer_geometry_info` confronta le geometrie
    /// con il conteggio dichiarato e corre prima del ciclo.
    ///
    /// Restano senza input anche «numero di record DBF fuori intervallo u64»,
    /// che sorveglia una conversione da `u32`, e «schema DBF senza accumulatore
    /// per un campo dichiarato», che sorveglia una mappa costruita dagli stessi
    /// descrittori su cui poi si cerca: perche' un campo non vi si trovasse
    /// servirebbero due descrittori omonimi, e `leggi_descrittori_dbf` li
    /// rifiuta prima.
    #[test]
    fn n1_le_due_letture_del_dbf_non_possono_divergere() {
        let dir = tempfile::tempdir().unwrap();

        // Il controllo positivo: due righe dichiarate, due presenti, schema
        // inferito. Senza, «rifiuta sempre» supererebbe la prova.
        let intatto = bundle_di_righe(dir.path(), "due.shp", &["NOME"], 2, |_| {});
        let inferenza = infer_shp_schema(&intatto).expect("un bundle conforme si legge");
        assert_eq!(inferenza.active_row_count, 2, "due righe attive");
        assert_eq!(inferenza.cols.len(), 1, "un campo oltre la geometria");

        // Prima strada: il file finisce prima di quanto l'header prometta.
        let troncato = bundle_di_righe(dir.path(), "troncato.shp", &["NOME"], 2, |byte| {
            let lunghezza_header = usize::from(u16::from_le_bytes([byte[8], byte[9]]));
            let lunghezza_record = usize::from(u16::from_le_bytes([byte[10], byte[11]]));
            byte.truncate(lunghezza_header + lunghezza_record);
        });
        let Err(errore) = infer_shp_schema(&troncato) else {
            panic!("un header che promette piu' record di quanti ce ne sono non e' affidabile");
        };
        assert_eq!(
            errore.message, "record DBF incompleto",
            "a rifiutare e' il lettore raw, prima che le due letture divergano"
        );

        // Seconda strada: il marcatore di fine file dove `dbase` lo leggerebbe.
        // Cade sul flag di cancellazione del terzo record, e il lettore raw
        // ammette solo lo spazio e l'asterisco.
        let marcato = bundle_di_righe(dir.path(), "marcato.shp", &["NOME"], 3, |byte| {
            let lunghezza_header = usize::from(u16::from_le_bytes([byte[8], byte[9]]));
            let lunghezza_record = usize::from(u16::from_le_bytes([byte[10], byte[11]]));
            byte[lunghezza_header + 2 * lunghezza_record] = 0x1A;
        });
        let Err(errore) = infer_shp_schema(&marcato) else {
            panic!("un marcatore di fine file in mezzo ai record non e' un record");
        };
        assert_eq!(
            errore.message, "marcatore di record DBF non valido",
            "a rifiutare e' il controllo sul flag di cancellazione"
        );

        // Le due difese che restano, e le loro precondizioni.
        //
        // La prima si prova senza asserzioni: il conteggio dei record e' un
        // `u32`, e la conversione a `u64` e' **infallibile** -- `From`, non
        // `TryFrom`. Che questa riga compili e' la prova, e vale piu' di
        // un'asserzione su un valore che il compilatore conosce gia'.
        let _: u64 = u64::from(u32::MAX);
        let doppi = leggi_descrittori_dbf(
            &mut std::io::Cursor::new(
                [descrittore(b'C', 10, 0), descrittore(b'C', 10, 0)].concat(),
            ),
            vec!["NOME".to_owned(), "nome".to_owned()],
            2,
        );
        assert!(
            doppi.is_err(),
            "due descrittori omonimi sono rifiutati prima: e' la ragione per cui la mappa \
             degli accumulatori ha una voce per ogni campo"
        );
    }

    /// `infer_geometry_info` pretende che i due file del bundle raccontino la
    /// stessa storia.
    ///
    /// Uno Shapefile e' due file che si contano a vicenda: il `.shp` porta le
    /// geometrie, il `.dbf` gli attributi, e la riga *n* e' la coppia dei due
    /// record *n*-esimi. Se i conteggi non coincidono non esiste un
    /// allineamento giusto -- non si sa quale dei due file abbia la riga in
    /// piu' -- e leggere comunque accoppierebbe attributi con geometrie di
    /// un'altra riga. E' la perdita silenziosa che il rifiuto esiste per
    /// evitare, e il messaggio dice esattamente questo invece di «file
    /// corrotto».
    ///
    /// Il conteggio ritoccato e' quello del `.dbf`, perche' e' un campo
    /// dell'header e si cambia senza toccare i record: il `.shp` resta quello
    /// che il writer ha prodotto, quindi l'unica cosa incoerente e' cio' che la
    /// sonda ha dichiarato.
    #[test]
    fn n1_infer_geometry_info_rifiuta_i_conteggi_che_non_coincidono() {
        let dir = tempfile::tempdir().unwrap();

        // Il controllo positivo: un bundle intatto ha una geometria e un
        // record, e l'inferenza restituisce il tipo dell'header.
        let intatto = bundle_con_dbf_ritoccato(dir.path(), "intatto-geom.shp", &["NOME"], |_| {});
        let info = infer_geometry_info(&intatto, 1).expect("un bundle conforme si legge");
        assert_eq!(
            info.shape_type,
            Some("point-xy"),
            "il tipo viene dall'header del .shp"
        );
        assert_eq!(info.dimensions, CoordinateDimensions::Xy);

        // Il conteggio dichiarato dal chiamante non coincide con le geometrie.
        for dichiarato in [0_u32, 2] {
            let Err(errore) = infer_geometry_info(&intatto, dichiarato) else {
                panic!("{dichiarato} record DBF contro una geometria non e' un allineamento");
            };
            assert_eq!(
                errore.message, "numero di geometrie diverso dal numero di record DBF",
                "il messaggio deve dire che i due file non si contano allo stesso modo"
            );
        }

        // Lo stesso disallineamento visto dall'altra parte: e' il `.dbf` a
        // dichiarare un conteggio che il `.shp` non conferma, ed e' la strada
        // per cui il rifiuto arriva davvero da un file e non dal chiamante.
        let ritoccato = bundle_con_dbf_ritoccato(
            dir.path(),
            "conteggio.shp",
            &["NOME"],
            |byte: &mut Vec<u8>| {
                byte[4..8].copy_from_slice(&7_u32.to_le_bytes());
            },
        );
        let layout = read_dbf_layout(&ritoccato).expect("l'header resta leggibile");
        assert_eq!(layout.record_count, 7, "il ritocco e' arrivato dove doveva");
        let Err(errore) = infer_geometry_info(&ritoccato, layout.record_count) else {
            panic!("sette record dichiarati contro una geometria non e' un allineamento");
        };
        assert_eq!(
            errore.message,
            "numero di geometrie diverso dal numero di record DBF"
        );

        // La propagazione dalla validazione della struttura: senza il `.shp`
        // non c'e' niente da contare, e il rifiuto arriva prima dell'apertura.
        let assente = dir.path().join("non-esiste.shp");
        assert!(
            infer_geometry_info(&assente, 0).is_err(),
            "un bundle senza `.shp` non ha geometrie da inferire"
        );
    }

    /// Un descrittore di campo DBF: trentadue byte, con tipo, larghezza e
    /// decimali nelle posizioni che il formato fissa.
    ///
    /// Il nome **non** sta qui: `leggi_descrittori_dbf` lo riceve gia'
    /// decodificato da `dbase`, e questa e' la separazione che rende provabile
    /// il rifiuto sui nomi senza costruire un file.
    fn descrittore(tipo: u8, larghezza: u8, decimali: u8) -> [u8; DBF_FIELD_DESCRIPTOR_SIZE] {
        let mut byte = [0_u8; DBF_FIELD_DESCRIPTOR_SIZE];
        byte[11] = tipo;
        byte[16] = larghezza;
        byte[17] = decimali;
        byte
    }

    /// `leggi_descrittori_dbf` rifiuta i nomi che perderebbero una colonna e le
    /// larghezze che non descrivono niente.
    ///
    /// I nomi duplicati sono il caso che vale di piu', e il rifiuto non e'
    /// pignoleria: il DBF confronta i nomi **senza distinguere maiuscole**, e
    /// due colonne omonime finirebbero nella stessa chiave. Chi legge
    /// perderebbe una colonna senza che nulla lo dica -- il file si aprirebbe,
    /// il conteggio dei campi tornerebbe, e mancherebbe un dato.
    ///
    /// Il nome vuoto e la larghezza zero sono la stessa famiglia: un campo che
    /// non ha un nome o non ha byte non e' una colonna, e accettarlo
    /// sposterebbe tutti gli offset successivi.
    ///
    /// Ogni messaggio porta l'**indice**, non il nome: l'indice e' prodotto
    /// dalla nostra enumerazione, il nome viene dal file.
    #[test]
    fn n1_leggi_descrittori_dbf_rifiuta_i_nomi_che_perderebbero_una_colonna() {
        let nomi = |elenco: &[&str]| -> Vec<String> {
            elenco.iter().map(|nome| (*nome).to_owned()).collect()
        };
        let flusso = |quanti: usize, larghezza: u8| {
            let mut byte = Vec::new();
            for _ in 0..quanti {
                byte.extend_from_slice(&descrittore(b'C', larghezza, 0));
            }
            byte
        };

        for (caso, nomi_campi, byte, atteso) in [
            (
                "descrittore troncato",
                nomi(&["NOME"]),
                vec![0_u8; DBF_FIELD_DESCRIPTOR_SIZE - 1],
                "descrittore di campo DBF incompleto",
            ),
            (
                "nome vuoto",
                nomi(&[""]),
                flusso(1, 10),
                "nome campo DBF vuoto, indice 0",
            ),
            (
                "nomi duplicati",
                nomi(&["NOME", "NOME"]),
                flusso(2, 10),
                "nomi campo DBF duplicati; il file e' rifiutato per non perdere una colonna, \
                 secondo indice 1",
            ),
            (
                "nomi duplicati con maiuscole diverse",
                nomi(&["Nome", "NOME"]),
                flusso(2, 10),
                "nomi campo DBF duplicati; il file e' rifiutato per non perdere una colonna, \
                 secondo indice 1",
            ),
            (
                "larghezza zero",
                nomi(&["NOME"]),
                flusso(1, 0),
                "campo DBF con larghezza zero, indice 0",
            ),
        ] {
            let quanti = nomi_campi.len();
            let esito = leggi_descrittori_dbf(&mut std::io::Cursor::new(byte), nomi_campi, quanti);
            let Err(errore) = esito else {
                panic!("{caso}: doveva essere rifiutato");
            };
            assert_eq!(errore.message, atteso, "{caso}: messaggio sbagliato");
        }

        // Le accettazioni, e le tre quantita' che la funzione calcola: gli
        // offset si accumulano a partire dal deletion flag, la lunghezza di
        // record e' l'offset finale, e lo slot di intero esatto si assegna solo
        // ai campi numerici senza decimali larghi almeno dieci.
        let mut byte = Vec::new();
        byte.extend_from_slice(&descrittore(b'C', 10, 0)); // testo: nessuno slot
        byte.extend_from_slice(&descrittore(b'N', 18, 0)); // intero esatto: slot 0
        byte.extend_from_slice(&descrittore(b'N', 9, 0)); // troppo stretto
        byte.extend_from_slice(&descrittore(b'N', 20, 8)); // con decimali
        byte.extend_from_slice(&descrittore(b'N', 12, 0)); // intero esatto: slot 1
        let (campi, lunghezza, esatti) = leggi_descrittori_dbf(
            &mut std::io::Cursor::new(byte),
            nomi(&["TESTO", "GRANDE", "STRETTO", "DECIMALE", "ALTRO"]),
            5,
        )
        .expect("cinque descrittori ben formati");

        assert_eq!(
            campi.iter().map(|campo| campo.offset).collect::<Vec<_>>(),
            vec![1, 11, 29, 38, 58],
            "gli offset partono dal deletion flag e si accumulano"
        );
        assert_eq!(lunghezza, 70, "la lunghezza di record e' l'offset finale");
        assert_eq!(esatti, 2, "solo due campi sono interi esatti");
        assert_eq!(
            campi
                .iter()
                .map(|campo| campo.exact_integer_slot)
                .collect::<Vec<_>>(),
            vec![None, Some(0), None, None, Some(1)],
            "gli slot sono numerati in ordine, e solo per i campi che li meritano"
        );
    }

    /// L'overflow della lunghezza record DBF e' irraggiungibile: l'aritmetica
    /// del formato non ci arriva.
    ///
    /// L'offset accumula larghezze, e una larghezza sta in **un byte**: al piu'
    /// 255. Il numero di descrittori e' limitato dalla lunghezza dell'header,
    /// che sta in un `u16`: al piu' 65 535 byte, cioe' meno di 2048
    /// descrittori da trentadue byte. Il massimo assoluto e' quindi circa mezzo
    /// milione, quindici ordini di grandezza sotto `usize::MAX` su un bersaglio
    /// a sessantaquattro bit. Non c'e' una guardia da eseguire: c'e' un limite
    /// di formato, verificato qui invece che argomentato.
    #[test]
    fn n1_la_lunghezza_record_dbf_non_puo_traboccare() {
        let descrittori_massimi = usize::from(u16::MAX) / DBF_FIELD_DESCRIPTOR_SIZE;
        let larghezza_massima = usize::from(u8::MAX);
        let massimo = 1 + descrittori_massimi * larghezza_massima;
        assert!(
            massimo < usize::MAX / 1_000_000,
            "il massimo di formato ({massimo}) deve restare lontanissimo da usize::MAX: \
             se un giorno non lo fosse, l'overflow andrebbe riclassificato"
        );
        assert!(
            1_usize.checked_add(massimo).is_some(),
            "la somma che la funzione esegue non trabocca nemmeno al massimo di formato"
        );
    }

    /// Un bundle Shapefile valido, con il `.dbf` ritoccato in un campo solo.
    ///
    /// Costruito con il writer vero e poi modificato: cosi' `.shp` e `.shx`
    /// restano quelli di un produttore conforme, e l'unico motivo per cui la
    /// lettura puo' fallire e' il byte che la sonda ha cambiato. Costruirlo a
    /// mano metterebbe in gioco anche la correttezza della fixture, e un
    /// rifiuto non distinguerebbe piu' le due cause.
    fn bundle_con_dbf_ritoccato(
        dir: &Path,
        nome: &str,
        campi: &[&str],
        ritocco: impl FnOnce(&mut Vec<u8>),
    ) -> PathBuf {
        bundle_di_righe(dir, nome, campi, 1, ritocco)
    }

    /// Come sopra, con il numero di righe scelto.
    fn bundle_di_righe(
        dir: &Path,
        nome: &str,
        campi: &[&str],
        righe: usize,
        ritocco: impl FnOnce(&mut Vec<u8>),
    ) -> PathBuf {
        let percorso = dir.join(nome);
        let mut tabella = TableWriterBuilder::new();
        for campo in campi {
            tabella = tabella
                .add_character_field(shapefile::dbase::FieldName::try_from(*campo).unwrap(), 10);
        }
        let mut writer = Writer::from_path(&percorso, tabella).expect("il writer si apre");
        let mut record = Record::default();
        for campo in campi {
            record.insert(
                (*campo).to_owned(),
                FieldValue::Character(Some("uno".to_owned())),
            );
        }
        for _ in 0..righe {
            writer
                .write_shape_and_record(&shapefile::Point::new(1.0, 2.0), &record)
                .expect("un punto si scrive");
        }
        drop(writer);

        let dbf = percorso.with_extension("dbf");
        let mut byte = std::fs::read(&dbf).expect("il dbf esiste");
        ritocco(&mut byte);
        std::fs::write(&dbf, byte).expect("il dbf si riscrive");
        percorso
    }

    /// `read_dbf_layout` rifiuta le incoerenze fra header e descrittori, e le
    /// altre le trova gia' fermate da `valida_intestazione_dbf`.
    ///
    /// Le due funzioni guardano lo stesso header e si sovrappongono di
    /// proposito: `valida_intestazione_dbf` esiste per chiudere i punti in cui
    /// `dbase` panica invece di tornare, quindi corre **prima** e per prima
    /// rifiuta l'header troncato, il backlink `Visual FoxPro` piu' lungo
    /// dell'header, l'offset del primo record piu' corto dell'intestazione e il
    /// terminatore sbagliato. Le righe omonime dentro `read_dbf_layout` restano
    /// come difesa di una lettura che non e' piu' isolata, e non hanno input.
    ///
    /// Cio' che `read_dbf_layout` decide **da se'** e' quello che l'altra
    /// tronca invece di rifiutare: un numero di byte di descrittori che non e'
    /// multiplo di trentadue. E la lunghezza di record dichiarata che non torna
    /// con la somma dei campi -- l'unica delle due che riguarda i **record** e
    /// non l'header.
    ///
    /// Resta fuori il confronto fra il conteggio dell'header e quello dei nomi
    /// decodificati. Non e' dichiarato coperto e non e' dichiarato
    /// irraggiungibile: i due conteggi vengono dalla stessa aritmetica
    /// sull'header, e non ho saputo costruire un file in cui divergano --
    /// nemmeno anticipando il terminatore fra i descrittori, che `dbase`
    /// attraversa senza fermarsi. E' registrato come rischio residuo, perche'
    /// dichiararlo irraggiungibile senza una prova sarebbe la supposizione che
    /// questo censimento esiste per escludere.
    #[test]
    fn n1_read_dbf_layout_rifiuta_le_incoerenze_fra_header_e_descrittori() {
        let dir = tempfile::tempdir().unwrap();

        // Il controllo positivo: il bundle non ritoccato si legge.
        let intatto = bundle_con_dbf_ritoccato(dir.path(), "intatto.shp", &["NOME"], |_| {});
        let layout = read_dbf_layout(&intatto).expect("un bundle conforme ha un layout");
        assert_eq!(layout.fields.len(), 1, "un campo dichiarato, uno letto");
        assert_eq!(layout.record_count, 1, "un record scritto, uno dichiarato");

        for (caso, campi, ritocco, atteso) in [
            (
                "byte di descrittori non multiplo di trentadue",
                &["NOME"][..],
                Box::new(|byte: &mut Vec<u8>| {
                    let lunghezza = u16::from_le_bytes([byte[8], byte[9]]) + 1;
                    byte[8..10].copy_from_slice(&lunghezza.to_le_bytes());
                }) as Box<dyn FnOnce(&mut Vec<u8>)>,
                "lunghezza descrittori DBF non valida",
            ),
            (
                "lunghezza di record che non torna con i campi",
                &["NOME"][..],
                Box::new(|byte: &mut Vec<u8>| {
                    byte[10..12].copy_from_slice(&99_u16.to_le_bytes());
                }),
                "lunghezza di record DBF dichiarata incoerente con i campi, byte richiesti 11",
            ),
        ] {
            let percorso = bundle_con_dbf_ritoccato(
                dir.path(),
                &format!("{}.shp", caso.len()),
                campi,
                ritocco,
            );
            let Err(errore) = read_dbf_layout(&percorso) else {
                panic!("{caso}: doveva essere rifiutato");
            };
            assert_eq!(errore.message, atteso, "{caso}: messaggio sbagliato");
        }
    }

    /// Le righe di `read_dbf_layout` che `valida_intestazione_dbf` raggiunge
    /// per prima.
    ///
    /// Non e' una duplicazione da togliere: `read_dbf_layout` legge l'header
    /// una seconda volta, con la propria aritmetica, e senza quelle guardie
    /// dipenderebbe dal fatto che qualcun altro l'abbia gia' controllato. La
    /// sonda esegue la **precedenza**, cioe' che a rifiutare sia la prima: se
    /// l'ordine cambiasse, il messaggio cambierebbe e la sonda diventerebbe
    /// rossa.
    #[test]
    fn n1_valida_intestazione_dbf_precede_le_guardie_omonime_di_read_dbf_layout() {
        let dir = tempfile::tempdir().unwrap();

        for (caso, ritocco, atteso) in [
            (
                "header troncato sotto i trentadue byte",
                Box::new(|byte: &mut Vec<u8>| byte.truncate(DBF_HEADER_SIZE - 1))
                    as Box<dyn FnOnce(&mut Vec<u8>)>,
                "header DBF incompleto",
            ),
            (
                "offset del primo record dentro l'intestazione",
                Box::new(|byte: &mut Vec<u8>| {
                    byte[8..10].copy_from_slice(&8_u16.to_le_bytes());
                }),
                "offset del primo record DBF piu' corto dell'intestazione",
            ),
            (
                "versione Visual FoxPro con header piu' corto del backlink",
                Box::new(|byte: &mut Vec<u8>| {
                    byte[0] = DBF_VISUAL_FOXPRO_VERSION;
                    byte[8..10].copy_from_slice(&4_u16.to_le_bytes());
                }),
                "header Visual FoxPro piu' corto del backlink",
            ),
        ] {
            let percorso = bundle_con_dbf_ritoccato(
                dir.path(),
                &format!("precedenza-{}.shp", caso.len()),
                &["NOME"],
                ritocco,
            );
            let Err(errore) = read_dbf_layout(&percorso) else {
                panic!("{caso}: doveva essere rifiutato");
            };
            assert_eq!(
                errore.message, atteso,
                "{caso}: a rifiutare deve essere la validazione dell'intestazione"
            );
        }
    }

    /// Un piano di scrittura con la sola geometria, piu' i campi dati.
    fn piano_di_scrittura(campi: Vec<Field>) -> WritePlan {
        let mut colonne = vec![geometry_field(GEOMETRY, "EPSG:4326")];
        colonne.extend(campi);
        WritePlan {
            layers: vec![WriteLayer {
                name: "strato".to_owned(),
                contract: DataContract {
                    schema: Arc::new(Schema::new(colonne)),
                    geometry: None,
                },
            }],
        }
    }

    /// `create` rifiuta il contratto senza geometria, il nome campo che il DBF
    /// non sa portare, e la destinazione gia' occupata in entrambe le forme.
    ///
    /// I tre rifiuti arrivano da tre autorita' diverse, e distinguerli conta.
    /// La colonna geometria manca nel **contratto**, cioe' in cio' che il piano
    /// dichiara. Il nome campo e' un limite del **formato**: il DBF porta al
    /// massimo dieci caratteri ASCII, e troncarlo produrrebbe due colonne
    /// omonime da un piano che ne aveva due distinte. La destinazione occupata
    /// e' lo stato del **disco**, e il no-clobber vale sull'intero set: un
    /// `.shp` che non esiste ma con un `.dbf` accanto e' comunque una
    /// pubblicazione che sovrascriverebbe.
    ///
    /// Le quattro estensioni compagne sono provate una per una, e non con una
    /// sola: sono quattro rami dello stesso ciclo, e una fixture sola non
    /// direbbe che le altre tre sono guardate.
    #[test]
    fn n1_create_rifiuta_il_contratto_il_nome_campo_e_la_destinazione_occupata() {
        let dir = tempfile::tempdir().unwrap();

        // Il controllo positivo: lo stesso piano su una destinazione libera.
        assert!(
            ShpDriver
                .create(
                    Sink::Path(dir.path().join("libero.shp")),
                    &piano_di_scrittura(Vec::new()),
                    &opzioni_scrittura_loose(),
                )
                .is_ok(),
            "un piano con la sola geometria e una destinazione libera si apre"
        );

        // Contratto senza colonna geometria.
        let senza_geometria = WritePlan {
            layers: vec![WriteLayer {
                name: "strato".to_owned(),
                contract: DataContract {
                    schema: Arc::new(Schema::new(vec![Field::new(
                        "nome",
                        arrow_schema::DataType::Utf8,
                        true,
                    )])),
                    geometry: None,
                },
            }],
        };
        let Err(errore) = ShpDriver.create(
            Sink::Path(dir.path().join("senza-geometria.shp")),
            &senza_geometria,
            &opzioni_scrittura_loose(),
        ) else {
            panic!("uno shapefile senza colonna geometria non e' uno shapefile");
        };
        assert_eq!(
            errore.message,
            "il contratto non ha una colonna geometria geoarrow.wkb"
        );

        // Nome campo oltre i dieci caratteri che il DBF porta: il rifiuto
        // **non** e' quello di `create`. Arriva prima dal capability-check, che
        // conosce il tetto sui nomi dal descrittore e lo applica a ogni driver
        // allo stesso modo. La riga di `create` che costruisce il `FieldName`
        // resta come difesa di tipo e non ha input.
        let Err(errore) = ShpDriver.create(
            Sink::Path(dir.path().join("nome-lungo.shp")),
            &piano_di_scrittura(vec![Field::new(
                "un_nome_troppo_lungo",
                arrow_schema::DataType::Utf8,
                true,
            )]),
            &opzioni_scrittura_loose(),
        ) else {
            panic!("un nome oltre il limite del DBF non puo' essere troncato in silenzio");
        };
        assert_eq!(
            errore.message, "nome oltre il limite del formato",
            "il rifiuto deve venire dal capability-check, non da `create`"
        );
        assert_eq!(
            errore.capability_reason,
            Some(plenora_io_model::CapabilityReason::FieldNameTooLong),
            "la ragione dichiarata e' cio' che rende il rifiuto interpretabile"
        );

        // La forma a directory: la destinazione che esiste gia'.
        let cartella = dir.path().join("occupata.shp.d");
        std::fs::create_dir(&cartella).unwrap();
        let Err(errore) = ShpDriver.create(
            Sink::Path(cartella),
            &piano_di_scrittura(Vec::new()),
            &opzioni_scrittura(),
        ) else {
            panic!("una directory che esiste non e' una destinazione libera");
        };
        assert_eq!(
            errore.code,
            plenora_io_model::IoErrorCode::OutputExists,
            "il rifiuto e' il no-clobber: {}",
            errore.message
        );

        // La forma a file sciolti: **ciascuna** delle quattro estensioni
        // compagne blocca la pubblicazione, anche quando il `.shp` non c'e'.
        for estensione in ["shp", "shx", "dbf", "prj"] {
            let destinazione = dir.path().join(format!("occupato-{estensione}.shp"));
            std::fs::write(destinazione.with_extension(estensione), b"").unwrap();
            let Err(errore) = ShpDriver.create(
                Sink::Path(destinazione),
                &piano_di_scrittura(Vec::new()),
                &opzioni_scrittura_loose(),
            ) else {
                panic!("un compagno `.{estensione}` gia' presente blocca il set");
            };
            assert_eq!(
                errore.code,
                plenora_io_model::IoErrorCode::OutputExists,
                "«{estensione}»: {}",
                errore.message
            );
        }
    }

    /// `write` nomina la causa di ogni riga che non sa scrivere, e non ne
    /// scrive nessuna finche' una sola e' rifiutata.
    ///
    /// Le cause sono cinque e sono diverse per chi ripara il dato: una
    /// geometria assente, un WKB che non si decodifica, una forma che
    /// Shapefile non rappresenta, un tipo diverso da quello che il file ha gia'
    /// -- il formato ne porta uno solo -- e una cella che il DBF non sa
    /// portare. Un'unica causa «riga non scrivibile» manderebbe a cercare nel
    /// posto sbagliato quattro volte su cinque.
    ///
    /// La proprieta' che le tiene insieme e' che il rifiuto arriva **prima** di
    /// scrivere: `write` prepara tutte le righe e le consegna al writer solo se
    /// nessuna e' stata rifiutata. Un file con dentro meta' batch sarebbe
    /// peggio di nessun file.
    // Cinque cause, la colonna non binaria e il controllo positivo in un
    // test solo: separarli spezzerebbe il confronto che li rende
    // interpretabili -- quale causa arriva davvero, e da chi.
    #[allow(clippy::too_many_lines)]
    #[test]
    fn n1_write_nomina_la_causa_di_ogni_riga_che_non_sa_scrivere() {
        let dir = tempfile::tempdir().unwrap();
        let punto = to_wkb(&geo_types::Geometry::Point(geo_types::Point::new(1.0, 2.0))).unwrap();
        let collezione = encode_wkb(
            &WkbGeometry {
                value: WkbValue::GeometryCollection(Vec::new()),
                dimensions: CoordinateDimensions::Xy,
                srid: None,
            },
            WkbFlavor::Iso,
        )
        .expect("una collezione vuota si codifica");

        let scrittore = |nome: &str, campi: Vec<Field>| {
            ShpDriver
                .create(
                    Sink::Path(dir.path().join(nome)),
                    &piano_di_scrittura(campi),
                    &opzioni_scrittura_loose(),
                )
                .expect("la destinazione e' libera")
        };
        let schema_solo_geometria: SchemaRef =
            Arc::new(Schema::new(vec![geometry_field(GEOMETRY, "EPSG:4326")]));
        let lotto = |geometrie: Vec<Option<&[u8]>>| {
            RecordBatch::try_new(
                schema_solo_geometria.clone(),
                vec![Arc::new(BinaryArray::from(geometrie))],
            )
            .unwrap()
        };

        for (caso, geometrie, causa) in [
            (
                "geometria assente",
                vec![None],
                "shapefile.null_geometry_unsupported",
            ),
            // Il rifiuto **non** e' quello del driver. `with_write_validation`
            // decodifica il WKB prima di consegnare il lotto, e i byte che non
            // sono WKB non arrivano mai a `ShpWriter::write`: la sua causa
            // `shapefile.invalid_geometry` resta una difesa senza input. La
            // riga successiva mostra invece dove il driver decide da se': una
            // GeometryCollection e' WKB valido, e solo Shapefile sa di non
            // saperla rappresentare.
            (
                "byte che non sono WKB",
                vec![Some(b"non e' WKB".as_slice())],
                "conversion.invalid_geometry",
            ),
            (
                "forma che WKB rappresenta e Shapefile no",
                vec![Some(collezione.as_slice())],
                "shapefile.geometry_not_representable",
            ),
        ] {
            let mut writer = scrittore(&format!("{}.shp", causa.replace('.', "-")), Vec::new());
            writer
                .declare_input_total(LayerId(0), 1)
                .expect("il totale d'ingresso si dichiara prima di scrivere");
            let Err(errore) = writer.write(&lotto(geometrie)) else {
                panic!("{caso}: doveva essere rifiutata");
            };
            let diagnostica = errore
                .row_diagnostics
                .as_deref()
                .expect("un rifiuto di riga porta la propria diagnostica");
            assert_eq!(
                diagnostica.counts.get(causa),
                Some(&1),
                "{caso}: causa attesa «{causa}», arrivate {:?}",
                diagnostica.counts
            );
        }

        // La colonna geometria che non e' binaria: non e' un rifiuto di riga
        // ma del lotto, perche' non c'e' una riga da incolpare. E anche qui il
        // rifiuto **non** e' quello del driver: `with_write_validation`
        // confronta il lotto con il contratto dichiarato prima di consegnarlo,
        // e un tipo Arrow diverso non passa quel confronto. La causa
        // «colonna geometria non binaria» di `ShpWriter::write` resta una
        // difesa senza input, come `shapefile.invalid_geometry`.
        let schema_testo: SchemaRef = Arc::new(Schema::new(vec![Field::new(
            GEOMETRY,
            arrow_schema::DataType::Utf8,
            true,
        )
        .with_metadata(geometry_field(GEOMETRY, "EPSG:4326").metadata().clone())]));
        let non_binaria = RecordBatch::try_new(
            schema_testo,
            vec![Arc::new(arrow_array::StringArray::from(vec![Some(
                "POINT (1 2)",
            )]))],
        )
        .unwrap();
        let mut writer = scrittore("non-binaria.shp", Vec::new());
        writer.declare_input_total(LayerId(0), 1).unwrap();
        let Err(errore) = writer.write(&non_binaria) else {
            panic!("una colonna geometria che non e' WKB non e' una geometria");
        };
        assert_eq!(
            errore.message,
            "batch diverso dal contratto dichiarato (schema, ordine, tipi, nullability o metadata) al layer 0"
        );
        assert!(
            errore.row_diagnostics.is_none(),
            "non c'e' una riga da incolpare: e' il lotto a essere sbagliato"
        );

        // Una cella che il DBF non sa portare: il tipo Arrow non ha una resa
        // testuale decisa, e la riga viene rifiutata con la propria causa.
        //
        // Il tipo e' una **durata**, non piu' una data. Lo Shapefile dichiara
        // `TypeCoercionPolicy::ExplicitText` e ammette la classe `Temporal`:
        // dal 2026-09-04 una data diventa il proprio testo ISO, come su ogni
        // altro formato testuale, e pretendere qui il rifiuto vorrebbe dire
        // scrivere nella prova la contraddizione che la capability aveva.
        // Una durata invece resta senza resa: non e' un istante, e la sua
        // forma testuale e' una scelta di rappresentazione che nessuno ha
        // preso.
        let schema_data: SchemaRef = Arc::new(Schema::new(vec![
            geometry_field(GEOMETRY, "EPSG:4326"),
            Field::new(
                "QUANDO",
                arrow_schema::DataType::Duration(arrow_schema::TimeUnit::Second),
                true,
            ),
        ]));
        let con_data = RecordBatch::try_new(
            schema_data,
            vec![
                Arc::new(BinaryArray::from(vec![Some(punto.as_slice())])),
                Arc::new(arrow_array::DurationSecondArray::from(vec![Some(90_i64)])),
            ],
        )
        .unwrap();
        let mut writer = scrittore(
            "cella.shp",
            vec![Field::new(
                "QUANDO",
                arrow_schema::DataType::Duration(arrow_schema::TimeUnit::Second),
                true,
            )],
        );
        writer.declare_input_total(LayerId(0), 1).unwrap();
        let Err(errore) = writer.write(&con_data) else {
            panic!("una cella senza resa DBF non puo' diventare un valore approssimato");
        };
        let diagnostica = errore.row_diagnostics.as_deref().unwrap();
        assert_eq!(
            diagnostica.counts.get("shapefile.cell_not_representable"),
            Some(&1),
            "arrivate {:?}",
            diagnostica.counts
        );

        // Il controllo positivo: un lotto interamente scrivibile passa, e senza
        // di lui una tabella di soli rifiuti la passerebbe anche un `write` che
        // rifiuta tutto.
        let mut writer = scrittore("buono.shp", Vec::new());
        writer.declare_input_total(LayerId(0), 1).unwrap();
        writer
            .write(&lotto(vec![Some(punto.as_slice())]))
            .expect("un punto XY e' scrivibile");
    }

    /// `write_shape` rifiuta `NullShape` e Multipatch, e `write` non ce li
    /// manda mai.
    ///
    /// Le due righe esistono perche' `Shape` e' un enum totale e il `match` lo
    /// deve essere: sono difese di tipo, non rifiuti raggiungibili. La strada
    /// per arrivarci e' chiusa due volte da `ShpWriter::write`, che rifiuta la
    /// geometria assente come `shapefile.null_geometry_unsupported` prima di
    /// costruire qualunque shape, e il Multipatch come
    /// `shapefile.geometry_type_unsupported` guardando `shape_tag`. In mezzo
    /// c'e' una terza chiusura: `shape_from_wkb` non produce mai `NullShape`,
    /// come la sonda del suo gruppo verifica su tutte le coppie costruibili.
    ///
    /// La sonda esegue entrambe le meta': i due rifiuti chiamati direttamente,
    /// e le due cause con cui `write` li ferma prima.
    #[test]
    fn n1_write_shape_rifiuta_nullshape_e_multipatch_ma_write_non_ce_li_manda() {
        let dir = tempfile::tempdir().unwrap();
        let percorso = dir.path().join("scarto.shp");
        let tabella = TableWriterBuilder::new();
        let mut writer = Writer::from_path(&percorso, tabella).expect("il writer si apre");
        let record = Record::default();

        let Err(errore) = write_shape(&mut writer, Shape::NullShape, &record) else {
            panic!("una geometria nulla non si scrive in uno Shapefile");
        };
        assert_eq!(
            errore.message,
            "geometria nulla non supportata in scrittura Shapefile"
        );

        let multipatch = Shape::Multipatch(shapefile::Multipatch::new(
            shapefile::Patch::TriangleStrip(vec![
                shapefile::PointZ::new(0.0, 0.0, 0.0, NO_DATA),
                shapefile::PointZ::new(1.0, 0.0, 0.0, NO_DATA),
                shapefile::PointZ::new(0.0, 1.0, 0.0, NO_DATA),
            ]),
        ));
        let Err(errore) = write_shape(&mut writer, multipatch, &record) else {
            panic!("il Multipatch non si scrive con questo driver");
        };
        assert_eq!(
            errore.message,
            "Multipatch non supportato in scrittura Shapefile"
        );

        // L'altra meta': le due cause con cui `write` ferma prima. La geometria
        // assente e' gia' provata nella tabella delle cause; qui conta che il
        // tag `unsupported` esista e sia quello del Multipatch, perche' e' il
        // confronto che chiude la strada.
        assert_eq!(
            shape_tag(&Shape::Multipatch(shapefile::Multipatch::new(
                shapefile::Patch::TriangleStrip(vec![shapefile::PointZ::new(
                    0.0, 0.0, 0.0, NO_DATA
                )]),
            ))),
            "unsupported",
            "e' il tag su cui `write` rifiuta la riga prima di costruire la shape"
        );
        assert_eq!(
            shape_tag(&Shape::NullShape),
            "",
            "la NullShape non ha tag, e `write` non la fa mai arrivare qui"
        );
    }

    /// `finish` conta i byte dello staging e non pubblica oltre il tetto.
    ///
    /// Il conteggio precede la pubblicazione, e non e' un dettaglio d'ordine:
    /// un tetto controllato dopo avrebbe gia' scritto i file, e «superato il
    /// limite» sarebbe una constatazione invece di un rifiuto. Il tetto vale
    /// sull'insieme delle quattro parti, non su ciascuna: e' il set a essere
    /// l'unita' pubblicata.
    #[test]
    fn n1_finish_conta_i_byte_dello_staging_prima_di_pubblicare() {
        let dir = tempfile::tempdir().unwrap();
        let punto = to_wkb(&geo_types::Geometry::Point(geo_types::Point::new(1.0, 2.0))).unwrap();
        let schema: SchemaRef = Arc::new(Schema::new(vec![geometry_field(GEOMETRY, "EPSG:4326")]));
        let lotto = RecordBatch::try_new(
            schema,
            vec![Arc::new(BinaryArray::from(vec![Some(punto.as_slice())]))],
        )
        .unwrap();

        // Prima la misura: quanti byte occupa davvero il set di una riga. Il
        // tetto va scelto **sotto** quel valore e sopra cio' che la validazione
        // della scrittura deriva dall'input osservato, altrimenti a rifiutare
        // sarebbe quella e non il conteggio di `finish`.
        let riferimento = dir.path().join("riferimento.shp");
        let mut writer = ShpDriver
            .create(
                Sink::Path(riferimento.clone()),
                &piano_di_scrittura(Vec::new()),
                &opzioni_scrittura_loose(),
            )
            .expect("la destinazione e' libera");
        writer.declare_input_total(LayerId(0), 1).unwrap();
        writer.write(&lotto).expect("il punto si scrive");
        let pubblicato = writer.finish().expect("senza tetto stretto si pubblica");
        let tetto = pubblicato.bytes - 1;

        let destinazione = dir.path().join("stretto.shp");
        let mut writer = ShpDriver
            .create(
                Sink::Path(destinazione.clone()),
                &piano_di_scrittura(Vec::new()),
                &opzioni_scrittura_con(
                    plenora_io_model::budget::PipelineLimits::default()
                        .with_max_output_bytes(tetto),
                )
                .with_format_option("publish_mode", LOOSE_SET_MODE),
            )
            .expect("la destinazione e' libera");
        writer.declare_input_total(LayerId(0), 1).unwrap();
        writer
            .write(&lotto)
            .expect("il punto si scrive nello staging");
        let Err(errore) = writer.finish() else {
            panic!("un set oltre il tetto non si pubblica");
        };
        assert_eq!(
            errore.category,
            plenora_io_model::ErrorCategory::ResourceLimit,
            "un tetto superato e' un limite: {}",
            errore.message
        );
        assert!(
            errore.message.contains("byte oltre il limite di"),
            "il messaggio deve dire qual e' il limite: «{}»",
            errore.message
        );
        for estensione in ["shp", "shx", "dbf", "prj"] {
            assert!(
                !destinazione.with_extension(estensione).exists(),
                "il rifiuto precede la pubblicazione: `.{estensione}` non deve esistere"
            );
        }

        // Il controllo positivo e' la corsa di riferimento qui sopra: stesso
        // lotto, stesso driver, tetto sufficiente, e il set pubblicato.
        assert!(
            riferimento.exists() && riferimento.with_extension("dbf").exists(),
            "sotto il tetto il set si pubblica per intero"
        );
        assert!(
            tetto < pubblicato.bytes,
            "il tetto della prova negativa sta sotto la misura, non accanto"
        );
    }

    /// Uno schema DBF minimo per provare `ShpRowDiagnosticsConfig::from_options`.
    ///
    /// Le colonne e il layout servono a due cose diverse: le prime dicono se il
    /// campo chiave esista, il secondo dice se sia numerico -- e quindi se il
    /// valore grezzo vada tenuto da parte per la diagnostica. Tenerli separati
    /// nella fixture riflette il fatto che nel codice sono due fonti distinte,
    /// e una sola non basta a rispondere a entrambe le domande.
    fn schema_di_prova() -> (Vec<ShpColumn>, DbfLayout) {
        let colonne = vec![
            ShpColumn {
                name: "NOME".to_owned(),
                column_type: ColType::Text,
                exact_integer_slot: None,
            },
            ShpColumn {
                name: "CODICE".to_owned(),
                column_type: ColType::Integer,
                exact_integer_slot: Some(0),
            },
        ];
        let layout = DbfLayout {
            header_length: 65,
            record_length: 21,
            record_count: 0,
            fields: vec![
                DbfFieldLayout {
                    name: "NOME".to_owned(),
                    field_type: b'C',
                    offset: 1,
                    width: 10,
                    exact_integer_slot: None,
                },
                DbfFieldLayout {
                    name: "CODICE".to_owned(),
                    field_type: b'N',
                    offset: 11,
                    width: 10,
                    exact_integer_slot: Some(0),
                },
            ],
            exact_integer_count: 1,
        };
        (colonne, layout)
    }

    /// `from_options` rifiuta ogni configurazione incoerente delle diagnostiche
    /// di riga, e ciascun rifiuto dice che cosa correggere.
    ///
    /// Sono opzioni che chi legge scrive a mano sulla riga di comando, e i
    /// quattro messaggi separano quattro errori diversi: un limite che non e' un
    /// numero, un limite fuori intervallo, una policy senza il campo a cui si
    /// applica, e una policy che non esiste. «Configurazione non valida» li
    /// coprirebbe tutti e non aiuterebbe nessuno.
    ///
    /// Il caso della policy senza campo e' quello che vale di piu': non e' un
    /// valore sbagliato ma una **coppia** incompleta, e senza il rifiuto la
    /// policy verrebbe ignorata in silenzio. Chi l'ha scritta crederebbe di aver
    /// redatto una chiave che invece non viene nemmeno emessa.
    // Sei rifiuti, i due confini del limite e quattro accettazioni: la
    // lunghezza e' quella della classe di equivalenza, non di un test che fa
    // troppe cose.
    #[allow(clippy::too_many_lines)]
    #[test]
    fn n1_from_options_rifiuta_ogni_configurazione_incoerente_delle_diagnostiche() {
        let (colonne, layout) = schema_di_prova();
        let opzioni = |coppie: &[(&str, &str)]| -> BTreeMap<String, String> {
            coppie
                .iter()
                .map(|(chiave, valore)| ((*chiave).to_owned(), (*valore).to_owned()))
                .collect()
        };

        for (caso, coppie, atteso) in [
            (
                "limite che non e' un intero",
                vec![("row_diagnostics.examples_limit", "molti")],
                "row_diagnostics.examples_limit deve essere un intero",
            ),
            (
                "limite negativo, che non e' un u64",
                vec![("row_diagnostics.examples_limit", "-1")],
                "row_diagnostics.examples_limit deve essere un intero",
            ),
            (
                "policy senza il campo a cui si applica",
                vec![("row_diagnostics.key_policy", "emit")],
                "row_diagnostics.key_policy richiede row_diagnostics.key_field",
            ),
            (
                "campo chiave che lo schema DBF non ha",
                vec![("row_diagnostics.key_field", "ASSENTE")],
                "row_diagnostics.key_field non esiste nello schema DBF",
            ),
            (
                "policy che non e' ne' emit ne' redact",
                vec![
                    ("row_diagnostics.key_field", "NOME"),
                    ("row_diagnostics.key_policy", "forse"),
                ],
                "row_diagnostics.key_policy deve essere 'emit' o 'redact'",
            ),
            (
                "campo chiave senza policy: la scelta non ha un default implicito",
                vec![("row_diagnostics.key_field", "NOME")],
                "row_diagnostics.key_policy deve essere 'emit' o 'redact'",
            ),
        ] {
            let esito = ShpRowDiagnosticsConfig::from_options(&opzioni(&coppie), &colonne, &layout);
            let Err(errore) = esito else {
                panic!("{caso}: doveva essere rifiutata");
            };
            assert_eq!(errore.message, atteso, "{caso}: messaggio sbagliato");
            assert_eq!(
                errore.category,
                plenora_io_model::ErrorCategory::InvalidConfiguration,
                "{caso}: e' una configurazione sbagliata, non un dato sbagliato"
            );
            assert_eq!(
                errore.phase,
                plenora_io_model::ErrorPhase::Validate,
                "{caso}: il rifiuto deve arrivare prima di leggere il file"
            );
        }

        // I due confini del limite, uno accanto all'altro: il rifiuto e'
        // fuori, l'accettazione dentro. Con una sola meta' non si saprebbe se
        // l'intervallo sia chiuso o aperto.
        for (caso, valore) in [
            ("zero, sotto il minimo", "0"),
            (
                "uno oltre il massimo",
                &(MAX_ROW_DIAGNOSTICS_EXAMPLES_LIMIT + 1).to_string(),
            ),
        ] {
            let esito = ShpRowDiagnosticsConfig::from_options(
                &opzioni(&[("row_diagnostics.examples_limit", valore)]),
                &colonne,
                &layout,
            );
            let Err(errore) = esito else {
                panic!("{caso}: doveva essere rifiutata");
            };
            assert!(
                errore
                    .message
                    .starts_with("row_diagnostics.examples_limit deve essere compreso fra 1 e"),
                "{caso}: arrivato «{}»",
                errore.message
            );
        }

        for (caso, valore, atteso) in [
            ("il minimo", "1", 1),
            (
                "il massimo",
                &MAX_ROW_DIAGNOSTICS_EXAMPLES_LIMIT.to_string(),
                MAX_ROW_DIAGNOSTICS_EXAMPLES_LIMIT,
            ),
        ] {
            let config = match ShpRowDiagnosticsConfig::from_options(
                &opzioni(&[("row_diagnostics.examples_limit", valore)]),
                &colonne,
                &layout,
            ) {
                Ok(config) => config,
                Err(errore) => panic!("{caso}: doveva essere accettato: {errore:?}"),
            };
            assert_eq!(config.examples_limit, atteso, "{caso}: limite perso");
        }

        // Nessuna opzione: il limite predefinito, e **nessuna** chiave. Il
        // commento del tipo lo dice -- «non esiste una policy implicita» -- e
        // senza questa riga un default che comparisse dal nulla passerebbe.
        let vuota = ShpRowDiagnosticsConfig::from_options(&opzioni(&[]), &colonne, &layout)
            .expect("nessuna opzione e' una configurazione valida");
        assert_eq!(vuota.examples_limit, DEFAULT_ROW_DIAGNOSTICS_EXAMPLES_LIMIT);
        assert!(
            vuota.key.is_none(),
            "senza key_field gli esempi non portano alcun oggetto chiave"
        );

        // Le due policy, e l'indice del campo numerico grezzo: `CODICE` e' di
        // tipo `N`, quindi il valore grezzo va tenuto; `NOME` e' `C`, e non
        // c'e' niente da tenere. Le due meta' vengono da fonti diverse -- le
        // colonne per l'esistenza, il layout per il tipo -- e una sola non
        // risponderebbe a entrambe.
        for (caso, campo, policy, indice_grezzo) in [
            ("campo testuale con emit", "NOME", "emit", None),
            ("campo testuale con redact", "NOME", "redact", None),
            ("campo numerico con emit", "CODICE", "emit", Some(1)),
        ] {
            let config = match ShpRowDiagnosticsConfig::from_options(
                &opzioni(&[
                    ("row_diagnostics.key_field", campo),
                    ("row_diagnostics.key_policy", policy),
                ]),
                &colonne,
                &layout,
            ) {
                Ok(config) => config,
                Err(errore) => panic!("{caso}: doveva essere accettato: {errore:?}"),
            };
            let Some(chiave) = config.key else {
                panic!("{caso}: la chiave doveva esserci");
            };
            assert_eq!(chiave.field, campo, "{caso}: campo perso");
            assert_eq!(
                matches!(chiave.policy, DiagnosticKeyPolicy::Emit),
                policy == "emit",
                "{caso}: policy invertita"
            );
            assert_eq!(
                chiave.raw_numeric_field_index, indice_grezzo,
                "{caso}: il valore grezzo si tiene solo per i campi numerici"
            );
        }
    }

    /// `__fuzz_wkb_roundtrip` propaga ogni rifiuto della catena e chiude il
    /// giro su cio' che l'attraversa.
    ///
    /// E' l'entry point del target del fuzzer, e la sua utilita' dipende da una
    /// cosa sola: che i rifiuti vengano dalla catena vera -- decodifica,
    /// conversione nella shape ESRI, ritorno a WKB -- e non da un controllo
    /// scritto nel target. Una sonda che guardasse solo «non va in panico» non
    /// distinguerebbe un target che legge davvero da uno che rifiuta tutto
    /// all'ingresso.
    #[test]
    fn n1_fuzz_wkb_roundtrip_propaga_i_rifiuti_della_catena() {
        // Byte che non sono WKB: il rifiuto viene dalla decodifica.
        assert!(
            __fuzz_wkb_roundtrip(b"non e' WKB").is_err(),
            "una sequenza che non e' WKB non attraversa la catena"
        );
        assert!(
            __fuzz_wkb_roundtrip(&[]).is_err(),
            "zero byte non sono una geometria"
        );

        // Una geometria vera attraversa e torna: la lunghezza restituita e'
        // quella del WKB rigenerato, quindi non zero.
        let punto = encode_wkb(
            &WkbGeometry {
                value: WkbValue::Point(WkbCoordinate {
                    x: 1.0,
                    y: 2.0,
                    z: None,
                    m: None,
                }),
                dimensions: CoordinateDimensions::Xy,
                srid: None,
            },
            WkbFlavor::Iso,
        )
        .expect("un punto XY si codifica");
        assert!(
            __fuzz_wkb_roundtrip(&punto).expect("un punto attraversa la catena") > 0,
            "il giro completo deve produrre byte, non un contatore a zero"
        );

        // Una forma che WKB rappresenta e Shapefile no: il rifiuto viene dalla
        // conversione, non dalla decodifica.
        let collezione = encode_wkb(
            &WkbGeometry {
                value: WkbValue::GeometryCollection(Vec::new()),
                dimensions: CoordinateDimensions::Xy,
                srid: None,
            },
            WkbFlavor::Iso,
        )
        .expect("una collezione vuota si codifica");
        assert!(
            __fuzz_wkb_roundtrip(&collezione).is_err(),
            "una GeometryCollection non ha una shape ESRI corrispondente"
        );
    }
}
