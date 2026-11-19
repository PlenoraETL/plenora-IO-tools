//! driver-filegdb — `FileGDB` ⇄ `RecordBatch` (Fase 1, "tier GDB"). È
//! l'unica eccezione alla policy puro-Rust: `FileGDB` richiede GDAL. Dietro la
//! feature `gdal-backend` legge via GDAL; senza feature è uno stub che fallisce
//! tipizzato (il binario di default resta puro-Rust). Multi-layer.
#![forbid(unsafe_code)]

use plenora_io_core::descriptor::{
    CrsHandling, Direction, Fidelity, FormatDescriptor, GeometryWriteSupport, ReadMode,
    ReaderConcurrency, Runtime, WriteMode,
};
use plenora_io_core::driver::{
    FormatDriver, FormatWriter, OpenDatasetHandle, ReadOptions, Sink, Source, WriteOptions,
};
use plenora_io_core::{
    validate_write, AttributeWriteSupport, CrsDerivation, CrsRepresentationCapabilities,
    CrsRepresentationState, CrsWriteSupport, FormatWriteCapabilities, SinkPathConstraint,
    TypeCoercionPolicy, WritePlan, UTF8_FIELD_NAMES,
};
use plenora_io_model::contract::{GeometryEncoding, GeometryType, SpatialSemantics};
use plenora_io_model::Result;

const FILEGDB_GEOMETRY: GeometryWriteSupport = GeometryWriteSupport {
    supported: true,
    encodings: &[GeometryEncoding::Wkb],
    dimensions: plenora_io_core::loss_table::FILEGDB.dimensions,
    spatial_semantics: &[SpatialSemantics::Geometry],
    geometry_types: &[
        GeometryType::Point,
        GeometryType::MultiPoint,
        GeometryType::MultiLineString,
        GeometryType::MultiPolygon,
    ],
    mixed_types: false,
    // Il FileGDB memorizza x poi y, easting o longitudine prima.
    axis_orders: plenora_io_core::EAST_FIRST_AXIS_ORDERS,
};

static DESCRIPTOR: FormatDescriptor = FormatDescriptor::const_new(
    "filegdb",
    &plenora_io_core::dataset_files::FILE_SETS[3],
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
        allowed_types: plenora_io_core::loss_table::FILEGDB.allowed_types,
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
        nullability: plenora_io_core::loss_table::native_nullability(
            plenora_io_core::loss_table::NativeDriver::FileGdb,
        ),
        multi_layer: true,
        sink_path: SinkPathConstraint::Free,
    }),
    // Il driver non interpreta alcuna format_option (L0.7): l'elenco vuoto
    // e' l'affermazione che qualunque chiave e' sconosciuta, non un'omissione.
    plenora_io_model::format_options::SchemaOpzioniFormato::VUOTO,
    &["gdb"],
    1,
    10,
    12,
);

pub struct FileGdbDriver;

/// Rifiuta piu' campi geometrici: il contratto porta una geometria per layer.
/// La verifica esiste anche nello stub; il rifiuto riguarda il singolo layer.
#[cfg_attr(not(feature = "gdal-backend"), allow(dead_code))]
fn rifiuta_piu_geometrie(campi_geometrici: usize) -> plenora_io_model::Result<()> {
    if campi_geometrici > 1 {
        return Err(plenora_io_model::PlenoraIoError::non_supportato_redatto(
            &plenora_io_model::PublicMessage::Curated(
                "FileGDB: il layer dichiara piu' campi geometrici, e il componente porta una \
                 sola geometria per layer (GEO-012)",
            ),
        ));
    }
    Ok(())
}

/// Legge il bundle canonico GDAL, non il payload del fuzzer.
/// Formato: magic `PLENORA-GDB-FIXTURE-1\n`, u32 parti; per parte u16
/// lunghezza nome, nome ASCII, u32 lunghezza contenuto, contenuto (little endian).
/// Il parser condiviso normalizza i basename e rifiuta percorsi e alias.
/// `None` indica un bundle malformato o vuoto; la fixture resta esterna al binario.
#[cfg(feature = "gdal-backend")]
#[doc(hidden)]
#[must_use]
pub fn __fuzz_parti_della_fixture(archivio: &[u8]) -> Option<Vec<(String, &[u8])>> {
    const INTESTAZIONE: &[u8] = b"PLENORA-GDB-FIXTURE-1\n";
    let corpo = archivio.strip_prefix(INTESTAZIONE)?;
    let (grezzo, mut resto) = corpo.split_at_checked(4)?;
    let quante = u32::from_le_bytes([grezzo[0], grezzo[1], grezzo[2], grezzo[3]]);

    let mut parti = Vec::new();
    let mut names = std::collections::BTreeSet::new();
    for _ in 0..quante {
        let (grezzo, dopo) = resto.split_at_checked(2)?;
        let lunghezza_nome = usize::from(u16::from_le_bytes([grezzo[0], grezzo[1]]));
        let (nome, dopo) = dopo.split_at_checked(lunghezza_nome)?;
        let (grezzo, dopo) = dopo.split_at_checked(4)?;
        let lunghezza = u32::from_le_bytes([grezzo[0], grezzo[1], grezzo[2], grezzo[3]]);
        let (contenuto, dopo) = dopo.split_at_checked(usize::try_from(lunghezza).ok()?)?;

        let nome = std::str::from_utf8(nome).ok()?;
        let member = driver_common::filegdb_names::parse(nome)?;
        if !names.insert(member.canonical.clone()) {
            return None;
        }
        parti.push((member.canonical, contenuto));
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

/// Basenames accepted by the shared `FileGDB` member parser.
#[cfg(all(test, feature = "gdal-backend"))]
fn nome_di_parte_ammesso(nome: &str) -> bool {
    driver_common::filegdb_names::parse(nome).is_some()
}

/// Un guasto di materializzazione e' un errore tipizzato, non un finding.
#[cfg(feature = "gdal-backend")]
fn errore_di_ambiente(_: std::io::Error) -> plenora_io_model::PlenoraIoError {
    // Unsupported resta riservato al backend non compilato.
    backend::err(&plenora_io_model::PublicMessage::Curated(
        "materializzazione della .gdb fallita: e' l'ambiente, non il file letto",
    ))
}

/// Materializza in una directory esistente; il payload sceglie un indice,
/// mai un nome. Separato per provare gli errori senza modificare `TMPDIR`.
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
        std::fs::write(
            dataset.join(
                driver_common::filegdb_names::parse(nome)
                    .ok_or_else(|| {
                        backend::err(&plenora_io_model::PublicMessage::Curated(
                            "nome membro FileGDB non valido",
                        ))
                    })?
                    .canonical,
            ),
            byte,
        )
        .map_err(errore_di_ambiente)?;
    }
    Ok(dataset)
}

// Seam di test: osserva che ogni invocazione usi una directory diversa,
// anche quando riscrivere la stessa directory darebbe gli stessi risultati.
#[cfg(all(test, feature = "gdal-backend"))]
thread_local! {
    static ULTIMA_DIRECTORY: std::cell::RefCell<Vec<std::path::PathBuf>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

/// Entry point non stabile: il byte iniziale seleziona una parte modulo il
/// numero di membri; i byte restanti la sostituiscono. Input vuoto = fixture
/// intatta. Ogni invocazione usa una directory nuova e drena tutte le righe.
/// Un rifiuto tipizzato e' atteso; panico, abort o consegna parziale sono finding.
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
    fn open(&self, source: Source, opts: ReadOptions) -> Result<Box<dyn OpenDatasetHandle>> {
        self.open_selected(source, opts, None)
    }

    #[cfg_attr(not(feature = "gdal-backend"), allow(unused_mut, unused_variables))]
    fn open_selected(
        &self,
        source: Source,
        mut opts: ReadOptions,
        selected: Option<plenora_io_model::contract::LayerId>,
    ) -> Result<Box<dyn OpenDatasetHandle>> {
        #[cfg(feature = "gdal-backend")]
        {
            radici::applica();
            let path = driver_common::structure::preflight(self.descriptor(), source, &mut opts)?;
            let dataset = backend::open_selected(&path, opts.assume_crs.as_deref(), selected)?;
            plenora_io_core::with_read_budget(dataset, &opts, false)
        }
        #[cfg(not(feature = "gdal-backend"))]
        {
            let _ = driver_common::structure::preflight(self.descriptor(), source, &mut opts)?;
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
        if !cfg!(feature = "gdal-backend") {
            plenora_io_model::format_options::valida_opzioni(
                self.descriptor().id(),
                self.descriptor().format_options(),
                &opts.format_options,
                plenora_io_model::format_options::FaseOpzione::Scrittura,
            )?;
            return Err(plenora_io_model::error_table::Situation::MissingWriteBackend.error());
        }
        validate_write(
            self.descriptor(),
            plan,
            opts.max_columns(),
            &opts.format_options,
        )?;
        #[cfg(feature = "gdal-backend")]
        {
            let Sink::Path(path) = sink;
            self.descriptor().files().ensure_absent(&path)?;
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
    #[cfg(test)]
    use std::fs::TryLockError;
    use std::fs::{File, OpenOptions};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Arc;

    use arrow_array::builder::{
        BinaryBuilder, Date32Builder, Float64Builder, Int32Builder, Int64Builder, StringBuilder,
        TimestampMicrosecondBuilder,
    };
    use arrow_array::{
        ArrayRef, BinaryArray, Float64Array, Int32Array, RecordBatch, RecordBatchOptions,
        StringArray,
    };
    use arrow_schema::{Field, Schema, SchemaRef};
    use gdal::vector::LayerAccess;
    use gdal::{Dataset, Metadata};

    #[cfg(test)]
    use driver_common::geometry_field;
    use plenora_io_core::driver::{
        spawn_batch_reader, BatchEmitter, LayerReader, OpenDatasetHandle,
    };
    use plenora_io_core::request::ReadRequest;
    use plenora_io_model::contract::{
        CoordinateDimensions, DataContract, FieldId, GeometryColumnContract, GeometryType,
        LayerContract, LayerId,
    };
    use plenora_io_model::crs::{CrsKind, RawCrs, ResolvedCrs};
    use plenora_io_model::{
        CapabilityReason, NumeroStrutturale, PlenoraIoError, PublicMessage, Result,
    };

    pub fn runtime_available() -> bool {
        install_diagnostics();
        let Ok(driver) = DriverManager::get_driver_by_name("OpenFileGDB") else {
            return false;
        };
        ["DCAP_VECTOR", "DCAP_OPEN", "DCAP_CREATE"]
            .into_iter()
            .all(|capability| driver.metadata_item(capability, "") == Some("YES".to_owned()))
    }

    #[derive(Clone, Default)]
    struct Diagnostics(std::sync::Arc<std::sync::atomic::AtomicU64>);
    thread_local! {
        static DIAGNOSTICS: std::cell::RefCell<Option<Diagnostics>> = const { std::cell::RefCell::new(None) };
    }
    struct DiagnosticScope(Option<Diagnostics>);
    impl Drop for DiagnosticScope {
        fn drop(&mut self) {
            DIAGNOSTICS.with(|slot| *slot.borrow_mut() = self.0.take());
        }
    }
    impl Diagnostics {
        fn record_warning(&self) {
            let mut count = self.0.load(Ordering::Relaxed);
            while count != u64::MAX {
                match self.0.compare_exchange_weak(
                    count,
                    count + 1,
                    Ordering::Relaxed,
                    Ordering::Relaxed,
                ) {
                    Ok(_) => break,
                    Err(observed) => count = observed,
                }
            }
        }
        fn enter(&self) -> DiagnosticScope {
            install_diagnostics();
            DiagnosticScope(DIAGNOSTICS.with(|slot| slot.replace(Some(self.clone()))))
        }
        fn assessment(&self) -> plenora_io_core::FidelityAssessment {
            let mut assessment = plenora_io_core::FidelityAssessment::lossless();
            let count = self.0.load(std::sync::atomic::Ordering::Relaxed);
            if count != 0 {
                assessment.add_reason(
                    plenora_io_core::loss::FidelityReasonCode::FormatConstraint,
                    format!("diagnostica libreria GDAL: classe warning; conteggio {count}; testo nativo redatto"),
                );
            }
            assessment
        }
    }
    fn with_native_diagnostics<T>(operation: impl FnOnce() -> T) -> T {
        install_diagnostics();
        operation()
    }
    fn install_diagnostics() {
        static INSTALL: std::sync::Once = std::sync::Once::new();
        INSTALL.call_once(|| {
            gdal::config::set_error_handler(|severity, _, _| {
                if severity == gdal::errors::CplErrType::Warning {
                    DIAGNOSTICS.with(|slot| {
                        if let Some(diagnostics) = slot.borrow().as_ref() {
                            diagnostics.record_warning();
                        }
                    });
                }
            });
        });
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
    use plenora_io_core::loss_table::{
        native_write_report, observe_native, NativeDriver, NativeElement, TransportDirection,
    };

    use plenora_io_core::publish::publish_dir_atomic;
    use plenora_io_core::{SingleReaderGate, WriteLayer, WritePlan};

    const OGR_FIELD_TYPE_KEY: &str = "plenora.filegdb.ogr_field_type";
    const OGR_FIELD_WIDTH_KEY: &str = "plenora.filegdb.width";
    const OGR_FIELD_PRECISION_KEY: &str = "plenora.filegdb.precision";
    const STAGING_MARKER: &str = plenora_io_core::dataset_files::FILEGDB_STAGING_MARKER;
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
                _ => {
                    let mut error =
                        plenora_io_core::capabilities::unsupported_arrow_type(field.data_type());
                    error.driver = Some("filegdb".to_owned());
                    error.capability_reason = Some(CapabilityReason::TypeNotRepresentable);
                    return Err(error);
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
            DESCRIPTOR.files().ensure_absent(dest)?;
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
        diagnostics: Diagnostics,
        metadata_check: plenora_io_core::property_plan::NativeMetadataCheck,
        loss: LossReport,
        ds: Option<Dataset>,
        staging: StagingGuard,
        dest: PathBuf,
        durable: bool,
        max_output_bytes: u64,
        /// Scadenza e cancellazione, per l'ultimo controllo prima del rename.
        contesto: plenora_io_model::budget::PipelineContext,
        layers: Vec<PlanLayer>,
        geometry_source: std::fs::File,
        geometry_count: u64,
        geometry_spill: Vec<plenora_io_model::budget::SpillLease>,
        geometry_written: u64,
        geometry_reserved: u64,
        wkb_limits: plenora_io_model::limits::WkbLimits,
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
            // Lo stesso confronto di `--assume-crs` (GEO-005): autorita'
            // senza maiuscole, codice come testo.
            if !plenora_io_model::crs::stesso_identificatore(expected, &actual) {
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

    #[cfg(test)]
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
        let diagnostics = Diagnostics::default();
        let _scope = diagnostics.enter();
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
            diagnostics,
            metadata_check: plenora_io_core::property_plan::NativeMetadataCheck::new(plan, opts),
            loss: native_write_report(NativeDriver::FileGdb, plan),
            ds: Some(ds),
            staging,
            dest: path.to_owned(),
            durable: opts.durable,
            max_output_bytes: opts.max_output_bytes(),
            contesto: opts.budget().context().clone(),
            layers: infos,
            geometry_source: tempfile::tempfile_in(
                plenora_io_core::driver::spool::spill_directory()?,
            )?,
            geometry_count: 0,
            geometry_spill: Vec::new(),
            geometry_written: 0,
            geometry_reserved: 0,
            wkb_limits: opts.wkb_limits(),
        }))
    }

    impl GdbWriter {
        fn compare_written_geometry(&mut self) -> Result<()> {
            use plenora_io_model::wkb::decode_wkb;
            use std::io::{Read, Seek};
            self.geometry_source.rewind()?;
            let dataset = Dataset::open(self.staging.path()).map_err(|_| {
                err(&PublicMessage::Curated(
                    "rilettura del FileGDB scritto fallita",
                ))
            })?;
            for _ in 0..self.geometry_count {
                self.contesto.ensure_active()?;
                let mut integer = || -> Result<u64> {
                    let mut bytes = [0; 8];
                    self.geometry_source.read_exact(&mut bytes)?;
                    Ok(u64::from_le_bytes(bytes))
                };
                let layer = integer()?;
                let fid = integer()?;
                let length = integer()?;
                let mut source = vec![
                    0;
                    usize::try_from(length).map_err(|_| {
                        err(&PublicMessage::Curated(
                            "dimensione geometria non rappresentabile",
                        ))
                    })?
                ];
                self.geometry_source.read_exact(&mut source)?;
                let layer_index = usize::try_from(layer).map_err(|_| {
                    err(&PublicMessage::Curated("indice layer non rappresentabile"))
                })?;
                let info = &self.layers[layer_index];
                let native = dataset.layer(info.gdal_idx).map_err(|_| {
                    err(&PublicMessage::Curated(
                        "rilettura del layer scritto fallita",
                    ))
                })?;
                let feature = native.feature(fid).ok_or_else(|| {
                    err(&PublicMessage::Curated(
                        "feature scritta assente alla rilettura",
                    ))
                })?;
                let before = decode_wkb(&source, &self.wkb_limits)?;
                let geometry = match feature.geometry_by_index(0) {
                    Ok(geometry) => Some(geometry),
                    Err(GdalError::NullPointer {
                        method_name: "OGR_F_GetGeomFieldRef",
                        ..
                    }) => None,
                    Err(_) => {
                        return Err(err(&PublicMessage::Curated(
                            "lettura della geometria scritta fallita",
                        )))
                    }
                };
                let Some(geometry) = geometry else {
                    observe_native(
                        &mut self.loss,
                        (NativeDriver::FileGdb, TransportDirection::Write),
                        NativeElement::EmptyGeometry,
                        1,
                        layer,
                        Some(info.geom_idx as u64),
                    );
                    continue;
                };
                let bytes = geometry.iso_wkb().map_err(|_| {
                    err(&PublicMessage::Curated(
                        "lettura WKB della geometria scritta fallita",
                    ))
                })?;
                let after = decode_wkb(&bytes, &self.wkb_limits)?;
                let (coordinates_before, orientation_before) = geometry_signature(&before);
                let (coordinates_after, orientation_after) = geometry_signature(&after);
                for (element, changed) in [
                    (
                        NativeElement::CoordinatePrecision,
                        coordinates_before != coordinates_after,
                    ),
                    (
                        NativeElement::RingOrientation,
                        orientation_before != orientation_after,
                    ),
                ] {
                    observe_native(
                        &mut self.loss,
                        (NativeDriver::FileGdb, TransportDirection::Write),
                        element,
                        u64::from(changed),
                        layer,
                        Some(info.geom_idx as u64),
                    );
                }
            }
            Ok(())
        }
    }

    // Coordinate multisets are insensitive to ring reversal. Ring signs are
    // compared separately, so orientation alone never reports quantization.
    fn geometry_signature(
        geometry: &plenora_io_model::wkb::WkbGeometry,
    ) -> (Vec<[Option<u64>; 4]>, Vec<i8>) {
        use plenora_io_model::wkb::{WkbCoordinate, WkbGeometry, WkbValue};
        fn visit(g: &WkbGeometry, coordinates: &mut Vec<[Option<u64>; 4]>, signs: &mut Vec<i8>) {
            let mut push = |c: &WkbCoordinate| {
                coordinates.push([
                    Some(c.x.to_bits()),
                    Some(c.y.to_bits()),
                    c.z.map(f64::to_bits),
                    c.m.map(f64::to_bits),
                ]);
            };
            match &g.value {
                WkbValue::Point(c) => push(c),
                WkbValue::LineString(cs) | WkbValue::CircularString(cs) => cs.iter().for_each(push),
                WkbValue::Polygon(rings) | WkbValue::Triangle(rings) => {
                    for ring in rings {
                        ring.iter().for_each(&mut push);
                        let area: f64 = ring
                            .windows(2)
                            .map(|p| p[1].x.mul_add(-p[0].y, p[0].x * p[1].y))
                            .sum();
                        signs.push(if area > 0.0 {
                            1
                        } else if area < 0.0 {
                            -1
                        } else {
                            0
                        });
                    }
                }
                WkbValue::MultiPoint(gs)
                | WkbValue::MultiLineString(gs)
                | WkbValue::MultiPolygon(gs)
                | WkbValue::GeometryCollection(gs)
                | WkbValue::CompoundCurve(gs)
                | WkbValue::CurvePolygon(gs)
                | WkbValue::MultiCurve(gs)
                | WkbValue::MultiSurface(gs)
                | WkbValue::PolyhedralSurface(gs)
                | WkbValue::Tin(gs) => {
                    for child in gs {
                        visit(child, coordinates, signs);
                    }
                }
            }
        }
        let mut coordinates = Vec::new();
        let mut signs = Vec::new();
        visit(geometry, &mut coordinates, &mut signs);
        coordinates.sort_unstable();
        (coordinates, signs)
    }

    impl FormatWriter for GdbWriter {
        fn write(&mut self, batch: &RecordBatch) -> Result<()> {
            self.write_to_layer(LayerId(0), batch)
        }

        fn write_to_layer(&mut self, layer: LayerId, batch: &RecordBatch) -> Result<()> {
            let _scope = self.diagnostics.enter();
            let li = layer.0 as usize;
            if li >= self.layers.len() {
                return Err(plenora_io_model::error_table::Situation::MissingLayer.error());
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
            // Dalla 0.19 `set_field` prende l'indice OGR invece del nome. Si
            // risolve **una volta** per layer e non per cella: la definizione
            // non cambia durante la scrittura, e chiedere lo stesso nome a
            // ogni riga per ogni campo sarebbe una ricerca per stringa dentro
            // il ciclo piu' interno.
            let indici_ogr = fields
                .iter()
                .map(|field| {
                    gl.defn().field_index(&field.name).map_err(|_| {
                        err(&PublicMessage::Curated(
                            "campo assente nella definizione del layer FileGDB",
                        ))
                    })
                })
                .collect::<Result<Vec<usize>>>()?;
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
                for (field, ogr_index) in fields.iter().zip(&indici_ogr) {
                    match field_value(field.kind, batch.column(field.index), row)? {
                        Some(value) => feature.set_field(*ogr_index, &value).map_err(|_| {
                            err(&PublicMessage::Curated("scrittura di un campo fallita"))
                        })?,
                        None => feature.set_field_null(*ogr_index).map_err(|_| {
                            err(&PublicMessage::Curated(
                                "scrittura di un campo nullo fallita",
                            ))
                        })?,
                    }
                }
                feature
                    .create(&gl)
                    .map_err(|_| err(&PublicMessage::Curated("scrittura della feature fallita")))?;
                if !geom_col.is_null(row) {
                    use std::io::Write;
                    let bytes = geom_col.value(row).len() as u64 + 24;
                    let needed = self.geometry_written.saturating_add(bytes);
                    if needed > self.geometry_reserved {
                        let missing = needed - self.geometry_reserved;
                        let lease = self
                            .contesto
                            .lease_spill(missing.max(1024 * 1024))
                            .or_else(|_| self.contesto.lease_spill(missing))?;
                        self.geometry_reserved += lease.bytes();
                        self.geometry_spill.push(lease);
                    }
                    self.geometry_written = needed;
                    let fid = feature.fid().ok_or_else(|| {
                        err(&PublicMessage::Curated(
                            "identita' della feature scritta assente",
                        ))
                    })?;
                    for value in [u64::from(layer.0), fid, geom_col.value(row).len() as u64] {
                        self.geometry_source.write_all(&value.to_le_bytes())?;
                    }
                    self.geometry_source.write_all(geom_col.value(row))?;
                    self.geometry_count += 1;
                }
            }
            #[cfg(test)]
            crash_failpoint("after_write");
            Ok(())
        }

        fn finish(mut self: Box<Self>) -> Result<Published> {
            let _scope = self.diagnostics.enter();
            let ds = self
                .ds
                .take()
                .ok_or_else(|| err(&PublicMessage::Curated("dataset writer già chiuso")))?;
            drop(ds); // chiude e flush della .gdb
            self.compare_written_geometry()?;
            self.loss.merge(&self.metadata_check.compare(
                &crate::FileGdbDriver,
                plenora_io_core::loss_table::NativeDriver::FileGdb,
                self.staging.path(),
            )?);
            let bytes = dir_size(self.staging.path());
            if bytes > self.max_output_bytes {
                // Misurata dopo la chiusura della .gdb: una scadenza passata
                // li' viene prima della quota.
                return Err(plenora_io_core::driver::limite_o_scadenza(
                    &self.contesto,
                    self.contesto
                        .output_quota(self.max_output_bytes)
                        .exceeded_measured(
                            plenora_io_model::ErrorPhase::Finalize,
                            bytes,
                            self.max_output_bytes,
                        ),
                    plenora_io_model::ErrorPhase::Finalize,
                ));
            }
            #[cfg(test)]
            crash_failpoint("before_publish");
            DESCRIPTOR.files().ensure_absent_except_staging(
                &self.dest,
                &[self.staging.path(), &self.staging.lock_path],
            )?;
            let outcome = publish_dir_atomic(
                self.staging.path(),
                &self.dest,
                self.durable,
                &self.contesto,
            )?;
            #[cfg(test)]
            crash_failpoint("after_publish");
            self.staging.disarm();
            Ok(Published {
                bytes,
                loss: self.loss.clone(),
                fidelity: self.diagnostics.assessment(),
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
        // Dalla 0.19 `auth_name` restituisce `Option`, non `Result`: l'assenza
        // di un'autorita' non e' un guasto, ed e' giusto che il tipo lo dica.
        // `auth_code` resta un `Result` -- upstream lo segnala come da
        // correggere -- quindi il quartetto non e' simmetrico.
        match (spatial_ref.auth_name(), spatial_ref.auth_code()) {
            (Some(authority), Ok(code)) => {
                Some(format!("{}:{code}", authority.to_ascii_uppercase()))
            }
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
        // Il FileGDB memorizza x poi y, e x e' easting o longitudine, qualunque
        // sia l'ordine con cui GDAL riporta gli assi dell'autorita': quello e'
        // l'ordine del registro (EPSG:4326 elenca la latitudine prima), e
        // GEO-002 vieta di ricavarne la dichiarazione. Fino alla 5.x lo si
        // leggeva da `axis_name`, e un EPSG:4326 usciva `lat_lon` con i
        // valori in longitudine prima. Il WKT di GDAL dice ancora se gli assi
        // sono una coppia est/nord: una polare (EPSG:3031) e' `other`.
        Ok(ResolvedCrs::new(id, kind, Some(definition)).stored_east_first())
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

    /// Apre con il solo `OpenFileGDB`: una directory `.gdb` contenente CSV
    /// non deve attivare implicitamente un altro driver GDAL.
    fn apri_solo_filegdb(path: &std::path::Path) -> gdal::errors::Result<Dataset> {
        with_native_diagnostics(|| {
            Dataset::open_ex(
                path,
                gdal::DatasetOptions {
                    open_flags: gdal::GdalOpenFlags::GDAL_OF_READONLY
                        | gdal::GdalOpenFlags::GDAL_OF_VECTOR,
                    allowed_drivers: Some(&["OpenFileGDB"]),
                    open_options: None,
                    sibling_files: None,
                },
            )
        })
    }

    struct CatalogObjects {
        objects: Vec<(u64, String)>,
        metadata: Vec<u64>,
    }

    fn catalog_objects(path: &std::path::Path) -> Result<CatalogObjects> {
        let catalog = apri_solo_filegdb(&driver_common::filegdb_names::resolve(
            path,
            "a00000002.gdbtable",
        )?)
        .map_err(|_| err(&PublicMessage::Curated("catalogo FileGDB non leggibile")))?;
        let mut layer = catalog
            .layer(0)
            .map_err(|_| err(&PublicMessage::Curated("catalogo FileGDB assente")))?;
        let mut objects = Vec::new();
        let mut metadata = Vec::new();
        for feature in layer.features() {
            let id = feature
                .fid()
                .ok_or_else(|| err(&PublicMessage::Curated("identita catalogo FileGDB assente")))?;
            let name = feature
                .field_as_string(0)
                .map_err(|_| err(&PublicMessage::Curated("voce catalogo FileGDB non valida")))?
                .ok_or_else(|| {
                    err(&PublicMessage::Curated(
                        "nome oggetto catalogo FileGDB assente",
                    ))
                })?;
            if matches!(
                name.as_str(),
                "GDB_Items" | "GDB_FeatureClasses" | "GDB_ObjectClasses"
            ) {
                metadata.push(id);
            }
            if !matches!(
                name.as_str(),
                "GDB_FeatureClasses"
                    | "GDB_ObjectClasses"
                    | "GDB_SystemCatalog"
                    | "GDB_DBTune"
                    | "GDB_SpatialRefs"
                    | "GDB_Items"
                    | "GDB_ItemTypes"
                    | "GDB_ItemRelationships"
                    | "GDB_ItemRelationshipTypes"
                    | "GDB_ReplicaLog"
            ) {
                objects.push((id, name));
            }
        }
        objects.sort_by_key(|(id, _)| *id);
        Ok(CatalogObjects { objects, metadata })
    }

    fn internal_file_error(reason: plenora_io_model::CapabilityReason) -> PlenoraIoError {
        PlenoraIoError::capability_redatta(
            "filegdb",
            None,
            reason,
            &PublicMessage::Curated(
                "oggetto FileGDB non leggibile; file interno assente, vuoto o non rappresentabile",
            ),
        )
    }

    #[cfg(test)]
    pub fn open(
        path: &std::path::Path,
        assume_crs: Option<&str>,
    ) -> Result<Box<dyn OpenDatasetHandle>> {
        open_selected(path, assume_crs, None)
    }

    pub fn open_selected(
        path: &std::path::Path,
        assume_crs: Option<&str>,
        selected: Option<LayerId>,
    ) -> Result<Box<dyn OpenDatasetHandle>> {
        install_diagnostics();
        with_native_diagnostics(|| open_objects(path, assume_crs, selected))
    }

    // Each native handle sees its own physical dependencies, even with canonical input.
    fn canonical_view(
        path: &std::path::Path,
        operation: driver_common::filegdb_names::Operation<'_>,
        catalog: Option<&std::path::Path>,
    ) -> Result<std::sync::Arc<tempfile::TempDir>> {
        let view = tempfile::Builder::new()
            .prefix("plenora-filegdb-")
            .suffix(".gdb")
            .tempdir()?;
        let members = driver_common::filegdb_names::required(path, operation)?;
        for (source, member) in members {
            let shared = match operation {
                driver_common::filegdb_names::Operation::Catalog => true,
                driver_common::filegdb_names::Operation::Object { catalog, .. } => member
                    .id
                    .is_some_and(|id| id == 1 || catalog.contains(&u64::from(id))),
            };
            let source = if shared {
                catalog.map_or_else(|| source.clone(), |root| root.join(&member.canonical))
            } else {
                source
            };
            // Decode the catalog as a standalone table: GDAL otherwise loads
            // GDB_Items before exposing its rows. The private ID has no siblings.
            let destination =
                if matches!(operation, driver_common::filegdb_names::Operation::Catalog) {
                    member.canonical.replacen("a00000001", "a00000002", 1)
                } else {
                    member.canonical
                };
            if !std::fs::symlink_metadata(&source)?.is_file() {
                return Err(err(&PublicMessage::Curated("membro FileGDB non regolare")));
            }
            std::fs::copy(source, view.path().join(destination))?;
        }
        Ok(std::sync::Arc::new(view))
    }

    #[allow(clippy::too_many_lines)]
    fn open_objects(
        path: &std::path::Path,
        assume_crs: Option<&str>,
        selected: Option<LayerId>,
    ) -> Result<Box<dyn OpenDatasetHandle>> {
        let catalog = canonical_view(path, driver_common::filegdb_names::Operation::Catalog, None)
            .map_err(|_| plenora_io_model::error_table::Situation::InvalidContent.error())?;
        let diagnostics = Diagnostics::default();
        let _scope = diagnostics.enter();
        // Schema dai def GDAL, SENZA leggere feature (poi il reader streamma).
        let mut layers = Vec::new();
        let mut metas = Vec::new();
        let mut rifiuti = std::collections::BTreeMap::new();
        let CatalogObjects { objects, metadata } = catalog_objects(catalog.path())
            .map_err(|_| plenora_io_model::error_table::Situation::InvalidContent.error())?;
        for &id in &metadata {
            driver_common::structure::filegdb_object(path, id)
                .map_err(|_| plenora_io_model::error_table::Situation::InvalidContent.error())?;
        }
        let catalog = canonical_view(
            path,
            driver_common::filegdb_names::Operation::Object {
                id: 1,
                catalog: &metadata,
            },
            None,
        )
        .map_err(|_| plenora_io_model::error_table::Situation::InvalidContent.error())?;
        // Validate shared native metadata before attributing errors to objects.
        drop(
            apri_solo_filegdb(&catalog.path().join("a00000001.gdbtable"))
                .map_err(|_| plenora_io_model::error_table::Situation::InvalidContent.error())?,
        );
        let unowned_members = std::fs::read_dir(path)?.try_fold(0_u64, |count, entry| {
            let entry = entry?;
            let owned = entry
                .file_name()
                .to_str()
                .and_then(driver_common::filegdb_names::parse)
                .is_some_and(|member| {
                    member.id.is_some_and(|id| {
                        id == 1
                            || metadata.contains(&u64::from(id))
                            || objects.iter().any(|(object, _)| *object == u64::from(id))
                    })
                });
            Ok::<_, std::io::Error>(count.saturating_add(u64::from(!owned)))
        })?;
        for (i, (file_id, name)) in objects.into_iter().enumerate() {
            if selected.is_some_and(|layer| u64::from(layer.0) != i as u64) {
                continue;
            }
            let result = (|| -> Result<(LayerContract, LayerMeta)> {
                driver_common::structure::filegdb_object(path, file_id)?;
                let view = canonical_view(
                    path,
                    driver_common::filegdb_names::Operation::Object {
                        id: file_id,
                        catalog: &metadata,
                    },
                    Some(catalog.path()),
                )?;
                let ds = apri_solo_filegdb(view.path())
                    .map_err(|_| err(&PublicMessage::Curated("apertura oggetto GDAL fallita")))?;
                let gdal_names: Vec<_> = ds.layers().map(|layer| layer.name()).collect();
                let gdal_idx = gdal_names
                    .iter()
                    .position(|candidate| candidate == &name)
                    .ok_or_else(|| {
                        internal_file_error(plenora_io_model::CapabilityReason::ObjectMissing)
                    })?;
                let layer = ds.layer(gdal_idx).map_err(|_| {
                    internal_file_error(plenora_io_model::CapabilityReason::ObjectMissing)
                })?;
                // Anche le tabelle senza geometria sono valide. Quando presente,
                // la geometria precede gli attributi Arrow.
                let ogr_geometry_type = layer
                    .defn()
                    .geom_fields()
                    .next()
                    .map(|field| field.field_type());
                // Quanti campi geometrici il layer dichiara. Il contratto ne porta
                // uno, il primo: gli altri sparivano senza traccia. Ora la scoperta
                // li conta, e la lettura del layer si rifiuta (GEO-012).
                let campi_geometrici = layer.defn().geom_fields().count();
                // Il CRS si risolve solo per chi ha una geometria: pretenderlo da
                // una tabella significava chiedere un `--assume-crs` per dei dati
                // che non sono spaziali, con un messaggio che parlava di geometria.
                let crs_e_geometria = match ogr_geometry_type {
                    Some(ogr_geometry_type) => {
                        let crs = resolve_layer_crs(layer.spatial_ref(), assume_crs)?;
                        let mut geometria = geometry_contract_from_ogr(ogr_geometry_type, crs);
                        if campi_geometrici > 1 {
                            // Il limite si vede dalla scoperta, come chiede
                            // GEO-012: il numero e' strutturale, non un dato.
                            geometria.native_metadata.insert(
                                "filegdb.geometry_field_count".to_owned(),
                                campi_geometrici.to_string(),
                            );
                        }
                        Some(geometria)
                    }
                    None => None,
                };
                let (mut loss, time_in_utc, native_order) =
                    native_definition_report(&ds, &name, i as u64)?;
                let empty_feature = Feature::new(layer.defn()).map_err(|_| {
                    err(&PublicMessage::Curated(
                        "lettura schema temporale FileGDB fallita",
                    ))
                })?;
                let native_fields: Vec<(LayerFieldMeta, Field)> = layer
                    .defn()
                    .fields()
                    .enumerate()
                    .map(|(field_index, field)| {
                        observe_native(
                            &mut loss,
                            (NativeDriver::FileGdb, TransportDirection::Read),
                            NativeElement::NotNull,
                            u64::from(!field.is_nullable()),
                            i as u64,
                            Some(field_index as u64 + u64::from(ogr_geometry_type.is_some())),
                        );
                        let name = field.name();
                        let field_type = field.field_type();
                        let mut data_type = ogr_to_arrow(field_type)?;
                        let mut timezone_metadata = None;
                        if field_type == gdal::vector::OGRFieldType::OFTDateTime {
                            let raw_flag = empty_feature
                                .field_datetime_timezone_flag(field_index)
                                .map_err(|_| {
                                    err(&PublicMessage::Curated("lettura fuso FileGDB fallita"))
                                })?;
                            let mut flag = schema_timezone_flag(raw_flag, time_in_utc)?;
                            // GDAL 3.9 exposes the row flag but leaves the field flag
                            // unknown. Its native layer definition carries IsTimeInUTC.
                            timezone_metadata = raw_flag.map(|flag| flag.to_string());
                            if flag == 0 && time_in_utc == Some(true) {
                                flag = 100;
                            }
                            data_type = DataType::Timestamp(
                                arrow_schema::TimeUnit::Microsecond,
                                ogr_timezone(flag)?,
                            );
                        }
                        Ok({
                            let mut metadata = HashMap::from([
                                (OGR_FIELD_TYPE_KEY.to_owned(), field_type.to_string()),
                                (OGR_FIELD_WIDTH_KEY.to_owned(), field.width().to_string()),
                                (
                                    OGR_FIELD_PRECISION_KEY.to_owned(),
                                    field.precision().to_string(),
                                ),
                            ]);
                            if let Some(flag) = timezone_metadata {
                                metadata.insert("filegdb.ogr_timezone_flag".to_owned(), flag);
                            }
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
                    Some(geometry) => {
                        let geometry_arrow_field = super::native_geometry_field(&geometry);
                        let mut campi = vec![geometry_arrow_field];
                        campi.extend(attribute_arrow_fields);
                        (campi, Some(geometry))
                    }
                    // Una tabella: gli attributi e basta, senza una colonna
                    // geometrica davanti. E' anche cio' che sposta gli indici, e il
                    // reader lo rilegge da `ha_geometria`.
                    None => (attribute_arrow_fields, None),
                };
                let delivered_names = arrow_fields
                    .iter()
                    .map(|f| f.name().as_str())
                    .collect::<Vec<_>>();
                // OID is GDAL's separate feature identity, not an attribute.
                // Compare the relative order of the fields actually delivered.
                let native_geometry_name =
                    layer.defn().geom_fields().next().map(|field| field.name());
                let order_changed = native_order
                    .iter()
                    .map(String::as_str)
                    .map(|name| {
                        if Some(name) == native_geometry_name.as_deref() {
                            GEOMETRY
                        } else {
                            name
                        }
                    })
                    .filter(|name| delivered_names.contains(name))
                    .ne(delivered_names.iter().copied());
                let schema: SchemaRef = Arc::new(Schema::new(arrow_fields));
                let ha_geometria = geometry.is_some();
                let contract = DataContract::new(
                    plenora_io_model::geometry::with_contract_version(schema),
                    geometry,
                );
                plenora_io_model::primary::reconcile_schema_contract(&contract)?;
                // Indice di layer di un FileGDB: il formato ne ammette al piu'
                // qualche migliaio, il cast a u32 non puo' troncare.
                #[allow(clippy::cast_possible_truncation)]
                let layer_id = LayerId(i as u32);
                let contract = LayerContract {
                    id: layer_id,
                    name: layer.name(),
                    contract,
                };
                observe_native(
                    &mut loss,
                    (NativeDriver::FileGdb, TransportDirection::Read),
                    NativeElement::FieldOrder,
                    u64::from(order_changed),
                    i as u64,
                    Some(0),
                );
                let meta = LayerMeta {
                    view: Some(view),
                    loss,
                    gdal_idx,
                    fields,
                    ha_geometria,
                    campi_geometrici,
                };
                Ok((contract, meta))
            })();
            let (contract, meta) = match result {
                Ok(value) => value,
                Err(error) => {
                    let id = LayerId(u32::try_from(i).map_err(|_| {
                        plenora_io_model::quota::Quota::InputEntries
                            .exceeded(plenora_io_model::ErrorPhase::Read)
                    })?);
                    rifiuti.insert(id.0, error);
                    (
                        LayerContract {
                            id,
                            name: name.clone(),
                            contract: DataContract::new(Arc::new(Schema::empty()), None),
                        },
                        LayerMeta {
                            view: None,
                            loss: LossReport::default(),
                            gdal_idx: i,
                            fields: Vec::new(),
                            ha_geometria: false,
                            campi_geometrici: 0,
                        },
                    )
                }
            };
            layers.push(contract);
            metas.push(meta);
        }
        Ok(Box::new(GdbDataset {
            unowned_members,
            diagnostics,
            layers,
            metas,
            rifiuti,
            reader_gate: SingleReaderGate::new(DESCRIPTOR.id()),
        }))
    }

    fn native_definition_report(
        ds: &Dataset,
        name: &str,
        layer: u64,
    ) -> Result<(LossReport, Option<bool>, Vec<String>)> {
        let mut report = LossReport::default();
        let mut time_in_utc = None;
        let mut field_order = Vec::new();
        let query = format!("GetLayerDefinition {name}");
        let mut result = ds
            .execute_sql(&query, None, gdal::vector::sql::Dialect::DEFAULT)
            .map_err(|_| {
                err(&PublicMessage::Curated(
                    "definizione nativa FileGDB non leggibile",
                ))
            })?
            .ok_or_else(|| {
                err(&PublicMessage::Curated(
                    "definizione nativa FileGDB assente",
                ))
            })?;
        for feature in result.features() {
            let definition = feature.field_as_string(0).map_err(|_| {
                err(&PublicMessage::Curated(
                    "definizione nativa FileGDB non testuale",
                ))
            })?;
            if let Some(definition) = definition {
                observe_definition_xml(&mut report, &definition, layer)?;
                time_in_utc = definition_time_in_utc(&definition)?;
                field_order = definition_field_order(&definition)?;
            }
        }
        drop(result);
        if let Some(mut metadata) = ds
            .execute_sql(
                format!("GetLayerMetadata {name}"),
                None,
                gdal::vector::sql::Dialect::DEFAULT,
            )
            .map_err(|_| temporal_error())?
        {
            for feature in metadata.features() {
                if feature
                    .field_as_string(0)
                    .map_err(|_| temporal_error())?
                    .is_some_and(|xml| !xml.trim().is_empty())
                {
                    observe_native(
                        &mut report,
                        (NativeDriver::FileGdb, TransportDirection::Read),
                        NativeElement::LayerMetadata,
                        1,
                        layer,
                        None,
                    );
                }
            }
        }
        Ok((report, time_in_utc, field_order))
    }

    fn definition_field_order(xml: &str) -> Result<Vec<String>> {
        use quick_xml::events::Event;
        let mut reader = quick_xml::Reader::from_str(xml);
        let mut stack = Vec::new();
        let mut names = Vec::new();
        loop {
            match reader.read_event() {
                Ok(Event::Start(tag))
                    if tag.local_name().as_ref() == "Name"
                        && stack.last().is_some_and(|name: &String| {
                            matches!(name.as_str(), "Field" | "GPFieldInfoEx")
                        }) =>
                {
                    let text = reader.read_text(tag.name()).map_err(|_| temporal_error())?;
                    names.push(
                        quick_xml::escape::unescape(&text)
                            .map_err(|_| temporal_error())?
                            .into_owned(),
                    );
                }
                Ok(Event::Start(tag)) => stack.push(tag.local_name().as_ref().to_owned()),
                Ok(Event::End(_)) => {
                    stack.pop();
                }
                Ok(Event::Eof) => return Ok(names),
                Err(_) => {
                    return Err(err(&PublicMessage::Curated(
                        "XML nativo FileGDB non valido",
                    )))
                }
                _ => {}
            }
        }
    }

    fn definition_time_in_utc(xml: &str) -> Result<Option<bool>> {
        use quick_xml::events::Event;
        let mut reader = quick_xml::Reader::from_str(xml);
        let mut result = None;
        loop {
            match reader.read_event() {
                Ok(Event::Start(element)) if element.local_name().as_ref() == "IsTimeInUTC" => {
                    let text = reader
                        .read_text(element.name())
                        .map_err(|_| temporal_error())?;
                    let value = match text.trim() {
                        "true" | "1" => true,
                        "false" | "0" => false,
                        _ => return Err(temporal_error()),
                    };
                    if result.is_some_and(|previous| previous != value) {
                        return Err(temporal_error());
                    }
                    result = Some(value);
                }
                Ok(Event::Eof) => return Ok(result),
                Err(_) => return Err(temporal_error()),
                _ => {}
            }
        }
    }

    fn observe_definition_xml(report: &mut LossReport, xml: &str, layer: u64) -> Result<()> {
        use quick_xml::events::Event;
        let mut reader = quick_xml::Reader::from_str(xml);
        let mut stack = Vec::new();
        loop {
            match reader.read_event() {
                Ok(Event::Start(tag)) if tag.local_name().as_ref() == "ExtensionProperties" => {
                    let payload = reader.read_text(tag.name()).map_err(|_| temporal_error())?;
                    observe_native(
                        report,
                        (NativeDriver::FileGdb, TransportDirection::Read),
                        NativeElement::Extensions,
                        u64::from(!payload.trim().is_empty()),
                        layer,
                        None,
                    );
                }
                Ok(Event::Start(tag)) if tag.local_name().as_ref() == "CatalogPath" => {
                    let path = reader.read_text(tag.name()).map_err(|_| temporal_error())?;
                    let depth = path.split('\\').filter(|part| !part.is_empty()).count();
                    observe_native(
                        report,
                        (NativeDriver::FileGdb, TransportDirection::Read),
                        NativeElement::Hierarchy,
                        u64::from(depth > 1),
                        layer,
                        None,
                    );
                }
                Ok(Event::Start(tag)) => {
                    let element = match tag.local_name().as_ref() {
                        "DomainName" => Some(NativeElement::Domains),
                        "AliasName" => Some(NativeElement::Aliases),
                        "Subtype" => Some(NativeElement::Subtypes),
                        _ => None,
                    };
                    if element == Some(NativeElement::Subtypes) {
                        observe_native(
                            report,
                            (NativeDriver::FileGdb, TransportDirection::Read),
                            NativeElement::Subtypes,
                            1,
                            layer,
                            None,
                        );
                    }
                    stack.push(element);
                }
                Ok(event @ (Event::Text(_) | Event::CData(_) | Event::GeneralRef(_))) => {
                    let present = match event {
                        Event::Text(text) => !text.as_ref().is_empty(),
                        Event::CData(text) => !text.as_ref().is_empty(),
                        Event::GeneralRef(_) => true,
                        _ => false,
                    };
                    if let Some(element @ (NativeElement::Domains | NativeElement::Aliases)) =
                        stack.last_mut().filter(|_| present).and_then(Option::take)
                    {
                        observe_native(
                            report,
                            (NativeDriver::FileGdb, TransportDirection::Read),
                            element,
                            1,
                            layer,
                            None,
                        );
                    }
                }
                Ok(Event::End(_)) => {
                    stack.pop();
                }
                Ok(Event::Eof) => break,
                Err(_) => {
                    return Err(err(&PublicMessage::Curated(
                        "XML nativo FileGDB non valido",
                    )))
                }
                _ => {}
            }
        }
        Ok(())
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
        } else if ft == OGRFieldType::OFTBinary {
            Ok(DataType::Binary)
        } else if ft == OGRFieldType::OFTDate {
            Ok(DataType::Date32)
        } else if ft == OGRFieldType::OFTDateTime {
            Ok(DataType::Timestamp(
                arrow_schema::TimeUnit::Microsecond,
                None,
            ))
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

    fn schema_timezone_flag(raw: Option<i32>, time_in_utc: Option<bool>) -> Result<i32> {
        raw.or_else(|| time_in_utc.map(|utc| if utc { 100 } else { 0 }))
            .ok_or_else(|| PlenoraIoError::non_supportato_redatto(
                &PublicMessage::Curated("GDAL non espone il fuso del campo e la definizione FileGDB non dichiara IsTimeInUTC"),
            ))
    }

    fn ogr_timezone(flag: i32) -> Result<Option<Arc<str>>> {
        match flag {
            0 | 1 => Ok(None),
            100 => Ok(Some("UTC".into())),
            3..=199 => {
                let minutes = (flag - 100) * 15;
                Ok(Some(
                    format!(
                        "{}{:02}:{:02}",
                        if minutes < 0 { '-' } else { '+' },
                        minutes.abs() / 60,
                        minutes.abs() % 60
                    )
                    .into(),
                ))
            }
            _ => Err(err(&PublicMessage::Curated(
                "fusi OGR misti o non rappresentabili nello stesso campo Arrow",
            ))),
        }
    }

    struct LayerFieldMeta {
        name: String,
        data_type: DataType,
        ogr_type: gdal::vector::OGRFieldType::Type,
    }

    struct ProjectedField {
        /// L'indice OGR del campo.
        ///
        /// Era un `i32`, come lo voleva `gdal` fino alla 0.18: da li' venivano
        /// una conversione all'andata, due al ritorno e due rami d'errore per
        /// un indice negativo. Dalla 0.19 gli accessor prendono un `usize`, e
        /// quei rami non descrivono piu' niente di possibile.
        ogr_index: usize,
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
        row_limit: usize,
        presente: bool,
        inclusa: bool,
    }

    struct LayerMeta {
        view: Option<std::sync::Arc<tempfile::TempDir>>,
        loss: LossReport,
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
        /// I campi geometrici che il layer dichiara; piu' di uno rende il
        /// layer illeggibile, vedi [`super::rifiuta_piu_geometrie`].
        campi_geometrici: usize,
    }

    struct GdbDataset {
        unowned_members: u64,
        diagnostics: Diagnostics,
        layers: Vec<LayerContract>,
        metas: Vec<LayerMeta>,
        rifiuti: std::collections::BTreeMap<u32, PlenoraIoError>,
        reader_gate: SingleReaderGate,
    }

    impl OpenDatasetHandle for GdbDataset {
        fn preserves_unreadable_objects(&self) -> bool {
            true
        }
        fn layers(&self) -> &[LayerContract] {
            &self.layers
        }
        fn rifiuto_del_layer(&self, layer: LayerId) -> Option<PlenoraIoError> {
            self.rifiuti.get(&layer.0).cloned()
        }
        fn fidelity_assessment(&self) -> plenora_io_core::FidelityAssessment {
            let mut assessment = plenora_io_core::FidelityAssessment::for_format(
                DESCRIPTOR.id(),
                DESCRIPTOR.fidelity_class(),
            );
            assessment.merge(&self.diagnostics.assessment());
            if self.unowned_members != 0 {
                assessment.add_reason(
                    plenora_io_core::loss::FidelityReasonCode::FormatConstraint,
                    format!(
                        "membri FileGDB non attribuiti esclusi dalla lettura; conteggio {}",
                        self.unowned_members
                    ),
                );
            }
            assessment
        }
        fn open_layer_reader(&self, request: &ReadRequest) -> Result<Box<dyn LayerReader>> {
            if !self.layers.iter().any(|layer| layer.id == request.layer) {
                return Err(plenora_io_model::error_table::Situation::MissingLayer.error());
            }

            if let Some(error) = self.rifiuto_del_layer(request.layer) {
                return Err(error);
            }
            plenora_io_core::validate_read_projection(&DESCRIPTOR, request)?;
            let idx = self
                .layers
                .iter()
                .position(|l| l.id.0 == request.layer.0)
                .ok_or_else(|| plenora_io_model::error_table::Situation::MissingLayer.error())?;
            let m = &self.metas[idx];
            super::rifiuta_piu_geometrie(m.campi_geometrici)?;
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
                fields.push(ProjectedField {
                    ogr_index: field_index,
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
                    self.diagnostics.clone(),
                    m.view
                        .as_ref()
                        .ok_or_else(|| err(&PublicMessage::Curated("vista oggetto assente")))?
                        .path()
                        .to_owned(),
                    m.view.clone(),
                    m.gdal_idx,
                    layer_contract.contract.schema.clone(),
                    fields,
                    GeometriaDelLayer {
                        row_limit: match request.scope {
                            plenora_io_core::ReadScope::Complete => usize::MAX,
                            plenora_io_core::ReadScope::AcceptedRows(limit) => {
                                usize::try_from(limit).unwrap_or(usize::MAX)
                            }
                        },
                        presente: m.ha_geometria,
                        inclusa: include_geometry,
                    },
                    batch_sizer,
                    layer_contract,
                )
            })?;
            Ok(plenora_io_core::with_cancellation(
                plenora_io_core::loss_table::with_native_report(reader, m.loss.clone()),
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
            let actual = actual_fields.get(field.ogr_index);
            if !matches!(
                actual,
                Some((name, ogr_type)) if name == &field.name && *ogr_type == field.ogr_type
            ) {
                return Err(err(&PublicMessage::CuratedWith(
                    "schema FileGDB cambiato fra apertura e lettura, campo di indice",
                    NumeroStrutturale::Indice(driver_common::saturating_u64(field.ogr_index)),
                )));
            }
        }
        Ok(())
    }

    fn configure_read_layer(
        layer: &mut gdal::vector::Layer<'_>,
        fields: &[ProjectedField],
        geometria: GeometriaDelLayer,
    ) -> Result<()> {
        // Il worker riapre il dataset: prima di usare gli indici OGR
        // pre-risolti verifica che nessuno abbia cambiato lo schema.
        let actual_fields: Vec<_> = layer
            .defn()
            .fields()
            .map(|field| (field.name(), field.field_type()))
            .collect();
        verifica_schema_invariato(&actual_fields, fields)?;
        let selected_fields: HashSet<usize> = fields.iter().map(|field| field.ogr_index).collect();
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
        Ok(())
    }

    fn observe_feature_identity(report: &mut LossReport, feature: &Feature<'_>, layer: u64) {
        observe_native(
            report,
            (NativeDriver::FileGdb, TransportDirection::Read),
            NativeElement::FeatureId,
            u64::from(feature.fid().is_some()),
            layer,
            None,
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn spawn_reader(
        diagnostics: Diagnostics,
        path: PathBuf,
        view: Option<std::sync::Arc<tempfile::TempDir>>,
        gdal_idx: usize,
        schema: SchemaRef,
        fields: Vec<ProjectedField>,
        geometria: GeometriaDelLayer,
        mut batch_sizer: plenora_io_core::AdaptiveBatchSizer,
        contract: LayerContract,
    ) -> Result<Box<dyn LayerReader>> {
        let (loss_tx, loss_rx) = std::sync::mpsc::channel();
        let layer_index = u64::from(contract.id.0);
        let reader = spawn_batch_reader(
            DESCRIPTOR.id(),
            contract,
            2,
            move |emitter: BatchEmitter| {
                let _view = view;
                let _scope = diagnostics.enter();
                with_native_diagnostics(|| {
                    let ds = apri_solo_filegdb(&path).map_err(|_| {
                        err(&PublicMessage::Curated("apertura del FileGDB fallita"))
                    })?;
                    let mut layer = ds.layer(gdal_idx).map_err(|_| {
                        err(&PublicMessage::Curated(
                            "apertura del layer FileGDB fallita",
                        ))
                    })?;
                    configure_read_layer(&mut layer, &fields, geometria)?;
                    let mut geom = geometria.inclusa.then(BinaryBuilder::new);
                    let mut builders: Vec<ReadCol> = fields
                        .iter()
                        .map(|field| ReadCol::new(&field.data_type))
                        .collect();
                    let mut n = 0usize;
                    let mut batch_loss = LossReport::default();
                    for feature in layer.features().take(geometria.row_limit) {
                        observe_feature_identity(&mut batch_loss, &feature, layer_index);
                        if let Some(builder) = &mut geom {
                            match feature.geometry_by_index(0) {
                                Ok(geometry) => {
                                    let bytes = geometry.iso_wkb().map_err(|_| {
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
                        for (index, (builder, field)) in
                            builders.iter_mut().zip(&fields).enumerate()
                        {
                            if builder.append_feature(&feature, field)? {
                                observe_native(
                                    &mut batch_loss,
                                    (NativeDriver::FileGdb, TransportDirection::Read),
                                    NativeElement::Temporal,
                                    1,
                                    layer_index,
                                    Some(driver_common::saturating_u64(
                                        index + usize::from(geometria.inclusa),
                                    )),
                                );
                            }
                        }
                        n += 1;
                        if n >= batch_sizer.rows() {
                            let batch = finish_read_batch(&schema, &mut geom, &mut builders, n)?;
                            plenora_io_core::driver::validate_external_batch(
                                &batch,
                                plenora_io_model::ErrorPhase::Read,
                            )?;
                            batch_sizer.observe(&batch);
                            if loss_tx.send(std::mem::take(&mut batch_loss)).is_err()
                                || !emitter.send(batch)
                            {
                                return Ok(());
                            }
                            n = 0;
                        }
                    }
                    if n > 0 {
                        let batch = finish_read_batch(&schema, &mut geom, &mut builders, n)?;
                        if loss_tx.send(batch_loss).is_err() || !emitter.send(batch) {
                            return Ok(());
                        }
                    }
                    Ok(())
                })
            },
        )?;
        Ok(Box::new(TemporalLossReader {
            reader,
            loss_rx,
            loss: LossReport::default(),
        }))
    }

    struct TemporalLossReader {
        reader: Box<dyn LayerReader>,
        loss_rx: std::sync::mpsc::Receiver<LossReport>,
        loss: LossReport,
    }

    impl LayerReader for TemporalLossReader {
        fn contract(&self) -> &LayerContract {
            self.reader.contract()
        }
        fn next_batch(&mut self) -> Result<Option<RecordBatch>> {
            let batch = self.reader.next_batch()?;
            if batch.is_some() {
                self.loss
                    .merge(&self.loss_rx.recv().map_err(|_| temporal_error())?);
            }
            Ok(batch)
        }
        fn loss_report(&self) -> LossReport {
            self.loss.clone()
        }
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

    fn temporal_error() -> PlenoraIoError {
        err(&PublicMessage::Curated(
            "data o fuso OGR non rappresentabile senza ambiguita'",
        ))
    }

    // OGR exposes seconds as f32. Round to Arrow microseconds; the reader
    // reports a precision loss only if the supplied value actually changes.
    #[allow(clippy::cast_possible_truncation)]
    fn temporal_value(components: (i32, i32, i32, i32, i32, f32, i32), zoned: bool) -> Result<i64> {
        let (year, month, day, hour, minute, second, flag) = components;
        if !second.is_finite()
            || !(0.0..60.0).contains(&second)
            || zoned != ogr_timezone(flag)?.is_some()
        {
            return Err(temporal_error());
        }
        let date = chrono::NaiveDate::from_ymd_opt(
            year,
            u32::try_from(month).map_err(|_| temporal_error())?,
            u32::try_from(day).map_err(|_| temporal_error())?,
        )
        .ok_or_else(temporal_error)?;
        let value = date
            .and_hms_opt(
                u32::try_from(hour).map_err(|_| temporal_error())?,
                u32::try_from(minute).map_err(|_| temporal_error())?,
                0,
            )
            .ok_or_else(temporal_error)?;
        let offset = if zoned {
            i64::from(flag - 100) * 900_000_000
        } else {
            0
        };
        value
            .and_utc()
            .timestamp_micros()
            .checked_add((f64::from(second) * 1_000_000.0).round() as i64)
            .and_then(|v| v.checked_sub(offset))
            .ok_or_else(temporal_error)
    }

    enum ReadCol {
        I32(Int32Builder),
        I64(Int64Builder),
        F64(Float64Builder),
        Str(StringBuilder),
        Bin(BinaryBuilder),
        Date(Date32Builder),
        Time(TimestampMicrosecondBuilder, bool),
    }

    impl ReadCol {
        fn new(dt: &DataType) -> Self {
            match dt {
                DataType::Int32 => Self::I32(Int32Builder::new()),
                DataType::Int64 => Self::I64(Int64Builder::new()),
                DataType::Float64 => Self::F64(Float64Builder::new()),
                DataType::Binary => Self::Bin(BinaryBuilder::new()),
                DataType::Date32 => Self::Date(Date32Builder::new()),
                DataType::Timestamp(_, tz) => Self::Time(
                    TimestampMicrosecondBuilder::new().with_timezone_opt(tz.clone()),
                    tz.is_some(),
                ),
                _ => Self::Str(StringBuilder::new()),
            }
        }
        fn append_feature(
            &mut self,
            feature: &Feature<'_>,
            field: &ProjectedField,
        ) -> Result<bool> {
            let mut rounded = false;
            // Ne' il nome del campo ne' il testo di GDAL escono: il primo
            // viene dallo schema letto dal file, il secondo dalla dipendenza.
            let read_error = |_| {
                err(&PublicMessage::CuratedWith(
                    "lettura di un campo FileGDB fallita, indice OGR",
                    // `unsigned_abs` rende la conversione **totale**: nessun ramo
                    // di riserva, quindi niente da registrare come fallback.
                    //
                    // (Il commento evita di nominare la forma alternativa: il
                    // registro dei fallback conta il testo, e citarla qui la
                    // farebbe contare come se ci fosse davvero.)
                    NumeroStrutturale::Indice(driver_common::saturating_u64(field.ogr_index)),
                ))
            };
            match self {
                Self::Bin(builder) => builder.append_option(
                    feature
                        .field_as_binary(field.ogr_index)
                        .map_err(read_error)?,
                ),
                Self::Date(builder) => {
                    let value = feature
                        .field_as_datetime_components(field.ogr_index)
                        .map_err(read_error)?;
                    builder.append_option(
                        value
                            .map(|v| {
                                temporal_value(v, false).and_then(|v| {
                                    i32::try_from(v.div_euclid(86_400_000_000))
                                        .map_err(|_| temporal_error())
                                })
                            })
                            .transpose()?,
                    );
                }
                Self::Time(builder, zoned) => {
                    let value = feature
                        .field_as_datetime_components(field.ogr_index)
                        .map_err(read_error)?;
                    if let Some(v) = value {
                        let micros = (f64::from(v.5) * 1_000_000.0).round();
                        rounded = (micros / 1_000_000.0).to_bits() != f64::from(v.5).to_bits();
                    }
                    builder.append_option(value.map(|v| temporal_value(v, *zoned)).transpose()?);
                }
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
            Ok(rounded)
        }
        fn finish(&mut self) -> ArrayRef {
            match self {
                Self::I32(b) => Arc::new(b.finish()),
                Self::I64(b) => Arc::new(b.finish()),
                Self::F64(b) => Arc::new(b.finish()),
                Self::Str(b) => Arc::new(b.finish()),
                Self::Bin(b) => Arc::new(b.finish()),
                Self::Date(b) => Arc::new(b.finish()),
                Self::Time(b, _) => Arc::new(b.finish()),
            }
        }
    }

    #[cfg(test)]
    #[path = "../r2_tests.rs"]
    mod r2_tests;

    #[cfg(test)]
    mod tests {
        use super::*;
        use crate::test_contract::declared;
        use arrow_array::RecordBatch;
        use plenora_io_core::descriptor::Fidelity;
        use plenora_io_core::driver::{FormatDriver, ReadOptions, Sink, Source, WriteOptions};
        use plenora_io_core::request::{BatchTarget, ProjectionMode, ReadScope};
        use plenora_io_model::contract::{GeometryEncoding, GeometryType};
        use plenora_io_model::crs::AxisOrder;
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

        #[test]
        fn geometry_losses_match_the_reopened_database() {
            let dir = tempfile::tempdir().unwrap();
            for (index, wkt) in [
                "POINT (1 2)",
                "POINT (1.1234567891234567 2.987654321987654)",
                "MULTIPOLYGON (((0 0, 2 0, 2 2, 0 2, 0 0)))",
                "MULTIPOLYGON (((0 0, 0 2, 2 2, 2 0, 0 0)))",
            ]
            .into_iter()
            .enumerate()
            {
                let source = Geometry::from_wkt(wkt).unwrap().iso_wkb().unwrap();
                let original = decode_wkb(&source, &WkbLimits::default()).unwrap();
                let mut geometry = GeometryColumnContract::wkb_xy(
                    FieldId(0),
                    GEOMETRY,
                    ResolvedCrs::new(Some("EPSG:3857".to_owned()), CrsKind::Projected, None),
                    true,
                );
                geometry.set_exact_geometry_types(vec![if index < 2 {
                    GeometryType::Point
                } else {
                    GeometryType::MultiPolygon
                }]);
                let schema = Arc::new(Schema::new(vec![geometry_field(GEOMETRY, "EPSG:3857")]));
                let plan = WritePlan {
                    layers: vec![WriteLayer {
                        name: "features".to_owned(),
                        contract: declared(&schema, geometry),
                    }],
                };
                let batch = RecordBatch::try_new(
                    plan.layers[0].contract.schema.clone(),
                    vec![Arc::new(BinaryArray::from(vec![source.as_slice()]))],
                )
                .unwrap();
                let path = dir.path().join(format!("case{index}.gdb"));
                let driver = super::super::FileGdbDriver;
                let mut writer = driver
                    .create(Sink::Path(path.clone()), &plan, &opzioni_scrittura())
                    .unwrap();
                writer.write(&batch).unwrap();
                let published = writer.finish().unwrap();
                let dataset = driver.open(Source::Path(path), opzioni_lettura()).unwrap();
                let mut reader = dataset.open_layer_reader(&read_request()).unwrap();
                let batch = reader.next_batch().unwrap().unwrap();
                let column = batch
                    .column(0)
                    .as_any()
                    .downcast_ref::<BinaryArray>()
                    .unwrap();
                let actual = decode_wkb(column.value(0), &WkbLimits::default()).unwrap();
                let (before_coordinates, before_orientation) = geometry_signature(&original);
                let (after_coordinates, after_orientation) = geometry_signature(&actual);
                let precision_changed = before_coordinates != after_coordinates;
                let orientation_changed = before_orientation != after_orientation;
                if index == 0 {
                    assert!(!precision_changed);
                }
                if index == 1 {
                    assert!(precision_changed);
                }
                if index == 2 {
                    assert!(orientation_changed);
                }
                if index == 3 {
                    assert!(!orientation_changed);
                }
                for (category, changed) in [
                    ("native_coordinate_precision", precision_changed),
                    ("native_ring_orientation_changed", orientation_changed),
                ] {
                    assert_eq!(
                        published.loss.counts.get(category).copied().unwrap_or(0),
                        u64::from(changed),
                        "{index}: {:?}",
                        published.loss
                    );
                }
            }
        }

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
                contract: declared(&schema, geometry),
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
        /// GDAL riporta gli assi dell'autorita' (EPSG:4326 latitudine prima),
        /// ma i valori del `FileGDB` memorizzano la longitudine prima: la
        /// dichiarazione e' `lon_lat` (GEO-001). Fino alla 5.x la prova si
        /// chiamava `gdal_reports_authority_axis_order_without_canonicalization`
        /// e fissava `lat_lon`, cioe' l'ordine del registro.
        fn gdal_axis_order_is_the_stored_one_not_the_registry_one() {
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

            assert_eq!(epsg.axis_order, AxisOrder::LongitudeLatitude);
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
            let mut layer = write_layer(ResolvedCrs::new(
                Some("EPSG:3857".to_owned()),
                CrsKind::Projected,
                None,
            ));
            // Corruzione intenzionale per provare il controllo GDAL del CRS,
            // dopo la costruzione della fixture valida.
            layer.contract.geometry.as_mut().unwrap().crs = ResolvedCrs::new(
                Some("EPSG:3857".to_owned()),
                CrsKind::Projected,
                Some(epsg_4326_wkt),
            )
            .into();
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
                    contract: declared(&schema, geometry),
                }],
            };
            let batch = RecordBatch::try_new(
                plan.layers[0].contract.schema.clone(),
                vec![Arc::new(BinaryArray::from(vec![Some(wkb.as_slice())]))],
            )
            .unwrap();

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
                    contract: declared(&schema, geometry),
                }],
            };
            let batch = RecordBatch::try_new(
                plan.layers[0].contract.schema.clone(),
                vec![Arc::new(BinaryArray::from(vec![Some(wkb.as_slice())]))],
            )
            .unwrap();

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
                    contract: declared(&schema, geometry),
                }],
            };
            let batch = RecordBatch::try_new(
                plan.layers[0].contract.schema.clone(),
                vec![
                    Arc::new(BinaryArray::from(vec![Some(wkb.as_slice()), None])),
                    Arc::new(Int32Array::from(vec![Some(i32::MAX), None])),
                    Arc::new(Float64Array::from(vec![Some(12.5), None])),
                    Arc::new(StringArray::from(vec![Some("città"), None])),
                ],
            )
            .unwrap();

            let driver = super::super::FileGdbDriver;
            let mut writer = driver
                .create(Sink::Path(path.clone()), &plan, &opzioni_scrittura())
                .unwrap();
            writer.write(&batch).unwrap();
            let published = writer.finish().unwrap();
            assert_eq!(published.fidelity.level, Fidelity::Approximating);
            assert_eq!(
                published.loss.counts.get("crs_id_not_preserved_derived"),
                None
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
            let mut geometry = GeometryColumnContract::wkb_xy(
                FieldId(0),
                GEOMETRY,
                ResolvedCrs::new(Some("EPSG:3857".to_owned()), CrsKind::Projected, None),
                true,
            );
            geometry.set_exact_geometry_types(vec![GeometryType::Point]);
            let plan = WritePlan {
                layers: vec![WriteLayer {
                    name: "float_edges".to_owned(),
                    contract: declared(&schema, geometry),
                }],
            };
            let batch = RecordBatch::try_new(
                plan.layers[0].contract.schema.clone(),
                vec![
                    Arc::new(geometries),
                    Arc::new(Float64Array::from(values.to_vec())),
                ],
            )
            .unwrap();

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
                    contract: declared(&schema, geometry),
                }],
            };
            let batch = RecordBatch::try_new(
                plan.layers[0].contract.schema.clone(),
                vec![Arc::new(BinaryArray::from(vec![Some(wkb.as_slice())]))],
            )
            .unwrap();

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
            let bytes = geometry.value(0);
            let type_code = if bytes[0] == 1 {
                u32::from_le_bytes(bytes[1..5].try_into().unwrap())
            } else {
                u32::from_be_bytes(bytes[1..5].try_into().unwrap())
            };
            assert_eq!(
                type_code & 0xe000_0000,
                0,
                "ISO WKB must not carry EWKB/25D flags"
            );
            let decoded = decode_wkb(bytes, &WkbLimits::default()).unwrap();
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

        /// Un `FileGDB` con un CRS incorporato e un `--assume-crs` diverso: l'opzione
        /// si ignorava, e il layer usciva col CRS del file senza dirlo. Lo stesso
        /// identificatore, con l'autorita' in minuscolo, resta ammesso.
        #[test]
        fn assume_crs_in_conflitto_col_crs_incorporato_e_rifiutato() {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("incorporato.gdb");
            let schema: SchemaRef =
                Arc::new(Schema::new(vec![geometry_field(GEOMETRY, "EPSG:3857")]));
            let wkb = point_wkb(CoordinateDimensions::Xy, None, None);
            let mut geometry = GeometryColumnContract::wkb_xy(
                FieldId(0),
                GEOMETRY,
                ResolvedCrs::new(Some("EPSG:3857".to_owned()), CrsKind::Projected, None),
                true,
            );
            geometry.set_exact_geometry_types(vec![GeometryType::Point]);
            let plan = WritePlan {
                layers: vec![WriteLayer {
                    name: "punti".to_owned(),
                    contract: declared(&schema, geometry),
                }],
            };
            let batch = RecordBatch::try_new(
                plan.layers[0].contract.schema.clone(),
                vec![Arc::new(BinaryArray::from(vec![Some(wkb.as_slice())]))],
            )
            .unwrap();

            let driver = super::super::FileGdbDriver;
            let mut writer = driver
                .create(Sink::Path(path.clone()), &plan, &opzioni_scrittura())
                .unwrap();
            writer.write(&batch).unwrap();
            writer.finish().unwrap();

            let dataset = driver
                .open(
                    Source::Path(path.clone()),
                    opzioni_lettura().with_assume_crs("EPSG:3003"),
                )
                .unwrap();
            let errore = dataset.rifiuto_del_layer(LayerId(0)).unwrap();
            assert_eq!(errore.category, plenora_io_model::ErrorCategory::Crs);
            let concorde = driver
                .open(
                    Source::Path(path),
                    opzioni_lettura().with_assume_crs("epsg:3857"),
                )
                .unwrap();
            let crs = &concorde.layers()[0].contract.geometry.as_ref().unwrap().crs;
            assert_eq!(crs.id(), Some("EPSG:3857"));
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
                        contract: declared(&schema, geometry),
                    }],
                };
                let batch = RecordBatch::try_new(
                    plan.layers[0].contract.schema.clone(),
                    vec![Arc::new(BinaryArray::from(vec![Some(wkb.as_slice())]))],
                )
                .unwrap();

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
                let bytes = geometry.value(0);
                let type_code = if bytes[0] == 1 {
                    u32::from_le_bytes(bytes[1..5].try_into().unwrap())
                } else {
                    u32::from_be_bytes(bytes[1..5].try_into().unwrap())
                };
                assert_eq!(
                    type_code & 0xe000_0000,
                    0,
                    "ISO WKB must not carry EWKB/25D flags"
                );
                let decoded = decode_wkb(bytes, &WkbLimits::default()).unwrap();
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
                    contract: declared(&schema, geometry),
                }],
            };
            let batch = RecordBatch::try_new(
                plan.layers[0].contract.schema.clone(),
                vec![Arc::new(BinaryArray::from(vec![Some(wkb.as_slice())]))],
            )
            .unwrap();

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
            let bytes = geometry.value(0);
            let type_code = if bytes[0] == 1 {
                u32::from_le_bytes(bytes[1..5].try_into().unwrap())
            } else {
                u32::from_be_bytes(bytes[1..5].try_into().unwrap())
            };
            assert_eq!(
                type_code & 0xe000_0000,
                0,
                "ISO WKB must not carry EWKB/25D flags"
            );
            let decoded = decode_wkb(bytes, &WkbLimits::default()).unwrap();
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
        fn foreign_staging_lock_before_finish_blocks_publication() {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("locked.gdb");
            let (plan, batch) = point_write_fixture();
            let mut writer = super::super::FileGdbDriver
                .create(Sink::Path(path.clone()), &plan, &opzioni_scrittura())
                .unwrap();
            writer.write(&batch).unwrap();
            let foreign = dir.path().join("LOCKED.PLENORA-TMP-FOREIGN.LOCK");
            std::fs::write(&foreign, b"foreign").unwrap();
            assert_eq!(
                writer.finish().err().unwrap().code,
                plenora_io_model::IoErrorCode::OutputExists
            );
            assert!(!path.exists());
            assert_eq!(std::fs::read(foreign).unwrap(), b"foreign");
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

            let error = super::super::FileGdbDriver
                .create(Sink::Path(path.clone()), &plan, &opzioni_scrittura())
                .err()
                .unwrap();
            assert_eq!(error.code, plenora_io_model::IoErrorCode::OutputExists);
            assert_eq!(staging_artifacts(&path).len(), 2);
            assert!(first_artifacts.iter().all(|artifact| artifact.exists()));

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
            let error = super::super::FileGdbDriver
                .create(Sink::Path(path.clone()), &plan, &opzioni_scrittura())
                .err()
                .unwrap();
            assert_eq!(error.code, plenora_io_model::IoErrorCode::OutputExists);
            let only_active_staging_exists = staging_artifacts(&path).len() == 2;
            let active_was_preserved = active_artifacts.iter().all(|artifact| artifact.exists());
            let active_survived_second_cleanup =
                active_artifacts.iter().all(|artifact| artifact.exists());

            File::create(release).unwrap();
            let status = child.wait().unwrap();
            assert!(status.success(), "writer attivo fallito: {status}");
            assert_eq!(active_artifacts.len(), 2);
            assert!(only_active_staging_exists);
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
                    let error = super::super::FileGdbDriver
                        .create(Sink::Path(path.clone()), &plan, &opzioni_scrittura())
                        .err()
                        .unwrap();
                    assert_eq!(error.code, plenora_io_model::IoErrorCode::OutputExists);
                    assert_eq!(recover_orphaned_staging(&path).unwrap(), 1);
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
                        contract: declared(&schema, geometry),
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
                layer.contract = declared(
                    &layer.contract.schema,
                    layer.contract.geometry.take().unwrap(),
                );
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
            layer.contract = declared(
                &layer.contract.schema,
                layer.contract.geometry.take().unwrap(),
            );
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
                (
                    DataType::Timestamp(arrow_schema::TimeUnit::Microsecond, Some("UTC".into())),
                    "datetime",
                ),
                (DataType::Binary, "binary"),
                (DataType::LargeBinary, "large_binary"),
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
                layer.contract = declared(
                    &layer.contract.schema,
                    layer.contract.geometry.take().unwrap(),
                );
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
            layer.contract = declared(
                &layer.contract.schema,
                layer.contract.geometry.take().unwrap(),
            );
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
        /// Una directory `.gdb` che non e' un `FileGDB` non si legge come tale.
        ///
        /// `Dataset::open` lasciava scegliere a GDAL il driver, e il driver CSV
        /// di GDAL apre una directory di CSV: un `.csvt` che marca due colonne
        /// `WKT` ne fa un layer con **due** campi geometrici, di cui il reader
        /// prendeva il primo. Due scelte silenziose in una: il formato e la
        /// geometria.
        #[test]
        fn una_directory_gdb_che_non_e_un_filegdb_non_si_apre() {
            let dir = tempfile::tempdir().unwrap();
            let percorso = dir.path().join("travestito.gdb");
            std::fs::create_dir(&percorso).unwrap();
            std::fs::write(
                percorso.join("a.csv"),
                "g1,g2,id\n\"POINT (1 2)\",\"POINT (3 4)\",1\n",
            )
            .unwrap();
            std::fs::write(percorso.join("a.csvt"), "WKT,WKT,Integer\n").unwrap();

            assert!(
                open(&percorso, Some("EPSG:4326")).is_err(),
                "una directory di CSV non e' un FileGDB"
            );
        }

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
mod tests;

/// Le sonde della superficie che il fuzz target attraversa.
///
/// Modulo a se' perche' il modulo `tests` di questo file e' gated su
/// `not(feature = "gdal-backend")`: senza la feature il driver e' uno stub, e
/// quelle sonde provano lo stub. Queste provano il contrario -- il percorso che
/// esiste **solo** con GDAL -- e metterle li' le avrebbe compilate via proprio
/// nella configurazione in cui servono.
#[cfg(all(test, feature = "gdal-backend"))]
mod sonde_del_fuzzing;

#[cfg(test)]
mod test_contract;

// Emissione dal formato nativo, condivisa con le prove senza GDAL.
#[cfg(any(test, feature = "gdal-backend"))]
fn native_geometry_field(
    geometry: &plenora_io_model::contract::GeometryColumnContract,
) -> arrow_schema::Field {
    use plenora_io_model::geometry::{
        with_geometry_contract_metadata, ARROW_EXTENSION_NAME_KEY, GEOARROW_WKB_EXTENSION,
        GEO_CRS_KEY,
    };
    let mut metadata = arrow_schema::Metadata::default();
    metadata.insert(ARROW_EXTENSION_NAME_KEY, GEOARROW_WKB_EXTENSION);
    // La chiave legacy contiene un identificativo, mai una definizione WKT.
    // La definizione resta nei metadati canonici del contratto geometrico.
    if let Some(id) = geometry.crs.id() {
        metadata.insert(GEO_CRS_KEY, id);
    }
    with_geometry_contract_metadata(
        &arrow_schema::Field::new(
            &geometry.name,
            arrow_schema::DataType::Binary,
            geometry.nullable,
        )
        .with_metadata(metadata),
        geometry,
    )
}
