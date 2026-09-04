//! driver-dxf — DXF ↔ `RecordBatch`. Lettura delle primitive native e delle
//! approssimazioni curve con coordinate WKB XY/XYZ, senza passare da
//! `geo-types`. Il motore esplode ricorsivamente i blocchi INSERT con
//! composizione di trasformazioni, OCS→WCS (algoritmo dell'asse arbitrario),
//! tassellazione fine di ARC/CIRCLE/ELLIPSE/SPLINE(NURBS) e archi bulge nelle
//! polilinee, più SOLID/POLYLINE/POINT/TEXT/MTEXT. La **scrittura** resta
//! `Approximating`: gli attributi, i multipart e gli anelli interni non sono
//! nativi. Il CRS è letto/scritto tramite GEODATA e può essere assunto
//! esplicitamente quando il metadato è assente o non risolvibile.
#![forbid(unsafe_code)]

use std::collections::{BTreeSet, HashMap, HashSet};
use std::fs::File;
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::PathBuf;
use std::sync::Arc;

use arrow_array::{Array, ArrayRef, BinaryArray, RecordBatch, StringArray};
use arrow_schema::{DataType, Field, Schema, SchemaRef};
use dxf::entities::{Entity, EntityType, Insert, LwPolyline, ModelPoint, Polyline, Vertex};
use dxf::enums::AcadVersion;
use dxf::objects::{GeoData, Object, ObjectType};
use dxf::{Block, Drawing, DrawingEntityReader, LwPolylineVertex, Point as DxfPoint, Vector};

mod geometry;
use geometry::{
    tessellate_arc, tessellate_bulge, tessellate_circle, tessellate_ellipse3, tessellate_spline3,
    Transform3,
};

use driver_common::{cell_string, geometry_field, geometry_index};
use plenora_io_core::descriptor::{
    CrsHandling, Direction, Fidelity, FormatDescriptor, ReadMode, ReaderConcurrency, Runtime,
    WriteMode,
};
use plenora_io_core::driver::{
    FormatDriver, FormatWriter, LayerReader, OpenDatasetHandle, Published, ReadOptions, Sink,
    Source, WriteOptions,
};
use plenora_io_core::loss::LossReport;
use plenora_io_core::publish::{create_staged_file, publish_file_atomic_limited};
use plenora_io_core::request::ReadRequest;
use plenora_io_core::{
    check_cancelled, check_cancelled_periodically, read_row_error, validate_write,
    with_write_validation, write_row_rejection, AttributeWriteSupport, CrsDerivation,
    CrsRepresentationCapabilities, CrsRepresentationState, CrsWriteSupport,
    FormatWriteCapabilities, NullabilitySupport, SingleReaderGate, TypeCoercionPolicy, WritePlan,
    SCALAR_TYPES, UTF8_FIELD_NAMES, WKB_XY_XYZ_GEOMETRY,
};
use plenora_io_model::budget::{OperationBudget, SpillLease};
use plenora_io_model::contract::{
    CoordinateDimensions, DataContract, FieldId, GeometryColumnContract, GeometryType,
    LayerContract, LayerId,
};
use plenora_io_model::crs::{CrsKind, RawCrs, ResolvedCrs};
use plenora_io_model::geometry::with_geometry_contract_metadata;
use plenora_io_model::limits::WkbLimits;
use plenora_io_model::wkb::{
    decode_wkb, encode_wkb, inspect_wkb, WkbCoordinate, WkbFlavor, WkbGeometry, WkbValue,
};
use plenora_io_model::{CancellationToken, ErrorPhase};
use plenora_io_model::{NumeroStrutturale, PlenoraIoError, PublicMessage, Result};

const GEOMETRY: &str = "geometry";
/// Segmenti per un giro intero (archi, cerchi, ellissi, bulge).
const ARC_SEGMENTS: usize = 24;
/// Limite di annidamento dei blocchi INSERT (anti-ricorsione patologica).
const MAX_INSERT_DEPTH: usize = 16;
/// Tetto complessivo di entità generate (anti-esplosione da array/annidamenti).
const MAX_ENTITIES: usize = 5_000_000;
const WGS84_ESRI_WKT: &str = "GEOGCS[\"WGS 84\",DATUM[\"D_WGS_1984\",SPHEROID[\"WGS 84\",6378137,298.257223563]],PRIMEM[\"Greenwich\",0],UNIT[\"Degree\",0.0174532925199433],AUTHORITY[\"EPSG\",\"4326\"]]";

fn err(reason: &PublicMessage) -> PlenoraIoError {
    PlenoraIoError::formato_redatto("dxf", reason)
}

fn crs_kind(id: Option<&str>, definition: Option<&str>) -> CrsKind {
    let id = id.unwrap_or_default();
    let definition = definition.unwrap_or_default().to_ascii_uppercase();
    if id == "OGC:CRS84"
        || id == "EPSG:4326"
        || definition.contains("GEOGCS[")
        || definition.contains("GEODCRS[")
    {
        CrsKind::Geographic
    } else if definition.contains("PROJCS[") || definition.contains("PROJCRS[") {
        CrsKind::Projected
    } else {
        CrsKind::Unknown
    }
}

fn digits_after(text: &str, marker: &str) -> Option<String> {
    let start = text.find(marker)? + marker.len();
    let tail = text[start..].trim_start_matches([' ', '"', '\'', ':']);
    let digits: String = tail.chars().take_while(char::is_ascii_digit).collect();
    (!digits.is_empty()).then_some(digits)
}

fn epsg_from_definition(definition: &str) -> Option<String> {
    let upper = definition.to_ascii_uppercase();
    if let Some(code) = digits_after(&upper, "EPSG:") {
        return Some(format!("EPSG:{code}"));
    }
    for marker in ["AUTHORITY[\"EPSG\",", "ID[\"EPSG\","] {
        if let Some(code) = digits_after(&upper, marker) {
            return Some(format!("EPSG:{code}"));
        }
    }
    let mut remaining = upper.as_str();
    while let Some(start) = remaining.find("<ALIAS") {
        remaining = &remaining[start..];
        let end = remaining
            .find("</ALIAS>")
            .map_or(remaining.len(), |index| index + "</ALIAS>".len());
        let alias = &remaining[..end];
        if alias.contains("EPSG") {
            if let Some(code) = digits_after(alias, "ID=") {
                return Some(format!("EPSG:{code}"));
            }
        }
        remaining = &remaining[end..];
    }
    None
}

fn embedded_crs_definition(drawing: &Drawing) -> Option<String> {
    drawing.objects().find_map(|object| match &object.specific {
        ObjectType::GeoData(geodata) => {
            let definition = geodata.coordinate_system_definition.trim();
            (!definition.is_empty()).then(|| definition.to_owned())
        }
        _ => None,
    })
}

fn resolve_dxf_crs(drawing: &Drawing, options: &ReadOptions) -> Result<ResolvedCrs> {
    let Some(definition) = embedded_crs_definition(drawing) else {
        let id = options.assume_crs.clone().ok_or_else(|| {
            PlenoraIoError::crs_redatto(&PublicMessage::Curated(
                "DXF senza GEODATA risolvibile: fornire --assume-crs",
            ))
        })?;
        let kind = crs_kind(Some(&id), None);
        return Ok(ResolvedCrs::new(Some(id), kind, None));
    };
    let embedded_id = epsg_from_definition(&definition).or_else(|| {
        let trimmed = definition.trim();
        (trimmed.eq_ignore_ascii_case("OGC:CRS84")).then(|| "OGC:CRS84".to_owned())
    });
    let id = embedded_id.or_else(|| options.assume_crs.clone());
    let Some(id) = id else {
        let raw = RawCrs::new(definition, None);
        return Err(PlenoraIoError::crs_non_risolto_redatto("dxf", &raw));
    };
    let kind = crs_kind(Some(&id), Some(&definition));
    Ok(ResolvedCrs::new(Some(id), kind, Some(definition)))
}

/// Gli identificatori per cui il writer sintetizza la definizione.
///
/// Una lista sola, usata da `definition_for_write`, che la scrive nel
/// `GEODATA`, e dalla capability `crs_id`, che la dichiara.
pub const CRS_CON_DEFINIZIONE_SINTETIZZATA: &[&str] = &["EPSG:4326", "OGC:CRS84"];

fn definition_for_write(geometry: &GeometryColumnContract) -> Result<String> {
    let resolved = geometry.resolved_crs().ok_or_else(|| {
        PlenoraIoError::crs_redatto(&PublicMessage::Curated(
            "DXF richiede un CRS risolto nel contratto",
        ))
    })?;
    if let Some(definition) = &resolved.definition {
        if !definition.trim().is_empty() {
            return Ok(definition.clone());
        }
    }
    match resolved.id.as_deref() {
        Some(id) if CRS_CON_DEFINIZIONE_SINTETIZZATA.contains(&id) => Ok(WGS84_ESRI_WKT.to_owned()),
        // L'identificativo non esce: viene dal contratto, che chi legge
        // l'errore ha gia'.
        Some(_) => Err(PlenoraIoError::crs_redatto(&PublicMessage::Curated(
            "DXF richiede la definizione WKT/XML del CRS, non il solo authority id",
        ))),
        None => Err(PlenoraIoError::crs_redatto(&PublicMessage::Curated(
            "DXF richiede una definizione WKT/XML del CRS",
        ))),
    }
}

fn embed_dxf_crs(drawing: &mut Drawing, geometry: &GeometryColumnContract) -> Result<()> {
    let definition = definition_for_write(geometry)?;
    drawing.header.version = AcadVersion::R2010;
    let geodata = GeoData {
        coordinate_system_definition: definition,
        ..Default::default()
    };
    drawing.add_object(Object::new(ObjectType::GeoData(geodata)));
    Ok(())
}

static DESCRIPTOR: FormatDescriptor = FormatDescriptor::const_new(
    "dxf",
    Direction::Bidirectional,
    ReadMode::StreamingSequential,
    // INV-7: il parser riversa l'intera sorgente in uno spool all'apertura.
    plenora_io_core::NativeReadMode::Materialized,
    // Il drenaggio e lo spool sono dell'adapter comune, non di
    // questo driver: `BudgetedReader` li impone a tutti.
    plenora_io_core::DeliverySemantics::OperationAtomic,
    plenora_io_core::BufferingStrategy::AdaptiveMemoryThenDisk,
    plenora_io_core::DeterminismLevel::Semantic,
    Some(WriteMode::Buffered),
    Some(plenora_io_core::DeterminismLevel::Semantic),
    false,
    false,
    ReaderConcurrency::SingleActiveReader,
    plenora_io_core::ProjectionSupport::None,
    plenora_io_core::PredicatePruningSupport::None,
    plenora_io_core::SpatialPruningSupport::None,
    CrsHandling::Embedded,
    Fidelity::Approximating,
    Runtime::PureRust,
    // `hostile_input_hardened`: non dichiarato: il parser DXF non e' passato da S12. Ha le sue difese
    // -- barriera anti-panic e tetti a valle -- e questa capability non le
    // riassume.
    false,
    // `spec_version_supported`: il formato non si versiona in un modo che
    // il driver possa dichiarare per intero.
    None,
    Some(FormatWriteCapabilities {
        field_names: UTF8_FIELD_NAMES,
        allowed_types: SCALAR_TYPES,
        type_coercion: TypeCoercionPolicy::LossReported,
        attributes: AttributeWriteSupport::LossReported,
        geometry: WKB_XY_XYZ_GEOMETRY,
        crs: CrsWriteSupport::Embedded,
        crs_representations: CrsRepresentationCapabilities::new(
            // L'identificatore si rilegge dal GEODATA incorporato. A
            // differenza dello Shapefile, un piano che non porta ne' WKT ne'
            // un identificatore sintetizzabile viene **rifiutato** invece che
            // scritto senza: quando la scrittura avviene, la definizione c'e'.
            CrsRepresentationState::Derived(CrsDerivation::FromDefinition {
                synthesized_for: CRS_CON_DEFINIZIONE_SINTETIZZATA,
            }),
            CrsRepresentationState::Absent,
            CrsRepresentationState::Preserved,
        ),
        nullability: NullabilitySupport::FormatDefined,
        multi_layer: false,
    }),
    // Il driver non interpreta alcuna format_option (L0.7): l'elenco vuoto
    // e' l'affermazione che qualunque chiave e' sconosciuta, non un'omissione.
    plenora_io_model::format_options::SchemaOpzioniFormato::VUOTO,
    1,
    5,
    9,
);

pub struct DxfDriver;

impl FormatDriver for DxfDriver {
    fn descriptor(&self) -> &FormatDescriptor {
        &DESCRIPTOR
    }

    fn open(&self, source: Source, mut opts: ReadOptions) -> Result<Box<dyn OpenDatasetHandle>> {
        let path = plenora_io_core::preflight_source(self.descriptor(), source, &mut opts)?;
        let mut stream = DrawingEntityReader::load_file(&path).map_err(|_| {
            err(&PublicMessage::Curated(
                "apertura progressiva del DXF fallita",
            ))
        })?;
        check_cancelled(opts.cancellation(), ErrorPhase::Read)?;
        let mut walker = Walker::new(
            stream.drawing(),
            DxfQuote::from_read_options(&opts),
            opts.cancellation(),
        )?;
        let mut stats = DxfContractStats::default();
        let mut spool_writer = DxfSpoolWriter::new(opts.max_input_bytes(), opts.budget().clone());
        let mut source_index = 0_u64;
        while let Some(entity) = stream.next_entity().map_err(|_| {
            read_row_error(
                err(&PublicMessage::Curated("lettura di un'entità DXF fallita")),
                None,
                "dxf.entity_decode_failed",
                Some(GEOMETRY),
            )
        })? {
            opts.ensure_active()?;
            let mut visiting = HashSet::new();
            walker
                .walk_entity(&entity, Transform3::IDENTITY, "0", 0, &mut visiting)
                .map_err(|error| {
                    read_row_error(
                        error,
                        Some(source_index),
                        "dxf.entity_not_representable",
                        Some(GEOMETRY),
                    )
                })?;
            stats.observe(&walker, opts.cancellation())?;
            spool_writer.write_and_clear(&mut walker, opts.cancellation())?;
            source_index = source_index.checked_add(1).ok_or_else(|| {
                PlenoraIoError::limite_redatto(&PublicMessage::Curated("troppe entita DXF"))
            })?;
        }
        let spool = spool_writer.finish()?;
        let drawing = stream.finish().map_err(|_| {
            err(&PublicMessage::Curated(
                "chiusura della scansione DXF fallita",
            ))
        })?;
        let crs = resolve_dxf_crs(&drawing, &opts)?;
        let loss = walker.loss.clone();
        let contract = dxf_contract(crs, stats.dimensions(), stats.geometry_types)?;
        let name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("layer")
            .to_owned();
        Ok(plenora_io_core::with_read_budget(
            Box::new(DxfDataset {
                layers: vec![LayerContract {
                    id: LayerId(0),
                    name,
                    contract,
                }],
                spool,
                rows: walker.emitted_rows,
                wkb_limits: opts.wkb_limits(),
                loss,
                reader_gate: SingleReaderGate::new(DESCRIPTOR.id()),
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
        let Sink::Path(path) = sink;
        if path.exists() {
            return Err(PlenoraIoError::destinazione_esistente());
        }
        if !path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("dxf"))
        {
            return Err(PlenoraIoError::non_supportato_redatto(
                &PublicMessage::Curated("l'output deve avere estensione .dxf"),
            ));
        }
        if plan.layers.len() != 1 {
            return Err(PlenoraIoError::non_supportato_redatto(
                &PublicMessage::Curated("DXF: un solo layer per file"),
            ));
        }
        let mut drawing = Drawing::new();
        let geometry = plan.layers[0].contract.geometry.as_ref().ok_or_else(|| {
            err(&PublicMessage::Curated(
                "DXF richiede un contratto geometrico esplicito con CRS risolto",
            ))
        })?;
        embed_dxf_crs(&mut drawing, geometry)?;
        with_write_validation(
            Box::new(DxfWriterState {
                drawing,
                path,
                durable: opts.durable,
                loss: LossReport::default(),
                dropped_cols: Vec::new(),
                rows: 0,
                input_total: None,
                first: true,
                wkb_limits: opts.wkb_limits(),
                max_output_bytes: opts.max_output_bytes(),
            }),
            self.descriptor(),
            plan,
            opts,
        )
    }
}

// --- scrittura (fedeltà Approximating: solo geometria + layer) -------------

struct DxfWriterState {
    drawing: Drawing,
    path: PathBuf,
    durable: bool,
    loss: LossReport,
    dropped_cols: Vec<String>,
    rows: u64,
    input_total: Option<u64>,
    first: bool,
    wkb_limits: WkbLimits,
    max_output_bytes: u64,
}

struct BoundedOutput<W> {
    inner: W,
    written: u64,
    limit: u64,
    exceeded: bool,
}

impl<W> BoundedOutput<W> {
    const fn new(inner: W, limit: u64) -> Self {
        Self {
            inner,
            written: 0,
            limit,
            exceeded: false,
        }
    }

    const fn exceeded(&self) -> bool {
        self.exceeded
    }

    fn into_inner(self) -> W {
        self.inner
    }
}

impl<W: Write> Write for BoundedOutput<W> {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        let requested = buffer.len() as u64;
        if requested > self.limit.saturating_sub(self.written) {
            self.exceeded = true;
            return Err(std::io::Error::other("limite output DXF superato"));
        }
        let written = self.inner.write(buffer)?;
        self.written = self.written.saturating_add(written as u64);
        Ok(written)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

impl FormatWriter for DxfWriterState {
    fn declare_input_total(&mut self, layer: LayerId, total: u64) -> Result<()> {
        if layer.0 != 0 {
            return Err(PlenoraIoError::non_supportato_redatto(
                &PublicMessage::Curated("DXF supporta un solo layer"),
            ));
        }
        self.input_total = Some(total);
        Ok(())
    }

    fn write(&mut self, batch: &RecordBatch) -> Result<()> {
        let schema = batch.schema();
        let geom_idx = geometry_index(&schema).ok_or_else(|| {
            err(&PublicMessage::Curated(
                "nessuna colonna geometria geoarrow.wkb",
            ))
        })?;
        let geom_col = batch
            .column(geom_idx)
            .as_any()
            .downcast_ref::<BinaryArray>()
            .ok_or_else(|| err(&PublicMessage::Curated("colonna geometria non binaria")))?;
        let layer_idx = schema.index_of("layer").ok();
        if self.first {
            for (i, f) in schema.fields().iter().enumerate() {
                if i != geom_idx && Some(i) != layer_idx {
                    self.dropped_cols.push(f.name().clone());
                }
            }
            self.first = false;
        }
        let limits = self.wkb_limits;
        let mut decoded = Vec::with_capacity(batch.num_rows());
        let mut rejections = Vec::new();
        for row in 0..batch.num_rows() {
            if geom_col.is_null(row) {
                rejections.push((row, "dxf.null_geometry_unsupported", GEOMETRY));
                decoded.push(None);
                continue;
            }
            let geometry = decode_wkb(geom_col.value(row), &limits)?;
            let cause = if geometry.srid.is_some() {
                Some("dxf.embedded_srid_unsupported")
            } else if matches!(
                geometry.dimensions,
                CoordinateDimensions::Xym
                    | CoordinateDimensions::Xyzm
                    | CoordinateDimensions::Unknown
            ) {
                Some("dxf.coordinate_dimensions_unsupported")
            } else {
                dxf_geometry_rejection_cause(&geometry.value)
            };
            if let Some(cause) = cause {
                rejections.push((row, cause, GEOMETRY));
            }
            decoded.push(Some(geometry));
        }
        let mut layers = Vec::with_capacity(batch.num_rows());
        for row in 0..batch.num_rows() {
            if let Ok(layer) = layer_idx
                .map(|index| cell_string(batch.column(index), row))
                .transpose()
            {
                layers.push(layer.flatten());
            } else {
                rejections.push((row, "dxf.layer_not_representable", "layer"));
                layers.push(None);
            }
        }
        if !rejections.is_empty() {
            return Err(write_row_rejection(
                "dxf",
                self.rows,
                batch.num_rows(),
                &rejections,
                self.input_total,
            ));
        }
        for (geometry, layer) in decoded.iter().zip(&layers) {
            let g = geometry
                .as_ref()
                .ok_or_else(|| err(&PublicMessage::Curated("geometria DXF validata ma assente")))?;
            add_geometry(&mut self.drawing, g, layer.as_deref(), &mut self.loss)?;
        }
        self.rows = self
            .rows
            .checked_add(u64::try_from(batch.num_rows()).map_err(|_| {
                PlenoraIoError::limite_redatto(&PublicMessage::Curated("troppe righe DXF"))
            })?)
            .ok_or_else(|| {
                PlenoraIoError::limite_redatto(&PublicMessage::Curated("troppe righe DXF"))
            })?;
        Ok(())
    }

    fn finish(mut self: Box<Self>) -> Result<Published> {
        // Gli attributi non rappresentabili in DXF sono dichiarati come perdita.
        for c in &self.dropped_cols {
            self.loss.record(
                &format!("attributo non rappresentato in DXF: {c}"),
                self.rows,
            );
        }
        let mut temp = create_staged_file(&self.path)?;
        let buffered = BufWriter::with_capacity(1024 * 1024, temp.as_file_mut());
        let mut output = BoundedOutput::new(buffered, self.max_output_bytes);
        let save_result = self.drawing.save(&mut output);
        if output.exceeded() {
            return Err(PlenoraIoError::limite_redatto(&PublicMessage::CuratedWith(
                "output DXF oltre il limite di byte:",
                NumeroStrutturale::Limite(self.max_output_bytes),
            )));
        }
        save_result.map_err(|_| err(&PublicMessage::Curated("serializzazione DXF fallita")))?;
        output.flush()?;
        let mut buffered = output.into_inner();
        buffered.flush()?;
        drop(buffered);
        let (bytes, outcome) =
            publish_file_atomic_limited(temp, &self.path, self.durable, self.max_output_bytes)?;
        Ok(Published {
            bytes,
            loss: self.loss,
            fidelity: plenora_io_core::FidelityAssessment::lossless(),
            outcome,
        })
    }
}

fn add_entity(dr: &mut Drawing, specific: EntityType, layer: Option<&str>) {
    let mut e = Entity::new(specific);
    if let Some(l) = layer {
        l.clone_into(&mut e.common.layer);
    }
    dr.add_entity(e);
}

fn lwpolyline(coordinates: &[WkbCoordinate], closed: bool) -> EntityType {
    let coordinates = ring_without_duplicate_end(coordinates, closed);
    let vertices = coordinates
        .iter()
        .map(|coordinate| LwPolylineVertex {
            x: coordinate.x,
            y: coordinate.y,
            ..Default::default()
        })
        .collect();
    EntityType::LwPolyline(LwPolyline {
        flags: i32::from(closed),
        vertices,
        ..Default::default()
    })
}

fn point_entity(coordinate: &WkbCoordinate) -> Result<EntityType> {
    validate_dxf_coordinate(coordinate)?;
    Ok(EntityType::ModelPoint(ModelPoint {
        location: DxfPoint::new(coordinate.x, coordinate.y, coordinate.z.unwrap_or(0.0)),
        ..Default::default()
    }))
}

fn ring_without_duplicate_end(coordinates: &[WkbCoordinate], closed: bool) -> &[WkbCoordinate] {
    if closed && coordinates.len() > 1 && coordinates.first() == coordinates.last() {
        &coordinates[..coordinates.len() - 1]
    } else {
        coordinates
    }
}

fn validate_dxf_coordinate(coordinate: &WkbCoordinate) -> Result<()> {
    if coordinate.m.is_some() {
        return Err(err(&PublicMessage::Curated(
            "DXF non rappresenta ordinate M",
        )));
    }
    if !coordinate.x.is_finite()
        || !coordinate.y.is_finite()
        || coordinate.z.is_some_and(|value| !value.is_finite())
    {
        return Err(err(&PublicMessage::Curated(
            "DXF non rappresenta coordinate non finite",
        )));
    }
    Ok(())
}

fn add_polyline(
    drawing: &mut Drawing,
    coordinates: &[WkbCoordinate],
    closed: bool,
    dimensions: CoordinateDimensions,
    layer: Option<&str>,
) -> Result<()> {
    let coordinates = ring_without_duplicate_end(coordinates, closed);
    for coordinate in coordinates {
        validate_dxf_coordinate(coordinate)?;
    }
    if coordinates.len() < 2 {
        return Err(err(&PublicMessage::Curated(
            "polilinea con meno di due coordinate",
        )));
    }
    match dimensions {
        CoordinateDimensions::Xy => add_entity(drawing, lwpolyline(coordinates, closed), layer),
        CoordinateDimensions::Xyz => {
            let mut polyline = Polyline::default();
            polyline.set_is_closed(closed);
            polyline.set_is_3d_polyline(true);
            for coordinate in coordinates {
                let z = coordinate.z.ok_or_else(|| {
                    err(&PublicMessage::Curated("coordinata XYZ senza ordinata Z"))
                })?;
                let mut vertex = Vertex::new(DxfPoint::new(coordinate.x, coordinate.y, z));
                vertex.set_is_3d_polyline_vertex(true);
                polyline.add_vertex(drawing, vertex);
            }
            add_entity(drawing, EntityType::Polyline(polyline), layer);
        }
        CoordinateDimensions::Xym | CoordinateDimensions::Xyzm | CoordinateDimensions::Unknown => {
            return Err(err(&PublicMessage::CuratedPair(
                "dimensionalità non rappresentabile in DXF:",
                dimensions.nome(),
            )))
        }
    }
    Ok(())
}

fn dxf_geometry_rejection_cause(value: &WkbValue) -> Option<&'static str> {
    fn coordinate_cause(coordinate: &WkbCoordinate) -> Option<&'static str> {
        if !coordinate.x.is_finite()
            || !coordinate.y.is_finite()
            || coordinate.z.is_some_and(|value| !value.is_finite())
        {
            Some("dxf.non_finite_coordinate")
        } else if coordinate.m.is_some() {
            Some("dxf.measure_ordinate_unsupported")
        } else {
            None
        }
    }

    let coordinate_rejection = match value {
        WkbValue::Point(coordinate) => coordinate_cause(coordinate),
        WkbValue::LineString(coordinates) | WkbValue::CircularString(coordinates) => {
            coordinates.iter().find_map(coordinate_cause)
        }
        WkbValue::Polygon(rings) | WkbValue::Triangle(rings) => rings
            .iter()
            .flat_map(|ring| ring.iter())
            .find_map(coordinate_cause),
        WkbValue::MultiPoint(values)
        | WkbValue::MultiLineString(values)
        | WkbValue::MultiPolygon(values)
        | WkbValue::GeometryCollection(values)
        | WkbValue::CompoundCurve(values)
        | WkbValue::CurvePolygon(values)
        | WkbValue::MultiCurve(values)
        | WkbValue::MultiSurface(values)
        | WkbValue::PolyhedralSurface(values)
        | WkbValue::Tin(values) => values
            .iter()
            .find_map(|geometry| dxf_geometry_rejection_cause(&geometry.value)),
    };
    if coordinate_rejection.is_some() {
        return coordinate_rejection;
    }
    match value {
        WkbValue::Point(_) => None,
        WkbValue::LineString(line) => (line.len() < 2).then_some("dxf.degenerate_geometry"),
        WkbValue::Polygon(rings) => {
            if rings.is_empty() || rings.iter().any(|ring| ring.len() < 4) {
                Some("dxf.degenerate_geometry")
            } else if rings.iter().any(|ring| ring.first() != ring.last()) {
                Some("dxf.unclosed_polygon_ring")
            } else if rings.len() > 1 {
                Some("dxf.interior_rings_unsupported")
            } else {
                None
            }
        }
        WkbValue::MultiPoint(values)
        | WkbValue::MultiLineString(values)
        | WkbValue::MultiPolygon(values)
        | WkbValue::GeometryCollection(values) => {
            if values.is_empty() {
                Some("dxf.empty_geometry_unsupported")
            } else {
                values
                    .iter()
                    .find_map(|geometry| dxf_geometry_rejection_cause(&geometry.value))
            }
        }
        WkbValue::CircularString(_)
        | WkbValue::CompoundCurve(_)
        | WkbValue::CurvePolygon(_)
        | WkbValue::MultiCurve(_)
        | WkbValue::MultiSurface(_)
        | WkbValue::PolyhedralSurface(_)
        | WkbValue::Tin(_)
        | WkbValue::Triangle(_) => Some("dxf.geometry_type_unsupported"),
    }
}

fn add_geometry(
    drawing: &mut Drawing,
    geometry: &WkbGeometry,
    layer: Option<&str>,
    loss: &mut LossReport,
) -> Result<()> {
    if geometry.srid.is_some() {
        return Err(err(&PublicMessage::Curated(
            "SRID EWKB embedded non rappresentabile; usare il CRS GEODATA",
        )));
    }
    match &geometry.value {
        WkbValue::Point(point) => add_entity(drawing, point_entity(point)?, layer),
        WkbValue::LineString(line) => {
            add_polyline(drawing, line, false, geometry.dimensions, layer)?;
        }
        WkbValue::Polygon(rings) => {
            if rings.len() > 1 {
                return Err(err(&PublicMessage::Curated(
                    "anelli interni Polygon non rappresentabili in DXF",
                )));
            }
            let exterior = rings
                .first()
                .ok_or_else(|| err(&PublicMessage::Curated("Polygon senza anello esterno")))?;
            if exterior.first() != exterior.last() {
                return Err(err(&PublicMessage::Curated(
                    "anello Polygon DXF non chiuso",
                )));
            }
            add_polyline(drawing, exterior, true, geometry.dimensions, layer)?;
        }
        WkbValue::MultiPoint(points) => {
            loss.record("MultiPoint esploso in entità DXF", points.len() as u64);
            for point in points {
                add_geometry(drawing, point, layer, loss)?;
            }
        }
        WkbValue::MultiLineString(lines) => {
            loss.record("MultiLineString esplosa in entità DXF", lines.len() as u64);
            for line in lines {
                add_geometry(drawing, line, layer, loss)?;
            }
        }
        WkbValue::MultiPolygon(polygons) => {
            loss.record("MultiPolygon esplosa in entità DXF", polygons.len() as u64);
            for polygon in polygons {
                add_geometry(drawing, polygon, layer, loss)?;
            }
        }
        WkbValue::GeometryCollection(collection) => {
            loss.record(
                "GeometryCollection esplosa in entità DXF",
                collection.len() as u64,
            );
            for child in collection {
                add_geometry(drawing, child, layer, loss)?;
            }
        }
        WkbValue::CircularString(_)
        | WkbValue::CompoundCurve(_)
        | WkbValue::CurvePolygon(_)
        | WkbValue::MultiCurve(_)
        | WkbValue::MultiSurface(_)
        | WkbValue::PolyhedralSurface(_)
        | WkbValue::Tin(_)
        | WkbValue::Triangle(_) => {
            return Err(err(&PublicMessage::Curated(
                "tipo WKB esteso non rappresentabile nel profilo DXF corrente",
            )))
        }
    }
    Ok(())
}

const DXF_SPOOL_NULL: u32 = u32::MAX;
const DXF_SPOOL_MEMORY_LIMIT: u64 = 64 * 1024 * 1024;

enum DxfSpoolOutput {
    Memory {
        rows: Vec<DxfSpoolRow>,
        bytes: u64,
    },
    File {
        tempfile: Arc<tempfile::NamedTempFile>,
        output: BufWriter<File>,
    },
}

enum DxfSpoolStorage {
    Memory(Arc<Vec<DxfSpoolRow>>),
    File(Arc<tempfile::NamedTempFile>),
}

impl DxfSpoolStorage {
    fn reader(&self) -> Result<DxfSpoolReader> {
        match self {
            Self::Memory(rows) => Ok(DxfSpoolReader::Memory {
                rows: rows.clone(),
                index: 0,
            }),
            Self::File(tempfile) => Ok(DxfSpoolReader::File(BufReader::new(tempfile.reopen()?))),
        }
    }
}

enum DxfSpoolReader {
    Memory {
        rows: Arc<Vec<DxfSpoolRow>>,
        index: usize,
    },
    File(BufReader<File>),
}

struct DxfOutputRow {
    geometry: Option<Vec<u8>>,
    layer: Option<String>,
    entity_type: Option<String>,
    text: Option<String>,
}

struct DxfSpoolRow {
    geometry: Option<WkbGeometry>,
    layer: Option<String>,
    entity_type: Option<String>,
    text: Option<String>,
}

impl DxfSpoolReader {
    fn next_row(
        &mut self,
        dimensions: CoordinateDimensions,
        limits: &WkbLimits,
    ) -> Result<DxfOutputRow> {
        match self {
            Self::Memory { rows, index } => {
                let row = rows
                    .get(*index)
                    .ok_or_else(|| err(&PublicMessage::Curated("spool DXF in memoria troncato")))?;
                *index += 1;
                let geometry = row
                    .geometry
                    .as_ref()
                    .map(|geometry| {
                        if geometry.dimensions == dimensions {
                            encode_wkb(geometry, WkbFlavor::Iso)
                        } else {
                            let mut geometry = geometry.clone();
                            set_geometry_dimensions(&mut geometry, dimensions);
                            encode_wkb(&geometry, WkbFlavor::Iso)
                        }
                    })
                    .transpose()?;
                Ok(DxfOutputRow {
                    geometry,
                    layer: row.layer.clone(),
                    entity_type: row.entity_type.clone(),
                    text: row.text.clone(),
                })
            }
            Self::File(input) => {
                let geometry = match read_dxf_spool_value(input)? {
                    None => None,
                    Some(bytes) => {
                        if inspect_wkb(&bytes, limits)?.dimensions == dimensions {
                            Some(bytes)
                        } else {
                            let mut geometry = decode_wkb(&bytes, limits)?;
                            set_geometry_dimensions(&mut geometry, dimensions);
                            Some(encode_wkb(&geometry, WkbFlavor::Iso)?)
                        }
                    }
                };
                Ok(DxfOutputRow {
                    geometry,
                    layer: read_dxf_spool_string(input)?,
                    entity_type: read_dxf_spool_string(input)?,
                    text: read_dxf_spool_string(input)?,
                })
            }
        }
    }
}

struct DxfSpoolWriter {
    output: DxfSpoolOutput,
    bytes: u64,
    limit: u64,
    memory_limit: u64,
    budget: OperationBudget,
    /// Le prenotazioni di spill restano vive quanto il file temporaneo.
    ///
    /// Nel modello legacy si faceva `commit`, cioe' consumo definitivo: la
    /// quota non tornava mai, nemmeno dopo che il file era stato rimosso. Nel
    /// modello unificato lo spill e' occupazione trattenuta e la `SpillLease`
    /// la restituisce al drop, insieme allo spool che l'ha creata.
    leases: Vec<SpillLease>,
}

/// Budget dell'operazione per i costruttori di prova dello spool DXF.
///
/// Passa dalle opzioni pubbliche, non dalla decomposizione delle parti: la
/// seconda e' riservata a `plenora-io-model` e `plenora-io-core` (INV-13).
#[cfg(test)]
fn budget_di_prova() -> Result<OperationBudget> {
    let bundle = plenora_io_model::budget::PipelineBudget::builder().build()?;
    Ok(ReadOptions::from_read_parts(bundle.into_read_parts())
        .budget()
        .clone())
}

impl DxfSpoolWriter {
    const fn new(limit: u64, budget: OperationBudget) -> Self {
        Self {
            output: DxfSpoolOutput::Memory {
                rows: Vec::new(),
                bytes: 0,
            },
            bytes: 0,
            limit,
            memory_limit: DXF_SPOOL_MEMORY_LIMIT,
            budget,
            leases: Vec::new(),
        }
    }

    #[cfg(test)]
    fn with_memory_limit(limit: u64, memory_limit: u64) -> Self {
        Self {
            output: DxfSpoolOutput::Memory {
                rows: Vec::new(),
                bytes: 0,
            },
            bytes: 0,
            limit,
            memory_limit,
            // Il test misura solo la soglia di migrazione in memoria: una
            // pipeline coi limiti predefiniti basta, e non serve un pool.
            // Passa dalle opzioni e non dalla decomposizione delle parti:
            // quest'ultima e' riservata a model/core (INV-13).
            budget: match budget_di_prova() {
                Ok(budget) => budget,
                Err(error) => unreachable!("budget di test: {error:?}"),
            },
            leases: Vec::new(),
        }
    }

    fn write_file_value(output: &mut impl Write, value: Option<&[u8]>) -> Result<()> {
        let length = match value {
            None => DXF_SPOOL_NULL,
            Some(bytes) => u32::try_from(bytes.len()).map_err(|_| {
                PlenoraIoError::limite_redatto(&PublicMessage::Curated(
                    "valore DXF troppo grande per lo spool",
                ))
            })?,
        };
        output.write_all(&length.to_le_bytes())?;
        if let Some(bytes) = value {
            output.write_all(bytes)?;
        }
        Ok(())
    }

    fn write_file_row(output: &mut impl Write, row: &DxfSpoolRow) -> Result<()> {
        let geometry = row
            .geometry
            .as_ref()
            .map(|geometry| encode_wkb(geometry, WkbFlavor::Iso))
            .transpose()?;
        Self::write_file_value(output, geometry.as_deref())?;
        Self::write_file_value(output, row.layer.as_deref().map(str::as_bytes))?;
        Self::write_file_value(output, row.entity_type.as_deref().map(str::as_bytes))?;
        Self::write_file_value(output, row.text.as_deref().map(str::as_bytes))
    }

    fn spill_to_file(&mut self) -> Result<()> {
        let DxfSpoolOutput::Memory { rows, .. } = std::mem::replace(
            &mut self.output,
            DxfSpoolOutput::Memory {
                rows: Vec::new(),
                bytes: 0,
            },
        ) else {
            return Ok(());
        };
        let tempfile = Arc::new(tempfile::NamedTempFile::new()?);
        let mut output = BufWriter::new(tempfile.reopen()?);
        let lease = (self.bytes > 0)
            .then(|| self.budget.context().lease_spill(self.bytes))
            .transpose()?;
        for row in &rows {
            Self::write_file_row(&mut output, row)?;
        }
        if let Some(lease) = lease {
            self.leases.push(lease);
        }
        self.output = DxfSpoolOutput::File { tempfile, output };
        Ok(())
    }

    fn push(&mut self, row: DxfSpoolRow) -> Result<()> {
        let logical_bytes = dxf_spool_row_length(&row);
        let next = self.bytes.checked_add(logical_bytes).ok_or_else(|| {
            err(&PublicMessage::Curated(
                "dimensione spool DXF fuori intervallo",
            ))
        })?;
        if next > self.limit {
            return Err(PlenoraIoError::limite_redatto(
                &PublicMessage::CuratedBetween(
                    "spool DXF di",
                    NumeroStrutturale::Conteggio(next),
                    "byte oltre il limite di",
                    NumeroStrutturale::Limite(self.limit),
                ),
            ));
        }
        let memory_bytes = dxf_spool_row_memory(&row);
        let spill = matches!(
            &self.output,
            DxfSpoolOutput::Memory { bytes, .. }
                if bytes.saturating_add(memory_bytes) > self.memory_limit
        );
        if spill {
            self.spill_to_file()?;
        }
        let file_lease = matches!(&self.output, DxfSpoolOutput::File { .. })
            .then(|| self.budget.context().lease_spill(logical_bytes))
            .transpose()?;
        match &mut self.output {
            DxfSpoolOutput::Memory { rows, bytes } => {
                rows.push(row);
                *bytes = bytes.saturating_add(memory_bytes);
            }
            DxfSpoolOutput::File { output, .. } => Self::write_file_row(output, &row)?,
        }
        if let Some(lease) = file_lease {
            self.leases.push(lease);
        }
        self.bytes = next;
        Ok(())
    }

    fn write_and_clear(
        &mut self,
        walker: &mut Walker,
        cancellation: &CancellationToken,
    ) -> Result<()> {
        for (index, (((mut geometry, layer), entity_type), text)) in walker
            .geometries
            .drain(..)
            .zip(walker.layers.drain(..))
            .zip(walker.types.drain(..))
            .zip(walker.texts.drain(..))
            .enumerate()
        {
            check_cancelled_periodically(cancellation, ErrorPhase::Read, index)?;
            if let Some(geometry) = geometry.as_mut() {
                let dimensions = if geometry_has_nonzero_z(geometry) {
                    CoordinateDimensions::Xyz
                } else {
                    CoordinateDimensions::Xy
                };
                set_geometry_dimensions(geometry, dimensions);
            }
            self.push(DxfSpoolRow {
                geometry,
                layer,
                entity_type,
                text,
            })?;
        }
        Ok(())
    }

    fn finish(self) -> Result<DxfSpoolStorage> {
        match self.output {
            DxfSpoolOutput::Memory { rows, .. } => Ok(DxfSpoolStorage::Memory(Arc::new(rows))),
            DxfSpoolOutput::File {
                tempfile,
                mut output,
            } => {
                output.flush()?;
                drop(output);
                Ok(DxfSpoolStorage::File(tempfile))
            }
        }
    }
}

fn dxf_spool_row_length(row: &DxfSpoolRow) -> u64 {
    let value = |length: usize| 4_u64.saturating_add(u64::try_from(length).unwrap_or(u64::MAX));
    let geometry = row
        .geometry
        .as_ref()
        .map(wkb_iso_length)
        .unwrap_or_default();
    value(geometry)
        .saturating_add(value(row.layer.as_ref().map_or(0, String::len)))
        .saturating_add(value(row.entity_type.as_ref().map_or(0, String::len)))
        .saturating_add(value(row.text.as_ref().map_or(0, String::len)))
}

fn wkb_iso_length(geometry: &WkbGeometry) -> usize {
    let coordinate_bytes = match geometry.dimensions {
        CoordinateDimensions::Xy | CoordinateDimensions::Unknown => 16,
        CoordinateDimensions::Xyz | CoordinateDimensions::Xym => 24,
        CoordinateDimensions::Xyzm => 32,
    };
    let coordinates = |values: &[WkbCoordinate]| {
        4_usize.saturating_add(values.len().saturating_mul(coordinate_bytes))
    };
    5_usize.saturating_add(match &geometry.value {
        WkbValue::Point(_) => coordinate_bytes,
        WkbValue::LineString(values) | WkbValue::CircularString(values) => coordinates(values),
        WkbValue::Polygon(rings) | WkbValue::Triangle(rings) => {
            rings.iter().fold(4_usize, |length, ring| {
                length.saturating_add(coordinates(ring))
            })
        }
        WkbValue::MultiPoint(children)
        | WkbValue::MultiLineString(children)
        | WkbValue::MultiPolygon(children)
        | WkbValue::GeometryCollection(children)
        | WkbValue::CompoundCurve(children)
        | WkbValue::CurvePolygon(children)
        | WkbValue::MultiCurve(children)
        | WkbValue::MultiSurface(children)
        | WkbValue::PolyhedralSurface(children)
        | WkbValue::Tin(children) => children.iter().fold(4_usize, |length, child| {
            length.saturating_add(wkb_iso_length(child))
        }),
    })
}

fn dxf_spool_row_memory(row: &DxfSpoolRow) -> u64 {
    let string_bytes = |value: &Option<String>| {
        value
            .as_ref()
            .map_or(0_u64, |value| value.capacity() as u64)
    };
    (std::mem::size_of::<DxfSpoolRow>() as u64)
        .saturating_add(row.geometry.as_ref().map_or(0, geometry_heap_memory_bytes))
        .saturating_add(string_bytes(&row.layer))
        .saturating_add(string_bytes(&row.entity_type))
        .saturating_add(string_bytes(&row.text))
}

fn geometry_heap_memory_bytes(geometry: &WkbGeometry) -> u64 {
    const ALLOCATION_OVERHEAD: u64 = 32;
    let coordinates = |values: &Vec<WkbCoordinate>| {
        (values.capacity() * std::mem::size_of::<WkbCoordinate>()) as u64 + ALLOCATION_OVERHEAD
    };
    match &geometry.value {
        WkbValue::Point(_) => 0,
        WkbValue::LineString(values) | WkbValue::CircularString(values) => coordinates(values),
        WkbValue::Polygon(rings) | WkbValue::Triangle(rings) => {
            (rings.capacity() * std::mem::size_of::<Vec<WkbCoordinate>>()) as u64
                + ALLOCATION_OVERHEAD
                + rings.iter().map(coordinates).sum::<u64>()
        }
        WkbValue::MultiPoint(children)
        | WkbValue::MultiLineString(children)
        | WkbValue::MultiPolygon(children)
        | WkbValue::GeometryCollection(children)
        | WkbValue::CompoundCurve(children)
        | WkbValue::CurvePolygon(children)
        | WkbValue::MultiCurve(children)
        | WkbValue::MultiSurface(children)
        | WkbValue::PolyhedralSurface(children)
        | WkbValue::Tin(children) => {
            (children.capacity() * std::mem::size_of::<WkbGeometry>()) as u64
                + ALLOCATION_OVERHEAD
                + children.iter().map(geometry_heap_memory_bytes).sum::<u64>()
        }
    }
}

fn read_dxf_spool_value(input: &mut impl Read) -> Result<Option<Vec<u8>>> {
    let mut length = [0_u8; 4];
    input
        .read_exact(&mut length)
        .map_err(|_| err(&PublicMessage::Curated("spool DXF troncato")))?;
    let length = u32::from_le_bytes(length);
    if length == DXF_SPOOL_NULL {
        return Ok(None);
    }
    let mut value = vec![
        0;
        usize::try_from(length).map_err(|_| err(&PublicMessage::Curated(
            "lunghezza spool DXF non valida"
        )))?
    ];
    input
        .read_exact(&mut value)
        .map_err(|_| err(&PublicMessage::Curated("spool DXF troncato")))?;
    Ok(Some(value))
}

fn read_dxf_spool_string(input: &mut impl Read) -> Result<Option<String>> {
    let Some(bytes) = read_dxf_spool_value(input)? else {
        return Ok(None);
    };
    String::from_utf8(bytes)
        .map(Some)
        .map_err(|_| err(&PublicMessage::Curated("testo dello spool DXF non UTF-8")))
}

struct DxfDataset {
    layers: Vec<LayerContract>,
    spool: DxfSpoolStorage,
    rows: usize,
    wkb_limits: WkbLimits,
    loss: LossReport,
    reader_gate: SingleReaderGate,
}

impl OpenDatasetHandle for DxfDataset {
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
        let reader = self.reader_gate.open(request.layer, || {
            Ok(Box::new(DxfReader {
                input: self.spool.reader()?,
                remaining_rows: self.rows,
                wkb_limits: self.wkb_limits,
                layer: self.layers[0].clone(),
                loss: self.loss.clone(),
                batch_sizer: plenora_io_core::AdaptiveBatchSizer::new(
                    self.layers[0].contract.schema.as_ref(),
                    request.batch_target,
                ),
                cancellation: request.cancellation.clone(),
            }))
        })?;
        Ok(plenora_io_core::with_batch_target(
            reader,
            request.batch_target,
            request.cancellation.clone(),
        ))
    }
}

struct DxfReader {
    input: DxfSpoolReader,
    remaining_rows: usize,
    wkb_limits: WkbLimits,
    layer: LayerContract,
    loss: LossReport,
    batch_sizer: plenora_io_core::AdaptiveBatchSizer,
    cancellation: CancellationToken,
}

impl LayerReader for DxfReader {
    fn contract(&self) -> &LayerContract {
        &self.layer
    }
    fn next_batch(&mut self) -> Result<Option<RecordBatch>> {
        check_cancelled(&self.cancellation, ErrorPhase::Read)?;
        if self.remaining_rows == 0 {
            return Ok(None);
        }
        let rows = self.remaining_rows.min(self.batch_sizer.rows());
        let dimensions = self
            .layer
            .contract
            .geometry
            .as_ref()
            .map_or(CoordinateDimensions::Unknown, |geometry| {
                geometry.dimensions
            });
        let mut geometries = Vec::with_capacity(rows);
        let mut layers = Vec::with_capacity(rows);
        let mut types = Vec::with_capacity(rows);
        let mut texts = Vec::with_capacity(rows);
        for index in 0..rows {
            check_cancelled_periodically(&self.cancellation, ErrorPhase::Read, index)?;
            let row = self.input.next_row(dimensions, &self.wkb_limits)?;
            geometries.push(row.geometry);
            layers.push(row.layer);
            types.push(row.entity_type);
            texts.push(row.text);
        }
        self.remaining_rows -= rows;
        let arrays: Vec<ArrayRef> = vec![
            Arc::new(BinaryArray::from(
                geometries
                    .iter()
                    .map(|value| value.as_deref())
                    .collect::<Vec<_>>(),
            )),
            Arc::new(StringArray::from(layers)),
            Arc::new(StringArray::from(types)),
            Arc::new(StringArray::from(texts)),
        ];
        let batch =
            RecordBatch::try_new(self.layer.contract.schema.clone(), arrays).map_err(|_| {
                err(&PublicMessage::Curated(
                    "batch DXF da spool non ricostruibile",
                ))
            })?;
        self.batch_sizer.observe(&batch);
        Ok(Some(batch))
    }
    fn loss_report(&self) -> LossReport {
        self.loss.clone()
    }
}

const fn coordinate(point: [f64; 3]) -> WkbCoordinate {
    WkbCoordinate {
        x: point[0],
        y: point[1],
        z: Some(point[2]),
        m: None,
    }
}

/// Trasformazione OCS→WCS dall'extrusion direction di un'entità.
fn ocs_of(normal: &Vector) -> Transform3 {
    Transform3::ocs([normal.x, normal.y, normal.z])
}

/// Mappa i punti locali (già tassellati) in coordinate WCS via la trasformazione.
fn mapped(local: &[[f64; 2]], elevation: f64, transform: &Transform3) -> Vec<WkbCoordinate> {
    local
        .iter()
        .map(|point| coordinate(transform.apply([point[0], point[1], elevation])))
        .collect()
}

/// Walker ricorsivo: converte le entità (esplodendo gli INSERT) in righe
/// colonnari WKB. È il motore di plenora-dxf-tools adattato all'interfaccia del
/// driver (AST WKB lossless + `LossReport` invece di `GeoJSON`).
struct Walker {
    blocks: HashMap<String, Arc<Block>>,
    geometries: Vec<Option<WkbGeometry>>,
    layers: Vec<Option<String>>,
    types: Vec<Option<String>>,
    texts: Vec<Option<String>>,
    loss: LossReport,
    budget: usize,
    max_rows: usize,
    remaining_vertices: usize,
    cancellation: CancellationToken,
    visited_entities: usize,
    emitted_rows: usize,
}

impl Walker {
    /// Prende i due scalari che consulta invece di un `Limits`: e' la forma
    /// che sopravvive alla migrazione, perche' nel modello unificato quel
    /// tipo non esiste.
    fn new(drawing: &Drawing, quote: DxfQuote, cancellation: &CancellationToken) -> Result<Self> {
        let mut blocks = HashMap::new();
        for (index, block) in drawing.blocks().enumerate() {
            check_cancelled_periodically(cancellation, ErrorPhase::Read, index)?;
            blocks.insert(block.name.clone(), Arc::new(block.clone()));
        }
        Ok(Self {
            blocks,
            geometries: Vec::new(),
            layers: Vec::new(),
            types: Vec::new(),
            texts: Vec::new(),
            loss: LossReport::default(),
            budget: MAX_ENTITIES,
            max_rows: quote.righe,
            remaining_vertices: quote.vertici,
            cancellation: cancellation.clone(),
            visited_entities: 0,
            emitted_rows: 0,
        })
    }

    /// Il layer "0" (o vuoto) eredita quello del contesto (blocco padre).
    fn effective_layer(layer: &str, context: &str) -> String {
        if layer.is_empty() || layer == "0" {
            context.to_owned()
        } else {
            layer.to_owned()
        }
    }

    fn push(
        &mut self,
        value: WkbValue,
        layer: &str,
        ty: &'static str,
        text: Option<String>,
    ) -> Result<()> {
        if self.emitted_rows >= self.max_rows {
            return Err(PlenoraIoError::limite_redatto(&PublicMessage::CuratedWith(
                "righe DXF oltre il limite di",
                NumeroStrutturale::Limite(driver_common::saturating_u64(self.max_rows)),
            )));
        }
        self.emitted_rows = self.emitted_rows.saturating_add(1);
        let vertices = value_coordinate_count(&value);
        if vertices > self.remaining_vertices {
            return Err(PlenoraIoError::limite_redatto(&PublicMessage::CuratedWith(
                "vertici DXF oltre il limite residuo di",
                NumeroStrutturale::Limite(driver_common::saturating_u64(self.remaining_vertices)),
            )));
        }
        self.remaining_vertices -= vertices;
        self.geometries.push(Some(WkbGeometry {
            value,
            dimensions: CoordinateDimensions::Xyz,
            srid: None,
        }));
        self.layers.push(Some(layer.to_owned()));
        self.types.push(Some(ty.to_owned()));
        self.texts.push(text);
        Ok(())
    }

    // Dispatcher esaustivo sulle entita' DXF: un solo `match` per tipo tiene
    // adiacenti la conversione e le sue perdite dichiarate. Spezzarlo
    // disperderebbe la tabella di corrispondenza fra tipo DXF e WKB.
    #[allow(clippy::too_many_lines)]
    fn walk_entity(
        &mut self,
        entity: &Entity,
        transform: Transform3,
        context: &str,
        depth: usize,
        visiting: &mut HashSet<String>,
    ) -> Result<()> {
        check_cancelled_periodically(&self.cancellation, ErrorPhase::Read, self.visited_entities)?;
        self.visited_entities = self.visited_entities.saturating_add(1);
        self.budget = self.budget.saturating_sub(1);
        if self.budget == 0 {
            return Err(err(&PublicMessage::CuratedWith(
                "DXF oltre il limite di entità:",
                NumeroStrutturale::Limite(driver_common::saturating_u64(MAX_ENTITIES)),
            )));
        }
        let layer = Self::effective_layer(&entity.common.layer, context);
        match &entity.specific {
            EntityType::Line(l) => {
                // LINE è già in WCS: nessun OCS.
                let p1 = transform.apply([l.p1.x, l.p1.y, l.p1.z]);
                let p2 = transform.apply([l.p2.x, l.p2.y, l.p2.z]);
                self.push(
                    WkbValue::LineString(vec![coordinate(p1), coordinate(p2)]),
                    &layer,
                    "LINE",
                    None,
                )?;
            }
            EntityType::LwPolyline(p) => {
                let object_to_world = transform.then(ocs_of(&p.extrusion_direction));
                let vertices: Vec<([f64; 3], f64)> = p
                    .vertices
                    .iter()
                    .map(|vertex| ([vertex.x, vertex.y, entity.common.elevation], vertex.bulge))
                    .collect();
                self.emit_polyline(
                    &layer,
                    &vertices,
                    p.flags & 1 == 1,
                    object_to_world,
                    "LWPOLYLINE",
                )?;
            }
            EntityType::Polyline(p) => {
                let object_to_world = transform.then(ocs_of(&p.normal));
                let vertices: Vec<([f64; 3], f64)> = p
                    .vertices()
                    .map(|vertex| {
                        (
                            [vertex.location.x, vertex.location.y, vertex.location.z],
                            vertex.bulge,
                        )
                    })
                    .collect();
                self.emit_polyline(
                    &layer,
                    &vertices,
                    p.is_closed(),
                    object_to_world,
                    "POLYLINE",
                )?;
            }
            EntityType::Circle(cir) => {
                let object_to_world = transform.then(ocs_of(&cir.normal));
                let local =
                    tessellate_circle([cir.center.x, cir.center.y], cir.radius, ARC_SEGMENTS);
                if local.len() < 4 {
                    return Err(err(&PublicMessage::Curated(
                        "CIRCLE degenere non convertibile",
                    )));
                }
                self.loss.record("CIRCLE tassellata", 1);
                self.push(
                    WkbValue::Polygon(vec![mapped(&local, cir.center.z, &object_to_world)]),
                    &layer,
                    "CIRCLE",
                    None,
                )?;
            }
            EntityType::Arc(a) => {
                let object_to_world = transform.then(ocs_of(&a.normal));
                let local = tessellate_arc(
                    [a.center.x, a.center.y],
                    a.radius,
                    a.start_angle,
                    a.end_angle,
                    ARC_SEGMENTS,
                );
                if local.len() < 2 {
                    return Err(err(&PublicMessage::Curated(
                        "ARC degenere non convertibile",
                    )));
                }
                self.loss.record("ARC tassellato", 1);
                self.push(
                    WkbValue::LineString(mapped(&local, a.center.z, &object_to_world)),
                    &layer,
                    "ARC",
                    None,
                )?;
            }
            EntityType::ModelPoint(pt) => {
                let object_to_world = transform.then(ocs_of(&pt.extrusion_direction));
                let mapped = object_to_world.apply([pt.location.x, pt.location.y, pt.location.z]);
                self.push(WkbValue::Point(coordinate(mapped)), &layer, "POINT", None)?;
            }
            EntityType::Text(txt) => {
                let object_to_world = transform.then(ocs_of(&txt.normal));
                let mapped =
                    object_to_world.apply([txt.location.x, txt.location.y, txt.location.z]);
                self.emit_text(&layer, mapped, &txt.value, "TEXT")?;
                self.loss.record("TEXT rappresentato come punto", 1);
            }
            EntityType::MText(txt) => {
                // MTEXT usa un insertion point già in WCS.
                let mapped = transform.apply([
                    txt.insertion_point.x,
                    txt.insertion_point.y,
                    txt.insertion_point.z,
                ]);
                self.emit_text(&layer, mapped, &txt.text, "MTEXT")?;
                self.loss.record("MTEXT rappresentato come punto", 1);
            }
            EntityType::Solid(s) => {
                let object_to_world = transform.then(ocs_of(&s.extrusion_direction));
                // Ordine di traversata del quadrilatero DXF: 1,2,4,3.
                let corners = [
                    object_to_world.apply([s.first_corner.x, s.first_corner.y, s.first_corner.z]),
                    object_to_world.apply([
                        s.second_corner.x,
                        s.second_corner.y,
                        s.second_corner.z,
                    ]),
                    object_to_world.apply([
                        s.fourth_corner.x,
                        s.fourth_corner.y,
                        s.fourth_corner.z,
                    ]),
                    object_to_world.apply([s.third_corner.x, s.third_corner.y, s.third_corner.z]),
                ];
                let mut ring: Vec<WkbCoordinate> =
                    corners.iter().copied().map(coordinate).collect();
                ring.push(ring[0]);
                self.push(WkbValue::Polygon(vec![ring]), &layer, "SOLID", None)?;
                self.loss.record("SOLID rappresentato come Polygon", 1);
            }
            EntityType::Ellipse(el) => {
                let local = tessellate_ellipse3(
                    [el.center.x, el.center.y, el.center.z],
                    [el.major_axis.x, el.major_axis.y, el.major_axis.z],
                    [el.normal.x, el.normal.y, el.normal.z],
                    el.minor_axis_ratio,
                    el.start_parameter,
                    el.end_parameter,
                    ARC_SEGMENTS,
                );
                if local.len() < 2 {
                    return Err(err(&PublicMessage::Curated(
                        "ELLIPSE degenere non convertibile",
                    )));
                }
                let full = local.first() == local.last() && local.len() >= 4;
                let coordinates: Vec<WkbCoordinate> = local
                    .iter()
                    .map(|point| coordinate(transform.apply(*point)))
                    .collect();
                self.loss.record("ELLIPSE tassellata", 1);
                if full {
                    self.push(
                        WkbValue::Polygon(vec![coordinates]),
                        &layer,
                        "ELLIPSE",
                        None,
                    )?;
                } else {
                    self.push(WkbValue::LineString(coordinates), &layer, "ELLIPSE", None)?;
                }
            }
            EntityType::Spline(sp) => {
                // I control point della SPLINE sono in WCS: nessun OCS.
                let controls: Vec<[f64; 3]> = sp
                    .control_points
                    .iter()
                    .map(|point| [point.x, point.y, point.z])
                    .collect();
                let samples = controls.len().max(2) * 6;
                // `.max(1)` garantisce un valore positivo: la conversione a
                // usize non puo' perdere il segno.
                #[allow(clippy::cast_sign_loss)]
                let degree = sp.degree_of_curve.max(1) as usize;
                let local = tessellate_spline3(
                    degree,
                    &sp.knot_values,
                    &controls,
                    &sp.weight_values,
                    samples,
                );
                if local.len() < 2 {
                    return Err(err(&PublicMessage::Curated(
                        "SPLINE degenere non convertibile",
                    )));
                }
                self.loss.record("SPLINE tassellata", 1);
                let closed = sp.flags & 1 == 1;
                let mut coordinates: Vec<WkbCoordinate> = local
                    .iter()
                    .map(|point| coordinate(transform.apply(*point)))
                    .collect();
                if closed {
                    if coordinates.first() != coordinates.last() {
                        coordinates.push(coordinates[0]);
                    }
                    if coordinates.len() >= 4 {
                        self.push(WkbValue::Polygon(vec![coordinates]), &layer, "SPLINE", None)?;
                    } else {
                        return Err(err(&PublicMessage::Curated(
                            "SPLINE chiusa degenere non convertibile",
                        )));
                    }
                } else {
                    self.push(WkbValue::LineString(coordinates), &layer, "SPLINE", None)?;
                }
            }
            EntityType::Insert(insert) => {
                self.walk_insert(insert, transform, &layer, depth, visiting)?;
            }
            EntityType::Region(_) | EntityType::Body(_) => {
                return Err(err(&PublicMessage::Curated(
                    "REGION/BODY (ACIS) non convertibile",
                )));
            }
            EntityType::AttributeDefinition(_)
            | EntityType::Attribute(_)
            | EntityType::Seqend(_)
            | EntityType::Vertex(_) => {
                // Elementi di struttura/template: nessuna geometria autonoma.
            }
            _ => return Err(err(&PublicMessage::Curated("entità DXF non gestita"))),
        }
        Ok(())
    }

    // Il confronto esatto delle quote e' voluto: l'approssimazione del bulge
    // sul piano iniziale va segnalata appena le due Z differiscono di un bit,
    // e una tolleranza nasconderebbe la perdita al report.
    #[allow(clippy::float_cmp)]
    fn emit_polyline(
        &mut self,
        layer: &str,
        vertices: &[([f64; 3], f64)],
        closed: bool,
        transform: Transform3,
        kind: &'static str,
    ) -> Result<()> {
        if vertices.len() < 2 {
            return Err(err(&PublicMessage::Curated(
                "polilinea degenere (<2 vertici) non convertibile",
            )));
        }
        if vertices.iter().enumerate().any(|(index, (point, bulge))| {
            let next = if index + 1 < vertices.len() {
                Some(vertices[index + 1].0)
            } else if closed {
                Some(vertices[0].0)
            } else {
                None
            };
            *bulge != 0.0 && next.is_some_and(|next| next[2] != point[2])
        }) {
            self.loss.record(
                "bulge DXF con quote diverse approssimato sul piano iniziale",
                1,
            );
        }
        let bulges = vertices
            .iter()
            .filter(|(_, bulge)| bulge.is_finite() && *bulge != 0.0)
            .count() as u64;
        if bulges > 0 {
            self.loss.record("archi bulge tassellati", bulges);
        }
        let mut positions = polyline_positions(vertices, closed, &transform);
        if closed {
            if positions.first() != positions.last() {
                positions.push(positions[0]);
            }
            if positions.len() < 4 {
                return Err(err(&PublicMessage::Curated(
                    "polilinea chiusa degenere non convertibile",
                )));
            }
            self.push(WkbValue::Polygon(vec![positions]), layer, kind, None)?;
        } else {
            self.push(WkbValue::LineString(positions), layer, kind, None)?;
        }
        Ok(())
    }

    fn emit_text(
        &mut self,
        layer: &str,
        at: [f64; 3],
        value: &str,
        kind: &'static str,
    ) -> Result<()> {
        let cleaned = value.trim();
        let text = if cleaned.is_empty() {
            None
        } else {
            Some(cleaned.to_owned())
        };
        self.push(WkbValue::Point(coordinate(at)), layer, kind, text)
    }

    fn walk_insert(
        &mut self,
        insert: &Insert,
        transform: Transform3,
        layer: &str,
        depth: usize,
        visiting: &mut HashSet<String>,
    ) -> Result<()> {
        if depth >= MAX_INSERT_DEPTH {
            return Err(err(&PublicMessage::CuratedWith(
                "annidamento INSERT oltre il limite di",
                NumeroStrutturale::Limite(driver_common::saturating_u64(MAX_INSERT_DEPTH)),
            )));
        }
        // L'INSERT ha una propria OCS: inserimento, rotazione e scala vi sono espressi.
        let base = transform.then(ocs_of(&insert.extrusion_direction));
        let at = base.apply([insert.location.x, insert.location.y, insert.location.z]);
        // Timbro: se l'INSERT porta attributi, emette un punto all'inserimento col
        // nome del blocco (i valori dei tag non hanno colonna → perdita dichiarata).
        let tags = insert
            .attributes()
            .filter(|a| !a.attribute_tag.trim().is_empty())
            .count() as u64;
        if tags > 0 {
            self.push(
                WkbValue::Point(coordinate(at)),
                layer,
                "INSERT",
                Some(insert.name.clone()),
            )?;
            self.loss.record(
                "valori attributi INSERT non rappresentati come colonne",
                tags,
            );
        }

        if !visiting.insert(insert.name.clone()) {
            // Il nome del blocco non esce: e' letto dal file DXF. Resta la
            // condizione, che e' cio' che il chiamante non puo' dedurre.
            return Err(err(&PublicMessage::Curated(
                "riferimento ciclico fra blocchi DXF",
            )));
        }
        let composed = base.then(Transform3::insert(
            [insert.location.x, insert.location.y, insert.location.z],
            insert.rotation,
            if insert.x_scale_factor == 0.0 {
                1.0
            } else {
                insert.x_scale_factor
            },
            if insert.y_scale_factor == 0.0 {
                1.0
            } else {
                insert.y_scale_factor
            },
            if insert.z_scale_factor == 0.0 {
                1.0
            } else {
                insert.z_scale_factor
            },
        ));
        if let Some(block) = self.blocks.get(&insert.name).cloned() {
            self.loss.record("blocco INSERT esploso", 1);
            for entity in &block.entities {
                self.walk_entity(entity, composed, layer, depth + 1, visiting)?;
            }
        } else if insert.attributes().next().is_none() {
            return Err(err(&PublicMessage::Curated("blocco INSERT assente")));
        }
        visiting.remove(&insert.name);
        Ok(())
    }
}

fn value_coordinate_count(value: &WkbValue) -> usize {
    match value {
        WkbValue::Point(_) => 1,
        WkbValue::LineString(coordinates) | WkbValue::CircularString(coordinates) => {
            coordinates.len()
        }
        WkbValue::Polygon(rings) | WkbValue::Triangle(rings) => rings.iter().map(Vec::len).sum(),
        WkbValue::MultiPoint(children)
        | WkbValue::MultiLineString(children)
        | WkbValue::MultiPolygon(children)
        | WkbValue::GeometryCollection(children)
        | WkbValue::CompoundCurve(children)
        | WkbValue::CurvePolygon(children)
        | WkbValue::MultiCurve(children)
        | WkbValue::MultiSurface(children)
        | WkbValue::PolyhedralSurface(children)
        | WkbValue::Tin(children) => children
            .iter()
            .map(|geometry| value_coordinate_count(&geometry.value))
            .sum(),
    }
}

/// Punti di una polilinea con eventuali archi (bulge) tassellati fra i vertici.
fn polyline_positions(
    vertices: &[([f64; 3], f64)],
    closed: bool,
    transform: &Transform3,
) -> Vec<WkbCoordinate> {
    let mut output = Vec::with_capacity(vertices.len());
    let n = vertices.len();
    for i in 0..n {
        let (point, bulge) = vertices[i];
        output.push(coordinate(transform.apply(point)));
        let next = if i + 1 < n {
            Some(vertices[i + 1].0)
        } else if closed {
            Some(vertices[0].0)
        } else {
            None
        };
        if let Some(next) = next {
            for arc_point in tessellate_bulge(
                [point[0], point[1]],
                [next[0], next[1]],
                bulge,
                ARC_SEGMENTS,
            ) {
                output.push(coordinate(transform.apply([
                    arc_point[0],
                    arc_point[1],
                    point[2],
                ])));
            }
        }
    }
    output
}

fn coordinates_have_nonzero_z(coordinates: &[WkbCoordinate]) -> bool {
    coordinates
        .iter()
        .any(|coordinate| coordinate.z.is_some_and(|z| z != 0.0))
}

fn geometry_has_nonzero_z(geometry: &WkbGeometry) -> bool {
    match &geometry.value {
        WkbValue::Point(coordinate) => coordinate.z.is_some_and(|z| z != 0.0),
        WkbValue::LineString(coordinates) | WkbValue::CircularString(coordinates) => {
            coordinates_have_nonzero_z(coordinates)
        }
        WkbValue::Polygon(rings) | WkbValue::Triangle(rings) => {
            rings.iter().any(|ring| coordinates_have_nonzero_z(ring))
        }
        WkbValue::MultiPoint(children)
        | WkbValue::MultiLineString(children)
        | WkbValue::MultiPolygon(children)
        | WkbValue::GeometryCollection(children)
        | WkbValue::CompoundCurve(children)
        | WkbValue::CurvePolygon(children)
        | WkbValue::MultiCurve(children)
        | WkbValue::MultiSurface(children)
        | WkbValue::PolyhedralSurface(children)
        | WkbValue::Tin(children) => children.iter().any(geometry_has_nonzero_z),
    }
}

fn set_geometry_dimensions(geometry: &mut WkbGeometry, dimensions: CoordinateDimensions) {
    let set_coordinates = |coordinates: &mut [WkbCoordinate]| {
        for coordinate in coordinates {
            match dimensions {
                CoordinateDimensions::Xy => coordinate.z = None,
                CoordinateDimensions::Xyz => {
                    coordinate.z.get_or_insert(0.0);
                }
                _ => {}
            }
        }
    };
    match &mut geometry.value {
        WkbValue::Point(coordinate) => match dimensions {
            CoordinateDimensions::Xy => coordinate.z = None,
            CoordinateDimensions::Xyz => {
                coordinate.z.get_or_insert(0.0);
            }
            _ => {}
        },
        WkbValue::LineString(coordinates) | WkbValue::CircularString(coordinates) => {
            set_coordinates(coordinates);
        }
        WkbValue::Polygon(rings) | WkbValue::Triangle(rings) => {
            for ring in rings {
                set_coordinates(ring);
            }
        }
        WkbValue::MultiPoint(children)
        | WkbValue::MultiLineString(children)
        | WkbValue::MultiPolygon(children)
        | WkbValue::GeometryCollection(children)
        | WkbValue::CompoundCurve(children)
        | WkbValue::CurvePolygon(children)
        | WkbValue::MultiCurve(children)
        | WkbValue::MultiSurface(children)
        | WkbValue::PolyhedralSurface(children)
        | WkbValue::Tin(children) => {
            for child in children {
                set_geometry_dimensions(child, dimensions);
            }
        }
    }
    geometry.dimensions = dimensions;
}

#[derive(Default)]
struct DxfContractStats {
    has_nonzero_z: bool,
    geometry_types: BTreeSet<GeometryType>,
}

impl DxfContractStats {
    fn observe(&mut self, walker: &Walker, cancellation: &CancellationToken) -> Result<()> {
        for (index, geometry) in walker.geometries.iter().flatten().enumerate() {
            check_cancelled_periodically(cancellation, ErrorPhase::Read, index)?;
            self.has_nonzero_z |= geometry_has_nonzero_z(geometry);
            self.geometry_types.insert(geometry.geometry_type());
        }
        Ok(())
    }

    const fn dimensions(&self) -> CoordinateDimensions {
        if self.has_nonzero_z {
            CoordinateDimensions::Xyz
        } else {
            CoordinateDimensions::Xy
        }
    }
}

fn dxf_contract(
    crs: ResolvedCrs,
    dimensions: CoordinateDimensions,
    geometry_types: BTreeSet<GeometryType>,
) -> Result<DataContract> {
    // L'etichetta CRS viene estratta prima di cedere `crs` al contratto: nessuna
    // copia dell'intero CRS risolto e nessun parametro non consumato.
    let crs_label = crs.id.clone().ok_or_else(|| {
        PlenoraIoError::crs_redatto(&PublicMessage::Curated(
            "DXF: CRS risolto senza identificatore; vietato inventare DXF:GEODATA",
        ))
    })?;
    let mut geometry_contract = GeometryColumnContract::wkb_xy(FieldId(0), GEOMETRY, crs, true);
    geometry_contract.dimensions = dimensions;
    geometry_contract.set_exact_geometry_types(geometry_types.into_iter().collect());
    geometry_contract
        .native_metadata
        .insert("dxf.geometry_model".to_owned(), "wcs".to_owned());
    geometry_contract.native_metadata.insert(
        "dxf.z_inference".to_owned(),
        "xyz_if_any_nonzero_z_else_xy".to_owned(),
    );
    let fields = vec![
        with_geometry_contract_metadata(&geometry_field(GEOMETRY, &crs_label), &geometry_contract),
        Field::new("layer", DataType::Utf8, true),
        Field::new("dxf_type", DataType::Utf8, true),
        Field::new("text", DataType::Utf8, true),
    ];
    let schema: SchemaRef = Arc::new(Schema::new(fields));
    Ok(DataContract::new(schema, Some(geometry_contract)))
}

fn batch_from_walker(
    walker: &mut Walker,
    contract: &DataContract,
    cancellation: &CancellationToken,
) -> Result<RecordBatch> {
    let dimensions = contract
        .geometry
        .as_ref()
        .map_or(CoordinateDimensions::Unknown, |geometry| {
            geometry.dimensions
        });
    let mut geometries = std::mem::take(&mut walker.geometries);
    let mut encoded = Vec::with_capacity(geometries.len());
    for (index, geometry) in geometries.iter_mut().enumerate() {
        check_cancelled_periodically(cancellation, ErrorPhase::Read, index)?;
        let Some(geometry) = geometry else {
            encoded.push(None);
            continue;
        };
        set_geometry_dimensions(geometry, dimensions);
        encoded.push(Some(encode_wkb(geometry, WkbFlavor::Iso)?));
    }
    let arrays: Vec<ArrayRef> = vec![
        Arc::new(BinaryArray::from(
            encoded
                .iter()
                .map(|value| value.as_deref())
                .collect::<Vec<_>>(),
        )),
        Arc::new(StringArray::from(std::mem::take(&mut walker.layers))),
        Arc::new(StringArray::from(std::mem::take(&mut walker.types))),
        Arc::new(StringArray::from(std::mem::take(&mut walker.texts))),
    ];
    RecordBatch::try_new(contract.schema.clone(), arrays).map_err(|_| {
        err(&PublicMessage::Curated(
            "batch DXF progressivo non costruibile",
        ))
    })
}

/// Quote consultate dal percorso non streaming del driver.
///
/// Sono tre scalari e non un `Limits`: nel modello unificato quel tipo non
/// esiste, e portarselo dietro per tre campi lo terrebbe in vita oltre la
/// migrazione.
#[derive(Clone, Copy)]
struct DxfQuote {
    colonne: usize,
    righe: usize,
    vertici: usize,
}

impl DxfQuote {
    /// Quote per i percorsi che non ricevono opzioni: il fuzz harness e i
    /// test unitari del batch.
    ///
    /// Costanti esplicite e non le opzioni predefinite. Passare da quelle
    /// legherebbe un harness di fuzzing e due test unitari al
    /// **default di produzione**, che la migrazione sta cambiando: il primo
    /// effetto e' stato far salire di uno l'inventario legacy, e il secondo
    /// sarebbe stato veder cambiare i limiti del fuzz quando cambia il
    /// modello. Qui servono soltanto bound stabili che impediscano un OOM.
    const fn predefinite() -> Self {
        Self {
            colonne: 4_096,
            righe: 10_000_000,
            vertici: 50_000_000,
        }
    }

    fn from_read_options(opts: &ReadOptions) -> Self {
        Self {
            colonne: opts.max_columns(),
            righe: opts.max_rows(),
            vertici: opts.max_vertices(),
        }
    }
}

fn build_batch(
    drawing: &Drawing,
    crs: ResolvedCrs,
    quote: DxfQuote,
) -> Result<(RecordBatch, LossReport, DataContract)> {
    build_batch_cancellable(drawing, crs, quote, &CancellationToken::new())
}

fn build_batch_cancellable(
    drawing: &Drawing,
    crs: ResolvedCrs,
    quote: DxfQuote,
    cancellation: &CancellationToken,
) -> Result<(RecordBatch, LossReport, DataContract)> {
    const DXF_OUTPUT_COLUMNS: usize = 4;

    check_cancelled(cancellation, ErrorPhase::Read)?;
    if quote.colonne < DXF_OUTPUT_COLUMNS {
        return Err(PlenoraIoError::limite_redatto(
            &PublicMessage::CuratedBetween(
                "DXF produce",
                NumeroStrutturale::Conteggio(driver_common::saturating_u64(DXF_OUTPUT_COLUMNS)),
                "colonne, oltre il limite di",
                NumeroStrutturale::Limite(driver_common::saturating_u64(quote.colonne)),
            ),
        ));
    }
    let mut walker = Walker::new(drawing, quote, cancellation)?;
    let mut visiting: HashSet<String> = HashSet::new();
    for e in drawing.entities() {
        walker.walk_entity(e, Transform3::IDENTITY, "0", 0, &mut visiting)?;
    }
    let mut stats = DxfContractStats::default();
    stats.observe(&walker, cancellation)?;
    let contract = dxf_contract(crs, stats.dimensions(), stats.geometry_types)?;
    let batch = batch_from_walker(&mut walker, &contract, cancellation)?;
    Ok((batch, walker.loss, contract))
}

/// Entry point non stabile per libFuzzer: esercita il parser DXF e l'intero
/// walker geometrico senza I/O su filesystem. Il CRS sintetico serve solo a
/// completare il contratto dopo il parsing.
#[doc(hidden)]
pub fn __fuzz_read_dxf(bytes: &[u8]) -> Result<usize> {
    use std::io::Cursor;

    const MAX_FUZZ_INPUT_BYTES: usize = 1_048_576;
    if bytes.len() > MAX_FUZZ_INPUT_BYTES {
        return Err(err(&PublicMessage::CuratedWith(
            "input fuzz DXF oltre il limite di byte:",
            NumeroStrutturale::Limite(driver_common::saturating_u64(MAX_FUZZ_INPUT_BYTES)),
        )));
    }
    let drawing = Drawing::load(&mut Cursor::new(bytes))
        .map_err(|_| err(&PublicMessage::Curated("DXF non valido")))?;
    let crs = ResolvedCrs::new(Some("EPSG:4326".to_owned()), CrsKind::Geographic, None);
    let (batch, _, _) = build_batch(&drawing, crs, DxfQuote::predefinite())?;
    Ok(batch.num_rows())
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

    fn opzioni_lettura() -> ReadOptions {
        match plenora_io_model::budget::PipelineBudget::builder().build() {
            Ok(bundle) => ReadOptions::from_read_parts(bundle.into_read_parts()),
            Err(error) => unreachable!("bundle di test non costruibile: {error:?}"),
        }
    }

    use plenora_io_core::request::{BatchTarget, ProjectionMode, ReadScope};
    use plenora_io_core::WriteLayer;
    use plenora_io_model::contract::GeometryType;
    use plenora_io_model::crs::CrsResolution;

    #[test]
    fn bounded_output_fails_before_exceeding_the_limit() {
        let mut output = BoundedOutput::new(Vec::new(), 3);
        output.write_all(b"ab").unwrap();
        assert!(output.write_all(b"cd").is_err());
        assert!(output.exceeded());
        assert_eq!(output.into_inner(), b"ab");
    }

    #[test]
    fn spool_spills_to_file_without_changing_the_row() {
        let mut spool = DxfSpoolWriter::with_memory_limit(4096, 1);
        spool
            .push(DxfSpoolRow {
                geometry: Some(WkbGeometry {
                    value: WkbValue::Point(WkbCoordinate {
                        x: 1.0,
                        y: 2.0,
                        z: None,
                        m: None,
                    }),
                    dimensions: CoordinateDimensions::Xy,
                    srid: None,
                }),
                layer: Some("layer".to_owned()),
                entity_type: Some("POINT".to_owned()),
                text: None,
            })
            .unwrap();
        let storage = spool.finish().unwrap();
        assert!(matches!(storage, DxfSpoolStorage::File(_)));
        let mut reader = storage.reader().unwrap();
        let row = reader
            .next_row(CoordinateDimensions::Xy, &WkbLimits::default())
            .unwrap();
        assert_eq!(row.layer.as_deref(), Some("layer"));
        assert_eq!(row.entity_type.as_deref(), Some("POINT"));
        let geometry = decode_wkb(row.geometry.as_deref().unwrap(), &WkbLimits::default()).unwrap();
        assert_eq!(geometry.dimensions, CoordinateDimensions::Xy);
        assert!(matches!(geometry.value, WkbValue::Point(_)));
    }

    fn resolved_wgs84() -> ResolvedCrs {
        ResolvedCrs::new(
            Some("EPSG:4326".to_owned()),
            CrsKind::Geographic,
            Some(WGS84_ESRI_WKT.to_owned()),
        )
    }

    fn wkb(value: WkbValue, dimensions: CoordinateDimensions) -> Vec<u8> {
        encode_wkb(
            &WkbGeometry {
                value,
                dimensions,
                srid: None,
            },
            WkbFlavor::Iso,
        )
        .unwrap()
    }

    fn request() -> ReadRequest {
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

    // Un unico round-trip copre scrittura XYZ, CRS GEODATA embedded e
    // rilettura: separarlo duplicherebbe la fixture e ne perderebbe la catena.
    #[allow(clippy::too_many_lines)]
    #[test]
    fn write_then_read_xyz_and_embedded_crs_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("out.dxf");
        let wkb1 = wkb(
            WkbValue::Point(WkbCoordinate {
                x: 1.0,
                y: 2.0,
                z: Some(7.5),
                m: None,
            }),
            CoordinateDimensions::Xyz,
        );
        let wkb2 = wkb(
            WkbValue::LineString(vec![
                WkbCoordinate {
                    x: 3.0,
                    y: 4.0,
                    z: Some(8.0),
                    m: None,
                },
                WkbCoordinate {
                    x: 5.0,
                    y: 6.0,
                    z: Some(9.0),
                    m: None,
                },
            ]),
            CoordinateDimensions::Xyz,
        );
        let mut geometry_contract = GeometryColumnContract::wkb_xy(
            FieldId(0),
            GEOMETRY,
            CrsResolution::resolved(resolved_wgs84()),
            true,
        );
        geometry_contract.dimensions = CoordinateDimensions::Xyz;
        geometry_contract
            .set_exact_geometry_types(vec![GeometryType::Point, GeometryType::LineString]);
        let field = with_geometry_contract_metadata(
            &geometry_field(GEOMETRY, "EPSG:4326"),
            &geometry_contract,
        );
        let schema: SchemaRef = Arc::new(Schema::new(vec![
            field,
            Field::new("val", DataType::Int64, true),
        ]));
        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![
                Arc::new(BinaryArray::from(vec![
                    Some(wkb1.as_slice()),
                    Some(wkb2.as_slice()),
                ])),
                Arc::new(arrow_array::Int64Array::from(vec![1i64, 2])),
            ],
        )
        .unwrap();

        let driver = DxfDriver;
        let plan = WritePlan {
            layers: vec![WriteLayer {
                name: "l".to_owned(),
                contract: DataContract {
                    schema,
                    geometry: Some(geometry_contract),
                },
            }],
        };
        let mut w = driver
            .create(Sink::Path(out.clone()), &plan, &opzioni_scrittura())
            .unwrap();
        let planned_fidelity = w.fidelity_assessment();
        assert_eq!(
            planned_fidelity.level,
            plenora_io_core::Fidelity::Approximating
        );
        assert!(planned_fidelity
            .ragioni_v1()
            .iter()
            .any(|reason| { reason.code == plenora_io_core::FidelityReasonCode::AttributeLoss }));
        w.write(&batch).unwrap();
        let published = w.finish().unwrap();
        // "val" non è rappresentabile in DXF -> dichiarato come perdita (Approximating).
        assert!(!published.loss.is_empty());
        assert_eq!(
            published.fidelity.level,
            plenora_io_core::Fidelity::Approximating
        );
        assert!(published
            .fidelity
            .ragioni_v1()
            .iter()
            .any(|reason| reason.detail.contains("occorrenze")));

        let ds = driver.open(Source::Path(out), opzioni_lettura()).unwrap();
        assert_eq!(
            ds.fidelity_assessment().level,
            plenora_io_core::Fidelity::Approximating
        );
        let output_geometry = ds.layers()[0].contract.geometry.as_ref().unwrap();
        assert_eq!(output_geometry.dimensions, CoordinateDimensions::Xyz);
        assert_eq!(output_geometry.crs.id(), Some("EPSG:4326"));
        let mut r = ds.open_layer_reader(&request()).unwrap();
        let rb = r.next_batch().unwrap().unwrap();
        assert_eq!(rb.num_rows(), 2);
        let geometry = rb.column(0).as_any().downcast_ref::<BinaryArray>().unwrap();
        let decoded: Vec<WkbGeometry> = (0..geometry.len())
            .map(|index| decode_wkb(geometry.value(index), &WkbLimits::default()).unwrap())
            .collect();
        assert!(decoded
            .iter()
            .all(|value| value.dimensions == CoordinateDimensions::Xyz));
        assert!(decoded.iter().any(|value| matches!(
            &value.value,
            WkbValue::Point(point) if point.z == Some(7.5)
        )));
        assert!(decoded.iter().any(|value| matches!(
            &value.value,
            WkbValue::LineString(points)
                if points.first().and_then(|point| point.z) == Some(8.0)
                    && points.last().and_then(|point| point.z) == Some(9.0)
        )));
    }

    #[test]
    fn insert_block_is_exploded_and_translated() {
        // Blocco BOX = una LINE (0,0)-(1,0) su layer "0"; un INSERT lo colloca a
        // (10,5) su layer MYLAYER. L'esplosione deve tradurre la LINE a
        // (10,5)-(11,5) ed ereditare il layer dell'INSERT.
        let dxf = "\
0\nSECTION\n2\nBLOCKS\n\
0\nBLOCK\n8\n0\n2\nBOX\n10\n0.0\n20\n0.0\n30\n0.0\n\
0\nLINE\n8\n0\n10\n0.0\n20\n0.0\n30\n0.0\n11\n1.0\n21\n0.0\n31\n0.0\n\
0\nENDBLK\n\
0\nENDSEC\n\
0\nSECTION\n2\nENTITIES\n\
0\nINSERT\n8\nMYLAYER\n2\nBOX\n10\n10.0\n20\n5.0\n30\n0.0\n\
0\nENDSEC\n0\nEOF\n";
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("blk.dxf");
        std::fs::write(&path, dxf).unwrap();
        let drawing = Drawing::load_file(&path).unwrap();
        let (batch, _loss, _contract) =
            build_batch(&drawing, resolved_wgs84(), DxfQuote::predefinite()).unwrap();

        let geom = batch
            .column(0)
            .as_any()
            .downcast_ref::<BinaryArray>()
            .unwrap();
        let layers = batch
            .column(1)
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap();
        let types = batch
            .column(2)
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap();
        let mut found = false;
        for i in 0..batch.num_rows() {
            if types.value(i) == "LINE" {
                if let WkbValue::LineString(points) =
                    decode_wkb(geom.value(i), &WkbLimits::default())
                        .unwrap()
                        .value
                {
                    assert!((points[0].x - 10.0).abs() < 1e-6 && (points[0].y - 5.0).abs() < 1e-6);
                    assert!((points[1].x - 11.0).abs() < 1e-6 && (points[1].y - 5.0).abs() < 1e-6);
                    assert_eq!(layers.value(i), "MYLAYER");
                    found = true;
                }
            }
        }
        assert!(found, "la LINE del blocco esploso non è stata trovata");
    }

    #[test]
    fn missing_geodata_requires_explicit_assumption() {
        let drawing = Drawing::new();
        let error = resolve_dxf_crs(&drawing, &opzioni_lettura()).unwrap_err();
        assert!(error.to_string().contains("assume-crs"));

        let resolved =
            resolve_dxf_crs(&drawing, &opzioni_lettura().with_assume_crs("EPSG:3857")).unwrap();
        assert_eq!(resolved.id.as_deref(), Some("EPSG:3857"));
    }

    #[test]
    fn geodata_epsg_is_resolved_without_fallback() {
        let mut drawing = Drawing::new();
        drawing.add_object(Object::new(ObjectType::GeoData(GeoData {
            coordinate_system_definition: WGS84_ESRI_WKT.to_owned(),
            ..Default::default()
        })));
        let resolved = resolve_dxf_crs(&drawing, &opzioni_lettura()).unwrap();
        assert_eq!(resolved.id.as_deref(), Some("EPSG:4326"));
        assert_eq!(resolved.kind, CrsKind::Geographic);
        assert_eq!(resolved.definition.as_deref(), Some(WGS84_ESRI_WKT));
    }

    #[test]
    fn unresolved_geodata_is_preserved_in_typed_error() {
        let definition = "LOCAL_CS[\"survey-grid-secret\"]";
        let mut drawing = Drawing::new();
        drawing.add_object(Object::new(ObjectType::GeoData(GeoData {
            coordinate_system_definition: definition.to_owned(),
            ..Default::default()
        })));

        let error = resolve_dxf_crs(&drawing, &opzioni_lettura()).unwrap_err();
        assert_eq!(error.code, plenora_io_model::IoErrorCode::CrsUnresolved);
        assert_eq!(error.driver.as_deref(), Some("dxf"));
        assert!(!error.to_string().contains("survey-grid-secret"));
    }

    #[test]
    fn fuzz_entrypoint_accepts_minimal_ascii_dxf() {
        let dxf = b"0\nSECTION\n2\nENTITIES\n0\nPOINT\n10\n1\n20\n2\n30\n3\n0\nENDSEC\n0\nEOF\n";
        assert_eq!(__fuzz_read_dxf(dxf).unwrap(), 1);
    }

    /// Un `BLOCK` che non arriva mai a `ENDBLK` finisce, invece di non finire.
    ///
    /// Undici righe tenevano il lettore occupato per sempre. `Entity::read`
    /// restituisce `Ok(None)` senza consumare niente quando trova `0/ENDSEC`,
    /// e il ciclo di `read_block` riprendeva la stessa coppia all'infinito,
    /// allocando a ogni giro: non lentezza, lavoro senza fine, su un ingresso
    /// che chiunque puo' fabbricare.
    ///
    /// L'ha trovato la fuzz smoke -- e l'ha trovato solo dopo che il job ha
    /// ricominciato a costruire tutti i target. Qui la stessa cosa e' una prova
    /// che costa millisecondi e non dipende da quale input il fuzzer peschi.
    ///
    /// La prova e' che **ritorni**: `assert` sull'esito verrebbe dopo, e se il
    /// difetto tornasse questa prova non fallirebbe -- resterebbe appesa. E' il
    /// motivo per cui il seme versionato in `fuzz/seeds/dxf_reader/` le sta
    /// accanto: li' il tetto di libFuzzer trasforma l'attesa in un rosso.
    #[test]
    fn un_blocco_senza_endblk_non_gira_a_vuoto() {
        let dxf = b"0\nSECTION\n2\nBLOCKS\n0\nBLOCK\n2\nsenza-endblk\n0\nENDSEC\n0\nEOF\n";
        assert!(
            __fuzz_read_dxf(dxf).is_err(),
            "un BLOCK non terminato non e' un documento leggibile"
        );
    }

    #[test]
    fn row_level_dxf_failure_reports_the_top_level_entity_index() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("invalid-row.dxf");
        let mut drawing = Drawing::new();
        drawing.add_entity(Entity::new(EntityType::ModelPoint(ModelPoint {
            location: DxfPoint::new(1.0, 2.0, 3.0),
            ..Default::default()
        })));
        drawing.add_entity(Entity::new(EntityType::Circle(
            dxf::entities::Circle::default(),
        )));
        let mut file = File::create(&path).unwrap();
        drawing.save(&mut file).unwrap();

        let error = DxfDriver
            .open(
                Source::Path(path),
                opzioni_lettura().with_assume_crs("EPSG:4326"),
            )
            .err()
            .expect("il CIRCLE degenere deve essere rifiutato");
        let diagnostics = error.row_diagnostics.as_deref().unwrap();
        assert_eq!(diagnostics.examples[0].source_index, 1);
        assert_eq!(diagnostics.counts["dxf.entity_not_representable"], 1);
        assert!(diagnostics.validate().is_ok());
    }

    #[test]
    fn read_limits_are_enforced_before_batch_creation() {
        let mut drawing = Drawing::new();
        drawing.add_entity(Entity::new(EntityType::ModelPoint(ModelPoint {
            location: DxfPoint::new(1.0, 2.0, 3.0),
            ..Default::default()
        })));
        let row_error = build_batch(
            &drawing,
            resolved_wgs84(),
            DxfQuote {
                righe: 0,
                ..DxfQuote::predefinite()
            },
        )
        .unwrap_err();
        assert_eq!(row_error.code, plenora_io_model::IoErrorCode::LimitExceeded);

        let column_error = build_batch(
            &drawing,
            resolved_wgs84(),
            DxfQuote {
                colonne: 3,
                ..DxfQuote::predefinite()
            },
        )
        .unwrap_err();
        assert_eq!(
            column_error.code,
            plenora_io_model::IoErrorCode::LimitExceeded
        );
    }

    #[test]
    fn unsupported_m_is_rejected_before_output_creation() {
        let directory = tempfile::tempdir().unwrap();
        let output = directory.path().join("m.dxf");
        let mut geometry = GeometryColumnContract::wkb_xy(
            FieldId(0),
            GEOMETRY,
            CrsResolution::resolved(resolved_wgs84()),
            true,
        );
        geometry.dimensions = CoordinateDimensions::Xym;
        geometry.set_exact_geometry_types(vec![GeometryType::Point]);
        let schema: SchemaRef = Arc::new(Schema::new(vec![with_geometry_contract_metadata(
            &geometry_field(GEOMETRY, "EPSG:4326"),
            &geometry,
        )]));
        let plan = WritePlan {
            layers: vec![WriteLayer {
                name: "m".to_owned(),
                contract: DataContract {
                    schema,
                    geometry: Some(geometry),
                },
            }],
        };

        assert!(DxfDriver
            .create(Sink::Path(output.clone()), &plan, &opzioni_scrittura())
            .is_err());
        assert!(!output.exists());
    }

    #[test]
    fn dxf_conversion_rejects_non_finite_coordinates() {
        for coordinate in [
            WkbCoordinate {
                x: f64::NAN,
                y: 0.0,
                z: None,
                m: None,
            },
            WkbCoordinate {
                x: 0.0,
                y: f64::INFINITY,
                z: None,
                m: None,
            },
            WkbCoordinate {
                x: 0.0,
                y: 0.0,
                z: Some(f64::NEG_INFINITY),
                m: None,
            },
        ] {
            let error = point_entity(&coordinate).unwrap_err();
            assert_eq!(error.category, plenora_io_model::ErrorCategory::DataMapping);
            assert!(error.message.contains("coordinate non finite"));
        }
    }

    #[test]
    fn declared_input_total_enables_dxf_specific_row_diagnostics() {
        let directory = tempfile::tempdir().unwrap();
        let output = directory.path().join("null.dxf");
        let mut geometry = GeometryColumnContract::wkb_xy(
            FieldId(0),
            GEOMETRY,
            CrsResolution::resolved(resolved_wgs84()),
            true,
        );
        geometry.set_exact_geometry_types(vec![GeometryType::Point]);
        let schema: SchemaRef = Arc::new(Schema::new(vec![with_geometry_contract_metadata(
            &geometry_field(GEOMETRY, "EPSG:4326"),
            &geometry,
        )]));
        let plan = WritePlan {
            layers: vec![WriteLayer {
                name: "null".to_owned(),
                contract: DataContract {
                    schema: schema.clone(),
                    geometry: Some(geometry),
                },
            }],
        };
        let batch = RecordBatch::try_new(
            schema,
            vec![Arc::new(BinaryArray::from(vec![None::<&[u8]>]))],
        )
        .unwrap();
        let mut writer = DxfDriver
            .create(Sink::Path(output.clone()), &plan, &opzioni_scrittura())
            .unwrap();
        writer.declare_input_total(LayerId(0), 1).unwrap();

        let error = writer.write(&batch).unwrap_err();
        let diagnostics = error.row_diagnostics.as_deref().unwrap();
        assert_eq!(diagnostics.input_total, Some(1));
        assert_eq!(diagnostics.observed_total, 1);
        assert_eq!(diagnostics.examples[0].source_index, 0);
        assert_eq!(
            diagnostics.counts.get("dxf.null_geometry_unsupported"),
            Some(&1)
        );
        assert!(diagnostics.validate().is_ok());
        assert!(!output.exists());
    }

    #[test]
    fn writer_adapter_attributes_non_finite_dxf_row_and_prevents_publish() {
        let directory = tempfile::tempdir().unwrap();
        let output = directory.path().join("nan.dxf");
        let mut geometry = GeometryColumnContract::wkb_xy(
            FieldId(0),
            GEOMETRY,
            CrsResolution::resolved(resolved_wgs84()),
            false,
        );
        geometry.set_exact_geometry_types(vec![GeometryType::Point]);
        let schema: SchemaRef = Arc::new(Schema::new(vec![with_geometry_contract_metadata(
            &geometry_field(GEOMETRY, "EPSG:4326"),
            &geometry,
        )]));
        let plan = WritePlan {
            layers: vec![WriteLayer {
                name: "nan".to_owned(),
                contract: DataContract {
                    schema: schema.clone(),
                    geometry: Some(geometry),
                },
            }],
        };
        let bytes = encode_wkb(
            &WkbGeometry {
                value: WkbValue::Point(WkbCoordinate {
                    x: f64::NAN,
                    y: 1.0,
                    z: None,
                    m: None,
                }),
                dimensions: CoordinateDimensions::Xy,
                srid: None,
            },
            WkbFlavor::Iso,
        )
        .unwrap();
        let batch = RecordBatch::try_new(
            schema,
            vec![Arc::new(BinaryArray::from(vec![Some(bytes.as_slice())]))],
        )
        .unwrap();
        let mut writer = DxfDriver
            .create(Sink::Path(output.clone()), &plan, &opzioni_scrittura())
            .unwrap();
        writer.declare_input_total(LayerId(0), 1).unwrap();

        let error = writer.write(&batch).unwrap_err();
        let diagnostics = error.row_diagnostics.as_deref().unwrap();
        assert_eq!(diagnostics.examples[0].source_index, 0);
        assert_eq!(diagnostics.counts["dxf.non_finite_coordinate"], 1);
        assert!(diagnostics.validate().is_ok());
        assert!(writer.finish().is_err());
        assert!(!output.exists());
    }

    fn dxf_writer_for_geometry_type(
        output: &std::path::Path,
        geometry_type: GeometryType,
        input_total: u64,
    ) -> (Box<dyn FormatWriter>, SchemaRef) {
        let mut geometry = GeometryColumnContract::wkb_xy(
            FieldId(0),
            GEOMETRY,
            CrsResolution::resolved(resolved_wgs84()),
            false,
        );
        geometry.set_exact_geometry_types(vec![geometry_type]);
        let schema: SchemaRef = Arc::new(Schema::new(vec![with_geometry_contract_metadata(
            &geometry_field(GEOMETRY, "EPSG:4326"),
            &geometry,
        )]));
        let plan = WritePlan {
            layers: vec![WriteLayer {
                name: "geometry".to_owned(),
                contract: DataContract {
                    schema: schema.clone(),
                    geometry: Some(geometry),
                },
            }],
        };
        let mut writer = DxfDriver
            .create(
                Sink::Path(output.to_path_buf()),
                &plan,
                &opzioni_scrittura(),
            )
            .unwrap();
        writer.declare_input_total(LayerId(0), input_total).unwrap();
        (writer, schema)
    }

    #[test]
    fn writer_adapter_rejects_empty_multipart_and_collection_without_publish() {
        let point = || WkbGeometry {
            value: WkbValue::Point(WkbCoordinate {
                x: 1.0,
                y: 2.0,
                z: None,
                m: None,
            }),
            dimensions: CoordinateDimensions::Xy,
            srid: None,
        };
        for (name, geometry_type, valid_value, empty_value) in [
            (
                "multipoint",
                GeometryType::MultiPoint,
                WkbValue::MultiPoint(vec![point()]),
                WkbValue::MultiPoint(Vec::new()),
            ),
            (
                "collection",
                GeometryType::GeometryCollection,
                WkbValue::GeometryCollection(vec![point()]),
                WkbValue::GeometryCollection(Vec::new()),
            ),
        ] {
            let directory = tempfile::tempdir().unwrap();
            let output = directory.path().join(format!("{name}.dxf"));
            let (mut writer, schema) = dxf_writer_for_geometry_type(&output, geometry_type, 2);
            let valid = wkb(valid_value, CoordinateDimensions::Xy);
            let valid_batch = RecordBatch::try_new(
                schema.clone(),
                vec![Arc::new(BinaryArray::from(vec![Some(valid.as_slice())]))],
            )
            .unwrap();
            writer.write(&valid_batch).unwrap();
            let empty = wkb(empty_value, CoordinateDimensions::Xy);
            let batch = RecordBatch::try_new(
                schema,
                vec![Arc::new(BinaryArray::from(vec![Some(empty.as_slice())]))],
            )
            .unwrap();

            let error = writer.write(&batch).unwrap_err();
            let diagnostics = error.row_diagnostics.as_deref().unwrap();
            assert_eq!(diagnostics.observed_total, 1);
            assert_eq!(diagnostics.examples.len(), 1);
            assert_eq!(diagnostics.examples[0].source_index, 1);
            assert_eq!(diagnostics.counts["dxf.empty_geometry_unsupported"], 1);
            assert_eq!(diagnostics.input_total, Some(2));
            assert_eq!(
                diagnostics
                    .write_outcome
                    .as_ref()
                    .unwrap()
                    .certainly_rejected,
                plenora_io_model::KnownOrUnknownCount::Known { value: 1 }
            );
            assert!(diagnostics.validate().is_ok());
            assert!(writer.write(&batch).is_err(), "poison deve essere sticky");
            assert!(writer.finish().is_err());
            assert!(!output.exists());
        }
    }

    #[test]
    fn empty_supported_collections_and_nested_empty_use_the_stable_cause() {
        let empty = "dxf.empty_geometry_unsupported";
        assert_eq!(
            dxf_geometry_rejection_cause(&WkbValue::MultiLineString(Vec::new())),
            Some(empty)
        );
        assert_eq!(
            dxf_geometry_rejection_cause(&WkbValue::MultiPolygon(Vec::new())),
            Some(empty)
        );
        assert_eq!(
            dxf_geometry_rejection_cause(&WkbValue::GeometryCollection(vec![WkbGeometry {
                value: WkbValue::MultiPoint(Vec::new()),
                dimensions: CoordinateDimensions::Xy,
                srid: None,
            }])),
            Some(empty)
        );
        assert_eq!(
            dxf_geometry_rejection_cause(&WkbValue::CompoundCurve(Vec::new())),
            Some("dxf.geometry_type_unsupported")
        );
    }

    #[test]
    fn non_empty_multipoint_preserves_explosion_fidelity_and_entities() {
        let directory = tempfile::tempdir().unwrap();
        let output = directory.path().join("multipoint.dxf");
        let (mut writer, schema) =
            dxf_writer_for_geometry_type(&output, GeometryType::MultiPoint, 1);
        let bytes = wkb(
            WkbValue::MultiPoint(vec![
                WkbGeometry {
                    value: WkbValue::Point(WkbCoordinate {
                        x: 1.0,
                        y: 2.0,
                        z: None,
                        m: None,
                    }),
                    dimensions: CoordinateDimensions::Xy,
                    srid: None,
                },
                WkbGeometry {
                    value: WkbValue::Point(WkbCoordinate {
                        x: 3.0,
                        y: 4.0,
                        z: None,
                        m: None,
                    }),
                    dimensions: CoordinateDimensions::Xy,
                    srid: None,
                },
            ]),
            CoordinateDimensions::Xy,
        );
        let batch = RecordBatch::try_new(
            schema,
            vec![Arc::new(BinaryArray::from(vec![Some(bytes.as_slice())]))],
        )
        .unwrap();
        writer.write(&batch).unwrap();
        let published = writer.finish().unwrap();
        assert_eq!(published.loss.counts["MultiPoint esploso in entità DXF"], 2);

        let dataset = DxfDriver
            .open(Source::Path(output), opzioni_lettura())
            .unwrap();
        let mut reader = dataset.open_layer_reader(&request()).unwrap();
        assert_eq!(reader.next_batch().unwrap().unwrap().num_rows(), 2);
    }

    #[test]
    fn missing_geometry_contract_is_rejected_before_output_creation() {
        let directory = tempfile::tempdir().unwrap();
        let output = directory.path().join("missing-crs.dxf");
        let plan = WritePlan {
            layers: vec![WriteLayer {
                name: "empty".to_owned(),
                contract: DataContract {
                    schema: Arc::new(Schema::empty()),
                    geometry: None,
                },
            }],
        };
        assert!(DxfDriver
            .create(Sink::Path(output.clone()), &plan, &opzioni_scrittura())
            .is_err());
        assert!(!output.exists());
    }
}
