//! driver-geoparquet — `GeoParquet` ⇄ `RecordBatch` (Fase 1). La geometria è WKB:
//! in lettura la colonna binaria viene ri-etichettata `geoarrow.wkb` + `crs`
//! SENZA decodifica (pass-through, V4); in scrittura si emette il metadato `geo`
//! dal contratto. Compressione configurabile (`format_options["compression"]`:
//! snappy default, oppure zstd/gzip/brotli/lz4) — zstd via zstd-sys.
#![forbid(unsafe_code)]

use std::collections::{BTreeSet, HashMap};
use std::fs::File;
use std::path::PathBuf;
use std::sync::Arc;

use arrow_array::{
    Array, ArrayRef, BinaryArray, Float64Array, LargeBinaryArray, RecordBatch, RecordBatchOptions,
};
use arrow_schema::{DataType, Field, Schema, SchemaRef};
use parquet::arrow::arrow_reader::{ParquetRecordBatchReader, ParquetRecordBatchReaderBuilder};
use parquet::arrow::{ArrowWriter, ProjectionMask};
use parquet::basic::{BrotliLevel, Compression, GzipLevel, ZstdLevel};
use parquet::file::metadata::KeyValue;
use parquet::file::properties::WriterProperties;
use parquet::file::statistics::Statistics;

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
use plenora_io_core::request::{
    Bbox, ProjectionMode, PruningComparison, PruningPredicate, PruningScalar, ReadRequest,
};
use plenora_io_core::{
    validate_write, with_write_validation, AttributeWriteSupport, CrsRepresentationCapabilities,
    CrsRepresentationState, CrsWriteSupport, FormatWriteCapabilities, NullabilitySupport,
    TypeCoercionPolicy, WritePlan, ALL_ARROW_TYPES, UTF8_FIELD_NAMES, WKB_PASSTHROUGH_GEOMETRY,
};
use plenora_io_model::contract::{
    CoordinateDimensions, DataContract, FieldId, GeometryColumnContract, GeometryType,
    LayerContract, LayerId,
};
use plenora_io_model::crs::{
    crs_kind_for_authority_id, CrsKind, CrsResolution, RawCrs, ResolvedCrs,
};
use plenora_io_model::geometry::{ARROW_EXTENSION_NAME_KEY, GEOARROW_WKB_EXTENSION, GEO_CRS_KEY};
use plenora_io_model::limits::WkbLimits;
use plenora_io_model::wkb::inspect_wkb;
use plenora_io_model::{PlenoraIoError, Result};

fn fmt_err(reason: impl Into<String>) -> PlenoraIoError {
    PlenoraIoError::format("geoparquet", reason)
}

static DESCRIPTOR: FormatDescriptor = FormatDescriptor {
    id: "geoparquet",
    direction: Direction::Bidirectional,
    read_mode: ReadMode::StreamingColumnar,
    read_determinism: plenora_io_core::DeterminismLevel::Semantic,
    write_mode: Some(WriteMode::Streaming),
    write_determinism: Some(plenora_io_core::DeterminismLevel::Semantic),
    multi_layer: false,
    multi_file: false,
    reader_concurrency: ReaderConcurrency::MultipleIndependentReaders, // Parquet è seekable
    projection_support: plenora_io_core::ProjectionSupport::Exact,
    predicate_pruning_support: plenora_io_core::PredicatePruningSupport::NumericMinMaxStatistics,
    spatial_pruning_support: plenora_io_core::SpatialPruningSupport::BoundingBoxStatistics,
    crs_handling: CrsHandling::Embedded,
    fidelity_class: Fidelity::Lossless,
    runtime: Runtime::PureRust,
    write_capabilities: Some(FormatWriteCapabilities {
        field_names: UTF8_FIELD_NAMES,
        allowed_types: ALL_ARROW_TYPES,
        type_coercion: TypeCoercionPolicy::Reject,
        attributes: AttributeWriteSupport::All,
        geometry: WKB_PASSTHROUGH_GEOMETRY,
        crs: CrsWriteSupport::Embedded,
        crs_representations: CrsRepresentationCapabilities::new(
            CrsRepresentationState::Preserved,
            CrsRepresentationState::Absent,
            CrsRepresentationState::Absent,
        ),
        nullability: NullabilitySupport::Preserve,
        multi_layer: false,
    }),
    semantic_version: 1,
    driver_version: 5,
    descriptor_version: 6,
};

pub struct GeoParquetDriver;

impl FormatDriver for GeoParquetDriver {
    fn descriptor(&self) -> &FormatDescriptor {
        &DESCRIPTOR
    }

    fn open(&self, source: Source, opts: &ReadOptions) -> Result<Box<dyn OpenDatasetHandle>> {
        let path =
            source.into_path_checked(&opts.limits, &opts.cancellation, &opts.resource_budget)?;
        // Il footer Parquet puo' portare la chiave `ARROW:schema`, che e' un
        // messaggio Arrow IPC deserializzato qui dentro: un `.parquet` ostile
        // raggiunge quindi lo stesso panico di un `.arrow`. Vedi
        // `leggendo_arrow`.
        let builder = plenora_io_core::driver::leggendo_arrow("parquet", || {
            ParquetRecordBatchReaderBuilder::try_new(File::open(&path)?)
                .map_err(|e| fmt_err(format!("Parquet non valido: {e}")))
        })?;
        let parquet_schema = builder.schema().clone();
        let geo = read_geo_meta(&builder);
        let (geom_name, crs) = resolve_geometry_and_crs(&parquet_schema, geo.as_ref())?;
        // Finding #4 follow-up follow-up review 2026-08-15: il fallback
        // legacy per-nome (accettare `_bbox_minx/miny/maxx/maxy` come
        // covering anche in assenza di metadata `covering.bbox`) e' stato
        // rimosso dal percorso predefinito perche' un GeoParquet esterno
        // con attributi utente omonimi veniva silenziosamente trattato
        // come covering — perdendo colonne o applicando pruning
        // sbagliato. Il fallback e' ora un opt-in esplicito via
        // `format_options["bbox_legacy_by_name"] = "true"`: chi ha file
        // scritti prima del covering GeoParquet 1.1 lo abilita
        // esplicitamente, prendendosi responsabilita' del comportamento
        // documentato.
        let legacy_by_name_opt_in = opts
            .format_options
            .get("bbox_legacy_by_name")
            .is_some_and(|value| matches!(value.as_str(), "1" | "true" | "yes"));
        let covering_names = covering_bbox_columns(geo.as_ref(), &geom_name);
        // Retag strippa SOLO i nomi realmente dichiarati come covering o —
        // se il caller ha chiesto il fallback legacy — quelli
        // convenzionali. Un file senza covering metadata e senza opt-in
        // conserva tutte le colonne utente esattamente come sono.
        // Il ramo `map_or_else` suggerito da clippy annida due rami
        // logici distinti dentro una sola espressione: qui l'`if let`
        // separa "dichiarato dal metadata" da "opt-in legacy" in modo
        // lineare.
        #[allow(clippy::option_if_let_else)]
        let strip_names: Option<Vec<String>> = if let Some(names) = &covering_names {
            Some(names.clone())
        } else if legacy_by_name_opt_in
            && BBOX_COLS.iter().all(|n| parquet_schema.index_of(n).is_ok())
        {
            Some(BBOX_COLS.iter().map(|s| (*s).to_owned()).collect())
        } else {
            None
        };
        let out_schema = retag_schema(&parquet_schema, &geom_name, &crs, strip_names.as_deref());
        // Il covering realmente utilizzabile per il pruning: solo se
        // dichiarato dal metadata E tutte le colonne dichiarate sono
        // presenti, oppure se l'opt-in legacy e' attivo E i quattro nomi
        // convenzionali sono presenti. In ogni altro caso il pruning
        // spaziale resta disabilitato.
        #[allow(clippy::option_if_let_else)]
        let bbox_covering: Option<[String; 4]> = if let Some(declared) = &covering_names {
            let all_present = declared.iter().all(|n| parquet_schema.index_of(n).is_ok());
            if all_present && declared.len() == 4 {
                Some([
                    declared[0].clone(),
                    declared[1].clone(),
                    declared[2].clone(),
                    declared[3].clone(),
                ])
            } else {
                None
            }
        } else if legacy_by_name_opt_in
            && BBOX_COLS.iter().all(|n| parquet_schema.index_of(n).is_ok())
        {
            Some([
                BBOX_COLS[0].to_owned(),
                BBOX_COLS[1].to_owned(),
                BBOX_COLS[2].to_owned(),
                BBOX_COLS[3].to_owned(),
            ])
        } else {
            None
        };
        // Mappa logico → fisico prima di consumare `out_schema` per il
        // contratto. Ogni campo esposto viene localizzato per nome nello
        // schema Parquet originale. Un campo esposto senza corrispondente
        // fisico e' un errore di contratto (mai atteso, ma fail-closed).
        let mut visible_to_physical: Vec<usize> = Vec::with_capacity(out_schema.fields().len());
        for field in out_schema.fields() {
            let index = parquet_schema.index_of(field.name()).map_err(|_| {
                fmt_err(format!(
                    "campo esposto '{}' non presente nello schema Parquet fisico",
                    field.name()
                ))
            })?;
            visible_to_physical.push(index);
        }
        let geom_idx = out_schema
            .index_of(&geom_name)
            .map_err(|e| fmt_err(format!("colonna geometria: {e}")))?;
        // Indice di colonna di uno schema Parquet: limitato a poche migliaia di
        // campi, il cast a u32 non puo' troncare.
        #[allow(clippy::cast_possible_truncation)]
        let geometry_field_id = FieldId(geom_idx as u32);
        let mut geometry = GeometryColumnContract::wkb_passthrough(
            geometry_field_id,
            geom_name.clone(),
            crs,
            out_schema.field(geom_idx).is_nullable(),
        );
        apply_geo_column_metadata(&mut geometry, geo.as_ref(), &geom_name);
        let contract = DataContract::new(out_schema, Some(geometry));
        // `DataContract::new` rende i metadati geometrici del contratto
        // autoritativi; anche i batch runtime devono essere retaggati con
        // quello stesso schema, non con la versione intermedia.
        let out_schema = contract.schema.clone();
        let name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("layer")
            .to_owned();
        let layer = LayerContract {
            id: LayerId(0),
            name,
            contract,
        };
        Ok(plenora_io_core::with_read_budget(
            Box::new(GeoParquetDataset {
                path,
                out_schema,
                bbox_covering,
                visible_to_physical,
                layers: vec![layer],
            }),
            opts.resource_budget.clone(),
            true,
        ))
    }

    fn create(
        &self,
        sink: Sink,
        plan: &WritePlan,
        opts: &WriteOptions,
    ) -> Result<Box<dyn FormatWriter>> {
        validate_write(self.descriptor(), plan, &opts.limits)?;
        let Sink::Path(path) = sink;
        if path.exists() {
            return Err(PlenoraIoError::OutputExists(path.display().to_string()));
        }
        if !path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("parquet"))
        {
            return Err(PlenoraIoError::Unsupported(
                "l'output deve avere estensione .parquet".to_owned(),
            ));
        }
        if plan.layers.len() != 1 {
            return Err(PlenoraIoError::Unsupported(
                "GeoParquet: un solo layer per dataset nella v1".to_owned(),
            ));
        }
        let layer = &plan.layers[0];
        let schema = layer.contract.schema.clone();
        // Finding #4 review 2026-08-15: prima del fix il writer aggiungeva
        // sempre le 4 colonne bbox interne alle colonne utente, senza
        // controllare che non esistessero gia' con quei nomi. Il risultato
        // era una sovrascrittura silenziosa che alterava il contratto
        // dichiarato dall'utente. Fail-closed qui rifiuta il piano prima
        // di aprire il sink: e' la stessa policy di collisione applicata
        // dagli altri driver ai propri metadati interni.
        if let Some(collision) = schema
            .fields()
            .iter()
            .map(|field| field.name().as_str())
            .find(|name| is_bbox_col(name))
        {
            return Err(fmt_err(format!(
                "GeoParquet: colonna utente '{collision}' entrerebbe in collisione con \
                 le colonne bbox interne del covering spaziale ({}); rinominare la colonna \
                 utente prima della scrittura",
                BBOX_COLS.join(", ")
            )));
        }
        let (geom_idx, geom_name, legacy_crs_meta) = geometry_field(&schema)?;
        let crs_meta = crs_meta_for_write(layer.contract.geometry.as_ref(), legacy_crs_meta);
        // Schema di scrittura = utente + colonne bbox covering (spatial pruning).
        let mut aug_fields: Vec<Field> =
            schema.fields().iter().map(|f| f.as_ref().clone()).collect();
        aug_fields.extend(bbox_fields());
        let write_schema: SchemaRef = Arc::new(Schema::new_with_metadata(
            aug_fields,
            schema.metadata().clone(),
        ));
        let staging = StagedFile::new(&path, opts.durable, opts.max_output_bytes())?;
        // Row group da 64k righe: statistiche min/max abbastanza granulari da
        // rendere efficace il row-group pruning in lettura (Fase 2C).
        let props = WriterProperties::builder()
            .set_compression(compression_from(opts))
            .set_max_row_group_row_count(Some(65_536))
            .build();
        let writer = ArrowWriter::try_new(staging.reopen()?, write_schema.clone(), Some(props))
            .map_err(|e| fmt_err(format!("writer: {e}")))?;
        with_write_validation(
            Box::new(GeoParquetWriter {
                staging,
                writer: Some(writer),
                write_schema,
                geom_idx,
                geom_name,
                crs_meta,
                geometry_types: BTreeSet::new(),
                wkb_limits: opts.limits.effective_wkb(),
            }),
            self.descriptor(),
            plan,
            opts.limits,
            opts.cancellation.clone(),
            opts.resource_budget.clone(),
        )
    }
}

fn crs_meta_for_write(
    geometry: Option<&GeometryColumnContract>,
    legacy_crs_meta: Option<String>,
) -> Option<String> {
    geometry
        .and_then(|geometry| match &geometry.crs {
            CrsResolution::Resolved(crs) => crs.id.as_deref(),
            CrsResolution::DeclaredButUnresolved(raw) => raw.authority_hint.as_deref(),
            CrsResolution::Missing => None,
        })
        .map(str::to_owned)
        .or(legacy_crs_meta)
}

struct GeoParquetDataset {
    path: PathBuf,
    out_schema: SchemaRef,
    /// Nomi delle colonne bbox del covering spaziale, se il file dichiara
    /// `covering.bbox` nel metadata `GeoParquet` 1.1 o, in fallback, se sono
    /// presenti tutti e quattro i nomi convenzionali `_bbox_minx`/... .
    /// Il pruning spaziale legge min/max da queste colonne (finding #4
    /// review 2026-08-15 + follow-up).
    bbox_covering: Option<[String; 4]>,
    /// Mappa dagli indici logici dello schema esposto (`out_schema`, senza
    /// le colonne bbox interne) agli indici fisici root dello schema
    /// `Parquet`. Prima del follow-up review 2026-08-15 la CLI passava
    /// direttamente `0..out_schema.len()` a `ProjectionMask::roots`, che
    /// coincide col fisico solo se le colonne rimosse sono in coda: un
    /// `GeoParquet` esterno con bbox intercalate produceva colonne
    /// sbagliate o errore di schema. Ora la mappa e' calcolata una volta
    /// all'apertura e ogni projection la usa per tradurre.
    visible_to_physical: Vec<usize>,
    layers: Vec<LayerContract>,
}

impl OpenDatasetHandle for GeoParquetDataset {
    fn layers(&self) -> &[LayerContract] {
        &self.layers
    }

    fn fidelity_assessment(&self) -> plenora_io_core::FidelityAssessment {
        plenora_io_core::FidelityAssessment::for_format(DESCRIPTOR.id, DESCRIPTOR.fidelity_class)
    }

    fn open_layer_reader(&self, request: &ReadRequest) -> Result<Box<dyn LayerReader>> {
        plenora_io_core::validate_read_projection(&DESCRIPTOR, request)?;
        let path = self.path.clone();
        let builder = plenora_io_core::driver::leggendo_arrow("parquet", move || {
            ParquetRecordBatchReaderBuilder::try_new(File::open(&path)?)
                .map_err(|e| fmt_err(format!("Parquet non valido: {e}")))
        })?;

        // Projection pushdown (Fase 2C): se richiesto, leggi SOLO quelle colonne.
        // Con bbox covering, le colonne bbox interne sono SEMPRE proiettate via.
        //
        // Finding #4 follow-up review 2026-08-15: `ProjectionMask::roots`
        // interpreta gli indici come fisici (root Parquet). Gli indici
        // logici dello schema esposto NON coincidono col fisico quando le
        // colonne bbox interne sono intercalate — cosa che i nostri writer
        // non producono ma un GeoParquet esterno puo'. `visible_to_physical`
        // fa la traduzione una volta all'apertura e ogni projection la usa.
        let (builder, out_schema, layer) = match &request.projected_fields {
            None if self.bbox_covering.is_some() => {
                let mask = ProjectionMask::roots(
                    builder.parquet_schema(),
                    self.visible_to_physical.iter().copied(),
                );
                (
                    builder.with_projection(mask),
                    self.out_schema.clone(),
                    self.layers[0].clone(),
                )
            }
            None => (builder, self.out_schema.clone(), self.layers[0].clone()),
            Some(field_ids) => {
                let ncols = self.out_schema.fields().len();
                let mut logical_idx: Vec<usize> = Vec::new();
                for fid in field_ids {
                    let i = fid.0 as usize;
                    if i >= ncols {
                        if request.projection_mode == ProjectionMode::Required {
                            return Err(PlenoraIoError::Unsupported(format!(
                                "projection Required: field id {} fuori range",
                                fid.0
                            )));
                        }
                        continue;
                    }
                    if !logical_idx.contains(&i) {
                        logical_idx.push(i);
                    }
                }
                logical_idx.sort_unstable();
                // Schema proiettato: sottoinsieme in ordine originale (geometria già
                // ri-etichettata geoarrow.wkb se presente fra le colonne scelte).
                let fields: Vec<Field> = logical_idx
                    .iter()
                    .map(|&i| self.out_schema.field(i).as_ref().clone())
                    .collect();
                let projected: SchemaRef = Arc::new(Schema::new_with_metadata(
                    fields,
                    self.out_schema.metadata().clone(),
                ));
                // Traduce gli indici logici richiesti nei corrispondenti
                // indici fisici prima di costruire la mask.
                let physical_idx: Vec<usize> = logical_idx
                    .iter()
                    .map(|&i| self.visible_to_physical[i])
                    .collect();
                let mask = ProjectionMask::roots(builder.parquet_schema(), physical_idx);
                let mut layer = self.layers[0].clone();
                layer.contract = DataContract {
                    schema: projected.clone(),
                    geometry: layer.contract.geometry.and_then(|g| {
                        projected.index_of(&g.name).ok().map(|i| {
                            // Indice di colonna di uno schema Arrow: il
                            // cast a u32 non puo' troncare.
                            #[allow(clippy::cast_possible_truncation)]
                            let field_id = FieldId(i as u32);
                            GeometryColumnContract { field_id, ..g }
                        })
                    }),
                };
                (builder.with_projection(mask), projected, layer)
            }
        };

        // Batch sizing adattivo: combina il tetto righe con target_bytes.
        let batch_size =
            plenora_io_core::effective_batch_rows(out_schema.as_ref(), request.batch_target);
        let builder = builder.with_batch_size(batch_size);
        // Row-group pruning (2C): salta i row group esclusi dalle statistiche
        // min/max (mai filtering riga-per-riga; over-return, mai under-return).
        let builder = apply_pruning(
            builder,
            request.pruning_predicate.as_ref(),
            self.out_schema.as_ref(),
        );
        // Spatial pruning (2C): salta i row group il cui bbox non interseca l'hint.
        let builder = apply_spatial_pruning(
            builder,
            request.spatial_pruning_hint.as_ref(),
            self.bbox_covering.as_ref(),
        );

        let reader = builder
            .build()
            .map_err(|e| fmt_err(format!("lettura: {e}")))?;
        let reader: Box<dyn LayerReader> = Box::new(GeoParquetReader {
            reader,
            out_schema,
            layer,
        });
        Ok(plenora_io_core::with_cancellation(
            reader,
            request.cancellation.clone(),
        ))
    }
}

struct GeoParquetReader {
    reader: ParquetRecordBatchReader,
    out_schema: SchemaRef,
    layer: LayerContract,
}

impl LayerReader for GeoParquetReader {
    fn contract(&self) -> &LayerContract {
        &self.layer
    }

    fn next_batch(&mut self) -> Result<Option<RecordBatch>> {
        // La barriera copre la decodifica dei buffer, non solo lo schema letto
        // all'apertura: arrow decodifica il batch a ogni `next()`, e un offset
        // oltre la lunghezza dichiarata panica invece di dare un errore.
        //
        // Dopo un panico catturato il reader resta in uno stato non definito.
        // Non e' un problema: il chiamante riceve un errore e il contratto di
        // `LayerReader` non prevede di proseguire dopo un errore.
        let reader = &mut self.reader;
        let prossimo =
            plenora_io_core::driver::leggendo_arrow("parquet", move || match reader.next() {
                None => Ok(None),
                Some(Err(e)) => Err(fmt_err(format!("batch: {e}"))),
                Some(Ok(batch)) => Ok(Some(batch)),
            })?;
        match prossimo {
            None => Ok(None),
            Some(batch) => {
                // Ri-etichetta lo schema (geometria -> geoarrow.wkb) senza toccare
                // i buffer: pass-through delle colonne.
                let options = RecordBatchOptions::new().with_row_count(Some(batch.num_rows()));
                let retagged = RecordBatch::try_new_with_options(
                    self.out_schema.clone(),
                    batch.columns().to_vec(),
                    &options,
                )
                .map_err(|e| fmt_err(format!("re-tag schema: {e}")))?;
                Ok(Some(retagged))
            }
        }
    }
}

struct GeoParquetWriter {
    staging: StagedFile,
    writer: Option<ArrowWriter<File>>,
    write_schema: SchemaRef,
    geom_idx: usize,
    geom_name: String,
    crs_meta: Option<String>,
    geometry_types: BTreeSet<(GeometryType, CoordinateDimensions)>,
    wkb_limits: WkbLimits,
}

impl FormatWriter for GeoParquetWriter {
    fn write(&mut self, batch: &RecordBatch) -> Result<()> {
        let geom = batch.column(self.geom_idx);
        accumulate_geometry_types(geom, &mut self.geometry_types, &self.wkb_limits)?;
        // Aggiunge le 4 colonne bbox covering per il pruning spaziale.
        let bbox_cols = geom.as_any().downcast_ref::<BinaryArray>().map_or_else(
            || {
                bbox_fields()
                    .iter()
                    .map(|_| Arc::new(Float64Array::new_null(batch.num_rows())) as ArrayRef)
                    .collect()
            },
            build_bbox_columns,
        );
        let mut cols: Vec<ArrayRef> = batch.columns().to_vec();
        cols.extend(bbox_cols);
        let aug = RecordBatch::try_new(self.write_schema.clone(), cols)
            .map_err(|e| fmt_err(format!("augment bbox: {e}")))?;
        self.writer
            .as_mut()
            .ok_or_else(|| fmt_err("writer Parquet non disponibile"))?
            .write(&aug)
            .map_err(|e| fmt_err(format!("write: {e}")))
    }

    fn finish(mut self: Box<Self>) -> Result<Published> {
        let mut writer = self
            .writer
            .take()
            .ok_or_else(|| fmt_err("writer Parquet non disponibile al finish"))?;
        let geo = build_geo_metadata(
            &self.geom_name,
            &self.geometry_types,
            self.crs_meta.as_deref(),
        )?;
        writer.append_key_value_metadata(KeyValue::new("geo".to_owned(), geo));
        writer.close().map_err(|e| fmt_err(format!("close: {e}")))?;
        let (bytes, outcome) = self.staging.publish()?;
        Ok(Published {
            bytes,
            loss: LossReport::default(),
            fidelity: plenora_io_core::FidelityAssessment::lossless(),
            outcome,
        })
    }
}

// --- helpers ---------------------------------------------------------------

// --- row-group pruning (2C) ------------------------------------------------

#[derive(Clone, Copy)]
enum NumericRange {
    Int64(i64, i64),
    Float64(f64, f64),
}

/// Predicato opaco "colonna OP valore" (OP: >, >=, <, <=, =/==).
fn parse_opaque_predicate(s: &str) -> Option<(String, PruningComparison, PruningScalar)> {
    for (symbol, comparison) in [
        (">=", PruningComparison::GreaterThanOrEqual),
        ("<=", PruningComparison::LessThanOrEqual),
        ("==", PruningComparison::Equal),
        (">", PruningComparison::GreaterThan),
        ("<", PruningComparison::LessThan),
        ("=", PruningComparison::Equal),
    ] {
        if let Some((left, right)) = s.split_once(symbol) {
            let column = left.trim().to_owned();
            if column.is_empty() {
                return None;
            }
            let literal = right.trim();
            let value = literal
                .parse::<i64>()
                .map(PruningScalar::Int64)
                .or_else(|_| literal.parse::<f64>().map(PruningScalar::Float64))
                .ok()?;
            if matches!(value, PruningScalar::Float64(value) if !value.is_finite()) {
                return None;
            }
            return Some((column, comparison, value));
        }
    }
    None
}

fn stat_range(stats: &Statistics) -> Option<NumericRange> {
    match stats {
        Statistics::Int32(stats) => {
            let min = i64::from(*stats.min_opt()?);
            let max = i64::from(*stats.max_opt()?);
            (min <= max).then_some(NumericRange::Int64(min, max))
        }
        Statistics::Int64(stats) => {
            let min = *stats.min_opt()?;
            let max = *stats.max_opt()?;
            (min <= max).then_some(NumericRange::Int64(min, max))
        }
        Statistics::Float(stats) => {
            let min = f64::from(*stats.min_opt()?);
            let max = f64::from(*stats.max_opt()?);
            (min.is_finite() && max.is_finite() && min <= max)
                .then_some(NumericRange::Float64(min, max))
        }
        Statistics::Double(stats) => {
            let min = *stats.min_opt()?;
            let max = *stats.max_opt()?;
            (min.is_finite() && max.is_finite() && min <= max)
                .then_some(NumericRange::Float64(min, max))
        }
        _ => None,
    }
}

fn stat_f64_range(stats: &Statistics) -> Option<(f64, f64)> {
    let NumericRange::Float64(min, max) = stat_range(stats)? else {
        return None;
    };
    Some((min, max))
}

fn range_matches(
    range: NumericRange,
    comparison: PruningComparison,
    value: PruningScalar,
) -> Option<bool> {
    match (range, value) {
        (NumericRange::Int64(min, max), PruningScalar::Int64(value)) => {
            Some(comparison_matches(min, max, comparison, value))
        }
        (NumericRange::Float64(min, max), PruningScalar::Float64(value)) if value.is_finite() => {
            Some(comparison_matches(min, max, comparison, value))
        }
        _ => None,
    }
}

fn comparison_matches<T: PartialOrd + Copy>(
    min: T,
    max: T,
    comparison: PruningComparison,
    value: T,
) -> bool {
    match comparison {
        PruningComparison::GreaterThan => max > value,
        PruningComparison::GreaterThanOrEqual => max >= value,
        PruningComparison::LessThan => min < value,
        PruningComparison::LessThanOrEqual => min <= value,
        PruningComparison::Equal => min <= value && value <= max,
    }
}

/// Seleziona i row group che POSSONO soddisfare il predicato (over-return: se le
/// statistiche mancano o il predicato non è riconosciuto, tiene tutto).
fn apply_pruning(
    builder: ParquetRecordBatchReaderBuilder<File>,
    pred: Option<&PruningPredicate>,
    arrow_schema: &Schema,
) -> ParquetRecordBatchReaderBuilder<File> {
    let Some(predicate) = pred else {
        return builder;
    };
    let resolved = match predicate {
        PruningPredicate::NumericComparison {
            field,
            comparison,
            value,
        } => arrow_schema
            .fields()
            .get(field.0 as usize)
            .map(|field| (field.name().clone(), *comparison, *value)),
        PruningPredicate::Opaque(expression) => parse_opaque_predicate(expression),
    };
    let Some((column, comparison, value)) = resolved else {
        return builder;
    };
    let schema = builder.parquet_schema();
    let mut matching =
        (0..schema.num_columns()).filter(|&i| schema.column(i).name() == column.as_str());
    let Some(cidx) = matching.next() else {
        return builder;
    };
    if matching.next().is_some() {
        return builder;
    }
    let md = builder.metadata();
    let mut keep = Vec::new();
    for rg in 0..md.num_row_groups() {
        let keep_it = md
            .row_group(rg)
            .column(cidx)
            .statistics()
            .and_then(stat_range)
            .is_none_or(|range| range_matches(range, comparison, value).unwrap_or(true));
        if keep_it {
            keep.push(rg);
        }
    }
    builder.with_row_groups(keep)
}

/// Spatial pruning: tiene i row group il cui estensione bbox interseca l'hint.
/// Over-return (stats mancanti → tiene); mai under-return.
// I nomi `cminx`/`cminy` e `minx`/`miny` sono le componenti canoniche di un
// bounding box: rinominarle per soddisfare `similar_names` renderebbe il codice
// meno leggibile, non più.
#[allow(clippy::similar_names)]
fn apply_spatial_pruning(
    builder: ParquetRecordBatchReaderBuilder<File>,
    hint: Option<&Bbox>,
    // Finding #4 follow-up: i nomi delle 4 colonne bbox (xmin, ymin, xmax,
    // ymax) sono passati dal chiamante, che li ha risolti da
    // `covering.bbox` GeoParquet 1.1 o dal fallback storico su `BBOX_COLS`.
    // Non piu' hard-coded: un covering con nomi personalizzati viene ora
    // realmente usato dal pruning.
    covering: Option<&[String; 4]>,
) -> ParquetRecordBatchReaderBuilder<File> {
    let (Some(q), Some(covering)) = (hint, covering) else {
        return builder;
    };
    let schema = builder.parquet_schema();
    let leaf = |name: &str| (0..schema.num_columns()).find(|&i| schema.column(i).name() == name);
    let (Some(cminx), Some(cminy), Some(cmaxx), Some(cmaxy)) = (
        leaf(&covering[0]),
        leaf(&covering[1]),
        leaf(&covering[2]),
        leaf(&covering[3]),
    ) else {
        return builder;
    };
    let md = builder.metadata();
    let mut keep = Vec::new();
    for rg in 0..md.num_row_groups() {
        let g = md.row_group(rg);
        // Estensione del row group: min(minx),min(miny) .. max(maxx),max(maxy).
        let ext = (
            g.column(cminx)
                .statistics()
                .and_then(stat_f64_range)
                .map(|(a, _)| a),
            g.column(cminy)
                .statistics()
                .and_then(stat_f64_range)
                .map(|(a, _)| a),
            g.column(cmaxx)
                .statistics()
                .and_then(stat_f64_range)
                .map(|(_, b)| b),
            g.column(cmaxy)
                .statistics()
                .and_then(stat_f64_range)
                .map(|(_, b)| b),
        );
        let keep_it = match ext {
            (Some(minx), Some(miny), Some(maxx), Some(maxy)) => {
                // Interseca l'hint? (nessuna intersezione = fuori da un lato)
                !(maxx < q.minx || minx > q.maxx || maxy < q.miny || miny > q.maxy)
            }
            _ => true,
        };
        if keep_it {
            keep.push(rg);
        }
    }
    builder.with_row_groups(keep)
}

/// Compressione dal `format_options["compression"]` (default snappy). zstd via
/// zstd-sys (unica dep C oltre a GDAL/filegdb), sia in lettura che scrittura.
fn compression_from(opts: &WriteOptions) -> Compression {
    match opts.format_options.get("compression").map(String::as_str) {
        Some("zstd") => Compression::ZSTD(ZstdLevel::default()),
        Some("gzip") => Compression::GZIP(GzipLevel::default()),
        Some("brotli") => Compression::BROTLI(BrotliLevel::default()),
        Some("lz4") => Compression::LZ4,
        Some("none" | "uncompressed") => Compression::UNCOMPRESSED,
        _ => Compression::SNAPPY,
    }
}

fn read_geo_meta(builder: &ParquetRecordBatchReaderBuilder<File>) -> Option<serde_json::Value> {
    let kv = builder.metadata().file_metadata().key_value_metadata()?;
    let raw = kv
        .iter()
        .find(|e| e.key == "geo")
        .and_then(|e| e.value.clone())?;
    serde_json::from_str(&raw).ok()
}

/// Nome colonna geometria + CRS risolto dai metadati `geo`.
fn resolve_geometry_and_crs(
    schema: &Schema,
    geo: Option<&serde_json::Value>,
) -> Result<(String, ResolvedCrs)> {
    let primary = geo
        .and_then(|g| g.get("primary_column"))
        .and_then(|v| v.as_str())
        .map(str::to_owned)
        .or_else(|| {
            ["geometry", "geom", "wkb"]
                .iter()
                .find(|n| schema.index_of(n).is_ok())
                .map(std::string::ToString::to_string)
        })
        .ok_or_else(|| fmt_err("nessuna colonna geometria: non è GeoParquet"))?;
    let crs = crs_from(geo, &primary)?;
    Ok((primary, crs))
}

fn crs_from(geo: Option<&serde_json::Value>, primary: &str) -> Result<ResolvedCrs> {
    let crs = geo
        .and_then(|g| g.get("columns"))
        .and_then(|c| c.get(primary))
        .and_then(|c| c.get("crs"));
    match crs {
        None | Some(serde_json::Value::Null) => Ok(ResolvedCrs::wgs84()),
        Some(v) => {
            let id = v.get("id").and_then(|i| {
                let a = i.get("authority").and_then(|a| a.as_str())?;
                let code = i.get("code").map(|c| match c {
                    serde_json::Value::Number(n) => n.to_string(),
                    serde_json::Value::String(s) => s.clone(),
                    other => other.to_string(),
                })?;
                Some(format!("{a}:{code}"))
            });
            let definition = v.to_string();
            let Some(id) = id else {
                let raw = RawCrs::new(
                    definition,
                    v.get("id")
                        .and_then(|i| i.get("authority"))
                        .and_then(|a| a.as_str())
                        .map(str::to_owned),
                );
                return Err(PlenoraIoError::crs_unresolved("geoparquet", &raw));
            };
            let kind = if crs_kind_for_authority_id(&id) == CrsKind::Geographic
                || v.get("type").and_then(|t| t.as_str()) == Some("GeographicCRS")
            {
                CrsKind::Geographic
            } else if v.get("type").and_then(|t| t.as_str()) == Some("ProjectedCRS") {
                CrsKind::Projected
            } else {
                CrsKind::Unknown
            };
            Ok(ResolvedCrs::new(Some(id), kind, Some(definition)))
        }
    }
}

// La catena di suffissi è ordinata (" ZM" prima di " Z"/" M"): tradurla in
// `map_or_else` annidati non cambierebbe il risultato ma nasconderebbe la
// priorità, che qui è il contratto della funzione.
#[allow(clippy::option_if_let_else)]
fn parse_geo_type_label(label: &str) -> Option<(GeometryType, CoordinateDimensions)> {
    let (name, dimensions) = if let Some(name) = label.strip_suffix(" ZM") {
        (name, CoordinateDimensions::Xyzm)
    } else if let Some(name) = label.strip_suffix(" Z") {
        (name, CoordinateDimensions::Xyz)
    } else if let Some(name) = label.strip_suffix(" M") {
        (name, CoordinateDimensions::Xym)
    } else {
        (label, CoordinateDimensions::Xy)
    };
    let geometry_type = match name {
        "Point" => GeometryType::Point,
        "LineString" => GeometryType::LineString,
        "Polygon" => GeometryType::Polygon,
        "MultiPoint" => GeometryType::MultiPoint,
        "MultiLineString" => GeometryType::MultiLineString,
        "MultiPolygon" => GeometryType::MultiPolygon,
        "GeometryCollection" => GeometryType::GeometryCollection,
        _ => return None,
    };
    Some((geometry_type, dimensions))
}

fn apply_geo_column_metadata(
    contract: &mut GeometryColumnContract,
    geo: Option<&serde_json::Value>,
    primary: &str,
) {
    let Some(column) = geo
        .and_then(|value| value.get("columns"))
        .and_then(|columns| columns.get(primary))
    else {
        return;
    };
    contract
        .native_metadata
        .insert("geoparquet.column".to_owned(), column.to_string());
    let mut dimensions = BTreeSet::new();
    if let Some(labels) = column.get("geometry_types").and_then(|v| v.as_array()) {
        for label in labels.iter().filter_map(|value| value.as_str()) {
            if let Some((geometry_type, dimension)) = parse_geo_type_label(label) {
                if !contract.geometry_types.contains(&geometry_type) {
                    contract.geometry_types.push(geometry_type);
                }
                dimensions.insert(dimension);
            }
        }
    }
    if !contract.geometry_types.is_empty() {
        let geometry_types = std::mem::take(&mut contract.geometry_types);
        contract.set_exact_geometry_types(geometry_types);
    }
    if dimensions.len() == 1 {
        contract.dimensions = dimensions
            .first()
            .copied()
            .unwrap_or(CoordinateDimensions::Unknown);
    }
    contract.srid = contract
        .crs
        .id()
        .and_then(|id| id.strip_prefix("EPSG:"))
        .and_then(|code| code.parse().ok());
}

// --- bbox covering (spatial pruning, 2C) -----------------------------------

/// Colonne bbox interne (covering "plenora"): 4 f64 flat per row, con statistiche
/// min/max Parquet per row group → pruning spaziale.
const BBOX_COLS: [&str; 4] = ["_bbox_minx", "_bbox_miny", "_bbox_maxx", "_bbox_maxy"];

fn is_bbox_col(name: &str) -> bool {
    BBOX_COLS.contains(&name)
}

fn bbox_fields() -> Vec<Field> {
    BBOX_COLS
        .iter()
        .map(|n| Field::new(*n, DataType::Float64, true))
        .collect()
}

fn upd(bb: &mut [f64; 4], x: f64, y: f64) {
    if x < bb[0] {
        bb[0] = x;
    }
    if y < bb[1] {
        bb[1] = y;
    }
    if x > bb[2] {
        bb[2] = x;
    }
    if y > bb[3] {
        bb[3] = y;
    }
}

fn rd_u32(b: &[u8], off: &mut usize, le: bool) -> Option<u32> {
    let s = b.get(*off..*off + 4)?;
    *off += 4;
    let a = [s[0], s[1], s[2], s[3]];
    Some(if le {
        u32::from_le_bytes(a)
    } else {
        u32::from_be_bytes(a)
    })
}

fn rd_f64(b: &[u8], off: &mut usize, le: bool) -> Option<f64> {
    let s = b.get(*off..*off + 8)?;
    *off += 8;
    let mut a = [0u8; 8];
    a.copy_from_slice(s);
    Some(if le {
        f64::from_le_bytes(a)
    } else {
        f64::from_be_bytes(a)
    })
}

fn scan_wkb(bytes: &[u8], off: &mut usize, bb: &mut [f64; 4], depth: u32) -> Option<()> {
    if depth > 32 {
        return None;
    }
    let le = match *bytes.get(*off)? {
        0 => false,
        1 => true,
        _ => return None,
    };
    *off += 1;
    let raw = rd_u32(bytes, off, le)?;
    let (base, has_z, has_m, srid) = if raw & 0xE000_0000 != 0 {
        (
            raw & 0x1FFF_FFFF,
            raw & 0x8000_0000 != 0,
            raw & 0x4000_0000 != 0,
            raw & 0x2000_0000 != 0,
        )
    } else {
        let dimension = raw / 1000;
        if dimension > 3 {
            return None;
        }
        (
            raw % 1000,
            dimension == 1 || dimension == 3,
            dimension == 2 || dimension == 3,
            false,
        )
    };
    if srid {
        rd_u32(bytes, off, le)?;
    }
    let scan_coord = |off: &mut usize, bb: &mut [f64; 4]| -> Option<()> {
        let x = rd_f64(bytes, off, le)?;
        let y = rd_f64(bytes, off, le)?;
        if has_z {
            rd_f64(bytes, off, le)?;
        }
        if has_m {
            rd_f64(bytes, off, le)?;
        }
        upd(bb, x, y);
        Some(())
    };
    match base {
        1 => {
            scan_coord(off, bb)?;
        }
        2 => {
            let count = rd_u32(bytes, off, le)?;
            for _ in 0..count {
                scan_coord(off, bb)?;
            }
        }
        3 => {
            let rings = rd_u32(bytes, off, le)?;
            for _ in 0..rings {
                let npts = rd_u32(bytes, off, le)?;
                for _ in 0..npts {
                    scan_coord(off, bb)?;
                }
            }
        }
        4..=7 => {
            let count = rd_u32(bytes, off, le)?;
            for _ in 0..count {
                scan_wkb(bytes, off, bb, depth + 1)?;
            }
        }
        _ => return None,
    }
    Some(())
}

/// Bounding box 2D da WKB senza costruire geometrie. `None` se non-2D o malformato
/// (robusto al fuzzing: nessun panic, nessun loop illimitato).
#[doc(hidden)] // esposto solo per il fuzzer (plenora-fuzz)
#[must_use]
pub fn wkb_bbox(bytes: &[u8]) -> Option<[f64; 4]> {
    let mut bb = [
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    ];
    let mut off = 0usize;
    scan_wkb(bytes, &mut off, &mut bb, 0)?;
    if off != bytes.len() {
        return None;
    }
    if bb.iter().all(|v| v.is_finite()) {
        Some(bb)
    } else {
        None
    }
}

/// Costruisce le 4 colonne bbox per un batch, dalla colonna geometria WKB.
// `minx`/`miny` e `maxx`/`maxy` sono le componenti canoniche di un bounding
// box: rinominarle per soddisfare `similar_names` peggiorerebbe la leggibilità.
#[allow(clippy::similar_names)]
fn build_bbox_columns(geom: &BinaryArray) -> Vec<ArrayRef> {
    let rows = geom.len();
    let (mut minx, mut miny, mut maxx, mut maxy) = (
        Vec::with_capacity(rows),
        Vec::with_capacity(rows),
        Vec::with_capacity(rows),
        Vec::with_capacity(rows),
    );
    for row in 0..rows {
        let bbox = if geom.is_null(row) {
            None
        } else {
            wkb_bbox(geom.value(row))
        };
        minx.push(bbox.map(|bbox| bbox[0]));
        miny.push(bbox.map(|bbox| bbox[1]));
        maxx.push(bbox.map(|bbox| bbox[2]));
        maxy.push(bbox.map(|bbox| bbox[3]));
    }
    vec![
        Arc::new(Float64Array::from(minx)),
        Arc::new(Float64Array::from(miny)),
        Arc::new(Float64Array::from(maxx)),
        Arc::new(Float64Array::from(maxy)),
    ]
}

/// Estrae dai metadati `geo.columns.<primary>.covering.bbox` la lista dei
/// nomi delle colonne bbox del covering spaziale (finding #4 review
/// 2026-08-15). Un file scritto dal writer post-fix dichiara esplicitamente
/// il mapping; per i file legacy (anche quelli emessi dai nostri writer
/// precedenti) il chiamante puo' fare fallback ai nomi convenzionali
/// `BBOX_COLS`.
fn covering_bbox_columns(geo: Option<&serde_json::Value>, primary: &str) -> Option<Vec<String>> {
    let covering = geo?
        .get("columns")?
        .get(primary)?
        .get("covering")?
        .get("bbox")?;
    let mut names = Vec::with_capacity(4);
    for edge in ["xmin", "ymin", "xmax", "ymax"] {
        // Il covering `GeoParquet` 1.1 espone i column path come array di
        // stringhe; la specifica ammette anche path annidati per campi
        // dentro struct (es. `["bbox", "xmin"]`). Il pruning del driver
        // opera sui root fields Parquet, quindi accetta esplicitamente
        // solo path di lunghezza 1. Un covering annidato non viene
        // interpretato: il fallback documentato torna al comportamento
        // "nessun covering utilizzabile" invece di prendere il primo
        // elemento e perdere il leaf (follow-up review 2026-08-15).
        let path = covering.get(edge)?.as_array()?;
        if path.len() != 1 {
            return None;
        }
        let name = path.first()?.as_str()?.to_owned();
        names.push(name);
    }
    Some(names)
}

/// Ricostruisce lo schema marcando la geometria come `geoarrow.wkb`+`crs` ed
/// ESCLUDENDO le colonne dichiarate come covering in `covering_names`.
///
/// Finding #4 follow-up follow-up review 2026-08-15: la funzione non ha
/// piu' un fallback per-nome implicito. Il caller (`open()`) decide se
/// includere i nomi convenzionali `BBOX_COLS` sulla base di
/// `format_options["bbox_legacy_by_name"]`. `None` significa "non
/// strippare nulla" — cosi' un file esterno con colonne omonime NON
/// perde dati.
fn retag_schema(
    schema: &Schema,
    geom_name: &str,
    crs: &ResolvedCrs,
    covering_names: Option<&[String]>,
) -> SchemaRef {
    let is_internal = |name: &str| -> bool {
        covering_names.is_some_and(|names| names.iter().any(|declared| declared == name))
    };
    let fields: Vec<Field> = schema
        .fields()
        .iter()
        .filter(|f| !is_internal(f.name()))
        .map(|f| {
            if f.name() == geom_name {
                let mut md = f.metadata().clone();
                md.insert(
                    ARROW_EXTENSION_NAME_KEY.to_owned(),
                    GEOARROW_WKB_EXTENSION.to_owned(),
                );
                if let Some(id) = &crs.id {
                    md.insert(GEO_CRS_KEY.to_owned(), id.clone());
                }
                f.as_ref().clone().with_metadata(md)
            } else {
                f.as_ref().clone()
            }
        })
        .collect();
    Arc::new(Schema::new_with_metadata(fields, schema.metadata().clone()))
}

/// Trova la colonna geometria (`geoarrow.wkb`) in uno schema in scrittura.
fn geometry_field(schema: &Schema) -> Result<(usize, String, Option<String>)> {
    for (i, f) in schema.fields().iter().enumerate() {
        if plenora_io_model::geometry::is_geometry_field(f) {
            let crs = f.metadata().get(GEO_CRS_KEY).cloned();
            return Ok((i, f.name().clone(), crs));
        }
    }
    Err(fmt_err(
        "nessuna colonna geometria geoarrow.wkb nel contratto",
    ))
}

fn geometry_type_name(geometry_type: GeometryType) -> Result<&'static str> {
    match geometry_type {
        GeometryType::Point => Ok("Point"),
        GeometryType::LineString => Ok("LineString"),
        GeometryType::Polygon => Ok("Polygon"),
        GeometryType::MultiPoint => Ok("MultiPoint"),
        GeometryType::MultiLineString => Ok("MultiLineString"),
        GeometryType::MultiPolygon => Ok("MultiPolygon"),
        GeometryType::GeometryCollection => Ok("GeometryCollection"),
        other => Err(fmt_err(format!(
            "tipo geometrico {} non supportato dal profilo GeoParquet corrente",
            other.canonical_name()
        ))),
    }
}

fn geometry_type_label(
    geometry_type: GeometryType,
    dimensions: CoordinateDimensions,
) -> Result<String> {
    let suffix = match dimensions {
        CoordinateDimensions::Xy => "",
        CoordinateDimensions::Xyz => " Z",
        CoordinateDimensions::Xym => " M",
        CoordinateDimensions::Xyzm => " ZM",
        CoordinateDimensions::Unknown => return Err(fmt_err("dimensionalità WKB ignota")),
    };
    Ok(format!("{}{suffix}", geometry_type_name(geometry_type)?))
}

fn accumulate_geometry_types(
    col: &dyn Array,
    out: &mut BTreeSet<(GeometryType, CoordinateDimensions)>,
    limits: &WkbLimits,
) -> Result<()> {
    if let Some(a) = col.as_any().downcast_ref::<BinaryArray>() {
        for i in 0..a.len() {
            if !a.is_null(i) {
                let inspection = inspect_wkb(a.value(i), limits)?;
                out.insert((inspection.geometry_type, inspection.dimensions));
            }
        }
    } else if let Some(a) = col.as_any().downcast_ref::<LargeBinaryArray>() {
        for i in 0..a.len() {
            if !a.is_null(i) {
                let inspection = inspect_wkb(a.value(i), limits)?;
                out.insert((inspection.geometry_type, inspection.dimensions));
            }
        }
    }
    Ok(())
}

fn build_geo_metadata(
    geom_name: &str,
    types: &BTreeSet<(GeometryType, CoordinateDimensions)>,
    crs: Option<&str>,
) -> Result<String> {
    let mut geometry_types = types
        .iter()
        .map(|(geometry_type, dimensions)| geometry_type_label(*geometry_type, *dimensions))
        .collect::<Result<Vec<_>>>()?;
    // Mantiene l'ordine lessicografico emesso dal precedente BTreeSet<String>:
    // l'ottimizzazione non deve cambiare neppure incidentalmente il metadato.
    geometry_types.sort_unstable();
    // Finding #4: covering GeoParquet 1.1 dichiarato in modo esplicito. Il
    // lettore usa questa dichiarazione per identificare le colonne bbox
    // interne invece di dipendere dai soli nomi. La forma segue lo schema
    // pubblico `covering.bbox.<edge>` di GeoParquet
    // (https://geoparquet.org/releases/v1.1.0/) ed e' additiva rispetto ai
    // consumer che ignorano l'attributo.
    let mut column = serde_json::json!({
        "encoding": "WKB",
        "geometry_types": geometry_types,
        "covering": {
            "bbox": {
                "xmin": [BBOX_COLS[0]],
                "ymin": [BBOX_COLS[1]],
                "xmax": [BBOX_COLS[2]],
                "ymax": [BBOX_COLS[3]],
            }
        },
    });
    // crs "AUTH:CODE" -> {"id":{authority,code}}, altrimenti null.
    if let Some(id) = crs {
        if let Some((auth, code)) = id.split_once(':') {
            let code_val: serde_json::Value = code
                .parse::<i64>()
                .map_or_else(|_| serde_json::Value::from(code), serde_json::Value::from);
            column["crs"] = serde_json::json!({"id": {"authority": auth, "code": code_val}});
        }
    }
    let mut columns = HashMap::new();
    columns.insert(geom_name.to_owned(), column);
    Ok(serde_json::json!({
        "version": "1.0.0",
        "primary_column": geom_name,
        "columns": columns,
    })
    .to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use arrow_array::Int64Array;
    use arrow_schema::DataType;
    use geo_types::{Geometry, Point};
    use plenora_io_core::request::{BatchTarget, ReadScope};
    use plenora_io_core::WriteLayer;
    use plenora_io_model::wkb::{
        encode_wkb, to_wkb, WkbCoordinate, WkbFlavor, WkbGeometry, WkbValue,
    };
    use plenora_io_model::CancellationToken;

    /// I file che facevano panicare arrow decodificando lo schema devono
    /// uscire come errore del driver, non abbattere il processo. Un `.parquet`
    /// ci arriva perche' il footer puo' portare `ARROW:schema`, che viene
    /// decodificato dallo stesso codice dell'IPC.
    ///
    /// Il target di fuzzing non puo' verificarlo: `libfuzzer-sys` installa un
    /// panic hook che chiama `abort()` prima dell'unwinding, quindi
    /// `catch_unwind` non entra mai in gioco e il target continua a segnalare
    /// un crash anche con la barriera al suo posto. Fuori dal fuzzer
    /// l'unwinding e' quello di default — nel workspace non c'e' alcun
    /// `panic = "abort"` — e la barriera si osserva.
    #[test]
    fn un_parquet_che_fa_panicare_arrow_diventa_un_errore_del_driver() {
        for nome in [
            "arrow-schema-che-fa-panicare.parquet",
            "arrow-schema-che-fa-panicare-2.parquet",
        ] {
            let seme = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../fuzz/seeds/geoparquet_reader")
                .join(nome);
            match GeoParquetDriver.open(Source::Path(seme), &ReadOptions::default()) {
                Ok(_) => panic!("{nome}: il file doveva essere rifiutato"),
                Err(errore) => assert!(
                    errore.to_string().contains("arrow in panico"),
                    "{nome}: la barriera non e' scattata: {errore}"
                ),
            }
        }
    }

    fn geometry_field_meta(crs: &str) -> HashMap<String, String> {
        let mut m = HashMap::new();
        m.insert(
            ARROW_EXTENSION_NAME_KEY.to_owned(),
            GEOARROW_WKB_EXTENSION.to_owned(),
        );
        m.insert(GEO_CRS_KEY.to_owned(), crs.to_owned());
        m
    }

    // Scrive un Parquet minimo senza metadata `geo` (simula un file legacy
    // o esterno) con colonne: geometry (WKB pass-through), i 4 nomi
    // convenzionali `_bbox_*` popolati con f64, e un attributo utente
    // `id`. Il file NON dichiara covering `GeoParquet` 1.1: il driver deve
    // trattare le `_bbox_*` come attributi utente per default.
    // `minx`/`miny`/`maxx`/`maxy` sono le componenti canoniche di un
    // bounding box: rinominarle per soddisfare `similar_names` peggiorerebbe
    // la leggibilita' del test.
    #[allow(clippy::similar_names)]
    fn write_parquet_without_covering_metadata(path: &std::path::Path) {
        use arrow_array::Float64Array;
        let wkb: Vec<u8> = to_wkb(&Geometry::Point(Point::new(1.0, 2.0))).unwrap();
        let geom = BinaryArray::from(vec![Some(wkb.as_slice()), Some(wkb.as_slice())]);
        let minx = Float64Array::from(vec![1.0, 2.0]);
        let miny = Float64Array::from(vec![1.0, 2.0]);
        let maxx = Float64Array::from(vec![1.0, 2.0]);
        let maxy = Float64Array::from(vec![1.0, 2.0]);
        let ids = Int64Array::from(vec![1_i64, 2]);
        let schema = Arc::new(Schema::new(vec![
            Field::new("geometry", DataType::Binary, false),
            Field::new(BBOX_COLS[0], DataType::Float64, true),
            Field::new(BBOX_COLS[1], DataType::Float64, true),
            Field::new(BBOX_COLS[2], DataType::Float64, true),
            Field::new(BBOX_COLS[3], DataType::Float64, true),
            Field::new("id", DataType::Int64, false),
        ]));
        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![
                Arc::new(geom),
                Arc::new(minx),
                Arc::new(miny),
                Arc::new(maxx),
                Arc::new(maxy),
                Arc::new(ids),
            ],
        )
        .unwrap();
        let file = File::create(path).unwrap();
        let mut writer = ArrowWriter::try_new(file, schema, None).unwrap();
        writer.write(&batch).unwrap();
        writer.close().unwrap();
    }

    #[test]
    fn legacy_bbox_names_are_preserved_by_default() {
        // Finding #4 follow-up follow-up review 2026-08-15: senza
        // metadata `covering.bbox` e senza opt-in, le colonne
        // `_bbox_minx/miny/maxx/maxy` devono restare esposte come dati
        // utente. Prima del fix il driver le nascondeva silenziosamente,
        // perdendo dati per i consumer che le usano legittimamente.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("legacy.parquet");
        write_parquet_without_covering_metadata(&path);

        let dataset = GeoParquetDriver
            .open(Source::Path(path), &ReadOptions::default())
            .unwrap();
        let contract_schema = &dataset.layers()[0].contract.schema;
        // Tutte e 4 le colonne bbox restano visibili con i nomi originali.
        for name in BBOX_COLS {
            assert!(
                contract_schema.index_of(name).is_ok(),
                "colonna {name} deve restare esposta senza opt-in"
            );
        }
        // La colonna id resta visibile e la geometria resta la prima.
        assert!(contract_schema.index_of("id").is_ok());
        assert!(contract_schema.index_of("geometry").is_ok());
    }

    #[test]
    fn legacy_bbox_names_are_hidden_with_explicit_opt_in() {
        // Simmetrica del test precedente: chi ha davvero un file scritto
        // dal writer plenora-io pre-fix (senza covering metadata) puo'
        // riattivare il vecchio comportamento via format_option esplicito.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("legacy_optin.parquet");
        write_parquet_without_covering_metadata(&path);

        let mut opts = ReadOptions::default();
        opts.format_options
            .insert("bbox_legacy_by_name".to_owned(), "true".to_owned());
        let dataset = GeoParquetDriver.open(Source::Path(path), &opts).unwrap();
        let contract_schema = &dataset.layers()[0].contract.schema;
        // Con opt-in le 4 colonne bbox sono nascoste (fallback legacy attivo).
        for name in BBOX_COLS {
            assert!(
                contract_schema.index_of(name).is_err(),
                "colonna {name} deve essere nascosta con bbox_legacy_by_name=true"
            );
        }
        assert!(contract_schema.index_of("id").is_ok());
        assert!(contract_schema.index_of("geometry").is_ok());
    }

    #[test]
    fn default_crs_is_crs84_with_longitude_latitude_axis_order() {
        let crs = crs_from(None, "geometry").unwrap();
        assert_eq!(crs.id.as_deref(), Some("OGC:CRS84"));
        assert_eq!(
            crs.axis_order,
            plenora_io_model::crs::AxisOrder::LongitudeLatitude
        );
    }

    #[test]
    fn writer_crs_id_comes_from_the_contract_without_legacy_field_metadata() {
        let geometry = GeometryColumnContract::wkb_xy(
            FieldId(0),
            "geometry",
            ResolvedCrs::new(Some("EPSG:3003".to_owned()), CrsKind::Projected, None),
            true,
        );

        assert_eq!(
            crs_meta_for_write(Some(&geometry), None).as_deref(),
            Some("EPSG:3003")
        );
    }

    #[test]
    fn projjson_without_identifier_is_a_typed_unresolved_crs() {
        let geo = serde_json::json!({
            "columns": {
                "geometry": {
                    "crs": {
                        "type": "ProjectedCRS",
                        "name": "survey-grid-secret"
                    }
                }
            }
        });
        let error = crs_from(Some(&geo), "geometry").unwrap_err();
        assert_eq!(error.code, plenora_io_model::IoErrorCode::CrsUnresolved);
        assert_eq!(error.driver.as_deref(), Some("geoparquet"));
        assert!(!error.to_string().contains("survey-grid-secret"));
    }

    #[test]
    fn pruning_predicates_preserve_integer_precision_and_fail_open() {
        let exact = 9_007_199_254_740_993_i64;
        assert_eq!(
            parse_opaque_predicate("id = 9007199254740993"),
            Some((
                "id".to_owned(),
                PruningComparison::Equal,
                PruningScalar::Int64(exact),
            ))
        );
        assert_eq!(
            range_matches(
                NumericRange::Int64(exact, exact),
                PruningComparison::Equal,
                PruningScalar::Int64(exact),
            ),
            Some(true)
        );
        // 2^53+1 NON è rappresentabile in f64: la perdita di precisione è
        // esattamente cio' che il test verifica (domini misti → fail-open).
        #[allow(clippy::cast_precision_loss)]
        let inexact = exact as f64;
        assert_eq!(
            range_matches(
                NumericRange::Int64(exact, exact),
                PruningComparison::Equal,
                PruningScalar::Float64(inexact),
            ),
            None,
            "domini numerici diversi devono tenere il row group"
        );
        assert_eq!(
            range_matches(
                NumericRange::Float64(0.0, 1.0),
                PruningComparison::GreaterThan,
                PruningScalar::Float64(f64::NAN),
            ),
            None,
            "un literal non finito deve tenere il row group"
        );
        assert!(parse_opaque_predicate("id > NaN").is_none());
        assert!(parse_opaque_predicate("espressione arbitraria").is_none());
    }

    #[test]
    fn round_trip_file_recordbatch_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("out.parquet");

        // RecordBatch con colonna geometria geoarrow.wkb + una colonna int.
        let wkb: Vec<u8> = to_wkb(&Geometry::Point(Point::new(12.5, 45.9))).unwrap();
        let geom = BinaryArray::from(vec![Some(wkb.as_slice()), Some(wkb.as_slice())]);
        let ids = Int64Array::from(vec![1i64, 2]);
        let schema = Arc::new(Schema::new(vec![
            Field::new("geometry", DataType::Binary, true)
                .with_metadata(geometry_field_meta("EPSG:4326")),
            Field::new("id", DataType::Int64, false),
        ]));
        let batch =
            RecordBatch::try_new(schema.clone(), vec![Arc::new(geom), Arc::new(ids)]).unwrap();

        // create -> write -> finish
        let driver = GeoParquetDriver;
        let plan = WritePlan {
            layers: vec![WriteLayer {
                name: "l".to_owned(),
                contract: DataContract {
                    schema,
                    geometry: None,
                },
            }],
        };
        let mut writer = driver
            .create(Sink::Path(path.clone()), &plan, &WriteOptions::default())
            .unwrap();
        writer.write(&batch).unwrap();
        let published = writer.finish().unwrap();
        assert!(published.bytes > 0);

        // open -> read back
        let ds = driver
            .open(Source::Path(path), &ReadOptions::default())
            .unwrap();
        assert_eq!(ds.layers().len(), 1);
        let layer = &ds.layers()[0];
        let geom_c = layer.contract.geometry.as_ref().unwrap();
        assert_eq!(geom_c.name, "geometry");
        assert_eq!(geom_c.crs.id(), Some("EPSG:4326"));

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
        // La geometria è marcata geoarrow.wkb nello schema effettivo.
        let field = out.schema().field_with_name("geometry").unwrap().clone();
        assert!(plenora_io_model::geometry::is_geometry_field(&field));
        // I byte WKB sono pass-through identici.
        let col = out
            .column_by_name("geometry")
            .unwrap()
            .as_any()
            .downcast_ref::<BinaryArray>()
            .unwrap();
        assert_eq!(col.value(0), wkb.as_slice());
        assert!(reader.next_batch().unwrap().is_none());
    }

    #[test]
    fn geoparquet_preserves_xyz_and_emits_dimensional_metadata() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("z.parquet");
        let wkb = encode_wkb(
            &WkbGeometry {
                value: WkbValue::Point(WkbCoordinate {
                    x: 12.5,
                    y: 45.9,
                    z: Some(123.0),
                    m: None,
                }),
                dimensions: CoordinateDimensions::Xyz,
                srid: None,
            },
            WkbFlavor::Iso,
        )
        .unwrap();
        let schema: SchemaRef = Arc::new(Schema::new(vec![Field::new(
            "geometry",
            DataType::Binary,
            true,
        )
        .with_metadata(geometry_field_meta("EPSG:4326"))]));
        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![Arc::new(BinaryArray::from(vec![Some(wkb.as_slice())]))],
        )
        .unwrap();
        let mut geometry = GeometryColumnContract::wkb_passthrough(
            FieldId(0),
            "geometry",
            ResolvedCrs::new(Some("EPSG:4326".to_owned()), CrsKind::Geographic, None),
            true,
        );
        geometry.dimensions = CoordinateDimensions::Xyz;
        geometry.set_exact_geometry_types(vec![GeometryType::Point]);
        let plan = WritePlan {
            layers: vec![WriteLayer {
                name: "z".to_owned(),
                contract: DataContract {
                    schema,
                    geometry: Some(geometry),
                },
            }],
        };
        let driver = GeoParquetDriver;
        let mut writer = driver
            .create(Sink::Path(path.clone()), &plan, &WriteOptions::default())
            .unwrap();
        writer.write(&batch).unwrap();
        writer.finish().unwrap();

        assert_eq!(wkb_bbox(&wkb), Some([12.5, 45.9, 12.5, 45.9]));
        let dataset = driver
            .open(Source::Path(path), &ReadOptions::default())
            .unwrap();
        let geometry = dataset.layers()[0].contract.geometry.as_ref().unwrap();
        assert_eq!(geometry.dimensions, CoordinateDimensions::Xyz);
        assert_eq!(geometry.geometry_types, vec![GeometryType::Point]);
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
            .column_by_name("geometry")
            .unwrap()
            .as_any()
            .downcast_ref::<BinaryArray>()
            .unwrap();
        assert_eq!(geometry_array.value(0), wkb);
    }

    /// Conformità alla spec `GeoParquet` 1.0.0, verificata riaprendo il file
    /// GREZZO col crate `parquet` (indipendente dal nostro reader): metadato
    /// `geo` file-level + colonna geometria fisicamente `BYTE_ARRAY` (WKB).
    #[test]
    fn geoparquet_spec_conformance() {
        use parquet::basic::Type as PhysicalType;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("conf.parquet");

        let wkb: Vec<u8> = to_wkb(&Geometry::Point(Point::new(9.19, 45.46))).unwrap();
        let schema = Arc::new(Schema::new(vec![
            Field::new("geometry", DataType::Binary, true)
                .with_metadata(geometry_field_meta("EPSG:4326")),
            Field::new("id", DataType::Int64, false),
        ]));
        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![
                Arc::new(BinaryArray::from(vec![Some(wkb.as_slice())])),
                Arc::new(Int64Array::from(vec![7i64])),
            ],
        )
        .unwrap();

        let driver = GeoParquetDriver;
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

        // Riapri il file grezzo col crate parquet.
        let builder = ParquetRecordBatchReaderBuilder::try_new(File::open(&path).unwrap()).unwrap();

        // 1) metadato file-level "geo" conforme.
        let kv = builder
            .metadata()
            .file_metadata()
            .key_value_metadata()
            .expect("key_value_metadata assenti");
        let geo_raw = kv
            .iter()
            .find(|e| e.key == "geo")
            .and_then(|e| e.value.clone())
            .expect("metadato 'geo' assente (non è GeoParquet)");
        let geo: serde_json::Value = serde_json::from_str(&geo_raw).unwrap();

        assert_eq!(geo["version"].as_str(), Some("1.0.0"));
        assert_eq!(geo["primary_column"].as_str(), Some("geometry"));
        let col = &geo["columns"]["geometry"];
        assert_eq!(col["encoding"].as_str(), Some("WKB"));
        let types: Vec<&str> = col["geometry_types"]
            .as_array()
            .expect("geometry_types deve essere un array")
            .iter()
            .filter_map(|v| v.as_str())
            .collect();
        assert!(
            types.contains(&"Point"),
            "geometry_types deve contenere 'Point', era {types:?}"
        );
        assert_eq!(col["crs"]["id"]["authority"].as_str(), Some("EPSG"));
        assert_eq!(col["crs"]["id"]["code"].as_i64(), Some(4326));

        // 2) la colonna geometria è fisicamente BYTE_ARRAY (WKB) nel Parquet.
        let pschema = builder.metadata().file_metadata().schema_descr();
        let geom_col = (0..pschema.num_columns())
            .map(|i| pschema.column(i))
            .find(|c| c.name() == "geometry")
            .expect("colonna 'geometry' assente nello schema Parquet");
        assert_eq!(geom_col.physical_type(), PhysicalType::BYTE_ARRAY);
    }

    #[test]
    fn projection_pushdown_reads_only_requested() {
        use plenora_io_model::contract::FieldId;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("proj.parquet");
        let wkb: Vec<u8> = to_wkb(&Geometry::Point(Point::new(1.0, 2.0))).unwrap();
        let schema = Arc::new(Schema::new(vec![
            Field::new("geometry", DataType::Binary, true)
                .with_metadata(geometry_field_meta("EPSG:4326")),
            Field::new("id", DataType::Int64, false),
            Field::new("name", DataType::Utf8, false),
        ]));
        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![
                Arc::new(BinaryArray::from(vec![Some(wkb.as_slice())])),
                Arc::new(Int64Array::from(vec![7i64])),
                Arc::new(arrow_array::StringArray::from(vec!["x"])),
            ],
        )
        .unwrap();

        let driver = GeoParquetDriver;
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

        // Proietta SOLO la colonna "id" (indice 1) in modalità Required.
        let ds = driver
            .open(Source::Path(path), &ReadOptions::default())
            .unwrap();
        let mut reader = ds
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
        // Il contratto del reader riflette la projection (1 colonna, niente geometria).
        assert_eq!(reader.contract().contract.schema.fields().len(), 1);
        assert!(reader.contract().contract.geometry.is_none());
        let out = reader.next_batch().unwrap().unwrap();
        assert_eq!(out.num_columns(), 1);
        assert_eq!(out.schema().field(0).name(), "id");
        assert_eq!(
            out.column(0)
                .as_any()
                .downcast_ref::<Int64Array>()
                .unwrap()
                .value(0),
            7
        );
    }

    #[test]
    fn row_group_pruning_skips_blocks() {
        use plenora_io_core::request::PruningPredicate;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("prune.parquet");
        let wkb: Vec<u8> = to_wkb(&Geometry::Point(Point::new(1.0, 2.0))).unwrap();
        let n = 200_000usize; // > 3 row group (65536/row group)
        let schema = Arc::new(Schema::new(vec![
            Field::new("geometry", DataType::Binary, true)
                .with_metadata(geometry_field_meta("EPSG:4326")),
            Field::new("id", DataType::Int64, false),
        ]));
        // `n` è la costante 200_000: il cast a i64 è esatto.
        #[allow(clippy::cast_possible_wrap)]
        let rows = n as i64;
        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![
                Arc::new(BinaryArray::from(
                    (0..n).map(|_| Some(wkb.as_slice())).collect::<Vec<_>>(),
                )),
                Arc::new(Int64Array::from((0..rows).collect::<Vec<_>>())),
            ],
        )
        .unwrap();

        let driver = GeoParquetDriver;
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

        // Pruning "id > 150000": salta i row group con max(id) <= 150000.
        let ds = driver
            .open(Source::Path(path.clone()), &ReadOptions::default())
            .unwrap();
        let mut reader = ds
            .open_layer_reader(&ReadRequest {
                layer: LayerId(0),
                projected_fields: None,
                projection_mode: ProjectionMode::BestEffort,
                pruning_predicate: Some(PruningPredicate::NumericComparison {
                    field: FieldId(1),
                    comparison: PruningComparison::GreaterThan,
                    value: PruningScalar::Int64(150_000),
                }),
                spatial_pruning_hint: None,
                scope: ReadScope::default(),
                batch_target: BatchTarget::default(),
                cancellation: CancellationToken::default(),
            })
            .unwrap();
        let mut total = 0;
        while let Some(b) = reader.next_batch().unwrap() {
            total += b.num_rows();
        }
        // Over-return: legge solo i row group che POSSONO contenere id>150000
        // (meno di tutte le 200k righe, ma tutte le righe matchanti sono incluse).
        assert!(
            total < n,
            "il pruning deve saltare row group, letti {total}"
        );
        assert!(total >= n - 150_000, "under-return vietato, letti {total}");

        let legacy = driver
            .open(Source::Path(path), &ReadOptions::default())
            .unwrap();
        let mut legacy_reader = legacy
            .open_layer_reader(&ReadRequest {
                layer: LayerId(0),
                projected_fields: None,
                projection_mode: ProjectionMode::BestEffort,
                pruning_predicate: Some(PruningPredicate::Opaque("id > 150000".to_owned())),
                spatial_pruning_hint: None,
                scope: ReadScope::default(),
                batch_target: BatchTarget::default(),
                cancellation: CancellationToken::default(),
            })
            .unwrap();
        let mut legacy_total = 0;
        while let Some(batch) = legacy_reader.next_batch().unwrap() {
            legacy_total += batch.num_rows();
        }
        assert_eq!(
            legacy_total, total,
            "il formato Opaque v1 deve restare compatibile"
        );
    }

    #[test]
    fn spatial_pruning_skips_blocks() {
        use plenora_io_core::request::Bbox;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sp.parquet");
        let n = 200_000usize;
        // Punti con x crescente (0..200), y=45 → row group con estensione x diversa.
        // `i` < 200_000 < 2^53: la conversione a f64 è esatta.
        #[allow(clippy::cast_precision_loss)]
        let wkb: Vec<Vec<u8>> = (0..n)
            .map(|i| to_wkb(&Geometry::Point(Point::new(i as f64 * 0.001, 45.0))).unwrap())
            .collect();
        let schema = Arc::new(Schema::new(vec![
            Field::new("geometry", DataType::Binary, true)
                .with_metadata(geometry_field_meta("EPSG:4326")),
            Field::new("id", DataType::Int64, false),
        ]));
        // `n` è la costante 200_000: il cast a i64 è esatto.
        #[allow(clippy::cast_possible_wrap)]
        let rows = n as i64;
        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![
                Arc::new(BinaryArray::from(
                    wkb.iter().map(|w| Some(w.as_slice())).collect::<Vec<_>>(),
                )),
                Arc::new(Int64Array::from((0..rows).collect::<Vec<_>>())),
            ],
        )
        .unwrap();

        let driver = GeoParquetDriver;
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

        let ds = driver
            .open(Source::Path(path), &ReadOptions::default())
            .unwrap();
        // Il bbox covering NON è esposto: il contratto ha solo geometry + id.
        assert_eq!(ds.layers()[0].contract.schema.fields().len(), 2);

        // Hint spaziale x in [190,210]: interseca solo gli ultimi row group.
        let mut reader = ds
            .open_layer_reader(&ReadRequest {
                layer: LayerId(0),
                projected_fields: None,
                projection_mode: ProjectionMode::BestEffort,
                pruning_predicate: None,
                spatial_pruning_hint: Some(Bbox {
                    minx: 190.0,
                    miny: 40.0,
                    maxx: 210.0,
                    maxy: 50.0,
                }),
                scope: ReadScope::default(),
                batch_target: BatchTarget::default(),
                cancellation: CancellationToken::default(),
            })
            .unwrap();
        let mut total = 0;
        while let Some(b) = reader.next_batch().unwrap() {
            // Il batch NON contiene le colonne bbox interne.
            assert_eq!(b.num_columns(), 2);
            total += b.num_rows();
        }
        // Pruning: legge meno di tutto ma include tutte le ~10000 righe con x in [190,200].
        assert!(
            total < n,
            "spatial pruning deve saltare row group, letti {total}"
        );
        assert!(total >= 10_000, "under-return vietato, letti {total}");
    }

    #[test]
    fn zstd_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("z.parquet");
        let wkb: Vec<u8> = to_wkb(&Geometry::Point(Point::new(12.5, 45.9))).unwrap();
        let schema = Arc::new(Schema::new(vec![
            Field::new("geometry", DataType::Binary, true)
                .with_metadata(geometry_field_meta("EPSG:4326")),
            Field::new("id", DataType::Int64, false),
        ]));
        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![
                Arc::new(BinaryArray::from(vec![Some(wkb.as_slice())])),
                Arc::new(Int64Array::from(vec![42i64])),
            ],
        )
        .unwrap();

        let driver = GeoParquetDriver;
        let plan = WritePlan {
            layers: vec![WriteLayer {
                name: "l".to_owned(),
                contract: DataContract {
                    schema,
                    geometry: None,
                },
            }],
        };
        // Scrive compresso zstd.
        let wopts = WriteOptions {
            durable: false,
            format_options: std::iter::once(("compression".to_owned(), "zstd".to_owned()))
                .collect(),
            ..WriteOptions::default()
        };
        let mut w = driver
            .create(Sink::Path(path.clone()), &plan, &wopts)
            .unwrap();
        w.write(&batch).unwrap();
        w.finish().unwrap();

        // Rilegge il file zstd (prima veniva RIFIUTATO).
        let ds = driver
            .open(Source::Path(path), &ReadOptions::default())
            .unwrap();
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
        assert_eq!(out.num_rows(), 1);
        let col = out
            .column_by_name("geometry")
            .unwrap()
            .as_any()
            .downcast_ref::<BinaryArray>()
            .unwrap();
        assert_eq!(col.value(0), wkb.as_slice());
    }
}
