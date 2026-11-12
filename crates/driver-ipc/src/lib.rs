//! driver-ipc — Arrow IPC (`.arrow`) ⇄ `RecordBatch`. **Pass-through nativo**:
//! l'IPC È già Arrow, quindi schema (inclusi i metadati `geoarrow.wkb` + `crs`) e
//! buffer passano SENZA conversione — Lossless, zero decode/encode WKB, streaming
//! reale (il `FileReader` è un iteratore pull, nessun thread). È il formato di
//! interscambio canonico fra plenora-IO-tools e plenora-data-tools.
#![forbid(unsafe_code)]

use std::fs::File;
use std::io::{BufReader, BufWriter, Write as _};
use std::path::PathBuf;
use std::sync::Arc;

use arrow_array::RecordBatch;
use arrow_ipc::reader::{FileReader, StreamReader};
use arrow_ipc::writer::{FileWriter, StreamWriter};
use arrow_schema::SchemaRef;

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
use plenora_io_model::crs::CrsResolution;
use plenora_io_model::geometry::{
    is_geometry_field, read_geometry_contract_metadata, validate_contract_version,
    validate_geometry_field_identity, with_contract_version, with_field_identity,
    with_geometry_contract_metadata, PLENORA_CONTRACT_VERSION_KEY,
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
    OPZIONI,
    &["arrow", "arrows"],
    1,
    3,
    10,
);

/// Le due serializzazioni che Arrow IPC definisce, e come si scelgono.
///
/// # Perche' e' un'opzione di formato e non un campo dello schema d'ingresso
///
/// Perche' e' una proprieta' del **formato**, non dell'operazione: `io.read` e
/// `io.write` non cambiano semantica, cambia come i byte sono disposti. Il
/// catalogo pubblica le opzioni di formato driver per driver, quindi un
/// consumatore la trova dove cerca le altre -- e aggiungerla non tocca
/// `plenora-io-read-input-v1`, che e' un contratto pubblicato.
///
/// # Che cosa **non** cambia
///
/// La consegna resta atomica. Il formato a flusso si scrive per intero nello
/// staging e poi si pubblica, esattamente come il file: `stream` descrive la
/// disposizione dei byte, non il momento in cui diventano visibili. Chi legge
/// non vede un batch prima che l'operazione sia finita, e non lo vede in
/// nessuna delle due serializzazioni.
static OPZIONI: plenora_io_model::format_options::SchemaOpzioniFormato =
    plenora_io_model::format_options::SchemaOpzioniFormato::nuovo(&[
        plenora_io_model::format_options::OpzioneFormato {
            chiave: "serialization",
            fase: plenora_io_model::format_options::FaseOpzione::Scrittura,
            valore: plenora_io_model::format_options::ValoreAmmesso::Enumerato(&["file", "stream"]),
            predefinito: Some("file"),
            descrizione: "serializzazione IPC: contenitore `file` con footer, o flusso `stream`",
        },
    ]);

/// Quale serializzazione porta un file, letta dai **byte** e non dal nome.
///
/// Il contenitore `file` porta `ARROW1` in testa; il flusso no, comincia
/// direttamente con un messaggio. E' il discriminante che la specifica Arrow
/// definisce, ed e' l'unico che non si possa sbagliare rinominando un file.
///
/// Il suffisso resta dichiarato in `recognised_suffixes` e serve a scegliere il
/// **driver** quando il formato non e' dichiarato; a scegliere la
/// serializzazione dentro il driver servono i byte. Sono due domande, e
/// tenerle separate e' la stessa regola per cui `io.convert` pretende i formati
/// espliciti invece di dedurli dalle estensioni.
///
/// # Errors
///
/// Quando il file non si apre.
pub fn serializzazione_del_file(percorso: &std::path::Path) -> Result<Serializzazione> {
    let mut file = File::open(percorso)?;
    let mut magic = [0_u8; 6];
    // Due rami con lo stesso esito, e la ragione e' la stessa: cio' che non
    // comincia con `ARROW1` -- perche' porta altri byte, o perche' di byte non
    // ne ha abbastanza -- non e' il contenitore. Se non e' nemmeno un flusso
    // valido, lo dira' la prevalidazione con il proprio messaggio: non e'
    // questa funzione a decidere se il file sia buono, solo che cosa dichiari
    // di essere.
    match std::io::Read::read_exact(&mut file, &mut magic) {
        Ok(()) if magic == *b"ARROW1" => Ok(Serializzazione::File),
        _ => Ok(Serializzazione::Flusso),
    }
}

/// Le due disposizioni dei byte che Arrow IPC definisce.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Serializzazione {
    /// `ARROW1` in testa e in coda, con il footer che indicizza i blocchi.
    File,
    /// Una sequenza di messaggi, senza footer: nessun accesso casuale.
    Flusso,
}

impl Serializzazione {
    /// Il content type registrato da `ARROW-INTERCHANGE-1.0 §1`.
    ///
    /// Sta qui e non nella CLI perche' il nome deve avere un posto solo: il
    /// driver sceglie la serializzazione e il driver la nomina. Due tabelle --
    /// una che sceglie e una che riferisce -- divergerebbero in silenzio, e chi
    /// sceglie il lettore sul content type aprirebbe con lo strumento
    /// sbagliato.
    #[must_use]
    pub const fn content_type(self) -> &'static str {
        match self {
            Self::File => "application/vnd.apache.arrow.file",
            Self::Flusso => "application/vnd.apache.arrow.stream",
        }
    }
}

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
        let serializzazione = serializzazione_del_file(&path)?;
        match serializzazione {
            Serializzazione::File => {
                driver_common::prevalida_arrow::valida_file_ipc("arrow", &path)?;
            }
            Serializzazione::Flusso => {
                driver_common::prevalida_arrow::valida_flusso_ipc("arrow", &path)?;
            }
        }
        let schema = plenora_io_core::driver::leggendo_arrow("arrow", || match serializzazione {
            Serializzazione::File => FileReader::try_new(File::open(&path)?, None)
                .map(|lettore| lettore.schema())
                .map_err(|_| err(&PublicMessage::Curated("Arrow IPC non valido"))),
            Serializzazione::Flusso => {
                StreamReader::try_new(BufReader::new(File::open(&path)?), None)
                    .map(|lettore| lettore.schema())
                    .map_err(|_| err(&PublicMessage::Curated("flusso Arrow IPC non valido")))
            }
        })?;
        // Arrow Geometry Semantics 1.0, REJ-001/002: versione, vocabolario, CRS e
        // supporto nelle loro classi e nel loro ordine, prima di ogni altra lettura.
        plenora_io_model::vocabolario::verifica_schema(schema.as_ref())?;
        validate_contract_version(schema.as_ref())?;
        let canonical_version_present =
            schema.metadata().contains_key(PLENORA_CONTRACT_VERSION_KEY);
        let geometry_fields: Vec<(usize, &arrow_schema::FieldRef)> = schema
            .fields()
            .iter()
            .enumerate()
            .filter(|(_, field)| is_geometry_field(field))
            .collect();
        // REJ-002: i difetti di vocabolario di **ogni** campo geometria prima
        // del limite di supporto, che non deve coprirli.
        for (_, field) in &geometry_fields {
            validate_geometry_field_identity(field, canonical_version_present)?;
        }
        let received_schema = schema.clone();
        let primary = plenora_io_model::primary::PrimaryState::read(&schema)?;
        let geometry = primary
            .position(&schema)
            .map(|i| {
                let f = schema.field(i);
                let physical = u32::try_from(i).map_err(|_| {
                    err(&PublicMessage::Curated(
                        "indice geometrico non rappresentabile",
                    ))
                })?;
                let mut contract = GeometryColumnContract::wkb_passthrough(
                    FieldId(physical),
                    f.name(),
                    CrsResolution::Missing,
                    f.is_nullable(),
                );
                read_geometry_contract_metadata(f, &mut contract)?;
                Ok::<_, PlenoraIoError>(contract)
            })
            .transpose()?;
        let schema = plenora_io_model::primary::emit(&schema, primary.position(&schema))?;
        let name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("layer")
            .to_owned();
        plenora_io_core::with_read_budget(
            Box::new(IpcDataset {
                path,
                received_schema,
                layers: vec![LayerContract {
                    id: LayerId(0),
                    name,
                    contract: DataContract::new(schema, geometry),
                }],
            }),
            &opts,
            true,
        )
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
        // Le chiavi di schema che IO non possiede passano (ARROW-010), anche un
        // `geo` che contraddice il campo: limite dichiarato in docs/PRODUCT.md
        // («un `geo` di schema passa tale e quale»).
        let schema = with_contract_version(Arc::new(arrow_schema::Schema::new_with_metadata(
            fields,
            layer.schema.metadata().clone(),
        )));
        let schema = plenora_io_model::primary::emit(
            &schema,
            layer.geometry.as_ref().map(|g| g.field_id.0 as usize),
        )?;
        // Guardia del produttore: lo schema porta la versione, quindi vale la
        // verifica di `open` (REJ-001/002), qui prima dello staging: un file che
        // IO stesso rifiuterebbe in lettura non si produce in silenzio.
        plenora_io_model::vocabolario::verifica_schema(schema.as_ref())?;
        // La serializzazione richiesta. Il valore e' gia' stato validato da
        // `validate_write` contro lo schema delle opzioni -- `Enumerato(&["file",
        // "stream"])` -- quindi qui l'unico caso da decidere e' l'assenza, che
        // vale il default dichiarato.
        let flusso = match opts.format_options.get("serialization").map(String::as_str) {
            Some("stream") => true,
            None | Some("file") => false,
            Some(_) => {
                // Ramo difensivo: il validatore centrale ha gia' respinto ogni
                // altro valore con il proprio token bounded.
                return Err(PlenoraIoError::non_supportato_redatto(
                    &PublicMessage::Curated("Arrow IPC: serializzazione sconosciuta"),
                ));
            }
        };
        // Lo staging non cambia con la serializzazione, ed e' il punto: i byte
        // si scrivono per intero in un file temporaneo e la pubblicazione e'
        // l'ultima operazione. Il flusso descrive **come** i byte sono
        // disposti, non quando diventano visibili -- la consegna resta atomica
        // sull'operazione in entrambe le forme.
        let staging = StagedFile::new(
            &path,
            opts.durable,
            opts.max_output_bytes(),
            opts.budget().context(),
        )?;
        let scrittore = if flusso {
            ScrittoreIpc::Flusso(
                StreamWriter::try_new(BufWriter::new(staging.reopen()?), &schema)
                    .map_err(|_| err(&PublicMessage::Curated("apertura del writer IPC fallita")))?,
            )
        } else {
            ScrittoreIpc::File(
                FileWriter::try_new(BufWriter::new(staging.reopen()?), &schema)
                    .map_err(|_| err(&PublicMessage::Curated("apertura del writer IPC fallita")))?,
            )
        };
        with_write_validation(
            Box::new(IpcWriter {
                staging,
                writer: Some(scrittore),
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
    received_schema: SchemaRef,
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
        let (indices, layer) = plenora_io_core::project_layer_contract(source_layer, request)?;
        let projection = request.projected_fields.as_ref().map(|_| indices);
        let path = self.path.clone();
        // Il file viene riaperto qui, quindi viene riverificato qui: fra
        // `open` e questa chiamata il contenuto su disco puo' essere cambiato,
        // e una verifica fatta una volta sola varrebbe per un file che non e'
        // piu' quello.
        let serializzazione = serializzazione_del_file(&path)?;
        match serializzazione {
            Serializzazione::File => {
                driver_common::prevalida_arrow::valida_file_ipc("arrow", &path)?;
            }
            Serializzazione::Flusso => {
                driver_common::prevalida_arrow::valida_flusso_ipc("arrow", &path)?;
            }
        }
        let reader = plenora_io_core::driver::leggendo_arrow("arrow", move || {
            match serializzazione {
                Serializzazione::File => FileReader::try_new(File::open(&path)?, None)
                    .map(|lettore| LettoreIpc::File {
                        lettore,
                        proiezione: projection,
                    })
                    .map_err(|_| err(&PublicMessage::Curated("Arrow IPC non valido"))),
                // Entrambe le serializzazioni decodificano tutte le colonne:
                // il decoder deve verificare anche quelle escluse dal risultato.
                Serializzazione::Flusso => {
                    StreamReader::try_new(BufReader::new(File::open(&path)?), None)
                        .map(|lettore| LettoreIpc::Flusso {
                            lettore: Box::new(lettore),
                            proiezione: projection,
                        })
                        .map_err(|_| err(&PublicMessage::Curated("flusso Arrow IPC non valido")))
                }
            }
        })?;
        let reopened = match &reader {
            LettoreIpc::File { lettore, .. } => lettore.schema(),
            LettoreIpc::Flusso { lettore, .. } => lettore.schema(),
        };
        plenora_io_model::vocabolario::verifica_schema(&reopened)?;
        if reopened != self.received_schema {
            return Err(PlenoraIoError::schema_redatto(&PublicMessage::Curated(
                "schema IPC cambiato alla riapertura",
            )));
        }
        Ok(plenora_io_core::with_batch_target(
            Box::new(IpcReader { reader, layer }),
            request.batch_target,
            request.cancellation.clone(),
        ))
    }
}

/// Le due serializzazioni dietro la stessa interfaccia di lettura.
///
/// `Box` sul flusso perche' `StreamReader` e' molto piu' grande di
/// `FileReader`, e senza il riquadro l'enum costerebbe a entrambe la
/// dimensione della piu' grande.
enum LettoreIpc {
    File {
        lettore: FileReader<File>,
        proiezione: Option<Vec<usize>>,
    },
    Flusso {
        lettore: Box<StreamReader<BufReader<File>>>,
        /// Gli indici delle colonne richieste, applicati al batch invece che
        /// alla sorgente: il flusso non ha un indice da cui saltarle.
        proiezione: Option<Vec<usize>>,
    },
}

struct IpcReader {
    reader: LettoreIpc,
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
        let schema = self.layer.contract.schema.clone();
        let reader = &mut self.reader;
        plenora_io_core::driver::leggendo_arrow("arrow", move || match reader {
            LettoreIpc::File {
                lettore,
                proiezione,
            } => match lettore.next() {
                None => Ok(None),
                Some(Ok(b)) => valida_e_proietta(b, proiezione.as_deref()),
                Some(Err(_)) => Err(err(&PublicMessage::Curated("batch IPC non leggibile"))),
            },
            LettoreIpc::Flusso {
                lettore,
                proiezione,
            } => match lettore.next() {
                None => Ok(None),
                Some(Ok(b)) => valida_e_proietta(b, proiezione.as_deref()),
                Some(Err(_)) => Err(err(&PublicMessage::Curated("batch IPC non leggibile"))),
            },
        })?
        .map(|batch| {
            RecordBatch::try_new_with_options(
                schema,
                batch.columns().to_vec(),
                &arrow_array::RecordBatchOptions::new().with_row_count(Some(batch.num_rows())),
            )
            .map_err(|_| err(&PublicMessage::Curated("schema IPC discorde")))
        })
        .transpose()
    }
}

// La proiezione non deve nascondere dati corrotti nelle colonne escluse.
fn valida_e_proietta(
    batch: RecordBatch,
    projection: Option<&[usize]>,
) -> Result<Option<RecordBatch>> {
    plenora_io_model::vocabolario::verifica_schema(&batch.schema())?;
    plenora_io_core::driver::validate_external_batch(&batch, plenora_io_model::ErrorPhase::Read)?;
    match projection {
        None => Ok(Some(batch)),
        Some(indices) => batch
            .project(indices)
            .map(Some)
            .map_err(|_| err(&PublicMessage::Curated("proiezione del batch IPC fallita"))),
    }
}

/// Le due serializzazioni dietro la stessa interfaccia di scrittura.
enum ScrittoreIpc {
    File(FileWriter<BufWriter<File>>),
    Flusso(StreamWriter<BufWriter<File>>),
}

impl ScrittoreIpc {
    fn scrivi(&mut self, batch: &RecordBatch) -> std::result::Result<(), arrow_schema::ArrowError> {
        match self {
            Self::File(w) => w.write(batch),
            Self::Flusso(w) => w.write(batch),
        }
    }

    /// Chiude la serializzazione e restituisce il flusso sottostante.
    ///
    /// Chiudere e' parte del formato, non una cortesia: il contenitore scrive
    /// il footer e il magic finale, il flusso il marcatore di fine. Un file a
    /// cui mancasse la chiusura sarebbe illeggibile in entrambi i casi, e in
    /// entrambi lo sarebbe **dopo** che la pubblicazione lo ha reso visibile.
    fn chiudi(self) -> Result<BufWriter<File>> {
        match self {
            Self::File(mut w) => {
                w.finish().map_err(|_| {
                    err(&PublicMessage::Curated("chiusura dello stream IPC fallita"))
                })?;
                w.into_inner()
                    .map_err(|_| err(&PublicMessage::Curated("recupero del writer IPC fallito")))
            }
            Self::Flusso(mut w) => {
                w.finish().map_err(|_| {
                    err(&PublicMessage::Curated("chiusura dello stream IPC fallita"))
                })?;
                w.into_inner()
                    .map_err(|_| err(&PublicMessage::Curated("recupero del writer IPC fallito")))
            }
        }
    }
}

struct IpcWriter {
    staging: StagedFile,
    writer: Option<ScrittoreIpc>,
    schema: SchemaRef,
}

impl FormatWriter for IpcWriter {
    fn write(&mut self, batch: &RecordBatch) -> Result<()> {
        plenora_io_core::driver::validate_external_batch(
            batch,
            plenora_io_model::ErrorPhase::Validate,
        )?;
        let batch = RecordBatch::try_new(self.schema.clone(), batch.columns().to_vec())
            .map_err(|_| err(&PublicMessage::Curated("retag del contratto IPC fallito")))?;
        self.writer
            .as_mut()
            .ok_or_else(|| err(&PublicMessage::Curated("writer chiuso")))?
            .scrivi(&batch)
            .map_err(|_| err(&PublicMessage::Curated("scrittura IPC fallita")))
    }

    fn finish(mut self: Box<Self>) -> Result<Published> {
        let w = self
            .writer
            .take()
            .ok_or_else(|| err(&PublicMessage::Curated("writer già chiuso")))?;
        let mut inner = w.chiudi()?;
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
