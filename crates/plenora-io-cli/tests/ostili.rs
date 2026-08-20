//! Prove ostili end-to-end: nessun payload esce dalla busta d'errore.
//!
//! # Perche' un test d'integrazione e non un test di modulo
//!
//! Questo file compila come **crate separato**: vede dei driver soltanto
//! l'API pubblica, esattamente come un consumatore. Un test dentro un driver
//! potrebbe chiamare un helper interno e verificare una busta che nessuno
//! costruisce davvero.
//!
//! Le chiamate attraversano gli entry point veri — `FormatDriver::open`,
//! `OpenDatasetHandle::open_layer_reader`, `LayerReader::next_batch`,
//! `FormatDriver::create` — e la busta e' **serializzata**, non ispezionata
//! via `Display`: `Display` puo' essere corretto mentre il campo `message`
//! porta il payload.
//!
//! # Che cosa si prova
//!
//! 1. il marcatore del payload non compare nella busta serializzata;
//! 2. nemmeno il testo delle dipendenze (nomi di crate, tipi di `io::Error`);
//! 3. il messaggio sta entro `MAX_MESSAGE_BYTES`;
//! 4. il quartetto e' quello atteso, **`remote_effect` compreso** — e' il
//!    quinto asse, e un errore locale che dichiarasse `Unknown` farebbe
//!    ritentare un'operazione che non e' mai partita;
//! 5. apertura, lettura e scrittura sono prove **separate**: fallire in
//!    apertura e fallire a meta' stream sono due contratti diversi;
//! 6. una lettura fallita non consegna righe, una scrittura fallita non
//!    lascia destinazione;
//! 7. nessun panic: ogni chiamata torna un `Result`.
//!
//! # Nessuna scorciatoia introdotta per i test
//!
//! Le buste sono costruite con i costruttori pubblici redatti, gli stessi che
//! usa la produzione. Non esiste in questo file una via di costruzione degli
//! errori che non esista anche altrove.

use std::path::{Path, PathBuf};

use plenora_io_core::driver::{FormatDriver, ReadOptions, Sink, Source, WriteOptions};
use plenora_io_core::request::{
    BatchTarget, ProjectionMode, ReadRequest, ReadScope, WriteLayer, WritePlan,
};
use plenora_io_model::budget::{PipelineBudget, PipelineLimits};
use plenora_io_model::contract::LayerId;
use plenora_io_model::{
    ErrorCategory, ErrorPhase, PlenoraIoError, RemoteEffect, RetryDisposition, MAX_MESSAGE_BYTES,
};

/// Il marcatore che ogni fixture porta in chiaro.
///
/// Versionato insieme alle fixture: se qui e nei file divergesse, il test
/// passerebbe cercando qualcosa che non c'e'. La sonda
/// `il_marcatore_e_davvero_nelle_fixture` lo impedisce.
const MARCATORE: &str = "ZZ-MARCATORE-PAYLOAD-9F3A-ZZ";

/// Frammenti di testo che tradiscono una dipendenza.
///
/// Non sono un elenco esaustivo — non puo' esserlo — ma coprono le forme che
/// la migrazione ha effettivamente trovato nei dieci driver: nomi di crate,
/// varianti di `io::ErrorKind`, forme `Debug`, percorsi sorgente.
///
/// **Non contiene i nomi dei formati.** «Parquet», «`GeoJSON`», «Shapefile»
/// sono parole nostre e compaiono legittimamente nei messaggi: metterli qui
/// renderebbe il test rosso su un messaggio corretto — ed e' esattamente
/// quello che e' successo alla prima stesura, dove `"parquet"` matchava
/// `"driver":"geoparquet"` nella busta.
const TESTO_DI_DIPENDENZA: &[&str] = &[
    "calamine",
    "quick_xml",
    "quick-xml",
    "rusqlite",
    "serde_json",
    "ArrowError",
    "XlsxError",
    "SqliteFailure",
    "InvalidData",
    "UnexpectedEof",
    "Custom {",
    "Error {",
    "panicked",
    "unwrap",
    "src/",
    ".rs:",
];

fn fixture(nome: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/ostili")
        .join(nome)
}

fn opzioni_lettura() -> ReadOptions {
    let bundle = PipelineBudget::builder()
        .limits(PipelineLimits::default())
        .build()
        .expect("i limiti predefiniti sono validi");
    ReadOptions::from_read_parts(bundle.into_read_parts())
}

fn opzioni_scrittura() -> WriteOptions {
    let bundle = PipelineBudget::builder()
        .limits(PipelineLimits::default())
        .build()
        .expect("i limiti predefiniti sono validi");
    WriteOptions::from_write_parts(bundle.into_convert_parts().into_parts().1)
}

/// La busta **serializzata**, non `Display`.
///
/// `Display` puo' essere innocuo mentre `message` porta il payload: sono due
/// superfici diverse, e quella che raggiunge un consumatore e' questa.
fn busta(errore: &PlenoraIoError) -> String {
    serde_json::to_string(errore).expect("l'errore si serializza")
}

/// Il quartetto atteso, `remote_effect` compreso.
#[derive(Clone, Copy)]
struct Assi {
    category: ErrorCategory,
    phase: ErrorPhase,
    remote_effect: RemoteEffect,
    retry: RetryDisposition,
}

/// Ogni errore ostile passa di qui, qualunque sia la fase.
///
/// Le affermazioni sono **due, distinte**, e tenerle separate e' il punto.
///
/// 1. `message` — il testo libero, cioe' il soggetto di INV-10 — non contiene
///    ne' il marcatore ne' testo di dipendenza. Qui non c'e' tolleranza.
/// 2. Nel tipo Rust serializzato il marcatore puo' comparire **solo** nello
///    slot tipizzato `field`, e da nessun'altra parte.
///
/// La seconda non e' una concessione: `field` porta un nome che viene da un
/// contratto — per una scrittura e' il nome che il chiamante ha scritto nel
/// proprio piano, per una lettura e' un nome inferito dal file. La decisione
/// di spostarlo li' invece di interpolarlo nel messaggio e' della tranche 2, ed
/// e' proprio cio' che rende il testo curato. **`field` non e' sul wire v1**:
/// `err_doc` emette sei chiavi e quella non e' fra loro — lo verifica il test
/// che esegue il binario.
fn verifica(contesto: &str, errore: &PlenoraIoError, assi: Option<Assi>) {
    let serializzata = busta(errore);

    assert!(
        !errore.message.contains(MARCATORE),
        "{contesto}: il payload e' nel messaggio: {}",
        errore.message
    );

    for frammento in TESTO_DI_DIPENDENZA {
        assert!(
            !errore.message.contains(frammento),
            "{contesto}: testo di dipendenza `{frammento}` nel messaggio: {}",
            errore.message
        );
    }

    // Il marcatore, se c'e', sta solo dove il tipo lo ammette.
    if serializzata.contains(MARCATORE) {
        assert_eq!(
            errore.field.as_deref(),
            Some(MARCATORE),
            "{contesto}: il marcatore e' nella busta ma fuori da `field`: {serializzata}"
        );
        let senza_field =
            serializzata.replace(&format!("\"field\":\"{MARCATORE}\""), "\"field\":null");
        assert!(
            !senza_field.contains(MARCATORE),
            "{contesto}: il marcatore compare piu' di una volta: {serializzata}"
        );
    }

    assert!(
        errore.message.len() <= MAX_MESSAGE_BYTES,
        "{contesto}: messaggio da {} byte, tetto {MAX_MESSAGE_BYTES}",
        errore.message.len()
    );

    // Un errore locale non deve dichiarare un effetto remoto ignoto: chi legge
    // ritenterebbe un'operazione che non e' mai partita.
    assert_eq!(
        errore.remote_effect,
        RemoteEffect::None,
        "{contesto}: effetto remoto inatteso su un driver locale"
    );

    if let Some(attesi) = assi {
        assert_eq!(errore.category, attesi.category, "{contesto}: category");
        assert_eq!(errore.phase, attesi.phase, "{contesto}: phase");
        assert_eq!(
            errore.remote_effect, attesi.remote_effect,
            "{contesto}: remote_effect"
        );
        assert_eq!(errore.retry, attesi.retry, "{contesto}: retry");
    }
}

/// Un driver, la sua fixture d'apertura e l'estensione della destinazione.
struct Caso {
    nome: &'static str,
    driver: Box<dyn FormatDriver>,
    fixture: &'static str,
    /// Dichiarata, non dedotta dal nome della fixture.
    ///
    /// Dedurla richiedeva un ripiego per il caso «nessuna estensione», e un
    /// ripiego che non scatta mai e' peggio di inutile: nasconderebbe una
    /// fixture rinominata dietro un nome di destinazione plausibile.
    estensione: &'static str,
}

/// I dieci driver.
fn driver_e_fixture() -> Vec<Caso> {
    vec![
        Caso {
            nome: "geoparquet",
            driver: Box::new(driver_geoparquet::GeoParquetDriver),
            fixture: "apertura.parquet",
            estensione: "parquet",
        },
        Caso {
            nome: "geojson",
            driver: Box::new(driver_geojson::GeoJsonDriver),
            fixture: "apertura.geojson",
            estensione: "geojson",
        },
        Caso {
            nome: "csv",
            driver: Box::new(driver_csv::CsvDriver),
            fixture: "apertura.csv",
            estensione: "csv",
        },
        Caso {
            nome: "gpkg",
            driver: Box::new(driver_gpkg::GpkgDriver),
            fixture: "apertura.gpkg",
            estensione: "gpkg",
        },
        Caso {
            nome: "shp",
            driver: Box::new(driver_shp::ShpDriver),
            fixture: "apertura.shp",
            estensione: "shp",
        },
        Caso {
            nome: "kml",
            driver: Box::new(driver_kml::KmlDriver),
            fixture: "apertura.kml",
            estensione: "kml",
        },
        Caso {
            nome: "xls",
            driver: Box::new(driver_xls::XlsDriver),
            fixture: "apertura.xlsx",
            estensione: "xlsx",
        },
        Caso {
            nome: "dxf",
            driver: Box::new(driver_dxf::DxfDriver),
            fixture: "apertura.dxf",
            estensione: "dxf",
        },
        Caso {
            nome: "ipc",
            driver: Box::new(driver_ipc::IpcDriver),
            fixture: "apertura.arrow",
            estensione: "arrow",
        },
        Caso {
            nome: "filegdb",
            driver: Box::new(driver_filegdb::FileGdbDriver),
            fixture: "apertura.gdb",
            estensione: "gdb",
        },
    ]
}

// --- la premessa: le fixture contengono davvero il marcatore ----------------

/// Senza questa sonda l'intera batteria potrebbe essere **vacua**.
///
/// Se una fixture perdesse il marcatore — riscritta, troncata, sostituita —
/// tutti gli `assert!(!contiene(MARCATORE))` passerebbero senza provare nulla.
#[test]
fn il_marcatore_e_davvero_nelle_fixture() {
    let attese = [
        "apertura.parquet",
        "apertura.geojson",
        "apertura.csv",
        "apertura.gpkg",
        "apertura.shp",
        "apertura.kml",
        "apertura.xlsx",
        "apertura.dxf",
        "apertura.arrow",
        "apertura.gdb/gdb",
        "lettura.csv",
        "lettura.geojson",
        "lettura.kml",
    ];
    for nome in attese {
        let Ok(byte) = std::fs::read(fixture(nome)) else {
            panic!("fixture {nome} illeggibile");
        };
        let testo = String::from_utf8_lossy(&byte);
        assert!(
            testo.contains(MARCATORE),
            "{nome}: la fixture non contiene il marcatore, e il test su di essa sarebbe vacuo"
        );
    }
}

// --- apertura ----------------------------------------------------------------

/// Dieci driver, un file che non e' nel loro formato.
///
/// La fase attesa e' quella d'apertura: `Prepare`, `Validate` o `Read` a
/// seconda di dove il driver riconosce il guasto. Il quartetto completo non e'
/// fissato qui — lo fissa lo snapshot per sito — mentre le proprieta' che
/// contano per un consumatore lo sono.
#[test]
fn nessun_driver_fa_uscire_il_payload_in_apertura() {
    for Caso {
        nome,
        driver,
        fixture: file,
        ..
    } in driver_e_fixture()
    {
        let percorso = fixture(file);
        assert!(percorso.exists(), "{nome}: fixture {file} assente");

        match driver.open(Source::Path(percorso), opzioni_lettura()) {
            Ok(_) => panic!("{nome}: la fixture ostile e' stata accettata in apertura"),
            Err(errore) => verifica(&format!("{nome}/apertura"), &errore, None),
        }
    }
}

/// Una destinazione inesistente e' un errore d'apertura come un altro.
#[test]
fn nessun_driver_fa_uscire_il_percorso_di_una_sorgente_assente() {
    let directory = tempfile::tempdir().expect("directory temporanea");
    for Caso {
        nome,
        driver,
        fixture: file,
        ..
    } in driver_e_fixture()
    {
        let assente = directory
            .path()
            .join(format!("{MARCATORE}-inesistente-{file}"));

        match driver.open(Source::Path(assente), opzioni_lettura()) {
            Ok(_) => panic!("{nome}: una sorgente inesistente e' stata aperta"),
            Err(errore) => verifica(&format!("{nome}/sorgente-assente"), &errore, None),
        }
    }
}

// --- lettura -----------------------------------------------------------------

/// Apertura riuscita, guasto alla riga: e' un contratto diverso.
///
/// Le tre fixture coprono i driver in cui l'inferenza dello schema puo'
/// riuscire su un header valido mentre una riga successiva e' rotta. Nei
/// formati binari il guasto e' quasi sempre in apertura, e simularne uno a
/// meta' stream richiederebbe di costruire un file valido e poi corromperlo —
/// cioe' una fixture generata, che e' proprio cio' che si e' scelto di non
/// avere.
#[test]
fn una_lettura_fallita_non_consegna_righe_ne_payload() {
    // Le opzioni sono **per caso**: senza `wkt_column` e `assume_crs` il CSV
    // non interpreta mai la geometria, e il ramo che si vuole provare non
    // verrebbe raggiunto. Una fixture che non arriva al ramo e' un test vacuo
    // travestito da test verde.
    //
    // # Che cosa questa prova raggiunge davvero, misurato
    //
    // Solo **GeoJSON** fallisce a meta' stream. Gli altri due falliscono in
    // apertura, per ragioni diverse e verificate:
    //
    // * `csv` — `infer_wkt_geometry` parsa **ogni** WKT del file durante
    //   l'inferenza, quindi una geometria rotta viene colta in apertura
    //   qualunque riga occupi. Nessuna fixture puo' spostare quel guasto alla
    //   lettura: non e' un limite della fixture, e' come il driver e' fatto;
    // * `kml` — le coordinate non valide sono rilevate durante la scansione
    //   che precede il reader.
    //
    // Restano entrambi nell'elenco: toglierli farebbe sparire il fatto. Il
    // fail-fast e' un comportamento migliore, non peggiore — ma va scritto,
    // perche' significa che la fase di lettura e' provata da **un** driver e
    // non da tre.
    let casi: Vec<(&str, Box<dyn FormatDriver>, &str, ReadOptions)> = vec![
        (
            "csv",
            Box::new(driver_csv::CsvDriver),
            "lettura.csv",
            opzioni_lettura()
                .with_format_option("wkt_column", "geometry")
                .with_assume_crs("EPSG:4326"),
        ),
        (
            "geojson",
            Box::new(driver_geojson::GeoJsonDriver),
            "lettura.geojson",
            opzioni_lettura(),
        ),
        // KML resta nell'elenco pur fallendo **in apertura**: il driver
        // rileva le coordinate rotte durante la scansione, prima di
        // consegnare un reader. E' un fail-fast legittimo, e lasciarlo qui
        // documenta che la lettura di questo formato non e' raggiungibile con
        // una fixture di questa forma — invece di far sparire il caso.
        (
            "kml",
            Box::new(driver_kml::KmlDriver),
            "lettura.kml",
            opzioni_lettura(),
        ),
    ];

    let mut letture_fallite = 0_usize;
    let mut aperture_riuscite = 0_usize;
    for (nome, driver, file, opzioni) in casi {
        let Ok(dataset) = driver.open(Source::Path(fixture(file)), opzioni) else {
            // Se il driver rifiuta gia' in apertura, la proprieta' d'apertura
            // e' gia' coperta dall'altro test e qui non c'e' niente da
            // aggiungere: si passa oltre invece di fingere una lettura.
            continue;
        };
        aperture_riuscite += 1;
        if dataset.layers().is_empty() {
            continue;
        }

        let richiesta = ReadRequest {
            layer: LayerId(0),
            projected_fields: None,
            projection_mode: ProjectionMode::BestEffort,
            pruning_predicate: None,
            spatial_pruning_hint: None,
            scope: ReadScope::Complete,
            batch_target: BatchTarget::default(),
            cancellation: plenora_io_model::CancellationToken::default(),
        };
        let Ok(mut reader) = dataset.open_layer_reader(&richiesta) else {
            continue;
        };

        let mut righe_consegnate = 0_usize;
        let mut fallita = false;
        loop {
            match reader.next_batch() {
                Ok(Some(batch)) => righe_consegnate += batch.num_rows(),
                Ok(None) => break,
                Err(errore) => {
                    verifica(&format!("{nome}/lettura"), &errore, None);
                    fallita = true;
                    break;
                }
            }
        }

        if fallita {
            letture_fallite += 1;
            // Il contratto della lettura fallita: **niente righe**. Un batch
            // consegnato prima dell'errore e' un risultato parziale che il
            // chiamante non sa di dover buttare.
            assert_eq!(
                righe_consegnate, 0,
                "{nome}: {righe_consegnate} righe consegnate prima dell'errore"
            );
        }
    }

    // Senza queste due righe il test sarebbe **vacuo**: ogni `continue` sopra
    // e' legittimo preso da solo, ma se scattassero tutti il test sarebbe
    // verde senza aver misurato niente.
    //
    // I due conteggi sono separati perche' rispondono a domande diverse:
    // «qualche apertura e' riuscita?» e «qualche lettura e' fallita dopo
    // un'apertura riuscita?». Il secondo non implica il primo per caso.
    assert!(
        aperture_riuscite > 0,
        "nessuna fixture di lettura si e' aperta: il test non e' mai arrivato          alla fase che vuole provare"
    );
    assert!(
        letture_fallite > 0,
        "nessuna delle fixture di lettura ha prodotto un errore a meta' stream:          il test non ha misurato la fase di lettura"
    );
}

// --- scrittura ---------------------------------------------------------------

/// Il nome del campo viene dal contratto e non deve uscire.
///
/// Il piano dichiara un campo con un tipo che nessun driver geografico sa
/// rappresentare, **e il nome del campo porta il marcatore**: se il rifiuto lo
/// interpolasse, il test lo vedrebbe.
fn piano_non_rappresentabile() -> WritePlan {
    use arrow_schema::{DataType, Field, Schema};
    use plenora_io_model::contract::DataContract;
    use std::sync::Arc;

    let schema = Arc::new(Schema::new(vec![Field::new(
        MARCATORE,
        DataType::Duration(arrow_schema::TimeUnit::Nanosecond),
        true,
    )]));

    WritePlan {
        layers: vec![WriteLayer {
            name: MARCATORE.to_owned(),
            contract: DataContract {
                schema,
                geometry: None,
            },
        }],
    }
}

#[test]
fn una_scrittura_rifiutata_non_lascia_destinazione_ne_payload() {
    let directory = tempfile::tempdir().expect("directory temporanea");
    let piano = piano_non_rappresentabile();

    let mut rifiuti = 0_usize;
    for Caso {
        nome,
        driver,
        estensione,
        ..
    } in driver_e_fixture()
    {
        let destinazione = directory.path().join(format!("uscita-{nome}.{estensione}"));

        match driver.create(
            Sink::Path(destinazione.clone()),
            &piano,
            &opzioni_scrittura(),
        ) {
            Ok(_) => {
                // Alcuni driver accettano il piano e rifiutano alla scrittura:
                // il rifiuto e' comunque coperto, ma qui non c'e' errore da
                // ispezionare. Cio' che si verifica e' l'altra meta' del
                // contratto — vedi il test sulla destinazione.
            }
            Err(errore) => {
                rifiuti += 1;
                verifica(&format!("{nome}/scrittura"), &errore, None);
                // Il contratto della scrittura rifiutata: **niente
                // destinazione**. Un file mezzo scritto e' peggio di nessun
                // file, perche' somiglia a un risultato.
                assert!(
                    !destinazione.exists(),
                    "{nome}: la destinazione e' rimasta dopo un rifiuto: {}",
                    destinazione.display()
                );
            }
        }
    }

    // Se nessun driver avesse rifiutato il piano, il ramo che conta —
    // busta redatta e destinazione assente — non sarebbe mai stato eseguito.
    assert!(
        rifiuti > 0,
        "nessun driver ha rifiutato il piano ostile: il test non ha misurato          la fase di scrittura"
    );
}

/// Una destinazione che esiste gia' e' un conflitto, non un guasto.
///
/// Il quartetto e' fissato **per intero**, `remote_effect` compreso: e' un
/// caso in cui il contratto e' lo stesso per tutti i driver, quindi si puo'
/// pretendere l'uguaglianza invece della sola assenza di payload.
#[test]
fn una_destinazione_esistente_non_viene_sovrascritta_ne_fa_uscire_il_percorso() {
    let directory = tempfile::tempdir().expect("directory temporanea");
    let assi = Assi {
        category: ErrorCategory::Conflict,
        phase: ErrorPhase::Commit,
        remote_effect: RemoteEffect::None,
        retry: RetryDisposition::Never,
    };

    for Caso {
        nome,
        driver,
        estensione,
        ..
    } in driver_e_fixture()
    {
        let destinazione = directory
            .path()
            .join(format!("{MARCATORE}-esistente-{nome}.{estensione}"));
        let contenuto = format!("contenuto preesistente {MARCATORE}");
        std::fs::write(&destinazione, &contenuto).expect("scrittura della destinazione");

        let piano = piano_non_rappresentabile();
        if let Err(errore) = driver.create(
            Sink::Path(destinazione.clone()),
            &piano,
            &opzioni_scrittura(),
        ) {
            verifica(&format!("{nome}/destinazione-esistente"), &errore, None);
            if errore.code == plenora_io_model::IoErrorCode::OutputExists {
                assert_eq!(errore.category, assi.category, "{nome}: category");
                assert_eq!(errore.phase, assi.phase, "{nome}: phase");
                assert_eq!(
                    errore.remote_effect, assi.remote_effect,
                    "{nome}: remote_effect"
                );
                assert_eq!(errore.retry, assi.retry, "{nome}: retry");
            }
        }

        // In ogni caso il file preesistente non e' stato toccato.
        // Non un ripiego a stringa vuota: se il file sparisse, quel ripiego
        // trasformerebbe «la destinazione e' stata cancellata» in «il
        // contenuto e' cambiato», cioe' in una diagnosi sbagliata.
        let dopo = std::fs::read_to_string(&destinazione)
            .expect("la destinazione preesistente deve essere ancora leggibile");
        assert_eq!(
            dopo, contenuto,
            "{nome}: la destinazione preesistente e' stata modificata"
        );
    }
}

// --- FileGDB: due prove separate ---------------------------------------------

/// Senza `gdal-backend` il driver e' uno **stub tipizzato**, non un errore
/// d'ambiente.
///
/// La distinzione conta: uno stub che dicesse «GDAL non trovato» farebbe
/// cercare a chi legge una libreria da installare, quando la verita' e' che
/// questo binario non e' stato costruito per parlare con GDAL.
#[test]
#[cfg(not(feature = "gdal-backend"))]
fn filegdb_senza_feature_e_uno_stub_tipizzato() {
    let driver = driver_filegdb::FileGdbDriver;
    let esito = driver.open(Source::Path(fixture("apertura.gdb")), opzioni_lettura());

    let Err(errore) = esito else {
        panic!("senza `gdal-backend` l'apertura non puo' riuscire");
    };
    verifica("filegdb/stub", &errore, None);
    assert_eq!(
        errore.category,
        ErrorCategory::Unsupported,
        "lo stub e' una capability mancante, non un guasto d'ambiente"
    );
}

/// Con `gdal-backend` il driver parla davvero con GDAL, e il testo di GDAL
/// **non deve** arrivare nella busta.
#[test]
#[cfg(feature = "gdal-backend")]
fn filegdb_con_gdal_non_fa_uscire_il_testo_della_dipendenza() {
    let driver = driver_filegdb::FileGdbDriver;
    let esito = driver.open(Source::Path(fixture("apertura.gdb")), opzioni_lettura());

    let Err(errore) = esito else {
        panic!("una directory che non e' un FileGDB non puo' essere aperta");
    };
    verifica("filegdb/gdal", &errore, None);

    // La distinzione che conta, e che non e' ovvia: **nominare** GDAL in un
    // messaggio curato non e' far uscire testo di GDAL. «apertura GDAL
    // fallita» e' un letterale scritto da noi, e dice al lettore quale via e'
    // stata presa — informazione sul nostro build, non sul payload. Cio' che
    // non deve uscire e' il testo che GDAL **produce**: codici CPL, messaggi
    // di errore OGR, diagnostica del driver sottostante.
    //
    // E' la stessa lezione del nome «Parquet»: un elenco che vieta il nome
    // della dipendenza rossa su un messaggio corretto.
    for frammento in [
        "CPLE_",
        "ERROR 1:",
        "ERROR 4:",
        "not recognized as",
        "Cannot open datasource",
        "OGRErr",
        "GDALOpen",
    ] {
        assert!(
            !errore.message.contains(frammento),
            "testo prodotto da GDAL (`{frammento}`) nel messaggio: {}",
            errore.message
        );
    }
}

// --- la busta v1, dal binario vero -------------------------------------------

/// Le sei chiavi, misurate sul **processo**, non su una funzione.
///
/// E' l'unica prova che attraversa davvero tutto: argomenti, instradamento,
/// driver, mappatura, serializzazione, stderr. Una busta corretta in un test
/// di modulo e sbagliata nel binario e' un caso che nessun altro test qui
/// coglierebbe.
#[test]
fn la_busta_del_binario_ha_le_sei_chiavi_e_non_porta_il_payload() {
    let uscita = std::process::Command::new(env!("CARGO_BIN_EXE_plenora-io"))
        .arg("inspect")
        .arg(fixture("apertura.geojson"))
        .output()
        .expect("il binario si esegue");

    assert!(
        !uscita.status.success(),
        "la fixture ostile e' stata accettata"
    );
    assert!(
        uscita.stdout.is_empty(),
        "un errore non deve produrre uscita su stdout: {}",
        String::from_utf8_lossy(&uscita.stdout)
    );

    let testo = String::from_utf8(uscita.stderr).expect("stderr e' UTF-8");
    assert!(
        !testo.contains(MARCATORE),
        "il payload e' uscito dal binario: {testo}"
    );

    let documento: serde_json::Value =
        serde_json::from_str(testo.trim()).expect("stderr e' un documento JSON");
    assert_eq!(documento["status"], "error");
    assert_eq!(documento["protocol_version"], 1);
    assert_eq!(documento["contract"], "plenora-io-error-v1");

    let chiavi: std::collections::BTreeSet<&str> = documento["error"]
        .as_object()
        .expect("l'errore e' un oggetto")
        .keys()
        .map(String::as_str)
        .collect();
    let attese: std::collections::BTreeSet<&str> = [
        "category",
        "phase",
        "remote_effect",
        "retry",
        "code",
        "message",
    ]
    .into_iter()
    .collect();
    assert_eq!(
        chiavi, attese,
        "plenora-io-error-v1 ha cambiato forma nel binario: {chiavi:?}"
    );

    assert_eq!(
        documento["error"]["remote_effect"], "none",
        "un guasto locale non ha effetto remoto"
    );
}

// --- l'harness non e' cieco --------------------------------------------------

/// `verifica` deve **fallire** su una busta che porta il marcatore.
///
/// Senza questa prova l'intera batteria potrebbe essere verde perche' il
/// controllo non guarda dove crede. E' la stessa lezione dei checkpoint: un
/// verde che non ha misurato niente e' indistinguibile da un verde che ha
/// misurato tutto, finche' non si prova a farlo diventare rosso.
///
/// Il messaggio si costruisce con `PublicMessage::Curated(MARCATORE)`: il
/// marcatore e' un `&'static str`, quindi la via redatta lo accetta senza
/// bisogno di alcuna scorciatoia — nessun costruttore esiste qui che non
/// esista anche in produzione.
#[test]
fn la_verifica_diventa_rossa_se_il_marcatore_e_nel_messaggio() {
    use plenora_io_model::PublicMessage;

    let avvelenato = PlenoraIoError::schema_redatto(&PublicMessage::Curated(MARCATORE));
    assert!(
        avvelenato.message.contains(MARCATORE),
        "la premessa: il messaggio deve contenere il marcatore"
    );

    let esito = std::panic::catch_unwind(|| {
        verifica("sonda/marcatore", &avvelenato, None);
    });
    assert!(
        esito.is_err(),
        "`verifica` ha lasciato passare un messaggio con il marcatore"
    );
}

/// E deve fallire anche su testo di dipendenza.
#[test]
fn la_verifica_diventa_rossa_su_testo_di_dipendenza() {
    use plenora_io_model::PublicMessage;

    let avvelenato =
        PlenoraIoError::schema_redatto(&PublicMessage::Curated("SqliteFailure nel messaggio"));
    let esito = std::panic::catch_unwind(|| {
        verifica("sonda/dipendenza", &avvelenato, None);
    });
    assert!(
        esito.is_err(),
        "`verifica` ha lasciato passare testo di dipendenza"
    );
}

/// E su un `remote_effect` che non e' `None`.
///
/// E' il quinto asse, quello che i test precedenti a S9 non guardavano: un
/// guasto locale che dichiarasse `Unknown` farebbe ritentare al chiamante
/// un'operazione che non e' mai partita.
#[test]
fn la_verifica_diventa_rossa_su_un_effetto_remoto_dichiarato() {
    use plenora_io_model::{IoErrorCode, PublicMessage};

    let remoto = PlenoraIoError::redatto(
        IoErrorCode::Generic,
        ErrorCategory::Timeout,
        ErrorPhase::Commit,
        RemoteEffect::Unknown,
        RetryDisposition::RequiresRecovery,
        &PublicMessage::Curated("esito non verificabile"),
    );
    let esito = std::panic::catch_unwind(|| {
        verifica("sonda/effetto-remoto", &remoto, None);
    });
    assert!(
        esito.is_err(),
        "`verifica` ha lasciato passare un effetto remoto su un driver locale"
    );
}
