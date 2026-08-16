//! driver-gpkg — `GeoPackage` ⇄ `RecordBatch` (Fase 1 + ottimizzazione mirata).
//! Geometria WKB nativa: blob `GeoPackageBinaryHeader` + payload WKB, passato
//! senza decodifica (V4). Multi-layer.
//!
//! Prestazioni (misurate contro la baseline): la scrittura usa una singola
//! transazione + `synchronous=OFF`/`journal_mode=MEMORY` (sicuro perché il
//! tempfile è pubblicato atomicamente solo a `finish`) + statement preparato +
//! streaming per batch. La lettura è a pagine (keyset su `rowid`) con builder
//! Arrow tipizzati: memoria O(batch), non O(tabella). Uno
//! `spatial_pruning_hint` usa l'estensione `gpkg_rtree_index` quando registrata
//! e conforme, senza trasformarsi in filtering geometrico esatto.
#![forbid(unsafe_code)]

use std::path::PathBuf;
use std::sync::Arc;

use arrow_array::builder::{BinaryBuilder, Float64Builder, Int64Builder, StringBuilder};
use arrow_array::{
    Array, ArrayRef, BinaryArray, BinaryViewArray, BooleanArray, FixedSizeBinaryArray,
    Float16Array, Float32Array, Float64Array, Int16Array, Int32Array, Int64Array, Int8Array,
    LargeBinaryArray, LargeStringArray, RecordBatch, RecordBatchOptions, StringArray,
    StringViewArray, UInt16Array, UInt32Array, UInt64Array, UInt8Array,
};
use arrow_schema::{DataType, Field, Schema, SchemaRef};
use std::collections::BTreeSet;

use rusqlite::types::{ToSql, ToSqlOutput, ValueRef};
use rusqlite::{Connection, Statement};

use driver_common::{geometry_field, geometry_index};
use plenora_io_core::descriptor::{
    CrsHandling, Direction, Fidelity, FormatDescriptor, ReadMode, ReaderConcurrency, Runtime,
    WriteMode,
};
use plenora_io_core::driver::{
    FormatDriver, FormatWriter, LayerReader, OpenDatasetHandle, Published, ReadOptions, Sink,
    Source, WriteOptions,
};
use plenora_io_core::loss::LossReport;
use plenora_io_core::publish::StagedFile;
use plenora_io_core::request::{Bbox, ProjectionMode, ReadRequest};
use plenora_io_core::{
    validate_write, with_write_validation, ArrowTypeClass, AttributeWriteSupport,
    CrsRepresentationCapabilities, CrsRepresentationState, CrsWriteSupport,
    FormatWriteCapabilities, NullabilitySupport, TypeCoercionPolicy, WritePlan, UTF8_FIELD_NAMES,
    WKB_PASSTHROUGH_GEOMETRY,
};
use plenora_io_model::contract::{
    CoordinateDimensions, DataContract, FieldId, GeometryColumnContract, GeometryType,
    LayerContract, LayerId,
};
use plenora_io_model::crs::{CrsKind, RawCrs, ResolvedCrs};
use plenora_io_model::limits::WkbLimits;
use plenora_io_model::wkb::decode_wkb;

use plenora_io_model::{PlenoraIoError, Result};

fn err(reason: impl Into<String>) -> PlenoraIoError {
    PlenoraIoError::format("gpkg", reason)
}

// Firma per valore imposta dall'uso come funzione in `.map_err(sql_err)`:
// prenderla per riferimento costringerebbe a una closure in ogni chiamata.
#[allow(clippy::needless_pass_by_value)]
fn sql_err(e: rusqlite::Error) -> PlenoraIoError {
    err(format!("sqlite: {e}"))
}

// Categorie di perdita stabili, leggibili dagli harness di conformità. Stesso
// stile di `driver-shp` (dbf_numeric_integer_precision_unverifiable): SQLite
// non impone il tipo dichiarato dalla colonna, quindi un file legittimo può
// consegnare un REAL dove il contratto dichiara INTEGER. La conversione resta
// quella storica, ma smette di essere silenziosa (ADR-IO 5).
const INTEGER_COLUMN_REAL_TRUNCATED: &str = "gpkg_integer_column_real_truncated";
const INTEGER_COLUMN_REAL_SATURATED: &str = "gpkg_integer_column_real_saturated";
const INTEGER_COLUMN_NON_FINITE_DISCARDED: &str = "gpkg_integer_column_non_finite_discarded";
const REAL_COLUMN_INTEGER_PRECISION_UNVERIFIABLE: &str =
    "gpkg_real_column_integer_precision_unverifiable";

/// Primo intero f64 senza precisione unitaria: oltre questa soglia la
/// conversione `i64 -> f64` non è più esatta. Stessa costante di `driver-shp`.
const FIRST_F64_INTEGER_WITHOUT_UNIT_PRECISION: f64 = 9_007_199_254_740_992.0;

/// Limite di saturazione di `f64 as i64`: 2^63, primo valore positivo non
/// rappresentabile. `i64::MIN` vale esattamente -2^63 ed è rappresentabile,
/// quindi la soglia negativa è stretta e quella positiva è inclusiva.
const SATURATION_BOUND: f64 = 9_223_372_036_854_775_808.0;

/// Esito della coercizione dinamica di `SQLite` verso il tipo dichiarato dal
/// contratto. `None` significa che il valore era già rappresentabile.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Coercion {
    /// REAL con parte frazionaria in colonna INTEGER: il valore cambia.
    RealTruncated,
    /// REAL fuori dall'intervallo di `i64`: il valore satura ai limiti.
    RealSaturated,
    /// NaN o infinito in colonna INTEGER: scartato come null, mai convertito
    /// in uno zero plausibile e falso.
    NonFiniteDiscarded,
    /// INTEGER oltre 2^53 in colonna REAL: perde la precisione unitaria.
    IntegerPrecisionUnverifiable,
}

impl Coercion {
    const fn category(self) -> &'static str {
        match self {
            Self::RealTruncated => INTEGER_COLUMN_REAL_TRUNCATED,
            Self::RealSaturated => INTEGER_COLUMN_REAL_SATURATED,
            Self::NonFiniteDiscarded => INTEGER_COLUMN_NON_FINITE_DISCARDED,
            Self::IntegerPrecisionUnverifiable => REAL_COLUMN_INTEGER_PRECISION_UNVERIFIABLE,
        }
    }

    const fn detail(self) -> &'static str {
        match self {
            Self::RealTruncated => "REAL con parte frazionaria troncato verso zero",
            Self::RealSaturated => "REAL fuori dall'intervallo i64 saturato ai limiti",
            Self::NonFiniteDiscarded => "valore non finito scartato come null",
            Self::IntegerPrecisionUnverifiable => {
                "INTEGER oltre 2^53 senza precisione intera unitaria"
            }
        }
    }
}

const GPKG_ATTRIBUTE_TYPES: &[ArrowTypeClass] = &[
    ArrowTypeClass::Boolean,
    ArrowTypeClass::SignedInteger,
    ArrowTypeClass::UnsignedInteger,
    ArrowTypeClass::Floating,
    ArrowTypeClass::Utf8,
    ArrowTypeClass::Binary,
];

static DESCRIPTOR: FormatDescriptor = FormatDescriptor {
    id: "gpkg",
    direction: Direction::Bidirectional,
    read_mode: ReadMode::StreamingSequential, // pagine keyset, O(batch)
    read_determinism: plenora_io_core::DeterminismLevel::Semantic,
    write_mode: Some(WriteMode::Streaming), // per batch, in transazione
    write_determinism: Some(plenora_io_core::DeterminismLevel::Semantic),
    multi_layer: true,
    multi_file: false,
    reader_concurrency: ReaderConcurrency::MultipleIndependentReaders,
    projection_support: plenora_io_core::ProjectionSupport::Exact,
    predicate_pruning_support: plenora_io_core::PredicatePruningSupport::None,
    spatial_pruning_support: plenora_io_core::SpatialPruningSupport::OptionalRtreeIndex,
    crs_handling: CrsHandling::Embedded,
    fidelity_class: Fidelity::Conditional,
    runtime: Runtime::PureRust,
    write_capabilities: Some(FormatWriteCapabilities {
        field_names: UTF8_FIELD_NAMES,
        allowed_types: GPKG_ATTRIBUTE_TYPES,
        type_coercion: TypeCoercionPolicy::Reject,
        attributes: AttributeWriteSupport::All,
        geometry: WKB_PASSTHROUGH_GEOMETRY,
        crs: CrsWriteSupport::Embedded,
        crs_representations: CrsRepresentationCapabilities::new(
            CrsRepresentationState::Preserved,
            CrsRepresentationState::Derived,
            CrsRepresentationState::Derived,
        ),
        nullability: NullabilitySupport::FormatDefined,
        multi_layer: true,
    }),
    semantic_version: 1,
    driver_version: 6,
    descriptor_version: 7,
};

pub struct GpkgDriver;

impl FormatDriver for GpkgDriver {
    fn descriptor(&self) -> &FormatDescriptor {
        &DESCRIPTOR
    }

    fn open(&self, source: Source, mut opts: ReadOptions) -> Result<Box<dyn OpenDatasetHandle>> {
        let path = plenora_io_core::preflight_source(source, &mut opts)?;
        let conn = Connection::open(&path).map_err(sql_err)?;
        let tables = feature_tables(&conn)?;
        if tables.is_empty() {
            return Err(err(
                "nessuna feature table (gpkg_contents data_type='features')",
            ));
        }
        let mut layers = Vec::new();
        let mut metas = Vec::new();
        for (i, table_meta) in tables.into_iter().enumerate() {
            let crs = crs_for(&conn, table_meta.srs_id)?;
            let rtree_table = registered_rtree(&conn, &table_meta.table, &table_meta.geom_col)?;
            let (schema, attrs) =
                build_schema(&conn, &table_meta.table, &table_meta.geom_col, &crs)?;
            let mut geometry = GeometryColumnContract::wkb_passthrough(
                FieldId(0),
                table_meta.geom_col.clone(),
                crs,
                true,
            );
            geometry.dimensions = gpkg_dimensions(table_meta.z, table_meta.m);
            geometry.srid = i32::try_from(table_meta.srs_id).ok();
            if let Some(geometry_type) = gpkg_geometry_type(&table_meta.geometry_type_name) {
                geometry.set_exact_geometry_types(vec![geometry_type]);
            }
            geometry.native_metadata.insert(
                "gpkg.geometry_type_name".to_owned(),
                table_meta.geometry_type_name,
            );
            geometry
                .native_metadata
                .insert("gpkg.z".to_owned(), table_meta.z.to_string());
            geometry
                .native_metadata
                .insert("gpkg.m".to_owned(), table_meta.m.to_string());
            geometry.native_metadata.insert(
                "gpkg.rtree_index".to_owned(),
                rtree_table.is_some().to_string(),
            );
            let contract = DataContract::new(schema, Some(geometry));
            let runtime_schema = contract.schema.clone();
            // `i` indicizza le feature table di `gpkg_contents`: il loro numero
            // e' limitato dalle righe di `sqlite_master`, molti ordini di
            // grandezza sotto 2^32. Nessun troncamento possibile.
            #[allow(clippy::cast_possible_truncation)]
            let layer_id = LayerId(i as u32);
            layers.push(LayerContract {
                id: layer_id,
                name: table_meta.table.clone(),
                contract,
            });
            // `srs_id` di tabella e' i64 in `gpkg_geometry_columns`; l'header
            // per-feature codifica un i32 (finding #7). Se la tabella
            // dichiara un valore fuori range i32 il file e' gia'
            // non-conforme: fallire chiuso qui evita di normalizzarlo
            // silenziosamente prima ancora di leggere una feature.
            let layer_srs_id = i32::try_from(table_meta.srs_id).map_err(|_| {
                err(format!(
                    "gpkg_geometry_columns.srs_id={} fuori dal range i32; \
                     GeoPackage non conforme",
                    table_meta.srs_id
                ))
            })?;
            metas.push(LayerRead {
                rtree_table,
                table: table_meta.table,
                geom_col: table_meta.geom_col,
                schema: runtime_schema,
                attrs,
                layer_srs_id,
            });
        }
        plenora_io_core::with_read_budget(
            Box::new(GpkgDataset {
                path,
                layers,
                metas,
            }),
            &opts,
            false,
        )
    }

    fn create(
        &self,
        sink: Sink,
        plan: &WritePlan,
        opts: &WriteOptions,
    ) -> Result<Box<dyn FormatWriter>> {
        validate_write(self.descriptor(), plan, opts.max_columns())?;
        let Sink::Path(path) = sink;
        if path.exists() {
            return Err(PlenoraIoError::OutputExists(path.display().to_string()));
        }
        if !path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("gpkg"))
        {
            return Err(PlenoraIoError::Unsupported(
                "l'output deve avere estensione .gpkg".to_owned(),
            ));
        }
        let mut names = std::collections::BTreeSet::new();
        for l in &plan.layers {
            if !names.insert(l.name.clone()) {
                return Err(err(format!("nome layer duplicato: {}", l.name)));
            }
            if geometry_index(&l.contract.schema).is_none() {
                return Err(err(format!("layer '{}' senza colonna geometria", l.name)));
            }
        }
        let staging =
            StagedFile::with_suffix(&path, ".gpkg", opts.durable, opts.max_output_bytes())?;
        let conn = Connection::open(staging.path()?).map_err(sql_err)?;
        // Bulk-load veloce: la durabilità è garantita dal publish atomico, non
        // dal file temporaneo (un crash a metà non pubblica nulla).
        conn.execute_batch("PRAGMA synchronous = OFF; PRAGMA journal_mode = MEMORY;")
            .map_err(sql_err)?;
        init_gpkg(&conn)?;
        let mut layers: Vec<ActiveLayer> = Vec::with_capacity(plan.layers.len());
        for l in &plan.layers {
            let geom_idx = geometry_index(&l.contract.schema)
                .ok_or_else(|| err(format!("layer '{}' senza colonna geometria", l.name)))?;
            let (crs_id, crs_def) = layer_crs(l, geom_idx);
            let srs_id = register_srs(&conn, crs_id.as_deref(), crs_def.as_deref())?;
            layers.push(create_feature_table(
                &conn,
                &l.name,
                &l.contract.schema,
                srs_id,
                l.contract.geometry.as_ref(),
            )?);
        }
        if layers.is_empty() {
            return Err(err("WritePlan senza layer"));
        }
        conn.execute_batch("BEGIN").map_err(sql_err)?;
        with_write_validation(
            Box::new(GpkgWriter {
                staging,
                conn: Some(conn),
                layers,
            }),
            self.descriptor(),
            plan,
            opts,
        )
    }
}

// --- lettura (streaming a pagine) -----------------------------------------

struct LayerRead {
    table: String,
    geom_col: String,
    rtree_table: Option<String>,
    schema: SchemaRef,
    attrs: Vec<(String, DataType)>,
    /// SRS ID del layer come dichiarato in `gpkg_geometry_columns`. Il reader
    /// lo confronta con l'`srs_id` dell'header di ogni feature: la
    /// specifica `GeoPackage` vieta la coesistenza di sistemi di riferimento
    /// diversi nella stessa feature table, e una discordanza silenziosa
    /// verrebbe interpretata con il CRS sbagliato (finding #7 review
    /// 2026-08-15).
    layer_srs_id: i32,
}

struct GpkgDataset {
    path: PathBuf,
    layers: Vec<LayerContract>,
    metas: Vec<LayerRead>,
}

impl OpenDatasetHandle for GpkgDataset {
    fn layers(&self) -> &[LayerContract] {
        &self.layers
    }
    fn fidelity_assessment(&self) -> plenora_io_core::FidelityAssessment {
        use plenora_io_core::loss::FidelityReasonCode;

        // `Conditional` senza motivi direbbe solo che la fedeltà "dipende".
        // ADR-IO 5 chiede di elencare cosa la renderebbe approssimante: qui e'
        // la tipizzazione dinamica di SQLite, che non impone il tipo dichiarato
        // dalla colonna. La valutazione resta preventiva; le occorrenze reali
        // finiscono nel `LossReport` del reader.
        let mut assessment = plenora_io_core::FidelityAssessment::for_format(
            DESCRIPTOR.id,
            DESCRIPTOR.fidelity_class,
        );
        assessment.add_reason(
            FidelityReasonCode::TypeCoercion,
            "SQLite non impone il tipo dichiarato: un REAL in colonna INTEGER viene troncato o \
             saturato, un valore non finito viene scartato come null",
        );
        assessment.add_reason(
            FidelityReasonCode::PrecisionChanged,
            "un INTEGER oltre 2^53 in colonna REAL perde la precisione intera unitaria",
        );
        assessment
    }

    fn open_layer_reader(&self, request: &ReadRequest) -> Result<Box<dyn LayerReader>> {
        plenora_io_core::validate_read_projection(&DESCRIPTOR, request)?;
        let idx = self
            .layers
            .iter()
            .position(|l| l.id.0 == request.layer.0)
            .ok_or_else(|| err(format!("layer {} inesistente", request.layer.0)))?;
        let m = &self.metas[idx];
        let conn = Connection::open(&self.path).map_err(sql_err)?;
        let quote = |name: &str| format!("\"{}\"", name.replace('"', "\"\""));
        let (selected, schema, layer) = project_gpkg_layer(m, &self.layers[idx], request)?;
        let attr_cols: Vec<String> = selected
            .attrs
            .iter()
            .map(|(name, _)| format!("t.{}", quote(name)))
            .collect();
        let mut selected_cols =
            Vec::with_capacity(usize::from(selected.geometry) + attr_cols.len());
        if selected.geometry {
            selected_cols.push(format!("t.{}", quote(&m.geom_col)));
        }
        selected_cols.extend(attr_cols);
        let select = if selected_cols.is_empty() {
            "t.rowid".to_owned()
        } else {
            format!("t.rowid, {}", selected_cols.join(", "))
        };
        let spatial_hint = request.spatial_pruning_hint.filter(valid_bbox);
        // Finding #5 della review 2026-08-15: la paginazione keyset con cursore
        // `i64` inizializzato a zero perdeva le feature con `rowid <= 0`.
        // SQLite consente rowid negativi o zero se assegnati esplicitamente
        // dall'INSERT; senza un `Option<i64>` la prima query eseguirebbe
        // `WHERE rowid > 0` e le scarterebbe silenziosamente. Il pattern
        // `(?1 IS NULL OR t.rowid > ?1)` accetta il primo tick con cursore
        // NULL e i successivi con l'ultimo rowid osservato. `ORDER BY
        // t.rowid LIMIT ?2` conserva l'accesso ordinato al b-tree del rowid
        // in entrambi i rami.
        let (sql, spatial_hint) = match (&m.rtree_table, spatial_hint) {
            (Some(rtree), Some(bbox)) => (
                format!(
                    "SELECT {select} FROM {} AS t
                     JOIN {} AS r ON r.id = t.rowid
                     WHERE (?1 IS NULL OR t.rowid > ?1)
                       AND r.maxx >= ?3 AND r.minx <= ?4
                       AND r.maxy >= ?5 AND r.miny <= ?6
                     ORDER BY t.rowid LIMIT ?2",
                    quote(&m.table),
                    quote(rtree),
                ),
                Some(bbox),
            ),
            _ => (
                format!(
                    "SELECT {select} FROM {} AS t
                     WHERE (?1 IS NULL OR t.rowid > ?1) ORDER BY t.rowid LIMIT ?2",
                    quote(&m.table),
                ),
                None,
            ),
        };
        let reader: Box<dyn LayerReader> = Box::new(GpkgReader {
            conn,
            sql,
            spatial_hint,
            schema: schema.clone(),
            include_geometry: selected.geometry,
            attrs: selected.attrs,
            batch_sizer: plenora_io_core::AdaptiveBatchSizer::new(
                schema.as_ref(),
                request.batch_target,
            ),
            last_rowid: None,
            layer_srs_id: m.layer_srs_id,
            layer,
            loss: LossReport::default(),
            reported: BTreeSet::new(),
        });
        Ok(plenora_io_core::with_cancellation(
            reader,
            request.cancellation.clone(),
        ))
    }
}

struct ProjectedGpkgColumns {
    geometry: bool,
    attrs: Vec<(String, DataType)>,
}

fn project_gpkg_layer(
    meta: &LayerRead,
    source_layer: &LayerContract,
    request: &ReadRequest,
) -> Result<(ProjectedGpkgColumns, SchemaRef, LayerContract)> {
    let mut indices = match &request.projected_fields {
        None => (0..meta.schema.fields().len()).collect::<Vec<_>>(),
        Some(field_ids) => {
            let mut indices = Vec::with_capacity(field_ids.len());
            for field_id in field_ids {
                let index = field_id.0 as usize;
                if index >= meta.schema.fields().len() {
                    if request.projection_mode == ProjectionMode::Required {
                        return Err(PlenoraIoError::Contract(format!(
                            "projection Required: field id {} fuori range",
                            field_id.0
                        )));
                    }
                    continue;
                }
                if !indices.contains(&index) {
                    indices.push(index);
                }
            }
            indices
        }
    };
    indices.sort_unstable();

    let fields = indices
        .iter()
        .map(|&index| meta.schema.field(index).as_ref().clone())
        .collect::<Vec<_>>();
    let schema = Arc::new(Schema::new_with_metadata(
        fields,
        meta.schema.metadata().clone(),
    ));
    let geometry = indices.first() == Some(&0);
    let attrs = indices
        .iter()
        .filter_map(|&index| {
            index
                .checked_sub(1)
                .and_then(|attr_index| meta.attrs.get(attr_index))
                .cloned()
        })
        .collect();
    let mut layer = source_layer.clone();
    let projected_geometry = if geometry {
        layer
            .contract
            .geometry
            .map(|geometry| GeometryColumnContract {
                field_id: FieldId(0),
                ..geometry
            })
    } else {
        None
    };
    layer.contract = DataContract::new(schema, projected_geometry);
    let runtime_schema = layer.contract.schema.clone();
    Ok((
        ProjectedGpkgColumns { geometry, attrs },
        runtime_schema,
        layer,
    ))
}

struct GpkgReader {
    conn: Connection,
    sql: String,
    spatial_hint: Option<Bbox>,
    schema: SchemaRef,
    include_geometry: bool,
    attrs: Vec<(String, DataType)>,
    batch_sizer: plenora_io_core::AdaptiveBatchSizer,
    /// Cursore keyset per la paginazione. `None` prima della prima pagina;
    /// dopo la prima pagina resta `Some(rowid)` dell'ultimo record osservato.
    /// Usare `Option` invece di un sentinel `i64` copre anche i rowid `<= 0`
    /// che `SQLite` consente esplicitamente (finding #5).
    last_rowid: Option<i64>,
    /// SRS ID di tabella per il confronto per-feature (finding #7).
    layer_srs_id: i32,
    layer: LayerContract,
    /// Perdite osservate leggendo: conteggi per categoria ed esempi bounded.
    loss: LossReport,
    /// Coppie (campo, categoria) già illustrate da un esempio: gli esempi sono
    /// bounded e non devono ripetersi a ogni riga.
    reported: BTreeSet<(String, &'static str)>,
}

impl LayerReader for GpkgReader {
    fn contract(&self) -> &LayerContract {
        &self.layer
    }

    fn loss_report(&self) -> LossReport {
        self.loss.clone()
    }

    fn next_batch(&mut self) -> Result<Option<RecordBatch>> {
        let mut stmt = self.conn.prepare_cached(&self.sql).map_err(sql_err)?;
        let mut geom = BinaryBuilder::new();
        let mut attr_builders: Vec<ColBuilder> = self
            .attrs
            .iter()
            .map(|(_, dt)| ColBuilder::new(dt))
            .collect();
        let mut count = 0usize;
        // Prima della prima pagina il massimo osservato non esiste. Diventa
        // `Some(rowid)` non appena una riga viene letta e serve poi come
        // nuovo cursore keyset e come guardia anti-loop.
        let mut max_rowid: Option<i64> = self.last_rowid;
        // Clamp identico al precedente `min(i64::MAX as usize) as i64`, senza
        // cast: il LIMIT SQL satura a i64::MAX invece di avvolgersi.
        let limit = i64::try_from(self.batch_sizer.rows()).unwrap_or(i64::MAX);
        let mut rows = match self.spatial_hint {
            Some(bbox) => stmt.query(rusqlite::params![
                self.last_rowid,
                limit,
                bbox.minx,
                bbox.maxx,
                bbox.miny,
                bbox.maxy,
            ]),
            None => stmt.query(rusqlite::params![self.last_rowid, limit]),
        }
        .map_err(sql_err)?;
        while let Some(row) = rows.next().map_err(sql_err)? {
            let rowid: i64 = row.get(0).map_err(sql_err)?;
            // `Option::max` conserva il maggiore per confronto totale su i64.
            max_rowid = Some(max_rowid.map_or(rowid, |current| current.max(rowid)));
            let mut column = 1;
            if self.include_geometry {
                match row.get_ref(column).map_err(sql_err)? {
                    ValueRef::Null => geom.append_null(),
                    ValueRef::Blob(b) => {
                        let header = parse_gpkg_header(b)?;
                        // Finding #7: SRS discordante fra header per-feature
                        // e dichiarazione di tabella e' un GeoPackage non
                        // conforme: proseguire etichetterebbe le coordinate
                        // con il CRS sbagliato. Fail-closed anche nel caso
                        // simmetrico di layer con srs_id "undefined" (0/-1):
                        // il file dichiara di essere consistente, se non lo
                        // e' non spetta al reader inventare quale delle due
                        // dichiarazioni sia autoritativa.
                        if header.srs_id != self.layer_srs_id {
                            return Err(err(format!(
                                "SRS per-feature {} discordante dal layer {}; \
                                 GeoPackage non conforme",
                                header.srs_id, self.layer_srs_id
                            )));
                        }
                        geom.append_value(header.payload);
                    }
                    _ => return Err(err("colonna geometria non è un BLOB")),
                }
                column += 1;
            }
            for (i, b) in attr_builders.iter_mut().enumerate() {
                let Some(coercion) = b.append(row.get_ref(column + i).map_err(sql_err)?) else {
                    continue;
                };
                let category = coercion.category();
                self.loss.record(category, 1);
                let name = self.attrs[i].0.clone();
                if self.reported.insert((name.clone(), category)) {
                    self.loss.add_example(plenora_io_core::loss::LossExample {
                        category: category.to_owned(),
                        context: format!("field={name}: {}", coercion.detail()),
                    });
                }
            }
            count += 1;
        }
        if count == 0 {
            return Ok(None);
        }
        // La paginazione e' keyset su `rowid`: ogni pagina riparte da
        // `WHERE rowid > last_rowid`. Se il massimo osservato non supera il
        // cursore, la pagina successiva sarebbe identica a questa e il reader
        // non terminerebbe mai.
        //
        // Su un GeoPackage sano non puo' accadere: la clausola garantisce
        // `rowid > last_rowid` per ogni riga restituita. Su un file corrotto
        // si': SQLite confronta la chiave del b-tree, mentre il valore
        // restituito viene dal record, e i due possono divergere. Trovato dal
        // fuzzing con un file di 32 KiB che restituiva all'infinito la stessa
        // riga con `rowid` riportato 0, portando il processo oltre 4 GiB
        // residenti — non per una singola allocazione, ma per milioni di
        // iterazioni che allocano e liberano.
        //
        // Fail-closed: un file che non permette di avanzare e' incoerente, e
        // proseguire produrrebbe righe duplicate all'infinito. Con cursore
        // `Option<i64>` la guardia deve considerare `None` (prima pagina):
        // se una prima pagina non produce alcun rowid oltre nessun cursore,
        // il caso `count == 0` sopra ha gia' restituito Ok(None). Qui siamo
        // dopo che almeno una riga e' stata osservata, quindi `max_rowid` e'
        // sempre `Some(_)`. La guardia scatta se il cursore precedente esiste
        // ed e' >= del nuovo massimo.
        let Some(observed_max) = max_rowid else {
            return Err(err(
                "paginazione bloccata: pagina non vuota senza rowid osservato; \
                 il GeoPackage e' incoerente",
            ));
        };
        if let Some(previous) = self.last_rowid {
            if observed_max <= previous {
                return Err(err(format!(
                    "paginazione bloccata: rowid massimo {observed_max} non supera il cursore {previous}; \
                     il GeoPackage e' incoerente"
                )));
            }
        }
        self.last_rowid = Some(observed_max);
        let mut arrays: Vec<ArrayRef> =
            Vec::with_capacity(usize::from(self.include_geometry) + attr_builders.len());
        if self.include_geometry {
            arrays.push(Arc::new(geom.finish()));
        }
        for b in attr_builders {
            arrays.push(b.finish());
        }
        let options = RecordBatchOptions::new().with_row_count(Some(count));
        let batch = RecordBatch::try_new_with_options(self.schema.clone(), arrays, &options)
            .map_err(|e| err(format!("batch: {e}")))?;
        self.batch_sizer.observe(&batch);
        Ok(Some(batch))
    }
}

enum ColBuilder {
    I64(Int64Builder),
    F64(Float64Builder),
    Str(StringBuilder),
    Bin(BinaryBuilder),
}

impl ColBuilder {
    fn new(dt: &DataType) -> Self {
        match dt {
            DataType::Int64 => Self::I64(Int64Builder::new()),
            DataType::Float64 => Self::F64(Float64Builder::new()),
            DataType::Binary => Self::Bin(BinaryBuilder::new()),
            _ => Self::Str(StringBuilder::new()),
        }
    }
    /// Restituisce la coercizione applicata, se il valore non era già
    /// rappresentabile nel tipo dichiarato dal contratto. Il chiamante la
    /// registra nel `LossReport`: la conversione resta quella storica, ma non
    /// e' piu' silenziosa (ADR-IO 5).
    fn append(&mut self, v: ValueRef) -> Option<Coercion> {
        match self {
            Self::I64(b) => match v {
                ValueRef::Integer(i) => {
                    b.append_value(i);
                    None
                }
                // SQLite e' tipizzato dinamicamente: un REAL puo' arrivare in
                // una colonna dichiarata INTEGER anche in un file legittimo.
                ValueRef::Real(r) => {
                    if !r.is_finite() {
                        // Un NaN convertito con `as i64` diventerebbe 0: un
                        // valore plausibile e falso. Si scarta, come fa
                        // `driver-xls` con la guardia `is_finite`.
                        b.append_null();
                        return Some(Coercion::NonFiniteDiscarded);
                    }
                    // `as` satura ai limiti di i64 e tronca verso zero: e' la
                    // conversione storica e non va cambiata, ma i due casi
                    // vanno distinti perche' la perdita e' di natura diversa.
                    let saturated = !(-SATURATION_BOUND..SATURATION_BOUND).contains(&r);
                    #[allow(clippy::cast_possible_truncation)]
                    b.append_value(r as i64);
                    if saturated {
                        Some(Coercion::RealSaturated)
                    } else if r.fract() == 0.0 {
                        None
                    } else {
                        Some(Coercion::RealTruncated)
                    }
                }
                _ => {
                    b.append_null();
                    None
                }
            },
            Self::F64(b) => match v {
                ValueRef::Real(r) => {
                    b.append_value(r);
                    None
                }
                // Colonna dichiarata REAL: l'INTEGER SQLite viene rappresentato
                // in f64. Oltre 2^53 la precisione unitaria si perde, ed e' la
                // stessa condizione che `driver-shp` registra sui Numeric DBF.
                ValueRef::Integer(i) => {
                    #[allow(clippy::cast_precision_loss)]
                    let value = i as f64;
                    b.append_value(value);
                    (value.abs() >= FIRST_F64_INTEGER_WITHOUT_UNIT_PRECISION)
                        .then_some(Coercion::IntegerPrecisionUnverifiable)
                }
                _ => {
                    b.append_null();
                    None
                }
            },
            Self::Str(b) => {
                match v {
                    ValueRef::Text(t) => b.append_value(String::from_utf8_lossy(t)),
                    // La rappresentazione testuale di un intero o di un reale
                    // e' fedele: nessuna perdita da dichiarare.
                    ValueRef::Integer(i) => b.append_value(i.to_string()),
                    ValueRef::Real(r) => b.append_value(r.to_string()),
                    // NULL e BLOB in colonna testuale: nessuna transcodifica
                    // implicita, si scrive null.
                    ValueRef::Null | ValueRef::Blob(_) => b.append_null(),
                }
                None
            }
            Self::Bin(b) => {
                match v {
                    ValueRef::Blob(x) => b.append_value(x),
                    _ => b.append_null(),
                }
                None
            }
        }
    }
    fn finish(mut self) -> ArrayRef {
        match &mut self {
            Self::I64(b) => Arc::new(b.finish()),
            Self::F64(b) => Arc::new(b.finish()),
            Self::Str(b) => Arc::new(b.finish()),
            Self::Bin(b) => Arc::new(b.finish()),
        }
    }
}

// --- scrittura (transazione + streaming) -----------------------------------

struct ActiveLayer {
    geom_idx: usize,
    insert_sql: String,
    srs_id: i32,
}

struct GpkgWriter {
    staging: StagedFile,
    conn: Option<Connection>,
    layers: Vec<ActiveLayer>,
}

impl Drop for GpkgWriter {
    fn drop(&mut self) {
        // Su Windows il tempfile non può essere rimosso finché SQLite mantiene
        // aperto il file: l'ordine esplicito garantisce abort senza residui.
        drop(self.conn.take());
    }
}

impl FormatWriter for GpkgWriter {
    fn write(&mut self, batch: &RecordBatch) -> Result<()> {
        self.write_to_layer(LayerId(0), batch)
    }

    fn write_to_layer(&mut self, layer: LayerId, batch: &RecordBatch) -> Result<()> {
        let conn = self.conn.as_ref().ok_or_else(|| err("writer chiuso"))?;
        let a = self.layers.get(layer.0 as usize).ok_or_else(|| {
            err(format!(
                "layer {} inesistente nel piano di scrittura",
                layer.0
            ))
        })?;
        insert_batch(conn, a, batch)
    }

    fn finish(mut self: Box<Self>) -> Result<Published> {
        let conn = self.conn.take().ok_or_else(|| err("writer già chiuso"))?;
        conn.execute_batch("COMMIT").map_err(sql_err)?;
        drop(conn);
        let (bytes, outcome) = self.staging.publish()?;
        Ok(Published {
            bytes,
            loss: LossReport::default(),
            fidelity: plenora_io_core::FidelityAssessment::lossless(),
            outcome,
        })
    }
}

fn insert_batch(conn: &Connection, a: &ActiveLayer, batch: &RecordBatch) -> Result<()> {
    let geom_col = batch
        .column(a.geom_idx)
        .as_any()
        .downcast_ref::<BinaryArray>()
        .ok_or_else(|| err("colonna geometria non binaria"))?;
    let attr_idx: Vec<usize> = (0..batch.num_columns())
        .filter(|i| *i != a.geom_idx)
        .collect();
    let mut stmt = conn.prepare_cached(&a.insert_sql).map_err(sql_err)?;
    let mut geometry_blob = Vec::new();
    for row in 0..batch.num_rows() {
        geometry_blob.clear();
        let geometry = if geom_col.is_null(row) {
            None
        } else {
            let payload = geom_col.value(row);
            // Finding #12 follow-up review 2026-08-15: `wkb_shape` fallisce
            // chiuso su payload troncati o byte-order/flavor non validi.
            // Un WKB ambiguo non viene scritto con un flag "empty"
            // arbitrario: la feature viene rifiutata a monte del publish.
            let shape = wkb_shape(payload)?;
            geometry_blob.extend_from_slice(&gpkg_header(a.srs_id, shape.is_empty()));
            geometry_blob.extend_from_slice(payload);
            Some(geometry_blob.as_slice())
        };
        execute_insert_row(&mut stmt, batch, &attr_idx, row, geometry)?;
    }
    Ok(())
}

fn execute_insert_row(
    statement: &mut Statement<'_>,
    batch: &RecordBatch,
    attribute_indices: &[usize],
    row: usize,
    geometry: Option<&[u8]>,
) -> Result<()> {
    let mut params = Vec::with_capacity(1 + attribute_indices.len());
    params.push(geometry.map_or(BorrowedSqlValue::Null, BorrowedSqlValue::Blob));
    for &index in attribute_indices {
        params.push(arrow_cell_to_sql_ref(batch.column(index), row)?);
    }
    statement
        .execute(rusqlite::params_from_iter(params.iter()))
        .map_err(sql_err)?;
    Ok(())
}

#[derive(Debug, PartialEq)]
enum BorrowedSqlValue<'a> {
    Null,
    Integer(i64),
    Real(f64),
    Text(&'a str),
    Blob(&'a [u8]),
}

impl ToSql for BorrowedSqlValue<'_> {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        Ok(match self {
            Self::Null => ToSqlOutput::Borrowed(ValueRef::Null),
            Self::Integer(value) => ToSqlOutput::Borrowed(ValueRef::Integer(*value)),
            Self::Real(value) => ToSqlOutput::Borrowed(ValueRef::Real(*value)),
            Self::Text(value) => ToSqlOutput::Borrowed(ValueRef::Text(value.as_bytes())),
            Self::Blob(value) => ToSqlOutput::Borrowed(ValueRef::Blob(value)),
        })
    }
}

fn arrow_cell_to_sql_ref(array: &ArrayRef, row: usize) -> Result<BorrowedSqlValue<'_>> {
    if array.is_null(row) {
        return Ok(BorrowedSqlValue::Null);
    }
    let a = array.as_any();
    macro_rules! signed_integer {
        ($array:ty) => {
            if let Some(values) = a.downcast_ref::<$array>() {
                return Ok(BorrowedSqlValue::Integer(i64::from(values.value(row))));
            }
        };
    }
    signed_integer!(Int8Array);
    signed_integer!(Int16Array);
    signed_integer!(Int32Array);
    signed_integer!(Int64Array);
    macro_rules! unsigned_integer {
        ($array:ty) => {
            if let Some(values) = a.downcast_ref::<$array>() {
                return Ok(BorrowedSqlValue::Integer(i64::from(values.value(row))));
            }
        };
    }
    unsigned_integer!(UInt8Array);
    unsigned_integer!(UInt16Array);
    unsigned_integer!(UInt32Array);
    if let Some(values) = a.downcast_ref::<UInt64Array>() {
        let value = i64::try_from(values.value(row)).map_err(|_| {
            err("UInt64 oltre i64::MAX non rappresentabile come INTEGER GeoPackage")
        })?;
        return Ok(BorrowedSqlValue::Integer(value));
    }
    if let Some(x) = a.downcast_ref::<Float16Array>() {
        let value = f64::from(f32::from(x.value(row)));
        if !value.is_finite() {
            return Err(err("Float16 non finito non rappresentabile in GeoPackage"));
        }
        return Ok(BorrowedSqlValue::Real(value));
    }
    if let Some(x) = a.downcast_ref::<Float32Array>() {
        let value = f64::from(x.value(row));
        if !value.is_finite() {
            return Err(err("Float32 non finito non rappresentabile in GeoPackage"));
        }
        return Ok(BorrowedSqlValue::Real(value));
    }
    if let Some(x) = a.downcast_ref::<Float64Array>() {
        let value = x.value(row);
        if !value.is_finite() {
            return Err(err("Float64 non finito non rappresentabile in GeoPackage"));
        }
        return Ok(BorrowedSqlValue::Real(value));
    }
    if let Some(x) = a.downcast_ref::<BooleanArray>() {
        return Ok(BorrowedSqlValue::Integer(i64::from(x.value(row))));
    }
    if let Some(x) = a.downcast_ref::<StringArray>() {
        return Ok(BorrowedSqlValue::Text(x.value(row)));
    }
    if let Some(x) = a.downcast_ref::<LargeStringArray>() {
        return Ok(BorrowedSqlValue::Text(x.value(row)));
    }
    if let Some(x) = a.downcast_ref::<StringViewArray>() {
        return Ok(BorrowedSqlValue::Text(x.value(row)));
    }
    if let Some(x) = a.downcast_ref::<BinaryArray>() {
        return Ok(BorrowedSqlValue::Blob(x.value(row)));
    }
    if let Some(x) = a.downcast_ref::<LargeBinaryArray>() {
        return Ok(BorrowedSqlValue::Blob(x.value(row)));
    }
    if let Some(x) = a.downcast_ref::<BinaryViewArray>() {
        return Ok(BorrowedSqlValue::Blob(x.value(row)));
    }
    if let Some(x) = a.downcast_ref::<FixedSizeBinaryArray>() {
        return Ok(BorrowedSqlValue::Blob(x.value(row)));
    }
    Err(PlenoraIoError::Unsupported(format!(
        "GeoPackage: tipo Arrow {:?} non rappresentabile senza conversione esplicita",
        array.data_type()
    )))
}

// --- helpers comuni --------------------------------------------------------

struct FeatureTable {
    table: String,
    geom_col: String,
    srs_id: i64,
    geometry_type_name: String,
    z: i64,
    m: i64,
}

fn valid_bbox(bbox: &Bbox) -> bool {
    bbox.minx.is_finite()
        && bbox.miny.is_finite()
        && bbox.maxx.is_finite()
        && bbox.maxy.is_finite()
        && bbox.minx <= bbox.maxx
        && bbox.miny <= bbox.maxy
}

fn registered_rtree(conn: &Connection, table: &str, geom_col: &str) -> Result<Option<String>> {
    if !sqlite_table_exists(conn, "gpkg_extensions")? {
        return Ok(None);
    }
    let registered = conn.query_row(
        "SELECT EXISTS(
             SELECT 1 FROM gpkg_extensions
             WHERE table_name = ?1 AND column_name = ?2
               AND extension_name = 'gpkg_rtree_index'
         )",
        rusqlite::params![table, geom_col],
        |row| row.get::<_, bool>(0),
    );
    if !matches!(registered, Ok(true)) {
        return Ok(None);
    }

    let rtree = format!("rtree_{table}_{geom_col}");
    if !sqlite_table_exists(conn, &rtree)? {
        return Ok(None);
    }
    let quoted = format!("\"{}\"", rtree.replace('"', "\"\""));
    let Ok(mut statement) = conn.prepare(&format!("PRAGMA table_info({quoted})")) else {
        return Ok(None);
    };
    let Ok(columns) = statement.query_map([], |row| row.get::<_, String>(1)) else {
        return Ok(None);
    };
    let mut names = Vec::new();
    for column in columns {
        let Ok(column) = column else {
            return Ok(None);
        };
        names.push(column.to_ascii_lowercase());
    }
    let expected = ["id", "minx", "maxx", "miny", "maxy"];
    if expected.iter().all(|name| names.iter().any(|n| n == name)) {
        Ok(Some(rtree))
    } else {
        Ok(None)
    }
}

fn sqlite_table_exists(conn: &Connection, table: &str) -> Result<bool> {
    conn.query_row(
        "SELECT EXISTS(
             SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1
         )",
        [table],
        |row| row.get(0),
    )
    .map_err(sql_err)
}

fn feature_tables(conn: &Connection) -> Result<Vec<FeatureTable>> {
    let mut out = Vec::new();
    let mut stmt = conn
        .prepare(
            "SELECT c.table_name, g.column_name, g.srs_id,
                    g.geometry_type_name, g.z, g.m
             FROM gpkg_contents c JOIN gpkg_geometry_columns g ON c.table_name = g.table_name
             WHERE c.data_type = 'features' ORDER BY c.table_name",
        )
        .map_err(sql_err)?;
    let rows = stmt
        .query_map([], |r| {
            Ok(FeatureTable {
                table: r.get(0)?,
                geom_col: r.get(1)?,
                srs_id: r.get(2)?,
                geometry_type_name: r.get(3)?,
                z: r.get(4)?,
                m: r.get(5)?,
            })
        })
        .map_err(sql_err)?;
    for r in rows {
        out.push(r.map_err(sql_err)?);
    }
    Ok(out)
}

const fn gpkg_dimensions(z: i64, m: i64) -> CoordinateDimensions {
    match (z, m) {
        (0, 0) => CoordinateDimensions::Xy,
        (1, 0) => CoordinateDimensions::Xyz,
        (0, 1) => CoordinateDimensions::Xym,
        (1, 1) => CoordinateDimensions::Xyzm,
        // GeoPackage usa 2 per "optional": non attesta che l'ordinata sia
        // presente in ogni geometria, quindi il contratto runtime resta
        // deliberatamente Unknown invece di inventare XYZ/XYZM.
        _ => CoordinateDimensions::Unknown,
    }
}

fn gpkg_geometry_type(name: &str) -> Option<GeometryType> {
    Some(match name.to_ascii_uppercase().as_str() {
        "POINT" => GeometryType::Point,
        "LINESTRING" => GeometryType::LineString,
        "POLYGON" => GeometryType::Polygon,
        "MULTIPOINT" => GeometryType::MultiPoint,
        "MULTILINESTRING" => GeometryType::MultiLineString,
        "MULTIPOLYGON" => GeometryType::MultiPolygon,
        "GEOMETRYCOLLECTION" => GeometryType::GeometryCollection,
        // "GEOMETRY" (tipo generico) e ogni altro nome non vincolano il
        // contratto: nessun tipo esatto viene dichiarato.
        _ => return None,
    })
}

fn crs_for(conn: &Connection, srs_id: i64) -> Result<ResolvedCrs> {
    let row: rusqlite::Result<(String, i64, String)> = conn.query_row(
        "SELECT organization, organization_coordsys_id, definition FROM gpkg_spatial_ref_sys WHERE srs_id = ?1",
        [srs_id],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    );
    match row {
        Ok((org, code, def)) => {
            let id = format!("{}:{}", org.to_uppercase(), code);
            if org.eq_ignore_ascii_case("NONE") {
                let raw = RawCrs::new(
                    format!("GeoPackage srs_id={srs_id}; definition={def}"),
                    Some(id),
                );
                return Err(PlenoraIoError::crs_unresolved("gpkg", &raw));
            }
            let kind = if id.eq_ignore_ascii_case("EPSG:4326") {
                CrsKind::Geographic
            } else if id.eq_ignore_ascii_case("EPSG:3857")
                || def.to_ascii_uppercase().contains("PROJCS[")
                || def.to_ascii_uppercase().contains("PROJCRS[")
            {
                CrsKind::Projected
            } else if def.to_ascii_uppercase().contains("GEOGCS[")
                || def.to_ascii_uppercase().contains("GEOGCRS[")
            {
                CrsKind::Geographic
            } else {
                CrsKind::Unknown
            };
            let definition = if def == "undefined" || def.is_empty() {
                None
            } else {
                Some(def)
            };
            Ok(ResolvedCrs::new(Some(id), kind, definition))
        }
        Err(rusqlite::Error::QueryReturnedNoRows) => {
            let raw = RawCrs::new(format!("GeoPackage srs_id={srs_id}"), None);
            Err(PlenoraIoError::crs_unresolved("gpkg", &raw))
        }
        Err(error) => Err(err(format!("lettura gpkg_spatial_ref_sys: {error}"))),
    }
}

fn sqlite_declared_to_arrow(t: &str) -> DataType {
    let t = t.to_ascii_uppercase();
    if t.contains("INT") {
        DataType::Int64
    } else if t.contains("REAL") || t.contains("FLOA") || t.contains("DOUB") {
        DataType::Float64
    } else if t.contains("BLOB") {
        DataType::Binary
    } else {
        DataType::Utf8
    }
}

/// Schema (senza leggere dati) dai tipi dichiarati; attributi = colonne non
/// geometria e non chiave primaria.
fn build_schema(
    conn: &Connection,
    table: &str,
    geom_col: &str,
    crs: &ResolvedCrs,
) -> Result<(SchemaRef, Vec<(String, DataType)>)> {
    let mut stmt = conn
        .prepare(&format!(
            "PRAGMA table_info(\"{}\")",
            table.replace('"', "\"\"")
        ))
        .map_err(sql_err)?;
    let rows = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(1)?, // name
                r.get::<_, String>(2)?, // declared type
                r.get::<_, i64>(5)?,    // pk
            ))
        })
        .map_err(sql_err)?;
    let mut attrs: Vec<(String, DataType)> = Vec::new();
    for r in rows {
        let (name, decl, pk) = r.map_err(sql_err)?;
        // La lettura pagina con `WHERE t.rowid > ?`, dove `rowid` e' l'alias
        // implicito della chiave di riga. Se la tabella dichiara una colonna
        // con quel nome, SQLite risolve l'alias sulla colonna *utente* e la
        // paginazione avviene su valori arbitrari: righe saltate, ripetute, o
        // un cursore che non avanza mai.
        //
        // SQLite ha tre alias per la chiave di riga e sono tutti oscurabili;
        // non esiste una forma non oscurabile a cui ripiegare. L'unica difesa
        // e' rifiutare la tabella, ed e' coerente con la semantica fail-closed
        // del componente: meglio non leggere che leggere righe sbagliate senza
        // dirlo.
        //
        // Il confronto e' senza distinzione di maiuscole perche' SQLite
        // risolve i nomi di colonna in modo case-insensitive.
        if matches!(
            name.to_ascii_lowercase().as_str(),
            "rowid" | "_rowid_" | "oid"
        ) {
            return Err(err(format!(
                "la tabella \"{table}\" dichiara la colonna \"{name}\", che oscura l'alias della \
                 chiave di riga usato per la paginazione; il GeoPackage non e' leggibile in modo \
                 deterministico"
            )));
        }
        if name == geom_col || pk > 0 {
            continue;
        }
        attrs.push((name, sqlite_declared_to_arrow(&decl)));
    }
    let crs_id = crs.id.as_deref().ok_or_else(|| {
        PlenoraIoError::Crs(
            "GeoPackage: CRS risolto senza identificatore; vietato assumere OGC:CRS84".to_owned(),
        )
    })?;
    let mut fields = vec![geometry_field(geom_col, crs_id)];
    for (n, dt) in &attrs {
        fields.push(Field::new(n, dt.clone(), true));
    }
    Ok((Arc::new(Schema::new(fields)), attrs))
}

/// Header `GeoPackage` estratto dal blob geometria (`StandardGeoPackageBinary`).
///
/// Il chiamante confronta `srs_id` con quello del layer (finding #7 della
/// review 2026-08-15): un blob con SRS discordante non deve essere esposto
/// come se avesse il CRS della tabella, perche' le coordinate sarebbero
/// interpretate in un sistema sbagliato senza alcuna diagnostica.
struct GpkgBlobHeader<'a> {
    srs_id: i32,
    payload: &'a [u8],
}

fn parse_gpkg_header(blob: &[u8]) -> Result<GpkgBlobHeader<'_>> {
    if blob.len() < 8 || &blob[0..2] != b"GP" {
        return Err(err("blob geometria GeoPackage non valido (magic)"));
    }
    let envelope = match (blob[3] >> 1) & 0x07 {
        0 => 0,
        1 => 32,
        2 | 3 => 48,
        4 => 64,
        _ => return Err(err("envelope GeoPackage non valido")),
    };
    let start = 8 + envelope;
    if blob.len() < start {
        return Err(err("blob geometria GeoPackage troncato"));
    }
    // Byte 3 bit 0 = byte order (0 big endian, 1 little endian) per l'header;
    // per il payload la endianess e' nel primo byte del WKB. Qui interpreta
    // solo il campo srs_id dell'header GeoPackage secondo la stessa flag.
    let little_endian = (blob[3] & 0x01) == 0x01;
    let bytes: [u8; 4] = [blob[4], blob[5], blob[6], blob[7]];
    let srs_id = if little_endian {
        i32::from_le_bytes(bytes)
    } else {
        i32::from_be_bytes(bytes)
    };
    Ok(GpkgBlobHeader {
        srs_id,
        payload: &blob[start..],
    })
}

/// API storica: preserva l'estrazione del solo payload per i chiamanti che
/// non hanno accesso al CRS del layer (per esempio i test o gli helper di
/// utilita' su blob singoli). I reader di produzione usano
/// [`parse_gpkg_header`] e confrontano `srs_id`.
fn strip_gpkg_header(blob: &[u8]) -> Result<&[u8]> {
    parse_gpkg_header(blob).map(|header| header.payload)
}

/// Header `StandardGeoPackageBinary` (8 byte, senza envelope) per un payload
/// WKB. Il byte di flag comprende endianess (bit 0) e, dalla review
/// 2026-08-15 finding #12, il bit "empty geometry" (bit 4). Prima del fix
/// la bandiera restava sempre 0x01 anche per geometrie vuote.
const fn gpkg_header(srs_id: i32, is_empty: bool) -> [u8; 8] {
    let s = srs_id.to_le_bytes();
    // Bit 0 = 1 (little-endian). Bit 4 = 1 se il payload rappresenta una
    // geometria vuota, come richiesto dallo standard GeoPackage 1.3
    // (Clause 2.1.3.1.1 GeoPackageBinaryHeader).
    let flags: u8 = if is_empty { 0x11 } else { 0x01 };
    [b'G', b'P', 0, flags, s[0], s[1], s[2], s[3]]
}

/// Classifica un payload WKB come vuoto o non vuoto per l'unica
/// finalita' del bit "empty" nell'header `GeoPackage`. Supporta:
///
/// - ISO WKB XY / XYZ / XYM / XYZM (tipi 1-7 e le famiglie +1000/+2000/+3000);
/// - EWKB (`PostGIS`) con flag Z/M/SRID e prefisso SRID opzionale;
/// - `POINT EMPTY` codificato come coordinate tutte `NaN`;
/// - collezioni (Multi*, `GeometryCollection`) e famiglie `LineString`/
///   `Polygon` con conteggio zero.
///
/// Fail-closed: un payload troncato, con byte order non valido o con
/// flavor ISO sconosciuto restituisce `Err` invece di indovinare — il
/// caller (finding #12 follow-up review 2026-08-15) rifiuta la feature
/// invece di scrivere un header incoerente. I tipi geometrici che
/// stanno fuori dall'insieme dei sette canonici (per esempio le
/// varianti curve/superfici estese) sono considerati non-empty come
/// safe-default: emettere il flag "non vuoto" su una geometria
/// realmente vuota di quel tipo e' meno grave di segnalare vuoto un
/// payload che ha contenuto reale.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WkbShape {
    Empty,
    NonEmpty,
}

impl WkbShape {
    const fn is_empty(self) -> bool {
        matches!(self, Self::Empty)
    }
}

// `read_u32` / `read_f64` sono le due primitive canoniche di lettura WKB
// nella stessa endianess: rinominarle per soddisfare `similar_names`
// renderebbe meno chiaro il pattern.
#[allow(clippy::similar_names)]
fn wkb_shape(payload: &[u8]) -> Result<WkbShape> {
    if payload.len() < 5 {
        return Err(err("WKB troppo corto per l'header base"));
    }
    let little_endian = match payload[0] {
        0x00 => false,
        0x01 => true,
        other => {
            return Err(err(format!("byte order WKB non valido: 0x{other:02x}")));
        }
    };
    let read_u32 = |slice: &[u8]| -> u32 {
        let bytes = [slice[0], slice[1], slice[2], slice[3]];
        if little_endian {
            u32::from_le_bytes(bytes)
        } else {
            u32::from_be_bytes(bytes)
        }
    };
    let read_f64 = |slice: &[u8]| -> f64 {
        let mut bytes = [0_u8; 8];
        bytes.copy_from_slice(&slice[..8]);
        if little_endian {
            f64::from_le_bytes(bytes)
        } else {
            f64::from_be_bytes(bytes)
        }
    };
    let raw_type = read_u32(&payload[1..5]);
    let mut cursor: usize = 5;

    // EWKB (PostGIS): riconosciuto dai bit di flag alti del type. Un
    // qualsiasi bit alto attivo classifica il payload come EWKB; ISO WKB
    // resta l'ipotesi di default.
    let ewkb_flag_z = raw_type & 0x8000_0000 != 0;
    let ewkb_flag_m = raw_type & 0x4000_0000 != 0;
    let ewkb_flag_srid = raw_type & 0x2000_0000 != 0;
    let is_ewkb = ewkb_flag_z || ewkb_flag_m || ewkb_flag_srid;

    let (base_type, dims) = if is_ewkb {
        let base = raw_type & 0x0000_00FF;
        let dims: usize = 2 + usize::from(ewkb_flag_z) + usize::from(ewkb_flag_m);
        if ewkb_flag_srid {
            if payload.len() < cursor + 4 {
                return Err(err("WKB EWKB troncato: manca il SRID"));
            }
            cursor += 4;
        }
        (base, dims)
    } else {
        let base = raw_type % 1000;
        let flavor = raw_type / 1000;
        let dims = match flavor {
            0 => 2,     // XY
            1 | 2 => 3, // XYZ o XYM
            3 => 4,     // XYZM
            other => {
                return Err(err(format!("flavor WKB ISO non riconosciuto: {other}")));
            }
        };
        (base, dims)
    };

    let coord_bytes = dims
        .checked_mul(8)
        .ok_or_else(|| err("overflow dimensioni coordinate WKB"))?;

    match base_type {
        1 => {
            // Point: se tutte le coordinate sono NaN, la geometria e' EMPTY
            // (convenzione standard). Un payload troncato prima delle
            // coordinate e' un errore, non un default.
            if payload.len() < cursor + coord_bytes {
                return Err(err("WKB Point troncato: mancano le coordinate"));
            }
            let mut all_nan = true;
            for i in 0..dims {
                let offset = cursor + i * 8;
                if !read_f64(&payload[offset..offset + 8]).is_nan() {
                    all_nan = false;
                    break;
                }
            }
            Ok(if all_nan {
                WkbShape::Empty
            } else {
                WkbShape::NonEmpty
            })
        }
        2..=7 | 15..=17 => {
            // LineString/Polygon/Multi*/GeometryCollection/PolyhedralSurface/
            // Tin/Triangle: subito dopo l'header c'e' un uint32 con il
            // conteggio degli elementi. Zero → EMPTY.
            if payload.len() < cursor + 4 {
                return Err(err("WKB troncato: manca il conteggio"));
            }
            let count = read_u32(&payload[cursor..cursor + 4]);
            Ok(if count == 0 {
                WkbShape::Empty
            } else {
                WkbShape::NonEmpty
            })
        }
        _ => {
            // Tipo estese non gestite (curve, superfici circolari, ecc.):
            // safe default non-empty, per non segnalare vuoto un contenuto
            // che non sappiamo classificare.
            Ok(WkbShape::NonEmpty)
        }
    }
}

fn init_gpkg(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "PRAGMA application_id = 1196444487;
         PRAGMA user_version = 10300;
         CREATE TABLE gpkg_spatial_ref_sys (srs_name TEXT NOT NULL, srs_id INTEGER PRIMARY KEY,
            organization TEXT NOT NULL, organization_coordsys_id INTEGER NOT NULL,
            definition TEXT NOT NULL, description TEXT);
         CREATE TABLE gpkg_contents (table_name TEXT PRIMARY KEY, data_type TEXT NOT NULL,
            identifier TEXT, description TEXT DEFAULT '',
            last_change DATETIME NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
            min_x DOUBLE, min_y DOUBLE, max_x DOUBLE, max_y DOUBLE, srs_id INTEGER);
         CREATE TABLE gpkg_geometry_columns (table_name TEXT NOT NULL, column_name TEXT NOT NULL,
            geometry_type_name TEXT NOT NULL, srs_id INTEGER NOT NULL, z TINYINT NOT NULL,
            m TINYINT NOT NULL, PRIMARY KEY (table_name, column_name));
         INSERT INTO gpkg_spatial_ref_sys VALUES
            ('WGS 84 geodetic', 4326, 'EPSG', 4326, 'GEOGCS[\"WGS 84\"]', 'longitude/latitude'),
            ('undefined cartesian', -1, 'NONE', -1, 'undefined', 'undefined'),
            ('undefined geographic', 0, 'NONE', 0, 'undefined', 'undefined');",
    )
    .map_err(sql_err)?;
    Ok(())
}

const fn sqlite_type(dt: &DataType) -> &'static str {
    match dt {
        DataType::Int8
        | DataType::Int16
        | DataType::Int32
        | DataType::Int64
        | DataType::UInt8
        | DataType::UInt16
        | DataType::UInt32
        | DataType::UInt64
        | DataType::Boolean => "INTEGER",
        DataType::Float16 | DataType::Float32 | DataType::Float64 => "REAL",
        DataType::Binary
        | DataType::LargeBinary
        | DataType::BinaryView
        | DataType::FixedSizeBinary(_) => "BLOB",
        _ => "TEXT",
    }
}

/// CRS del layer in scrittura: dal contratto (id + WKT) se presente, altrimenti
/// dall'id nei metadati del campo geometria.
fn layer_crs(
    layer: &plenora_io_core::WriteLayer,
    geom_idx: usize,
) -> (Option<String>, Option<String>) {
    if let Some(g) = &layer.contract.geometry {
        return (
            g.crs.id().map(str::to_owned),
            g.crs.definition().map(str::to_owned),
        );
    }
    let id = layer
        .contract
        .schema
        .field(geom_idx)
        .metadata()
        .get(plenora_io_model::geometry::GEO_CRS_KEY)
        .cloned();
    (id, None)
}

/// Risolve il `srs_id` `GeoPackage` per il CRS dato, registrandolo in
/// `gpkg_spatial_ref_sys` se non è il WGS84 built-in. Senza WKT reale usa
/// `definition='undefined'`: GDAL risolve comunque il CRS da organization+code.
fn register_srs(conn: &Connection, id: Option<&str>, def: Option<&str>) -> Result<i32> {
    let Some(id) = id else {
        return Err(PlenoraIoError::Crs(
            "GeoPackage richiede un CRS esplicito; nessun default implicito".to_owned(),
        ));
    };
    if id.eq_ignore_ascii_case("EPSG:4326") {
        return Ok(4326);
    }
    if let Some((auth, code)) = id.split_once(':') {
        if let Ok(code_i) = code.parse::<i32>() {
            conn.execute(
                "INSERT OR IGNORE INTO gpkg_spatial_ref_sys \
                 (srs_name, srs_id, organization, organization_coordsys_id, definition, description) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                rusqlite::params![
                    id,
                    code_i,
                    auth.to_uppercase(),
                    code_i,
                    def.unwrap_or("undefined"),
                    "importato da plenora-io"
                ],
            )
            .map_err(sql_err)?;
            return Ok(code_i);
        }
    }
    let raw = RawCrs::new(def.unwrap_or(id).to_owned(), Some(id.to_owned()));
    Err(PlenoraIoError::crs_unresolved("gpkg", &raw))
}

fn create_feature_table(
    conn: &Connection,
    name: &str,
    schema: &Schema,
    srs_id: i32,
    geometry_contract: Option<&GeometryColumnContract>,
) -> Result<ActiveLayer> {
    let geom_idx = geometry_index(schema).ok_or_else(|| err("layer senza geometria"))?;
    let geom_name = schema.field(geom_idx).name().clone();

    let mut cols_ddl = vec!["fid INTEGER PRIMARY KEY AUTOINCREMENT".to_owned()];
    cols_ddl.push(format!("\"{}\" BLOB", geom_name.replace('"', "\"\"")));
    for (i, f) in schema.fields().iter().enumerate() {
        if i == geom_idx {
            continue;
        }
        cols_ddl.push(format!(
            "\"{}\" {}",
            f.name().replace('"', "\"\""),
            sqlite_type(f.data_type())
        ));
    }
    conn.execute(
        &format!(
            "CREATE TABLE \"{}\" ({})",
            name.replace('"', "\"\""),
            cols_ddl.join(", ")
        ),
        [],
    )
    .map_err(sql_err)?;
    // Finding #12 review 2026-08-15: `gpkg_contents.last_change` e' nullable
    // ma la specifica GeoPackage lo richiede come `%Y-%m-%dT%H:%M:%fZ`
    // (ISO 8601 UTC). Compilarlo alla CREATE evita che i validator conformi
    // rifiutino il file per un metadato omesso.
    conn.execute(
        "INSERT INTO gpkg_contents (table_name, data_type, identifier, last_change, srs_id) \
         VALUES (?1, 'features', ?1, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), ?2)",
        rusqlite::params![name, srs_id],
    )
    .map_err(sql_err)?;
    let geometry_type = geometry_contract
        .and_then(|contract| contract.geometry_types.first())
        .map_or_else(
            || "GEOMETRY".to_owned(),
            |geometry_type| format!("{geometry_type:?}").to_ascii_uppercase(),
        );
    let dimensions = geometry_contract.map_or(CoordinateDimensions::Unknown, |contract| {
        contract.dimensions
    });
    let (z, m) = match dimensions {
        CoordinateDimensions::Xy => (0, 0),
        CoordinateDimensions::Xyz => (1, 0),
        CoordinateDimensions::Xym => (0, 1),
        CoordinateDimensions::Xyzm => (1, 1),
        CoordinateDimensions::Unknown => (2, 2),
    };
    conn.execute(
        "INSERT INTO gpkg_geometry_columns VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        rusqlite::params![name, geom_name, geometry_type, srs_id, z, m],
    )
    .map_err(sql_err)?;

    let mut col_list = vec![format!("\"{}\"", geom_name.replace('"', "\"\""))];
    for (i, f) in schema.fields().iter().enumerate() {
        if i != geom_idx {
            col_list.push(format!("\"{}\"", f.name().replace('"', "\"\"")));
        }
    }
    let placeholders: Vec<String> = (1..=col_list.len()).map(|i| format!("?{i}")).collect();
    let insert_sql = format!(
        "INSERT INTO \"{}\" ({}) VALUES ({})",
        name.replace('"', "\"\""),
        col_list.join(", "),
        placeholders.join(", ")
    );
    Ok(ActiveLayer {
        geom_idx,
        insert_sql,
        srs_id,
    })
}

/// Entry point non stabile per libFuzzer: esercita l'header binario
/// `GeoPackageBinaryHeader` (magic, flag envelope, dimensionamento) e la
/// decodifica del WKB che lo segue, senza I/O su filesystem. Il payload
/// estratto e' esattamente quello che il reader mette in colonna geometria,
/// quindi un solo input copre entrambi i confini.
///
/// Ritorna l'offset del payload dentro il blob: e' il dato su cui il target
/// verifica che il salto dell'envelope resti quello dichiarato dai flag.
#[doc(hidden)]
pub fn __fuzz_gpkg_geometry(bytes: &[u8]) -> Result<usize> {
    let payload = strip_gpkg_header(bytes)?;
    decode_wkb(payload, &WkbLimits::default())?;
    Ok(bytes.len() - payload.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Opzioni di lettura sul modello unificato.
    ///
    /// Da S4.d il percorso di lettura vive interamente li': la memoria dei
    /// batch e' una `InternalMemoryLease`, che esiste solo dentro un
    /// `PipelineContext`. `opzioni_lettura()` costruisce ancora il ramo
    /// legacy — sparira' in S4.e — e con quello `open` fallisce chiuso.
    fn opzioni_lettura() -> ReadOptions {
        match plenora_io_model::budget::PipelineBudget::builder().build() {
            Ok(bundle) => ReadOptions::from_read_parts(bundle.into_read_parts()),
            Err(error) => unreachable!("bundle di test non costruibile: {error:?}"),
        }
    }

    use plenora_io_core::request::{BatchTarget, ProjectionMode, ReadScope};
    use plenora_io_core::WriteLayer;
    use plenora_io_model::wkb::{
        encode_wkb, to_wkb, WkbCoordinate, WkbFlavor, WkbGeometry, WkbValue,
    };
    use plenora_io_model::CancellationToken;

    /// Legge tutti i batch e restituisce il report di perdita accumulato.
    fn read_all_and_loss(dataset: &dyn OpenDatasetHandle) -> LossReport {
        let mut reader = dataset
            .open_layer_reader(&ReadRequest {
                layer: LayerId(0),
                projected_fields: None,
                projection_mode: ProjectionMode::BestEffort,
                pruning_predicate: None,
                spatial_pruning_hint: None,
                scope: ReadScope::default(),
                batch_target: BatchTarget::default(),
                cancellation: CancellationToken::default(),
            })
            .unwrap();
        while reader.next_batch().unwrap().is_some() {}
        reader.loss_report()
    }

    /// Scrive un gpkg con una colonna dichiarata INTEGER, poi vi inserisce
    /// valori REAL con SQL diretto. `SQLite` ha affinita', non vincoli: un REAL
    /// non convertibile senza perdita resta REAL anche in colonna INTEGER, ed
    /// e' esattamente il caso che un file legittimo puo' produrre.
    #[test]
    fn real_values_in_an_integer_column_are_reported_as_loss_not_silently_coerced() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("coercion.gpkg");
        let schema: SchemaRef = Arc::new(Schema::new(vec![
            geometry_field("geom", "EPSG:4326"),
            Field::new("id", DataType::Int64, false),
        ]));
        let geometry =
            to_wkb(&geo_types::Geometry::Point(geo_types::Point::new(0.0, 0.0))).unwrap();
        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![
                Arc::new(BinaryArray::from(vec![Some(geometry.as_slice())])),
                Arc::new(Int64Array::from(vec![1])),
            ],
        )
        .unwrap();
        let plan = WritePlan {
            layers: vec![WriteLayer {
                name: "features".to_owned(),
                contract: DataContract {
                    schema,
                    geometry: None,
                },
            }],
        };
        let driver = GpkgDriver;
        let mut writer = driver
            .create(Sink::Path(path.clone()), &plan, &WriteOptions::default())
            .unwrap();
        writer.write(&batch).unwrap();
        writer.finish().unwrap();

        let conn = Connection::open(&path).unwrap();
        // 7.0 viene convertito a INTEGER dall'affinita' (conversione lossless):
        // nessuna perdita. 1.5 e 1e300 restano REAL.
        conn.execute_batch(
            "UPDATE features SET id = 1.5 WHERE fid = 1;
             INSERT INTO features (fid, geom, id) SELECT 2, geom, 1e300 FROM features WHERE fid = 1;
             INSERT INTO features (fid, geom, id) SELECT 3, geom, 7.0 FROM features WHERE fid = 1;",
        )
        .unwrap();
        // Precondizione del test: l'affinita' si e' comportata come previsto.
        let kinds: Vec<String> = conn
            .prepare("SELECT typeof(id) FROM features ORDER BY fid")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .map(std::result::Result::unwrap)
            .collect();
        assert_eq!(kinds, vec!["real", "real", "integer"]);
        drop(conn);

        let dataset = driver.open(Source::Path(path), opzioni_lettura()).unwrap();
        let loss = read_all_and_loss(dataset.as_ref());

        assert_eq!(loss.counts.get(INTEGER_COLUMN_REAL_TRUNCATED), Some(&1));
        assert_eq!(loss.counts.get(INTEGER_COLUMN_REAL_SATURATED), Some(&1));
        // Il 7.0 convertito dall'affinita' non e' una perdita.
        assert_eq!(loss.counts.values().sum::<u64>(), 2);
        // Un esempio per coppia (campo, categoria), mai uno per riga.
        assert_eq!(loss.examples().len(), 2);
        assert!(loss
            .examples()
            .iter()
            .all(|example| example.context.starts_with("field=id:")));
    }

    /// Le due guardie difensive non sono raggiungibili da un file: `SQLite`
    /// memorizza NaN come NULL e l'affinita' REAL converte gli interi in
    /// virgola mobile prima che il driver li veda. Restano perche' rendono
    /// esplicita un'invariante che oggi dipende dalle regole di affinita', e
    /// si verificano al livello in cui sono scritte.
    #[test]
    fn non_finite_and_wide_integers_are_declared_never_silently_substituted() {
        let mut integer_column = ColBuilder::new(&DataType::Int64);
        assert_eq!(
            integer_column.append(ValueRef::Real(f64::NAN)),
            Some(Coercion::NonFiniteDiscarded)
        );
        assert_eq!(
            integer_column.append(ValueRef::Real(f64::INFINITY)),
            Some(Coercion::NonFiniteDiscarded)
        );
        // Un NaN non diventa mai lo zero plausibile e falso di `as i64`.
        let values = integer_column.finish();
        assert_eq!(values.null_count(), 2);

        let mut real_column = ColBuilder::new(&DataType::Float64);
        assert_eq!(
            real_column.append(ValueRef::Integer(9_007_199_254_740_993)),
            Some(Coercion::IntegerPrecisionUnverifiable)
        );
        assert_eq!(real_column.append(ValueRef::Integer(7)), None);
    }

    /// I valori gia' rappresentabili non producono perdita: il gate non deve
    /// diventare rumoroso su file corretti.
    #[test]
    fn representable_values_produce_no_loss() {
        let mut integer_column = ColBuilder::new(&DataType::Int64);
        assert_eq!(integer_column.append(ValueRef::Integer(42)), None);
        // REAL senza parte frazionaria e dentro l'intervallo: esatto.
        assert_eq!(integer_column.append(ValueRef::Real(7.0)), None);
        assert_eq!(integer_column.append(ValueRef::Real(-7.0)), None);
        // Il limite negativo e' rappresentabile, quello positivo no.
        assert_eq!(
            integer_column.append(ValueRef::Real(-SATURATION_BOUND)),
            None
        );
        assert_eq!(
            integer_column.append(ValueRef::Real(SATURATION_BOUND)),
            Some(Coercion::RealSaturated)
        );
    }

    #[test]
    fn fuzz_entrypoint_reports_the_envelope_offset_declared_by_the_flags() {
        let payload = to_wkb(&geo_types::Geometry::Point(geo_types::Point::new(1.0, 2.0))).unwrap();

        let mut senza_envelope = gpkg_header(4326, false).to_vec();
        senza_envelope.extend_from_slice(&payload);
        assert_eq!(__fuzz_gpkg_geometry(&senza_envelope).unwrap(), 8);

        // Envelope XY dichiarato nei flag: 32 byte fra header e payload.
        let mut con_envelope = gpkg_header(4326, false).to_vec();
        con_envelope[3] |= 0x02;
        con_envelope.extend_from_slice(&[0_u8; 32]);
        con_envelope.extend_from_slice(&payload);
        assert_eq!(__fuzz_gpkg_geometry(&con_envelope).unwrap(), 40);

        // Magic assente e blob troncato restano rifiuti, non panic.
        assert!(__fuzz_gpkg_geometry(b"XX\x00\x01\x00\x00\x00\x00").is_err());
        assert!(__fuzz_gpkg_geometry(&senza_envelope[..7]).is_err());
    }

    // Helper solo per test: header WKB `little-endian` per un tipo dato.
    fn wkb_le_header(geometry_type: u32) -> Vec<u8> {
        let mut buffer = vec![0x01_u8];
        buffer.extend_from_slice(&geometry_type.to_le_bytes());
        buffer
    }

    #[test]
    fn wkb_shape_riconosce_point_empty_come_nan_nan() {
        // Finding #12 follow-up review 2026-08-15: POINT EMPTY viene
        // codificato con coordinate tutte NaN. Il flag "empty" del header
        // GeoPackage deve rispecchiare questa condizione, altrimenti i
        // validator conformi rifiutano il file.
        let mut payload = wkb_le_header(1); // ISO WKB Point XY
        payload.extend_from_slice(&f64::NAN.to_le_bytes());
        payload.extend_from_slice(&f64::NAN.to_le_bytes());
        assert_eq!(wkb_shape(&payload).unwrap(), WkbShape::Empty);

        // Un Point con coordinate finite deve restare non-empty.
        let mut concreto = wkb_le_header(1);
        concreto.extend_from_slice(&1.5_f64.to_le_bytes());
        concreto.extend_from_slice(&2.5_f64.to_le_bytes());
        assert_eq!(wkb_shape(&concreto).unwrap(), WkbShape::NonEmpty);
    }

    #[test]
    fn wkb_shape_riconosce_point_empty_anche_in_xyz_e_xyzm() {
        // POINT Z EMPTY: type = 1001, 3 doubles NaN.
        let mut xyz = wkb_le_header(1001);
        for _ in 0..3 {
            xyz.extend_from_slice(&f64::NAN.to_le_bytes());
        }
        assert_eq!(wkb_shape(&xyz).unwrap(), WkbShape::Empty);

        // POINT ZM EMPTY: type = 3001, 4 doubles NaN.
        let mut xyzm = wkb_le_header(3001);
        for _ in 0..4 {
            xyzm.extend_from_slice(&f64::NAN.to_le_bytes());
        }
        assert_eq!(wkb_shape(&xyzm).unwrap(), WkbShape::Empty);

        // Point Z con Z finita e X/Y NaN NON e' empty (basta una
        // coordinata finita per contare come non-empty).
        let mut mixed = wkb_le_header(1001);
        mixed.extend_from_slice(&f64::NAN.to_le_bytes());
        mixed.extend_from_slice(&f64::NAN.to_le_bytes());
        mixed.extend_from_slice(&42.0_f64.to_le_bytes());
        assert_eq!(wkb_shape(&mixed).unwrap(), WkbShape::NonEmpty);
    }

    #[test]
    fn wkb_shape_supporta_ewkb_con_srid() {
        // EWKB Point con SRID (flag 0x2000_0000 | tipo 1): dopo il type
        // c'e' l'SRID (4 byte) e poi le coordinate. Un POINT (1,2) SRID=4326
        // deve risultare non-empty.
        let mut payload = vec![0x01_u8];
        let type_flags: u32 = 0x2000_0000 | 1;
        payload.extend_from_slice(&type_flags.to_le_bytes());
        payload.extend_from_slice(&4326_u32.to_le_bytes()); // SRID
        payload.extend_from_slice(&1.0_f64.to_le_bytes());
        payload.extend_from_slice(&2.0_f64.to_le_bytes());
        assert_eq!(wkb_shape(&payload).unwrap(), WkbShape::NonEmpty);

        // Stesso layout con NaN,NaN e' Point EMPTY.
        let mut empty = vec![0x01_u8];
        empty.extend_from_slice(&type_flags.to_le_bytes());
        empty.extend_from_slice(&4326_u32.to_le_bytes());
        empty.extend_from_slice(&f64::NAN.to_le_bytes());
        empty.extend_from_slice(&f64::NAN.to_le_bytes());
        assert_eq!(wkb_shape(&empty).unwrap(), WkbShape::Empty);

        // MULTIPOINT EMPTY EWKB con SRID: type 4 + SRID flag, poi
        // 4 byte SRID e 4 byte count=0.
        let mut multi_empty = vec![0x01_u8];
        let multi_type: u32 = 0x2000_0000 | 4;
        multi_empty.extend_from_slice(&multi_type.to_le_bytes());
        multi_empty.extend_from_slice(&4326_u32.to_le_bytes());
        multi_empty.extend_from_slice(&0_u32.to_le_bytes());
        assert_eq!(wkb_shape(&multi_empty).unwrap(), WkbShape::Empty);
    }

    #[test]
    fn wkb_shape_fallisce_chiuso_sui_payload_ambigui() {
        // Header troncato: 5 byte minimi non presenti.
        assert!(wkb_shape(&[0x01, 0x01]).is_err());
        // Byte-order invalido.
        assert!(wkb_shape(&[0x02, 0x00, 0x00, 0x00, 0x01]).is_err());
        // Point ISO con coordinate mancanti (solo header, niente doubles).
        assert!(wkb_shape(&wkb_le_header(1)).is_err());
        // LineString ISO senza il conteggio.
        assert!(wkb_shape(&wkb_le_header(2)).is_err());
        // Flavor ISO invalido (type = 4123 → flavor 4 sconosciuto).
        assert!(wkb_shape(&wkb_le_header(4123)).is_err());
        // EWKB con SRID ma payload che finisce prima dell'SRID.
        let mut ewkb_troncato = vec![0x01_u8];
        let type_flags: u32 = 0x2000_0000 | 1;
        ewkb_troncato.extend_from_slice(&type_flags.to_le_bytes());
        // niente 4 byte SRID: il payload finisce qui
        assert!(wkb_shape(&ewkb_troncato).is_err());
    }

    #[test]
    fn wkb_shape_rileva_le_collezioni_vuote() {
        // MULTIPOINT EMPTY (ISO): type 4, count 0.
        let mut mp = wkb_le_header(4);
        mp.extend_from_slice(&0_u32.to_le_bytes());
        assert_eq!(wkb_shape(&mp).unwrap(), WkbShape::Empty);

        // MULTIPOINT con un elemento: non-empty.
        let mut mp_full = wkb_le_header(4);
        mp_full.extend_from_slice(&1_u32.to_le_bytes());
        // (il payload del figlio non e' letto da wkb_shape: basta il count>0)
        assert_eq!(wkb_shape(&mp_full).unwrap(), WkbShape::NonEmpty);
    }

    #[test]
    fn last_change_gpkg_contents_e_not_null_con_default_valido() {
        // Finding #12 follow-up: la DDL deve dichiarare NOT NULL e un
        // default valido, non solo compilarlo nell'INSERT del driver. Un
        // consumer che scriva altri record in gpkg_contents senza
        // fornire last_change deve trovare comunque un valore ISO 8601.
        let conn = Connection::open_in_memory().unwrap();
        init_gpkg(&conn).unwrap();
        conn.execute(
            "INSERT INTO gpkg_contents (table_name, data_type, identifier, srs_id) \
             VALUES ('probe', 'features', 'probe', 4326)",
            [],
        )
        .unwrap();
        let last_change: String = conn
            .query_row(
                "SELECT last_change FROM gpkg_contents WHERE table_name = 'probe'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        // Formato dichiarato: `YYYY-MM-DDTHH:MM:SS.sssZ`.
        assert!(
            last_change.ends_with('Z') && last_change.contains('T'),
            "last_change atteso in formato ISO 8601 UTC: {last_change}"
        );
    }

    #[test]
    fn integer_widths_are_exact_and_overflow_never_becomes_sql_null() {
        let values: ArrayRef = Arc::new(Int32Array::from(vec![7]));
        let overflow: ArrayRef = Arc::new(UInt64Array::from(vec![u64::MAX]));

        assert_eq!(
            arrow_cell_to_sql_ref(&values, 0).unwrap(),
            BorrowedSqlValue::Integer(7)
        );
        assert!(arrow_cell_to_sql_ref(&overflow, 0).is_err());
    }

    #[test]
    fn undefined_and_dangling_srs_ids_fail_closed_with_raw_crs() {
        let conn = Connection::open_in_memory().unwrap();
        init_gpkg(&conn).unwrap();

        for srs_id in [0, -1, 999_999] {
            let error = crs_for(&conn, srs_id).unwrap_err();
            assert_eq!(error.code, plenora_io_model::IoErrorCode::CrsUnresolved);
            assert_eq!(error.driver.as_deref(), Some("gpkg"));
        }

        conn.execute("DELETE FROM gpkg_spatial_ref_sys WHERE srs_id = 4326", [])
            .unwrap();
        assert!(matches!(
            crs_for(&conn, 4326),
            Err(error) if error.code == plenora_io_model::IoErrorCode::CrsUnresolved
        ));
    }

    #[test]
    fn writer_does_not_relabel_crs84_as_epsg_4326() {
        let conn = Connection::open_in_memory().unwrap();
        init_gpkg(&conn).unwrap();
        assert!(matches!(
            register_srs(&conn, Some("OGC:CRS84"), None),
            Err(error) if error.code == plenora_io_model::IoErrorCode::CrsUnresolved
        ));
    }

    #[test]
    fn gpkg_epsg_axis_orders_are_explicit() {
        let conn = Connection::open_in_memory().unwrap();
        init_gpkg(&conn).unwrap();
        register_srs(&conn, Some("EPSG:3857"), None).unwrap();

        let geographic = crs_for(&conn, 4326).unwrap();
        let projected = crs_for(&conn, 3857).unwrap();
        assert_eq!(
            geographic.axis_order,
            plenora_io_model::crs::AxisOrder::LatitudeLongitude
        );
        assert_eq!(
            projected.axis_order,
            plenora_io_model::crs::AxisOrder::EastingNorthing
        );
    }

    fn ids_with_spatial_hint(
        dataset: &dyn OpenDatasetHandle,
        spatial_pruning_hint: Bbox,
    ) -> Vec<i64> {
        let mut reader = dataset
            .open_layer_reader(&ReadRequest {
                layer: LayerId(0),
                projected_fields: None,
                projection_mode: ProjectionMode::BestEffort,
                pruning_predicate: None,
                spatial_pruning_hint: Some(spatial_pruning_hint),
                scope: ReadScope::default(),
                batch_target: BatchTarget::default(),
                cancellation: CancellationToken::default(),
            })
            .unwrap();
        let mut ids = Vec::new();
        while let Some(batch) = reader.next_batch().unwrap() {
            ids.extend_from_slice(
                batch
                    .column_by_name("id")
                    .unwrap()
                    .as_any()
                    .downcast_ref::<Int64Array>()
                    .unwrap()
                    .values(),
            );
        }
        ids
    }

    // Il test copre in un solo scenario scrittura, registrazione dell'RTree e
    // le tre letture (non registrato, registrato, hint invalido): spezzarlo
    // duplicherebbe la fixture e ne perderebbe la sequenza.
    #[allow(clippy::too_many_lines)]
    #[test]
    fn spatial_pruning_uses_only_registered_rtree_and_never_filters_exactly() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rtree.gpkg");
        let geometries = [
            to_wkb(&geo_types::Geometry::Point(geo_types::Point::new(0.0, 0.0))).unwrap(),
            to_wkb(&geo_types::Geometry::LineString(
                geo_types::LineString::from(vec![(0.0, 0.0), (10.0, 10.0)]),
            ))
            .unwrap(),
            to_wkb(&geo_types::Geometry::Point(geo_types::Point::new(0.5, 9.5))).unwrap(),
            to_wkb(&geo_types::Geometry::Point(geo_types::Point::new(
                100.0, 100.0,
            )))
            .unwrap(),
        ];
        let schema: SchemaRef = Arc::new(Schema::new(vec![
            geometry_field("geom", "EPSG:4326"),
            Field::new("id", DataType::Int64, false),
        ]));
        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![
                Arc::new(BinaryArray::from(
                    geometries
                        .iter()
                        .map(|geometry| Some(geometry.as_slice()))
                        .collect::<Vec<_>>(),
                )),
                Arc::new(Int64Array::from(vec![1, 2, 3, 4])),
            ],
        )
        .unwrap();
        let plan = WritePlan {
            layers: vec![WriteLayer {
                name: "features".to_owned(),
                contract: DataContract {
                    schema,
                    geometry: None,
                },
            }],
        };
        let driver = GpkgDriver;
        let mut writer = driver
            .create(Sink::Path(path.clone()), &plan, &WriteOptions::default())
            .unwrap();
        writer.write(&batch).unwrap();
        writer.finish().unwrap();

        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(
            "CREATE TABLE gpkg_extensions (
                 table_name TEXT,
                 column_name TEXT,
                 extension_name TEXT NOT NULL,
                 definition TEXT NOT NULL,
                 scope TEXT NOT NULL
             );
             CREATE VIRTUAL TABLE rtree_features_geom
                 USING rtree(id, minx, maxx, miny, maxy);
             INSERT INTO rtree_features_geom VALUES (1, 0, 0, 0, 0);
             INSERT INTO rtree_features_geom VALUES (2, 0, 10, 0, 10);
             INSERT INTO rtree_features_geom VALUES (3, 0.5, 0.5, 9.5, 9.5);
             INSERT INTO rtree_features_geom VALUES (4, 100, 100, 100, 100);",
        )
        .unwrap();
        drop(conn);

        let hint = Bbox {
            minx: 0.0,
            miny: 9.0,
            maxx: 1.0,
            maxy: 10.0,
        };
        let unregistered = driver
            .open(Source::Path(path.clone()), opzioni_lettura())
            .unwrap();
        assert_eq!(
            unregistered.layers()[0]
                .contract
                .geometry
                .as_ref()
                .unwrap()
                .native_metadata["gpkg.rtree_index"],
            "false"
        );
        assert_eq!(
            ids_with_spatial_hint(unregistered.as_ref(), hint),
            vec![1, 2, 3, 4],
            "una tabella RTree non registrata deve essere ignorata"
        );
        drop(unregistered);

        let conn = Connection::open(&path).unwrap();
        conn.execute(
            "INSERT INTO gpkg_extensions
             VALUES (?1, ?2, 'gpkg_rtree_index',
                     'http://www.geopackage.org/spec/#extension_rtree', 'write-only')",
            rusqlite::params!["features", "geom"],
        )
        .unwrap();
        drop(conn);

        let indexed = driver.open(Source::Path(path), opzioni_lettura()).unwrap();
        assert_eq!(
            indexed.layers()[0]
                .contract
                .geometry
                .as_ref()
                .unwrap()
                .native_metadata["gpkg.rtree_index"],
            "true"
        );
        assert_eq!(
            ids_with_spatial_hint(indexed.as_ref(), hint),
            vec![2, 3],
            "il vero positivo deve restare e il falso positivo bbox è ammesso"
        );
        assert_eq!(
            ids_with_spatial_hint(
                indexed.as_ref(),
                Bbox {
                    minx: 1.0,
                    miny: 1.0,
                    maxx: -1.0,
                    maxy: -1.0,
                },
            ),
            vec![1, 2, 3, 4],
            "un hint invalido deve essere ignorato"
        );
    }

    /// Una colonna che oscura l'alias della chiave di riga rende la
    /// paginazione non deterministica, e il driver deve rifiutare il file.
    ///
    /// Non serve un file corrotto: e' un `GeoPackage` sintatticamente valido,
    /// scritto qui dal driver stesso e poi esteso con `ALTER TABLE`. Prima di
    /// questo controllo il driver paginava sulla colonna utente senza dirlo,
    /// con righe saltate o ripetute a seconda dei valori.
    #[test]
    fn una_colonna_che_oscura_il_rowid_viene_rifiutata() {
        for nome in ["rowid", "_rowid_", "OID"] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("shadow.gpkg");
            let schema: SchemaRef = Arc::new(Schema::new(vec![
                geometry_field("geom", "EPSG:4326"),
                Field::new("id", DataType::Int64, false),
            ]));
            let geometry =
                to_wkb(&geo_types::Geometry::Point(geo_types::Point::new(0.0, 0.0))).unwrap();
            let batch = RecordBatch::try_new(
                schema.clone(),
                vec![
                    Arc::new(BinaryArray::from(vec![Some(geometry.as_slice())])),
                    Arc::new(Int64Array::from(vec![1])),
                ],
            )
            .unwrap();
            let plan = WritePlan {
                layers: vec![WriteLayer {
                    name: "features".to_owned(),
                    contract: DataContract {
                        schema,
                        geometry: None,
                    },
                }],
            };
            let driver = GpkgDriver;
            let mut writer = driver
                .create(Sink::Path(path.clone()), &plan, &WriteOptions::default())
                .unwrap();
            writer.write(&batch).unwrap();
            writer.finish().unwrap();

            // Il file resta valido: aggiungiamo solo una colonna con un nome
            // che SQLite risolve come alias della chiave di riga.
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(&format!(
                "ALTER TABLE features ADD COLUMN \"{nome}\" INTEGER;"
            ))
            .unwrap();
            drop(conn);

            // `expect_err` richiederebbe `Debug` sul trait object restituito
            // in caso di successo, che non lo implementa.
            match driver.open(Source::Path(path), opzioni_lettura()) {
                Ok(_) => panic!("colonna {nome}: il file doveva essere rifiutato"),
                Err(errore) => assert!(
                    errore.to_string().contains("oscura l'alias"),
                    "colonna {nome}: messaggio inatteso {errore}"
                ),
            }
        }
    }

    /// Regressione sul file che il fuzzing ha usato per portare il reader
    /// oltre 4 GiB residenti: 32 KiB di `GeoPackage` in cui 228 byte sono
    /// stati scritti nello spazio libero delle pagine. `SQLite` legge la
    /// chiave del b-tree, ma il `rowid` che restituisce viene dal record, e i
    /// due divergono: la pagina successiva ripeteva la stessa riga senza mai
    /// avanzare il cursore.
    ///
    /// Il file e' versionato in `fuzz/seeds/`, quindi la campagna lo ricarica
    /// a ogni run e questa regressione resta coperta su entrambi i fronti.
    #[test]
    fn un_gpkg_che_non_fa_avanzare_la_paginazione_termina_con_errore() {
        let seme = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fuzz/seeds/gpkg_reader/paginazione-bloccata.gpkg");
        // Copiato in una directory temporanea: aprendolo `SQLite` puo'
        // affiancargli journal o WAL, e l'albero versionato resta pulito.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("paginazione-bloccata.gpkg");
        std::fs::copy(&seme, &path).unwrap();

        let driver = GpkgDriver;
        let Ok(dataset) = driver.open(Source::Path(path), opzioni_lettura()) else {
            // Il file e' corrotto: se una verifica a monte lo rifiuta prima
            // ancora di leggerlo, il non-avanzamento e' comunque irraggiungibile.
            return;
        };
        let mut reader = dataset
            .open_layer_reader(&ReadRequest {
                layer: LayerId(0),
                projected_fields: None,
                projection_mode: ProjectionMode::BestEffort,
                pruning_predicate: None,
                spatial_pruning_hint: None,
                scope: ReadScope::default(),
                batch_target: BatchTarget::default(),
                cancellation: CancellationToken::default(),
            })
            .unwrap();

        // Il tetto non e' una soglia di merito: prima della correzione questo
        // ciclo non terminava. Un limite basso trasforma la non terminazione
        // in un fallimento immediato invece che in un test appeso.
        for _ in 0..1_000 {
            match reader.next_batch() {
                Ok(None) => return,
                Ok(Some(_)) => {}
                Err(errore) => {
                    assert!(
                        errore.to_string().contains("paginazione bloccata"),
                        "messaggio inatteso: {errore}"
                    );
                    return;
                }
            }
        }
        panic!("la lettura non e' terminata entro 1000 batch: la paginazione non avanza");
    }

    #[test]
    fn round_trip_gpkg() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("out.gpkg");
        let wkb = to_wkb(&geo_types::Geometry::Point(geo_types::Point::new(
            12.5, 45.9,
        )))
        .unwrap();
        let geom = BinaryArray::from(vec![Some(wkb.as_slice()), Some(wkb.as_slice())]);
        let ids = Int64Array::from(vec![1i64, 2]);
        let schema: SchemaRef = Arc::new(Schema::new(vec![
            geometry_field("geom", "EPSG:4326"),
            Field::new("id", DataType::Int64, false),
        ]));
        let batch =
            RecordBatch::try_new(schema.clone(), vec![Arc::new(geom), Arc::new(ids)]).unwrap();

        let driver = GpkgDriver;
        let plan = WritePlan {
            layers: vec![WriteLayer {
                name: "vani".to_owned(),
                contract: DataContract {
                    schema,
                    geometry: None,
                },
            }],
        };
        let mut w = driver
            .create(Sink::Path(path.clone()), &plan, &WriteOptions::default())
            .unwrap();
        w.write(&batch).unwrap();
        w.finish().unwrap();

        let ds = driver.open(Source::Path(path), opzioni_lettura()).unwrap();
        assert_eq!(ds.layers().len(), 1);
        assert_eq!(ds.layers()[0].name, "vani");
        let mut reader = ds
            .open_layer_reader(&ReadRequest {
                layer: LayerId(0),
                projected_fields: None,
                projection_mode: ProjectionMode::BestEffort,
                pruning_predicate: None,
                spatial_pruning_hint: None,
                scope: ReadScope::default(),
                batch_target: BatchTarget::default(),
                cancellation: CancellationToken::default(),
            })
            .unwrap();
        let out = reader.next_batch().unwrap().unwrap();
        assert_eq!(out.num_rows(), 2);
        let gcol = out
            .column_by_name("geom")
            .unwrap()
            .as_any()
            .downcast_ref::<BinaryArray>()
            .unwrap();
        assert_eq!(gcol.value(0), wkb.as_slice());
        // id preservato come Int64
        let idcol = out
            .column_by_name("id")
            .unwrap()
            .as_any()
            .downcast_ref::<Int64Array>()
            .unwrap();
        assert_eq!(idcol.value(1), 2);
        assert!(reader.next_batch().unwrap().is_none());

        let mut attributes_only = ds
            .open_layer_reader(&ReadRequest {
                layer: LayerId(0),
                projected_fields: Some(vec![FieldId(1)]),
                projection_mode: ProjectionMode::Required,
                pruning_predicate: None,
                spatial_pruning_hint: None,
                scope: ReadScope::default(),
                batch_target: BatchTarget::default(),
                cancellation: CancellationToken::default(),
            })
            .unwrap();
        assert!(attributes_only.contract().contract.geometry.is_none());
        assert_eq!(attributes_only.contract().contract.schema.fields().len(), 1);
        let projected = attributes_only.next_batch().unwrap().unwrap();
        assert_eq!(projected.num_rows(), 2);
        assert_eq!(projected.num_columns(), 1);
        assert_eq!(projected.schema().field(0).name(), "id");

        let mut no_columns = ds
            .open_layer_reader(&ReadRequest {
                layer: LayerId(0),
                projected_fields: Some(Vec::new()),
                projection_mode: ProjectionMode::Required,
                pruning_predicate: None,
                spatial_pruning_hint: None,
                scope: ReadScope::default(),
                batch_target: BatchTarget::default(),
                cancellation: CancellationToken::default(),
            })
            .unwrap();
        let projected = no_columns.next_batch().unwrap().unwrap();
        assert_eq!(projected.num_rows(), 2);
        assert_eq!(projected.num_columns(), 0);
    }

    #[test]
    fn round_trip_gpkg_preserves_xyzm_payload_and_native_metadata() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("zm.gpkg");
        let wkb = encode_wkb(
            &WkbGeometry {
                value: WkbValue::Point(WkbCoordinate {
                    x: 1.0,
                    y: 2.0,
                    z: Some(3.0),
                    m: Some(4.0),
                }),
                dimensions: CoordinateDimensions::Xyzm,
                srid: None,
            },
            WkbFlavor::Iso,
        )
        .unwrap();
        let schema: SchemaRef = Arc::new(Schema::new(vec![geometry_field("geom", "EPSG:4326")]));
        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![Arc::new(BinaryArray::from(vec![Some(wkb.as_slice())]))],
        )
        .unwrap();
        let mut geometry = GeometryColumnContract::wkb_passthrough(
            FieldId(0),
            "geom",
            ResolvedCrs::new(Some("EPSG:4326".to_owned()), CrsKind::Geographic, None),
            true,
        );
        geometry.dimensions = CoordinateDimensions::Xyzm;
        geometry.srid = Some(4326);
        geometry.set_exact_geometry_types(vec![GeometryType::Point]);
        let plan = WritePlan {
            layers: vec![WriteLayer {
                name: "zm".to_owned(),
                contract: DataContract {
                    schema,
                    geometry: Some(geometry),
                },
            }],
        };
        let driver = GpkgDriver;
        let mut writer = driver
            .create(Sink::Path(path.clone()), &plan, &WriteOptions::default())
            .unwrap();
        writer.write(&batch).unwrap();
        writer.finish().unwrap();

        let dataset = driver.open(Source::Path(path), opzioni_lettura()).unwrap();
        let geometry = dataset.layers()[0].contract.geometry.as_ref().unwrap();
        assert_eq!(geometry.dimensions, CoordinateDimensions::Xyzm);
        assert_eq!(geometry.srid, Some(4326));
        assert_eq!(geometry.geometry_types, vec![GeometryType::Point]);
        assert_eq!(
            geometry
                .native_metadata
                .get("gpkg.geometry_type_name")
                .map(String::as_str),
            Some("POINT")
        );
        let mut reader = dataset
            .open_layer_reader(&ReadRequest {
                layer: LayerId(0),
                projected_fields: None,
                projection_mode: ProjectionMode::BestEffort,
                pruning_predicate: None,
                spatial_pruning_hint: None,
                scope: ReadScope::default(),
                batch_target: BatchTarget::default(),
                cancellation: CancellationToken::default(),
            })
            .unwrap();
        let output = reader.next_batch().unwrap().unwrap();
        let geometry_array = output
            .column_by_name("geom")
            .unwrap()
            .as_any()
            .downcast_ref::<BinaryArray>()
            .unwrap();
        assert_eq!(geometry_array.value(0), wkb);
    }

    #[test]
    fn write_two_layers_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("multi.gpkg");
        let wkb = to_wkb(&geo_types::Geometry::Point(geo_types::Point::new(1.0, 2.0))).unwrap();

        let s0: SchemaRef = Arc::new(Schema::new(vec![
            geometry_field("geom", "EPSG:4326"),
            Field::new("id", DataType::Int64, false),
        ]));
        let b0 = RecordBatch::try_new(
            s0.clone(),
            vec![
                Arc::new(BinaryArray::from(vec![Some(wkb.as_slice())])),
                Arc::new(Int64Array::from(vec![10i64])),
            ],
        )
        .unwrap();

        let s1: SchemaRef = Arc::new(Schema::new(vec![
            geometry_field("geom", "EPSG:4326"),
            Field::new("nome", DataType::Utf8, true),
        ]));
        let b1 = RecordBatch::try_new(
            s1.clone(),
            vec![
                Arc::new(BinaryArray::from(vec![
                    Some(wkb.as_slice()),
                    Some(wkb.as_slice()),
                ])),
                Arc::new(StringArray::from(vec!["A", "B"])),
            ],
        )
        .unwrap();

        let driver = GpkgDriver;
        let plan = WritePlan {
            layers: vec![
                WriteLayer {
                    name: "vani".to_owned(),
                    contract: DataContract {
                        schema: s0,
                        geometry: None,
                    },
                },
                WriteLayer {
                    name: "strade".to_owned(),
                    contract: DataContract {
                        schema: s1,
                        geometry: None,
                    },
                },
            ],
        };
        let mut w = driver
            .create(Sink::Path(path.clone()), &plan, &WriteOptions::default())
            .unwrap();
        w.write_to_layer(LayerId(0), &b0).unwrap();
        w.write_to_layer(LayerId(1), &b1).unwrap();
        w.finish().unwrap();

        let ds = driver.open(Source::Path(path), opzioni_lettura()).unwrap();
        assert_eq!(ds.layers().len(), 2);
        let names: Vec<&str> = ds.layers().iter().map(|l| l.name.as_str()).collect();
        assert!(
            names.contains(&"vani") && names.contains(&"strade"),
            "layer: {names:?}"
        );

        // Ogni layer ha il suo conteggio righe (instradamento corretto).
        for l in ds.layers() {
            let expected = if l.name == "vani" { 1 } else { 2 };
            let mut r = ds
                .open_layer_reader(&ReadRequest {
                    layer: l.id,
                    projected_fields: None,
                    projection_mode: ProjectionMode::BestEffort,
                    pruning_predicate: None,
                    spatial_pruning_hint: None,
                    scope: ReadScope::default(),
                    batch_target: BatchTarget::default(),
                    cancellation: CancellationToken::default(),
                })
                .unwrap();
            let rb = r.next_batch().unwrap().unwrap();
            assert_eq!(rb.num_rows(), expected, "layer '{}'", l.name);
        }
    }

    #[test]
    fn round_trip_non_wgs84_crs() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("m3857.gpkg");
        // Un punto in EPSG:3857 (Web Mercator).
        let wkb = to_wkb(&geo_types::Geometry::Point(geo_types::Point::new(
            1_113_194.0,
            5_621_521.0,
        )))
        .unwrap();
        let schema: SchemaRef = Arc::new(Schema::new(vec![
            geometry_field("geom", "EPSG:3857"),
            Field::new("id", DataType::Int64, false),
        ]));
        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![
                Arc::new(BinaryArray::from(vec![Some(wkb.as_slice())])),
                Arc::new(Int64Array::from(vec![1i64])),
            ],
        )
        .unwrap();
        let driver = GpkgDriver;
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
            .create(Sink::Path(path.clone()), &plan, &WriteOptions::default())
            .unwrap();
        w.write(&batch).unwrap();
        w.finish().unwrap();

        // Rilettura: il CRS NON è più 4326 fisso, è EPSG:3857.
        let ds = driver.open(Source::Path(path), opzioni_lettura()).unwrap();
        let crs = ds.layers()[0].contract.geometry.as_ref().unwrap().crs.id();
        assert_eq!(crs, Some("EPSG:3857"));
    }
}
