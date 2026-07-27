//! driver-kml — KML ⇄ RecordBatch. KML è WGS84 per specifica (`OGC:CRS84`).
//! I Placemark diventano feature con geometria WKB `geoarrow.wkb` XY/XYZ e
//! `name`/`description` come proprietà. KMZ resta un incremento successivo.
#![forbid(unsafe_code)]

use std::collections::{BTreeSet, HashMap};
use std::io::Write as _;
use std::path::PathBuf;
use std::sync::Arc;

use arrow_array::{Array, ArrayRef, BinaryArray, RecordBatch, StringArray};
use arrow_schema::{DataType, Field, Schema, SchemaRef};
use kml::types::{
    Coord as KmlCoord, Element, Geometry as KmlGeometry, LineString as KmlLineString, LinearRing,
    MultiGeometry, Placemark, Point as KmlPoint, Polygon as KmlPolygon,
};
use kml::{Kml, KmlDocument, KmlVersion, KmlWriter};

use driver_common::{geometry_field, json_from_array, OGC_CRS84};
use plenora_core::contract::{
    CoordinateDimensions, DataContract, FieldId, GeometryColumnContract, GeometryType,
    LayerContract, LayerId,
};
use plenora_core::crs::ResolvedCrs;
use plenora_core::geometry::{is_geometry_field, with_geometry_contract_metadata};
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
use plenora_io_core::publish::publish_file_atomic_limited;
use plenora_io_core::request::ReadRequest;
use plenora_io_core::{
    validate_write, with_write_validation, AttributeWriteSupport, CrsWriteSupport,
    FormatWriteCapabilities, NullabilitySupport, SingleReaderGate, TypeCoercionPolicy, WritePlan,
    SCALAR_TYPES, UTF8_FIELD_NAMES, WKB_XY_XYZ_GEOMETRY,
};
use quick_xml::events::{BytesStart, Event};
use quick_xml::Reader as XmlReader;

const GEOMETRY: &str = "geometry";
const MAX_XML_DEPTH: usize = 256;

fn err(reason: impl Into<String>) -> PlenoraError {
    PlenoraError::Format {
        driver: "kml",
        reason: reason.into(),
    }
}

fn valid_xml_name(name: &[u8]) -> bool {
    let mut parts = name.split(|byte| *byte == b':');
    let valid_part = |part: &[u8]| {
        !part.is_empty()
            && (part[0].is_ascii_alphabetic() || part[0] == b'_')
            && part[1..]
                .iter()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
    };
    let Some(first) = parts.next() else {
        return false;
    };
    valid_part(first) && parts.next().is_none_or(valid_part) && parts.next().is_none()
}

fn validate_element(event: &BytesStart<'_>) -> Result<()> {
    if !valid_xml_name(event.name().as_ref()) {
        return Err(err("nome di elemento XML non valido"));
    }
    for attribute in event.attributes().with_checks(true) {
        let attribute =
            attribute.map_err(|error| err(format!("attributo XML non valido: {error}")))?;
        if !valid_xml_name(attribute.key.as_ref()) {
            return Err(err("nome di attributo XML non valido"));
        }
    }
    Ok(())
}

fn local_xml_name(name: &[u8]) -> &[u8] {
    match name.iter().rposition(|byte| *byte == b':') {
        Some(separator) => &name[separator + 1..],
        None => name,
    }
}

fn observe_point_coordinate_text(
    stack: &[Vec<u8>],
    open_points_with_coordinates: &mut [bool],
    text: &[u8],
) -> Result<()> {
    let mut ancestors = stack.iter().rev();
    let direct_point_coordinates = matches!(
        (ancestors.next(), ancestors.next()),
        (Some(child), Some(parent))
            if local_xml_name(child) == b"coordinates"
                && local_xml_name(parent) == b"Point"
    );
    if direct_point_coordinates && text.iter().any(|byte| !byte.is_ascii_whitespace()) {
        let Some(point_has_coordinates) = open_points_with_coordinates.last_mut() else {
            return Err(err("coordinate Point KML fuori contesto"));
        };
        *point_has_coordinates = true;
    }
    Ok(())
}

/// Il parser `kml` è permissivo su alcuni token XML malformati e può non
/// avanzare; inoltre `kml 0.14.0` rimuove senza controllo la prima coordinata
/// di un `Point`. Una scansione XML limitata evita di consegnargli input
/// ambigui o punti senza coordinate.
fn validate_kml_xml(text: &str) -> Result<()> {
    let mut reader = XmlReader::from_str(text);
    let mut stack = Vec::<Vec<u8>>::new();
    let mut open_points_with_coordinates = Vec::<bool>::new();
    let mut previous_position = 0_u64;
    let mut events_left = text.len().saturating_add(1);

    loop {
        if events_left == 0 {
            return Err(err(
                "numero di eventi XML incoerente con la dimensione dell'input",
            ));
        }
        events_left -= 1;

        let event = reader
            .read_event()
            .map_err(|error| err(format!("XML KML non valido: {error}")))?;
        let position = reader.buffer_position();
        if !matches!(event, Event::Eof) && position <= previous_position {
            return Err(err("parser XML senza avanzamento"));
        }
        previous_position = position;

        match event {
            Event::Start(element) => {
                validate_element(&element)?;
                if stack.len() >= MAX_XML_DEPTH {
                    return Err(err(format!(
                        "profondità XML oltre il limite di {MAX_XML_DEPTH}"
                    )));
                }
                if element.local_name().as_ref() == b"Point" {
                    open_points_with_coordinates.push(false);
                }
                stack.push(element.name().as_ref().to_vec());
            }
            Event::Empty(element) => {
                validate_element(&element)?;
                if element.local_name().as_ref() == b"Point" {
                    return Err(err("Point KML senza coordinate"));
                }
            }
            Event::Text(text) => {
                observe_point_coordinate_text(
                    &stack,
                    &mut open_points_with_coordinates,
                    text.as_ref(),
                )?;
            }
            Event::CData(text) => {
                observe_point_coordinate_text(
                    &stack,
                    &mut open_points_with_coordinates,
                    text.as_ref(),
                )?;
            }
            Event::End(element) => {
                if !valid_xml_name(element.name().as_ref()) {
                    return Err(err("nome di chiusura XML non valido"));
                }
                let Some(opened) = stack.pop() else {
                    return Err(err("chiusura XML senza elemento aperto"));
                };
                if opened.as_slice() != element.name().as_ref() {
                    return Err(err("elementi XML annidati in modo non valido"));
                }
                if element.local_name().as_ref() == b"Point" {
                    let Some(has_coordinates) = open_points_with_coordinates.pop() else {
                        return Err(err("chiusura Point KML senza apertura"));
                    };
                    if !has_coordinates {
                        return Err(err("Point KML senza coordinate"));
                    }
                }
            }
            Event::DocType(_) => return Err(err("DOCTYPE non ammesso nei documenti KML")),
            Event::Eof => {
                if !stack.is_empty() {
                    return Err(err("documento XML troncato"));
                }
                return Ok(());
            }
            _ => {}
        }
    }
}

static DESCRIPTOR: FormatDescriptor = FormatDescriptor {
    id: "kml",
    direction: Direction::Bidirectional,
    read_mode: ReadMode::Materializing,
    write_mode: Some(WriteMode::Buffered),
    multi_layer: false,
    multi_file: false,
    reader_concurrency: ReaderConcurrency::SingleActiveReader,
    projection_support: plenora_io_core::ProjectionSupport::None,
    predicate_pruning_support: plenora_io_core::PredicatePruningSupport::None,
    spatial_pruning_support: plenora_io_core::SpatialPruningSupport::None,
    crs_handling: CrsHandling::FixedWgs84,
    fidelity_class: Fidelity::Conditional,
    runtime: Runtime::PureRust,
    write_capabilities: Some(FormatWriteCapabilities {
        field_names: UTF8_FIELD_NAMES,
        allowed_types: SCALAR_TYPES,
        type_coercion: TypeCoercionPolicy::Reject,
        attributes: AttributeWriteSupport::All,
        geometry: WKB_XY_XYZ_GEOMETRY,
        crs: CrsWriteSupport::Fixed("OGC:CRS84"),
        nullability: NullabilitySupport::FormatDefined,
        multi_layer: false,
    }),
    semantic_version: 1,
    driver_version: 4,
    descriptor_version: 4,
};

pub struct KmlDriver;

impl FormatDriver for KmlDriver {
    fn descriptor(&self) -> &FormatDescriptor {
        &DESCRIPTOR
    }

    fn open(&self, source: Source, opts: &ReadOptions) -> Result<Box<dyn OpenDatasetHandle>> {
        let path = source.into_path_checked(&opts.limits)?;
        let text = std::fs::read_to_string(&path)?;
        validate_kml_xml(&text)?;
        let root: Kml = text
            .parse()
            .map_err(|e| err(format!("KML non valido: {e}")))?;
        let mut placemarks = Vec::new();
        collect(&root, &mut placemarks);
        let (batch, contract) = build_batch(&placemarks)?;
        let name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("layer")
            .to_owned();
        Ok(Box::new(KmlDataset {
            layers: vec![LayerContract {
                id: LayerId(0),
                name,
                contract,
            }],
            batch,
            reader_gate: SingleReaderGate::new(DESCRIPTOR.id),
        }))
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
            return Err(PlenoraError::OutputExists(path.display().to_string()));
        }
        if !path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("kml"))
        {
            return Err(PlenoraError::Unsupported(
                "l'output deve avere estensione .kml".to_owned(),
            ));
        }
        if plan.layers.len() != 1 {
            return Err(PlenoraError::Unsupported(
                "KML: un solo layer per file".to_owned(),
            ));
        }
        with_write_validation(
            Box::new(KmlWriterState {
                path,
                durable: opts.durable,
                placemarks: Vec::new(),
                wkb_limits: opts.limits.effective_wkb(),
                max_output_bytes: opts.limits.max_output_bytes,
            }),
            self.descriptor(),
            plan,
            opts.limits,
        )
    }
}

struct KmlDataset {
    layers: Vec<LayerContract>,
    batch: RecordBatch,
    reader_gate: SingleReaderGate,
}

impl OpenDatasetHandle for KmlDataset {
    fn layers(&self) -> &[LayerContract] {
        &self.layers
    }
    fn fidelity_assessment(&self) -> plenora_io_core::FidelityAssessment {
        plenora_io_core::FidelityAssessment::for_format(DESCRIPTOR.id, DESCRIPTOR.fidelity_class)
    }
    fn open_layer_reader(&self, request: &ReadRequest) -> Result<Box<dyn LayerReader>> {
        plenora_io_core::validate_read_projection(&DESCRIPTOR, request)?;
        let reader = self.reader_gate.open(request.layer, || {
            Ok(Box::new(KmlReader {
                batch: Some(self.batch.clone()),
                layer: self.layers[0].clone(),
            }))
        })?;
        Ok(plenora_io_core::with_batch_target(
            reader,
            request.batch_target,
        ))
    }
}

struct KmlReader {
    batch: Option<RecordBatch>,
    layer: LayerContract,
}

impl LayerReader for KmlReader {
    fn contract(&self) -> &LayerContract {
        &self.layer
    }
    fn next_batch(&mut self) -> Result<Option<RecordBatch>> {
        Ok(self.batch.take())
    }
}

// --- scrittura -------------------------------------------------------------

struct KmlWriterState {
    path: PathBuf,
    durable: bool,
    placemarks: Vec<Placemark>,
    wkb_limits: WkbLimits,
    max_output_bytes: u64,
}

impl FormatWriter for KmlWriterState {
    fn write(&mut self, batch: &RecordBatch) -> Result<()> {
        let schema = batch.schema();
        let geom_idx = schema
            .fields()
            .iter()
            .position(|f| is_geometry_field(f))
            .ok_or_else(|| err("nessuna colonna geometria geoarrow.wkb"))?;
        let geom_col = batch
            .column(geom_idx)
            .as_any()
            .downcast_ref::<BinaryArray>()
            .ok_or_else(|| err("colonna geometria non binaria"))?;
        let limits = self.wkb_limits;
        let name_idx = schema.index_of("name").ok();
        let desc_idx = schema.index_of("description").ok();

        for row in 0..batch.num_rows() {
            let geometry = if geom_col.is_null(row) {
                None
            } else {
                let geometry = decode_wkb(geom_col.value(row), &limits)?;
                Some(kml_geometry_from_wkb(&geometry)?)
            };
            let name = name_idx.and_then(|i| cell_string(batch.column(i), row));
            let description = desc_idx.and_then(|i| cell_string(batch.column(i), row));

            // Colonne extra (non name/description/geometria) -> ExtendedData.
            let mut data = Vec::new();
            for (i, f) in schema.fields().iter().enumerate() {
                if i == geom_idx || Some(i) == name_idx || Some(i) == desc_idx {
                    continue;
                }
                if let Some(v) = cell_string(batch.column(i), row) {
                    data.push((f.name().clone(), v));
                }
            }
            let children = if data.is_empty() {
                Vec::new()
            } else {
                vec![extended_data(&data)]
            };

            self.placemarks.push(Placemark {
                name,
                description,
                geometry,
                children,
                ..Default::default()
            });
        }
        Ok(())
    }

    fn finish(self: Box<Self>) -> Result<Published> {
        // Avvolge i Placemark in <kml xmlns><Document>…</Document></kml> (root
        // KML valido, altrimenti GDAL/parser rifiutano il file).
        let doc = Kml::KmlDocument(KmlDocument {
            version: KmlVersion::V22,
            attrs: HashMap::from([(
                "xmlns".to_owned(),
                "http://www.opengis.net/kml/2.2".to_owned(),
            )]),
            elements: vec![Kml::Document {
                attrs: HashMap::new(),
                elements: self.placemarks.into_iter().map(Kml::Placemark).collect(),
            }],
        });
        let parent = self
            .path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."));
        let mut temp = tempfile::NamedTempFile::new_in(&parent)?;
        let mut buf: Vec<u8> = Vec::new();
        {
            let mut w = KmlWriter::from_writer(&mut buf);
            w.write(&doc)
                .map_err(|e| err(format!("serializzazione KML: {e}")))?;
        }
        temp.as_file_mut().write_all(&buf)?;
        temp.as_file_mut().flush()?;
        let (bytes, outcome) =
            publish_file_atomic_limited(temp, &self.path, self.durable, self.max_output_bytes)?;
        Ok(Published {
            bytes,
            loss: LossReport::default(),
            fidelity: plenora_io_core::FidelityAssessment::lossless(),
            outcome,
        })
    }
}

fn cell_string(array: &ArrayRef, row: usize) -> Option<String> {
    match json_from_array(array, row) {
        serde_json::Value::Null => None,
        serde_json::Value::String(s) => Some(s),
        other => Some(other.to_string()),
    }
}

fn extended_data(pairs: &[(String, String)]) -> Element {
    Element {
        name: "ExtendedData".to_owned(),
        attrs: HashMap::new(),
        content: None,
        children: pairs
            .iter()
            .map(|(k, v)| Element {
                name: "Data".to_owned(),
                attrs: HashMap::from([("name".to_owned(), k.clone())]),
                content: None,
                children: vec![Element {
                    name: "value".to_owned(),
                    attrs: HashMap::new(),
                    content: Some(v.clone()),
                    children: Vec::new(),
                }],
            })
            .collect(),
    }
}

fn collect(k: &Kml, out: &mut Vec<Placemark>) {
    match k {
        Kml::KmlDocument(d) => {
            for e in &d.elements {
                collect(e, out);
            }
        }
        Kml::Document { elements, .. } => {
            for e in elements {
                collect(e, out);
            }
        }
        Kml::Folder(folder) => {
            for e in &folder.elements {
                collect(e, out);
            }
        }
        Kml::Placemark(p) => out.push(p.clone()),
        _ => {}
    }
}

fn dimensions_for_kml_coords(coords: &[KmlCoord]) -> Result<CoordinateDimensions> {
    let mut has_z = None;
    for coordinate in coords {
        let current = coordinate.z.is_some();
        if has_z.is_some_and(|known| known != current) {
            return Err(err("coordinate KML con dimensionalità Z non uniforme"));
        }
        has_z = Some(current);
    }
    Ok(if has_z.unwrap_or(false) {
        CoordinateDimensions::Xyz
    } else {
        CoordinateDimensions::Xy
    })
}

fn wkb_coords_from_kml(coords: &[KmlCoord]) -> Result<(Vec<WkbCoordinate>, CoordinateDimensions)> {
    let dimensions = dimensions_for_kml_coords(coords)?;
    Ok((
        coords
            .iter()
            .map(|coordinate| WkbCoordinate {
                x: coordinate.x,
                y: coordinate.y,
                z: coordinate.z,
                m: None,
            })
            .collect(),
        dimensions,
    ))
}

fn wkb_geometry_from_kml(geometry: &KmlGeometry) -> Result<WkbGeometry> {
    let (value, dimensions) = match geometry {
        KmlGeometry::Point(point) => (
            WkbValue::Point(WkbCoordinate {
                x: point.coord.x,
                y: point.coord.y,
                z: point.coord.z,
                m: None,
            }),
            if point.coord.z.is_some() {
                CoordinateDimensions::Xyz
            } else {
                CoordinateDimensions::Xy
            },
        ),
        KmlGeometry::LineString(line) => {
            let (coordinates, dimensions) = wkb_coords_from_kml(&line.coords)?;
            (WkbValue::LineString(coordinates), dimensions)
        }
        KmlGeometry::LinearRing(ring) => {
            let (coordinates, dimensions) = wkb_coords_from_kml(&ring.coords)?;
            (WkbValue::LineString(coordinates), dimensions)
        }
        KmlGeometry::Polygon(polygon) => {
            let (outer, dimensions) = wkb_coords_from_kml(&polygon.outer.coords)?;
            let mut rings = Vec::with_capacity(1 + polygon.inner.len());
            rings.push(outer);
            for inner in &polygon.inner {
                let (ring, inner_dimensions) = wkb_coords_from_kml(&inner.coords)?;
                if inner_dimensions != dimensions {
                    return Err(err("anelli KML con dimensionalità Z non uniforme"));
                }
                rings.push(ring);
            }
            (WkbValue::Polygon(rings), dimensions)
        }
        KmlGeometry::MultiGeometry(multi) => {
            let values = multi
                .geometries
                .iter()
                .map(wkb_geometry_from_kml)
                .collect::<Result<Vec<_>>>()?;
            let dimensions = values
                .first()
                .map(|value| value.dimensions)
                .unwrap_or(CoordinateDimensions::Xy);
            if values.iter().any(|value| value.dimensions != dimensions) {
                return Err(err("MultiGeometry KML con dimensionalità Z non uniforme"));
            }
            let value = if values
                .iter()
                .all(|value| matches!(value.value, WkbValue::Point(_)))
            {
                WkbValue::MultiPoint(values)
            } else if values
                .iter()
                .all(|value| matches!(value.value, WkbValue::LineString(_)))
            {
                WkbValue::MultiLineString(values)
            } else if values
                .iter()
                .all(|value| matches!(value.value, WkbValue::Polygon(_)))
            {
                WkbValue::MultiPolygon(values)
            } else {
                WkbValue::GeometryCollection(values)
            };
            (value, dimensions)
        }
        KmlGeometry::Element(_) => {
            return Err(err(
                "elemento geometrico KML generico non rappresentabile in WKB",
            ))
        }
        _ => return Err(err("tipo geometrico KML non supportato")),
    };
    Ok(WkbGeometry {
        value,
        dimensions,
        srid: None,
    })
}

fn kml_coord_from_wkb(
    coordinate: &WkbCoordinate,
    dimensions: CoordinateDimensions,
) -> Result<KmlCoord> {
    if coordinate.m.is_some() {
        return Err(err("KML non rappresenta l’ordinata M"));
    }
    let z = match dimensions {
        CoordinateDimensions::Xy if coordinate.z.is_none() => None,
        CoordinateDimensions::Xyz => Some(
            coordinate
                .z
                .ok_or_else(|| err("coordinata WKB XYZ senza z"))?,
        ),
        CoordinateDimensions::Xy => return Err(err("coordinata WKB XY con z inattesa")),
        CoordinateDimensions::Xym | CoordinateDimensions::Xyzm => {
            return Err(err("KML non rappresenta l’ordinata M"))
        }
        CoordinateDimensions::Unknown => {
            return Err(err("dimensionalità WKB ignota non scrivibile in KML"))
        }
    };
    Ok(KmlCoord::new(coordinate.x, coordinate.y, z))
}

fn kml_coords_from_wkb(
    coordinates: &[WkbCoordinate],
    dimensions: CoordinateDimensions,
) -> Result<Vec<KmlCoord>> {
    coordinates
        .iter()
        .map(|coordinate| kml_coord_from_wkb(coordinate, dimensions))
        .collect()
}

fn kml_geometry_from_wkb(geometry: &WkbGeometry) -> Result<KmlGeometry> {
    if geometry.srid.is_some() {
        return Err(err("SRID EWKB non rappresentabile nel payload KML"));
    }
    let dimensions = geometry.dimensions;
    match &geometry.value {
        WkbValue::Point(coordinate) => Ok(KmlGeometry::Point(KmlPoint::from(kml_coord_from_wkb(
            coordinate, dimensions,
        )?))),
        WkbValue::LineString(coordinates) => Ok(KmlGeometry::LineString(KmlLineString::from(
            kml_coords_from_wkb(coordinates, dimensions)?,
        ))),
        WkbValue::Polygon(rings) => {
            let (outer, inner) = rings.split_first().ok_or_else(|| {
                err("Polygon WKB senza anello esterno non rappresentabile in KML")
            })?;
            let outer = LinearRing::from(kml_coords_from_wkb(outer, dimensions)?);
            let inner = inner
                .iter()
                .map(|ring| kml_coords_from_wkb(ring, dimensions).map(LinearRing::from))
                .collect::<Result<Vec<_>>>()?;
            Ok(KmlGeometry::Polygon(KmlPolygon::new(outer, inner)))
        }
        WkbValue::MultiPoint(values)
        | WkbValue::MultiLineString(values)
        | WkbValue::MultiPolygon(values) => Ok(KmlGeometry::MultiGeometry(MultiGeometry::new(
            values
                .iter()
                .map(kml_geometry_from_wkb)
                .collect::<Result<Vec<_>>>()?,
        ))),
        WkbValue::GeometryCollection(values) => {
            let homogeneous = values
                .first()
                .map(|first| {
                    let first_type = first.geometry_type();
                    values
                        .iter()
                        .all(|value| value.geometry_type() == first_type)
                })
                .unwrap_or(false);
            if homogeneous {
                return Err(err(
                    "GeometryCollection omogenea ambigua in KML: usare il tipo Multi* corrispondente",
                ));
            }
            Ok(KmlGeometry::MultiGeometry(MultiGeometry::new(
                values
                    .iter()
                    .map(kml_geometry_from_wkb)
                    .collect::<Result<Vec<_>>>()?,
            )))
        }
    }
}

fn build_batch(placemarks: &[Placemark]) -> Result<(RecordBatch, DataContract)> {
    let mut wkb: Vec<Option<Vec<u8>>> = Vec::with_capacity(placemarks.len());
    let mut names: Vec<Option<String>> = Vec::with_capacity(placemarks.len());
    let mut descs: Vec<Option<String>> = Vec::with_capacity(placemarks.len());
    let mut dimensions = BTreeSet::new();
    let mut geometry_types = BTreeSet::new();
    for p in placemarks {
        match &p.geometry {
            None => wkb.push(None),
            Some(geometry) => {
                let geometry = wkb_geometry_from_kml(geometry)?;
                dimensions.insert(geometry.dimensions);
                geometry_types.insert(geometry.geometry_type());
                wkb.push(Some(encode_wkb(&geometry, WkbFlavor::Iso)?));
            }
        }
        names.push(p.name.clone());
        descs.push(p.description.clone());
    }

    let mut geometry_contract =
        GeometryColumnContract::wkb_passthrough(FieldId(0), GEOMETRY, ResolvedCrs::wgs84(), true);
    geometry_contract.dimensions = if dimensions.len() == 1 {
        dimensions
            .first()
            .copied()
            .unwrap_or(CoordinateDimensions::Unknown)
    } else {
        CoordinateDimensions::Unknown
    };
    geometry_contract.geometry_types = geometry_types.into_iter().collect::<Vec<GeometryType>>();
    let fields = vec![
        with_geometry_contract_metadata(&geometry_field(GEOMETRY, OGC_CRS84), &geometry_contract),
        Field::new("name", DataType::Utf8, true),
        Field::new("description", DataType::Utf8, true),
    ];
    let arrays: Vec<ArrayRef> = vec![
        Arc::new(BinaryArray::from(
            wkb.iter().map(|o| o.as_deref()).collect::<Vec<_>>(),
        )),
        Arc::new(StringArray::from(names)),
        Arc::new(StringArray::from(descs)),
    ];
    let schema: SchemaRef = Arc::new(Schema::new(fields));
    let batch =
        RecordBatch::try_new(schema.clone(), arrays).map_err(|e| err(format!("batch: {e}")))?;
    let contract = DataContract {
        schema,
        geometry: Some(geometry_contract),
    };
    Ok((batch, contract))
}

/// Entry point non stabile per libFuzzer: parser KML e conversione diretta
/// KML→WKB devono rifiutare input ostili senza panic.
#[doc(hidden)]
pub fn __fuzz_read_kml(bytes: &[u8]) -> Result<usize> {
    let text = std::str::from_utf8(bytes).map_err(|error| err(format!("UTF-8 KML: {error}")))?;
    validate_kml_xml(text)?;
    let document: Kml = text
        .parse()
        .map_err(|error| err(format!("KML non valido: {error}")))?;
    let mut placemarks = Vec::new();
    collect(&document, &mut placemarks);
    let (batch, _) = build_batch(&placemarks)?;
    Ok(batch.num_rows())
}

#[cfg(test)]
mod tests {
    use super::*;
    use plenora_core::wkb::to_wkb;
    use plenora_io_core::request::{BatchTarget, ProjectionMode};
    use plenora_io_core::WriteLayer;

    const SAMPLE: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
    <kml xmlns="http://www.opengis.net/kml/2.2"><Document>
      <Placemark><name>A</name><description>primo</description>
        <Point><coordinates>12.5,45.9,0</coordinates></Point></Placemark>
      <Placemark><name>B</name>
        <LineString><coordinates>0,0,0 1,1,0</coordinates></LineString></Placemark>
    </Document></kml>"#;

    const FUZZ_TIMEOUT_REGRESSION: &[u8] = br#"<kml xmlns="http://www.opengis.net/kml/2.2"><Placemark><MultiGeomgis.net/kml/2.2"><Placemark><MultiGeometry>></LikeString></MultiGww.opengis.net/kml/2.2etry>></LikeString></MultiGww.opengis.net/kml/2.2"><>"#;
    const FUZZ_EMPTY_POINT_REGRESSION: &[u8] =
        br#"<kml xmlns="httpw.opengis.net/kml/2.2"><Placemark><Point></Point></Placemark></kml>"#;

    #[test]
    fn rejects_malformed_xml_that_stalled_the_kml_parser() {
        let started = std::time::Instant::now();
        assert!(__fuzz_read_kml(FUZZ_TIMEOUT_REGRESSION).is_err());
        assert!(started.elapsed() < std::time::Duration::from_secs(1));
    }

    #[test]
    fn rejects_empty_point_before_dependency_parser() {
        assert!(__fuzz_read_kml(FUZZ_EMPTY_POINT_REGRESSION).is_err());
        assert!(__fuzz_read_kml(
            br#"<kml><Placemark><Point><coordinates> </coordinates></Point></Placemark></kml>"#
        )
        .is_err());
        assert!(__fuzz_read_kml(
            br#"<kml><Placemark><Point><coordinates><![CDATA[ ]]></coordinates></Point></Placemark></kml>"#
        )
        .is_err());
    }

    #[test]
    fn reads_kml_placemarks() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("in.kml");
        std::fs::write(&path, SAMPLE).unwrap();
        let driver = KmlDriver;
        let ds = driver
            .open(Source::Path(path), &ReadOptions::default())
            .unwrap();
        assert_eq!(
            ds.layers()[0]
                .contract
                .geometry
                .as_ref()
                .unwrap()
                .resolved_crs()
                .unwrap()
                .axis_order,
            plenora_core::crs::AxisOrder::LongitudeLatitude
        );
        let mut r = ds
            .open_layer_reader(&ReadRequest {
                layer: LayerId(0),
                projected_fields: None,
                projection_mode: ProjectionMode::BestEffort,
                pruning_predicate: None,
                spatial_pruning_hint: None,
                batch_target: BatchTarget {
                    target_bytes: usize::MAX,
                    max_rows: 1,
                },
            })
            .unwrap();
        let batch = r.next_batch().unwrap().unwrap();
        assert_eq!(batch.num_rows(), 1);
        assert_eq!(batch.num_columns(), 3);
        assert_eq!(
            r.contract().contract.geometry.as_ref().unwrap().dimensions,
            CoordinateDimensions::Xyz
        );
        let geometries = batch
            .column(0)
            .as_any()
            .downcast_ref::<BinaryArray>()
            .unwrap();
        let point = decode_wkb(geometries.value(0), &WkbLimits::default()).unwrap();
        assert!(matches!(
            point.value,
            WkbValue::Point(WkbCoordinate { z: Some(0.0), .. })
        ));
        assert_eq!(r.next_batch().unwrap().unwrap().num_rows(), 1);
        assert!(r.next_batch().unwrap().is_none());
    }

    #[test]
    fn write_then_read_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("out.kml");
        let wkb = to_wkb(&geo_types::Geometry::Point(geo_types::Point::new(
            12.5, 45.9,
        )))
        .unwrap();
        let schema: SchemaRef = Arc::new(Schema::new(vec![
            geometry_field(GEOMETRY, OGC_CRS84),
            Field::new("name", DataType::Utf8, true),
            Field::new("description", DataType::Utf8, true),
        ]));
        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![
                Arc::new(BinaryArray::from(vec![Some(wkb.as_slice())])),
                Arc::new(StringArray::from(vec!["Roma"])),
                Arc::new(StringArray::from(vec!["capitale"])),
            ],
        )
        .unwrap();

        let driver = KmlDriver;
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

        let ds = driver
            .open(Source::Path(out), &ReadOptions::default())
            .unwrap();
        let mut r = ds
            .open_layer_reader(&ReadRequest {
                layer: LayerId(0),
                projected_fields: None,
                projection_mode: ProjectionMode::BestEffort,
                pruning_predicate: None,
                spatial_pruning_hint: None,
                batch_target: BatchTarget::default(),
            })
            .unwrap();
        let rb = r.next_batch().unwrap().unwrap();
        assert_eq!(rb.num_rows(), 1);
        let name = rb
            .column_by_name("name")
            .unwrap()
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap();
        assert_eq!(name.value(0), "Roma");
    }

    #[test]
    fn xyz_round_trip_preserves_altitude() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("xyz.kml");
        let geometry = WkbGeometry {
            value: WkbValue::Point(WkbCoordinate {
                x: 12.5,
                y: 45.9,
                z: Some(123.25),
                m: None,
            }),
            dimensions: CoordinateDimensions::Xyz,
            srid: None,
        };
        let wkb = encode_wkb(&geometry, WkbFlavor::Iso).unwrap();
        let mut geometry_contract =
            GeometryColumnContract::wkb_xy(FieldId(0), GEOMETRY, ResolvedCrs::wgs84(), true);
        geometry_contract.dimensions = CoordinateDimensions::Xyz;
        geometry_contract.geometry_types = vec![GeometryType::Point];
        let schema: SchemaRef = Arc::new(Schema::new(vec![
            with_geometry_contract_metadata(
                &geometry_field(GEOMETRY, OGC_CRS84),
                &geometry_contract,
            ),
            Field::new("name", DataType::Utf8, true),
            Field::new("description", DataType::Utf8, true),
        ]));
        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![
                Arc::new(BinaryArray::from(vec![Some(wkb.as_slice())])),
                Arc::new(StringArray::from(vec!["Quota"])),
                Arc::new(StringArray::from(vec!["XYZ"])),
            ],
        )
        .unwrap();
        let plan = WritePlan {
            layers: vec![WriteLayer {
                name: "xyz".to_owned(),
                contract: DataContract {
                    schema,
                    geometry: Some(geometry_contract),
                },
            }],
        };

        let driver = KmlDriver;
        let mut writer = driver
            .create(Sink::Path(out.clone()), &plan, &WriteOptions::default())
            .unwrap();
        writer.write(&batch).unwrap();
        writer.finish().unwrap();
        assert!(std::fs::read_to_string(&out)
            .unwrap()
            .contains("12.5,45.9,123.25"));

        let dataset = driver
            .open(Source::Path(out), &ReadOptions::default())
            .unwrap();
        assert_eq!(
            dataset.layers()[0]
                .contract
                .geometry
                .as_ref()
                .unwrap()
                .dimensions,
            CoordinateDimensions::Xyz
        );
        let mut reader = dataset
            .open_layer_reader(&ReadRequest {
                layer: LayerId(0),
                projected_fields: None,
                projection_mode: ProjectionMode::BestEffort,
                pruning_predicate: None,
                spatial_pruning_hint: None,
                batch_target: BatchTarget::default(),
            })
            .unwrap();
        let round_trip = reader.next_batch().unwrap().unwrap();
        let geometries = round_trip
            .column(0)
            .as_any()
            .downcast_ref::<BinaryArray>()
            .unwrap();
        let decoded = decode_wkb(geometries.value(0), &WkbLimits::default()).unwrap();
        assert!(matches!(
            decoded.value,
            WkbValue::Point(WkbCoordinate {
                z: Some(123.25),
                ..
            })
        ));
    }

    #[test]
    fn xym_contract_is_rejected_before_output_creation() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("m.kml");
        let mut geometry =
            GeometryColumnContract::wkb_xy(FieldId(0), GEOMETRY, ResolvedCrs::wgs84(), true);
        geometry.dimensions = CoordinateDimensions::Xym;
        let schema: SchemaRef = Arc::new(Schema::new(vec![
            with_geometry_contract_metadata(&geometry_field(GEOMETRY, OGC_CRS84), &geometry),
            Field::new("name", DataType::Utf8, true),
            Field::new("description", DataType::Utf8, true),
        ]));
        let plan = WritePlan {
            layers: vec![WriteLayer {
                name: "m".to_owned(),
                contract: DataContract {
                    schema,
                    geometry: Some(geometry),
                },
            }],
        };
        let driver = KmlDriver;
        assert!(driver
            .create(Sink::Path(out.clone()), &plan, &WriteOptions::default())
            .is_err());
        assert!(!out.exists());
    }

    #[test]
    fn direct_conversion_preserves_xyz_multipolygon() {
        let coordinate = |x, y, z| WkbCoordinate {
            x,
            y,
            z: Some(z),
            m: None,
        };
        let polygon = WkbGeometry {
            value: WkbValue::Polygon(vec![vec![
                coordinate(0.0, 0.0, 10.0),
                coordinate(1.0, 0.0, 11.0),
                coordinate(1.0, 1.0, 12.0),
                coordinate(0.0, 0.0, 10.0),
            ]]),
            dimensions: CoordinateDimensions::Xyz,
            srid: None,
        };
        let geometry = WkbGeometry {
            value: WkbValue::MultiPolygon(vec![polygon]),
            dimensions: CoordinateDimensions::Xyz,
            srid: None,
        };
        let kml = kml_geometry_from_wkb(&geometry).unwrap();
        assert_eq!(wkb_geometry_from_kml(&kml).unwrap(), geometry);
    }

    #[test]
    fn homogeneous_geometry_collection_is_rejected_as_ambiguous() {
        let point = |x| WkbGeometry {
            value: WkbValue::Point(WkbCoordinate {
                x,
                y: 2.0,
                z: None,
                m: None,
            }),
            dimensions: CoordinateDimensions::Xy,
            srid: None,
        };
        let geometry = WkbGeometry {
            value: WkbValue::GeometryCollection(vec![point(1.0), point(3.0)]),
            dimensions: CoordinateDimensions::Xy,
            srid: None,
        };
        assert!(kml_geometry_from_wkb(&geometry).is_err());
    }
}
