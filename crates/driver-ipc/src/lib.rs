//! driver-ipc — Arrow IPC (`.arrow`) ⇄ `RecordBatch`. **Pass-through nativo**:
//! l'IPC È già Arrow, quindi schema (inclusi i metadati `geoarrow.wkb` + `crs`) e
//! buffer passano SENZA conversione — Lossless, zero decode/encode WKB, streaming
//! reale (il `FileReader` è un iteratore pull, nessun thread). È il formato di
//! interscambio canonico fra plenora-IO-tools e plenora-data-tools.
#![forbid(unsafe_code)]

use std::fs::File;
use std::io::{BufWriter, Write as _};
use std::path::PathBuf;
use std::sync::Arc;

use arrow_array::RecordBatch;
use arrow_ipc::reader::FileReader;
use arrow_ipc::writer::FileWriter;
use arrow_schema::{Schema, SchemaRef};

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
    validate_write, with_write_validation, AttributeWriteSupport, CrsRepresentationCapabilities,
    CrsRepresentationState, CrsWriteSupport, FormatWriteCapabilities, NullabilitySupport,
    TypeCoercionPolicy, WritePlan, ALL_ARROW_TYPES, UTF8_FIELD_NAMES,
    WKB_EWKB_PASSTHROUGH_GEOMETRY,
};
use plenora_io_model::contract::{
    DataContract, FieldId, GeometryColumnContract, LayerContract, LayerId,
};
#[cfg(test)]
use plenora_io_model::crs::CrsKind;
use plenora_io_model::crs::{crs_kind_for_authority_id, CrsResolution, ResolvedCrs};
use plenora_io_model::geometry::{
    is_geometry_field, read_geometry_contract_metadata, validate_contract_version,
    validate_geometry_field_identity, with_contract_version, with_geometry_contract_metadata,
    GEO_CRS_KEY, PLENORA_CONTRACT_VERSION_KEY,
};
use plenora_io_model::{PlenoraIoError, Result};

fn err(reason: impl Into<String>) -> PlenoraIoError {
    PlenoraIoError::format("ipc", reason)
}

static DESCRIPTOR: FormatDescriptor = FormatDescriptor::const_new(
    "ipc",
    Direction::Bidirectional,
    ReadMode::StreamingSequential,
    // INV-7: footer IPC con gli offset dei blocchi.
    plenora_io_core::NativeReadMode::StreamingRandom,
    // Il drenaggio e lo spool sono dell'adapter comune, non di
    // questo driver: `BudgetedReader` li impone a tutti.
    plenora_io_core::DeliverySemantics::OperationAtomic,
    plenora_io_core::BufferingStrategy::AdaptiveMemoryThenDisk,
    plenora_io_core::DeterminismLevel::Semantic,
    Some(WriteMode::Streaming),
    Some(plenora_io_core::DeterminismLevel::Semantic),
    false,
    false,
    ReaderConcurrency::MultipleIndependentReaders,
    plenora_io_core::ProjectionSupport::Exact,
    plenora_io_core::PredicatePruningSupport::None,
    plenora_io_core::SpatialPruningSupport::None,
    CrsHandling::Embedded, // il CRS viaggia nei metadati del campo
    Fidelity::Lossless,
    Runtime::PureRust,
    Some(FormatWriteCapabilities {
        field_names: UTF8_FIELD_NAMES,
        allowed_types: ALL_ARROW_TYPES,
        type_coercion: TypeCoercionPolicy::Reject,
        attributes: AttributeWriteSupport::All,
        geometry: WKB_EWKB_PASSTHROUGH_GEOMETRY,
        crs: CrsWriteSupport::EmbeddedOptional,
        crs_representations: CrsRepresentationCapabilities::new(
            CrsRepresentationState::Preserved,
            CrsRepresentationState::Preserved,
            CrsRepresentationState::Preserved,
        ),
        nullability: NullabilitySupport::Preserve,
        multi_layer: false,
    }),
    // Il driver non interpreta alcuna format_option (L0.7): l'elenco vuoto
    // e' l'affermazione che qualunque chiave e' sconosciuta, non un'omissione.
    plenora_io_model::format_options::SchemaOpzioniFormato::VUOTO,
    1,
    3,
    9,
);

pub struct IpcDriver;

impl FormatDriver for IpcDriver {
    fn descriptor(&self) -> &FormatDescriptor {
        &DESCRIPTOR
    }

    fn open(&self, source: Source, mut opts: ReadOptions) -> Result<Box<dyn OpenDatasetHandle>> {
        let path = plenora_io_core::preflight_source(self.descriptor(), source, &mut opts)?;
        // FZ-0: schema e buffer dichiarati vengono verificati **prima** che
        // arrow li converta. `try_new` restituisce `Result`, ma la conversione
        // dello schema e l'affettamento del corpo sono infallibili nel tipo e
        // panicano sull'input non conforme; la barriera `leggendo_arrow` sotto
        // resta come difesa in profondita', non come mitigazione.
        driver_common::prevalida_arrow::valida_file_ipc("arrow", &path)?;
        let reader = plenora_io_core::driver::leggendo_arrow("arrow", || {
            FileReader::try_new(File::open(&path)?, None)
                .map_err(|e| err(format!("Arrow IPC non valido: {e}")))
        })?;
        let schema = reader.schema();
        validate_contract_version(schema.as_ref())?;
        let canonical_version_present =
            schema.metadata().contains_key(PLENORA_CONTRACT_VERSION_KEY);
        let mut geometry_fields = schema
            .fields()
            .iter()
            .enumerate()
            .filter(|(_, field)| is_geometry_field(field));
        let geometry = match geometry_fields.next() {
            None => None,
            Some((i, field)) => {
                validate_geometry_field_identity(field, canonical_version_present)?;
                if geometry_fields.next().is_some() {
                    return Err(PlenoraIoError::Contract(
                        "Arrow IPC contiene più colonne GeoArrow nel contratto v1".to_owned(),
                    ));
                }
                let f = schema.field(i);
                let crs =
                    f.metadata()
                        .get(GEO_CRS_KEY)
                        .cloned()
                        .map_or(CrsResolution::Missing, |id| {
                            let kind = crs_kind_for_authority_id(&id);
                            CrsResolution::resolved(ResolvedCrs::new(Some(id), kind, None))
                        });
                // Indice di colonna di uno schema Arrow: limitato a poche
                // migliaia di campi, il cast a u32 non puo' troncare.
                #[allow(clippy::cast_possible_truncation)]
                let physical_field_id = FieldId(i as u32);
                let mut contract = GeometryColumnContract::wkb_passthrough(
                    physical_field_id,
                    f.name(),
                    crs,
                    f.is_nullable(),
                );
                read_geometry_contract_metadata(f, &mut contract)?;
                // Finding #1 review 2026-08-15: `read_geometry_contract_metadata`
                // sovrascrive `field_id` con il valore letto dai metadati
                // (`plenora.field_id`). Un file `.arrow` ostile puo'
                // dichiarare un indice fuori range che a valle produrrebbe
                // `batch.column(index)` panic. Il metadato per convenzione
                // deve coincidere con la posizione fisica del campo
                // geometrico nello schema: divergenze indicano un file
                // corrotto o crafted, e vengono rifiutate come contratto
                // invece di essere accettate silenziosamente.
                if contract.field_id != physical_field_id {
                    return Err(PlenoraIoError::Contract(format!(
                        "Arrow IPC: plenora.field_id={} non coincide con l'indice fisico {} del campo geometria",
                        contract.field_id.0, physical_field_id.0
                    )));
                }
                Some(contract)
            }
        };
        let name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("layer")
            .to_owned();
        Ok(plenora_io_core::with_read_budget(
            Box::new(IpcDataset {
                path,
                layers: vec![LayerContract {
                    id: LayerId(0),
                    name,
                    contract: DataContract::new(schema, geometry),
                }],
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
            .is_some_and(|e| e.eq_ignore_ascii_case("arrow"))
        {
            return Err(PlenoraIoError::Unsupported(
                "l'output deve avere estensione .arrow".to_owned(),
            ));
        }
        if plan.layers.len() != 1 {
            return Err(PlenoraIoError::Unsupported(
                "Arrow IPC: un solo layer per file".to_owned(),
            ));
        }
        let layer = &plan.layers[0].contract;
        let fields = layer
            .schema
            .fields()
            .iter()
            .map(|field| {
                layer
                    .geometry
                    .as_ref()
                    .filter(|geometry| geometry.name.as_str() == field.name().as_str())
                    .map_or_else(
                        || field.as_ref().clone(),
                        |geometry| with_geometry_contract_metadata(field, geometry),
                    )
            })
            .collect::<Vec<_>>();
        let schema = with_contract_version(Arc::new(arrow_schema::Schema::new_with_metadata(
            fields,
            layer.schema.metadata().clone(),
        )));
        let staging = StagedFile::new(&path, opts.durable, opts.max_output_bytes())?;
        let writer = FileWriter::try_new(BufWriter::new(staging.reopen()?), &schema)
            .map_err(|e| err(format!("writer IPC: {e}")))?;
        with_write_validation(
            Box::new(IpcWriter {
                staging,
                writer: Some(writer),
                schema,
            }),
            self.descriptor(),
            plan,
            opts,
        )
    }
}

struct IpcDataset {
    path: PathBuf,
    layers: Vec<LayerContract>,
}

impl OpenDatasetHandle for IpcDataset {
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
        if request.layer != self.layers[0].id {
            return Err(err(format!("layer {} inesistente", request.layer.0)));
        }

        let source_layer = &self.layers[0];
        let (projection, layer) = match &request.projected_fields {
            None => (None, source_layer.clone()),
            Some(field_ids) => {
                let mut indices = Vec::new();
                for field_id in field_ids {
                    let index = field_id.0 as usize;
                    if index >= source_layer.contract.schema.fields().len() {
                        if request.projection_mode == plenora_io_core::ProjectionMode::Required {
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
                indices.sort_unstable();
                let fields = indices
                    .iter()
                    .map(|&index| source_layer.contract.schema.field(index).as_ref().clone())
                    .collect::<Vec<_>>();
                let schema = Arc::new(Schema::new_with_metadata(
                    fields,
                    source_layer.contract.schema.metadata().clone(),
                ));
                let geometry = source_layer.contract.geometry.clone().and_then(|geometry| {
                    schema.index_of(&geometry.name).ok().map(|index| {
                        // Indice di colonna di uno schema Arrow: il cast a
                        // u32 non puo' troncare.
                        #[allow(clippy::cast_possible_truncation)]
                        let field_id = FieldId(index as u32);
                        GeometryColumnContract {
                            field_id,
                            ..geometry
                        }
                    })
                });
                (
                    Some(indices),
                    LayerContract {
                        id: source_layer.id,
                        name: source_layer.name.clone(),
                        contract: DataContract::new(schema, geometry),
                    },
                )
            }
        };
        let path = self.path.clone();
        // Il file viene riaperto qui, quindi viene riverificato qui: fra
        // `open` e questa chiamata il contenuto su disco puo' essere cambiato,
        // e una verifica fatta una volta sola varrebbe per un file che non e'
        // piu' quello.
        driver_common::prevalida_arrow::valida_file_ipc("arrow", &path)?;
        let reader = plenora_io_core::driver::leggendo_arrow("arrow", move || {
            FileReader::try_new(File::open(&path)?, projection)
                .map_err(|e| err(format!("Arrow IPC non valido: {e}")))
        })?;
        Ok(plenora_io_core::with_batch_target(
            Box::new(IpcReader { reader, layer }),
            request.batch_target,
            request.cancellation.clone(),
        ))
    }
}

struct IpcReader {
    reader: FileReader<File>,
    layer: LayerContract,
}

impl LayerReader for IpcReader {
    fn contract(&self) -> &LayerContract {
        &self.layer
    }
    fn next_batch(&mut self) -> Result<Option<RecordBatch>> {
        // Anche qui serve la barriera, non solo sullo schema: arrow decodifica
        // i buffer del batch a ogni `next()`, e un offset oltre la lunghezza
        // dichiarata panica in `arrow-buffer` invece di restituire un errore.
        //
        // Dopo un panico catturato il `FileReader` resta in uno stato non
        // definito. Non e' un problema: il chiamante riceve un errore e il
        // contratto di `LayerReader` non prevede di proseguire dopo un errore.
        let reader = &mut self.reader;
        plenora_io_core::driver::leggendo_arrow("arrow", move || match reader.next() {
            None => Ok(None),
            Some(Ok(b)) => Ok(Some(b)),
            Some(Err(e)) => Err(err(format!("batch IPC: {e}"))),
        })
    }
}

struct IpcWriter {
    staging: StagedFile,
    writer: Option<FileWriter<BufWriter<File>>>,
    schema: SchemaRef,
}

impl FormatWriter for IpcWriter {
    fn write(&mut self, batch: &RecordBatch) -> Result<()> {
        let batch = RecordBatch::try_new(self.schema.clone(), batch.columns().to_vec())
            .map_err(|e| err(format!("retag contratto IPC: {e}")))?;
        self.writer
            .as_mut()
            .ok_or_else(|| err("writer chiuso"))?
            .write(&batch)
            .map_err(|e| err(format!("write IPC: {e}")))
    }

    fn finish(mut self: Box<Self>) -> Result<Published> {
        let mut w = self.writer.take().ok_or_else(|| err("writer già chiuso"))?;
        w.finish().map_err(|e| err(format!("finish IPC: {e}")))?;
        let mut inner = w
            .into_inner()
            .map_err(|e| err(format!("into_inner: {e}")))?;
        inner.flush()?;
        drop(inner);
        let (bytes, outcome) = self.staging.publish()?;
        Ok(Published {
            bytes,
            loss: LossReport::default(),
            fidelity: plenora_io_core::FidelityAssessment::lossless(),
            outcome,
        })
    }
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

    use std::sync::Arc;

    use arrow_array::{BinaryArray, Int64Array};
    use arrow_schema::{DataType, Field, Schema, SchemaRef};
    use plenora_io_core::request::{BatchTarget, ProjectionMode, ReadScope};
    use plenora_io_core::WriteLayer;
    use plenora_io_model::contract::{
        CoordinateDimensions, CoordinatePrecision, GeometryEncoding, GeometryType, SpatialSemantics,
    };
    use plenora_io_model::wkb::{
        encode_wkb, to_wkb, WkbCoordinate, WkbFlavor, WkbGeometry, WkbValue,
    };
    use plenora_io_model::CancellationToken;

    /// Il file che faceva panicare arrow viene ora **rifiutato prima** che
    /// arrow lo tocchi (FZ-0).
    ///
    /// Prima di FZ-0 questo test osservava la barriera `catch_unwind`: il
    /// panico avveniva e veniva convertito. Non bastava — un panico catturato
    /// e' pur sempre un panico, e sotto `libfuzzer-sys` diventa `abort()`
    /// prima dell'unwinding, quindi il target restava rosso e in quarantena.
    ///
    /// Ora il difetto e' impedito: `valida_file_ipc` verifica schema e buffer
    /// dichiarati contro il corpo del messaggio, e questo file dichiara un
    /// buffer oltre la fine del proprio corpo. La verifica e' che l'errore
    /// **non** venga dalla barriera.
    #[test]
    fn un_ipc_non_conforme_e_rifiutato_prima_di_arrow() {
        let seme = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fuzz/seeds/ipc_reader/schema-che-fa-panicare-arrow.arrow");

        let errore = match IpcDriver.open(Source::Path(seme), opzioni_lettura()) {
            Err(errore) => errore,
            Ok(dataset) => {
                let request = ReadRequest {
                    layer: LayerId(0),
                    projected_fields: None,
                    projection_mode: ProjectionMode::BestEffort,
                    pruning_predicate: None,
                    spatial_pruning_hint: None,
                    scope: ReadScope::default(),
                    batch_target: BatchTarget::default(),
                    cancellation: CancellationToken::default(),
                };
                match dataset.open_layer_reader(&request) {
                    Err(errore) => errore,
                    Ok(mut reader) => loop {
                        match reader.next_batch() {
                            Ok(Some(_)) => {}
                            Ok(None) => panic!("il file doveva essere rifiutato"),
                            Err(errore) => break errore,
                        }
                    },
                }
            }
        };
        assert!(
            !errore.to_string().contains("in panico"),
            "il rifiuto deve precedere arrow, non seguirne il panico: {errore}"
        );
        assert_eq!(errore.phase, plenora_io_model::ErrorPhase::Read);
        assert_eq!(errore.code, plenora_io_model::IoErrorCode::Format);
    }

    /// Un IPC conforme continua a essere letto: la prevalidazione non rifiuta
    /// cio' che il formato ammette.
    ///
    /// Senza questo, una verifica troppo severa passerebbe il test sopra e
    /// romperebbe ogni file reale senza che nessuno se ne accorgesse qui.
    #[test]
    fn un_ipc_conforme_supera_la_prevalidazione() {
        let seme = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fuzz/seeds/ipc_reader/minimal.arrow");
        assert!(seme.is_file(), "seme assente: {}", seme.display());
        driver_common::prevalida_arrow::valida_file_ipc("arrow", &seme)
            .expect("un IPC conforme non deve essere rifiutato");
    }

    #[test]
    fn geometry_without_crs_metadata_is_explicitly_missing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("missing-crs.arrow");
        let field = Field::new("geometry", DataType::Binary, true).with_metadata(
            std::iter::once((
                plenora_io_model::geometry::ARROW_EXTENSION_NAME_KEY.to_owned(),
                plenora_io_model::geometry::GEOARROW_WKB_EXTENSION.to_owned(),
            ))
            .collect(),
        );
        let schema = with_contract_version(Arc::new(Schema::new(vec![field])));
        {
            let file = File::create(&path).unwrap();
            let mut writer = FileWriter::try_new(file, schema.as_ref()).unwrap();
            writer.finish().unwrap();
        }

        let dataset = IpcDriver
            .open(Source::Path(path), opzioni_lettura())
            .unwrap();
        assert!(matches!(
            &dataset.layers()[0].contract.geometry.as_ref().unwrap().crs,
            CrsResolution::Missing
        ));
    }

    #[test]
    fn unresolved_authority_without_definition_is_preserved() {
        use plenora_io_model::geometry::{
            ARROW_EXTENSION_NAME_KEY, GEOARROW_WKB_EXTENSION, PLENORA_AXIS_ORDER_KEY,
            PLENORA_CRS_DEFINITION_FORMAT_KEY, PLENORA_CRS_DEFINITION_KEY, PLENORA_CRS_ID_KEY,
            PLENORA_CRS_RESOLUTION_KEY, PLENORA_DIMENSIONS_KEY, PLENORA_ENCODING_KEY,
            PLENORA_TYPES_DECLARATION_KEY,
        };

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("unresolved-authority.arrow");
        let field = Field::new("geometry", DataType::Binary, true).with_metadata(
            [
                (
                    ARROW_EXTENSION_NAME_KEY.to_owned(),
                    GEOARROW_WKB_EXTENSION.to_owned(),
                ),
                (
                    PLENORA_CRS_RESOLUTION_KEY.to_owned(),
                    "declared_unresolved".to_owned(),
                ),
                (PLENORA_ENCODING_KEY.to_owned(), "wkb".to_owned()),
                (PLENORA_DIMENSIONS_KEY.to_owned(), "xy".to_owned()),
                (
                    PLENORA_TYPES_DECLARATION_KEY.to_owned(),
                    "unresolved".to_owned(),
                ),
                (PLENORA_CRS_ID_KEY.to_owned(), "EPSG:99999".to_owned()),
                (PLENORA_AXIS_ORDER_KEY.to_owned(), "unknown".to_owned()),
            ]
            .into_iter()
            .collect(),
        );
        let schema = with_contract_version(Arc::new(Schema::new(vec![field])));
        {
            let file = File::create(&path).unwrap();
            let mut writer = FileWriter::try_new(file, schema.as_ref()).unwrap();
            writer.finish().unwrap();
        }

        let dataset = IpcDriver
            .open(Source::Path(path), opzioni_lettura())
            .unwrap();
        let geometry = dataset.layers()[0].contract.geometry.as_ref().unwrap();
        let raw = geometry.crs.raw().unwrap();
        assert_eq!(raw.authority_hint.as_deref(), Some("EPSG:99999"));
        assert_eq!(raw.definition, None);
        assert_eq!(raw.definition_format, None);

        let emitted = with_geometry_contract_metadata(
            &Field::new("geometry", DataType::Binary, true),
            geometry,
        );
        assert!(!emitted.metadata().contains_key(PLENORA_CRS_DEFINITION_KEY));
        assert!(!emitted
            .metadata()
            .contains_key(PLENORA_CRS_DEFINITION_FORMAT_KEY));
    }

    #[test]
    fn round_trip_preserves_declared_unresolved_srid_only_without_synthesis() {
        use plenora_io_model::geometry::{
            ARROW_EXTENSION_NAME_KEY, GEOARROW_WKB_EXTENSION, PLENORA_AXIS_ORDER_KEY,
            PLENORA_CRS_DEFINITION_KEY, PLENORA_CRS_ID_KEY, PLENORA_CRS_RESOLUTION_KEY,
            PLENORA_DIMENSIONS_KEY, PLENORA_ENCODING_KEY, PLENORA_GEOMETRY_TYPES_KEY,
            PLENORA_SRID_KEY, PLENORA_TYPES_DECLARATION_KEY,
        };

        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("srid-only-input.arrow");
        let output = dir.path().join("srid-only-output.arrow");
        let field = Field::new("geom", DataType::Binary, true).with_metadata(
            [
                (
                    ARROW_EXTENSION_NAME_KEY.to_owned(),
                    GEOARROW_WKB_EXTENSION.to_owned(),
                ),
                (PLENORA_ENCODING_KEY.to_owned(), "wkb".to_owned()),
                (PLENORA_DIMENSIONS_KEY.to_owned(), "xy".to_owned()),
                (PLENORA_TYPES_DECLARATION_KEY.to_owned(), "exact".to_owned()),
                (PLENORA_GEOMETRY_TYPES_KEY.to_owned(), "point".to_owned()),
                (
                    PLENORA_CRS_RESOLUTION_KEY.to_owned(),
                    "declared_unresolved".to_owned(),
                ),
                (PLENORA_SRID_KEY.to_owned(), "4326".to_owned()),
            ]
            .into_iter()
            .collect(),
        );
        let schema = with_contract_version(Arc::new(Schema::new(vec![field])));
        {
            let values = BinaryArray::from(vec![Some(
                &[
                    1_u8, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
                ][..],
            )]);
            let batch = RecordBatch::try_new(schema.clone(), vec![Arc::new(values)]).unwrap();
            let mut writer = FileWriter::try_new(File::create(&input).unwrap(), &schema).unwrap();
            writer.write(&batch).unwrap();
            writer.finish().unwrap();
        }

        let driver = IpcDriver;
        let dataset = driver.open(Source::Path(input), opzioni_lettura()).unwrap();
        let layer = dataset.layers()[0].clone();
        let geometry = layer.contract.geometry.as_ref().unwrap();
        assert_eq!(geometry.srid, Some(4326));
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
        let batch = reader.next_batch().unwrap().unwrap();
        assert_eq!(batch.num_rows(), 1);
        assert_eq!(batch.schema(), layer.contract.schema);
        assert!(reader.next_batch().unwrap().is_none());
        let plan = WritePlan {
            layers: vec![WriteLayer {
                name: "layer".to_owned(),
                contract: layer.contract,
            }],
        };
        driver
            .create(Sink::Path(output.clone()), &plan, &opzioni_scrittura())
            .unwrap()
            .finish()
            .unwrap();

        let output_schema = FileReader::try_new(File::open(output).unwrap(), None)
            .unwrap()
            .schema();
        let metadata = output_schema.field(0).metadata();
        assert_eq!(
            metadata.get(PLENORA_SRID_KEY).map(String::as_str),
            Some("4326")
        );
        for key in [
            PLENORA_CRS_ID_KEY,
            PLENORA_CRS_DEFINITION_KEY,
            PLENORA_AXIS_ORDER_KEY,
        ] {
            assert!(!metadata.contains_key(key), "chiave sintetizzata: {key}");
        }
    }

    #[test]
    fn declared_unresolved_srid_only_with_axis_order_fails_at_open() {
        use plenora_io_model::geometry::{
            ARROW_EXTENSION_NAME_KEY, GEOARROW_WKB_EXTENSION, PLENORA_AXIS_ORDER_KEY,
            PLENORA_CRS_RESOLUTION_KEY, PLENORA_DIMENSIONS_KEY, PLENORA_ENCODING_KEY,
            PLENORA_GEOMETRY_TYPES_KEY, PLENORA_SRID_KEY, PLENORA_TYPES_DECLARATION_KEY,
        };

        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("srid-only-with-axis.arrow");
        let field = Field::new("geom", DataType::Binary, true).with_metadata(
            [
                (
                    ARROW_EXTENSION_NAME_KEY.to_owned(),
                    GEOARROW_WKB_EXTENSION.to_owned(),
                ),
                (PLENORA_ENCODING_KEY.to_owned(), "wkb".to_owned()),
                (PLENORA_DIMENSIONS_KEY.to_owned(), "xy".to_owned()),
                (PLENORA_TYPES_DECLARATION_KEY.to_owned(), "exact".to_owned()),
                (PLENORA_GEOMETRY_TYPES_KEY.to_owned(), "point".to_owned()),
                (
                    PLENORA_CRS_RESOLUTION_KEY.to_owned(),
                    "declared_unresolved".to_owned(),
                ),
                (PLENORA_SRID_KEY.to_owned(), "4326".to_owned()),
                (PLENORA_AXIS_ORDER_KEY.to_owned(), "lon_lat".to_owned()),
            ]
            .into_iter()
            .collect(),
        );
        let schema = with_contract_version(Arc::new(Schema::new(vec![field])));
        let values = BinaryArray::from(vec![Some(
            &[
                1_u8, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            ][..],
        )]);
        let batch = RecordBatch::try_new(schema.clone(), vec![Arc::new(values)]).unwrap();
        let mut writer = FileWriter::try_new(File::create(&input).unwrap(), &schema).unwrap();
        writer.write(&batch).unwrap();
        writer.finish().unwrap();

        assert!(IpcDriver
            .open(Source::Path(input), opzioni_lettura())
            .is_err());
    }

    #[test]
    fn canonical_metadata_without_geoarrow_extension_is_geometry() {
        use plenora_io_model::geometry::{
            PLENORA_AXIS_ORDER_KEY, PLENORA_CRS_ID_KEY, PLENORA_CRS_RESOLUTION_KEY,
            PLENORA_DIMENSIONS_KEY, PLENORA_ENCODING_KEY, PLENORA_TYPES_DECLARATION_KEY,
        };

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("canonical-only.arrow");
        let field = Field::new("geometry", DataType::Binary, true).with_metadata(
            [
                (PLENORA_ENCODING_KEY.to_owned(), "wkb".to_owned()),
                (PLENORA_DIMENSIONS_KEY.to_owned(), "xy".to_owned()),
                (PLENORA_CRS_RESOLUTION_KEY.to_owned(), "resolved".to_owned()),
                (PLENORA_CRS_ID_KEY.to_owned(), "EPSG:4326".to_owned()),
                (PLENORA_AXIS_ORDER_KEY.to_owned(), "lat_lon".to_owned()),
                (PLENORA_TYPES_DECLARATION_KEY.to_owned(), "mixed".to_owned()),
            ]
            .into_iter()
            .collect(),
        );
        let schema = with_contract_version(Arc::new(Schema::new(vec![field])));
        {
            let file = File::create(&path).unwrap();
            let mut writer = FileWriter::try_new(file, schema.as_ref()).unwrap();
            writer.finish().unwrap();
        }

        let dataset = IpcDriver
            .open(Source::Path(path), opzioni_lettura())
            .unwrap();
        let geometry = dataset.layers()[0].contract.geometry.as_ref().unwrap();
        assert_eq!(geometry.name, "geometry");
        assert_eq!(geometry.crs.id(), Some("EPSG:4326"));
    }

    #[test]
    fn incomplete_or_conflicting_canonical_identity_is_rejected() {
        use plenora_io_model::geometry::{
            ARROW_EXTENSION_NAME_KEY, PLENORA_CRS_RESOLUTION_KEY, PLENORA_DIMENSIONS_KEY,
            PLENORA_ENCODING_KEY, PLENORA_TYPES_DECLARATION_KEY,
        };

        for (name, metadata) in [
            (
                "missing-version",
                [
                    (PLENORA_ENCODING_KEY.to_owned(), "wkb".to_owned()),
                    (PLENORA_DIMENSIONS_KEY.to_owned(), "xy".to_owned()),
                    (PLENORA_CRS_RESOLUTION_KEY.to_owned(), "missing".to_owned()),
                    (PLENORA_TYPES_DECLARATION_KEY.to_owned(), "mixed".to_owned()),
                ]
                .into_iter()
                .collect(),
            ),
            (
                "conflicting-extension",
                [
                    (PLENORA_ENCODING_KEY.to_owned(), "wkb".to_owned()),
                    (PLENORA_DIMENSIONS_KEY.to_owned(), "xy".to_owned()),
                    (PLENORA_CRS_RESOLUTION_KEY.to_owned(), "missing".to_owned()),
                    (PLENORA_TYPES_DECLARATION_KEY.to_owned(), "mixed".to_owned()),
                    (
                        ARROW_EXTENSION_NAME_KEY.to_owned(),
                        "vendor.opaque".to_owned(),
                    ),
                ]
                .into_iter()
                .collect(),
            ),
        ] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join(format!("{name}.arrow"));
            let field = Field::new("geometry", DataType::Binary, true).with_metadata(metadata);
            let schema = if name == "missing-version" {
                Arc::new(Schema::new(vec![field]))
            } else {
                with_contract_version(Arc::new(Schema::new(vec![field])))
            };
            {
                let file = File::create(&path).unwrap();
                let mut writer = FileWriter::try_new(file, schema.as_ref()).unwrap();
                writer.finish().unwrap();
            }
            assert!(matches!(
                IpcDriver.open(Source::Path(path), opzioni_lettura()),
                Err(error) if error.code == plenora_io_model::IoErrorCode::Contract
            ));
        }
    }

    #[test]
    fn multiple_geoarrow_fields_are_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ambiguous.arrow");
        let geometry_field = |name| {
            Field::new(name, DataType::Binary, true).with_metadata(
                std::iter::once((
                    plenora_io_model::geometry::ARROW_EXTENSION_NAME_KEY.to_owned(),
                    plenora_io_model::geometry::GEOARROW_WKB_EXTENSION.to_owned(),
                ))
                .collect(),
            )
        };
        let schema = Schema::new(vec![
            geometry_field("geometry_a"),
            geometry_field("geometry_b"),
        ]);
        {
            let file = File::create(&path).unwrap();
            let mut writer = FileWriter::try_new(file, &schema).unwrap();
            writer.finish().unwrap();
        }

        assert!(matches!(
            IpcDriver.open(Source::Path(path), opzioni_lettura()),
            Err(error) if error.code == plenora_io_model::IoErrorCode::Contract
        ));
    }

    #[test]
    fn round_trip_ipc_preserves_geometry_metadata() {
        use driver_common_geometry_field as geometry_field;

        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("t.arrow");
        let wkb = to_wkb(&geo_types::Geometry::Point(geo_types::Point::new(
            12.5, 45.9,
        )))
        .unwrap();
        let schema: SchemaRef = Arc::new(Schema::new(vec![
            geometry_field("geometry", "EPSG:4326"),
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

        let driver = IpcDriver;
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
        w.finish().unwrap();

        let ds = driver.open(Source::Path(out), opzioni_lettura()).unwrap();
        // Il CRS e la geometria sopravvivono nei metadati Arrow (pass-through).
        let g = ds.layers()[0].contract.geometry.as_ref().unwrap();
        assert_eq!(g.name, "geometry");
        assert_eq!(g.crs.id(), Some("EPSG:4326"));
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
        assert!(is_geometry_field(
            &rb.schema().field_with_name("geometry").unwrap().clone()
        ));
        let col = rb
            .column_by_name("geometry")
            .unwrap()
            .as_any()
            .downcast_ref::<BinaryArray>()
            .unwrap();
        assert_eq!(col.value(0), wkb.as_slice());
        assert!(r.next_batch().unwrap().is_none());

        let mut projected = ds
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
        assert_eq!(projected.contract().contract.schema.fields().len(), 1);
        assert_eq!(projected.contract().contract.schema.field(0).name(), "id");
        assert!(projected.contract().contract.geometry.is_none());
        let projected_batch = projected.next_batch().unwrap().unwrap();
        assert_eq!(projected_batch.num_columns(), 1);
        assert_eq!(
            projected_batch
                .column(0)
                .as_any()
                .downcast_ref::<Int64Array>()
                .unwrap()
                .value(0),
            7
        );
    }

    #[test]
    fn batch_target_slices_file_defined_ipc_batches() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("batch-target.arrow");
        let schema: SchemaRef =
            Arc::new(Schema::new(vec![Field::new("id", DataType::Int64, false)]));
        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![Arc::new(Int64Array::from(vec![0, 1, 2, 3, 4]))],
        )
        .unwrap();
        let plan = WritePlan {
            layers: vec![WriteLayer {
                name: "layer".to_owned(),
                contract: DataContract {
                    schema,
                    geometry: None,
                },
            }],
        };
        let driver = IpcDriver;
        let mut writer = driver
            .create(Sink::Path(out.clone()), &plan, &opzioni_scrittura())
            .unwrap();
        writer.write(&batch).unwrap();
        writer.finish().unwrap();

        let dataset = driver.open(Source::Path(out), opzioni_lettura()).unwrap();
        let mut reader = dataset
            .open_layer_reader(&ReadRequest {
                layer: LayerId(0),
                projected_fields: None,
                projection_mode: ProjectionMode::BestEffort,
                pruning_predicate: None,
                spatial_pruning_hint: None,
                scope: ReadScope::default(),
                batch_target: BatchTarget {
                    target_bytes: 16,
                    max_rows: 100,
                },
                cancellation: CancellationToken::default(),
            })
            .unwrap();
        let mut sizes = Vec::new();
        while let Some(batch) = reader.next_batch().unwrap() {
            sizes.push(batch.num_rows());
        }
        assert_eq!(sizes, vec![2, 2, 1]);
    }

    #[test]
    fn round_trip_ipc_preserves_ewkb_zm_contract_and_bytes() {
        use driver_common_geometry_field as geometry_field;

        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("zm.arrow");
        let ewkb = encode_wkb(
            &WkbGeometry {
                value: WkbValue::Point(WkbCoordinate {
                    x: 1.0,
                    y: 2.0,
                    z: Some(3.0),
                    m: Some(4.0),
                }),
                dimensions: CoordinateDimensions::Xyzm,
                srid: Some(4326),
            },
            WkbFlavor::Ewkb,
        )
        .unwrap();
        let schema: SchemaRef =
            Arc::new(Schema::new(vec![geometry_field("geometry", "EPSG:4326")]));
        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![Arc::new(BinaryArray::from(vec![Some(ewkb.as_slice())]))],
        )
        .unwrap();
        let mut geometry = GeometryColumnContract::wkb_passthrough(
            FieldId(0),
            "geometry",
            ResolvedCrs::new(Some("EPSG:4326".to_owned()), CrsKind::Geographic, None),
            true,
        );
        geometry.encoding = GeometryEncoding::Ewkb;
        geometry.dimensions = CoordinateDimensions::Xyzm;
        geometry.spatial_semantics = SpatialSemantics::Geography;
        geometry.srid = Some(4326);
        geometry.precision = CoordinatePrecision::Native;
        geometry.set_exact_geometry_types(vec![GeometryType::Point]);
        geometry.native_metadata.insert(
            "postgis.typmod".to_owned(),
            "geography(PointZM,4326)".to_owned(),
        );
        let plan = WritePlan {
            layers: vec![WriteLayer {
                name: "l".to_owned(),
                contract: DataContract {
                    schema,
                    geometry: Some(geometry),
                },
            }],
        };
        let driver = IpcDriver;
        let mut writer = driver
            .create(Sink::Path(out.clone()), &plan, &opzioni_scrittura())
            .unwrap();
        writer.write(&batch).unwrap();
        writer.finish().unwrap();

        let dataset = driver.open(Source::Path(out), opzioni_lettura()).unwrap();
        let geometry = dataset.layers()[0].contract.geometry.as_ref().unwrap();
        assert_eq!(geometry.encoding, GeometryEncoding::Ewkb);
        assert_eq!(geometry.dimensions, CoordinateDimensions::Xyzm);
        assert_eq!(geometry.spatial_semantics, SpatialSemantics::Geography);
        assert_eq!(geometry.srid, Some(4326));
        assert_eq!(geometry.precision, CoordinatePrecision::Native);
        assert_eq!(geometry.geometry_types, vec![GeometryType::Point]);
        assert_eq!(
            geometry
                .native_metadata
                .get("postgis.typmod")
                .map(String::as_str),
            Some("geography(PointZM,4326)")
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
        let read = reader.next_batch().unwrap().unwrap();
        let geometry_array = read
            .column(0)
            .as_any()
            .downcast_ref::<BinaryArray>()
            .unwrap();
        assert_eq!(geometry_array.value(0), ewkb);
    }

    // Il test confronta uno per uno tutti i metadati canonici pubblicati: la
    // lunghezza è la lista dei metadati, spezzarla nasconderebbe la copertura.
    #[allow(clippy::too_many_lines)]
    #[test]
    fn canonical_metadata_additions_are_published_without_changing_values() {
        use std::collections::HashMap;

        use plenora_io_model::geometry::{
            ARROW_EXTENSION_NAME_KEY, GEOARROW_WKB_EXTENSION, PLENORA_CRS_RESOLUTION_KEY,
            PLENORA_DIMENSIONS_KEY, PLENORA_ENCODING_KEY, PLENORA_FIELD_ID_KEY,
            PLENORA_GEOMETRY_TYPES_KEY, PLENORA_PRECISION_KEY, PLENORA_TYPES_DECLARATION_KEY,
        };

        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("physical.arrow");
        let output = dir.path().join("canonical.arrow");
        let metadata = HashMap::from([
            (
                ARROW_EXTENSION_NAME_KEY.to_owned(),
                GEOARROW_WKB_EXTENSION.to_owned(),
            ),
            (PLENORA_ENCODING_KEY.to_owned(), "wkb".to_owned()),
            (PLENORA_DIMENSIONS_KEY.to_owned(), "xy".to_owned()),
            (PLENORA_CRS_RESOLUTION_KEY.to_owned(), "missing".to_owned()),
            (PLENORA_TYPES_DECLARATION_KEY.to_owned(), "exact".to_owned()),
            (PLENORA_GEOMETRY_TYPES_KEY.to_owned(), "point".to_owned()),
            ("producer.normative".to_owned(), "retained".to_owned()),
        ]);
        assert!(!metadata.contains_key(PLENORA_FIELD_ID_KEY));
        assert!(!metadata.contains_key(PLENORA_PRECISION_KEY));
        let physical_schema = with_contract_version(Arc::new(Schema::new(vec![Field::new(
            "geometry",
            DataType::Binary,
            false,
        )
        .with_metadata(metadata)])));
        let values = [
            geo_types::Point::new(1.0, 2.0),
            geo_types::Point::new(3.0, 4.0),
            geo_types::Point::new(5.0, 6.0),
        ]
        .map(|point| to_wkb(&geo_types::Geometry::Point(point)).unwrap());
        let physical_batch = RecordBatch::try_new(
            physical_schema.clone(),
            vec![Arc::new(BinaryArray::from_iter_values(
                values.iter().map(Vec::as_slice),
            ))],
        )
        .unwrap();
        {
            let file = File::create(&source).unwrap();
            let mut writer = FileWriter::try_new(file, physical_schema.as_ref()).unwrap();
            writer.write(&physical_batch).unwrap();
            writer.finish().unwrap();
        }

        let driver = IpcDriver;
        let dataset = driver
            .open(Source::Path(source), opzioni_lettura())
            .unwrap();
        let layer = dataset.layers()[0].clone();
        let canonical_field = layer.contract.schema.field(0);
        assert_eq!(
            canonical_field
                .metadata()
                .get(PLENORA_FIELD_ID_KEY)
                .map(String::as_str),
            Some("0")
        );
        assert_eq!(
            canonical_field
                .metadata()
                .get(PLENORA_PRECISION_KEY)
                .map(String::as_str),
            Some("float64")
        );
        assert_eq!(
            canonical_field
                .metadata()
                .get("producer.normative")
                .map(String::as_str),
            Some("retained")
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
        let read = reader.next_batch().unwrap().unwrap();
        assert_eq!(read.schema(), layer.contract.schema);

        let plan = WritePlan {
            layers: vec![WriteLayer {
                name: layer.name,
                contract: layer.contract,
            }],
        };
        let mut writer = driver
            .create(Sink::Path(output.clone()), &plan, &opzioni_scrittura())
            .unwrap();
        writer.write(&read).unwrap();
        writer.finish().unwrap();

        let mut published = FileReader::try_new(File::open(output).unwrap(), None).unwrap();
        let published_batch = published.next().unwrap().unwrap();
        assert_eq!(published_batch.num_rows(), 3);
        let published_values = published_batch
            .column(0)
            .as_any()
            .downcast_ref::<BinaryArray>()
            .unwrap();
        for (row, expected) in values.iter().enumerate() {
            assert_eq!(published_values.value(row), expected);
        }
    }

    // geometry_field locale (evita la dipendenza driver-common nei test).
    fn driver_common_geometry_field(name: &str, crs: &str) -> Field {
        use std::collections::HashMap;
        let mut md = HashMap::new();
        md.insert(
            plenora_io_model::geometry::ARROW_EXTENSION_NAME_KEY.to_owned(),
            plenora_io_model::geometry::GEOARROW_WKB_EXTENSION.to_owned(),
        );
        md.insert(GEO_CRS_KEY.to_owned(), crs.to_owned());
        Field::new(name, DataType::Binary, true).with_metadata(md)
    }
}
