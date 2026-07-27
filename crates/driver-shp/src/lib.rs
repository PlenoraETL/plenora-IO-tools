//! driver-shp — Shapefile ⇄ RecordBatch. Le shape XY/M/Z diventano WKB
//! `geoarrow.wkb` XY/XYM/XYZ/XYZM senza passare da `geo-types`; il dbf fornisce
//! gli attributi e il `.prj` (o `assume_crs`) il CRS.
//!
//! Scrittura (Fase 2B): capability-check fail-closed (ADR-IO 3) — nomi campo dbf
//! ≤10 char (imposto da `FieldName`), tipo geometria unico per file (imposto da
//! shapefile). Il publish **multi-file** espone entrambe le modalità di ADR-IO 2:
//! `*.shp.d` è uno `ShapefileDirectoryDataset` pubblicato con un unico rename
//! atomico; `*.shp` è un `LooseShapefileSet` compatibile, pubblicato con rename
//! ordinati e `.shp` per ultimo. `.prj` è scritto se c'è una definizione WKT o
//! per WGS84; nessuna riproiezione (ADR-IO 4).
#![forbid(unsafe_code)]

use std::collections::{BTreeSet, HashMap};
use std::fs::File;
use std::io::BufWriter;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{sync_channel, Receiver};
use std::sync::Arc;

use arrow_array::builder::{
    BinaryBuilder, BooleanBuilder, Float64Builder, Int64Builder, StringBuilder,
};
use arrow_array::{Array, ArrayRef, BinaryArray, RecordBatch};
use arrow_schema::{DataType, Field, Schema, SchemaRef};
use serde_json::Value as JsonValue;
use shapefile::dbase::{FieldValue, Record, TableWriterBuilder};
use shapefile::{
    Multipoint, MultipointM, MultipointZ, Point, PointM, PointZ, Polygon, PolygonM, PolygonRing,
    PolygonZ, Polyline, PolylineM, PolylineZ, Shape, Writer, NO_DATA,
};

use driver_common::{geometry_field, json_from_array, ColType};
use plenora_core::contract::{
    CoordinateDimensions, DataContract, FieldId, GeometryColumnContract, GeometryType,
    LayerContract, LayerId,
};
use plenora_core::crs::{CrsKind, RawCrs, ResolvedCrs};
use plenora_core::geometry::{is_geometry_field, with_geometry_contract_metadata, GEO_CRS_KEY};
use plenora_core::limits::WkbLimits;
use plenora_core::wkb::{decode_wkb, encode_wkb, WkbCoordinate, WkbFlavor, WkbGeometry, WkbValue};
use plenora_core::{PlenoraError, Result};
use plenora_io_core::descriptor::{
    CrsHandling, Direction, Fidelity, FormatDescriptor, ReadMode, ReaderConcurrency, Runtime,
    WriteMode,
};
use plenora_io_core::driver::{
    FormatDriver, FormatWriter, LayerReader, OpenDatasetHandle, Published, ReadOptions, Sink,
    Source, WriteOptions,
};
use plenora_io_core::loss::LossReport;
use plenora_io_core::publish::{publish_dir_atomic, publish_files_ordered_limited};
use plenora_io_core::request::ReadRequest;
use plenora_io_core::{
    validate_write, with_write_validation, AttributeWriteSupport, CrsWriteSupport,
    FormatWriteCapabilities, NullabilitySupport, TypeCoercionPolicy, WritePlan, DBF_FIELD_NAMES,
    SCALAR_TYPES, WKB_SINGLE_TYPE_ALL_DIMENSIONS_GEOMETRY,
};

const GEOMETRY: &str = "geometry";
const DIRECTORY_DATASET_SUFFIX: &str = ".shp.d";
const DIRECTORY_DATASET_MODE: &str = "shapefile_directory_dataset";
const LOOSE_SET_MODE: &str = "loose_shapefile_set";

/// WKT standard per WGS84 (accettato da GDAL), usato per il `.prj` quando la
/// sorgente dà solo il codice autorità e non una definizione WKT.
const WGS84_WKT: &str = "GEOGCS[\"WGS 84\",DATUM[\"WGS_1984\",SPHEROID[\"WGS 84\",6378137,298.257223563]],PRIMEM[\"Greenwich\",0],UNIT[\"degree\",0.0174532925199433]]";

fn err(reason: impl Into<String>) -> PlenoraError {
    PlenoraError::Format {
        driver: "shp",
        reason: reason.into(),
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

fn publish_mode(path: &Path, opts: &WriteOptions) -> Result<ShapefilePublishMode> {
    let inferred = if is_directory_dataset_path(path) {
        ShapefilePublishMode::DirectoryDataset
    } else if path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("shp"))
    {
        ShapefilePublishMode::LooseSet
    } else {
        return Err(PlenoraError::Unsupported(
            "l'output Shapefile deve terminare con .shp (loose set) o .shp.d (directory dataset)"
                .to_owned(),
        ));
    };
    let Some(requested) = opts.format_options.get("publish_mode") else {
        return Ok(inferred);
    };
    let requested = match requested.as_str() {
        DIRECTORY_DATASET_MODE => ShapefilePublishMode::DirectoryDataset,
        LOOSE_SET_MODE => ShapefilePublishMode::LooseSet,
        other => {
            return Err(PlenoraError::Unsupported(format!(
                "publish_mode Shapefile '{other}' non valido; usare '{DIRECTORY_DATASET_MODE}' o '{LOOSE_SET_MODE}'"
            )))
        }
    };
    if requested != inferred {
        return Err(PlenoraError::Unsupported(format!(
            "publish_mode '{}' richiede una destinazione {}",
            requested.name(),
            requested.destination_suffix()
        )));
    }
    Ok(requested)
}

impl ShapefilePublishMode {
    fn name(self) -> &'static str {
        match self {
            Self::DirectoryDataset => DIRECTORY_DATASET_MODE,
            Self::LooseSet => LOOSE_SET_MODE,
        }
    }

    fn destination_suffix(self) -> &'static str {
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
        return Err(PlenoraError::Unsupported(format!(
            "directory Shapefile non riconosciuta: {} (atteso *.shp.d)",
            path.display()
        )));
    }
    let source = path.join("data.shp");
    if !source.is_file() {
        return Err(err(format!(
            "directory dataset senza data.shp: {}",
            path.display()
        )));
    }
    Ok(source)
}

static DESCRIPTOR: FormatDescriptor = FormatDescriptor {
    id: "shp",
    direction: Direction::Bidirectional,
    read_mode: ReadMode::StreamingSequential,
    write_mode: Some(WriteMode::Streaming),
    multi_layer: false,
    multi_file: true, // .shp/.shx/.dbf/.prj
    reader_concurrency: ReaderConcurrency::MultipleIndependentReaders,
    projection_support: plenora_io_core::ProjectionSupport::None,
    predicate_pruning_support: plenora_io_core::PredicatePruningSupport::None,
    spatial_pruning_support: plenora_io_core::SpatialPruningSupport::None,
    crs_handling: CrsHandling::Embedded,
    fidelity_class: Fidelity::Conditional,
    runtime: Runtime::PureRust,
    write_capabilities: Some(FormatWriteCapabilities {
        field_names: DBF_FIELD_NAMES,
        allowed_types: SCALAR_TYPES,
        type_coercion: TypeCoercionPolicy::ExplicitText,
        attributes: AttributeWriteSupport::All,
        geometry: WKB_SINGLE_TYPE_ALL_DIMENSIONS_GEOMETRY,
        crs: CrsWriteSupport::Embedded,
        nullability: NullabilitySupport::FormatDefined,
        multi_layer: false,
    }),
    semantic_version: 1,
    driver_version: 6,
    descriptor_version: 4,
};

pub struct ShpDriver;

impl FormatDriver for ShpDriver {
    fn descriptor(&self) -> &FormatDescriptor {
        &DESCRIPTOR
    }

    fn open(&self, source: Source, opts: &ReadOptions) -> Result<Box<dyn OpenDatasetHandle>> {
        let path = shapefile_source_path(source.into_path_checked(&opts.limits)?)?;
        let crs = resolve_crs(&path, opts)?;
        // Pass 1: inferenza schema (nomi + tipi) dai record, a RAM O(ncol).
        let (cols, geometry_info) = infer_shp_schema(&path)?;
        let mut geometry_contract =
            GeometryColumnContract::wkb_xy(FieldId(0), GEOMETRY, crs.clone(), true);
        geometry_contract.dimensions = geometry_info.dimensions;
        geometry_contract.geometry_types = geometry_info.geometry_types;
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
        for (n, ct) in &cols {
            fields.push(Field::new(n, coltype_to_dt(*ct), true));
        }
        let schema: SchemaRef = Arc::new(Schema::new(fields));
        let contract = DataContract {
            schema: schema.clone(),
            geometry: Some(geometry_contract.clone()),
        };
        let name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("layer")
            .to_owned();
        Ok(Box::new(ShpDataset {
            path,
            schema,
            cols,
            dimensions: geometry_contract.dimensions,
            layers: vec![LayerContract {
                id: LayerId(0),
                name,
                contract,
            }],
        }))
    }

    fn create(
        &self,
        sink: Sink,
        plan: &WritePlan,
        opts: &WriteOptions,
    ) -> Result<Box<dyn FormatWriter>> {
        validate_write(self.descriptor(), plan, &opts.limits)?;
        let Sink::Path(dest) = sink;
        let publish_mode = publish_mode(&dest, opts)?;
        if plan.layers.len() != 1 {
            return Err(PlenoraError::Unsupported(
                "Shapefile: un solo layer per file".to_owned(),
            ));
        }
        match publish_mode {
            ShapefilePublishMode::DirectoryDataset => {
                if dest.exists() {
                    return Err(PlenoraError::OutputExists(dest.display().to_string()));
                }
            }
            ShapefilePublishMode::LooseSet => {
                // no-clobber sull'intero set.
                for ext in ["shp", "shx", "dbf", "prj"] {
                    let sibling = dest.with_extension(ext);
                    if sibling.exists() {
                        return Err(PlenoraError::OutputExists(sibling.display().to_string()));
                    }
                }
            }
        }

        let layer = &plan.layers[0];
        let schema = &layer.contract.schema;
        let geom_idx = geometry_index(schema)
            .ok_or_else(|| err("il contratto non ha una colonna geometria geoarrow.wkb"))?;

        // Capability-check (ADR-IO 3): costruisce il dbf, fail-closed sui nomi.
        let mut table = TableWriterBuilder::new();
        let mut attrs: Vec<(usize, String, DbfKind)> = Vec::new();
        for (i, f) in schema.fields().iter().enumerate() {
            if i == geom_idx {
                continue;
            }
            let fname = shapefile::dbase::FieldName::try_from(f.name().as_str()).map_err(|_| {
                PlenoraError::Unsupported(format!(
                    "nome campo '{}' non valido per dbf (max 10 caratteri ASCII)",
                    f.name()
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

        let parent = dest
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."));
        let staging = tempfile::Builder::new().tempdir_in(&parent)?;
        let shp_path = staging.path().join("data.shp");
        let writer = Writer::from_path(&shp_path, table)
            .map_err(|e| err(format!("creazione shapefile: {e}")))?;

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
                wkb_limits: opts.limits.effective_wkb(),
                max_output_bytes: opts.limits.max_output_bytes,
            }),
            self.descriptor(),
            plan,
            opts.limits,
        )
    }
}

// --- lettura streaming -----------------------------------------------------

struct ShpDataset {
    path: PathBuf,
    schema: SchemaRef,
    cols: Vec<(String, ColType)>,
    dimensions: CoordinateDimensions,
    layers: Vec<LayerContract>,
}

impl OpenDatasetHandle for ShpDataset {
    fn layers(&self) -> &[LayerContract] {
        &self.layers
    }
    fn fidelity_assessment(&self) -> plenora_io_core::FidelityAssessment {
        plenora_io_core::FidelityAssessment::for_format(DESCRIPTOR.id, DESCRIPTOR.fidelity_class)
    }
    fn open_layer_reader(&self, request: &ReadRequest) -> Result<Box<dyn LayerReader>> {
        plenora_io_core::validate_read_projection(&DESCRIPTOR, request)?;
        let batch_size =
            plenora_io_core::effective_batch_rows(self.schema.as_ref(), request.batch_target);
        let rx = spawn_parser(
            self.path.clone(),
            self.schema.clone(),
            self.cols.clone(),
            self.dimensions,
            batch_size,
        );
        Ok(Box::new(ShpReader {
            rx,
            layer: self.layers[0].clone(),
        }))
    }
}

struct ShpReader {
    rx: Receiver<std::result::Result<RecordBatch, String>>,
    layer: LayerContract,
}

impl LayerReader for ShpReader {
    fn contract(&self) -> &LayerContract {
        &self.layer
    }
    fn next_batch(&mut self) -> Result<Option<RecordBatch>> {
        match self.rx.recv() {
            Ok(Ok(b)) => Ok(Some(b)),
            Ok(Err(e)) => Err(err(e)),
            Err(_) => Ok(None),
        }
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
    fn from(dt: &arrow_schema::DataType) -> Self {
        use arrow_schema::DataType as D;
        match dt {
            D::Int8
            | D::Int16
            | D::Int32
            | D::Int64
            | D::UInt8
            | D::UInt16
            | D::UInt32
            | D::UInt64 => DbfKind::Int,
            D::Float16 | D::Float32 | D::Float64 => DbfKind::Float,
            D::Boolean => DbfKind::Logical,
            _ => DbfKind::Char,
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
    wkb_limits: WkbLimits,
    max_output_bytes: u64,
}

impl FormatWriter for ShpWriter {
    fn write(&mut self, batch: &RecordBatch) -> Result<()> {
        let geom_col = batch
            .column(self.geom_idx)
            .as_any()
            .downcast_ref::<BinaryArray>()
            .ok_or_else(|| err("colonna geometria non binaria"))?;
        let limits = self.wkb_limits;
        let w = self.writer.as_mut().ok_or_else(|| err("writer chiuso"))?;
        let mut st = self.shape_type;
        for row in 0..batch.num_rows() {
            let shape = if geom_col.is_null(row) {
                Shape::NullShape
            } else {
                let geometry = decode_wkb(geom_col.value(row), &limits)?;
                shape_from_wkb(&geometry)?
            };
            // Capability-check (ADR-IO 3): un unico tipo di geometria per file.
            let tag = shape_tag(&shape);
            if tag == "unsupported" {
                return Err(err("tipo geometria non supportato da Shapefile"));
            }
            if !tag.is_empty() {
                match st {
                    None => st = Some(tag),
                    Some(e) if e != tag => {
                        return Err(err(format!(
                            "Shapefile richiede un unico tipo di geometria per file (trovati '{e}' e '{tag}')"
                        )))
                    }
                    _ => {}
                }
            }
            let mut rec = Record::default();
            for (col, name, kind) in &self.attrs {
                rec.insert(name.clone(), cell_to_field(batch.column(*col), row, *kind));
            }
            write_shape(w, shape, &rec)?;
        }
        self.shape_type = st;
        Ok(())
    }

    fn finish(mut self: Box<Self>) -> Result<Published> {
        // Finalizza .shp/.shx/.dbf (header + bounding box) rilasciando il writer.
        let w = self.writer.take().ok_or_else(|| err("writer già chiuso"))?;
        drop(w);
        let staging = self.staging.take().ok_or_else(|| err("staging mancante"))?;

        if let Some(wkt) = &self.prj {
            std::fs::write(staging.path().join("data.prj"), wkt)?;
        }

        let staged_bytes = ["dbf", "shx", "prj", "shp"]
            .into_iter()
            .map(|ext| staging.path().join(format!("data.{ext}")))
            .filter(|path| path.exists())
            .try_fold(0_u64, |total, path| {
                let bytes = std::fs::metadata(path)?.len();
                total.checked_add(bytes).ok_or_else(|| {
                    PlenoraError::LimitExceeded(
                        "overflow nel conteggio dell'output Shapefile".to_owned(),
                    )
                })
            })?;
        if staged_bytes > self.max_output_bytes {
            return Err(PlenoraError::LimitExceeded(format!(
                "output Shapefile da {staged_bytes} byte oltre il limite di {}",
                self.max_output_bytes
            )));
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

fn ensure_child<'a>(
    child: &'a WkbGeometry,
    parent: &WkbGeometry,
    expected: GeometryType,
) -> Result<&'a WkbValue> {
    if child.srid.is_some()
        || child.dimensions != parent.dimensions
        || child.geometry_type() != expected
    {
        return Err(err("geometria WKB annidata incoerente per Shapefile"));
    }
    Ok(&child.value)
}

fn polygon_rings(
    rings: &[Vec<WkbCoordinate>],
    destination: &mut Vec<(bool, Vec<WkbCoordinate>)>,
) -> Result<()> {
    if rings.is_empty() {
        return Err(err("poligono vuoto non rappresentabile in Shapefile"));
    }
    for (index, ring) in rings.iter().enumerate() {
        if ring.len() < 4 || ring.first() != ring.last() {
            return Err(err(
                "anello WKB non chiuso o con meno di quattro coordinate",
            ));
        }
        destination.push((index == 0, ring.clone()));
    }
    Ok(())
}

fn topology_from_wkb(geometry: &WkbGeometry) -> Result<ShpTopology> {
    if geometry.srid.is_some() {
        return Err(err(
            "SRID embedded non rappresentabile nel payload Shapefile; usare il CRS del layer",
        ));
    }
    match &geometry.value {
        WkbValue::Point(coordinate) => Ok(ShpTopology::Point(*coordinate)),
        WkbValue::MultiPoint(children) => {
            if children.is_empty() {
                return Err(err("MultiPoint vuoto non rappresentabile in Shapefile"));
            }
            let mut coordinates = Vec::with_capacity(children.len());
            for child in children {
                match ensure_child(child, geometry, GeometryType::Point)? {
                    WkbValue::Point(coordinate) => coordinates.push(*coordinate),
                    _ => return Err(err("MultiPoint con membro non-Point")),
                }
            }
            Ok(ShpTopology::Multipoint(coordinates))
        }
        WkbValue::LineString(coordinates) => {
            if coordinates.len() < 2 {
                return Err(err(
                    "LineString con meno di due coordinate non rappresentabile in Shapefile",
                ));
            }
            Ok(ShpTopology::Polyline(vec![coordinates.clone()]))
        }
        WkbValue::MultiLineString(children) => {
            if children.is_empty() {
                return Err(err(
                    "MultiLineString vuoto non rappresentabile in Shapefile",
                ));
            }
            let mut parts = Vec::with_capacity(children.len());
            for child in children {
                match ensure_child(child, geometry, GeometryType::LineString)? {
                    WkbValue::LineString(coordinates) if coordinates.len() >= 2 => {
                        parts.push(coordinates.clone());
                    }
                    WkbValue::LineString(_) => {
                        return Err(err(
                            "parte LineString con meno di due coordinate in Shapefile",
                        ))
                    }
                    _ => return Err(err("MultiLineString con membro non-LineString")),
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
                return Err(err("MultiPolygon vuoto non rappresentabile in Shapefile"));
            }
            let mut destination = Vec::new();
            for child in children {
                match ensure_child(child, geometry, GeometryType::Polygon)? {
                    WkbValue::Polygon(rings) => polygon_rings(rings, &mut destination)?,
                    _ => return Err(err("MultiPolygon con membro non-Polygon")),
                }
            }
            Ok(ShpTopology::Polygon(destination))
        }
        WkbValue::GeometryCollection(_) => {
            Err(err("GeometryCollection non rappresentabile in Shapefile"))
        }
    }
}

fn point_m(coordinate: WkbCoordinate) -> Result<PointM> {
    let measure = coordinate
        .m
        .ok_or_else(|| err("coordinata XYM senza ordinata M"))?;
    Ok(PointM::new(coordinate.x, coordinate.y, measure))
}

fn point_z(coordinate: WkbCoordinate, require_measure: bool) -> Result<PointZ> {
    let z = coordinate
        .z
        .ok_or_else(|| err("coordinata XYZ senza ordinata Z"))?;
    let measure = if require_measure {
        coordinate
            .m
            .ok_or_else(|| err("coordinata XYZM senza ordinata M"))?
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

fn shape_from_wkb(geometry: &WkbGeometry) -> Result<Shape> {
    let topology = topology_from_wkb(geometry)?;
    match (geometry.dimensions, topology) {
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
        (CoordinateDimensions::Unknown, _) => {
            Err(err("dimensionalità WKB ignota non scrivibile in Shapefile"))
        }
    }
}

fn shape_tag(s: &Shape) -> &'static str {
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

/// Scrive la shape come tipo ESRI concreto (l'enum `Shape` non è `EsriShape`).
fn write_shape(w: &mut Writer<BufWriter<File>>, shape: Shape, rec: &Record) -> Result<()> {
    let me = |e| err(format!("scrittura record shapefile: {e}"));
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
        Shape::NullShape => Err(err("geometria nulla non supportata in scrittura Shapefile")),
        Shape::Multipatch(_) => Err(err("Multipatch non supportato in scrittura Shapefile")),
    }
}

fn cell_to_field(array: &ArrayRef, row: usize, kind: DbfKind) -> FieldValue {
    let v = json_from_array(array, row);
    match kind {
        DbfKind::Char => FieldValue::Character(match v {
            JsonValue::Null => None,
            JsonValue::String(s) => Some(s),
            other => Some(other.to_string()),
        }),
        DbfKind::Int | DbfKind::Float => FieldValue::Numeric(v.as_f64()),
        DbfKind::Logical => FieldValue::Logical(v.as_bool()),
    }
}

fn geometry_index(schema: &Schema) -> Option<usize> {
    schema.fields().iter().position(|f| is_geometry_field(f))
}

fn wkt_for_id(id: Option<&str>) -> Option<String> {
    match id {
        Some("EPSG:4326") | Some("OGC:CRS84") => Some(WGS84_WKT.to_owned()),
        _ => None,
    }
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
            return Err(PlenoraError::CrsUnresolved {
                driver: "shp",
                raw: RawCrs {
                    definition: wkt,
                    authority_hint: None,
                },
            });
        };
        let kind = crs_kind(&id, Some(&wkt));
        return Ok(ResolvedCrs::new(Some(id), kind, Some(wkt)));
    }
    match &opts.assume_crs {
        Some(id) => Ok(ResolvedCrs::new(Some(id.clone()), crs_kind(id, None), None)),
        None => Err(PlenoraError::Crs(
            "Shapefile senza .prj: fornire --assume-crs".to_owned(),
        )),
    }
}

fn resolved_crs_id(crs: &ResolvedCrs) -> Result<&str> {
    crs.id.as_deref().ok_or_else(|| {
        PlenoraError::Crs(
            "Shapefile: CRS risolto senza identificatore; vietato inventare un'etichetta Arrow"
                .to_owned(),
        )
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
    if id.eq_ignore_ascii_case("OGC:CRS84")
        || id.eq_ignore_ascii_case("EPSG:4326")
        || definition.contains("GEOGCS[")
        || definition.contains("GEOGCRS[")
    {
        CrsKind::Geographic
    } else if definition.contains("PROJCS[")
        || definition.contains("PROJCRS[")
        || id.eq_ignore_ascii_case("EPSG:3857")
    {
        CrsKind::Projected
    } else {
        CrsKind::Unknown
    }
}

fn coltype_to_dt(ct: ColType) -> DataType {
    match ct {
        ColType::Integer => DataType::Int64,
        ColType::Number => DataType::Float64,
        ColType::Boolean => DataType::Boolean,
        ColType::Text => DataType::Utf8,
    }
}

#[derive(Clone, Copy)]
struct Acc {
    any: bool,
    all_int: bool,
    all_num: bool,
    all_bool: bool,
}

impl Default for Acc {
    fn default() -> Self {
        Acc {
            any: false,
            all_int: true,
            all_num: true,
            all_bool: true,
        }
    }
}

impl Acc {
    fn observe(&mut self, class: u8) {
        match class {
            0 => {}
            1 => {
                self.any = true;
                self.all_bool = false;
            }
            2 => {
                self.any = true;
                self.all_int = false;
                self.all_bool = false;
            }
            3 => {
                self.any = true;
                self.all_int = false;
                self.all_num = false;
            }
            _ => {
                self.any = true;
                self.all_int = false;
                self.all_num = false;
                self.all_bool = false;
            }
        }
    }
    fn coltype(&self) -> ColType {
        if !self.any {
            ColType::Text
        } else if self.all_int {
            ColType::Integer
        } else if self.all_bool {
            ColType::Boolean
        } else if self.all_num {
            ColType::Number
        } else {
            ColType::Text
        }
    }
}

/// Classe dbf per l'inferenza (Numeric/Double/Float=numero, Integer=int).
fn classify(v: &FieldValue) -> u8 {
    match v {
        FieldValue::Integer(_) => 1,
        FieldValue::Numeric(Some(_)) | FieldValue::Double(_) | FieldValue::Float(Some(_)) => 2,
        FieldValue::Logical(Some(_)) => 3,
        FieldValue::Character(Some(_)) | FieldValue::Date(Some(_)) => 4,
        _ => 0,
    }
}

struct ShpGeometryInfo {
    dimensions: CoordinateDimensions,
    geometry_types: Vec<GeometryType>,
    shape_type: Option<&'static str>,
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
    let (z, m) = match dimensions {
        CoordinateDimensions::Xy if point.z().is_none() && point.m().is_none() => (None, None),
        CoordinateDimensions::Xym if point.z().is_none() => (
            None,
            Some(
                point
                    .m()
                    .ok_or_else(|| err("coordinata ShapeM senza misura"))?,
            ),
        ),
        CoordinateDimensions::Xyz => {
            let z = point
                .z()
                .ok_or_else(|| err("coordinata ShapeZ senza quota"))?;
            if point.m().is_some_and(|measure| {
                !matches!(
                    measure.partial_cmp(&NO_DATA),
                    Some(std::cmp::Ordering::Less | std::cmp::Ordering::Equal)
                )
            }) {
                return Err(err(
                    "misura valida trovata in un dataset ShapeZ dichiarato XYZ",
                ));
            }
            (Some(z), None)
        }
        CoordinateDimensions::Xyzm => (
            Some(
                point
                    .z()
                    .ok_or_else(|| err("coordinata ShapeZ senza quota"))?,
            ),
            Some(
                point
                    .m()
                    .ok_or_else(|| err("coordinata ShapeZ senza misura nativa"))?,
            ),
        ),
        CoordinateDimensions::Unknown => {
            return Err(err("dimensionalità Shapefile non determinata"))
        }
        _ => {
            return Err(err(
                "variante Shape incoerente con la dimensionalità del layer",
            ))
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
                let current = current
                    .as_mut()
                    .ok_or_else(|| err("anello interno Shapefile senza anello esterno"))?;
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
        return Err(err("Polygon Shapefile senza anelli esterni"));
    }
    Ok(WkbGeometry {
        value: WkbValue::MultiPolygon(polygons),
        dimensions,
        srid: None,
    })
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
            return Err(err(
                "Multipatch non ha una conversione WKB univoca ed è rifiutato",
            ))
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

fn geometry_type_for_shape(shape: &Shape) -> Option<GeometryType> {
    match shape {
        Shape::Point(_) | Shape::PointM(_) | Shape::PointZ(_) => Some(GeometryType::Point),
        Shape::Polyline(_) | Shape::PolylineM(_) | Shape::PolylineZ(_) => {
            Some(GeometryType::MultiLineString)
        }
        Shape::Polygon(_) | Shape::PolygonM(_) | Shape::PolygonZ(_) => {
            Some(GeometryType::MultiPolygon)
        }
        Shape::Multipoint(_) | Shape::MultipointM(_) | Shape::MultipointZ(_) => {
            Some(GeometryType::MultiPoint)
        }
        Shape::NullShape | Shape::Multipatch(_) => None,
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

/// Pass 1: nomi campo, tipo DBF e contratto geometrico nativo, a RAM O(ncol).
fn infer_shp_schema(path: &Path) -> Result<(Vec<(String, ColType)>, ShpGeometryInfo)> {
    let mut reader =
        shapefile::Reader::from_path(path).map_err(|e| err(format!("apertura shapefile: {e}")))?;
    let mut order: Vec<String> = Vec::new();
    let mut accs: HashMap<String, Acc> = HashMap::new();
    let mut shape_type = None;
    let mut geometry_types = BTreeSet::new();
    let mut z_has_measure = false;
    for pair in reader.iter_shapes_and_records() {
        let (shape, record) = pair.map_err(|e| err(format!("record shapefile: {e}")))?;
        let tag = shape_tag(&shape);
        if tag == "unsupported" {
            return Err(err("Multipatch Shapefile non supportato"));
        }
        if !tag.is_empty() {
            match shape_type {
                None => shape_type = Some(tag),
                Some(existing) if existing != tag => {
                    return Err(err(format!(
                        "tipi Shape incoerenti nel file: '{existing}' e '{tag}'"
                    )))
                }
                _ => {}
            }
        }
        z_has_measure |= shape_has_valid_measure(&shape);
        if let Some(geometry_type) = geometry_type_for_shape(&shape) {
            geometry_types.insert(geometry_type);
        }
        for (name, value) in record {
            match accs.entry(name.clone()) {
                std::collections::hash_map::Entry::Vacant(entry) => {
                    order.push(name);
                    entry.insert(Acc::default()).observe(classify(&value));
                }
                std::collections::hash_map::Entry::Occupied(mut entry) => {
                    entry.get_mut().observe(classify(&value));
                }
            }
        }
    }
    let columns = order
        .into_iter()
        .map(|name| {
            let column_type = accs
                .get(&name)
                .ok_or_else(|| err(format!("schema DBF senza accumulatore per '{name}'")))?
                .coltype();
            Ok((name, column_type))
        })
        .collect::<Result<Vec<_>>>()?;
    Ok((
        columns,
        ShpGeometryInfo {
            dimensions: dimensions_for_shape_tag(shape_type, z_has_measure),
            geometry_types: geometry_types.into_iter().collect(),
            shape_type,
        },
    ))
}

/// Pass 2: thread che scorre i record e produce batch da `batch_size` righe.
fn spawn_parser(
    path: PathBuf,
    schema: SchemaRef,
    cols: Vec<(String, ColType)>,
    dimensions: CoordinateDimensions,
    batch_size: usize,
) -> Receiver<std::result::Result<RecordBatch, String>> {
    let (tx, rx) = sync_channel::<std::result::Result<RecordBatch, String>>(2);
    std::thread::spawn(move || {
        let run = || -> std::result::Result<(), String> {
            let mut reader = shapefile::Reader::from_path(&path).map_err(|e| e.to_string())?;
            let mut geom = BinaryBuilder::new();
            let mut builders: Vec<ShpColBuilder> =
                cols.iter().map(|(_, ct)| ShpColBuilder::new(*ct)).collect();
            let mut n = 0usize;
            for pair in reader.iter_shapes_and_records() {
                let (shape, record) = pair.map_err(|e| e.to_string())?;
                match shape_to_wkb(&shape, dimensions).map_err(|e| e.to_string())? {
                    Some(geometry) => {
                        let bytes =
                            encode_wkb(&geometry, WkbFlavor::Iso).map_err(|e| e.to_string())?;
                        geom.append_value(bytes);
                    }
                    None => geom.append_null(),
                }
                // Lookup per nome (l'ordine di iterazione del Record non è garantito).
                let map: HashMap<String, FieldValue> = record.into_iter().collect();
                for (k, (name, _)) in cols.iter().enumerate() {
                    builders[k].append(map.get(name));
                }
                n += 1;
                if n >= batch_size {
                    let batch = finish_batch(&schema, &mut geom, &mut builders)?;
                    if tx.send(Ok(batch)).is_err() {
                        return Ok(());
                    }
                    n = 0;
                }
            }
            if n > 0 {
                let batch = finish_batch(&schema, &mut geom, &mut builders)?;
                let _ = tx.send(Ok(batch));
            }
            Ok(())
        };
        if let Err(e) = run() {
            let _ = tx.send(Err(e));
        }
    });
    rx
}

fn finish_batch(
    schema: &SchemaRef,
    geom: &mut BinaryBuilder,
    builders: &mut [ShpColBuilder],
) -> std::result::Result<RecordBatch, String> {
    let mut arrays: Vec<ArrayRef> = Vec::with_capacity(1 + builders.len());
    arrays.push(Arc::new(geom.finish()));
    for b in builders.iter_mut() {
        arrays.push(b.finish());
    }
    RecordBatch::try_new(schema.clone(), arrays).map_err(|e| format!("batch: {e}"))
}

enum ShpColBuilder {
    I64(Int64Builder),
    F64(Float64Builder),
    Bool(BooleanBuilder),
    Str(StringBuilder),
}

impl ShpColBuilder {
    fn new(ct: ColType) -> Self {
        match ct {
            ColType::Integer => ShpColBuilder::I64(Int64Builder::new()),
            ColType::Number => ShpColBuilder::F64(Float64Builder::new()),
            ColType::Boolean => ShpColBuilder::Bool(BooleanBuilder::new()),
            ColType::Text => ShpColBuilder::Str(StringBuilder::new()),
        }
    }
    fn append(&mut self, v: Option<&FieldValue>) {
        match self {
            ShpColBuilder::I64(b) => b.append_option(v.and_then(fv_i64)),
            ShpColBuilder::F64(b) => b.append_option(v.and_then(fv_f64)),
            ShpColBuilder::Bool(b) => b.append_option(v.and_then(fv_bool)),
            ShpColBuilder::Str(b) => match v.and_then(fv_string) {
                Some(s) => b.append_value(s),
                None => b.append_null(),
            },
        }
    }
    fn finish(&mut self) -> ArrayRef {
        match self {
            ShpColBuilder::I64(b) => Arc::new(b.finish()),
            ShpColBuilder::F64(b) => Arc::new(b.finish()),
            ShpColBuilder::Bool(b) => Arc::new(b.finish()),
            ShpColBuilder::Str(b) => Arc::new(b.finish()),
        }
    }
}

fn fv_i64(v: &FieldValue) -> Option<i64> {
    match v {
        FieldValue::Integer(i) => Some(*i as i64),
        FieldValue::Numeric(Some(n)) => Some(*n as i64),
        FieldValue::Double(d) => Some(*d as i64),
        FieldValue::Float(Some(f)) => Some(*f as i64),
        _ => None,
    }
}

fn fv_f64(v: &FieldValue) -> Option<f64> {
    match v {
        FieldValue::Numeric(Some(n)) => Some(*n),
        FieldValue::Double(d) => Some(*d),
        FieldValue::Float(Some(f)) => Some(*f as f64),
        FieldValue::Integer(i) => Some(*i as f64),
        _ => None,
    }
}

fn fv_bool(v: &FieldValue) -> Option<bool> {
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
    let shape = shape_from_wkb(&geometry)?;
    let round_trip = shape_to_wkb(&shape, dimensions)?
        .ok_or_else(|| err("la conversione di una geometria ha prodotto NullShape"))?;
    Ok(encode_wkb(&round_trip, WkbFlavor::Iso)?.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use plenora_core::wkb::to_wkb;
    use plenora_io_core::request::{BatchTarget, ProjectionMode};
    use plenora_io_core::WriteLayer;

    fn read_opts() -> ReadOptions {
        ReadOptions {
            assume_crs: Some("EPSG:4326".to_owned()),
            format_options: Default::default(),
            ..ReadOptions::default()
        }
    }

    fn req() -> ReadRequest {
        ReadRequest {
            layer: LayerId(0),
            projected_fields: None,
            projection_mode: ProjectionMode::BestEffort,
            pruning_predicate: None,
            spatial_pruning_hint: None,
            batch_target: BatchTarget::default(),
        }
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

        let crs = resolve_crs(&path, &ReadOptions::default()).unwrap();
        assert_eq!(crs.id.as_deref(), Some("EPSG:4326"));
        assert_eq!(
            crs.axis_order,
            plenora_core::crs::AxisOrder::LatitudeLongitude
        );
    }

    #[test]
    fn unresolved_prj_is_preserved_in_typed_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("local.shp");
        let definition = "LOCAL_CS[\"survey-grid-secret\"]";
        std::fs::write(path.with_extension("prj"), definition).unwrap();

        let error = resolve_crs(&path, &ReadOptions::default()).unwrap_err();
        match &error {
            PlenoraError::CrsUnresolved { driver, raw } => {
                assert_eq!(*driver, "shp");
                assert_eq!(raw.definition, definition);
            }
            other => panic!("errore inatteso: {other}"),
        }
        assert!(!error.to_string().contains("survey-grid-secret"));
    }

    #[test]
    fn assumed_unknown_epsg_does_not_invent_an_axis_order() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("no-prj.shp");
        let crs = resolve_crs(
            &path,
            &ReadOptions {
                assume_crs: Some("EPSG:4258".to_owned()),
                ..ReadOptions::default()
            },
        )
        .unwrap();
        assert_eq!(crs.kind, CrsKind::Unknown);
        assert_eq!(crs.axis_order, plenora_core::crs::AxisOrder::Unknown);
    }

    #[test]
    fn resolved_crs_without_id_cannot_be_relabelled_as_unknown() {
        let crs = ResolvedCrs::new(
            None,
            CrsKind::Unknown,
            Some("LOCAL_CS[\"private\"]".to_owned()),
        );

        assert!(matches!(resolved_crs_id(&crs), Err(PlenoraError::Crs(_))));
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
                Arc::new(Int64Array::from(vec![2800000i64, 1400000])),
            ],
        )
        .unwrap();

        let driver = ShpDriver;
        let plan = WritePlan {
            layers: vec![WriteLayer {
                name: "l".to_owned(),
                contract: DataContract {
                    schema: schema.clone(),
                    geometry: None,
                },
            }],
        };
        let mut w = driver
            .create(Sink::Path(out.clone()), &plan, &WriteOptions::default())
            .unwrap();
        w.write(&batch).unwrap();
        w.finish().unwrap();

        // il set è stato pubblicato
        assert!(out.exists());
        assert!(out.with_extension("dbf").exists());
        assert!(out.with_extension("prj").exists());

        // rilettura
        let ds = driver.open(Source::Path(out), &read_opts()).unwrap();
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
        let mut options = WriteOptions {
            durable: true,
            ..WriteOptions::default()
        };
        options
            .format_options
            .insert("publish_mode".to_owned(), DIRECTORY_DATASET_MODE.to_owned());

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

        let dataset = driver.open(Source::Path(output), &read_opts()).unwrap();
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
            .create(Sink::Path(output.clone()), &plan, &WriteOptions::default())
            .unwrap();
        drop(writer);

        assert!(!output.exists());
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
        let mut options = WriteOptions::default();
        options
            .format_options
            .insert("publish_mode".to_owned(), DIRECTORY_DATASET_MODE.to_owned());

        let result = ShpDriver
            .create(Sink::Path(root.path().join("points.shp")), &plan, &options)
            .map(|_| ());

        assert!(matches!(result, Err(PlenoraError::Unsupported(_))));
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
                &WriteOptions::default(),
            )
            .map(|_| ())
            .unwrap_err();
        assert!(matches!(
            e,
            PlenoraError::Capability {
                reason: plenora_core::CapabilityReason::FieldNameTooLong,
                ..
            }
        ));
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
                    i as f64, i as f64,
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
                    schema: schema.clone(),
                    geometry: None,
                },
            }],
        };
        let mut w = driver
            .create(Sink::Path(out.clone()), &plan, &WriteOptions::default())
            .unwrap();
        w.write(&batch).unwrap();
        w.finish().unwrap();

        let ds = driver.open(Source::Path(out), &read_opts()).unwrap();
        let req = ReadRequest {
            layer: LayerId(0),
            projected_fields: None,
            projection_mode: ProjectionMode::BestEffort,
            pruning_predicate: None,
            spatial_pruning_hint: None,
            batch_target: BatchTarget {
                target_bytes: 8 * 1024 * 1024,
                max_rows: 4,
            },
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
        geometry: WkbGeometry,
    ) -> WkbGeometry {
        use arrow_array::Int64Array;

        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join(format!("{dimensions:?}.shp"));
        let bytes = encode_wkb(&geometry, WkbFlavor::Iso).unwrap();
        let mut geometry_contract =
            GeometryColumnContract::wkb_xy(FieldId(0), GEOMETRY, ResolvedCrs::wgs84(), false);
        geometry_contract.dimensions = dimensions;
        geometry_contract.geometry_types = vec![GeometryType::Point];
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
            .create(Sink::Path(out.clone()), &plan, &WriteOptions::default())
            .unwrap();
        writer.write(&batch).unwrap();
        writer.finish().unwrap();

        let mut native = shapefile::Reader::from_path(&out).unwrap();
        let (shape, _) = native.iter_shapes_and_records().next().unwrap().unwrap();
        match dimensions {
            CoordinateDimensions::Xym => assert!(matches!(shape, Shape::PointM(_))),
            CoordinateDimensions::Xyz | CoordinateDimensions::Xyzm => {
                assert!(matches!(shape, Shape::PointZ(_)))
            }
            _ => unreachable!("test solo dimensionale"),
        }

        let dataset = driver.open(Source::Path(out), &read_opts()).unwrap();
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
            let actual = round_trip_dimensional_point(expected.dimensions, expected.clone());
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
        let shape = shape_from_wkb(&line).unwrap();
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
        let shape = shape_from_wkb(&polygon).unwrap();
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
        assert!(shape_from_wkb(&geometry).is_err());
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

        assert!(shape_from_wkb(&missing_z).is_err());
        assert!(shape_from_wkb(&missing_m).is_err());
    }
}
