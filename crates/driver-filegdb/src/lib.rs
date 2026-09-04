//! driver-filegdb — `FileGDB` ⇄ `RecordBatch` (Fase 1, "tier GDB"). È
//! l'unica eccezione alla policy puro-Rust: `FileGDB` richiede GDAL. Dietro la
//! feature `gdal-backend` legge via GDAL; senza feature è uno stub che fallisce
//! tipizzato (il binario di default resta puro-Rust). Multi-layer.
#![forbid(unsafe_code)]

use plenora_io_core::descriptor::{
    ArrowTypeClass, CrsHandling, Direction, Fidelity, FormatDescriptor, GeometryWriteSupport,
    ReadMode, ReaderConcurrency, Runtime, WriteMode,
};
use plenora_io_core::driver::{
    FormatDriver, FormatWriter, OpenDatasetHandle, ReadOptions, Sink, Source, WriteOptions,
};
use plenora_io_core::{
    validate_write, AttributeWriteSupport, CrsDerivation, CrsRepresentationCapabilities,
    CrsRepresentationState, CrsWriteSupport, FormatWriteCapabilities, NullabilitySupport,
    TypeCoercionPolicy, WritePlan, UTF8_FIELD_NAMES,
};
use plenora_io_model::contract::{
    CoordinateDimensions, GeometryEncoding, GeometryType, SpatialSemantics,
};
use plenora_io_model::Result;

const FILEGDB_ATTRIBUTE_TYPES: &[ArrowTypeClass] = &[
    ArrowTypeClass::SignedInteger,
    ArrowTypeClass::Floating,
    ArrowTypeClass::Utf8,
];

const FILEGDB_GEOMETRY: GeometryWriteSupport = GeometryWriteSupport {
    supported: true,
    encodings: &[GeometryEncoding::Wkb],
    dimensions: &[
        CoordinateDimensions::Xy,
        CoordinateDimensions::Xyz,
        CoordinateDimensions::Xym,
        CoordinateDimensions::Xyzm,
    ],
    spatial_semantics: &[SpatialSemantics::Geometry],
    geometry_types: &[
        GeometryType::Point,
        GeometryType::MultiPoint,
        GeometryType::MultiLineString,
        GeometryType::MultiPolygon,
    ],
    mixed_types: false,
};

static DESCRIPTOR: FormatDescriptor = FormatDescriptor::const_new(
    "filegdb",
    Direction::Bidirectional,
    ReadMode::Materializing,
    // INV-7: `for feature in layer.features()`, iteratore GDAL in avanti.
    plenora_io_core::NativeReadMode::StreamingSequential,
    // Il drenaggio e lo spool sono dell'adapter comune, non di
    // questo driver: `BudgetedReader` li impone a tutti.
    plenora_io_core::DeliverySemantics::OperationAtomic,
    plenora_io_core::BufferingStrategy::AdaptiveMemoryThenDisk,
    plenora_io_core::DeterminismLevel::Semantic,
    Some(WriteMode::Streaming),
    Some(plenora_io_core::DeterminismLevel::Semantic),
    true,
    true, // una .gdb è una directory
    ReaderConcurrency::SingleActiveReader,
    plenora_io_core::ProjectionSupport::Exact,
    plenora_io_core::PredicatePruningSupport::None,
    plenora_io_core::SpatialPruningSupport::None,
    CrsHandling::Embedded,
    Fidelity::Conditional,
    Runtime::Gdal,
    // `hostile_input_hardened`: non dichiarato: il percorso passa da GDAL,
    // che non e' nostro e non e' strumentato.
    false,
    // `spec_version_supported`: il formato non si versiona in un modo che
    // il driver possa dichiarare per intero.
    None,
    Some(FormatWriteCapabilities {
        field_names: UTF8_FIELD_NAMES,
        allowed_types: FILEGDB_ATTRIBUTE_TYPES,
        type_coercion: TypeCoercionPolicy::Reject,
        attributes: AttributeWriteSupport::All,
        geometry: FILEGDB_GEOMETRY,
        crs: CrsWriteSupport::Embedded,
        crs_representations: CrsRepresentationCapabilities::new(
            // Il secondo caso di `Derived` senza alcun `Preserved`: a
            // ricavarle non e' il nostro codice ma GDAL, che risolve il CRS
            // quando scrive il dataset.
            CrsRepresentationState::Derived(CrsDerivation::RuntimeResolved),
            CrsRepresentationState::Absent,
            CrsRepresentationState::Derived(CrsDerivation::RuntimeResolved),
        ),
        nullability: NullabilitySupport::FormatDefined,
        multi_layer: true,
    }),
    // Il driver non interpreta alcuna format_option (L0.7): l'elenco vuoto
    // e' l'affermazione che qualunque chiave e' sconosciuta, non un'omissione.
    plenora_io_model::format_options::SchemaOpzioniFormato::VUOTO,
    1,
    10,
    11,
);

pub struct FileGdbDriver;

// --- la superficie del fuzz target -----------------------------------------
//
// Vive qui, e non dentro `fuzz/fuzz_targets/`, per la stessa ragione per cui ci
// vive quella di `driver-shp`: qui e' **provabile**. Le sonde del driver
// chiamano lo stesso entry point del target sulla stessa fixture, e verificano
// cio' che un replay senza crash non verifica -- che l'isolamento fra
// invocazioni ci sia, che i nomi dei file non vengano dal payload, che una
// lettura riuscita porti davvero delle righe.

/// Le parti di un FileGDB, per nome, lette dall'archivio della fixture.
///
/// Un FileGDB non e' un file: e' una **directory** di tabelle che si citano a
/// vicenda per GUID. Il fuzzer consegna pero' un solo blob, e da un blob non si
/// costruisce una directory coerente -- non perche' sia difficile, ma perche' il
/// formato e' proprietario e ricostruito per reverse engineering: costruirlo da
/// zero significherebbe riscrivere `OpenFileGDB` e produrre file validi rispetto
/// alla nostra idea del formato invece che a quella di GDAL.
///
/// La strada e' l'opposta. Si parte da un FileGDB **vero**, prodotto da GDAL da
/// un GeoJSON committato, e il fuzzer ne sostituisce **una parte per volta**.
/// Cio' che muta e' il contenuto di una tabella; cio' che resta e' una directory
/// che il driver ha una ragione di aprire.
///
/// ```text
/// PLENORA-GDB-FIXTURE-1\n
/// u32          numero di parti
/// per parte:   u16 lunghezza del nome, nome ASCII, u32 lunghezza, byte
/// ```
///
/// L'archivio e' nostro e non viene dal fuzzer: e' `include_bytes!` dal lato del
/// target. Qui e' un parametro proprio per questo -- il driver di produzione non
/// porta dentro di se' cinquantadue kilobyte di fixture, e la sonda puo' passare
/// l'archivio letto dal disco.
///
/// `None` = archivio malformato. Non e' un caso che il fuzzer possa produrre:
/// l'archivio non e' l'input.
#[cfg(feature = "gdal-backend")]
#[doc(hidden)]
#[must_use]
pub fn __fuzz_parti_della_fixture(archivio: &[u8]) -> Option<Vec<(String, &[u8])>> {
    const INTESTAZIONE: &[u8] = b"PLENORA-GDB-FIXTURE-1\n";
    let corpo = archivio.strip_prefix(INTESTAZIONE)?;
    let (grezzo, mut resto) = corpo.split_at_checked(4)?;
    let quante = u32::from_le_bytes([grezzo[0], grezzo[1], grezzo[2], grezzo[3]]);

    let mut parti = Vec::new();
    for _ in 0..quante {
        let (grezzo, dopo) = resto.split_at_checked(2)?;
        let lunghezza_nome = usize::from(u16::from_le_bytes([grezzo[0], grezzo[1]]));
        let (nome, dopo) = dopo.split_at_checked(lunghezza_nome)?;
        let (grezzo, dopo) = dopo.split_at_checked(4)?;
        let lunghezza = u32::from_le_bytes([grezzo[0], grezzo[1], grezzo[2], grezzo[3]]);
        let (contenuto, dopo) = dopo.split_at_checked(usize::try_from(lunghezza).ok()?)?;

        let nome = std::str::from_utf8(nome).ok()?;
        if !nome_di_parte_ammesso(nome) {
            // Il nome finisce in un percorso. Non viene dal fuzzer -- viene
            // dall'archivio, che e' nostro -- ma un archivio corrotto non deve
            // poter far scrivere fuori dalla directory.
            return None;
        }
        parti.push((nome.to_owned(), contenuto));
        resto = dopo;
    }
    if !resto.is_empty() {
        return None;
    }
    if parti.is_empty() {
        // Un archivio con zero parti non e' una `.gdb` vuota: e' un archivio che
        // non ne descrive nessuna. Accettarlo porterebbe il chiamante a
        // scegliere una parte **modulo zero**, cioe' a dividere per zero.
        return None;
    }
    Some(parti)
}

/// I soli nomi che una parte di `FileGDB` puo' avere.
///
/// Ogni parte che `OpenFileGDB` scrive comincia con una lettera minuscola --
/// `gdb`, `timestamps`, `a00000001.gdbtable` -- e prosegue con minuscole,
/// cifre, punti e trattini bassi. Niente separatori e niente nomi vuoti: un
/// nome che finisce in un `join` e non e' stato guardato e' un percorso
/// costruito da dei byte.
///
/// La condizione sulla **prima** lettera non e' un dettaglio estetico: senza,
/// `".."` sarebbe fatto di soli caratteri ammessi e passerebbe. E' il buco che
/// la sonda di questo file ha trovato nella prima stesura.
#[cfg(feature = "gdal-backend")]
fn nome_di_parte_ammesso(nome: &str) -> bool {
    let mut byte = nome.bytes();
    byte.next().is_some_and(|primo| primo.is_ascii_lowercase())
        && byte.all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'.' || b == b'_')
}

/// Un errore dell'ambiente non e' un difetto del file letto.
///
/// Un filesystem pieno o una directory non creabile diventano un errore
/// tipizzato e non un panico: un panico dell'harness verrebbe archiviato dal
/// fuzzer come finding del driver, e la campagna misurerebbe il proprio
/// scaffolding.
#[cfg(feature = "gdal-backend")]
fn errore_di_ambiente(_: std::io::Error) -> plenora_io_model::PlenoraIoError {
    // Lo stesso costruttore del resto del driver, e non `non_supportato_redatto`.
    //
    // In questa crate `Unsupported` ha un significato preciso e difeso da
    // `tests/ostili.rs`: «il binario non e' stato costruito per parlare con
    // GDAL». Riusarlo per «il filesystem non ha collaborato» farebbe cercare a
    // chi legge una libreria da installare, e annacquerebbe la sola categoria
    // che distingue lo stub dal guasto. `driver-shp` fa la stessa scelta per il
    // proprio errore d'ambiente.
    backend::err(&plenora_io_model::PublicMessage::Curated(
        "materializzazione della .gdb fallita: e' l'ambiente, non il file letto",
    ))
}

/// Scrive le parti dentro una directory **gia' esistente**, sostituendone una.
///
/// Separata da `__fuzz_leggi_gdb` per una ragione sola: cosi' una sonda puo'
/// passarle una radice inesistente e osservare l'errore, invece di forzare il
/// fallimento mutando `TMPDIR` -- che renderebbe il difetto visibile agli altri
/// test in parallelo e il fallimento intermittente invece che riproducibile.
///
/// `sostituita` e' un **indice**, non un nome: il payload sceglie quale parte
/// cambiare, mai come si chiama. Nessun percorso deriva dai byte del fuzzer.
#[cfg(feature = "gdal-backend")]
fn materializza_gdb(
    radice: &std::path::Path,
    parti: &[(String, &[u8])],
    sostituita: usize,
    contenuto: &[u8],
) -> Result<std::path::PathBuf> {
    let dataset = radice.join("citta.gdb");
    std::fs::create_dir_all(&dataset).map_err(errore_di_ambiente)?;
    for (indice, (nome, originale)) in parti.iter().enumerate() {
        let byte = if indice == sostituita {
            contenuto
        } else {
            originale
        };
        std::fs::write(dataset.join(nome), byte).map_err(errore_di_ambiente)?;
    }
    Ok(dataset)
}

// L'ultima directory che `__fuzz_leggi_gdb` ha usato, per la sola sonda che
// prova l'isolamento.
//
// Serve perche' l'isolamento **non si vede dall'esito**: la materializzazione
// riscrive tutte le parti a ogni invocazione, quindi una directory riusata
// darebbe gli stessi risultati di una nuova. Una sonda che si limitasse a
// rileggere la fixture dopo averne rotta una parte passerebbe in entrambi i
// casi, e proverebbe soltanto che la riscrittura funziona.
//
// Cio' che va provato e' che la directory sia **diversa** ogni volta, e per
// provarlo bisogna vederla. Il seam esiste solo sotto `cfg(test)`: in campagna
// non c'e' niente da registrare.
#[cfg(all(test, feature = "gdal-backend"))]
thread_local! {
    static ULTIMA_DIRECTORY: std::cell::RefCell<Vec<std::path::PathBuf>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

/// Entry point non stabile per libFuzzer: una `.gdb` completa, letta davvero.
///
/// L'input del fuzzer si legge cosi':
///
/// ```text
/// byte 0    quale parte sostituire (modulo il numero di parti)
/// byte 1..  il contenuto che prende il suo posto
/// ```
///
/// Il modulo non e' pigrizia: **ogni** byte deve scegliere una parte, altrimenti
/// il fuzzer passerebbe il tempo a produrre input scartati invece che input che
/// arrivano al driver. Un input vuoto materializza la fixture intatta, ed e' il
/// caso che prova che la base si legge.
///
/// La directory e' **nuova a ogni invocazione**. Riusarla farebbe sopravvivere
/// la tabella mutata dall'input precedente, e la campagna misurerebbe la propria
/// directory invece del formato.
///
/// # Che cosa `Err` significa qui
///
/// Quasi tutto. Una tabella sostituita con byte casuali non e' un FileGDB, e il
/// rifiuto tipizzato e' l'esito atteso. Il finding e' il **panico**, l'abort, o
/// un output parziale consegnato prima di un errore terminale.
#[cfg(feature = "gdal-backend")]
#[doc(hidden)]
pub fn __fuzz_leggi_gdb(archivio: &[u8], dati: &[u8], opts: ReadOptions) -> Result<usize> {
    let Some(parti) = __fuzz_parti_della_fixture(archivio) else {
        return Err(backend::err(&plenora_io_model::PublicMessage::Curated(
            "archivio della fixture illeggibile",
        )));
    };

    let (sostituita, contenuto) = match dati.split_first() {
        // Nessun input: la fixture intatta. E' il caso che dice se la base si
        // legge ancora, e senza di esso una campagna verde potrebbe voler dire
        // che nessun input arriva al driver.
        None => (parti.len(), &[][..]),
        Some((scelta, resto)) => (usize::from(*scelta) % parti.len(), resto),
    };

    let directory = tempfile::Builder::new()
        .prefix("plenora-fuzz-gdb-")
        .tempdir()
        .map_err(errore_di_ambiente)?;
    let dataset = materializza_gdb(directory.path(), &parti, sostituita, contenuto)?;
    #[cfg(test)]
    ULTIMA_DIRECTORY.with(|viste| viste.borrow_mut().push(dataset.clone()));

    __fuzz_drena(&dataset, opts)
}

/// Apre e **drena**: catalogo, schema e righe.
///
/// Fermarsi all'apertura coprirebbe il riconoscimento del formato e nient'altro.
/// Il catalogo viene da `layers()`, lo schema dal contratto di ciascun layer, le
/// righe da `next_batch` fino a `None`.
#[cfg(feature = "gdal-backend")]
fn __fuzz_drena(dataset: &std::path::Path, opts: ReadOptions) -> Result<usize> {
    use plenora_io_core::request::{BatchTarget, ProjectionMode, ReadRequest, ReadScope};

    let aperto = FileGdbDriver.open(Source::Path(dataset.to_path_buf()), opts)?;
    let layer_id: Vec<plenora_io_model::contract::LayerId> =
        aperto.layers().iter().map(|layer| layer.id).collect();

    let mut righe = 0_usize;
    for layer in layer_id {
        let mut reader = aperto.open_layer_reader(&ReadRequest {
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

/// Verifica a runtime che GDAL esponga `OpenFileGDB` come driver vettoriale
/// bidirezionale.
///
/// In assenza della feature, o se una capability non è dichiarata dal runtime
/// caricato, il risultato è deliberatamente fail-closed.
// Senza `gdal-backend` il corpo si riduce a `false` e clippy la vorrebbe
// `const fn`; con la feature attiva chiama GDAL e const non è possibile. La
// firma pubblica resta unica in entrambe le configurazioni.
#[allow(clippy::missing_const_for_fn)]
#[must_use]
pub fn runtime_available() -> bool {
    #[cfg(feature = "gdal-backend")]
    {
        backend::runtime_available()
    }
    #[cfg(not(feature = "gdal-backend"))]
    {
        false
    }
}

/// Le radici dell'artefatto, applicate **dentro GDAL** e non solo all'ambiente.
///
/// # Perche' non basta l'ambiente
///
/// La CLI le impone gia' con `std::env::set_var`, e su Linux e' abbastanza:
/// `setenv` aggiorna `environ`, che e' quello che `getenv` legge anche dalle
/// librerie native.
///
/// Su Windows no. `set_var` chiama `SetEnvironmentVariableW`, che aggiorna il
/// blocco d'ambiente **del processo**; il runtime C ne mantiene una copia, e
/// `getenv` legge quella. GDAL e PROJ chiamano `getenv`: la variabile impostata
/// da Rust non la vedono affatto, e l'artefatto Windows falliva alla prima
/// conversione con un CRS -- «Cannot find proj.db» -- pur avendo `share/proj`
/// accanto a se'.
///
/// # Due meccanismi, perche' uno non copre l'altro
///
/// GDAL e PROJ non condividono la tabella delle config option. E' stato
/// misurato, non supposto -- default nascosto e ambiente avvelenato, su GDAL
/// 3.6:
///
/// - `GDAL_DATA` come config option arriva a GDAL;
/// - `PROJ_DATA` come config option **non** arriva a PROJ, e nemmeno `PROJ_LIB`;
/// - `OSRSetPROJSearchPaths` arriva.
///
/// La prima stesura applicava le sole config option. Il relocation smoke
/// Windows e' rimasto rosso con lo stesso messaggio, ed e' cosi' che la
/// differenza e' venuta fuori.
#[cfg(feature = "gdal-backend")]
pub mod radici {
    use std::sync::Once;

    static UNA_VOLTA: Once = Once::new();

    /// Applica le radici a GDAL e a PROJ, una volta per processo.
    ///
    /// Va chiamata **prima di qualunque uso di GDAL**: il finder dei dati si
    /// inizializza al primo `CPLFindFile`, e una config option impostata dopo
    /// non lo sposta piu'. E' il motivo per cui la chiama `main`, e non solo
    /// l'apertura di un dataset: la risoluzione del CRS avviene in validazione,
    /// prima che questo driver apra qualcosa, e applicare le radici li' dentro
    /// arrivava troppo tardi.
    ///
    /// Un errore nell'impostare una config option non ferma il comando: GDAL
    /// continuerebbe con i propri default, e il rifiuto arriverebbe piu' avanti
    /// da chi cerca un dato che non c'e' -- con un messaggio che parla del dato,
    /// non della configurazione. E' l'unica scelta che lascia la diagnosi al
    /// posto giusto.
    pub fn applica() {
        UNA_VOLTA.call_once(|| {
            for (chiave, valore) in plenora_io_core::radici::piano_del_processo() {
                if let Some(testo) = valore.to_str() {
                    let _ = gdal::config::set_config_option(chiave, testo);
                }
            }
            // PROJ a parte: la sua ricerca non passa dalle config option.
            if let Some(proj) = plenora_io_core::radici::proj_del_processo() {
                if let Some(testo) = proj.to_str() {
                    let _ = gdal::spatial_ref::set_proj_search_paths(&[testo]);
                }
            }
        });
    }
}

impl FormatDriver for FileGdbDriver {
    fn descriptor(&self) -> &FormatDescriptor {
        &DESCRIPTOR
    }

    // I `return` cfg-gated servono per il caso feature-on (il blocco feature-off,
    // pur rimosso, segue sintatticamente); clippy non lo coglie.
    #[allow(clippy::needless_return)]
    // `mut` serve solo al ramo con `gdal-backend`, che passa `&mut opts` al
    // preflight. Senza la feature nessuno la muta, e clippy ha ragione: il
    // lint si spegne dove il ramo non c'e', non dappertutto.
    #[cfg_attr(not(feature = "gdal-backend"), allow(unused_mut))]
    fn open(&self, source: Source, mut opts: ReadOptions) -> Result<Box<dyn OpenDatasetHandle>> {
        #[cfg(feature = "gdal-backend")]
        {
            radici::applica();
            let path = plenora_io_core::preflight_source(self.descriptor(), source, &mut opts)?;
            let dataset = backend::open(&path, opts.assume_crs.as_deref())?;
            return Ok(plenora_io_core::with_read_budget(dataset, &opts, false));
        }
        #[cfg(not(feature = "gdal-backend"))]
        {
            let _ = (source, opts);
            Err(plenora_io_model::PlenoraIoError::non_supportato_redatto(
                &plenora_io_model::PublicMessage::Curated(
                    "FileGDB richiede il tier GDB: compilare con --features gdal-backend",
                ),
            ))
        }
    }

    #[allow(clippy::needless_return)]
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
        #[cfg(feature = "gdal-backend")]
        {
            let Sink::Path(path) = sink;
            return backend::create(&path, plan, opts).and_then(|writer| {
                plenora_io_core::with_write_validation(writer, self.descriptor(), plan, opts)
            });
        }
        #[cfg(not(feature = "gdal-backend"))]
        {
            let _ = (sink, plan, opts);
            Err(plenora_io_model::PlenoraIoError::non_supportato_redatto(
                &plenora_io_model::PublicMessage::Curated(
                    "scrittura FileGDB richiede il tier GDB: compilare con --features gdal-backend",
                ),
            ))
        }
    }
}

#[cfg(feature = "gdal-backend")]
mod backend {
    use super::DESCRIPTOR;

    use std::collections::{HashMap, HashSet};
    use std::fs::{File, OpenOptions, TryLockError};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Arc;

    use arrow_array::builder::{
        BinaryBuilder, Float64Builder, Int32Builder, Int64Builder, StringBuilder,
    };
    use arrow_array::{
        ArrayRef, BinaryArray, Float64Array, Int32Array, RecordBatch, RecordBatchOptions,
        StringArray,
    };
    use arrow_schema::{Field, Schema, SchemaRef};
    use gdal::vector::LayerAccess;
    use gdal::{Dataset, Metadata};

    use driver_common::geometry_field;
    use plenora_io_core::driver::{
        spawn_batch_reader, BatchEmitter, LayerReader, OpenDatasetHandle,
    };
    use plenora_io_core::request::ReadRequest;
    use plenora_io_model::contract::{
        CoordinateDimensions, DataContract, FieldId, GeometryColumnContract, GeometryType,
        LayerContract, LayerId,
    };
    use plenora_io_model::crs::{AxisOrder, CrsKind, RawCrs, ResolvedCrs};
    use plenora_io_model::geometry::with_geometry_contract_metadata;
    use plenora_io_model::{
        CapabilityReason, NumeroStrutturale, PlenoraIoError, PublicMessage, Result,
    };

    pub fn runtime_available() -> bool {
        let Ok(driver) = DriverManager::get_driver_by_name("OpenFileGDB") else {
            return false;
        };
        ["DCAP_VECTOR", "DCAP_OPEN", "DCAP_CREATE"]
            .into_iter()
            .all(|capability| driver.metadata_item(capability, "") == Some("YES".to_owned()))
    }

    // --- scrittura (tier GDB via GDAL OpenFileGDB) --------------------------
    use std::path::{Path, PathBuf};

    use arrow_array::Array;
    use arrow_schema::DataType;
    use driver_common::geometry_index;
    use gdal::errors::GdalError;
    use gdal::spatial_ref::SpatialRef;
    use gdal::vector::{Feature, FieldDefn, Geometry, LayerOptions, OGRwkbGeometryType};
    use gdal::DriverManager;

    use plenora_io_core::driver::{FormatWriter, Published, WriteOptions};
    use plenora_io_core::loss::LossReport;
    use plenora_io_core::publish::publish_dir_atomic;
    use plenora_io_core::{SingleReaderGate, WriteLayer, WritePlan};

    const OGR_FIELD_TYPE_KEY: &str = "plenora.filegdb.ogr_field_type";
    const OGR_FIELD_WIDTH_KEY: &str = "plenora.filegdb.width";
    const OGR_FIELD_PRECISION_KEY: &str = "plenora.filegdb.precision";
    const STAGING_MARKER: &str = ".plenora-tmp-";
    static STAGING_SEQUENCE: AtomicU64 = AtomicU64::new(0);

    #[derive(Clone, Copy)]
    enum FieldKind {
        Int32,
        Float64,
        Utf8,
    }

    impl FieldKind {
        fn from(field: &Field) -> Result<Self> {
            let kind = match field.data_type() {
                DataType::Int32 => Self::Int32,
                DataType::Float64 => Self::Float64,
                DataType::Utf8 => Self::Utf8,
                other => {
                    return Err(PlenoraIoError::capability_redatta(
                        "filegdb",
                        None,
                        CapabilityReason::TypeNotRepresentable,
                        &PublicMessage::CuratedPair(
                            "tipo Arrow non round-trip nativo (supportati esattamente Int32, \
                             Float64 e Utf8), classe:",
                            driver_common::classe_arrow(other),
                        ),
                    ));
                }
            };
            Ok(kind)
        }

        const fn ogr(self) -> gdal::vector::OGRFieldType::Type {
            match self {
                Self::Int32 => gdal::vector::OGRFieldType::OFTInteger,
                Self::Float64 => gdal::vector::OGRFieldType::OFTReal,
                Self::Utf8 => gdal::vector::OGRFieldType::OFTString,
            }
        }
    }

    fn native_i32(field: &Field, key: &'static str) -> Result<Option<i32>> {
        let Some(value) = field.metadata().get(key) else {
            return Ok(None);
        };
        let parsed = value.parse::<i32>().map_err(|_| {
            PlenoraIoError::capability_redatta(
                "filegdb",
                None,
                CapabilityReason::TypeNotRepresentable,
                &PublicMessage::CuratedPair("metadato nativo non intero:", key),
            )
        })?;
        if parsed < 0 {
            return Err(PlenoraIoError::capability_redatta(
                "filegdb",
                None,
                CapabilityReason::TypeNotRepresentable,
                &PublicMessage::CuratedPair("metadato nativo negativo:", key),
            ));
        }
        Ok(Some(parsed))
    }

    #[derive(Clone)]
    struct PlanField {
        name: String,
        index: usize,
        kind: FieldKind,
        width: Option<i32>,
        precision: Option<i32>,
    }

    struct PlanLayer {
        name: String,
        geom_idx: usize,
        fields: Vec<PlanField>,
        srs: SpatialRef,
        ogr_type: OGRwkbGeometryType::Type,
        gdal_idx: usize,
    }

    struct StagingGuard {
        path: PathBuf,
        lock_path: PathBuf,
        lock: Option<File>,
        armed: bool,
    }

    impl StagingGuard {
        fn create(dest: &Path) -> Result<Self> {
            recover_orphaned_staging(dest)?;
            let parent = dataset_parent(dest);
            let prefix = staging_prefix(dest);
            loop {
                let sequence = STAGING_SEQUENCE.fetch_add(1, Ordering::Relaxed);
                let token = format!("{}-{sequence}", std::process::id());
                let base = format!("{prefix}{token}");
                let path = parent.join(format!("{base}.gdb"));
                let lock_path = parent.join(format!("{base}.lock"));
                let lock = match OpenOptions::new()
                    .read(true)
                    .write(true)
                    .create_new(true)
                    .open(&lock_path)
                {
                    Ok(lock) => lock,
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                    Err(error) => return Err(error.into()),
                };
                lock.lock()?;
                return Ok(Self {
                    path,
                    lock_path,
                    lock: Some(lock),
                    armed: true,
                });
            }
        }

        fn path(&self) -> &Path {
            &self.path
        }

        fn disarm(&mut self) {
            drop(self.lock.take());
            let _ = std::fs::remove_file(&self.lock_path);
            self.armed = false;
        }

        fn cleanup(&mut self) {
            if self.armed {
                let _ = std::fs::remove_dir_all(&self.path);
                drop(self.lock.take());
                let _ = std::fs::remove_file(&self.lock_path);
                self.armed = false;
            }
        }
    }

    impl Drop for StagingGuard {
        fn drop(&mut self) {
            self.cleanup();
        }
    }

    struct GdbWriter {
        ds: Option<Dataset>,
        staging: StagingGuard,
        dest: PathBuf,
        durable: bool,
        max_output_bytes: u64,
        layers: Vec<PlanLayer>,
    }

    impl Drop for GdbWriter {
        fn drop(&mut self) {
            drop(self.ds.take());
            self.staging.cleanup();
        }
    }

    fn layer_spatial_ref(layer: &WriteLayer) -> Result<SpatialRef> {
        let resolved = layer
            .contract
            .geometry
            .as_ref()
            .and_then(GeometryColumnContract::resolved_crs)
            .ok_or_else(|| {
                PlenoraIoError::crs_redatto(&PublicMessage::Curated(
                    "FileGDB richiede un CRS risolto per ogni layer",
                ))
            })?;
        let definition = resolved
            .definition
            .as_deref()
            .filter(|definition| !definition.trim().is_empty())
            .or(resolved.id.as_deref())
            .ok_or_else(|| {
                PlenoraIoError::crs_non_risolto_redatto(
                    "filegdb",
                    &RawCrs::new(
                        "ResolvedCrs senza identificatore o definizione".to_owned(),
                        None,
                    ),
                )
            })?;
        let spatial_ref = SpatialRef::from_definition(definition).map_err(|_| {
            PlenoraIoError::crs_non_risolto_redatto(
                "filegdb",
                &RawCrs::new(definition.to_owned(), resolved.id.clone()),
            )
        })?;
        if let (Some(expected), Some(actual)) = (resolved.id.as_deref(), authority_id(&spatial_ref))
        {
            if !expected.eq_ignore_ascii_case(&actual) {
                return Err(PlenoraIoError::crs_non_risolto_redatto(
                    "filegdb",
                    &RawCrs::new(definition.to_owned(), resolved.id.clone()),
                ));
            }
        }
        Ok(spatial_ref)
    }

    /// Una capability geometrica assente, redatta.
    ///
    /// Il campo **non viene piu' passato come nome nudo**. `ContractIdentifier`
    /// nasce solo da un contratto validato, e qui il nome puo' arrivare tanto
    /// dal piano quanto dallo schema che GDAL ha letto dal file: distinguerli
    /// sito per sito costerebbe piu' di quanto vale, e sbagliarne uno
    /// significherebbe far entrare un nome del payload in un tipo che promette
    /// il contrario.
    ///
    /// Il campo geometrico di un `FileGDB` e' uno solo, e chi legge l'errore ha
    /// il contratto: l'identita' non si perde, cambia solo da dove si legge.
    fn geometry_capability(reason: CapabilityReason, detail: &PublicMessage) -> PlenoraIoError {
        PlenoraIoError::capability_redatta("filegdb", None, reason, detail)
    }

    /// `FileGDB` richiede tipo e dimensionalità del feature class prima dei dati:
    /// non li deduciamo dal primo record, che renderebbe vuoti/null dipendenti
    /// dall'ordine dei batch.
    fn contract_ogr_type(layer: &WriteLayer) -> Result<OGRwkbGeometryType::Type> {
        use CoordinateDimensions as D;
        use GeometryType as G;
        use OGRwkbGeometryType as O;

        let geometry = layer.contract.geometry.as_ref().ok_or_else(|| {
            geometry_capability(
                CapabilityReason::GeometryNotSupported,
                &PublicMessage::Curated("layer senza contratto geometrico"),
            )
        })?;
        let geometry_type = match geometry.geometry_types.as_slice() {
            [geometry_type] => *geometry_type,
            [] => {
                return Err(geometry_capability(
                    CapabilityReason::GeometryNotSupported,
                    &PublicMessage::Curated("FileGDB richiede un tipo geometrico dichiarato"),
                ));
            }
            _ => {
                return Err(geometry_capability(
                    CapabilityReason::MixedGeometry,
                    &PublicMessage::Curated("FileGDB richiede un solo tipo geometrico per layer"),
                ));
            }
        };
        if geometry_type == GeometryType::GeometryCollection {
            return Err(geometry_capability(
                CapabilityReason::GeometryNotSupported,
                &PublicMessage::Curated("GeometryCollection non è un feature-class FileGDB nativo"),
            ));
        }

        match (geometry_type, geometry.dimensions) {
            (G::Point, D::Xy) => Ok(O::wkbPoint),
            (G::MultiPoint, D::Xy) => Ok(O::wkbMultiPoint),
            (G::MultiLineString, D::Xy) => Ok(O::wkbMultiLineString),
            (G::MultiPolygon, D::Xy) => Ok(O::wkbMultiPolygon),
            (G::Point, D::Xyz) => Ok(O::wkbPoint25D),
            (G::MultiPoint, D::Xyz) => Ok(O::wkbMultiPoint25D),
            (G::MultiLineString, D::Xyz) => Ok(O::wkbMultiLineString25D),
            (G::MultiPolygon, D::Xyz) => Ok(O::wkbMultiPolygon25D),
            (G::Point, D::Xym) => Ok(O::wkbPointM),
            (G::MultiPoint, D::Xym) => Ok(O::wkbMultiPointM),
            (G::MultiLineString, D::Xym) => Ok(O::wkbMultiLineStringM),
            (G::MultiPolygon, D::Xym) => Ok(O::wkbMultiPolygonM),
            (G::Point, D::Xyzm) => Ok(O::wkbPointZM),
            (G::MultiPoint, D::Xyzm) => Ok(O::wkbMultiPointZM),
            (G::MultiLineString, D::Xyzm) => Ok(O::wkbMultiLineStringZM),
            (G::MultiPolygon, D::Xyzm) => Ok(O::wkbMultiPolygonZM),
            (G::LineString | G::Polygon, D::Xy | D::Xyz | D::Xym | D::Xyzm) => {
                Err(geometry_capability(
                    CapabilityReason::GeometryNotSupported,
                    &PublicMessage::Curated("FileGDB normalizza le famiglie lineari/poligonali native a MultiLineString/MultiPolygon; dichiarare il tipo multipart per un round-trip stabile"),
                ))
            }
            (_, D::Unknown) => Err(geometry_capability(
                CapabilityReason::CoordinateDimensions,
                &PublicMessage::Curated("FileGDB richiede dimensionalità XY o XYZ dichiarata"),
            )),
            (G::GeometryCollection, _) => Err(geometry_capability(
                CapabilityReason::GeometryNotSupported,
                &PublicMessage::Curated("GeometryCollection non rappresentabile nel profilo FileGDB corrente"),
            )),
            (unsupported, _) => Err(geometry_capability(
                CapabilityReason::GeometryNotSupported,
                &PublicMessage::CuratedPair(
                    "tipo geometrico non rappresentabile nel profilo FileGDB corrente:",
                    unsupported.canonical_name(),
                ),
            )),
        }
    }

    fn dataset_parent(dest: &Path) -> PathBuf {
        dest.parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."))
            .to_owned()
    }

    fn staging_prefix(dest: &Path) -> String {
        let stem = dest.file_stem().and_then(|s| s.to_str()).unwrap_or("out");
        format!("{stem}{STAGING_MARKER}")
    }

    fn recover_orphaned_staging(dest: &Path) -> Result<usize> {
        let parent = dataset_parent(dest);
        let prefix = staging_prefix(dest);
        let mut recovered = 0;
        for entry in std::fs::read_dir(&parent)? {
            let entry = entry?;
            if !entry.file_type()?.is_file() {
                continue;
            }
            let name = entry.file_name();
            let Some(name) = name.to_str() else {
                continue;
            };
            let Some(base) = name
                .strip_prefix(&prefix)
                .and_then(|rest| rest.strip_suffix(".lock"))
            else {
                continue;
            };
            if base.is_empty() || base.contains(std::path::MAIN_SEPARATOR) {
                continue;
            }
            let lock = OpenOptions::new()
                .read(true)
                .write(true)
                .open(entry.path())?;
            match lock.try_lock() {
                Ok(()) => {
                    let staging = parent.join(format!("{prefix}{base}.gdb"));
                    if staging.exists() {
                        std::fs::remove_dir_all(staging)?;
                    }
                    drop(lock);
                    std::fs::remove_file(entry.path())?;
                    recovered += 1;
                }
                Err(TryLockError::WouldBlock) => {}
                Err(TryLockError::Error(error)) => return Err(error.into()),
            }
        }
        Ok(recovered)
    }

    #[cfg(test)]
    fn staging_artifacts(dest: &Path) -> Vec<PathBuf> {
        let parent = dataset_parent(dest);
        let prefix = staging_prefix(dest);
        let mut artifacts = std::fs::read_dir(parent)
            .into_iter()
            .flatten()
            .filter_map(std::result::Result::ok)
            .filter_map(|entry| {
                let name = entry.file_name();
                let name = name.to_str()?;
                // Gli artefatti di staging sono creati da questo driver con
                // estensione minuscola: il confronto case-sensitive e' voluto e
                // non va allargato a varianti maiuscole altrui.
                #[allow(clippy::case_sensitive_file_extension_comparisons)]
                let is_artifact = name.starts_with(&prefix)
                    && (name.ends_with(".gdb") || name.ends_with(".lock"));
                is_artifact.then(|| entry.path())
            })
            .collect::<Vec<_>>();
        artifacts.sort();
        artifacts
    }

    #[cfg(test)]
    fn crash_failpoint(point: &str) {
        if std::env::var("PLENORA_FILEGDB_CRASH_POINT").ok().as_deref() == Some(point) {
            std::process::abort();
        }
    }

    fn field_value(
        kind: FieldKind,
        array: &ArrayRef,
        row: usize,
    ) -> Result<Option<gdal::vector::FieldValue>> {
        use gdal::vector::FieldValue as F;
        if array.is_null(row) {
            return Ok(None);
        }
        let value = match kind {
            FieldKind::Int32 => F::IntegerValue(
                array
                    .as_any()
                    .downcast_ref::<Int32Array>()
                    .ok_or_else(|| {
                        err(&PublicMessage::Curated(
                            "schema Int32 ma array runtime differente",
                        ))
                    })?
                    .value(row),
            ),
            FieldKind::Float64 => F::RealValue(
                array
                    .as_any()
                    .downcast_ref::<Float64Array>()
                    .ok_or_else(|| {
                        err(&PublicMessage::Curated(
                            "schema Float64 ma array runtime differente",
                        ))
                    })?
                    .value(row),
            ),
            FieldKind::Utf8 => F::StringValue(
                array
                    .as_any()
                    .downcast_ref::<StringArray>()
                    .ok_or_else(|| {
                        err(&PublicMessage::Curated(
                            "schema Utf8 ma array runtime differente",
                        ))
                    })?
                    .value(row)
                    .to_owned(),
            ),
        };
        Ok(Some(value))
    }

    fn dir_size(p: &Path) -> u64 {
        let mut total = 0;
        if let Ok(rd) = std::fs::read_dir(p) {
            for e in rd.flatten() {
                match e.metadata() {
                    Ok(m) if m.is_dir() => total += dir_size(&e.path()),
                    Ok(m) => total += m.len(),
                    _ => {}
                }
            }
        }
        total
    }

    // La sequenza staging → creazione layer → validazione CRS è un'unica
    // transazione: spezzarla in helper renderebbe meno leggibile l'ordine dei
    // fallimenti fail-closed senza cambiarne il comportamento.
    #[allow(clippy::too_many_lines)]
    pub fn create(
        path: &Path,
        plan: &WritePlan,
        opts: &WriteOptions,
    ) -> Result<Box<dyn FormatWriter>> {
        if path.exists() {
            return Err(PlenoraIoError::destinazione_esistente());
        }

        // Risolvi ogni CRS prima di creare lo staging: un CRS non rappresentabile
        // fallisce senza lasciare output parziali.
        let mut infos = Vec::new();
        for l in &plan.layers {
            let schema = &l.contract.schema;
            let geom_idx = geometry_index(schema)
                .ok_or_else(|| err(&PublicMessage::Curated("layer senza colonna geometria")))?;
            let fields = schema
                .fields()
                .iter()
                .enumerate()
                .filter(|(i, _)| *i != geom_idx)
                .map(|(index, field)| {
                    let kind = FieldKind::from(field)?;
                    // OGRFieldType::Type è un u32 con costanti nell'intervallo
                    // 0..=13: il cast a i32 non può cambiare segno.
                    #[allow(clippy::cast_possible_wrap)]
                    let ogr_type = kind.ogr() as i32;
                    if native_i32(field, OGR_FIELD_TYPE_KEY)?
                        .is_some_and(|native_type| native_type != ogr_type)
                    {
                        return Err(PlenoraIoError::capability_redatta(
                            "filegdb",
                            None,
                            CapabilityReason::TypeNotRepresentable,
                            &PublicMessage::Curated("tipo Arrow e metadato OGR nativo incoerenti"),
                        ));
                    }
                    Ok(PlanField {
                        name: field.name().clone(),
                        index,
                        kind,
                        width: native_i32(field, OGR_FIELD_WIDTH_KEY)?,
                        precision: native_i32(field, OGR_FIELD_PRECISION_KEY)?,
                    })
                })
                .collect::<Result<Vec<_>>>()?;
            infos.push(PlanLayer {
                name: l.name.clone(),
                geom_idx,
                fields,
                srs: layer_spatial_ref(l)?,
                ogr_type: contract_ogr_type(l)?,
                gdal_idx: infos.len(),
            });
        }

        let staging = StagingGuard::create(path)?;
        let driver = DriverManager::get_driver_by_name("OpenFileGDB").map_err(|_| {
            err(&PublicMessage::Curated(
                "driver OpenFileGDB non disponibile",
            ))
        })?;
        let mut ds = driver
            .create_vector_only(staging.path())
            .map_err(|_| err(&PublicMessage::Curated("creazione del FileGDB fallita")))?;

        // Il contratto basta a creare anche layer vuoti o con sole geometrie
        // nulle, senza dipendere dal primo record osservato. Verifichiamo
        // subito anche che GDAL non abbia rinominato o riclassificato campi.
        let layer_result = (|| -> Result<()> {
            for info in &infos {
                let layer = ds
                    .create_layer(LayerOptions {
                        name: &info.name,
                        srs: Some(&info.srs),
                        ty: info.ogr_type,
                        options: None,
                    })
                    // Il nome del layer non esce: viene dal piano.
                    .map_err(|_| err(&PublicMessage::Curated("create_layer fallita")))?;
                for field in &info.fields {
                    let definition =
                        FieldDefn::new(&field.name, field.kind.ogr()).map_err(|_| {
                            err(&PublicMessage::Curated("definizione di un campo fallita"))
                        })?;
                    if let Some(width) = field.width {
                        definition.set_width(width);
                    }
                    if let Some(precision) = field.precision {
                        definition.set_precision(precision);
                    }
                    definition.add_to_layer(&layer).map_err(|_| {
                        err(&PublicMessage::Curated("creazione di un campo fallita"))
                    })?;
                }
                let actual: Vec<(String, gdal::vector::OGRFieldType::Type, i32, i32)> = layer
                    .defn()
                    .fields()
                    .map(|field| {
                        (
                            field.name(),
                            field.field_type(),
                            field.width(),
                            field.precision(),
                        )
                    })
                    .collect();
                if actual.len() != info.fields.len() {
                    return Err(geometry_capability(
                        CapabilityReason::TypeNotRepresentable,
                        &PublicMessage::Curated(
                            "GDAL ha creato un numero di campi diverso dal contratto",
                        ),
                    ));
                }
                for (expected, (actual_name, actual_type, actual_width, actual_precision)) in
                    info.fields.iter().zip(actual)
                {
                    if expected.name != actual_name {
                        // Ne' il nome atteso ne' quello normalizzato escono:
                        // il primo viene dal piano, il secondo e' un derivato
                        // che GDAL ha prodotto leggendolo.
                        return Err(PlenoraIoError::capability_redatta(
                            "filegdb",
                            None,
                            CapabilityReason::FieldNameCollision,
                            &PublicMessage::Curated(
                                "GDAL ha normalizzato il nome del campo; scrittura rifiutata",
                            ),
                        ));
                    }
                    if expected.kind.ogr() != actual_type {
                        return Err(PlenoraIoError::capability_redatta(
                            "filegdb",
                            None,
                            CapabilityReason::TypeNotRepresentable,
                            &PublicMessage::Curated(
                                "GDAL ha riclassificato il tipo OGR del campo; scrittura rifiutata",
                            ),
                        ));
                    }
                    if expected.width.is_some_and(|width| width != actual_width)
                        || expected
                            .precision
                            .is_some_and(|precision| precision != actual_precision)
                    {
                        return Err(PlenoraIoError::capability_redatta(
                            "filegdb",
                            None,
                            CapabilityReason::TypeNotRepresentable,
                            &PublicMessage::Curated(
                                "GDAL ha normalizzato width o precision del campo",
                            ),
                        ));
                    }
                }
            }
            Ok(())
        })();
        if let Err(error) = layer_result {
            drop(ds);
            return Err(error);
        }

        Ok(Box::new(GdbWriter {
            ds: Some(ds),
            staging,
            dest: path.to_owned(),
            durable: opts.durable,
            max_output_bytes: opts.max_output_bytes(),
            layers: infos,
        }))
    }

    impl FormatWriter for GdbWriter {
        fn write(&mut self, batch: &RecordBatch) -> Result<()> {
            self.write_to_layer(LayerId(0), batch)
        }

        fn write_to_layer(&mut self, layer: LayerId, batch: &RecordBatch) -> Result<()> {
            let li = layer.0 as usize;
            if li >= self.layers.len() {
                return Err(err(&PublicMessage::CuratedWith(
                    "layer inesistente, indice",
                    NumeroStrutturale::Indice(u64::from(layer.0)),
                )));
            }
            let geom_idx = self.layers[li].geom_idx;
            let geom_col = batch
                .column(geom_idx)
                .as_any()
                .downcast_ref::<BinaryArray>()
                .ok_or_else(|| err(&PublicMessage::Curated("colonna geometria non binaria")))?;

            let gidx = self.layers[li].gdal_idx;
            let fields = self.layers[li].fields.clone();
            let gl = self
                .ds
                .as_ref()
                .ok_or_else(|| err(&PublicMessage::Curated("dataset writer già chiuso")))?
                .layer(gidx)
                .map_err(|_| {
                    err(&PublicMessage::CuratedWith(
                        "accesso al layer GDAL fallito, indice",
                        NumeroStrutturale::Indice(driver_common::saturating_u64(gidx)),
                    ))
                })?;
            for row in 0..batch.num_rows() {
                let mut feature = Feature::new(gl.defn())
                    .map_err(|_| err(&PublicMessage::Curated("creazione della feature fallita")))?;
                if !geom_col.is_null(row) {
                    let geometry = Geometry::from_wkb(geom_col.value(row)).map_err(|_| {
                        err(&PublicMessage::Curated(
                            "conversione WKB verso GDAL fallita",
                        ))
                    })?;
                    feature.set_geometry(geometry).map_err(|_| {
                        err(&PublicMessage::Curated(
                            "assegnazione della geometria alla feature fallita",
                        ))
                    })?;
                }
                for field in &fields {
                    match field_value(field.kind, batch.column(field.index), row)? {
                        Some(value) => feature.set_field(&field.name, &value).map_err(|_| {
                            err(&PublicMessage::Curated("scrittura di un campo fallita"))
                        })?,
                        None => feature.set_field_null(&field.name).map_err(|_| {
                            err(&PublicMessage::Curated(
                                "scrittura di un campo nullo fallita",
                            ))
                        })?,
                    }
                }
                feature
                    .create(&gl)
                    .map_err(|_| err(&PublicMessage::Curated("scrittura della feature fallita")))?;
            }
            #[cfg(test)]
            crash_failpoint("after_write");
            Ok(())
        }

        fn finish(mut self: Box<Self>) -> Result<Published> {
            let ds = self
                .ds
                .take()
                .ok_or_else(|| err(&PublicMessage::Curated("dataset writer già chiuso")))?;
            drop(ds); // chiude e flush della .gdb
            let bytes = dir_size(self.staging.path());
            if bytes > self.max_output_bytes {
                return Err(PlenoraIoError::limite_redatto(
                    &PublicMessage::CuratedBetween(
                        "output FileGDB da",
                        NumeroStrutturale::Conteggio(bytes),
                        "byte oltre il limite di",
                        NumeroStrutturale::Limite(self.max_output_bytes),
                    ),
                ));
            }
            #[cfg(test)]
            crash_failpoint("before_publish");
            let outcome = publish_dir_atomic(self.staging.path(), &self.dest, self.durable)?;
            #[cfg(test)]
            crash_failpoint("after_publish");
            self.staging.disarm();
            Ok(Published {
                bytes,
                loss: LossReport::default(),
                fidelity: plenora_io_core::FidelityAssessment::lossless(),
                outcome,
            })
        }
    }

    const GEOMETRY: &str = "geometry";

    // Visibile alla crate perche' la superficie del fuzz target, che sta fuori
    // da questo modulo, deve costruire i propri errori con **lo stesso**
    // costruttore: due costruttori diversi darebbero due quartetti diversi per
    // lo stesso genere di guasto.
    pub fn err(reason: &PublicMessage) -> PlenoraIoError {
        PlenoraIoError::formato_redatto("filegdb", reason)
    }

    fn authority_id(spatial_ref: &SpatialRef) -> Option<String> {
        match (spatial_ref.auth_name(), spatial_ref.auth_code()) {
            (Ok(authority), Ok(code)) => Some(format!("{}:{code}", authority.to_ascii_uppercase())),
            _ => None,
        }
    }

    fn crs_kind(spatial_ref: &SpatialRef) -> CrsKind {
        if spatial_ref.is_geographic() {
            CrsKind::Geographic
        } else if spatial_ref.is_projected() {
            CrsKind::Projected
        } else {
            CrsKind::Unknown
        }
    }

    fn has_any(value: &str, needles: &[&str]) -> bool {
        let value = value.to_ascii_lowercase();
        needles.iter().any(|needle| value.contains(needle))
    }

    fn declared_axis_order(spatial_ref: &SpatialRef, kind: CrsKind) -> AxisOrder {
        let target = match kind {
            CrsKind::Geographic => "GEOGCS",
            CrsKind::Projected => "PROJCS",
            CrsKind::Unknown => return AxisOrder::Unknown,
        };
        let Ok(first) = spatial_ref.axis_name(target, 0) else {
            return AxisOrder::Unknown;
        };
        let Ok(second) = spatial_ref.axis_name(target, 1) else {
            return AxisOrder::Unknown;
        };
        match kind {
            CrsKind::Geographic
                if has_any(&first, &["longitude", "lon"])
                    && has_any(&second, &["latitude", "lat"]) =>
            {
                AxisOrder::LongitudeLatitude
            }
            CrsKind::Geographic
                if has_any(&first, &["latitude", "lat"])
                    && has_any(&second, &["longitude", "lon"]) =>
            {
                AxisOrder::LatitudeLongitude
            }
            CrsKind::Projected
                if has_any(&first, &["easting", "east"])
                    && has_any(&second, &["northing", "north"]) =>
            {
                AxisOrder::EastingNorthing
            }
            CrsKind::Geographic | CrsKind::Projected | CrsKind::Unknown => AxisOrder::Unknown,
        }
    }

    fn resolve_layer_crs(
        embedded: Option<SpatialRef>,
        assume_crs: Option<&str>,
    ) -> Result<ResolvedCrs> {
        let spatial_ref = if let Some(spatial_ref) = embedded {
            spatial_ref
        } else {
            let definition = assume_crs.ok_or_else(|| {
                PlenoraIoError::crs_redatto(&PublicMessage::Curated(
                    "FileGDB con geometria senza CRS: fornire --assume-crs",
                ))
            })?;
            SpatialRef::from_definition(definition).map_err(|_| {
                PlenoraIoError::crs_non_risolto_redatto(
                    "filegdb",
                    &RawCrs::new(definition.to_owned(), Some(definition.to_owned())),
                )
            })?
        };
        let definition = spatial_ref.to_wkt().map_err(|_| {
            PlenoraIoError::crs_non_risolto_redatto(
                "filegdb",
                &RawCrs::new(
                    "SpatialRef GDAL presente ma WKT non esportabile".to_owned(),
                    authority_id(&spatial_ref),
                ),
            )
        })?;
        let id = authority_id(&spatial_ref);
        let kind = crs_kind(&spatial_ref);
        let mut resolved = ResolvedCrs::new(id, kind, Some(definition));
        let declared_axis_order = declared_axis_order(&spatial_ref, kind);
        if declared_axis_order != AxisOrder::Unknown {
            resolved.axis_order = declared_axis_order;
        }
        Ok(resolved)
    }

    fn geometry_contract_from_ogr(
        ogr_type: OGRwkbGeometryType::Type,
        crs: ResolvedCrs,
    ) -> GeometryColumnContract {
        let raw = ogr_type;
        let without_25d = raw & !0x8000_0000;
        let dimension_code = without_25d / 1000;
        let dimensions = match (
            raw & 0x8000_0000 != 0 || matches!(dimension_code, 1 | 3),
            matches!(dimension_code, 2 | 3),
        ) {
            (false, false) => CoordinateDimensions::Xy,
            (true, false) => CoordinateDimensions::Xyz,
            (false, true) => CoordinateDimensions::Xym,
            (true, true) => CoordinateDimensions::Xyzm,
        };
        let geometry_types = match without_25d % 1000 {
            1 => vec![GeometryType::Point],
            2 => vec![GeometryType::LineString],
            3 => vec![GeometryType::Polygon],
            4 => vec![GeometryType::MultiPoint],
            5 => vec![GeometryType::MultiLineString],
            6 => vec![GeometryType::MultiPolygon],
            7 => vec![GeometryType::GeometryCollection],
            _ => Vec::new(),
        };
        let mut geometry = GeometryColumnContract::wkb_xy(FieldId(0), GEOMETRY, crs, true);
        geometry.dimensions = if geometry_types.is_empty() {
            CoordinateDimensions::Unknown
        } else {
            dimensions
        };
        geometry.set_exact_geometry_types(geometry_types);
        geometry
            .native_metadata
            .insert("filegdb.ogr_geometry_type".to_owned(), raw.to_string());
        geometry
    }

    pub fn open(
        path: &std::path::Path,
        assume_crs: Option<&str>,
    ) -> Result<Box<dyn OpenDatasetHandle>> {
        // Schema dai def GDAL, SENZA leggere feature (poi il reader streamma).
        let ds = Dataset::open(path)
            .map_err(|_| err(&PublicMessage::Curated("apertura GDAL fallita")))?;
        let mut layers = Vec::new();
        let mut metas = Vec::new();
        for (i, layer) in ds.layers().enumerate() {
            // Il campo geometrico si cerca **per primo**, e la sua assenza non
            // e' un errore.
            //
            // Un FileGDB e' fatto di feature class **e** di tabelle: le seconde
            // non hanno geometria, il formato le ammette, ed e' cosi' che sono
            // fatte le GDB reali. Il driver le trattava come un layer rotto e
            // rifiutava l'intero dataset, in fase di apertura: quattro feature
            // class valide diventavano inaccessibili perche' accanto a loro
            // c'era una tabella, e nessun `--layer` poteva aggirarlo, perche'
            // il rifiuto precedeva la scelta.
            //
            // Una tabella viene percio' **enumerata**, con `geometry: None`.
            // Non saltata: saltarla sposterebbe gli indici di layer sotto i
            // piedi di chi li ha letti dal catalogo, e `--layer 1`
            // significherebbe due cose diverse a seconda di che cosa il driver
            // ha deciso di nascondere.
            let ogr_geometry_type = layer
                .defn()
                .geom_fields()
                .next()
                .map(|field| field.field_type());
            // Il CRS si risolve solo per chi ha una geometria: pretenderlo da
            // una tabella significava chiedere un `--assume-crs` per dei dati
            // che non sono spaziali, con un messaggio che parlava di geometria.
            let crs_e_geometria = match ogr_geometry_type {
                Some(ogr_geometry_type) => {
                    let crs = resolve_layer_crs(layer.spatial_ref(), assume_crs)?;
                    let crs_label = crs
                        .id
                        .as_deref()
                        .or(crs.definition.as_deref())
                        .ok_or_else(|| {
                            PlenoraIoError::crs_redatto(&PublicMessage::Curated(
                                "CRS FileGDB risolto senza identificativo né definizione",
                            ))
                        })?
                        .to_owned();
                    Some((
                        crs_label,
                        geometry_contract_from_ogr(ogr_geometry_type, crs),
                    ))
                }
                None => None,
            };
            let native_fields: Vec<(LayerFieldMeta, Field)> = layer
                .defn()
                .fields()
                .map(|field| {
                    let name = field.name();
                    let field_type = field.field_type();
                    ogr_to_arrow(field_type).map(|data_type| {
                        let metadata = HashMap::from([
                            (OGR_FIELD_TYPE_KEY.to_owned(), field_type.to_string()),
                            (OGR_FIELD_WIDTH_KEY.to_owned(), field.width().to_string()),
                            (
                                OGR_FIELD_PRECISION_KEY.to_owned(),
                                field.precision().to_string(),
                            ),
                        ]);
                        let arrow_field =
                            Field::new(&name, data_type.clone(), true).with_metadata(metadata);
                        (
                            LayerFieldMeta {
                                name,
                                data_type,
                                ogr_type: field_type,
                            },
                            arrow_field,
                        )
                    })
                })
                .collect::<Result<Vec<_>>>()?;
            let (fields, attribute_arrow_fields): (Vec<_>, Vec<_>) =
                native_fields.into_iter().unzip();
            let (arrow_fields, geometry) = match crs_e_geometria {
                Some((crs_label, geometry)) => {
                    let geometry_arrow_field = with_geometry_contract_metadata(
                        &geometry_field(GEOMETRY, &crs_label),
                        &geometry,
                    );
                    let mut campi = vec![geometry_arrow_field];
                    campi.extend(attribute_arrow_fields);
                    (campi, Some(geometry))
                }
                // Una tabella: gli attributi e basta, senza una colonna
                // geometrica davanti. E' anche cio' che sposta gli indici, e il
                // reader lo rilegge da `ha_geometria`.
                None => (attribute_arrow_fields, None),
            };
            let schema: SchemaRef = Arc::new(Schema::new(arrow_fields));
            let ha_geometria = geometry.is_some();
            let contract = DataContract::new(schema.clone(), geometry);
            // Indice di layer di un FileGDB: il formato ne ammette al piu'
            // qualche migliaio, il cast a u32 non puo' troncare.
            #[allow(clippy::cast_possible_truncation)]
            let layer_id = LayerId(i as u32);
            layers.push(LayerContract {
                id: layer_id,
                name: layer.name(),
                contract,
            });
            metas.push(LayerMeta {
                gdal_idx: i,
                fields,
                ha_geometria,
            });
        }
        Ok(Box::new(GdbDataset {
            path: path.to_owned(),
            layers,
            metas,
            reader_gate: SingleReaderGate::new(DESCRIPTOR.id()),
        }))
    }

    fn ogr_to_arrow(ft: gdal::vector::OGRFieldType::Type) -> Result<DataType> {
        use gdal::vector::OGRFieldType;
        if ft == OGRFieldType::OFTInteger {
            Ok(DataType::Int32)
        } else if ft == OGRFieldType::OFTInteger64 {
            Ok(DataType::Int64)
        } else if ft == OGRFieldType::OFTReal {
            Ok(DataType::Float64)
        } else if ft == OGRFieldType::OFTString || ft == OGRFieldType::OFTWideString {
            Ok(DataType::Utf8)
        } else {
            // Ne' il nome ne' il codice del tipo OGR escono: entrambi vengono
            // dallo schema che GDAL ha letto dal file.
            Err(PlenoraIoError::capability_redatta(
                "filegdb",
                None,
                CapabilityReason::TypeNotRepresentable,
                &PublicMessage::Curated(
                    "tipo campo OGR non ancora rappresentato senza perdita nel bordo Arrow",
                ),
            ))
        }
    }

    struct LayerFieldMeta {
        name: String,
        data_type: DataType,
        ogr_type: gdal::vector::OGRFieldType::Type,
    }

    struct ProjectedField {
        ogr_index: i32,
        name: String,
        data_type: DataType,
        ogr_type: gdal::vector::OGRFieldType::Type,
    }

    /// Che cosa il reader deve sapere della geometria di **questo** layer.
    ///
    /// Due booleani distinti e non uno: `presente` dice se il layer ha una
    /// colonna geometrica -- una tabella non ne ha -- e `inclusa` se la
    /// projection l'ha chiesta. Confonderli farebbe ignorare `OGR_GEOMETRY` su
    /// un layer che quel campo non ha.
    #[derive(Clone, Copy)]
    struct GeometriaDelLayer {
        presente: bool,
        inclusa: bool,
    }

    struct LayerMeta {
        gdal_idx: usize,
        fields: Vec<LayerFieldMeta>,
        /// Il layer porta una colonna geometrica.
        ///
        /// Non e' una comodita': decide **lo scostamento** fra indice Arrow e
        /// indice OGR. In una feature class la geometria sta in posizione zero
        /// e gli attributi cominciano da uno; in una tabella cominciano da
        /// zero. Il reader sottraeva sempre uno, e su una tabella avrebbe
        /// letto il campo sbagliato -- o nessuno.
        ha_geometria: bool,
    }

    struct GdbDataset {
        path: PathBuf,
        layers: Vec<LayerContract>,
        metas: Vec<LayerMeta>,
        reader_gate: SingleReaderGate,
    }

    impl OpenDatasetHandle for GdbDataset {
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
            let idx = self
                .layers
                .iter()
                .position(|l| l.id.0 == request.layer.0)
                .ok_or_else(|| {
                    err(&PublicMessage::CuratedWith(
                        "layer runtime inesistente, indice",
                        NumeroStrutturale::Indice(u64::from(request.layer.0)),
                    ))
                })?;
            let m = &self.metas[idx];
            let (indices, layer_contract) =
                plenora_io_core::project_layer_contract(&self.layers[idx], request)?;
            // In una feature class l'indice Arrow zero e' la geometria e gli
            // attributi sono spostati di uno; in una tabella non c'e' niente
            // davanti e lo scostamento e' zero.
            let scostamento = usize::from(m.ha_geometria);
            let include_geometry = m.ha_geometria && indices.binary_search(&0).is_ok();
            let mut fields = Vec::with_capacity(indices.len());
            for &index in &indices {
                let Some(field_index) = index.checked_sub(scostamento) else {
                    continue;
                };
                let field = m.fields.get(field_index).ok_or_else(|| {
                    err(&PublicMessage::CuratedWith(
                        "indice Arrow non presente nello schema FileGDB, indice",
                        NumeroStrutturale::Indice(driver_common::saturating_u64(index)),
                    ))
                })?;
                let ogr_index = i32::try_from(field_index).map_err(|_| {
                    err(&PublicMessage::CuratedWith(
                        "indice OGR fuori intervallo i32, indice",
                        NumeroStrutturale::Indice(driver_common::saturating_u64(field_index)),
                    ))
                })?;
                fields.push(ProjectedField {
                    ogr_index,
                    name: field.name.clone(),
                    data_type: field.data_type.clone(),
                    ogr_type: field.ogr_type,
                });
            }
            let batch_sizer = plenora_io_core::AdaptiveBatchSizer::new(
                layer_contract.contract.schema.as_ref(),
                request.batch_target,
            );
            let reader = self.reader_gate.open(request.layer, || {
                spawn_reader(
                    self.path.clone(),
                    m.gdal_idx,
                    layer_contract.contract.schema.clone(),
                    fields,
                    GeometriaDelLayer {
                        presente: m.ha_geometria,
                        inclusa: include_geometry,
                    },
                    batch_sizer,
                    layer_contract,
                )
            })?;
            Ok(plenora_io_core::with_cancellation(
                reader,
                request.cancellation.clone(),
            ))
        }
    }

    /// Il thread apre il PROPRIO Dataset GDAL (non-Send, quindi mai attraversa il
    /// confine) e scorre le feature in batch, consegnati via canale.
    /// Verifica che lo schema non sia cambiato fra `open` e l'avvio del worker.
    ///
    /// Il worker riapre il dataset: prima di usare gli indici OGR pre-risolti
    /// controlla che nessun processo abbia cambiato lo schema nel frattempo. Un
    /// mismatch fallisce **chiuso** invece di convertire silenziosamente il
    /// campo sbagliato.
    ///
    /// Il nome del campo non entra nel messaggio: viene dallo schema che GDAL
    /// ha letto dal file. Esce l'indice, che e' nostro.
    fn verifica_schema_invariato(
        actual_fields: &[(String, gdal::vector::OGRFieldType::Type)],
        fields: &[ProjectedField],
    ) -> Result<()> {
        for field in fields {
            let index = usize::try_from(field.ogr_index)
                .map_err(|_| err(&PublicMessage::Curated("indice OGR negativo")))?;
            let actual = actual_fields.get(index);
            if !matches!(
                actual,
                Some((name, ogr_type)) if name == &field.name && *ogr_type == field.ogr_type
            ) {
                return Err(err(&PublicMessage::CuratedWith(
                    "schema FileGDB cambiato fra apertura e lettura, campo di indice",
                    NumeroStrutturale::Indice(driver_common::saturating_u64(index)),
                )));
            }
        }
        Ok(())
    }

    fn spawn_reader(
        path: PathBuf,
        gdal_idx: usize,
        schema: SchemaRef,
        fields: Vec<ProjectedField>,
        geometria: GeometriaDelLayer,
        mut batch_sizer: plenora_io_core::AdaptiveBatchSizer,
        contract: LayerContract,
    ) -> Result<Box<dyn LayerReader>> {
        spawn_batch_reader(
            DESCRIPTOR.id(),
            contract,
            2,
            move |emitter: BatchEmitter| {
                let ds = Dataset::open(&path)
                    .map_err(|_| err(&PublicMessage::Curated("apertura del FileGDB fallita")))?;
                let mut layer = ds.layer(gdal_idx).map_err(|_| {
                    err(&PublicMessage::Curated(
                        "apertura del layer FileGDB fallita",
                    ))
                })?;
                // Il worker riapre il dataset: prima di usare gli indici OGR
                // pre-risolti verifica che nessuno abbia cambiato lo schema.
                let actual_fields: Vec<_> = layer
                    .defn()
                    .fields()
                    .map(|field| (field.name(), field.field_type()))
                    .collect();
                verifica_schema_invariato(&actual_fields, &fields)?;
                let selected_fields = fields
                    .iter()
                    .map(|field| usize::try_from(field.ogr_index))
                    .collect::<std::result::Result<HashSet<_>, _>>()
                    .map_err(|_| {
                        err(&PublicMessage::Curated(
                            "indice OGR negativo nella projection FileGDB",
                        ))
                    })?;
                let mut ignored_fields = actual_fields
                    .iter()
                    .enumerate()
                    .filter(|(index, _)| !selected_fields.contains(index))
                    .map(|(_, (name, _))| name.as_str())
                    .collect::<Vec<_>>();
                // `OGR_GEOMETRY` si ignora solo dove esiste: su una tabella
                // non c'e' campo geometrico da nominare, e nominarlo sarebbe
                // una projection su un campo che il layer non ha.
                if geometria.presente && !geometria.inclusa {
                    ignored_fields.push("OGR_GEOMETRY");
                }
                layer.set_ignored_fields(&ignored_fields).map_err(|_| {
                    err(&PublicMessage::Curated(
                        "projection fisica FileGDB non applicabile",
                    ))
                })?;
                let mut geom = geometria.inclusa.then(BinaryBuilder::new);
                let mut builders: Vec<ReadCol> = fields
                    .iter()
                    .map(|field| ReadCol::new(&field.data_type))
                    .collect();
                let mut n = 0usize;
                for feature in layer.features() {
                    if let Some(builder) = &mut geom {
                        match feature.geometry_by_index(0) {
                            Ok(geometry) => {
                                let bytes = geometry.wkb().map_err(|_| {
                                    err(&PublicMessage::Curated(
                                        "conversione della geometria FileGDB in WKB fallita",
                                    ))
                                })?;
                                builder.append_value(&bytes);
                            }
                            Err(GdalError::NullPointer {
                                method_name: "OGR_F_GetGeomFieldRef",
                                ..
                            }) => {
                                builder.append_null();
                            }
                            Err(_) => {
                                return Err(err(&PublicMessage::Curated(
                                    "lettura della geometria FileGDB fallita",
                                )))
                            }
                        }
                    }
                    for (builder, field) in builders.iter_mut().zip(&fields) {
                        builder.append_feature(&feature, field)?;
                    }
                    n += 1;
                    if n >= batch_sizer.rows() {
                        let batch = finish_read_batch(&schema, &mut geom, &mut builders, n)?;
                        batch_sizer.observe(&batch);
                        if !emitter.send(batch) {
                            return Ok(());
                        }
                        n = 0;
                    }
                }
                if n > 0 {
                    let batch = finish_read_batch(&schema, &mut geom, &mut builders, n)?;
                    if !emitter.send(batch) {
                        return Ok(());
                    }
                }
                Ok(())
            },
        )
    }

    fn finish_read_batch(
        schema: &SchemaRef,
        geom: &mut Option<BinaryBuilder>,
        builders: &mut [ReadCol],
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

    enum ReadCol {
        I32(Int32Builder),
        I64(Int64Builder),
        F64(Float64Builder),
        Str(StringBuilder),
    }

    impl ReadCol {
        fn new(dt: &DataType) -> Self {
            match dt {
                DataType::Int32 => Self::I32(Int32Builder::new()),
                DataType::Int64 => Self::I64(Int64Builder::new()),
                DataType::Float64 => Self::F64(Float64Builder::new()),
                _ => Self::Str(StringBuilder::new()),
            }
        }
        fn append_feature(&mut self, feature: &Feature<'_>, field: &ProjectedField) -> Result<()> {
            // Ne' il nome del campo ne' il testo di GDAL escono: il primo
            // viene dallo schema letto dal file, il secondo dalla dipendenza.
            let read_error = |_| {
                err(&PublicMessage::CuratedWith(
                    "lettura di un campo FileGDB fallita, indice OGR",
                    // `unsigned_abs` rende la conversione **totale**: nessun ramo
                    // di riserva, quindi niente da registrare come fallback.
                    // L'indice OGR e' gia' verificato non negativo a monte.
                    //
                    // (Il commento evita di nominare la forma alternativa: il
                    // registro dei fallback conta il testo, e citarla qui la
                    // farebbe contare come se ci fosse davvero.)
                    NumeroStrutturale::Indice(u64::from(field.ogr_index.unsigned_abs())),
                ))
            };
            match self {
                Self::I32(builder) => {
                    match feature
                        .field_as_integer(field.ogr_index)
                        .map_err(read_error)?
                    {
                        Some(value) => builder.append_value(value),
                        None => builder.append_null(),
                    }
                }
                Self::I64(builder) => {
                    match feature
                        .field_as_integer64(field.ogr_index)
                        .map_err(read_error)?
                    {
                        Some(value) => builder.append_value(value),
                        None => builder.append_null(),
                    }
                }
                Self::F64(builder) => {
                    match feature
                        .field_as_double(field.ogr_index)
                        .map_err(read_error)?
                    {
                        Some(value) => builder.append_value(value),
                        None => builder.append_null(),
                    }
                }
                Self::Str(builder) => {
                    match feature
                        .field_as_string(field.ogr_index)
                        .map_err(read_error)?
                    {
                        Some(value) => builder.append_value(value),
                        None => builder.append_null(),
                    }
                }
            }
            Ok(())
        }
        fn finish(&mut self) -> ArrayRef {
            match self {
                Self::I32(b) => Arc::new(b.finish()),
                Self::I64(b) => Arc::new(b.finish()),
                Self::F64(b) => Arc::new(b.finish()),
                Self::Str(b) => Arc::new(b.finish()),
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use arrow_array::RecordBatch;
        use plenora_io_core::descriptor::Fidelity;
        use plenora_io_core::driver::{FormatDriver, ReadOptions, Sink, Source, WriteOptions};
        use plenora_io_core::request::{BatchTarget, ProjectionMode, ReadScope};
        use plenora_io_model::contract::{GeometryEncoding, GeometryType};
        use plenora_io_model::limits::WkbLimits;

        /// Opzioni di scrittura sul modello unificato (S4.e).
        fn opzioni_scrittura() -> WriteOptions {
            match plenora_io_model::budget::PipelineBudget::builder().build() {
                Ok(bundle) => WriteOptions::from_write_parts(bundle.into_write_parts()),
                Err(error) => unreachable!("bundle di test non costruibile: {error:?}"),
            }
        }

        /// Opzioni di lettura sul modello unificato (S4.d).
        fn opzioni_lettura() -> ReadOptions {
            match plenora_io_model::budget::PipelineBudget::builder().build() {
                Ok(bundle) => ReadOptions::from_read_parts(bundle.into_read_parts()),
                Err(error) => unreachable!("bundle di test non costruibile: {error:?}"),
            }
        }
        use plenora_io_model::wkb::{
            decode_wkb, encode_wkb, WkbCoordinate, WkbFlavor, WkbGeometry, WkbValue,
        };
        use plenora_io_model::CancellationToken;
        use std::process::{Child, Command, ExitStatus};
        use std::time::{Duration, Instant};

        fn read_request() -> ReadRequest {
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

        fn write_layer(crs: ResolvedCrs) -> WriteLayer {
            let schema: SchemaRef = Arc::new(Schema::new(vec![geometry_field(
                GEOMETRY,
                crs.id.as_deref().unwrap_or("custom"),
            )]));
            let mut geometry = GeometryColumnContract::wkb_xy(FieldId(0), GEOMETRY, crs, true);
            geometry.set_exact_geometry_types(vec![GeometryType::Point]);
            WriteLayer {
                name: "points".to_owned(),
                contract: DataContract {
                    schema,
                    geometry: Some(geometry),
                },
            }
        }

        fn point_wkb(dimensions: CoordinateDimensions, z: Option<f64>, m: Option<f64>) -> Vec<u8> {
            encode_wkb(
                &WkbGeometry {
                    value: WkbValue::Point(WkbCoordinate {
                        x: 10.5,
                        y: 20.25,
                        z,
                        m,
                    }),
                    dimensions,
                    srid: None,
                },
                WkbFlavor::Iso,
            )
            .unwrap()
        }

        fn point_write_fixture() -> (WritePlan, RecordBatch) {
            let layer = write_layer(ResolvedCrs::new(
                Some("EPSG:3857".to_owned()),
                CrsKind::Projected,
                None,
            ));
            let geometry = point_wkb(CoordinateDimensions::Xy, None, None);
            let batch = RecordBatch::try_new(
                layer.contract.schema.clone(),
                vec![Arc::new(BinaryArray::from(vec![Some(geometry.as_slice())]))],
            )
            .unwrap();
            (
                WritePlan {
                    layers: vec![layer],
                },
                batch,
            )
        }

        fn assert_complete_point_dataset(path: PathBuf) {
            let dataset = super::super::FileGdbDriver
                .open(Source::Path(path), opzioni_lettura())
                .unwrap();
            let mut reader = dataset.open_layer_reader(&read_request()).unwrap();
            assert_eq!(reader.next_batch().unwrap().unwrap().num_rows(), 1);
            assert!(reader.next_batch().unwrap().is_none());
        }

        fn run_crash_subprocess(dest: &Path, point: &str) -> ExitStatus {
            Command::new(std::env::current_exe().unwrap())
                .args([
                    "--ignored",
                    "--exact",
                    "backend::tests::filegdb_crash_subprocess_helper",
                    "--nocapture",
                    "--test-threads=1",
                ])
                .env("PLENORA_FILEGDB_CRASH_DEST", dest)
                .env("PLENORA_FILEGDB_CRASH_POINT", point)
                .env("RUST_BACKTRACE", "0")
                .status()
                .unwrap()
        }

        fn spawn_active_subprocess(dest: &Path, ready: &Path, release: &Path) -> Child {
            Command::new(std::env::current_exe().unwrap())
                .args([
                    "--ignored",
                    "--exact",
                    "backend::tests::filegdb_active_subprocess_helper",
                    "--nocapture",
                    "--test-threads=1",
                ])
                .env("PLENORA_FILEGDB_ACTIVE_DEST", dest)
                .env("PLENORA_FILEGDB_ACTIVE_READY", ready)
                .env("PLENORA_FILEGDB_ACTIVE_RELEASE", release)
                .env("RUST_BACKTRACE", "0")
                .spawn()
                .unwrap()
        }

        fn wait_until_ready(child: &mut Child, ready: &Path) {
            let deadline = Instant::now() + Duration::from_secs(10);
            while Instant::now() < deadline {
                if ready.exists() {
                    return;
                }
                if let Some(status) = child.try_wait().unwrap() {
                    panic!("il writer attivo è terminato prematuramente: {status}");
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("timeout in attesa del writer attivo");
        }

        #[test]
        fn gdal_reports_authority_axis_order_without_canonicalization() {
            let epsg = resolve_layer_crs(
                Some(SpatialRef::from_definition("EPSG:4326").unwrap()),
                None,
            )
            .unwrap();
            let crs84 = resolve_layer_crs(
                Some(SpatialRef::from_definition("OGC:CRS84").unwrap()),
                None,
            )
            .unwrap();
            let projected = resolve_layer_crs(
                Some(SpatialRef::from_definition("EPSG:3857").unwrap()),
                None,
            )
            .unwrap();

            assert_eq!(epsg.axis_order, AxisOrder::LatitudeLongitude);
            assert_eq!(crs84.axis_order, AxisOrder::LongitudeLatitude);
            assert_ne!(crs84.id.as_deref(), Some("EPSG:4326"));
            assert_eq!(projected.axis_order, AxisOrder::EastingNorthing);

            let write_crs84 = resolve_layer_crs(
                Some(layer_spatial_ref(&write_layer(ResolvedCrs::wgs84())).unwrap()),
                None,
            )
            .unwrap();
            assert_eq!(write_crs84.axis_order, AxisOrder::LongitudeLatitude);
            assert_ne!(write_crs84.id.as_deref(), Some("EPSG:4326"));
        }

        #[test]
        fn conflicting_id_and_wkt_fail_before_output_creation() {
            let epsg_4326_wkt = SpatialRef::from_definition("EPSG:4326")
                .unwrap()
                .to_wkt()
                .unwrap();
            let layer = write_layer(ResolvedCrs::new(
                Some("EPSG:3857".to_owned()),
                CrsKind::Projected,
                Some(epsg_4326_wkt),
            ));
            assert!(matches!(
                layer_spatial_ref(&layer),
                Err(error) if error.code == plenora_io_model::IoErrorCode::CrsUnresolved
            ));
        }

        #[test]
        fn missing_crs_requires_a_valid_explicit_assumption() {
            assert!(matches!(
                resolve_layer_crs(None, None),
                Err(error) if error.code == plenora_io_model::IoErrorCode::Crs
            ));
            assert!(matches!(
                resolve_layer_crs(None, Some("not-a-crs-secret")),
                Err(error) if error.code == plenora_io_model::IoErrorCode::CrsUnresolved
            ));
            let assumed = resolve_layer_crs(None, Some("EPSG:3857")).unwrap();
            assert_eq!(assumed.id.as_deref(), Some("EPSG:3857"));
            assert_eq!(assumed.axis_order, AxisOrder::EastingNorthing);
        }

        #[test]
        fn filegdb_round_trip_preserves_crs_and_enforces_single_reader() {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("round-trip.gdb");
            let wkb = encode_wkb(
                &WkbGeometry {
                    value: WkbValue::Point(WkbCoordinate {
                        x: 1_113_194.0,
                        y: 5_621_521.0,
                        z: None,
                        m: None,
                    }),
                    dimensions: plenora_io_model::contract::CoordinateDimensions::Xy,
                    srid: None,
                },
                WkbFlavor::Iso,
            )
            .unwrap();
            let schema: SchemaRef =
                Arc::new(Schema::new(vec![geometry_field(GEOMETRY, "EPSG:3857")]));
            let batch = RecordBatch::try_new(
                schema.clone(),
                vec![Arc::new(BinaryArray::from(vec![Some(wkb.as_slice())]))],
            )
            .unwrap();
            let mut geometry = GeometryColumnContract::wkb_xy(
                FieldId(0),
                GEOMETRY,
                ResolvedCrs::new(Some("EPSG:3857".to_owned()), CrsKind::Projected, None),
                true,
            );
            geometry.set_exact_geometry_types(vec![GeometryType::Point]);
            let plan = WritePlan {
                layers: vec![WriteLayer {
                    name: "points".to_owned(),
                    contract: DataContract {
                        schema,
                        geometry: Some(geometry),
                    },
                }],
            };

            let driver = super::super::FileGdbDriver;
            let mut writer = driver
                .create(Sink::Path(path.clone()), &plan, &opzioni_scrittura())
                .unwrap();
            writer.write(&batch).unwrap();
            writer.finish().unwrap();

            let dataset = driver.open(Source::Path(path), opzioni_lettura()).unwrap();
            let crs = dataset.layers()[0]
                .contract
                .geometry
                .as_ref()
                .unwrap()
                .resolved_crs()
                .unwrap();
            assert_eq!(crs.id.as_deref(), Some("EPSG:3857"));
            assert_eq!(crs.axis_order, AxisOrder::EastingNorthing);
            assert!(crs.definition.is_some());

            let first = dataset.open_layer_reader(&read_request()).unwrap();
            assert!(matches!(
                dataset.open_layer_reader(&read_request()),
                Err(error)
                    if error.code == plenora_io_model::IoErrorCode::ReaderBusy
                        && error.driver.as_deref() == Some("filegdb")
            ));
            drop(first);
            assert!(dataset.open_layer_reader(&read_request()).is_ok());
        }

        #[test]
        fn repeated_filegdb_writes_are_semantically_deterministic() {
            let dir = tempfile::tempdir().unwrap();
            let paths = [
                dir.path().join("determinism-a.gdb"),
                dir.path().join("determinism-b.gdb"),
            ];
            let wkb = encode_wkb(
                &WkbGeometry {
                    value: WkbValue::Point(WkbCoordinate {
                        x: 1_113_194.0,
                        y: 5_621_521.0,
                        z: None,
                        m: None,
                    }),
                    dimensions: CoordinateDimensions::Xy,
                    srid: None,
                },
                WkbFlavor::Iso,
            )
            .unwrap();
            let schema: SchemaRef =
                Arc::new(Schema::new(vec![geometry_field(GEOMETRY, "EPSG:3857")]));
            let batch = RecordBatch::try_new(
                schema.clone(),
                vec![Arc::new(BinaryArray::from(vec![Some(wkb.as_slice())]))],
            )
            .unwrap();
            let mut geometry = GeometryColumnContract::wkb_xy(
                FieldId(0),
                GEOMETRY,
                ResolvedCrs::new(Some("EPSG:3857".to_owned()), CrsKind::Projected, None),
                true,
            );
            geometry.set_exact_geometry_types(vec![GeometryType::Point]);
            let plan = WritePlan {
                layers: vec![WriteLayer {
                    name: "points".to_owned(),
                    contract: DataContract {
                        schema,
                        geometry: Some(geometry),
                    },
                }],
            };
            let driver = super::super::FileGdbDriver;
            for path in &paths {
                let mut writer = driver
                    .create(Sink::Path(path.clone()), &plan, &opzioni_scrittura())
                    .unwrap();
                writer.write(&batch).unwrap();
                writer.finish().unwrap();
            }

            let semantic_signature = |path: &Path| {
                let dataset = driver
                    .open(Source::Path(path.to_owned()), opzioni_lettura())
                    .unwrap();
                let layer = &dataset.layers()[0];
                let geometry = layer.contract.geometry.as_ref().unwrap();
                let contract_signature = (
                    layer.name.clone(),
                    geometry.dimensions,
                    geometry.geometry_types.clone(),
                    geometry.crs.id().map(str::to_owned),
                    geometry.resolved_crs().map(|crs| crs.axis_order).unwrap(),
                );
                let mut reader = dataset.open_layer_reader(&read_request()).unwrap();
                let batch = reader.next_batch().unwrap().unwrap();
                assert!(reader.next_batch().unwrap().is_none());
                let values = batch
                    .column(0)
                    .as_any()
                    .downcast_ref::<BinaryArray>()
                    .unwrap();
                let decoded = decode_wkb(values.value(0), &WkbLimits::default()).unwrap();
                (contract_signature, decoded)
            };

            assert_eq!(semantic_signature(&paths[0]), semantic_signature(&paths[1]));
        }

        // Il test copre in sequenza scrittura, riletture proiettate e proiezione
        // vuota sullo stesso dataset: spezzarlo perderebbe la condivisione della
        // fixture e la verifica dell'ordine delle proiezioni.
        #[allow(clippy::too_many_lines)]
        #[test]
        fn filegdb_round_trip_preserves_null_geometry_and_exact_attributes() {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("null-and-attributes.gdb");
            let geometry_field = geometry_field(GEOMETRY, "EPSG:3857");
            let native_string_metadata = HashMap::from([
                (
                    OGR_FIELD_TYPE_KEY.to_owned(),
                    gdal::vector::OGRFieldType::OFTString.to_string(),
                ),
                (OGR_FIELD_WIDTH_KEY.to_owned(), "80".to_owned()),
                (OGR_FIELD_PRECISION_KEY.to_owned(), "0".to_owned()),
            ]);
            let schema: SchemaRef = Arc::new(Schema::new(vec![
                geometry_field,
                Field::new("count", DataType::Int32, true),
                Field::new("ratio", DataType::Float64, true),
                Field::new("label", DataType::Utf8, true)
                    .with_metadata(native_string_metadata.clone()),
            ]));
            let wkb = point_wkb(CoordinateDimensions::Xy, None, None);
            let batch = RecordBatch::try_new(
                schema.clone(),
                vec![
                    Arc::new(BinaryArray::from(vec![Some(wkb.as_slice()), None])),
                    Arc::new(Int32Array::from(vec![Some(i32::MAX), None])),
                    Arc::new(Float64Array::from(vec![Some(12.5), None])),
                    Arc::new(StringArray::from(vec![Some("città"), None])),
                ],
            )
            .unwrap();
            let mut geometry = GeometryColumnContract::wkb_xy(
                FieldId(0),
                GEOMETRY,
                ResolvedCrs::new(Some("EPSG:3857".to_owned()), CrsKind::Projected, None),
                true,
            );
            geometry.set_exact_geometry_types(vec![GeometryType::Point]);
            let plan = WritePlan {
                layers: vec![WriteLayer {
                    name: "points".to_owned(),
                    contract: DataContract {
                        schema,
                        geometry: Some(geometry),
                    },
                }],
            };

            let driver = super::super::FileGdbDriver;
            let mut writer = driver
                .create(Sink::Path(path.clone()), &plan, &opzioni_scrittura())
                .unwrap();
            writer.write(&batch).unwrap();
            let published = writer.finish().unwrap();
            assert_eq!(published.fidelity.level, Fidelity::Approximating);
            assert_eq!(
                published.loss.counts.get("crs_id_not_preserved_derived"),
                Some(&1)
            );

            let dataset = driver.open(Source::Path(path), opzioni_lettura()).unwrap();
            let output_geometry = dataset.layers()[0].contract.geometry.as_ref().unwrap();
            assert_eq!(output_geometry.dimensions, CoordinateDimensions::Xy);
            assert_eq!(output_geometry.geometry_types, vec![GeometryType::Point]);
            assert!(output_geometry
                .native_metadata
                .contains_key("filegdb.ogr_geometry_type"));
            assert_eq!(
                dataset.layers()[0].contract.schema.field(3).metadata(),
                &native_string_metadata
            );

            let mut reader = dataset.open_layer_reader(&read_request()).unwrap();
            let output = reader.next_batch().unwrap().unwrap();
            assert_eq!(output.num_rows(), 2);
            let output_geometry = output
                .column(0)
                .as_any()
                .downcast_ref::<BinaryArray>()
                .unwrap();
            assert!(!output_geometry.is_null(0));
            assert!(output_geometry.is_null(1));
            let counts = output
                .column(1)
                .as_any()
                .downcast_ref::<Int32Array>()
                .unwrap();
            assert_eq!(counts.value(0), i32::MAX);
            assert!(counts.is_null(1));
            let ratios = output
                .column(2)
                .as_any()
                .downcast_ref::<Float64Array>()
                .unwrap();
            // Il round-trip deve restituire il valore identico bit a bit: il
            // confronto esatto e' il contratto, non un'approssimazione.
            #[allow(clippy::float_cmp)]
            {
                assert_eq!(ratios.value(0), 12.5);
            }
            assert!(ratios.is_null(1));
            let labels = output
                .column(3)
                .as_any()
                .downcast_ref::<StringArray>()
                .unwrap();
            assert_eq!(labels.value(0), "città");
            assert!(labels.is_null(1));
            assert!(reader.next_batch().unwrap().is_none());
            drop(reader);

            let mut projected = dataset
                .open_layer_reader(&ReadRequest {
                    layer: LayerId(0),
                    projected_fields: Some(vec![FieldId(3)]),
                    projection_mode: ProjectionMode::Required,
                    pruning_predicate: None,
                    spatial_pruning_hint: None,
                    scope: ReadScope::default(),
                    batch_target: BatchTarget::default(),
                    cancellation: CancellationToken::default(),
                })
                .unwrap();
            assert!(projected.contract().contract.geometry.is_none());
            let output = projected.next_batch().unwrap().unwrap();
            assert_eq!(output.num_rows(), 2);
            assert_eq!(output.num_columns(), 1);
            assert_eq!(output.schema().field(0).name(), "label");
            assert!(projected.next_batch().unwrap().is_none());
            drop(projected);

            let mut reversed = dataset
                .open_layer_reader(&ReadRequest {
                    layer: LayerId(0),
                    projected_fields: Some(vec![FieldId(3), FieldId(1)]),
                    projection_mode: ProjectionMode::Required,
                    pruning_predicate: None,
                    spatial_pruning_hint: None,
                    scope: ReadScope::default(),
                    batch_target: BatchTarget::default(),
                    cancellation: CancellationToken::default(),
                })
                .unwrap();
            let output = reversed.next_batch().unwrap().unwrap();
            assert_eq!(output.num_rows(), 2);
            assert_eq!(output.num_columns(), 2);
            assert_eq!(output.schema().field(0).name(), "count");
            assert_eq!(output.schema().field(1).name(), "label");
            assert!(reversed.next_batch().unwrap().is_none());
            drop(reversed);

            let mut empty = dataset
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
            assert!(empty.contract().contract.geometry.is_none());
            let output = empty.next_batch().unwrap().unwrap();
            assert_eq!(output.num_rows(), 2);
            assert_eq!(output.num_columns(), 0);
            assert!(empty.next_batch().unwrap().is_none());
        }

        #[test]
        fn filegdb_float64_edge_values_do_not_silently_change() {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("float-edges.gdb");
            let schema: SchemaRef = Arc::new(Schema::new(vec![
                geometry_field(GEOMETRY, "EPSG:3857"),
                Field::new("value", DataType::Float64, false),
            ]));
            let wkb = point_wkb(CoordinateDimensions::Xy, None, None);
            let values = [
                f64::MIN,
                f64::MAX,
                -0.0,
                f64::NAN,
                f64::INFINITY,
                f64::NEG_INFINITY,
            ];
            let geometries = BinaryArray::from(
                values
                    .iter()
                    .map(|_| Some(wkb.as_slice()))
                    .collect::<Vec<_>>(),
            );
            let batch = RecordBatch::try_new(
                schema.clone(),
                vec![
                    Arc::new(geometries),
                    Arc::new(Float64Array::from(values.to_vec())),
                ],
            )
            .unwrap();
            let mut geometry = GeometryColumnContract::wkb_xy(
                FieldId(0),
                GEOMETRY,
                ResolvedCrs::new(Some("EPSG:3857".to_owned()), CrsKind::Projected, None),
                false,
            );
            geometry.set_exact_geometry_types(vec![GeometryType::Point]);
            let plan = WritePlan {
                layers: vec![WriteLayer {
                    name: "float_edges".to_owned(),
                    contract: DataContract {
                        schema,
                        geometry: Some(geometry),
                    },
                }],
            };

            let driver = super::super::FileGdbDriver;
            let mut writer = driver
                .create(Sink::Path(path.clone()), &plan, &opzioni_scrittura())
                .unwrap();
            writer.write(&batch).unwrap();
            writer.finish().unwrap();
            let dataset = driver.open(Source::Path(path), opzioni_lettura()).unwrap();
            let mut reader = dataset.open_layer_reader(&read_request()).unwrap();
            let output = reader.next_batch().unwrap().unwrap();
            let actual = output
                .column(1)
                .as_any()
                .downcast_ref::<Float64Array>()
                .unwrap();
            for (index, expected) in values.iter().enumerate() {
                assert!(!actual.is_null(index));
                assert_eq!(actual.value(index).to_bits(), expected.to_bits());
            }
        }

        #[test]
        fn filegdb_xyz_round_trip_preserves_z_and_contract_metadata() {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("xyz.gdb");
            let schema: SchemaRef =
                Arc::new(Schema::new(vec![geometry_field(GEOMETRY, "EPSG:3857")]));
            let wkb = point_wkb(CoordinateDimensions::Xyz, Some(123.25), None);
            let batch = RecordBatch::try_new(
                schema.clone(),
                vec![Arc::new(BinaryArray::from(vec![Some(wkb.as_slice())]))],
            )
            .unwrap();
            let mut geometry = GeometryColumnContract::wkb_xy(
                FieldId(0),
                GEOMETRY,
                ResolvedCrs::new(Some("EPSG:3857".to_owned()), CrsKind::Projected, None),
                true,
            );
            geometry.dimensions = CoordinateDimensions::Xyz;
            geometry.set_exact_geometry_types(vec![GeometryType::Point]);
            let plan = WritePlan {
                layers: vec![WriteLayer {
                    name: "points_z".to_owned(),
                    contract: DataContract {
                        schema,
                        geometry: Some(geometry),
                    },
                }],
            };

            let driver = super::super::FileGdbDriver;
            let mut writer = driver
                .create(Sink::Path(path.clone()), &plan, &opzioni_scrittura())
                .unwrap();
            writer.write(&batch).unwrap();
            writer.finish().unwrap();

            let dataset = driver.open(Source::Path(path), opzioni_lettura()).unwrap();
            let output_contract = dataset.layers()[0].contract.geometry.as_ref().unwrap();
            assert_eq!(output_contract.dimensions, CoordinateDimensions::Xyz);
            assert_eq!(output_contract.geometry_types, vec![GeometryType::Point]);
            let mut reader = dataset.open_layer_reader(&read_request()).unwrap();
            let output = reader.next_batch().unwrap().unwrap();
            let geometry = output
                .column(0)
                .as_any()
                .downcast_ref::<BinaryArray>()
                .unwrap();
            let decoded = decode_wkb(geometry.value(0), &WkbLimits::default()).unwrap();
            assert_eq!(decoded.dimensions, CoordinateDimensions::Xyz);
            assert_eq!(
                decoded.value,
                WkbValue::Point(WkbCoordinate {
                    x: 10.5,
                    y: 20.25,
                    z: Some(123.25),
                    m: None,
                })
            );
        }

        #[test]
        fn filegdb_measure_round_trip_preserves_xym_and_xyzm() {
            for (dimensions, z, m, suffix) in [
                (CoordinateDimensions::Xym, None, Some(7.5), "xym"),
                (CoordinateDimensions::Xyzm, Some(123.25), Some(7.5), "xyzm"),
            ] {
                let dir = tempfile::tempdir().unwrap();
                let path = dir.path().join(format!("{suffix}.gdb"));
                let schema: SchemaRef =
                    Arc::new(Schema::new(vec![geometry_field(GEOMETRY, "EPSG:3857")]));
                let wkb = point_wkb(dimensions, z, m);
                let batch = RecordBatch::try_new(
                    schema.clone(),
                    vec![Arc::new(BinaryArray::from(vec![Some(wkb.as_slice())]))],
                )
                .unwrap();
                let mut geometry = GeometryColumnContract::wkb_xy(
                    FieldId(0),
                    GEOMETRY,
                    ResolvedCrs::new(Some("EPSG:3857".to_owned()), CrsKind::Projected, None),
                    true,
                );
                geometry.dimensions = dimensions;
                geometry.set_exact_geometry_types(vec![GeometryType::Point]);
                let plan = WritePlan {
                    layers: vec![WriteLayer {
                        name: format!("points_{suffix}"),
                        contract: DataContract {
                            schema,
                            geometry: Some(geometry),
                        },
                    }],
                };

                let driver = super::super::FileGdbDriver;
                let mut writer = driver
                    .create(Sink::Path(path.clone()), &plan, &opzioni_scrittura())
                    .unwrap();
                writer.write(&batch).unwrap();
                writer.finish().unwrap();

                let dataset = driver.open(Source::Path(path), opzioni_lettura()).unwrap();
                let output_contract = dataset.layers()[0].contract.geometry.as_ref().unwrap();
                assert_eq!(output_contract.dimensions, dimensions);
                let mut reader = dataset.open_layer_reader(&read_request()).unwrap();
                let output = reader.next_batch().unwrap().unwrap();
                let geometry = output
                    .column(0)
                    .as_any()
                    .downcast_ref::<BinaryArray>()
                    .unwrap();
                let decoded = decode_wkb(geometry.value(0), &WkbLimits::default()).unwrap();
                assert_eq!(decoded.dimensions, dimensions);
                assert_eq!(
                    decoded.value,
                    WkbValue::Point(WkbCoordinate {
                        x: 10.5,
                        y: 20.25,
                        z,
                        m,
                    })
                );
            }
        }

        #[test]
        fn filegdb_multipart_xyzm_round_trip_preserves_every_ordinate() {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("multiline-xyzm.gdb");
            let dimensions = CoordinateDimensions::Xyzm;
            let geometry_value = WkbGeometry {
                value: WkbValue::MultiLineString(vec![WkbGeometry {
                    value: WkbValue::LineString(vec![
                        WkbCoordinate {
                            x: 1.0,
                            y: 2.0,
                            z: Some(3.0),
                            m: Some(4.0),
                        },
                        WkbCoordinate {
                            x: 5.0,
                            y: 6.0,
                            z: Some(7.0),
                            m: Some(8.0),
                        },
                    ]),
                    dimensions,
                    srid: None,
                }]),
                dimensions,
                srid: None,
            };
            let wkb = encode_wkb(&geometry_value, WkbFlavor::Iso).unwrap();
            let schema: SchemaRef =
                Arc::new(Schema::new(vec![geometry_field(GEOMETRY, "EPSG:3857")]));
            let batch = RecordBatch::try_new(
                schema.clone(),
                vec![Arc::new(BinaryArray::from(vec![Some(wkb.as_slice())]))],
            )
            .unwrap();
            let mut geometry = GeometryColumnContract::wkb_xy(
                FieldId(0),
                GEOMETRY,
                ResolvedCrs::new(Some("EPSG:3857".to_owned()), CrsKind::Projected, None),
                true,
            );
            geometry.dimensions = dimensions;
            geometry.set_exact_geometry_types(vec![GeometryType::MultiLineString]);
            let plan = WritePlan {
                layers: vec![WriteLayer {
                    name: "multiline_xyzm".to_owned(),
                    contract: DataContract {
                        schema,
                        geometry: Some(geometry),
                    },
                }],
            };

            let driver = super::super::FileGdbDriver;
            let mut writer = driver
                .create(Sink::Path(path.clone()), &plan, &opzioni_scrittura())
                .unwrap();
            writer.write(&batch).unwrap();
            writer.finish().unwrap();

            let dataset = driver.open(Source::Path(path), opzioni_lettura()).unwrap();
            let mut reader = dataset.open_layer_reader(&read_request()).unwrap();
            let output = reader.next_batch().unwrap().unwrap();
            let geometry = output
                .column(0)
                .as_any()
                .downcast_ref::<BinaryArray>()
                .unwrap();
            let decoded = decode_wkb(geometry.value(0), &WkbLimits::default()).unwrap();
            assert_eq!(decoded, geometry_value);
        }

        #[test]
        fn filegdb_empty_layer_is_created_from_the_contract() {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("empty.gdb");
            let plan = WritePlan {
                layers: vec![write_layer(ResolvedCrs::new(
                    Some("EPSG:3857".to_owned()),
                    CrsKind::Projected,
                    None,
                ))],
            };
            let driver = super::super::FileGdbDriver;
            driver
                .create(Sink::Path(path.clone()), &plan, &opzioni_scrittura())
                .unwrap()
                .finish()
                .unwrap();

            let dataset = driver.open(Source::Path(path), opzioni_lettura()).unwrap();
            assert_eq!(dataset.layers().len(), 1);
            let geometry = dataset.layers()[0].contract.geometry.as_ref().unwrap();
            assert_eq!(geometry.dimensions, CoordinateDimensions::Xy);
            assert_eq!(geometry.geometry_types, vec![GeometryType::Point]);
            let mut reader = dataset.open_layer_reader(&read_request()).unwrap();
            assert!(reader.next_batch().unwrap().is_none());
        }

        #[test]
        fn filegdb_drop_writer_aborts_and_removes_staging() {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("aborted.gdb");
            let plan = WritePlan {
                layers: vec![write_layer(ResolvedCrs::new(
                    Some("EPSG:3857".to_owned()),
                    CrsKind::Projected,
                    None,
                ))],
            };
            let writer = super::super::FileGdbDriver
                .create(Sink::Path(path.clone()), &plan, &opzioni_scrittura())
                .unwrap();
            let artifacts = staging_artifacts(&path);
            assert_eq!(artifacts.len(), 2);
            assert!(!path.exists());

            drop(writer);
            assert!(artifacts.iter().all(|artifact| !artifact.exists()));
            assert!(staging_artifacts(&path).is_empty());
            assert!(!path.exists());
        }

        #[test]
        fn filegdb_concurrent_staging_does_not_delete_active_writer() {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("concurrent.gdb");
            let plan = WritePlan {
                layers: vec![write_layer(ResolvedCrs::new(
                    Some("EPSG:3857".to_owned()),
                    CrsKind::Projected,
                    None,
                ))],
            };
            let first = super::super::FileGdbDriver
                .create(Sink::Path(path.clone()), &plan, &opzioni_scrittura())
                .unwrap();
            let first_artifacts = staging_artifacts(&path);
            assert_eq!(first_artifacts.len(), 2);

            let second = super::super::FileGdbDriver
                .create(Sink::Path(path.clone()), &plan, &opzioni_scrittura())
                .unwrap();
            assert_eq!(staging_artifacts(&path).len(), 4);
            assert!(first_artifacts.iter().all(|artifact| artifact.exists()));

            drop(second);
            assert!(first_artifacts.iter().all(|artifact| artifact.exists()));
            first.finish().unwrap();
            assert!(path.exists());
            assert!(staging_artifacts(&path).is_empty());
        }

        #[test]
        #[ignore = "helper eseguito dal test di ownership cross-process"]
        fn filegdb_active_subprocess_helper() {
            let path = PathBuf::from(
                std::env::var_os("PLENORA_FILEGDB_ACTIVE_DEST")
                    .expect("PLENORA_FILEGDB_ACTIVE_DEST mancante"),
            );
            let ready = PathBuf::from(
                std::env::var_os("PLENORA_FILEGDB_ACTIVE_READY")
                    .expect("PLENORA_FILEGDB_ACTIVE_READY mancante"),
            );
            let release = PathBuf::from(
                std::env::var_os("PLENORA_FILEGDB_ACTIVE_RELEASE")
                    .expect("PLENORA_FILEGDB_ACTIVE_RELEASE mancante"),
            );
            let (plan, batch) = point_write_fixture();
            let mut writer = super::super::FileGdbDriver
                .create(Sink::Path(path), &plan, &opzioni_scrittura())
                .unwrap();
            writer.write(&batch).unwrap();
            File::create(ready).unwrap();

            let deadline = Instant::now() + Duration::from_secs(10);
            while !release.exists() {
                assert!(
                    Instant::now() < deadline,
                    "timeout in attesa del rilascio dal processo padre"
                );
                std::thread::sleep(Duration::from_millis(10));
            }
            writer.finish().unwrap();
        }

        #[test]
        fn filegdb_recovery_preserves_active_cross_process_staging() {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("cross-process.gdb");
            let ready = dir.path().join("writer.ready");
            let release = dir.path().join("writer.release");
            let mut child = spawn_active_subprocess(&path, &ready, &release);
            wait_until_ready(&mut child, &ready);

            let active_artifacts = staging_artifacts(&path);
            let (plan, _) = point_write_fixture();
            let second = super::super::FileGdbDriver
                .create(Sink::Path(path.clone()), &plan, &opzioni_scrittura())
                .unwrap();
            let both_stagings_exist = staging_artifacts(&path).len() == 4;
            let active_was_preserved = active_artifacts.iter().all(|artifact| artifact.exists());
            drop(second);
            let active_survived_second_cleanup =
                active_artifacts.iter().all(|artifact| artifact.exists());

            File::create(release).unwrap();
            let status = child.wait().unwrap();
            assert!(status.success(), "writer attivo fallito: {status}");
            assert_eq!(active_artifacts.len(), 2);
            assert!(both_stagings_exist);
            assert!(active_was_preserved);
            assert!(active_survived_second_cleanup);
            assert!(staging_artifacts(&path).is_empty());
            assert_complete_point_dataset(path);
        }

        #[test]
        #[ignore = "helper eseguito dai test di fault injection in un sottoprocesso"]
        fn filegdb_crash_subprocess_helper() {
            let path = PathBuf::from(
                std::env::var_os("PLENORA_FILEGDB_CRASH_DEST")
                    .expect("PLENORA_FILEGDB_CRASH_DEST mancante"),
            );
            let point = std::env::var("PLENORA_FILEGDB_CRASH_POINT")
                .expect("PLENORA_FILEGDB_CRASH_POINT mancante");
            assert!(
                matches!(
                    point.as_str(),
                    "after_write" | "before_publish" | "after_publish"
                ),
                "failpoint sconosciuto: {point}"
            );

            let (plan, batch) = point_write_fixture();
            let mut writer = super::super::FileGdbDriver
                .create(Sink::Path(path), &plan, &opzioni_scrittura())
                .unwrap();
            writer.write(&batch).unwrap();
            writer.finish().unwrap();
            panic!("il failpoint '{point}' non ha terminato il sottoprocesso");
        }

        #[test]
        fn filegdb_process_crashes_leave_absent_or_complete_destination() {
            for point in ["after_write", "before_publish", "after_publish"] {
                let dir = tempfile::tempdir().unwrap();
                let path = dir.path().join(format!("{point}.gdb"));
                let status = run_crash_subprocess(&path, point);
                assert!(!status.success(), "il failpoint '{point}' non è scattato");

                let orphaned = staging_artifacts(&path);
                if point == "after_publish" {
                    assert!(path.exists(), "destinazione assente dopo il rename");
                    assert_eq!(orphaned.len(), 1, "sidecar orfano atteso");
                    assert_complete_point_dataset(path.clone());

                    assert_eq!(recover_orphaned_staging(&path).unwrap(), 1);
                } else {
                    assert!(!path.exists(), "output parziale reso visibile");
                    assert_eq!(orphaned.len(), 2, "staging orfano atteso");

                    let (plan, batch) = point_write_fixture();
                    let mut writer = super::super::FileGdbDriver
                        .create(Sink::Path(path.clone()), &plan, &opzioni_scrittura())
                        .unwrap();
                    assert!(
                        orphaned.iter().all(|artifact| !artifact.exists()),
                        "lo staging orfano non è stato recuperato"
                    );
                    writer.write(&batch).unwrap();
                    writer.finish().unwrap();
                }

                assert!(staging_artifacts(&path).is_empty());
                assert_complete_point_dataset(path);
            }
        }

        #[test]
        fn filegdb_failed_batch_poisons_writer_and_prevents_publish() {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("failed-write.gdb");
            let layer = write_layer(ResolvedCrs::new(
                Some("EPSG:3857".to_owned()),
                CrsKind::Projected,
                None,
            ));
            let schema = layer.contract.schema.clone();
            let hidden_z = point_wkb(CoordinateDimensions::Xyz, Some(3.0), None);
            let batch = RecordBatch::try_new(
                schema,
                vec![Arc::new(BinaryArray::from(vec![Some(hidden_z.as_slice())]))],
            )
            .unwrap();
            let plan = WritePlan {
                layers: vec![layer],
            };
            let mut writer = super::super::FileGdbDriver
                .create(Sink::Path(path.clone()), &plan, &opzioni_scrittura())
                .unwrap();
            writer.declare_input_total(LayerId(0), 1).unwrap();

            let write_error = writer
                .write(&batch)
                .expect_err("WKB XYZ nascosto in un contratto XY deve essere rifiutato");
            assert_eq!(
                write_error.category,
                plenora_io_model::ErrorCategory::DataMapping
            );
            assert_eq!(write_error.phase, plenora_io_model::ErrorPhase::Write);
            assert_eq!(write_error.retry, plenora_io_model::RetryDisposition::Never);
            assert_eq!(
                write_error.capability_reason,
                Some(CapabilityReason::CoordinateDimensions)
            );
            let diagnostics = write_error.row_diagnostics.as_deref().unwrap();
            assert_eq!(diagnostics.observed_total, 1);
            assert_eq!(diagnostics.input_total, Some(1));
            assert_eq!(diagnostics.examples[0].source_index, 0);
            assert_eq!(
                diagnostics.examples[0].cause,
                "contract.coordinate_dimensions"
            );
            let artifacts = staging_artifacts(&path);
            assert_eq!(artifacts.len(), 2);
            assert!(matches!(
                writer.write(&batch),
                Err(error)
                    if error.code == plenora_io_model::IoErrorCode::Format
                        && error.driver.as_deref() == Some("filegdb")
            ));
            assert!(matches!(
                writer.finish(),
                Err(error)
                    if error.code == plenora_io_model::IoErrorCode::Format
                        && error.driver.as_deref() == Some("filegdb")
            ));
            assert!(artifacts.iter().all(|artifact| !artifact.exists()));
            assert!(staging_artifacts(&path).is_empty());
            assert!(!path.exists());
        }

        #[test]
        fn filegdb_output_limit_failure_removes_staging_without_publish() {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("too-large.gdb");
            let plan = WritePlan {
                layers: vec![write_layer(ResolvedCrs::new(
                    Some("EPSG:3857".to_owned()),
                    CrsKind::Projected,
                    None,
                ))],
            };
            // Un tetto di uscita a zero: la scrittura deve fallire prima di
            // pubblicare, e lo staging deve sparire.
            let options = match plenora_io_model::budget::PipelineBudget::builder()
                .limits(
                    plenora_io_model::budget::PipelineLimits::default().with_max_output_bytes(1),
                )
                .build()
            {
                Ok(bundle) => WriteOptions::from_write_parts(bundle.into_write_parts()),
                Err(error) => unreachable!("bundle di test: {error:?}"),
            };
            let writer = super::super::FileGdbDriver
                .create(Sink::Path(path.clone()), &plan, &options)
                .unwrap();
            let artifacts = staging_artifacts(&path);
            assert_eq!(artifacts.len(), 2);

            assert!(matches!(
                writer.finish(),
                Err(error) if error.code == plenora_io_model::IoErrorCode::LimitExceeded
            ));
            assert!(artifacts.iter().all(|artifact| !artifact.exists()));
            assert!(staging_artifacts(&path).is_empty());
            assert!(!path.exists());
        }

        #[test]
        fn filegdb_empty_layers_preserve_native_families_and_dimensions() {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("geometry-families.gdb");
            let geometry_types = [
                GeometryType::Point,
                GeometryType::MultiPoint,
                GeometryType::MultiLineString,
                GeometryType::MultiPolygon,
            ];
            let dimensions = [
                CoordinateDimensions::Xy,
                CoordinateDimensions::Xyz,
                CoordinateDimensions::Xym,
                CoordinateDimensions::Xyzm,
            ];
            let expected: Vec<(GeometryType, CoordinateDimensions)> = geometry_types
                .iter()
                .flat_map(|geometry_type| {
                    dimensions
                        .iter()
                        .map(move |dimensions| (*geometry_type, *dimensions))
                })
                .collect();
            let layers = expected
                .iter()
                .enumerate()
                .map(|(index, (geometry_type, dimensions))| {
                    let name = format!("family_{index}");
                    let schema: SchemaRef =
                        Arc::new(Schema::new(vec![geometry_field(GEOMETRY, "EPSG:3857")]));
                    let mut geometry = GeometryColumnContract::wkb_xy(
                        FieldId(0),
                        GEOMETRY,
                        ResolvedCrs::new(Some("EPSG:3857".to_owned()), CrsKind::Projected, None),
                        true,
                    );
                    geometry.dimensions = *dimensions;
                    geometry.set_exact_geometry_types(vec![*geometry_type]);
                    WriteLayer {
                        name,
                        contract: DataContract {
                            schema,
                            geometry: Some(geometry),
                        },
                    }
                })
                .collect();
            let plan = WritePlan { layers };
            let driver = super::super::FileGdbDriver;
            driver
                .create(Sink::Path(path.clone()), &plan, &opzioni_scrittura())
                .unwrap()
                .finish()
                .unwrap();

            let dataset = driver.open(Source::Path(path), opzioni_lettura()).unwrap();
            assert_eq!(dataset.layers().len(), expected.len());
            for (layer, (expected_type, expected_dimensions)) in
                dataset.layers().iter().zip(expected)
            {
                let geometry = layer.contract.geometry.as_ref().unwrap();
                assert_eq!(geometry.dimensions, expected_dimensions);
                assert_eq!(geometry.geometry_types, vec![expected_type]);
            }
        }

        #[test]
        fn filegdb_rejects_geometry_families_normalized_by_the_format() {
            for geometry_type in [GeometryType::LineString, GeometryType::Polygon] {
                let dir = tempfile::tempdir().unwrap();
                let path = dir.path().join("normalized-family.gdb");
                let mut layer = write_layer(ResolvedCrs::new(
                    Some("EPSG:3857".to_owned()),
                    CrsKind::Projected,
                    None,
                ));
                layer
                    .contract
                    .geometry
                    .as_mut()
                    .unwrap()
                    .set_exact_geometry_types(vec![geometry_type]);
                let plan = WritePlan {
                    layers: vec![layer],
                };

                let result = super::super::FileGdbDriver.create(
                    Sink::Path(path.clone()),
                    &plan,
                    &opzioni_scrittura(),
                );
                assert!(matches!(
                    result,
                    Err(error)
                        if error.capability_reason
                            == Some(CapabilityReason::GeometryNotSupported)
                ));
                assert!(!path.exists());
            }
        }

        #[test]
        fn filegdb_rejects_ewkb_before_output_creation() {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("ewkb.gdb");
            let mut layer = write_layer(ResolvedCrs::new(
                Some("EPSG:3857".to_owned()),
                CrsKind::Projected,
                None,
            ));
            layer.contract.geometry.as_mut().unwrap().encoding = GeometryEncoding::Ewkb;
            let plan = WritePlan {
                layers: vec![layer],
            };

            let result = super::super::FileGdbDriver.create(
                Sink::Path(path.clone()),
                &plan,
                &opzioni_scrittura(),
            );
            assert!(matches!(
                result,
                Err(error)
                    if error.capability_reason == Some(CapabilityReason::GeometryEncoding)
            ));
            assert!(!path.exists());
        }

        #[test]
        fn filegdb_rejects_non_round_trip_attribute_types_before_output() {
            for (data_type, suffix) in [
                (DataType::Int64, "int64"),
                (DataType::Boolean, "boolean"),
                (DataType::Date32, "date32"),
                (DataType::Binary, "binary"),
            ] {
                let dir = tempfile::tempdir().unwrap();
                let path = dir.path().join(format!("{suffix}.gdb"));
                let mut layer = write_layer(ResolvedCrs::new(
                    Some("EPSG:3857".to_owned()),
                    CrsKind::Projected,
                    None,
                ));
                layer.contract.schema = Arc::new(Schema::new(vec![
                    geometry_field(GEOMETRY, "EPSG:3857"),
                    Field::new("unsupported", data_type, true),
                ]));
                let plan = WritePlan {
                    layers: vec![layer],
                };

                let result = super::super::FileGdbDriver.create(
                    Sink::Path(path.clone()),
                    &plan,
                    &opzioni_scrittura(),
                );
                assert!(matches!(
                    result,
                    Err(error)
                        if error.capability_reason
                            == Some(CapabilityReason::TypeNotRepresentable)
                ));
                assert!(!path.exists());
            }
        }

        #[test]
        fn filegdb_rejects_incoherent_native_field_metadata_before_output() {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("bad-native-metadata.gdb");
            let mut layer = write_layer(ResolvedCrs::new(
                Some("EPSG:3857".to_owned()),
                CrsKind::Projected,
                None,
            ));
            let metadata = HashMap::from([(
                OGR_FIELD_TYPE_KEY.to_owned(),
                gdal::vector::OGRFieldType::OFTReal.to_string(),
            )]);
            layer.contract.schema = Arc::new(Schema::new(vec![
                geometry_field(GEOMETRY, "EPSG:3857"),
                Field::new("text", DataType::Utf8, true).with_metadata(metadata),
            ]));
            let plan = WritePlan {
                layers: vec![layer],
            };

            let result = super::super::FileGdbDriver.create(
                Sink::Path(path.clone()),
                &plan,
                &opzioni_scrittura(),
            );
            assert!(matches!(
                result,
                Err(error)
                    if error.capability_reason
                        == Some(CapabilityReason::TypeNotRepresentable)
            ));
            assert!(!path.exists());
        }

        // --- una tabella non spaziale non chiude il dataset -----------------

        /// Un `FileGDB` con una feature class **e** una tabella non spaziale.
        ///
        /// E' come sono fatte le GDB vere: accanto alle feature class stanno le
        /// tabelle, che il formato ammette e che ESRI usa di continuo. La
        /// fixture si costruisce con OGR e non con il writer del driver, che
        /// scrive soltanto layer spaziali e quindi non saprebbe produrre il
        /// caso.
        fn gdb_con_tabella(percorso: &std::path::Path) {
            use gdal::vector::{FieldDefn, LayerOptions, OGRFieldType, OGRwkbGeometryType};

            let driver = DriverManager::get_driver_by_name("OpenFileGDB")
                .expect("driver OpenFileGDB disponibile");
            let mut ds = driver
                .create_vector_only(percorso)
                .expect("FileGDB creabile");
            let srs = gdal::spatial_ref::SpatialRef::from_epsg(3857).expect("EPSG:3857");

            let spaziale = ds
                .create_layer(LayerOptions {
                    name: "punti",
                    srs: Some(&srs),
                    ty: OGRwkbGeometryType::wkbPoint,
                    options: None,
                })
                .expect("feature class creabile");
            let campo = FieldDefn::new("nome", OGRFieldType::OFTString).expect("campo");
            campo.add_to_layer(&spaziale).expect("campo aggiunto");

            // Nessun SRS e nessuna geometria: e' una tabella.
            let tabella = ds
                .create_layer(LayerOptions {
                    name: "tabella",
                    srs: None,
                    ty: OGRwkbGeometryType::wkbNone,
                    options: None,
                })
                .expect("tabella creabile");
            let campo = FieldDefn::new("nota", OGRFieldType::OFTString).expect("campo");
            campo.add_to_layer(&tabella).expect("campo aggiunto");
            let campo = FieldDefn::new("peso", OGRFieldType::OFTInteger).expect("campo");
            campo.add_to_layer(&tabella).expect("campo aggiunto");
        }

        /// Una tabella non spaziale non deve rendere illeggibile la GDB.
        ///
        /// Il driver rifiutava l'**intero dataset** -- «layer senza campo
        /// geometrico» -- appena incontrava un layer senza geometria, e lo
        /// faceva enumerando: quattro feature class valide diventavano
        /// inaccessibili per la presenza di una tabella accanto a loro. Il
        /// rifiuto arrivava per giunta in fase di apertura, prima di qualunque
        /// `--layer`, quindi non c'era modo di aggirarlo scegliendo.
        ///
        /// La tabella va **enumerata**, non saltata: saltarla cambierebbe gli
        /// indici di layer sotto i piedi di chi li ha letti dal catalogo, e un
        /// `--layer 1` significherebbe due cose diverse a seconda di quali
        /// layer il driver ha deciso di nascondere.
        #[test]
        fn una_tabella_non_spaziale_non_impedisce_di_aprire_la_gdb() {
            let dir = tempfile::tempdir().unwrap();
            let percorso = dir.path().join("con_tabella.gdb");
            gdb_con_tabella(&percorso);

            let dataset = open(&percorso, None)
                .expect("una tabella accanto a una feature class non chiude il dataset");
            let layers = dataset.layers();
            assert_eq!(layers.len(), 2, "entrambi i layer vanno enumerati");

            let spaziale = layers
                .iter()
                .find(|l| l.name == "punti")
                .expect("la feature class e' enumerata");
            assert!(
                spaziale.contract.geometry.is_some(),
                "la feature class porta il proprio contratto geometrico"
            );

            let tabella = layers
                .iter()
                .find(|l| l.name == "tabella")
                .expect("la tabella e' enumerata");
            assert!(
                tabella.contract.geometry.is_none(),
                "una tabella non ha geometria, e il contratto lo dice invece di inventarne una"
            );
            assert_eq!(
                tabella
                    .contract
                    .schema
                    .fields()
                    .iter()
                    .map(|f| f.name().as_str())
                    .collect::<Vec<_>>(),
                vec!["nota", "peso"],
                "gli attributi della tabella ci sono tutti, e senza una colonna \
                 geometrica davanti"
            );
        }

        /// La tabella si legge, e i suoi attributi non scivolano di una
        /// colonna.
        ///
        /// E' la controprova che l'enumerazione non basta: lo schema Arrow di
        /// una feature class ha la geometria in posizione zero e gli attributi
        /// spostati di uno, e il reader traduceva l'indice Arrow in indice OGR
        /// sottraendo sempre quell'uno. Su una tabella, dove lo scostamento non
        /// c'e', la stessa sottrazione avrebbe letto il campo sbagliato -- o
        /// nessuno.
        #[test]
        fn la_tabella_si_legge_e_gli_attributi_non_scivolano() {
            let dir = tempfile::tempdir().unwrap();
            let percorso = dir.path().join("lettura.gdb");
            gdb_con_tabella(&percorso);

            let dataset = open(&percorso, None).expect("dataset aperto");
            let indice = dataset
                .layers()
                .iter()
                .position(|l| l.name == "tabella")
                .expect("la tabella e' enumerata");
            // Indice di layer: due layer, il cast non puo' troncare.
            #[allow(clippy::cast_possible_truncation)]
            let id = LayerId(indice as u32);
            let mut lettore = dataset
                .open_layer_reader(&ReadRequest {
                    layer: id,
                    ..read_request()
                })
                .expect("una tabella si apre in lettura");
            let mut righe = 0usize;
            while let Some(batch) = lettore.next_batch().expect("lettura della tabella") {
                assert_eq!(
                    batch
                        .schema()
                        .fields()
                        .iter()
                        .map(|f| f.name().as_str())
                        .collect::<Vec<_>>(),
                    vec!["nota", "peso"],
                    "il batch porta gli attributi della tabella, nell'ordine dello schema"
                );
                righe += batch.num_rows();
            }
            assert_eq!(righe, 0, "la tabella della fixture e' vuota");
        }

        /// La feature class resta leggibile **accanto** alla tabella.
        ///
        /// Senza questa, «il dataset si apre» sarebbe vero anche di un driver
        /// che ha smesso di leggere le geometrie: e' la meta' che il difetto
        /// rendeva irraggiungibile, ed e' quella che conta.
        #[test]
        fn la_feature_class_resta_leggibile_accanto_alla_tabella() {
            let dir = tempfile::tempdir().unwrap();
            let percorso = dir.path().join("accanto.gdb");
            gdb_con_tabella(&percorso);

            let dataset = open(&percorso, None).expect("dataset aperto");
            let indice = dataset
                .layers()
                .iter()
                .position(|l| l.name == "punti")
                .expect("la feature class e' enumerata");
            #[allow(clippy::cast_possible_truncation)]
            let id = LayerId(indice as u32);
            let mut lettore = dataset
                .open_layer_reader(&ReadRequest {
                    layer: id,
                    ..read_request()
                })
                .expect("la feature class si apre in lettura");
            while lettore
                .next_batch()
                .expect("lettura della feature class")
                .is_some()
            {}
        }
    }
}

#[cfg(all(test, not(feature = "gdal-backend")))]
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
    // Il helper serve solo ai test che girano con `gdal-backend`:
    // senza la feature non lo chiama nessuno, e clippy ha ragione.
    #[cfg(feature = "gdal-backend")]
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

    /// Il gemello in scrittura di `open_without_gdal_feature_is_typed`.
    ///
    /// Senza `gdal-backend` anche `create` deve fallire **tipizzato**, non
    /// panicare ne' restituire un errore generico: il chiamante deve poter
    /// distinguere «formato non disponibile in questo build» da «piano non
    /// valido», e le due cose hanno rimedi diversi.
    ///
    /// Il piano dev'essere **valido**: `create` esegue `validate_write` prima
    /// del ramo stub, quindi un piano scorretto fallirebbe nella validazione e
    /// il test proverebbe un'altra cosa.
    ///
    /// Aggiunto dopo la diagnostica differenziale del checkpoint su `8e64965`,
    /// che ha trovato questo ramo mai eseguito.
    #[cfg(not(feature = "gdal-backend"))]
    #[test]
    fn create_without_gdal_feature_is_typed() {
        use driver_common::geometry_field;
        use plenora_io_core::{WriteLayer, WritePlan};
        use plenora_io_model::contract::{
            DataContract, FieldId, GeometryColumnContract, GeometryType,
        };
        use plenora_io_model::crs::{CrsKind, ResolvedCrs};

        let crs = ResolvedCrs::new(Some("EPSG:3857".to_owned()), CrsKind::Projected, None);
        let schema: std::sync::Arc<arrow_schema::Schema> = std::sync::Arc::new(
            arrow_schema::Schema::new(vec![geometry_field("geometry", "EPSG:3857")]),
        );
        let mut geometria = GeometryColumnContract::wkb_xy(FieldId(0), "geometry", crs, true);
        geometria.set_exact_geometry_types(vec![GeometryType::Point]);
        let plan = WritePlan {
            layers: vec![WriteLayer {
                name: "points".to_owned(),
                contract: DataContract {
                    schema,
                    geometry: Some(geometria),
                },
            }],
        };

        let opzioni = match plenora_io_model::budget::PipelineBudget::builder().build() {
            Ok(bundle) => WriteOptions::from_write_parts(bundle.into_write_parts()),
            Err(errore) => unreachable!("bundle di test non costruibile: {errore:?}"),
        };

        let errore = FileGdbDriver
            .create(Sink::Path("x.gdb".into()), &plan, &opzioni)
            .map(|_| ())
            .expect_err("senza il tier GDB la scrittura non puo' riuscire");

        assert_eq!(errore.code, plenora_io_model::IoErrorCode::Unsupported);
        assert_eq!(
            errore.category,
            plenora_io_model::ErrorCategory::Unsupported
        );
        assert!(
            errore.message.contains("--features gdal-backend"),
            "il messaggio deve dire come rimediare: {}",
            errore.message
        );
    }

    #[test]
    fn open_without_gdal_feature_is_typed() {
        // Nel build di default (senza feature) l'apertura fallisce tipizzata.
        let e = FileGdbDriver
            .open(Source::Path("x.gdb".into()), opzioni_lettura())
            .map(|_| ())
            .unwrap_err();
        assert!(matches!(
            e,
            error if error.code == plenora_io_model::IoErrorCode::Unsupported
        ));
    }
}

/// Le sonde della superficie che il fuzz target attraversa.
///
/// Modulo a se' perche' il modulo `tests` di questo file e' gated su
/// `not(feature = "gdal-backend")`: senza la feature il driver e' uno stub, e
/// quelle sonde provano lo stub. Queste provano il contrario -- il percorso che
/// esiste **solo** con GDAL -- e metterle li' le avrebbe compilate via proprio
/// nella configurazione in cui servono.
#[cfg(all(test, feature = "gdal-backend"))]
mod sonde_del_fuzzing {
    use super::*;

    // --- la superficie del fuzz target, e la fixture che la alimenta -------
    //
    // Una build che compila e un replay senza crash non dimostrano che gli
    // input raggiungano il driver: una `.gdb` rifiutata al riconoscimento del
    // formato non fa crashare niente ed e', da fuori, indistinguibile da una
    // letta per intero. Queste sonde chiamano lo **stesso** entry point del
    // target sulla **stessa** fixture, e guardano che cosa ne esce.

    /// La fixture vive accanto al target che la usa.
    #[cfg(feature = "gdal-backend")]
    fn archivio_della_fixture() -> Vec<u8> {
        let percorso = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fuzz/fixtures/filegdb/citta.gdb.bundle");
        std::fs::read(&percorso).unwrap_or_else(|errore| {
            panic!("fixture {} non leggibile: {errore}", percorso.display())
        })
    }

    /// Limiti dello stesso ordine di quelli della campagna.
    ///
    /// Non sono gli **stessi**: `harness::limits()` vive nella crate di
    /// fuzzing, e un driver non puo' dipenderne. Quel che conta e' che ci siano
    /// tetti -- una sonda che leggesse senza limiti percorrerebbe una strada
    /// che il fuzzer non percorre mai.
    #[cfg(feature = "gdal-backend")]
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

    /// L'input vuoto materializza la fixture **intatta**, e la fixture si
    /// legge.
    ///
    /// E' il caso che tiene onesta l'intera campagna: senza, un verde potrebbe
    /// voler dire che nessun input arriva mai al driver.
    #[test]
    #[cfg(feature = "gdal-backend")]
    fn la_fixture_intatta_arriva_al_drenaggio() {
        let righe = __fuzz_leggi_gdb(&archivio_della_fixture(), &[], opzioni_di_campagna())
            .expect("la fixture deve essere letta: se no il target non copre il parsing");
        assert_eq!(
            righe, 2,
            "due feature nel GeoJSON di partenza: un conteggio diverso vuol dire \
             che la fixture non attraversa piu' il drenaggio"
        );
    }

    /// Sostituire una parte qualunque non fa panicare il driver.
    ///
    /// Non si pretende che **tutte** falliscano: alcune tabelle di metadati
    /// sono facoltative, e una `.gdb` puo' restare leggibile. Cio' che si
    /// pretende e' che l'esito sia un esito -- `Ok` o `Err`, mai un panico e
    /// mai un abort.
    #[test]
    #[cfg(feature = "gdal-backend")]
    fn ogni_parte_sostituita_da_un_esito_non_un_panico() {
        let archivio = archivio_della_fixture();
        let parti = __fuzz_parti_della_fixture(&archivio).expect("archivio leggibile");
        assert!(parti.len() >= 2, "una .gdb ha piu' di una parte");

        let mut rifiutate = 0;
        for indice in 0..parti.len() {
            let mut input = vec![u8::try_from(indice).expect("meno di 256 parti")];
            input.extend_from_slice(b"NON-E-UNA-TABELLA-FILEGDB");
            if __fuzz_leggi_gdb(&archivio, &input, opzioni_di_campagna()).is_err() {
                rifiutate += 1;
            }
        }
        assert!(
            rifiutate > 0,
            "sostituire una tabella con testo non puo' lasciare la .gdb sempre \
             leggibile: se nessuna sostituzione viene rifiutata, l'input non sta \
             arrivando al driver"
        );
    }

    /// **Isolamento fra invocazioni**, provato dalla directory e non dall'esito.
    ///
    /// La prima stesura di questa sonda rompeva una parte e poi rileggeva la
    /// fixture intatta, aspettandosi che riuscisse. Non discriminava niente: la
    /// materializzazione riscrive **tutte** le parti a ogni invocazione, quindi
    /// anche riusando la stessa directory la seconda lettura sarebbe riuscita.
    /// Provava che la riscrittura funziona, non che la directory sia nuova.
    ///
    /// Cio' che conta e' la directory. Due invocazioni devono usarne due
    /// diverse, e nessuna delle due deve sopravvivere alla propria chiamata --
    /// altrimenti una campagna lunga riempirebbe il disco di `.gdb`.
    #[test]
    #[cfg(feature = "gdal-backend")]
    fn ogni_invocazione_ha_la_propria_directory() {
        let archivio = archivio_della_fixture();
        let parti = __fuzz_parti_della_fixture(&archivio).expect("archivio leggibile");
        let tabella = parti
            .iter()
            .position(|(nome, _)| nome.ends_with(".gdbtable"))
            .expect("la fixture ha almeno una tabella");

        ULTIMA_DIRECTORY.with(|viste| viste.borrow_mut().clear());

        let mut rotta = vec![u8::try_from(tabella).expect("indice piccolo")];
        rotta.extend_from_slice(b"SPAZZATURA");
        let _ = __fuzz_leggi_gdb(&archivio, &rotta, opzioni_di_campagna());
        let dopo = __fuzz_leggi_gdb(&archivio, &[], opzioni_di_campagna());
        assert!(dopo.is_ok(), "la fixture intatta deve leggersi: {dopo:?}");

        let viste = ULTIMA_DIRECTORY.with(|viste| viste.borrow().clone());
        assert_eq!(viste.len(), 2, "due invocazioni, due materializzazioni");
        assert_ne!(
            viste[0], viste[1],
            "le due invocazioni hanno usato la **stessa** directory: la tabella \
             rotta della prima e' rimasta li' per la seconda, e l'esito non lo \
             mostra perche' ogni parte viene riscritta"
        );
        for percorso in &viste {
            assert!(
                !percorso.exists(),
                "la directory {} e' sopravvissuta alla propria invocazione: una \
                 campagna lunga riempirebbe il disco",
                percorso.display()
            );
        }
    }

    /// **I nomi dei file non vengono dal payload.**
    ///
    /// Il primo byte sceglie un **indice**, e l'indice e' preso modulo il numero
    /// di parti: nessun byte del fuzzer finisce in un percorso. La sonda prova
    /// che l'insieme dei file materializzati e' sempre quello della fixture,
    /// qualunque cosa l'input dica.
    #[test]
    #[cfg(feature = "gdal-backend")]
    fn nessun_percorso_deriva_dal_payload() {
        let archivio = archivio_della_fixture();
        let parti = __fuzz_parti_della_fixture(&archivio).expect("archivio leggibile");
        let attesi: std::collections::BTreeSet<&str> =
            parti.iter().map(|(nome, _)| nome.as_str()).collect();

        let temporanea = tempfile::tempdir().expect("directory temporanea");
        // Un indice ben oltre il numero di parti, e un contenuto che somiglia a
        // un percorso: ne' l'uno ne' l'altro devono uscire dalla directory.
        let dataset = materializza_gdb(
            temporanea.path(),
            &parti,
            usize::from(u8::MAX) % parti.len(),
            b"../../fuori.txt",
        )
        .expect("materializzazione");

        let prodotti: std::collections::BTreeSet<String> = std::fs::read_dir(&dataset)
            .expect("la directory del dataset esiste")
            .map(|voce| {
                voce.expect("voce leggibile")
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        let prodotti: std::collections::BTreeSet<&str> =
            prodotti.iter().map(String::as_str).collect();
        assert_eq!(
            prodotti, attesi,
            "i nomi materializzati sono quelli della fixture"
        );
        assert!(
            !temporanea.path().join("fuori.txt").exists(),
            "nessun file e' stato scritto fuori dalla .gdb"
        );
    }

    /// **Fail-closed dell'archivio**: un archivio corrotto non fa scrivere
    /// niente e non panica.
    ///
    /// L'archivio non e' l'input del fuzzer -- e' nostro, e sta nel binario --
    /// ma un archivio troncato o con un nome di parte inventato non deve poter
    /// far scrivere fuori dalla directory.
    #[test]
    #[cfg(feature = "gdal-backend")]
    fn un_archivio_malformato_non_produce_parti() {
        let archivio = archivio_della_fixture();
        assert!(__fuzz_parti_della_fixture(&archivio).is_some());

        // Senza intestazione.
        assert!(__fuzz_parti_della_fixture(b"qualcosa").is_none());
        // Intestazione giusta e niente altro.
        assert!(__fuzz_parti_della_fixture(b"PLENORA-GDB-FIXTURE-1\n").is_none());
        // Troncato a meta' di una parte.
        for taglio in [24, 40, archivio.len() / 2, archivio.len() - 1] {
            assert!(
                __fuzz_parti_della_fixture(&archivio[..taglio]).is_none(),
                "troncato a {taglio} byte"
            );
        }
        // Byte di coda che l'indice non dichiara.
        let mut lungo = archivio;
        lungo.push(0);
        assert!(__fuzz_parti_della_fixture(&lungo).is_none());

        // Un nome di parte che **risale** la directory. E' il caso che la
        // prima stesura accettava, perche' `".."` e' fatto di soli caratteri
        // ammessi, ed e' il solo per cui questa funzione guarda il nome: il
        // nome finisce in un `join`, e da li' si scriverebbe fuori dalla
        // `.gdb`. Il gemello Python ha la sua sonda; questa mancava, e la
        // riga di rifiuto restava l'unica difesa di questo ramo mai eseguita.
        let mut ostile = b"PLENORA-GDB-FIXTURE-1\n".to_vec();
        ostile.extend_from_slice(&1_u32.to_le_bytes());
        ostile.extend_from_slice(&2_u16.to_le_bytes());
        ostile.extend_from_slice(b"..");
        ostile.extend_from_slice(&1_u32.to_le_bytes());
        ostile.push(b'x');
        assert!(
            __fuzz_parti_della_fixture(&ostile).is_none(),
            "un nome di parte che risale la directory non e' un nome di parte"
        );
    }

    /// Un archivio che dichiara **zero** parti non e' una `.gdb` vuota.
    ///
    /// Accettarlo portava il chiamante a scegliere la parte da sostituire
    /// **modulo zero**: una divisione per zero su qualunque input non vuoto.
    /// L'archivio non viene dal fuzzer, ma un archivio corrotto non deve poter
    /// far panicare il target.
    #[test]
    #[cfg(feature = "gdal-backend")]
    fn un_archivio_senza_parti_e_rifiutato() {
        let mut vuoto = b"PLENORA-GDB-FIXTURE-1\n".to_vec();
        vuoto.extend_from_slice(&0_u32.to_le_bytes());
        assert!(
            __fuzz_parti_della_fixture(&vuoto).is_none(),
            "zero parti: il chiamante dividerebbe per zero"
        );

        // E il target lo rifiuta come errore tipizzato, non con un panico.
        let esito = __fuzz_leggi_gdb(&vuoto, b"\x01qualcosa", opzioni_di_campagna());
        assert!(esito.is_err(), "un archivio senza parti non e' leggibile");
    }

    /// I nomi ammessi sono quelli che `OpenFileGDB` scrive, e nessun altro.
    #[test]
    #[cfg(feature = "gdal-backend")]
    fn un_nome_di_parte_non_puo_essere_un_percorso() {
        for buono in ["gdb", "a00000001.gdbtable", "timestamps", "a00000009.spx"] {
            assert!(nome_di_parte_ammesso(buono), "{buono}");
        }
        for cattivo in [
            "",
            "..",
            "../fuori",
            "a/b",
            "a\\b",
            "MAIUSCOLO",
            "con spazio",
        ] {
            assert!(!nome_di_parte_ammesso(cattivo), "{cattivo}");
        }
    }

    /// **Errore d'ambiente e non finding**: una radice che non esiste produce un
    /// errore tipizzato, non un panico.
    #[test]
    #[cfg(feature = "gdal-backend")]
    fn una_radice_non_creabile_e_un_errore_di_ambiente() {
        let temporanea = tempfile::tempdir().expect("directory temporanea");
        // Un **file** dove la materializzazione vuole una directory: `create_dir_all`
        // fallisce, ed e' l'ambiente a non collaborare, non il file letto.
        let ostacolo = temporanea.path().join("citta.gdb");
        std::fs::write(&ostacolo, b"non sono una directory").expect("scrittura");

        let archivio = archivio_della_fixture();
        let parti = __fuzz_parti_della_fixture(&archivio).expect("archivio leggibile");
        let errore = materializza_gdb(temporanea.path(), &parti, parti.len(), b"")
            .expect_err("una .gdb non creabile deve fallire");
        assert!(
            errore.message.contains("ambiente"),
            "un errore d'ambiente non va confuso con un difetto del file letto: {errore:?}"
        );
    }
}
