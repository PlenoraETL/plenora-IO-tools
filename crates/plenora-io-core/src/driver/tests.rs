//! Le prove di [`super`], in un file loro.
//!
//! Restano un **modulo figlio**: vedono i privati del genitore
//! esattamente come quando stavano dentro di lui, e non allargano di
//! una riga la superficie pubblica del crate.

/// Descrittore minimo per i test del preflight.
///
/// Questi test verificano l'enumerazione della sorgente, non lo schema
/// delle opzioni: schema vuoto e mappa vuota li lasciano invariati, e la
/// validazione che `preflight_source` ora esegue non entra in mezzo.
const DESCRITTORE_DI_PROVA: crate::descriptor::FormatDescriptor =
    crate::descriptor::FormatDescriptor::const_new(
        "prova",
        crate::descriptor::Direction::Read,
        crate::descriptor::ReadMode::StreamingSequential,
        // I tre assi di INV-7: il descrittore di prova dichiara la
        // combinazione che tutti i driver reali dichiarano.
        crate::descriptor::NativeReadMode::StreamingSequential,
        crate::descriptor::DeliverySemantics::OperationAtomic,
        crate::descriptor::BufferingStrategy::AdaptiveMemoryThenDisk,
        crate::descriptor::DeterminismLevel::Semantic,
        None,
        None,
        false,
        false,
        crate::descriptor::ReaderConcurrency::SingleActiveReader,
        crate::descriptor::ProjectionSupport::None,
        crate::descriptor::PredicatePruningSupport::None,
        crate::descriptor::SpatialPruningSupport::None,
        crate::descriptor::CrsHandling::None,
        crate::descriptor::Fidelity::Lossless,
        crate::descriptor::Runtime::PureRust,
        // `hostile_input_hardened`: un descrittore di prova non parla di
        // input ostile: dichiara il valore che non afferma niente.
        false,
        // `spec_version_supported`: un descrittore di prova non parla di
        // nessun formato reale, quindi non ne dichiara la versione.
        None,
        None,
        plenora_io_model::format_options::SchemaOpzioniFormato::VUOTO,
        &["test"],
        1,
        1,
        1,
    );
use std::collections::HashMap;
use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use arrow_array::{BinaryArray, Int64Array};
use arrow_schema::{DataType, Field, Schema};
use plenora_io_model::contract::{CoordinateDimensions, FieldId, GeometryColumnContract};
use plenora_io_model::crs::{CrsKind, CrsResolution, ResolvedCrs};
use plenora_io_model::geometry::{
    ARROW_EXTENSION_NAME_KEY, GEOARROW_WKB_EXTENSION, PLENORA_DIMENSIONS_KEY,
};
use plenora_io_model::wkb::{encode_wkb, WkbCoordinate, WkbFlavor, WkbGeometry, WkbValue};

use super::*;
use crate::descriptor::WKB_XY_GEOMETRY;

fn scan_dir_with(
    entries: usize,
    limits: plenora_io_model::budget::PipelineLimits,
) -> Result<PathBuf> {
    let root = tempfile::tempdir().expect("tempdir");
    for index in 0..entries {
        let mut file =
            std::fs::File::create(root.path().join(format!("entry-{index}.bin"))).expect("file");
        file.write_all(b"x").expect("write");
    }
    let mut opts = opzioni_pipeline(limits);
    preflight_source(
        &DESCRITTORE_DI_PROVA,
        Source::Path(root.path().to_path_buf()),
        &mut opts,
    )
}

/// L0.9: senza tetto sulle entry una directory ostile fa crescere la coda
/// dello scan senza limite, perche' i byte si sommano solo sui file.
#[test]
fn directory_scan_over_max_input_entries_rejects_with_typed_error() {
    let limits = plenora_io_model::budget::PipelineLimits::default().with_max_input_entries(4);
    // La radice conta come entry: 4 file piu' la directory sono 5.
    let error = scan_dir_with(4, limits).expect_err("il quinto elemento deve far fallire");
    assert_eq!(error.code, plenora_io_model::IoErrorCode::LimitExceeded);
    assert_eq!(
        error.category,
        plenora_io_model::ErrorCategory::ResourceLimit
    );
}

#[test]
fn directory_scan_within_max_input_entries_succeeds() {
    let limits = plenora_io_model::budget::PipelineLimits::default().with_max_input_entries(4);
    assert!(
        scan_dir_with(3, limits).is_ok(),
        "radice + 3 file = 4 entry"
    );
}

#[test]
fn max_input_entries_default_admits_a_realistic_directory() {
    // Il default non deve rifiutare una directory di file legittimi:
    // un tetto troppo stretto sarebbe un fail-closed inutile.
    assert!(scan_dir_with(64, plenora_io_model::budget::PipelineLimits::default()).is_ok());
}

#[test]
fn entry_cap_is_checked_before_the_byte_sum() {
    // Con un tetto di entry raggiunto e un limite di byte larghissimo,
    // deve vincere il tetto delle entry: e' l'ordine dichiarato da INV-9.
    let limits = plenora_io_model::budget::PipelineLimits::default()
        .with_max_input_entries(2)
        .with_max_input_bytes(u64::MAX);
    let error = scan_dir_with(8, limits).expect_err("il tetto entry deve intervenire");
    assert!(
        error.message.contains("entry"),
        "messaggio: {}",
        error.message
    );
}

/// La barriera contro i panic di arrow deve restituire un errore del
/// driver invece di far abortire il processo, e non deve interferire con
/// il percorso normale.
///
/// Questo test e' l'unica copertura possibile del meccanismo: i fuzz
/// target che esercitano i percorsi Arrow e Parquet restano rossi anche a
/// barriera funzionante, perche' `libfuzzer-sys` chiama
/// `std::process::abort()` prima dell'unwinding (0.4.10, src/lib.rs:92-95)
/// apposta perche' un `catch_unwind` non possa nascondergli difetti.
#[test]
fn la_barriera_arrow_converte_il_panico_in_errore_del_driver() {
    // L'hook del processo stamperebbe comunque su stderr e farebbe
    // sembrare la suite fallita: silenziato per la durata del test.
    let precedente = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));

    // Panico con messaggio formattato: il payload e' una `String`.
    let formattato = leggendo_arrow("parquet", || -> Result<()> {
        panic!("precisione {} non supportata", 194)
    });
    // Panico con letterale: il payload e' un `&'static str`.
    let letterale = leggendo_arrow("arrow", || -> Result<()> {
        panic!("Type NONE not supported")
    });
    // Percorso normale: la barriera non deve alterare nulla.
    let riuscito = leggendo_arrow("arrow", || Ok(7_u8));
    // Errore ordinario: deve passare invariato, non essere riclassificato.
    let fallito = leggendo_arrow("arrow", || -> Result<u8> {
        Err(PlenoraIoError::formato_redatto(
            "arrow",
            &PublicMessage::Curated("payload troncato"),
        ))
    });

    // Stesso panico del primo: il messaggio pubblico deve coincidere.
    let ripetuto = leggendo_arrow("parquet", || -> Result<()> {
        panic!("precisione {} non supportata", 194)
    });

    std::panic::set_hook(precedente);

    // `message` dichiara di non contenere payload: il testo del panico
    // arriva da una libreria di terze parti ed e' derivato dal file, quindi
    // non deve comparire.
    let formattato = formattato.expect_err("il panico deve diventare errore");
    assert!(
        !formattato.to_string().contains("precisione 194"),
        "il messaggio del panico non deve finire nell'errore pubblico: {formattato}"
    );
    assert!(
        formattato.to_string().contains(MESSAGGIO_PANICO_ARROW),
        "il messaggio pubblico e' quello statico curato: {formattato}"
    );

    let letterale = letterale.expect_err("il panico deve diventare errore");
    assert!(
        !letterale.to_string().contains("Type NONE"),
        "nemmeno il payload letterale deve comparire: {letterale}"
    );

    // Il messaggio non distingue piu' un panico dall'altro, ed e' voluto:
    // FZ-0 ha tolto l'impronta perche' era un valore derivato dall'input
    // che finiva in un errore serializzato. Cio' che distingue le
    // occorrenze sono i log del processo, dove l'hook di panico scrive il
    // testo completo.
    let ripetuto = ripetuto.expect_err("il panico deve diventare errore");
    assert_eq!(formattato.to_string(), ripetuto.to_string());
    assert_eq!(
        formattato.to_string(),
        letterale.to_string(),
        "il messaggio e' statico, quindi identico per qualunque panico"
    );

    assert_eq!(riuscito.expect("il percorso normale non e' toccato"), 7);
    assert!(fallito
        .expect_err("l'errore ordinario resta tale")
        .to_string()
        .contains("payload troncato"));
}

#[test]
fn periodic_cancellation_has_a_bounded_check_interval() {
    let token = CancellationToken::new();
    token.cancel();
    assert!(matches!(
        check_cancelled_periodically(&token, ErrorPhase::Read, 0),
        Err(error) if error.code == plenora_io_model::IoErrorCode::Cancelled
    ));
    assert!(check_cancelled_periodically(&token, ErrorPhase::Read, 1).is_ok());
    assert!(matches!(
        check_cancelled_periodically(
            &token,
            ErrorPhase::Read,
            CANCELLATION_CHECK_INTERVAL
        ),
        Err(error) if error.code == plenora_io_model::IoErrorCode::Cancelled
    ));
}

struct FinishTrackingWriter {
    finished: Arc<AtomicBool>,
}

impl FormatWriter for FinishTrackingWriter {
    fn write(&mut self, _batch: &RecordBatch) -> Result<()> {
        Ok(())
    }

    fn finish(self: Box<Self>) -> Result<Published> {
        self.finished.store(true, Ordering::SeqCst);
        Ok(Published {
            bytes: 0,
            loss: LossReport::default(),
            fidelity: FidelityAssessment::lossless(),
            outcome: crate::publish::PublishOutcome::Published,
        })
    }
}

#[test]
fn failed_write_poisons_writer_and_prevents_finish() {
    let finished = Arc::new(AtomicBool::new(false));
    let opts = WriteOptions::from_write_parts(
        match plenora_io_model::budget::PipelineBudget::builder()
            // Il modello rifiuta le quote nulle, quindi il tetto e' uno
            // e il batch ne porta due: il rifiuto scatta comunque alla
            // prima scrittura, che e' cio' che il test verifica.
            .limits(limiti_di_prova().with_max_rows(1))
            .build()
        {
            Ok(bundle) => bundle.into_write_parts(),
            Err(error) => unreachable!("limiti di test: {error:?}"),
        },
    );
    let mut writer = with_write_limits(
        Box::new(FinishTrackingWriter {
            finished: finished.clone(),
        }),
        &opts,
    );
    let schema = Arc::new(Schema::new(vec![Field::new(
        "value",
        DataType::Int64,
        false,
    )]));
    let batch = RecordBatch::try_new(schema, vec![Arc::new(Int64Array::from(vec![1, 2]))]).unwrap();

    assert!(matches!(
        writer.write(&batch),
        Err(error) if error.code == plenora_io_model::IoErrorCode::LimitExceeded
    ));
    assert!(matches!(
        writer.write(&batch),
        Err(error) if error.code == plenora_io_model::IoErrorCode::Format
    ));
    assert!(matches!(
        writer.finish(),
        Err(error) if error.code == plenora_io_model::IoErrorCode::Format
    ));
    assert!(!finished.load(Ordering::SeqCst));
}

#[test]
fn declared_input_total_rejects_extra_rows() {
    let finished = Arc::new(AtomicBool::new(false));
    let mut writer = with_write_limits(
        Box::new(FinishTrackingWriter {
            finished: finished.clone(),
        }),
        &opzioni_scrittura(),
    );
    writer.declare_input_total(LayerId(0), 0).unwrap();
    let schema = Arc::new(Schema::new(vec![Field::new(
        "value",
        DataType::Int64,
        false,
    )]));
    let batch = RecordBatch::try_new(schema, vec![Arc::new(Int64Array::from(vec![1]))]).unwrap();

    assert!(matches!(
        writer.write(&batch),
        Err(error) if error.code == plenora_io_model::IoErrorCode::Contract
    ));
    assert!(!finished.load(Ordering::SeqCst));
}

#[test]
fn declared_input_total_rejects_early_eof_before_publish() {
    let finished = Arc::new(AtomicBool::new(false));
    let mut writer = with_write_limits(
        Box::new(FinishTrackingWriter {
            finished: finished.clone(),
        }),
        &opzioni_scrittura(),
    );
    writer.declare_input_total(LayerId(0), 10).unwrap();
    let schema = Arc::new(Schema::new(vec![Field::new(
        "value",
        DataType::Int64,
        false,
    )]));
    let batch =
        RecordBatch::try_new(schema, vec![Arc::new(Int64Array::from_iter_values(0..9))]).unwrap();
    writer.write(&batch).unwrap();

    let Err(error) = writer.finish() else {
        panic!("EOF anticipato pubblicato")
    };
    assert_eq!(error.code, plenora_io_model::IoErrorCode::Contract);
    assert_eq!(error.category, ErrorCategory::InvalidPlan);
    assert_eq!(error.phase, ErrorPhase::Validate);
    assert!(!finished.load(Ordering::SeqCst));
}

#[test]
fn each_layer_can_declare_its_total_before_its_first_write() {
    let finished = Arc::new(AtomicBool::new(false));
    let mut writer: Box<dyn FormatWriter> = Box::new(LimitedWriter {
        inner: Box::new(FinishTrackingWriter { finished }),
        driver: "test",
        limits: opzioni_scrittura().write_limits(),
        rows: 0,
        layer_rows: vec![0, 0],
        input_totals: vec![None, None],
        failed: false,
        contracts: Vec::new(),
        geometry_validation: None,
        fidelity: FidelityAssessment::lossless(),
        planned_loss: LossReport::default(),
        cancellation: CancellationToken::new(),
        budget: opzioni_scrittura().budget().clone(),
        _operation_lease: None,
    });
    writer.declare_input_total(LayerId(0), 1).unwrap();
    let schema = Arc::new(Schema::new(vec![Field::new(
        "value",
        DataType::Int64,
        false,
    )]));
    let batch = RecordBatch::try_new(schema, vec![Arc::new(Int64Array::from(vec![1]))]).unwrap();
    writer.write_to_layer(LayerId(0), &batch).unwrap();

    writer.declare_input_total(LayerId(1), 0).unwrap();
}

#[test]
fn source_size_is_checked_before_parsing() {
    let mut file = tempfile::NamedTempFile::new().unwrap();
    file.write_all(&[0_u8; 8]).unwrap();
    let mut opts = opzioni_pipeline(
        plenora_io_model::budget::PipelineLimits::default().with_max_input_bytes(7),
    );
    let result = preflight_source(
        &DESCRITTORE_DI_PROVA,
        Source::Path(file.path().to_owned()),
        &mut opts,
    );
    assert!(matches!(
        result,
        Err(error) if error.code == plenora_io_model::IoErrorCode::LimitExceeded
    ));
}

#[test]
fn cancelled_source_is_rejected_before_filesystem_probe() {
    let token = CancellationToken::new();
    token.cancel();
    let mut opts = match plenora_io_model::budget::PipelineBudget::builder()
        .cancellation(token)
        .build()
    {
        Ok(bundle) => ReadOptions::from_read_parts(bundle.into_read_parts()),
        Err(error) => unreachable!("bundle di test: {error:?}"),
    };
    let result = preflight_source(
        &DESCRITTORE_DI_PROVA,
        Source::Path(std::path::PathBuf::from("not-observed")),
        &mut opts,
    );
    assert!(matches!(
        result,
        Err(error)
            if error.code == plenora_io_model::IoErrorCode::Cancelled
                && error.phase == ErrorPhase::Probe
    ));
}

#[test]
fn cancellation_before_finish_never_publishes() {
    let finished = Arc::new(AtomicBool::new(false));
    let token = CancellationToken::new();
    let writer: Box<dyn FormatWriter> = Box::new(LimitedWriter {
        inner: Box::new(FinishTrackingWriter {
            finished: finished.clone(),
        }),
        driver: "test",
        limits: opzioni_scrittura().write_limits(),
        rows: 0,
        layer_rows: vec![0],
        input_totals: vec![None],
        failed: false,
        contracts: Vec::new(),
        geometry_validation: None,
        fidelity: FidelityAssessment::lossless(),
        planned_loss: LossReport::default(),
        cancellation: token.clone(),
        budget: opzioni_scrittura().budget().clone(),
        _operation_lease: None,
    });
    token.cancel();

    assert!(matches!(
        writer.finish(),
        Err(error)
            if error.code == plenora_io_model::IoErrorCode::Cancelled
                && error.phase == ErrorPhase::Finalize
    ));
    assert!(!finished.load(Ordering::SeqCst));
}

struct TestReader {
    layer: LayerContract,
    batches: usize,
    fail: bool,
}

impl LayerReader for TestReader {
    fn contract(&self) -> &LayerContract {
        &self.layer
    }

    fn next_batch(&mut self) -> Result<Option<RecordBatch>> {
        if self.fail {
            self.fail = false;
            return Err(PlenoraIoError::contratto_redatto(&PublicMessage::Curated(
                "errore terminale",
            )));
        }
        if self.batches == 0 {
            return Ok(None);
        }
        self.batches -= 1;
        Ok(Some(RecordBatch::new_empty(Arc::new(Schema::empty()))))
    }
}

fn test_reader(batches: usize, fail: bool) -> Box<dyn LayerReader> {
    Box::new(TestReader {
        layer: test_layer(),
        batches,
        fail,
    })
}

fn test_layer() -> LayerContract {
    LayerContract {
        id: LayerId(0),
        name: "layer".to_owned(),
        contract: plenora_io_model::contract::DataContract {
            schema: Arc::new(Schema::empty()),
            geometry: None,
        },
    }
}

fn fixed_batch_reader(values: Vec<i64>) -> Box<dyn LayerReader> {
    let schema = Arc::new(Schema::new(vec![Field::new(
        "value",
        DataType::Int64,
        false,
    )]));
    let batch =
        RecordBatch::try_new(schema.clone(), vec![Arc::new(Int64Array::from(values))]).unwrap();
    Box::new(FixedBatchReader {
        layer: LayerContract {
            id: LayerId(0),
            name: "layer".to_owned(),
            contract: plenora_io_model::contract::DataContract {
                schema,
                geometry: None,
            },
        },
        batch: Some(batch),
    })
}

struct FixedBatchReader {
    layer: LayerContract,
    batch: Option<RecordBatch>,
}

impl LayerReader for FixedBatchReader {
    fn contract(&self) -> &LayerContract {
        &self.layer
    }

    fn next_batch(&mut self) -> Result<Option<RecordBatch>> {
        Ok(self.batch.take())
    }
}

#[test]
fn batch_target_slices_without_reordering_and_releases_gate_at_eof() {
    let gate = SingleReaderGate::new("test");
    let inner = gate
        .open(LayerId(0), || Ok(fixed_batch_reader(vec![0, 1, 2, 3, 4])))
        .unwrap();
    let mut reader = with_batch_target(
        inner,
        BatchTarget {
            target_bytes: 16,
            max_rows: 100,
        },
        CancellationToken::new(),
    );
    assert!(matches!(
        gate.open(LayerId(0), || Ok(test_reader(1, false))),
        Err(error) if error.code == plenora_io_model::IoErrorCode::ReaderBusy
    ));

    let mut sizes = Vec::new();
    let mut values = Vec::new();
    while let Some(batch) = reader.next_batch().unwrap() {
        sizes.push(batch.num_rows());
        values.extend_from_slice(
            batch
                .column(0)
                .as_any()
                .downcast_ref::<Int64Array>()
                .unwrap()
                .values(),
        );
    }
    assert_eq!(sizes, vec![2, 2, 1]);
    assert_eq!(values, vec![0, 1, 2, 3, 4]);
    assert!(gate.open(LayerId(0), || Ok(test_reader(1, false))).is_ok());
}

#[test]
fn single_reader_gate_releases_on_drop_eof_and_error() {
    let gate = SingleReaderGate::new("test");
    let first = gate.open(LayerId(0), || Ok(test_reader(1, false))).unwrap();
    assert!(matches!(
        gate.open(LayerId(0), || Ok(test_reader(1, false))),
        Err(error)
            if error.code == plenora_io_model::IoErrorCode::ReaderBusy
                && error.driver.as_deref() == Some("test")
    ));

    drop(first);
    assert!(gate
        .open(LayerId(0), || {
            Err(PlenoraIoError::contratto_redatto(&PublicMessage::Curated(
                "costruzione fallita",
            )))
        })
        .is_err());
    let mut exhausted = gate.open(LayerId(0), || Ok(test_reader(1, false))).unwrap();
    assert!(exhausted.next_batch().unwrap().is_some());
    assert!(exhausted.next_batch().unwrap().is_none());
    let after_eof = gate.open(LayerId(0), || Ok(test_reader(1, false))).unwrap();
    drop(after_eof);

    let mut failed = gate.open(LayerId(0), || Ok(test_reader(0, true))).unwrap();
    assert!(failed.next_batch().is_err());
    assert!(gate.open(LayerId(0), || Ok(test_reader(1, false))).is_ok());
}

#[test]
fn cancelled_reader_releases_single_reader_lease() {
    let gate = SingleReaderGate::new("test");
    let inner = gate.open(LayerId(0), || Ok(test_reader(1, false))).unwrap();
    let token = CancellationToken::new();
    let mut reader = with_cancellation(inner, token.clone());
    token.cancel();

    assert!(matches!(
        reader.next_batch(),
        Err(error)
            if error.code == plenora_io_model::IoErrorCode::Cancelled
                && error.phase == ErrorPhase::Read
    ));
    assert!(gate.open(LayerId(0), || Ok(test_reader(1, false))).is_ok());
}

fn crs_reader(crs_id: &str, srid: i32) -> Box<dyn LayerReader> {
    let schema = Arc::new(Schema::new(vec![Field::new(
        "geometry",
        DataType::Binary,
        true,
    )]));
    let mut geometry = GeometryColumnContract::wkb_xy(
        FieldId(0),
        "geometry",
        ResolvedCrs::new(Some(crs_id.to_owned()), CrsKind::Geographic, None),
        true,
    );
    geometry.srid = Some(srid);
    Box::new(TestReader {
        layer: LayerContract {
            id: LayerId(0),
            name: "conflicting".to_owned(),
            contract: plenora_io_model::contract::DataContract::new(schema, Some(geometry)),
        },
        batches: 0,
        fail: false,
    })
}

#[test]
fn read_boundary_preserves_and_reports_conflicting_crs_representations() {
    let reader = with_cancellation(crs_reader("EPSG:4326", 3003), CancellationToken::new());

    assert_eq!(
        reader.contract().contract.geometry.as_ref().unwrap().srid,
        Some(3003)
    );
    assert_eq!(
        reader
            .contract()
            .contract
            .geometry
            .as_ref()
            .unwrap()
            .crs
            .id(),
        Some("EPSG:4326")
    );
    let loss = reader.loss_report();
    assert_eq!(
        loss.counts.get(crate::INCONSISTENT_CRS_REPRESENTATIONS),
        Some(&1)
    );
    assert_eq!(loss.esempi_trattenuti(), 1);
    let esempio = loss.esempi_canonici().next().expect("un esempio");
    // `crs_id` non c'e' piu': e' un identificatore che viene dal file. I
    // tre SRID restano, perche' sono codici di autorita' e sono **la cosa**
    // che l'esempio deve dire.
    assert!(!esempio.context.contains("EPSG:4326"));
    assert!(esempio.context.contains("srid=3003"));
    assert!(esempio.context.contains("definition_epsg="));
}

#[test]
fn read_boundary_does_not_report_matching_crs_representations() {
    let reader = with_batch_target(
        crs_reader("EPSG:4326", 4326),
        BatchTarget::default(),
        CancellationToken::new(),
    );

    assert!(reader.loss_report().is_empty());
}

/// Una derivazione decade **solo** quando manca la fonte che la nomina.
///
/// Le quattro provenienze si comportano in modo diverso davanti allo
/// stesso piano, ed e' l'unica cosa che le distingue: chi non dipende dal
/// piano -- il CRS fisso del formato, il runtime che scrive -- non decade
/// mai. Provarlo qui, sulla regola nuda, chiude anche gli angoli che
/// nessuna conversione della CLI raggiunge: `gpkg` con un piano che porta
/// un SRID e non l'identificatore da cui `gpkg` lo ricaverebbe.
#[test]
fn una_derivazione_decade_solo_quando_le_manca_la_fonte() {
    const SINTETIZZABILI: &[&str] = &["EPSG:4326"];
    let da_definizione = CrsRepresentationState::Derived(CrsDerivation::FromDefinition {
        synthesized_for: SINTETIZZABILI,
    });
    let da_identificatore = CrsRepresentationState::Derived(CrsDerivation::FromIdentifier);
    let dal_formato = CrsRepresentationState::Derived(CrsDerivation::FixedByFormat);
    let dal_runtime = CrsRepresentationState::Derived(CrsDerivation::RuntimeResolved);

    let nudo = FontiDelPiano {
        crs_id: Some("EPSG:3003"),
        crs_definition: None,
    };
    let con_definizione = FontiDelPiano {
        crs_id: Some("EPSG:3003"),
        crs_definition: Some("PROJCS[...]"),
    };
    let sintetizzabile = FontiDelPiano {
        crs_id: Some("EPSG:4326"),
        crs_definition: None,
    };
    let senza_niente = FontiDelPiano {
        crs_id: None,
        crs_definition: None,
    };

    // Dalla definizione: il caso dello Shapefile, in tutte e tre le forme.
    assert_eq!(
        stato_per_il_piano(da_definizione, &nudo),
        CrsRepresentationState::Absent,
        "senza definizione da emettere non c'e' niente da cui derivare"
    );
    assert_eq!(
        stato_per_il_piano(da_definizione, &con_definizione),
        da_definizione
    );
    assert_eq!(
        stato_per_il_piano(da_definizione, &sintetizzabile),
        da_definizione
    );

    // Dall'identificatore: il caso del GeoPackage, compreso l'angolo in cui
    // il piano porta l'SRID e non l'identificatore.
    assert_eq!(
        stato_per_il_piano(da_identificatore, &nudo),
        da_identificatore
    );
    assert_eq!(
        stato_per_il_piano(da_identificatore, &senza_niente),
        CrsRepresentationState::Absent
    );

    // I due che non dipendono dal piano: i controesempi alla regola
    // sbagliata, e non si muovono nemmeno davanti a un piano vuoto.
    for stato in [dal_formato, dal_runtime] {
        assert_eq!(stato_per_il_piano(stato, &senza_niente), stato);
        assert_eq!(stato_per_il_piano(stato, &nudo), stato);
    }

    // Gli stati che non sono derivazioni non li tocca nessuno.
    for stato in [
        CrsRepresentationState::Preserved,
        CrsRepresentationState::Absent,
    ] {
        assert_eq!(stato_per_il_piano(stato, &senza_niente), stato);
    }
}

#[test]
fn write_loss_names_each_non_preserved_crs_representation_and_state() {
    let mut loss = LossReport::default();
    record_crs_representation_loss(
        &mut loss,
        Posizione {
            layer_index: Some(0),
            field_index: Some(0),
            type_class: None,
        },
        RappresentazioneDelCrs::CrsId,
        Some(9),
        CrsRepresentationState::Derived(CrsDerivation::FixedByFormat),
    );
    record_crs_representation_loss(
        &mut loss,
        Posizione {
            layer_index: Some(0),
            field_index: Some(0),
            type_class: None,
        },
        RappresentazioneDelCrs::Srid,
        Some(4),
        CrsRepresentationState::Absent,
    );
    record_crs_representation_loss(
        &mut loss,
        Posizione {
            layer_index: Some(0),
            field_index: Some(0),
            type_class: None,
        },
        RappresentazioneDelCrs::CrsDefinition,
        Some(42),
        CrsRepresentationState::Preserved,
    );

    assert_eq!(loss.counts.get("crs_id_not_preserved_derived"), Some(&1));
    assert_eq!(loss.counts.get("srid_not_preserved_absent"), Some(&1));
    assert!(!loss
        .counts
        .contains_key("crs_definition_not_preserved_absent"));
    assert!(loss
        .esempi_canonici()
        .any(|example| example.context.contains("value_bytes=9")));
}

fn geometry_batch(bytes: Option<&[u8]>) -> RecordBatch {
    let schema = Arc::new(Schema::new(vec![Field::new(
        "geometry",
        DataType::Binary,
        true,
    )]));
    let geometry = BinaryArray::from(vec![bytes]);
    RecordBatch::try_new(schema, vec![Arc::new(geometry)]).unwrap()
}

fn xy_contract(nullable: bool) -> GeometryColumnContract {
    GeometryColumnContract::wkb_xy(FieldId(0), "geometry", CrsResolution::Missing, nullable)
}

#[test]
fn runtime_geometry_validation_rejects_hidden_z_payload() {
    let xyz = WkbGeometry {
        value: WkbValue::Point(WkbCoordinate {
            x: 1.0,
            y: 2.0,
            z: Some(3.0),
            m: None,
        }),
        dimensions: CoordinateDimensions::Xyz,
        srid: None,
    };
    let bytes = encode_wkb(&xyz, WkbFlavor::Iso).unwrap();
    let result = validate_geometry_batch_at(
        "test",
        WKB_XY_GEOMETRY,
        Some(&xy_contract(true)),
        &geometry_batch(Some(&bytes)),
        PipelineLimits::default().wkb_limits(),
        0,
        Some(1),
    );
    assert!(matches!(
        result,
        Err(error)
            if error.capability_reason == Some(CapabilityReason::CoordinateDimensions)
    ));
}

#[test]
fn runtime_geometry_validation_rejects_undeclared_ewkb_srid() {
    let ewkb = WkbGeometry {
        value: WkbValue::Point(WkbCoordinate {
            x: 1.0,
            y: 2.0,
            z: None,
            m: None,
        }),
        dimensions: CoordinateDimensions::Xy,
        srid: Some(4326),
    };
    let bytes = encode_wkb(&ewkb, WkbFlavor::Ewkb).unwrap();
    let result = validate_geometry_batch_at(
        "test",
        WKB_XY_GEOMETRY,
        Some(&xy_contract(true)),
        &geometry_batch(Some(&bytes)),
        PipelineLimits::default().wkb_limits(),
        0,
        Some(1),
    );
    assert!(matches!(
        result,
        Err(error)
            if error.capability_reason == Some(CapabilityReason::GeometryEncoding)
    ));
}

#[test]
fn runtime_geometry_validation_enforces_nullability() {
    let result = validate_geometry_batch_at(
        "test",
        WKB_XY_GEOMETRY,
        Some(&xy_contract(false)),
        &geometry_batch(None),
        PipelineLimits::default().wkb_limits(),
        0,
        Some(1),
    );
    assert!(matches!(
        result,
        Err(error) if error.capability_reason == Some(CapabilityReason::Nullability)
    ));
}

#[test]
fn runtime_write_rejections_have_bounded_global_row_diagnostics() {
    let schema = Arc::new(Schema::new(vec![Field::new(
        "geometry",
        DataType::Binary,
        true,
    )]));
    let geometry = BinaryArray::from(vec![Some(&[1_u8, 1, 0][..]), None, Some(&[1_u8, 1, 0][..])]);
    let batch = RecordBatch::try_new(schema, vec![Arc::new(geometry)]).unwrap();

    let error = validate_geometry_batch_at(
        "test",
        WKB_XY_GEOMETRY,
        Some(&xy_contract(false)),
        &batch,
        PipelineLimits::default().wkb_limits(),
        1_000,
        Some(1_003),
    )
    .unwrap_err();

    assert_eq!(error.category, ErrorCategory::DataMapping);
    assert_eq!(error.phase, ErrorPhase::Write);
    let diagnostics = error.row_diagnostics.as_deref().unwrap();
    assert_eq!(diagnostics.observed_total, 3);
    assert_eq!(diagnostics.input_total, Some(1_003));
    assert_eq!(diagnostics.examples[0].source_index, 1_000);
    assert_eq!(diagnostics.examples[1].source_index, 1_001);
    assert_eq!(diagnostics.examples[2].source_index, 1_002);
    assert!(diagnostics.validate().is_ok());
}

/// Senza `input_total` manca il **report**, non la causa.
///
/// Questo test asseriva l'opposto: che l'errore diventasse un `Contract`
/// sull'`input_total` mancante, con categoria `InvalidPlan` e fase
/// `Validate`. Fissava un difetto come requisito — la causa primaria, cioe'
/// la riga rifiutata, veniva sostituita da una condizione
/// dell'infrastruttura diagnostica, e chi leggeva l'errore vedeva un
/// problema interno al posto del proprio.
///
/// Il report resta assente, perche' `plenora-io-row-diagnostics-v1`
/// pretende `input_total` positivo e inventarlo sarebbe peggio che
/// ometterlo. Tutto il resto sopravvive: categoria, fase, causa, e la
/// ragione di capability.
#[test]
fn row_scoped_write_rejection_without_input_total_keeps_the_primary_cause() {
    let error = write_row_rejection("test", 0, 1, &[(0, "test.rejected", "value")], None);

    assert_eq!(error.category, ErrorCategory::DataMapping);
    assert_eq!(error.phase, ErrorPhase::Write);
    assert_eq!(error.driver.as_deref(), Some("test"));
    assert!(
        error.message.contains("test.rejected"),
        "senza report la causa deve stare nel messaggio: {error}"
    );
    assert!(
        !error.message.contains("input_total"),
        "l'assenza del totale non e' la causa del rifiuto: {error}"
    );
    // Il totale non viene inventato.
    assert!(error.row_diagnostics.is_none());

    // Controprova: con il totale il report c'e', e porta la stessa causa.
    let con_totale = write_row_rejection("test", 0, 1, &[(0, "test.rejected", "value")], Some(1));
    let report = con_totale.row_diagnostics.as_deref().unwrap();
    assert_eq!(report.input_total, Some(1));
    assert_eq!(report.counts["test.rejected"], 1);
    assert_eq!(con_totale.category, ErrorCategory::DataMapping);
}

#[test]
fn row_diagnostics_hide_unattestable_columns_without_losing_causes() {
    for invalid in [String::new(), "private".repeat(40)] {
        let error = write_row_rejection(
            "test",
            0,
            1,
            &[(0, "test.cell_not_representable", invalid.as_str())],
            Some(1),
        );
        let diagnostics = error.row_diagnostics.as_deref().unwrap();
        assert_eq!(diagnostics.counts["test.cell_not_representable"], 1);
        assert_eq!(diagnostics.examples[0].column, None);
        assert!(diagnostics
            .knowledge_limits
            .as_deref()
            .unwrap()
            .contains(&ROW_DIAGNOSTIC_COLUMN_UNATTESTABLE.to_owned()));
        assert!(diagnostics.validate().is_ok());
    }

    let unicode = "citta_ðŸŒ";
    let error = write_row_rejection(
        "test",
        0,
        1,
        &[(0, "test.cell_not_representable", unicode)],
        Some(1),
    );
    let diagnostics = error.row_diagnostics.as_deref().unwrap();
    assert_eq!(diagnostics.examples[0].column.as_deref(), Some(unicode));
    assert!(diagnostics.validate().is_ok());
}

#[test]
fn read_row_error_emits_examples_only_for_attestable_indices() {
    let attested = read_row_error(
        PlenoraIoError::formato_redatto("test", &PublicMessage::Curated("bad row")),
        Some(7),
        "test.invalid_row",
        Some("value"),
    );
    let diagnostics = attested.row_diagnostics.as_deref().unwrap();
    assert_eq!(
        diagnostics.completeness,
        RowDiagnosticsCompleteness::Partial
    );
    assert_eq!(diagnostics.examples[0].source_index, 7);
    assert!(diagnostics.validate().is_ok());

    let unknown = read_row_error(
        PlenoraIoError::formato_redatto("test", &PublicMessage::Curated("bad row")),
        None,
        "test.invalid_row",
        Some("value"),
    );
    let diagnostics = unknown.row_diagnostics.as_deref().unwrap();
    assert_eq!(
        diagnostics.completeness,
        RowDiagnosticsCompleteness::Unknown
    );
    assert!(diagnostics.examples.is_empty());
    assert!(diagnostics
        .knowledge_limits
        .as_deref()
        .unwrap()
        .contains(&"source_row_identity_unattestable".to_owned()));
    assert!(diagnostics.validate().is_ok());
}

#[test]
fn non_geometry_row_rejection_uses_a_non_geometry_capability_reason() {
    let error = write_row_rejection(
        "test",
        0,
        1,
        &[(0, "test.cell_not_representable", "value")],
        Some(1),
    );
    assert_eq!(
        error.capability_reason,
        Some(CapabilityReason::TypeNotRepresentable)
    );
}

fn geoarrow_field(name: &str, dimensions: Option<&str>) -> Field {
    let mut metadata = HashMap::from([(
        ARROW_EXTENSION_NAME_KEY.to_owned(),
        GEOARROW_WKB_EXTENSION.to_owned(),
    )]);
    if let Some(dimensions) = dimensions {
        metadata.insert(PLENORA_DIMENSIONS_KEY.to_owned(), dimensions.to_owned());
    }
    Field::new(name, DataType::Binary, true).with_metadata(metadata)
}

/// Un descrittore che dichiara perdita su tutt'e tre gli assi, cosi' che i
/// tre rami redatti di `assess_write_contract` si accendano tutti.
fn descrittore_che_dichiara_perdite() -> crate::descriptor::FormatDescriptor {
    crate::descriptor::FormatDescriptor::const_new(
        "prova-con-perdite",
        crate::descriptor::Direction::Bidirectional,
        crate::descriptor::ReadMode::StreamingSequential,
        crate::descriptor::NativeReadMode::StreamingSequential,
        crate::descriptor::DeliverySemantics::OperationAtomic,
        crate::descriptor::BufferingStrategy::AdaptiveMemoryThenDisk,
        crate::descriptor::DeterminismLevel::Semantic,
        Some(crate::descriptor::WriteMode::Streaming),
        Some(crate::descriptor::DeterminismLevel::Semantic),
        false,
        false,
        crate::descriptor::ReaderConcurrency::SingleActiveReader,
        crate::descriptor::ProjectionSupport::None,
        crate::descriptor::PredicatePruningSupport::None,
        crate::descriptor::SpatialPruningSupport::None,
        crate::descriptor::CrsHandling::Embedded,
        crate::descriptor::Fidelity::Conditional,
        crate::descriptor::Runtime::PureRust,
        false,
        None,
        Some(crate::descriptor::FormatWriteCapabilities {
            field_names: crate::descriptor::DBF_FIELD_NAMES,
            // Nessun tipo ammesso: cosi' il ramo della coercizione si
            // accende su ogni attributo invece che su alcuni.
            allowed_types: &[],
            type_coercion: crate::descriptor::TypeCoercionPolicy::LossReported,
            attributes: crate::descriptor::AttributeWriteSupport::LossReported,
            geometry: WKB_XY_GEOMETRY,
            crs: crate::descriptor::CrsWriteSupport::Embedded,
            crs_representations: crate::descriptor::CrsRepresentationCapabilities::new(
                crate::descriptor::CrsRepresentationState::Preserved,
                crate::descriptor::CrsRepresentationState::Preserved,
                crate::descriptor::CrsRepresentationState::Preserved,
            ),
            nullability: crate::descriptor::NullabilitySupport::FormatDefined,
            multi_layer: true,
            sink_path: crate::SinkPathConstraint::Free,
        }),
        plenora_io_model::format_options::SchemaOpzioniFormato::VUOTO,
        &["test"],
        1,
        1,
        1,
    )
}

/// Due layer, due attributi ciascuno, con nomi che nessun testo curato
/// potrebbe contenere per caso.
fn piano_con_nomi_canary() -> WritePlan {
    let campi = || {
        vec![
            Field::new("CANARY_CAMPO_àèì'\"uno", DataType::Int64, true),
            Field::new("CANARY_CAMPO_àèì'\"due", DataType::Utf8, true),
        ]
    };
    WritePlan {
        layers: vec![
            crate::request::WriteLayer {
                name: "CANARY_LAYER_àèì'\"alfa".to_owned(),
                contract: plenora_io_model::contract::DataContract {
                    schema: Arc::new(Schema::new(campi())),
                    geometry: None,
                },
            },
            crate::request::WriteLayer {
                name: "CANARY_LAYER_àèì'\"beta".to_owned(),
                contract: plenora_io_model::contract::DataContract {
                    schema: Arc::new(Schema::new(campi())),
                    geometry: None,
                },
            },
        ],
    }
}

/// I nomi presi dal file restano nel v1 e spariscono dal v2.
///
/// I nomi della fixture sono canary con accenti e apostrofo -- l'apostrofo
/// perche' e' il carattere che le frasi congelate mettono fra virgolette --
/// e la sonda pretende che **nessuno** compaia in cio' che il v2 trattiene,
/// nemmeno dentro una stringa piu' lunga.
#[test]
fn i_nomi_del_file_restano_nel_v1_e_spariscono_dal_v2() {
    let valutazione = assess_write_contract(
        &descrittore_che_dichiara_perdite(),
        &piano_con_nomi_canary(),
    );

    // Il v1 li porta, alla lettera e non ricostruiti: sono i dettagli
    // dinamici, non un esempio statico.
    let v1: Vec<&str> = valutazione
        .ragioni_v1()
        .iter()
        .map(crate::loss::FidelityReason::detail_v1)
        .collect();
    assert!(!v1.is_empty(), "la fixture deve produrre ragioni");
    assert!(
        v1.iter().any(|d| d.contains("CANARY_LAYER_àèì'\"alfa")),
        "il v1 deve conservare il nome del layer: {v1:?}"
    );
    assert!(
        v1.iter().any(|d| d.contains("CANARY_CAMPO_àèì'\"due")),
        "il v1 deve conservare il nome dell'attributo: {v1:?}"
    );

    // Il v2 non li porta, e porta invece gli indici.
    //
    // La posizione si pretende sui **quattro codici redatti**, non su ogni
    // ragione: `for_format` ne aggiunge una di livello formato, che una
    // posizione non ce l'ha e non deve averla -- non parla di un layer ne'
    // di un campo. Pretenderla anche li' verificherebbe una cosa falsa.
    let mut con_indice_di_campo = 0_usize;
    for ragione in valutazione.ragioni_canoniche() {
        assert!(
            !ragione.detail.contains("CANARY"),
            "un nome del file e' rimasto nel testo curato: {}",
            ragione.detail
        );
        let redatta = matches!(
            ragione.code,
            FidelityReasonCode::AttributeLoss
                | FidelityReasonCode::TypeCoercion
                | FidelityReasonCode::NullabilityChanged
                | FidelityReasonCode::StructureChanged
        );
        if redatta {
            assert!(
                ragione.posizione.layer_index.is_some(),
                "una ragione redatta senza layer_index non dice dove: {ragione:?}"
            );
            if ragione.posizione.field_index.is_some() {
                con_indice_di_campo += 1;
            }
        }
    }
    assert!(
        con_indice_di_campo >= 2,
        "gli indici di campo devono distinguere cio' che i nomi distinguevano"
    );

    // E i due layer restano distinti: se la redazione li appiattisse,
    // la deduplicazione canonica li fonderebbe in uno.
    let layer_visti: std::collections::BTreeSet<_> = valutazione
        .ragioni_canoniche()
        .filter_map(|r| r.posizione.layer_index)
        .collect();
    assert_eq!(
        layer_visti,
        [0, 1].into_iter().collect(),
        "i due layer devono restare distinti dopo la redazione"
    );
}

/// La fusione conserva l'identita' legacy, che `add_reason(code, detail)`
/// avrebbe buttato via facendo cambiare byte alla sezione v1.
#[test]
fn la_fusione_non_perde_la_frase_congelata() {
    let letta = assess_write_contract(
        &descrittore_che_dichiara_perdite(),
        &piano_con_nomi_canary(),
    );
    let mut fusa = FidelityAssessment::con_livello(crate::descriptor::Fidelity::Approximating);
    fusa.merge(&letta);

    let prima: Vec<&str> = letta
        .ragioni_v1()
        .iter()
        .map(crate::loss::FidelityReason::detail_v1)
        .collect();
    let dopo: Vec<&str> = fusa
        .ragioni_v1()
        .iter()
        .map(crate::loss::FidelityReason::detail_v1)
        .collect();
    assert_eq!(prima, dopo, "la fusione deve conservare le frasi del v1");

    let posizioni_prima: Vec<_> = letta.ragioni_canoniche().map(|r| r.posizione).collect();
    let posizioni_dopo: Vec<_> = fusa.ragioni_canoniche().map(|r| r.posizione).collect();
    assert_eq!(
        posizioni_prima, posizioni_dopo,
        "la fusione deve conservare le posizioni del v2"
    );
}

fn legacy_plan(fields: Vec<Field>) -> WritePlan {
    WritePlan {
        layers: vec![crate::request::WriteLayer {
            name: "layer".to_owned(),
            contract: plenora_io_model::contract::DataContract {
                schema: Arc::new(Schema::new(fields)),
                geometry: None,
            },
        }],
    }
}

#[test]
fn legacy_geometry_defaults_xy_only_when_dimensions_are_absent() {
    let absent =
        geometry_contracts_for_validation(&legacy_plan(vec![geoarrow_field("geometry", None)]))
            .unwrap();
    let explicit_unknown = geometry_contracts_for_validation(&legacy_plan(vec![geoarrow_field(
        "geometry",
        Some("unknown"),
    )]))
    .unwrap();

    assert_eq!(
        absent[0].as_ref().unwrap().dimensions,
        CoordinateDimensions::Xy
    );
    assert_eq!(
        explicit_unknown[0].as_ref().unwrap().dimensions,
        CoordinateDimensions::Unknown
    );
}

#[test]
fn ambiguous_or_invalid_legacy_geometry_metadata_is_rejected() {
    let ambiguous = legacy_plan(vec![
        geoarrow_field("geometry_a", None),
        geoarrow_field("geometry_b", None),
    ]);
    let invalid = legacy_plan(vec![geoarrow_field("geometry", Some("future"))]);

    assert!(matches!(
        geometry_contracts_for_validation(&ambiguous),
        Err(error) if error.code == plenora_io_model::IoErrorCode::Contract
    ));
    assert!(matches!(
        geometry_contracts_for_validation(&invalid),
        Err(error) if error.code == plenora_io_model::IoErrorCode::Contract
    ));
}

// ---- Le opzioni sul modello unificato (Lotto 0, S4.e) ----
//
// Fino a S4.d qui vivevano i test del ponte transitorio: parita' fra i
// due rami, guardie direzionali, "il ramo pipeline non consulta Limits".
// Con un solo modello non descrivono piu' nulla — non c'e' un secondo
// ramo con cui confrontarsi — e sono stati rimossi invece di essere
// riscritti in forme che passano per costruzione.
//
// Restano i test che dicono ancora qualcosa: che gli scalari arrivano
// dai limiti della pipeline, e che il permit e' one-shot.

const QUOTA_INPUT_BYTES: u64 = 4_242;
const QUOTA_INPUT_ENTRIES: u64 = 37;
const QUOTA_ROWS: usize = 911;
const QUOTA_COLUMNS: usize = 17;
const QUOTA_VERTICES: usize = 5_000;
const QUOTA_WKB_CELL: usize = 8_192;
const QUOTA_WKB_COMPONENTS: usize = 640;
const QUOTA_WKB_DEPTH: usize = 9;
const QUOTA_OUTPUT_BYTES: u64 = 77_000;

fn limiti_di_prova() -> PipelineLimits {
    PipelineLimits::default()
        .with_max_input_bytes(QUOTA_INPUT_BYTES)
        .with_max_input_entries(QUOTA_INPUT_ENTRIES)
        .with_max_rows(QUOTA_ROWS as u64)
        .with_max_columns(QUOTA_COLUMNS as u64)
        .with_max_vertices(QUOTA_VERTICES)
        .with_max_output_bytes(QUOTA_OUTPUT_BYTES)
        .with_max_wkb_cell_bytes(QUOTA_WKB_CELL)
        .with_max_wkb_components(QUOTA_WKB_COMPONENTS)
        .with_max_wkb_depth(QUOTA_WKB_DEPTH)
}

fn bundle_di_prova() -> plenora_io_model::budget::PipelineBundle {
    match plenora_io_model::budget::PipelineBudget::builder()
        .limits(limiti_di_prova())
        .build()
    {
        Ok(bundle) => bundle,
        Err(error) => unreachable!("limiti di prova non validi: {error:?}"),
    }
}

fn opzioni_lettura() -> ReadOptions {
    ReadOptions::from_read_parts(bundle_di_prova().into_read_parts())
}

fn opzioni_scrittura() -> WriteOptions {
    WriteOptions::from_write_parts(bundle_di_prova().into_write_parts())
}

fn opzioni_pipeline(limits: PipelineLimits) -> ReadOptions {
    match plenora_io_model::budget::PipelineBudget::builder()
        .limits(limits)
        .build()
    {
        Ok(bundle) => ReadOptions::from_read_parts(bundle.into_read_parts()),
        Err(error) => unreachable!("limiti di test non validi: {error:?}"),
    }
}

#[test]
fn gli_scalari_arrivano_dai_limiti_della_pipeline() {
    let opts = opzioni_lettura();

    assert_eq!(opts.max_input_bytes(), QUOTA_INPUT_BYTES);
    assert_eq!(opts.max_input_entries(), QUOTA_INPUT_ENTRIES);
    assert_eq!(opts.max_rows(), QUOTA_ROWS);
    assert_eq!(opts.max_columns(), QUOTA_COLUMNS);
    assert_eq!(opts.max_vertices(), QUOTA_VERTICES);
    assert_eq!(opts.wkb_limits().max_cell_bytes, QUOTA_WKB_CELL);
    // Composto con `max_vertices`, come faceva `Limits::effective_wkb()`.
    assert_eq!(opts.wkb_limits().max_components, QUOTA_WKB_COMPONENTS);
    assert_eq!(opts.wkb_limits().max_depth, QUOTA_WKB_DEPTH);
}

#[test]
fn la_vista_di_scrittura_arriva_dagli_stessi_limiti() {
    let wopts = opzioni_scrittura();
    let vista = wopts.write_limits();

    assert_eq!(vista.max_columns, QUOTA_COLUMNS);
    assert_eq!(vista.max_rows, QUOTA_ROWS);
    assert_eq!(vista.wkb.max_cell_bytes, QUOTA_WKB_CELL);
    assert_eq!(vista.wkb.max_components, QUOTA_WKB_COMPONENTS);
    assert_eq!(vista.wkb.max_depth, QUOTA_WKB_DEPTH);
    // Senza input osservato non si applica alcuna espansione: il tetto e'
    // quello assoluto.
    assert_eq!(wopts.max_output_bytes(), QUOTA_OUTPUT_BYTES);
}

#[test]
fn ensure_active_osserva_la_cancellazione_del_context() {
    let token = CancellationToken::new();
    let bundle = match plenora_io_model::budget::PipelineBudget::builder()
        .limits(limiti_di_prova())
        .cancellation(token.clone())
        .build()
    {
        Ok(bundle) => bundle,
        Err(error) => unreachable!("bundle di prova: {error:?}"),
    };
    let opts = ReadOptions::from_read_parts(bundle.into_read_parts());

    assert!(opts.ensure_active().is_ok());
    token.cancel();
    assert!(opts.ensure_active().is_err());
}

#[test]
fn permit_snapshot_e_budget_attraversano_i_costruttori_senza_rigenerazione() {
    let bundle = bundle_di_prova();
    let contesto = bundle.context().clone();
    let mut opts = ReadOptions::from_read_parts(bundle.into_read_parts());

    // Il budget e' lo stesso, non uno nuovo con gli stessi limiti:
    // `is_same_pipeline` confronta l'identita' del context, non i valori.
    assert!(opts.budget().context().is_same_pipeline(&contesto));

    // Il permit e' l'esemplare unico trasportato dalle parti: lo prova il
    // fatto che il context lo accetti. Un permit rigenerato porterebbe un
    // pipeline id che questo context rifiuta.
    let permit = opts
        .take_input_permit()
        .expect("le parti read trasportano il permit");
    assert!(contesto.observe_input(permit).is_ok());
    assert!(
        opts.take_input_permit().is_none(),
        "il permit e' spendibile una sola volta"
    );

    // La cancellazione e' quella del context, non un token nuovo.
    assert!(!opts.cancellation().is_cancelled());
    contesto.cancellation().cancel();
    assert!(opts.cancellation().is_cancelled());
}

#[test]
fn lo_snapshot_atteso_sopravvive_alla_costruzione_dalle_parti_di_scan() {
    // Lo snapshot atteso viene da un'osservazione precedente: qui basta
    // quello di un footprint qualunque, perche' il test guarda il
    // trasporto e non il contenuto.
    let bundle = bundle_di_prova();
    let contesto = bundle.context().clone();
    let (_budget, permit, _atteso) = bundle.into_read_parts().into_components();
    let footprint = contesto
        .observe_input(permit.expect("permit"))
        .expect("osservazione");
    let parts = bundle_di_prova().into_scan_parts(footprint.snapshot());
    let atteso = *parts.expected_footprint();
    let opts = ReadOptions::from_read_parts(
        plenora_io_model::budget::IntoReadParts::into_read_budget_parts(parts),
    );

    assert_eq!(
        opts.expected_footprint().copied(),
        Some(atteso),
        "lo snapshot attraversa il costruttore invariato"
    );
}

#[test]
fn le_opzioni_per_valore_rendono_estraibile_il_permit_una_volta_sola() {
    // Verifica la **forma**, non il comportamento del preflight: la
    // funzione locale sotto imita la firma che `preflight_source` usa,
    // non e' `preflight_source`. Con `open` che riceve le opzioni per
    // valore, una funzione che le prende `&mut` puo' estrarre il permit
    // per move; con `&ReadOptions` non si estrae nulla, e le vie per
    // aggirarlo — `Mutex<Option<InputPermit>>`, o un permit clonato —
    // reintrodurrebbero l'osservazione doppia che il permit esiste per
    // escludere.
    //
    // Il consumo vero e' esercitato dai test del preflight e da quello
    // end-to-end, non da qui.
    fn con_la_stessa_firma_del_preflight(opts: &mut ReadOptions) -> Option<InputPermit> {
        opts.take_input_permit()
    }

    let mut opts = opzioni_lettura();
    assert!(
        con_la_stessa_firma_del_preflight(&mut opts).is_some(),
        "il permit deve essere estraibile attraverso un prestito mutabile"
    );
    assert!(
        con_la_stessa_firma_del_preflight(&mut opts).is_none(),
        "one-shot: la seconda estrazione non deve dare un secondo permit"
    );

    // E le opzioni restano utilizzabili: dopo il preflight e' l'adapter a
    // leggerle, e consumare il permit non consuma le opzioni.
    assert_eq!(opts.max_columns(), QUOTA_COLUMNS);
    assert_eq!(opts.max_input_bytes(), QUOTA_INPUT_BYTES);
}

/// Due percorsi non-UTF-8 distinti non devono collassare sullo stesso
/// digest.
///
/// `to_string_lossy` sostituisce ogni sequenza non valida con U+FFFD:
/// `b"\xff"` e `b"\xfe"` diventano **la stessa** stringa, e il footprint
/// direbbe che due sorgenti diverse sono la stessa. E' il caso che la
/// rappresentazione a byte esclude.
///
/// Solo Unix: su Windows i nomi sono UTF-16 e la collisione non si pone
/// nella stessa forma.
#[cfg(unix)]
#[test]
fn percorsi_non_utf8_distinti_non_collidono_nel_digest() {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;

    let primo = std::path::PathBuf::from(OsStr::from_bytes(b"/tmp/\xff"));
    let secondo = std::path::PathBuf::from(OsStr::from_bytes(b"/tmp/\xfe"));

    assert_eq!(
        primo.to_string_lossy(),
        secondo.to_string_lossy(),
        "la premessa del test: la forma lossy li rende indistinguibili"
    );
    assert_ne!(
        byte_identita_percorso(&primo),
        byte_identita_percorso(&secondo),
        "la forma normalizzata deve restare iniettiva"
    );
}

#[test]
fn la_forma_normalizzata_del_percorso_e_stabile() {
    let percorso = std::path::PathBuf::from("dati/a.csv");
    assert_eq!(
        byte_identita_percorso(&percorso),
        byte_identita_percorso(&percorso),
        "due corse sullo stesso percorso devono dare lo stesso digest"
    );
    assert_ne!(
        byte_identita_percorso(&percorso),
        byte_identita_percorso(&std::path::PathBuf::from("dati/b.csv"))
    );
}

#[test]
fn il_preflight_applica_le_quote_e_pubblica_il_footprint() {
    let mut file = tempfile::NamedTempFile::new().expect("tempfile");
    file.write_all(&[0_u8; 8]).expect("write");

    let mut stretto = opzioni_pipeline(
        plenora_io_model::budget::PipelineLimits::default().with_max_input_bytes(7),
    );
    let errore = preflight_source(
        &DESCRITTORE_DI_PROVA,
        Source::Path(file.path().to_owned()),
        &mut stretto,
    )
    .expect_err("otto byte non stanno in sette");
    assert_eq!(errore.code, plenora_io_model::IoErrorCode::LimitExceeded);

    // Con quota capiente lo stesso file passa, e il footprint pubblicato
    // descrive cio' che e' stato davvero osservato: un file, otto byte.
    let mut largo = opzioni_pipeline(plenora_io_model::budget::PipelineLimits::default());
    let budget = largo.budget().clone();
    assert_eq!(
        budget.context().observed_input(),
        plenora_io_model::budget::ObservedInput::NotObserved,
        "prima del preflight nulla e' osservato"
    );
    assert!(preflight_source(
        &DESCRITTORE_DI_PROVA,
        Source::Path(file.path().to_owned()),
        &mut largo
    )
    .is_ok());
    assert_eq!(
        budget.context().observed_input(),
        plenora_io_model::budget::ObservedInput::Bytes(8)
    );
    assert_eq!(budget.context().entries_visited(), 1);
}

#[test]
fn il_preflight_spende_il_permit_e_non_osserva_due_volte() {
    let file = tempfile::NamedTempFile::new().expect("tempfile");
    let mut opts = opzioni_pipeline(plenora_io_model::budget::PipelineLimits::default());

    assert!(preflight_source(
        &DESCRITTORE_DI_PROVA,
        Source::Path(file.path().to_owned()),
        &mut opts
    )
    .is_ok());
    // Il permit e' stato speso: una seconda osservazione non ha nulla con
    // cui pubblicare, e fallisce invece di lasciare il footprint vuoto.
    let errore = preflight_source(
        &DESCRITTORE_DI_PROVA,
        Source::Path(file.path().to_owned()),
        &mut opts,
    )
    .expect_err("il permit e' one-shot");
    assert_eq!(errore.code, plenora_io_model::IoErrorCode::LimitExceeded);
}
