//! driver-kml — KML ⇄ `RecordBatch`. KML è WGS84 per specifica (`OGC:CRS84`).
//! I Placemark diventano feature con geometria WKB `geoarrow.wkb` XY/XYZ e
//! `name`/`description` come proprietà. KMZ resta un incremento successivo.
#![forbid(unsafe_code)]

use std::collections::{BTreeSet, HashMap};
use std::fs::File;
use std::io::{BufRead, BufReader, BufWriter, Read, Write as _};
use std::path::Path;
use std::sync::Arc;

use arrow_array::{Array, ArrayRef, BinaryArray, RecordBatch, StringArray};
use arrow_schema::{DataType, Field, Schema, SchemaRef};
use kml::types::{
    coords_from_str, Coord as KmlCoord, Element, Geometry as KmlGeometry,
    LineString as KmlLineString, LinearRing, MultiGeometry, Placemark, Point as KmlPoint,
    Polygon as KmlPolygon,
};
use kml::{Kml, KmlWriter};

use driver_common::{cell_string, geometry_field, geometry_index, OGC_CRS84};
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
use plenora_io_core::request::ReadRequest;
use plenora_io_core::{
    check_cancelled, check_cancelled_periodically, read_row_error, validate_write,
    with_write_validation, write_row_rejection, AttributeWriteSupport,
    CrsRepresentationCapabilities, CrsRepresentationState, CrsWriteSupport,
    FormatWriteCapabilities, NullabilitySupport, SingleReaderGate, TypeCoercionPolicy, WritePlan,
    SCALAR_TYPES, UTF8_FIELD_NAMES, WKB_XY_XYZ_GEOMETRY,
};
use plenora_io_model::budget::{OperationBudget, SpillLease};
#[cfg(test)]
use plenora_io_model::contract::GeometryType;
use plenora_io_model::contract::{
    CoordinateDimensions, DataContract, FieldId, GeometryColumnContract, LayerContract, LayerId,
};
use plenora_io_model::crs::ResolvedCrs;
use plenora_io_model::geometry::with_geometry_contract_metadata;
use plenora_io_model::limits::WkbLimits;
use plenora_io_model::wkb::{
    decode_wkb, encode_wkb, WkbCoordinate, WkbFlavor, WkbGeometry, WkbValue,
};
use plenora_io_model::{CancellationToken, ErrorPhase, PlenoraIoError, Result};
use quick_xml::events::{BytesStart, Event};
use quick_xml::Reader as XmlReader;

const GEOMETRY: &str = "geometry";
const MAX_XML_DEPTH: usize = 256;
const KML_IO_BUFFER_BYTES: usize = 4 * 1024 * 1024;

fn err(reason: impl Into<String>) -> PlenoraIoError {
    PlenoraIoError::format("kml", reason)
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
    name.iter()
        .rposition(|byte| *byte == b':')
        .map_or(name, |separator| &name[separator + 1..])
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
fn validate_kml_xml<R: BufRead>(input: R, input_bytes: usize) -> Result<()> {
    let mut reader = XmlReader::from_reader(input);
    let mut event_buffer = Vec::new();
    let mut stack = Vec::<Vec<u8>>::new();
    let mut open_points_with_coordinates = Vec::<bool>::new();
    let mut previous_position = 0_u64;
    let mut events_left = input_bytes.saturating_add(1);

    loop {
        if events_left == 0 {
            return Err(err(
                "numero di eventi XML incoerente con la dimensione dell'input",
            ));
        }
        events_left -= 1;

        let event = reader
            .read_event_into(&mut event_buffer)
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
        event_buffer.clear();
    }
}

static DESCRIPTOR: FormatDescriptor = FormatDescriptor::const_new(
    "kml",
    Direction::Bidirectional,
    ReadMode::StreamingSequential,
    // INV-7: il parser riversa l'intera sorgente in uno spool all'apertura.
    plenora_io_core::NativeReadMode::Materialized,
    // Il drenaggio e lo spool sono dell'adapter comune, non di
    // questo driver: `BudgetedReader` li impone a tutti.
    plenora_io_core::DeliverySemantics::OperationAtomic,
    plenora_io_core::BufferingStrategy::AdaptiveMemoryThenDisk,
    plenora_io_core::DeterminismLevel::Semantic,
    Some(WriteMode::Streaming),
    Some(plenora_io_core::DeterminismLevel::Semantic),
    false,
    false,
    ReaderConcurrency::SingleActiveReader,
    plenora_io_core::ProjectionSupport::None,
    plenora_io_core::PredicatePruningSupport::None,
    plenora_io_core::SpatialPruningSupport::None,
    CrsHandling::FixedWgs84,
    Fidelity::Conditional,
    Runtime::PureRust,
    Some(FormatWriteCapabilities {
        field_names: UTF8_FIELD_NAMES,
        allowed_types: SCALAR_TYPES,
        type_coercion: TypeCoercionPolicy::Reject,
        attributes: AttributeWriteSupport::All,
        geometry: WKB_XY_XYZ_GEOMETRY,
        crs: CrsWriteSupport::Fixed("OGC:CRS84"),
        crs_representations: CrsRepresentationCapabilities::new(
            CrsRepresentationState::Derived,
            CrsRepresentationState::Absent,
            CrsRepresentationState::Absent,
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

pub struct KmlDriver;

impl FormatDriver for KmlDriver {
    fn descriptor(&self) -> &FormatDescriptor {
        &DESCRIPTOR
    }

    fn open(&self, source: Source, mut opts: ReadOptions) -> Result<Box<dyn OpenDatasetHandle>> {
        let path = plenora_io_core::preflight_source(self.descriptor(), source, &mut opts)?;
        let mut stream = PlacemarkStream::open(&path)?;
        let mut stats = KmlContractStats::default();
        let spool = Arc::new(tempfile::NamedTempFile::new()?);
        let mut spool_writer = KmlSpoolWriter::new(
            spool.as_file(),
            opts.max_input_bytes(),
            opts.budget().clone(),
        );
        while let Some(placemark) = stream.next_placemark(
            opts.cancellation(),
            u64::try_from(stats.rows)
                .map_err(|_| PlenoraIoError::LimitExceeded("troppe righe KML".to_owned()))?,
        )? {
            opts.ensure_active()?;
            if stats.rows >= opts.max_rows() {
                return Err(PlenoraIoError::LimitExceeded(format!(
                    "KML: più di {} Placemark",
                    opts.max_rows()
                )));
            }
            let source_index = u64::try_from(stats.rows)
                .map_err(|_| PlenoraIoError::LimitExceeded("troppe righe KML".to_owned()))?;
            let geometry = stats
                .observe(&placemark, opts.cancellation())
                .map_err(|error| {
                    read_row_error(
                        error,
                        Some(source_index),
                        "kml.geometry_not_representable",
                        Some(GEOMETRY),
                    )
                })?;
            spool_writer.row(
                geometry.as_deref(),
                placemark.name.as_deref(),
                placemark.description.as_deref(),
            )?;
        }
        spool_writer.finish()?;
        let rows = stats.rows;
        let contract = stats.contract();
        let name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("layer")
            .to_owned();
        Ok(plenora_io_core::with_read_budget(
            Box::new(KmlDataset {
                layers: vec![LayerContract {
                    id: LayerId(0),
                    name,
                    contract,
                }],
                spool,
                rows,
                reader_gate: SingleReaderGate::new(DESCRIPTOR.id()),
            }),
            &opts,
            true,
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
            return Err(PlenoraIoError::OutputExists(path.display().to_string()));
        }
        if !path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("kml"))
        {
            return Err(PlenoraIoError::Unsupported(
                "l'output deve avere estensione .kml".to_owned(),
            ));
        }
        if plan.layers.len() != 1 {
            return Err(PlenoraIoError::Unsupported(
                "KML: un solo layer per file".to_owned(),
            ));
        }
        let staging = StagedFile::new(&path, opts.durable, opts.max_output_bytes())?;
        let mut output = BufWriter::with_capacity(KML_IO_BUFFER_BYTES, staging.reopen()?);
        output.write_all(
            br#"<?xml version="1.0" encoding="UTF-8"?><kml xmlns="http://www.opengis.net/kml/2.2"><Document>"#,
        )?;
        with_write_validation(
            Box::new(KmlWriterState {
                staging,
                output,
                rows: 0,
                input_total: None,
                wkb_limits: opts.wkb_limits(),
            }),
            self.descriptor(),
            plan,
            opts,
        )
    }
}

struct KmlDataset {
    layers: Vec<LayerContract>,
    spool: Arc<tempfile::NamedTempFile>,
    rows: usize,
    reader_gate: SingleReaderGate,
}

impl OpenDatasetHandle for KmlDataset {
    fn layers(&self) -> &[LayerContract] {
        &self.layers
    }
    fn fidelity_assessment(&self) -> plenora_io_core::FidelityAssessment {
        plenora_io_core::FidelityAssessment::for_format(
            DESCRIPTOR.id(),
            DESCRIPTOR.fidelity_class(),
        )
    }
    fn open_layer_reader(&self, request: &ReadRequest) -> Result<Box<dyn LayerReader>> {
        plenora_io_core::validate_read_projection(&DESCRIPTOR, request)?;
        let reader = self.reader_gate.open(request.layer, || {
            Ok(Box::new(KmlReader {
                input: BufReader::with_capacity(KML_IO_BUFFER_BYTES, self.spool.reopen()?),
                remaining_rows: self.rows,
                layer: self.layers[0].clone(),
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

struct KmlReader {
    input: BufReader<File>,
    remaining_rows: usize,
    layer: LayerContract,
    batch_sizer: plenora_io_core::AdaptiveBatchSizer,
    cancellation: CancellationToken,
}

impl LayerReader for KmlReader {
    fn contract(&self) -> &LayerContract {
        &self.layer
    }

    fn next_batch(&mut self) -> Result<Option<RecordBatch>> {
        check_cancelled(&self.cancellation, ErrorPhase::Read)?;
        if self.remaining_rows == 0 {
            return Ok(None);
        }
        let rows = self.remaining_rows.min(self.batch_sizer.rows());
        let mut geometries = Vec::with_capacity(rows);
        let mut names = Vec::with_capacity(rows);
        let mut descriptions = Vec::with_capacity(rows);
        for index in 0..rows {
            check_cancelled_periodically(&self.cancellation, ErrorPhase::Read, index)?;
            geometries.push(read_spool_value(&mut self.input)?);
            names.push(read_spool_string(&mut self.input)?);
            descriptions.push(read_spool_string(&mut self.input)?);
        }
        self.remaining_rows -= rows;
        let arrays: Vec<ArrayRef> = vec![
            Arc::new(BinaryArray::from(
                geometries
                    .iter()
                    .map(|value| value.as_deref())
                    .collect::<Vec<_>>(),
            )),
            Arc::new(StringArray::from(names)),
            Arc::new(StringArray::from(descriptions)),
        ];
        let batch = RecordBatch::try_new(self.layer.contract.schema.clone(), arrays)
            .map_err(|error| err(format!("batch KML da spool: {error}")))?;
        self.batch_sizer.observe(&batch);
        Ok(Some(batch))
    }
}

const SPOOL_NULL: u32 = u32::MAX;

struct KmlSpoolWriter<'a> {
    output: BufWriter<&'a File>,
    bytes: u64,
    limit: u64,
    budget: OperationBudget,
    /// Le prenotazioni di spill restano vive quanto il file temporaneo.
    ///
    /// Nel modello legacy si faceva `commit`, cioe' consumo definitivo: la
    /// quota non tornava mai, nemmeno dopo che il file era stato rimosso. Nel
    /// modello unificato lo spill e' occupazione trattenuta e la `SpillLease`
    /// la restituisce al drop, insieme allo spool che l'ha creata.
    leases: Vec<SpillLease>,
}

impl<'a> KmlSpoolWriter<'a> {
    fn new(file: &'a File, limit: u64, budget: OperationBudget) -> Self {
        Self {
            output: BufWriter::new(file),
            bytes: 0,
            limit,
            budget,
            leases: Vec::new(),
        }
    }

    fn write(&mut self, bytes: &[u8]) -> Result<()> {
        let length =
            u64::try_from(bytes.len()).map_err(|_| err("spool KML non rappresentabile"))?;
        let next = self
            .bytes
            .checked_add(length)
            .ok_or_else(|| err("dimensione spool KML fuori intervallo"))?;
        if next > self.limit {
            return Err(PlenoraIoError::LimitExceeded(format!(
                "spool KML: {next} byte eccedono il limite {}",
                self.limit
            )));
        }
        let lease = self.budget.context().lease_spill(length)?;
        self.output.write_all(bytes)?;
        self.leases.push(lease);
        self.bytes = next;
        Ok(())
    }

    fn value(&mut self, value: Option<&[u8]>) -> Result<()> {
        let length = match value {
            None => SPOOL_NULL,
            Some(bytes) => u32::try_from(bytes.len()).map_err(|_| {
                PlenoraIoError::LimitExceeded("valore KML troppo grande per lo spool".to_owned())
            })?,
        };
        self.write(&length.to_le_bytes())?;
        if let Some(bytes) = value {
            self.write(bytes)?;
        }
        Ok(())
    }

    fn row(
        &mut self,
        geometry: Option<&[u8]>,
        name: Option<&str>,
        description: Option<&str>,
    ) -> Result<()> {
        self.value(geometry)?;
        self.value(name.map(str::as_bytes))?;
        self.value(description.map(str::as_bytes))
    }

    fn finish(mut self) -> Result<()> {
        self.output.flush()?;
        Ok(())
    }
}

fn read_spool_value(input: &mut impl Read) -> Result<Option<Vec<u8>>> {
    let mut length = [0_u8; 4];
    input
        .read_exact(&mut length)
        .map_err(|error| err(format!("spool KML troncato: {error}")))?;
    let length = u32::from_le_bytes(length);
    if length == SPOOL_NULL {
        return Ok(None);
    }
    let mut value =
        vec![0; usize::try_from(length).map_err(|_| err("lunghezza spool KML non valida"))?];
    input
        .read_exact(&mut value)
        .map_err(|error| err(format!("spool KML troncato: {error}")))?;
    Ok(Some(value))
}

fn read_spool_string(input: &mut impl Read) -> Result<Option<String>> {
    let Some(bytes) = read_spool_value(input)? else {
        return Ok(None);
    };
    String::from_utf8(bytes)
        .map(Some)
        .map_err(|error| err(format!("testo spool KML non UTF-8: {error}")))
}

struct PlacemarkStream {
    reader: XmlReader<BufReader<File>>,
    event_buffer: Vec<u8>,
    ancestors: Vec<Vec<u8>>,
    visited_events: usize,
    events_left: usize,
    previous_position: u64,
    xml_depth: usize,
    element_stack: Vec<Vec<u8>>,
    open_points_with_coordinates: Vec<bool>,
}

impl PlacemarkStream {
    fn open(path: &Path) -> Result<Self> {
        let input_bytes = usize::try_from(std::fs::metadata(path)?.len())
            .map_err(|_| err("dimensione KML non rappresentabile"))?;
        let input = BufReader::with_capacity(KML_IO_BUFFER_BYTES, File::open(path)?);
        Ok(Self {
            reader: XmlReader::from_reader(input),
            event_buffer: Vec::new(),
            ancestors: Vec::new(),
            visited_events: 0,
            events_left: input_bytes.saturating_add(1),
            previous_position: 0,
            xml_depth: 0,
            element_stack: Vec::new(),
            open_points_with_coordinates: Vec::new(),
        })
    }

    fn next_event(&mut self, cancellation: &CancellationToken) -> Result<Event<'static>> {
        if self.events_left == 0 {
            return Err(err(
                "numero di eventi XML incoerente con la dimensione dell'input",
            ));
        }
        self.events_left -= 1;
        check_cancelled_periodically(cancellation, ErrorPhase::Read, self.visited_events)?;
        self.visited_events = self.visited_events.saturating_add(1);
        self.event_buffer.clear();
        let event = self
            .reader
            .read_event_into(&mut self.event_buffer)
            .map(Event::into_owned)
            .map_err(|error| err(format!("XML KML non valido: {error}")))?;
        let position = self.reader.buffer_position();
        if !matches!(event, Event::Eof) && position <= self.previous_position {
            return Err(err("parser XML senza avanzamento"));
        }
        self.previous_position = position;
        match &event {
            Event::Start(element) => {
                validate_element(element)?;
                if self.xml_depth >= MAX_XML_DEPTH {
                    return Err(err(format!(
                        "profondità XML oltre il limite di {MAX_XML_DEPTH}"
                    )));
                }
                self.xml_depth += 1;
                if element.local_name().as_ref() == b"Point" {
                    self.open_points_with_coordinates.push(false);
                }
                self.element_stack.push(element.name().as_ref().to_vec());
            }
            Event::Empty(element) => {
                validate_element(element)?;
                if element.local_name().as_ref() == b"Point" {
                    return Err(err("Point KML senza coordinate"));
                }
            }
            Event::Text(text) => observe_point_coordinate_text(
                &self.element_stack,
                &mut self.open_points_with_coordinates,
                text.as_ref(),
            )?,
            Event::CData(text) => observe_point_coordinate_text(
                &self.element_stack,
                &mut self.open_points_with_coordinates,
                text.as_ref(),
            )?,
            Event::GeneralRef(_) => observe_point_coordinate_text(
                &self.element_stack,
                &mut self.open_points_with_coordinates,
                b"x",
            )?,
            Event::End(element) => {
                if !valid_xml_name(element.name().as_ref()) {
                    return Err(err("nome di chiusura XML non valido"));
                }
                let opened = self
                    .element_stack
                    .pop()
                    .ok_or_else(|| err("chiusura XML senza elemento aperto"))?;
                if opened.as_slice() != element.name().as_ref() {
                    return Err(err("elementi XML annidati in modo non valido"));
                }
                if element.local_name().as_ref() == b"Point" {
                    let has_coordinates = self
                        .open_points_with_coordinates
                        .pop()
                        .ok_or_else(|| err("chiusura Point KML senza apertura"))?;
                    if !has_coordinates {
                        return Err(err("Point KML senza coordinate"));
                    }
                }
                self.xml_depth = self
                    .xml_depth
                    .checked_sub(1)
                    .ok_or_else(|| err("chiusura XML senza elemento aperto"))?;
            }
            Event::DocType(_) => return Err(err("DOCTYPE non ammesso nei documenti KML")),
            Event::Eof if self.xml_depth != 0 => return Err(err("documento XML troncato")),
            _ => {}
        }
        Ok(event)
    }

    fn traversed_by_legacy_reader(&self) -> bool {
        self.ancestors
            .iter()
            .all(|name| matches!(local_xml_name(name), b"kml" | b"Document" | b"Folder"))
    }

    fn decode_general_ref(reference: &quick_xml::events::BytesRef<'_>) -> Result<String> {
        if let Some(character) = reference
            .resolve_char_ref()
            .map_err(|error| err(format!("riferimento XML non valido: {error}")))?
        {
            return Ok(character.to_string());
        }
        let name = reference
            .decode()
            .map_err(|error| err(format!("riferimento XML non valido: {error}")))?;
        quick_xml::escape::resolve_xml_entity(&name)
            .map(str::to_owned)
            .ok_or_else(|| err(format!("entità XML sconosciuta: &{name};")))
    }

    fn read_text(&mut self, cancellation: &CancellationToken) -> Result<String> {
        let mut output = String::new();
        loop {
            match self.next_event(cancellation)? {
                Event::Text(text) => output.push_str(&text.decode().map_or_else(
                    |_| text.escape_ascii().to_string(),
                    std::borrow::Cow::into_owned,
                )),
                Event::GeneralRef(reference) => {
                    output.push_str(&Self::decode_general_ref(&reference)?);
                }
                Event::CData(text) => output.push_str(
                    &String::from_utf8(text.to_vec())
                        .unwrap_or_else(|_| text.escape_ascii().to_string()),
                ),
                Event::End(_) => return Ok(output),
                event => return Err(err(format!("contenuto testuale KML non valido: {event:?}"))),
            }
        }
    }

    fn skip_element(&mut self, cancellation: &CancellationToken) -> Result<()> {
        let mut depth = 1usize;
        while depth > 0 {
            match self.next_event(cancellation)? {
                Event::Start(_) => depth = depth.saturating_add(1),
                Event::End(_) => depth = depth.saturating_sub(1),
                Event::Eof => return Err(err("documento KML troncato")),
                _ => {}
            }
        }
        Ok(())
    }

    fn read_geometry_coordinates(
        &mut self,
        end_tag: &[u8],
        cancellation: &CancellationToken,
    ) -> Result<Vec<KmlCoord>> {
        let mut coordinates = Vec::new();
        loop {
            match self.next_event(cancellation)? {
                Event::Start(element) if element.local_name().as_ref() == b"coordinates" => {
                    let text = self.read_text(cancellation)?;
                    coordinates = coords_from_str(&text)
                        .map_err(|error| err(format!("coordinate KML non valide: {error}")))?;
                }
                Event::Start(_) => self.skip_element(cancellation)?,
                Event::End(element) if element.local_name().as_ref() == end_tag => {
                    return Ok(coordinates)
                }
                Event::Eof => return Err(err("documento KML troncato nella geometria")),
                _ => {}
            }
        }
    }

    fn read_boundary(
        &mut self,
        end_tag: &[u8],
        cancellation: &CancellationToken,
    ) -> Result<Vec<LinearRing>> {
        let mut rings = Vec::new();
        loop {
            match self.next_event(cancellation)? {
                Event::Start(element) if element.local_name().as_ref() == b"LinearRing" => {
                    rings.push(LinearRing::from(
                        self.read_geometry_coordinates(b"LinearRing", cancellation)?,
                    ));
                }
                Event::Start(_) => self.skip_element(cancellation)?,
                Event::End(element) if element.local_name().as_ref() == end_tag => {
                    return Ok(rings)
                }
                Event::Eof => return Err(err("documento KML troncato nel boundary")),
                _ => {}
            }
        }
    }

    fn read_polygon(&mut self, cancellation: &CancellationToken) -> Result<KmlPolygon> {
        let mut outer = None;
        let mut inner = Vec::new();
        loop {
            match self.next_event(cancellation)? {
                Event::Start(element) if element.local_name().as_ref() == b"outerBoundaryIs" => {
                    let rings = self.read_boundary(b"outerBoundaryIs", cancellation)?;
                    outer = rings.into_iter().next();
                }
                Event::Start(element) if element.local_name().as_ref() == b"innerBoundaryIs" => {
                    inner.extend(self.read_boundary(b"innerBoundaryIs", cancellation)?);
                }
                Event::Start(_) => self.skip_element(cancellation)?,
                Event::End(element) if element.local_name().as_ref() == b"Polygon" => {
                    let outer = outer.ok_or_else(|| err("Polygon KML senza anello esterno"))?;
                    return Ok(KmlPolygon::new(outer, inner));
                }
                Event::Eof => return Err(err("documento KML troncato nel Polygon")),
                _ => {}
            }
        }
    }

    fn read_multi_geometry(&mut self, cancellation: &CancellationToken) -> Result<MultiGeometry> {
        let mut geometries = Vec::new();
        loop {
            match self.next_event(cancellation)? {
                Event::Start(element) => {
                    if let Some(geometry) =
                        self.read_geometry(element.local_name().as_ref(), cancellation)?
                    {
                        geometries.push(geometry);
                    }
                }
                Event::End(element) if element.local_name().as_ref() == b"MultiGeometry" => {
                    return Ok(MultiGeometry::new(geometries))
                }
                Event::Eof => return Err(err("documento KML troncato nella MultiGeometry")),
                _ => {}
            }
        }
    }

    fn read_geometry(
        &mut self,
        name: &[u8],
        cancellation: &CancellationToken,
    ) -> Result<Option<KmlGeometry>> {
        Ok(match name {
            b"Point" => {
                let coordinates = self.read_geometry_coordinates(b"Point", cancellation)?;
                let coordinate = coordinates
                    .into_iter()
                    .next()
                    .ok_or_else(|| err("Point KML senza coordinate"))?;
                Some(KmlGeometry::Point(KmlPoint::from(coordinate)))
            }
            b"LineString" => Some(KmlGeometry::LineString(KmlLineString::from(
                self.read_geometry_coordinates(b"LineString", cancellation)?,
            ))),
            b"LinearRing" => Some(KmlGeometry::LinearRing(LinearRing::from(
                self.read_geometry_coordinates(b"LinearRing", cancellation)?,
            ))),
            b"Polygon" => Some(KmlGeometry::Polygon(self.read_polygon(cancellation)?)),
            b"MultiGeometry" => Some(KmlGeometry::MultiGeometry(
                self.read_multi_geometry(cancellation)?,
            )),
            b"Model" | b"Track" | b"MultiTrack" => {
                return Err(err("geometria KML non supportata dal contratto corrente"))
            }
            _ => {
                self.skip_element(cancellation)?;
                None
            }
        })
    }

    fn read_placemark(&mut self, cancellation: &CancellationToken) -> Result<Placemark> {
        let mut placemark = Placemark::default();
        loop {
            match self.next_event(cancellation)? {
                Event::Start(element) => match element.local_name().as_ref() {
                    b"name" => placemark.name = Some(self.read_text(cancellation)?),
                    b"description" => placemark.description = Some(self.read_text(cancellation)?),
                    name => {
                        if let Some(geometry) = self.read_geometry(name, cancellation)? {
                            if placemark.geometry.is_some() {
                                return Err(err("Placemark KML con piu geometrie top-level"));
                            }
                            placemark.geometry = Some(geometry);
                        }
                    }
                },
                Event::End(element) if element.local_name().as_ref() == b"Placemark" => {
                    return Ok(placemark)
                }
                Event::Eof => return Err(err("documento KML troncato nel Placemark")),
                _ => {}
            }
        }
    }

    fn next_placemark(
        &mut self,
        cancellation: &CancellationToken,
        source_index: u64,
    ) -> Result<Option<Placemark>> {
        loop {
            let event = self.next_event(cancellation)?;
            match event {
                Event::Start(element)
                    if element.local_name().as_ref() == b"Placemark"
                        && self.traversed_by_legacy_reader() =>
                {
                    return self
                        .read_placemark(cancellation)
                        .map(Some)
                        .map_err(|error| {
                            read_row_error(
                                error,
                                Some(source_index),
                                "kml.invalid_placemark",
                                Some(GEOMETRY),
                            )
                        });
                }
                Event::Empty(element)
                    if element.local_name().as_ref() == b"Placemark"
                        && self.traversed_by_legacy_reader() =>
                {
                    return Ok(Some(Placemark::default()));
                }
                Event::Start(element) => {
                    self.ancestors.push(element.name().as_ref().to_vec());
                }
                Event::End(_) => {
                    self.ancestors.pop();
                }
                Event::Eof => return Ok(None),
                _ => {}
            }
        }
    }
}

#[derive(Default)]
struct KmlContractStats {
    dimensions: BTreeSet<CoordinateDimensions>,
    geometry_types: BTreeSet<plenora_io_model::contract::GeometryType>,
    rows: usize,
    visited_geometry_items: usize,
}

impl KmlContractStats {
    fn observe(
        &mut self,
        placemark: &Placemark,
        cancellation: &CancellationToken,
    ) -> Result<Option<Vec<u8>>> {
        check_cancelled_periodically(cancellation, ErrorPhase::Read, self.rows)?;
        self.rows = self.rows.saturating_add(1);
        if let Some(geometry) = &placemark.geometry {
            let geometry = wkb_geometry_from_kml_cancellable(
                geometry,
                cancellation,
                &mut self.visited_geometry_items,
            )?;
            self.dimensions.insert(geometry.dimensions);
            self.geometry_types.insert(geometry.geometry_type());
            return encode_wkb(&geometry, WkbFlavor::Iso).map(Some);
        }
        Ok(None)
    }

    fn contract(self) -> DataContract {
        kml_contract(&self.dimensions, self.geometry_types)
    }
}

// --- scrittura -------------------------------------------------------------

struct KmlWriterState {
    staging: StagedFile,
    output: BufWriter<File>,
    rows: u64,
    input_total: Option<u64>,
    wkb_limits: WkbLimits,
}

impl FormatWriter for KmlWriterState {
    fn declare_input_total(&mut self, layer: LayerId, total: u64) -> Result<()> {
        if layer.0 != 0 {
            return Err(PlenoraIoError::Unsupported(
                "KML supporta un solo layer".to_owned(),
            ));
        }
        self.input_total = Some(total);
        Ok(())
    }

    // Il ciclo riga-per-riga classifica geometria, name/description ed
    // ExtendedData in un solo passaggio: separarlo duplicherebbe la gestione
    // delle reiezioni per riga senza cambiarne il comportamento.
    #[allow(clippy::too_many_lines)]
    fn write(&mut self, batch: &RecordBatch) -> Result<()> {
        let schema = batch.schema();
        let geom_idx =
            geometry_index(&schema).ok_or_else(|| err("nessuna colonna geometria geoarrow.wkb"))?;
        let geom_col = batch
            .column(geom_idx)
            .as_any()
            .downcast_ref::<BinaryArray>()
            .ok_or_else(|| err("colonna geometria non binaria"))?;
        let limits = self.wkb_limits;
        let name_idx = schema.index_of("name").ok();
        let desc_idx = schema.index_of("description").ok();
        let mut placemarks = Vec::with_capacity(batch.num_rows());
        let mut rejections = Vec::new();

        for row in 0..batch.num_rows() {
            let geometry = if geom_col.is_null(row) {
                None
            } else {
                let Ok(geometry) = decode_wkb(geom_col.value(row), &limits) else {
                    rejections.push((row, "kml.invalid_geometry", GEOMETRY));
                    continue;
                };
                let Ok(geometry) = kml_geometry_from_wkb(&geometry) else {
                    rejections.push((row, "kml.geometry_not_representable", GEOMETRY));
                    continue;
                };
                Some(geometry)
            };
            let Ok(name) = name_idx
                .map(|index| cell_string(batch.column(index), row))
                .transpose()
            else {
                rejections.push((row, "kml.cell_not_representable", "name"));
                continue;
            };
            let name = name.flatten();
            let Ok(description) = desc_idx
                .map(|index| cell_string(batch.column(index), row))
                .transpose()
            else {
                rejections.push((row, "kml.cell_not_representable", "description"));
                continue;
            };
            let description = description.flatten();

            // Colonne extra (non name/description/geometria) -> ExtendedData.
            let mut data = Vec::new();
            for (i, f) in schema.fields().iter().enumerate() {
                if i == geom_idx || Some(i) == name_idx || Some(i) == desc_idx {
                    continue;
                }
                match cell_string(batch.column(i), row) {
                    Ok(Some(value)) => data.push((f.name().clone(), value)),
                    Ok(None) => {}
                    Err(_) => {
                        rejections.push((row, "kml.cell_not_representable", f.name().as_str()));
                        data.clear();
                        break;
                    }
                }
            }
            if rejections
                .last()
                .is_some_and(|(rejected_row, _, _)| *rejected_row == row)
            {
                continue;
            }
            let children = if data.is_empty() {
                Vec::new()
            } else {
                vec![extended_data(&data)]
            };

            placemarks.push(Placemark {
                name,
                description,
                geometry,
                children,
                ..Default::default()
            });
        }
        if !rejections.is_empty() {
            return Err(write_row_rejection(
                "kml",
                self.rows,
                batch.num_rows(),
                &rejections,
                self.input_total,
            ));
        }
        let mut writer = KmlWriter::from_writer(&mut self.output);
        for placemark in placemarks {
            writer
                .write(&Kml::Placemark(placemark))
                .map_err(|error| err(format!("serializzazione KML: {error}")))?;
        }
        drop(writer);
        self.rows = self
            .rows
            .checked_add(
                u64::try_from(batch.num_rows())
                    .map_err(|_| PlenoraIoError::LimitExceeded("troppe righe KML".to_owned()))?,
            )
            .ok_or_else(|| PlenoraIoError::LimitExceeded("troppe righe KML".to_owned()))?;
        Ok(())
    }

    fn finish(mut self: Box<Self>) -> Result<Published> {
        self.output.write_all(b"</Document></kml>")?;
        self.output.flush()?;
        drop(self.output);
        let (bytes, outcome) = self.staging.publish()?;
        Ok(Published {
            bytes,
            loss: LossReport::default(),
            fidelity: plenora_io_core::FidelityAssessment::lossless(),
            outcome,
        })
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

fn collect<'a>(
    k: &'a Kml,
    out: &mut Vec<&'a Placemark>,
    cancellation: &CancellationToken,
    visited: &mut usize,
) -> Result<()> {
    check_cancelled_periodically(cancellation, ErrorPhase::Read, *visited)?;
    *visited = visited.saturating_add(1);
    match k {
        Kml::KmlDocument(d) => {
            for e in &d.elements {
                collect(e, out, cancellation, visited)?;
            }
        }
        Kml::Document { elements, .. } => {
            for e in elements {
                collect(e, out, cancellation, visited)?;
            }
        }
        Kml::Folder(folder) => {
            for e in &folder.elements {
                collect(e, out, cancellation, visited)?;
            }
        }
        Kml::Placemark(p) => out.push(p),
        _ => {}
    }
    Ok(())
}

#[cfg(test)]
fn dimensions_for_kml_coords(coords: &[KmlCoord]) -> Result<CoordinateDimensions> {
    if coords.is_empty() {
        return Err(err("geometria KML senza coordinate"));
    }
    let mut has_z = None;
    for coordinate in coords {
        let current = coordinate.z.is_some();
        if has_z.is_some_and(|known| known != current) {
            return Err(err("coordinate KML con dimensionalità Z non uniforme"));
        }
        has_z = Some(current);
    }
    Ok(if has_z == Some(true) {
        CoordinateDimensions::Xyz
    } else {
        CoordinateDimensions::Xy
    })
}

fn wkb_coords_from_kml_cancellable(
    coords: &[KmlCoord],
    cancellation: &CancellationToken,
    visited: &mut usize,
) -> Result<(Vec<WkbCoordinate>, CoordinateDimensions)> {
    if coords.is_empty() {
        return Err(err("geometria KML senza coordinate"));
    }
    let mut has_z = None;
    let mut coordinates = Vec::with_capacity(coords.len());
    for coordinate in coords {
        check_cancelled_periodically(cancellation, ErrorPhase::Read, *visited)?;
        *visited = visited.saturating_add(1);
        let current = coordinate.z.is_some();
        if has_z.is_some_and(|known| known != current) {
            return Err(err("coordinate KML con dimensionalità Z non uniforme"));
        }
        has_z = Some(current);
        coordinates.push(WkbCoordinate {
            x: coordinate.x,
            y: coordinate.y,
            z: coordinate.z,
            m: None,
        });
    }
    let dimensions = if has_z == Some(true) {
        CoordinateDimensions::Xyz
    } else {
        CoordinateDimensions::Xy
    };
    Ok((coordinates, dimensions))
}

fn wkb_geometry_from_kml_cancellable(
    geometry: &KmlGeometry,
    cancellation: &CancellationToken,
    visited: &mut usize,
) -> Result<WkbGeometry> {
    check_cancelled_periodically(cancellation, ErrorPhase::Read, *visited)?;
    *visited = visited.saturating_add(1);
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
            let (coordinates, dimensions) =
                wkb_coords_from_kml_cancellable(&line.coords, cancellation, visited)?;
            (WkbValue::LineString(coordinates), dimensions)
        }
        KmlGeometry::LinearRing(ring) => {
            let (coordinates, dimensions) =
                wkb_coords_from_kml_cancellable(&ring.coords, cancellation, visited)?;
            (WkbValue::LineString(coordinates), dimensions)
        }
        KmlGeometry::Polygon(polygon) => {
            let (outer, dimensions) =
                wkb_coords_from_kml_cancellable(&polygon.outer.coords, cancellation, visited)?;
            let mut rings = Vec::with_capacity(1 + polygon.inner.len());
            rings.push(outer);
            for inner in &polygon.inner {
                let (ring, inner_dimensions) =
                    wkb_coords_from_kml_cancellable(&inner.coords, cancellation, visited)?;
                if inner_dimensions != dimensions {
                    return Err(err("anelli KML con dimensionalità Z non uniforme"));
                }
                rings.push(ring);
            }
            (WkbValue::Polygon(rings), dimensions)
        }
        KmlGeometry::MultiGeometry(multi) => {
            let mut values = Vec::with_capacity(multi.geometries.len());
            for child in &multi.geometries {
                values.push(wkb_geometry_from_kml_cancellable(
                    child,
                    cancellation,
                    visited,
                )?);
            }
            let dimensions = values
                .first()
                .map(|value| value.dimensions)
                .ok_or_else(|| err("MultiGeometry KML vuota"))?;
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

#[cfg(test)]
fn wkb_geometry_from_kml(geometry: &KmlGeometry) -> Result<WkbGeometry> {
    wkb_geometry_from_kml_cancellable(geometry, &CancellationToken::new(), &mut 0)
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
            let first_type = values
                .first()
                .map(WkbGeometry::geometry_type)
                .ok_or_else(|| err("GeometryCollection vuota non rappresentabile in KML"))?;
            let homogeneous = values
                .iter()
                .all(|value| value.geometry_type() == first_type);
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
        WkbValue::CircularString(_)
        | WkbValue::CompoundCurve(_)
        | WkbValue::CurvePolygon(_)
        | WkbValue::MultiCurve(_)
        | WkbValue::MultiSurface(_)
        | WkbValue::PolyhedralSurface(_)
        | WkbValue::Tin(_)
        | WkbValue::Triangle(_) => Err(err(
            "tipo WKB esteso non rappresentabile in KML senza linearizzazione",
        )),
    }
}

fn kml_contract(
    dimensions: &BTreeSet<CoordinateDimensions>,
    geometry_types: BTreeSet<plenora_io_model::contract::GeometryType>,
) -> DataContract {
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
    geometry_contract.set_exact_geometry_types(geometry_types.into_iter().collect());
    let fields = vec![
        with_geometry_contract_metadata(&geometry_field(GEOMETRY, OGC_CRS84), &geometry_contract),
        Field::new("name", DataType::Utf8, true),
        Field::new("description", DataType::Utf8, true),
    ];
    let schema: SchemaRef = Arc::new(Schema::new(fields));
    DataContract::new(schema, Some(geometry_contract))
}

fn build_batch_cancellable(
    placemarks: &[&Placemark],
    cancellation: &CancellationToken,
) -> Result<(RecordBatch, DataContract)> {
    check_cancelled(cancellation, ErrorPhase::Read)?;
    let mut wkb: Vec<Option<Vec<u8>>> = Vec::with_capacity(placemarks.len());
    let mut names: Vec<Option<String>> = Vec::with_capacity(placemarks.len());
    let mut descs: Vec<Option<String>> = Vec::with_capacity(placemarks.len());
    let mut dimensions = BTreeSet::new();
    let mut geometry_types = BTreeSet::new();
    let mut visited_geometry_items = 0;
    for (index, p) in placemarks.iter().enumerate() {
        check_cancelled_periodically(cancellation, ErrorPhase::Read, index)?;
        match &p.geometry {
            None => wkb.push(None),
            Some(geometry) => {
                let geometry = wkb_geometry_from_kml_cancellable(
                    geometry,
                    cancellation,
                    &mut visited_geometry_items,
                )?;
                dimensions.insert(geometry.dimensions);
                geometry_types.insert(geometry.geometry_type());
                wkb.push(Some(encode_wkb(&geometry, WkbFlavor::Iso)?));
            }
        }
        names.push(p.name.clone());
        descs.push(p.description.clone());
    }

    let contract = kml_contract(&dimensions, geometry_types);
    let arrays: Vec<ArrayRef> = vec![
        Arc::new(BinaryArray::from(
            wkb.iter().map(|o| o.as_deref()).collect::<Vec<_>>(),
        )),
        Arc::new(StringArray::from(names)),
        Arc::new(StringArray::from(descs)),
    ];
    let batch = RecordBatch::try_new(contract.schema.clone(), arrays)
        .map_err(|e| err(format!("batch: {e}")))?;
    Ok((batch, contract))
}

fn build_batch(placemarks: &[&Placemark]) -> Result<(RecordBatch, DataContract)> {
    build_batch_cancellable(placemarks, &CancellationToken::new())
}

/// Entry point non stabile per libFuzzer: parser KML e conversione diretta
/// KML→WKB devono rifiutare input ostili senza panic.
#[doc(hidden)]
pub fn __fuzz_read_kml(bytes: &[u8]) -> Result<usize> {
    let text = std::str::from_utf8(bytes).map_err(|error| err(format!("UTF-8 KML: {error}")))?;
    validate_kml_xml(bytes, bytes.len())?;
    let document: Kml = text
        .parse()
        .map_err(|error| err(format!("KML non valido: {error}")))?;
    let mut placemarks = Vec::new();
    let cancellation = CancellationToken::new();
    let mut visited = 0;
    collect(&document, &mut placemarks, &cancellation, &mut visited)?;
    let (batch, _) = build_batch(&placemarks)?;
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
    use plenora_io_model::wkb::to_wkb;

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
            br"<kml><Placemark><Point><coordinates> </coordinates></Point></Placemark></kml>"
        )
        .is_err());
        assert!(__fuzz_read_kml(
            br"<kml><Placemark><Point><coordinates><![CDATA[ ]]></coordinates></Point></Placemark></kml>"
        )
        .is_err());
    }

    fn event_parser_error(xml: &str) -> PlenoraIoError {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("unsupported.kml");
        std::fs::write(&path, xml).unwrap();
        let mut stream = PlacemarkStream::open(&path).unwrap();
        stream
            .next_placemark(&CancellationToken::new(), 0)
            .unwrap_err()
    }

    #[test]
    fn event_parser_rejects_model_track_and_multitrack() {
        for geometry in [
            "<Model><Location/></Model>",
            "<gx:Track><when>2026-01-01T00:00:00Z</when></gx:Track>",
            "<gx:MultiTrack><gx:Track/></gx:MultiTrack>",
        ] {
            let xml = format!(
                r#"<kml xmlns="http://www.opengis.net/kml/2.2" xmlns:gx="http://www.google.com/kml/ext/2.2"><Placemark>{geometry}</Placemark></kml>"#
            );
            let error = event_parser_error(&xml);
            assert_eq!(error.category, plenora_io_model::ErrorCategory::DataMapping);
            assert!(error.message.contains("geometria KML non supportata"));
            let diagnostics = error.row_diagnostics.as_deref().unwrap();
            assert_eq!(diagnostics.examples[0].source_index, 0);
            assert_eq!(diagnostics.counts["kml.invalid_placemark"], 1);
            assert!(diagnostics.validate().is_ok());
        }
    }

    #[test]
    fn event_parser_rejects_multiple_top_level_geometries() {
        let error = event_parser_error(
            r#"<kml xmlns="http://www.opengis.net/kml/2.2"><Placemark><Point><coordinates>1,2</coordinates></Point><Point><coordinates>3,4</coordinates></Point></Placemark></kml>"#,
        );

        assert_eq!(error.category, plenora_io_model::ErrorCategory::DataMapping);
        assert!(error.message.contains("piu geometrie top-level"));
        let diagnostics = error.row_diagnostics.as_deref().unwrap();
        assert_eq!(diagnostics.examples[0].source_index, 0);
        assert_eq!(diagnostics.counts["kml.invalid_placemark"], 1);
        assert!(diagnostics.validate().is_ok());
    }

    #[test]
    fn reads_kml_placemarks() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("in.kml");
        std::fs::write(&path, SAMPLE).unwrap();
        let driver = KmlDriver;
        let ds = driver.open(Source::Path(path), opzioni_lettura()).unwrap();
        assert_eq!(
            ds.layers()[0]
                .contract
                .geometry
                .as_ref()
                .unwrap()
                .resolved_crs()
                .unwrap()
                .axis_order,
            plenora_io_model::crs::AxisOrder::LongitudeLatitude
        );
        let mut r = ds
            .open_layer_reader(&ReadRequest {
                layer: LayerId(0),
                projected_fields: None,
                projection_mode: ProjectionMode::BestEffort,
                pruning_predicate: None,
                spatial_pruning_hint: None,
                scope: ReadScope::default(),
                batch_target: BatchTarget {
                    target_bytes: usize::MAX,
                    max_rows: 1,
                },
                cancellation: CancellationToken::default(),
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
    fn event_stream_matches_legacy_document_traversal() {
        let text = r#"<?xml version="1.0" encoding="UTF-8"?>
        <kml:kml xmlns:kml="http://www.opengis.net/kml/2.2">
          <kml:Document>
            <kml:Folder>
              <kml:Placemark id="a"><kml:name>A &amp; B</kml:name>
                <kml:Point><kml:coordinates>12,45</kml:coordinates></kml:Point>
              </kml:Placemark>
            </kml:Folder>
            <Update>
              <kml:Placemark><kml:name>non attraversato</kml:name>
                <kml:Point><kml:coordinates>0,0</kml:coordinates></kml:Point>
              </kml:Placemark>
            </Update>
            <kml:Placemark><kml:description><![CDATA[testo <grezzo>]]></kml:description>
              <kml:LineString><kml:coordinates>0,0,0 1,1,0</kml:coordinates></kml:LineString>
            </kml:Placemark>
          </kml:Document>
        </kml:kml>"#;
        let document: Kml = text.parse().unwrap();
        let mut legacy_placemarks = Vec::new();
        collect(
            &document,
            &mut legacy_placemarks,
            &CancellationToken::new(),
            &mut 0,
        )
        .unwrap();
        let (legacy, _) = build_batch(&legacy_placemarks).unwrap();

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("semantic-equivalence.kml");
        std::fs::write(&path, text).unwrap();
        let dataset = KmlDriver
            .open(Source::Path(path), opzioni_lettura())
            .unwrap();
        let mut reader = dataset
            .open_layer_reader(&ReadRequest {
                layer: LayerId(0),
                projected_fields: None,
                projection_mode: ProjectionMode::BestEffort,
                pruning_predicate: None,
                spatial_pruning_hint: None,
                scope: ReadScope::default(),
                batch_target: BatchTarget::default(),
                cancellation: CancellationToken::new(),
            })
            .unwrap();
        let streamed = reader.next_batch().unwrap().unwrap();
        assert!(reader.next_batch().unwrap().is_none());
        assert_eq!(streamed.num_rows(), 2);
        assert_eq!(streamed.schema(), legacy.schema());
        for index in 0..streamed.num_columns() {
            assert_eq!(
                streamed.column(index).to_data(),
                legacy.column(index).to_data()
            );
        }
        assert_eq!(DESCRIPTOR.read_mode(), ReadMode::StreamingSequential);
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
            Field::new("population", DataType::Int64, true),
        ]));
        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![
                Arc::new(BinaryArray::from(vec![Some(wkb.as_slice())])),
                Arc::new(StringArray::from(vec!["Roma"])),
                Arc::new(StringArray::from(vec!["capitale"])),
                Arc::new(arrow_array::Int64Array::from(vec![2_800_000])),
            ],
        )
        .unwrap();

        let driver = KmlDriver;
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
            .create(Sink::Path(out.clone()), &plan, &opzioni_scrittura())
            .unwrap();
        w.write(&batch).unwrap();
        let published = w.finish().unwrap();
        assert_eq!(
            published.loss.counts.get("coercion tipo attributo"),
            Some(&1)
        );
        assert_eq!(
            published.fidelity.level,
            plenora_io_core::Fidelity::Approximating
        );

        let ds = driver.open(Source::Path(out), opzioni_lettura()).unwrap();
        let mut r = ds
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
        geometry_contract.set_exact_geometry_types(vec![GeometryType::Point]);
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
            .create(Sink::Path(out.clone()), &plan, &opzioni_scrittura())
            .unwrap();
        writer.write(&batch).unwrap();
        writer.finish().unwrap();
        assert!(std::fs::read_to_string(&out)
            .unwrap()
            .contains("12.5,45.9,123.25"));

        let dataset = driver.open(Source::Path(out), opzioni_lettura()).unwrap();
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
                scope: ReadScope::default(),
                batch_target: BatchTarget::default(),
                cancellation: CancellationToken::default(),
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
            .create(Sink::Path(out.clone()), &plan, &opzioni_scrittura())
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

    #[test]
    fn writer_adapter_attributes_kml_specific_rejection_and_prevents_publish() {
        let directory = tempfile::tempdir().unwrap();
        let output = directory.path().join("ambiguous.kml");
        let contract = kml_contract(
            &BTreeSet::from([CoordinateDimensions::Xy]),
            BTreeSet::from([GeometryType::GeometryCollection]),
        );
        let point = |x| WkbGeometry {
            value: WkbValue::Point(WkbCoordinate {
                x,
                y: 1.0,
                z: None,
                m: None,
            }),
            dimensions: CoordinateDimensions::Xy,
            srid: None,
        };
        let bytes = encode_wkb(
            &WkbGeometry {
                value: WkbValue::GeometryCollection(vec![point(1.0), point(2.0)]),
                dimensions: CoordinateDimensions::Xy,
                srid: None,
            },
            WkbFlavor::Iso,
        )
        .unwrap();
        let batch = RecordBatch::try_new(
            contract.schema.clone(),
            vec![
                Arc::new(BinaryArray::from(vec![Some(bytes.as_slice())])),
                Arc::new(StringArray::from(vec![Some("name")])),
                Arc::new(StringArray::from(vec![None::<&str>])),
            ],
        )
        .unwrap();
        let plan = WritePlan {
            layers: vec![plenora_io_core::WriteLayer {
                name: "ambiguous".to_owned(),
                contract,
            }],
        };
        let mut writer = KmlDriver
            .create(Sink::Path(output.clone()), &plan, &opzioni_scrittura())
            .unwrap();
        writer.declare_input_total(LayerId(0), 1).unwrap();

        let error = writer.write(&batch).unwrap_err();
        let diagnostics = error.row_diagnostics.as_deref().unwrap();
        assert_eq!(diagnostics.examples[0].source_index, 0);
        assert_eq!(diagnostics.counts["kml.geometry_not_representable"], 1);
        assert!(diagnostics.validate().is_ok());
        assert!(writer.finish().is_err());
        assert!(!output.exists());
    }

    #[test]
    fn empty_geometry_does_not_invent_xy_dimensions() {
        assert!(dimensions_for_kml_coords(&[]).is_err());
        let empty = WkbGeometry {
            value: WkbValue::GeometryCollection(vec![]),
            dimensions: CoordinateDimensions::Xy,
            srid: None,
        };
        assert!(kml_geometry_from_wkb(&empty).is_err());
    }
}
