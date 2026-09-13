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
    SinkPathConstraint, TypeCoercionPolicy, WritePlan, ALL_ARROW_TYPES, UTF8_FIELD_NAMES,
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
    validate_geometry_field_identity, with_contract_version, with_field_identity,
    with_geometry_contract_metadata, GEO_CRS_KEY, PLENORA_CONTRACT_VERSION_KEY,
};
use plenora_io_model::{NumeroStrutturale, PlenoraIoError, PublicMessage, Result};

fn err(reason: &PublicMessage) -> PlenoraIoError {
    PlenoraIoError::formato_redatto("ipc", reason)
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
    // `hostile_input_hardened`: non dichiarato: l'input e' binario, con prevalidazione dello schema.
    false,
    // `spec_version_supported`: il formato non si versiona in un modo che
    // il driver possa dichiarare per intero.
    None,
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
        sink_path: SinkPathConstraint::Free,
    }),
    // Il driver non interpreta alcuna format_option (L0.7): l'elenco vuoto
    // e' l'affermazione che qualunque chiave e' sconosciuta, non un'omissione.
    plenora_io_model::format_options::SchemaOpzioniFormato::VUOTO,
    &["arrow"],
    1,
    3,
    10,
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
                .map_err(|_| err(&PublicMessage::Curated("Arrow IPC non valido")))
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
                    return Err(PlenoraIoError::contratto_redatto(&PublicMessage::Curated(
                        "Arrow IPC contiene più colonne GeoArrow nel contratto v1",
                    )));
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
                // Il rifiuto che c'era qui non c'e' piu', e la ragione per
                // cui c'era resta valida: `plenora.field_id` veniva dal
                // payload e finiva in `batch.column(index)`, dove un indice
                // fuori range e' un panico. La risposta era pretendere che il
                // metadato coincidesse con la posizione fisica, e rifiutare il
                // file quando non coincideva.
                //
                // Rifiutava pero' anche i file **conformi**: ARROW-003 chiede
                // un identificatore unico e preservato, non una posizione, e un
                // produttore che numerasse diversamente era conforme e veniva
                // respinto. Ora `read_geometry_contract_metadata` mette il
                // valore letto in `identita_dichiarata` e lascia `field_id`
                // alla nostra enumerazione: il numero del payload non indicizza
                // piu' niente, quindi non c'e' piu' niente da difendere.
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
            return Err(PlenoraIoError::destinazione_esistente());
        }
        // Nessun controllo sul suffisso della destinazione.
        //
        // Ce n'era uno, e rifiutava una destinazione il cui nome non portasse
        // l'estensione attesa. Non era un requisito del formato: dopo il
        // controllo l'estensione non veniva usata per **niente** -- ne' per
        // derivare un nome, ne' per scegliere un comportamento -- ed era una
        // convenzione travestita da vincolo. Il costo era che il formato
        // esplicito non bastava a scegliere la destinazione: chi pubblicava su
        // un percorso di staging, o su un nome generato, veniva rifiutato per
        // il nome invece che per i dati.
        //
        // Il suffisso resta rilevante per chi **rilegge** senza dichiarare il
        // formato, e quella parte e' dichiarata e non taciuta:
        // `recognised_suffixes` del descrittore la rende, e `io.catalog` la
        // pubblica. Scrivere e riconoscere sono due cose, e ora si vedono
        // entrambe.
        if plan.layers.len() != 1 {
            return Err(PlenoraIoError::non_supportato_redatto(
                &PublicMessage::Curated("Arrow IPC: un solo layer per file"),
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
        // ARROW-003: l'identita' dei campi deve sopravvivere al round trip, e
        // il round trip e' una forma di prima classe -- `io.read` e `io.write`
        // sono dichiarate l'una l'inversa dell'altra. Gli id che arrivano dalla
        // sorgente non vengono riscritti: e' cio' che distingue una
        // conservazione da un ricalcolo.
        let fields = with_field_identity(fields);
        let schema = with_contract_version(Arc::new(arrow_schema::Schema::new_with_metadata(
            fields,
            layer.schema.metadata().clone(),
        )));
        let staging = StagedFile::new(&path, opts.durable, opts.max_output_bytes())?;
        let writer = FileWriter::try_new(BufWriter::new(staging.reopen()?), &schema)
            .map_err(|_| err(&PublicMessage::Curated("apertura del writer IPC fallita")))?;
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
            return Err(err(&PublicMessage::CuratedWith(
                "layer runtime inesistente, indice",
                NumeroStrutturale::Indice(u64::from(request.layer.0)),
            )));
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
                            return Err(PlenoraIoError::contratto_redatto(
                                &PublicMessage::CuratedWith(
                                    "projection Required: field id fuori range,",
                                    NumeroStrutturale::Indice(u64::from(field_id.0)),
                                ),
                            ));
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
                .map_err(|_| err(&PublicMessage::Curated("Arrow IPC non valido")))
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
            Some(Err(_)) => Err(err(&PublicMessage::Curated("batch IPC non leggibile"))),
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
            .map_err(|_| err(&PublicMessage::Curated("retag del contratto IPC fallito")))?;
        self.writer
            .as_mut()
            .ok_or_else(|| err(&PublicMessage::Curated("writer chiuso")))?
            .write(&batch)
            .map_err(|_| err(&PublicMessage::Curated("scrittura IPC fallita")))
    }

    fn finish(mut self: Box<Self>) -> Result<Published> {
        let mut w = self
            .writer
            .take()
            .ok_or_else(|| err(&PublicMessage::Curated("writer già chiuso")))?;
        w.finish()
            .map_err(|_| err(&PublicMessage::Curated("chiusura dello stream IPC fallita")))?;
        let mut inner = w
            .into_inner()
            .map_err(|_| err(&PublicMessage::Curated("recupero del writer IPC fallito")))?;
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
mod tests;
