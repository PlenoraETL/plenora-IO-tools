//! Le prove di [`super`], in un file loro.
//!
//! Restano un **modulo figlio**: vedono i privati del genitore
//! esattamente come quando stavano dentro di lui, e non allargano di
//! una riga la superficie pubblica del crate.

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

/// Il testo sta nel tetto, il WKB codificato no.
///
/// I driver tabellari controllano la **rappresentazione d'ingresso** — qui
/// il testo WKT — prima di costruire l'AST. Ma la codifica WKB puo' essere
/// piu' grande di quel testo: `POINT (1 2)` occupa 11 caratteri e 21 byte
/// in WKB, perche' due `f64` costano 16 byte da soli.
///
/// Fino a S5.1 il buffer cresceva comunque fino a 21 byte e il rifiuto
/// arrivava dall'adapter, a memoria gia' allocata. Ora l'encoder e'
/// bounded e si ferma al tetto.
#[test]
fn il_wkb_codificato_non_supera_il_tetto_anche_se_il_testo_ci_sta() {
    // Fra la lunghezza del testo (11) e quella del WKB (21).
    const SOGLIA: usize = 15;

    let dir = tempfile::tempdir().expect("tempdir");
    let percorso = csv_con_wkt(&dir, "POINT (1 2)");
    assert!(
        "POINT (1 2)".len() <= SOGLIA,
        "la premessa: il testo deve stare nel tetto"
    );

    // L'inferenza passa — il testo ci sta — e il rifiuto arriva dalla
    // codifica, non dal controllo sul testo.
    let esito = CsvDriver.open(Source::Path(percorso), opzioni_con_cella(SOGLIA, 1_000));
    let Ok(dataset) = esito else {
        unreachable!("l'inferenza deve passare: il testo sta nel tetto");
    };
    let mut reader = dataset
        .open_layer_reader(&req(1_000))
        .expect("il reader si apre");
    let esito = reader.next_batch();
    assert!(
        matches!(
            esito,
            Err(ref errore) if errore.message.contains("oltre il limite")
        ),
        "la codifica WKB deve fermarsi al tetto: {esito:?}"
    );
}

/// `infer_types` applica il proprio tetto di righe, da solo.
///
/// I due punti di enforcement dell'inferenza si coprono a vicenda quando
/// li si esercita da `open`, perche' `infer_types` gira per primo: una
/// mutazione su uno solo sopravvive. Chiamarli direttamente — sono privati,
/// ma il modulo di test li raggiunge — verifica ciascuno in isolamento.
#[test]
fn infer_types_si_ferma_al_tetto_di_righe() {
    let dir = tempfile::tempdir().expect("tempdir");
    let percorso = csv_con_righe(&dir, 8);
    let headers = vec!["id".to_owned(), "geometry".to_owned()];
    let geom_cols = HashSet::from([1_usize]);

    assert!(infer_types(&percorso, b',', &headers, &geom_cols, quote(8)).is_ok());

    let errore = infer_types(&percorso, b',', &headers, &geom_cols, quote(7))
        .expect_err("otto righe con tetto sette devono fermare la passata");
    assert_eq!(errore.code, plenora_io_model::IoErrorCode::LimitExceeded);
}

/// `infer_wkt_geometry` applica il proprio tetto di righe, da solo.
#[test]
fn infer_wkt_geometry_si_ferma_al_tetto_di_righe() {
    let dir = tempfile::tempdir().expect("tempdir");
    let percorso = csv_con_righe(&dir, 8);

    assert!(infer_wkt_geometry(&percorso, b',', 1, quote(8)).is_ok());

    let errore = infer_wkt_geometry(&percorso, b',', 1, quote(7))
        .expect_err("otto righe con tetto sette devono fermare la passata");
    assert_eq!(errore.code, plenora_io_model::IoErrorCode::LimitExceeded);
}

/// `infer_wkt_geometry` applica il tetto per cella, da solo.
#[test]
fn infer_wkt_geometry_applica_il_tetto_per_cella() {
    let dir = tempfile::tempdir().expect("tempdir");
    let percorso = csv_con_wkt(&dir, "POINT (1 2)");

    assert!(infer_wkt_geometry(&percorso, b',', 1, quote_con_cella(con_byte(11), 100)).is_ok());

    let errore = infer_wkt_geometry(&percorso, b',', 1, quote_con_cella(con_byte(10), 100))
        .expect_err("undici caratteri con tetto dieci devono fallire");
    assert_eq!(errore.code, plenora_io_model::IoErrorCode::LimitExceeded);
}

/// `append_geometry` applica il tetto per cella sul percorso di lettura.
///
/// E' il gemello di `infer_wkt_geometry`: la stessa quota, applicata nella
/// seconda passata. Esercitarlo da `open` non lo isolerebbe — l'inferenza
/// rifiuta per prima — quindi il test lo chiama direttamente.
#[test]
fn append_geometry_applica_il_tetto_per_cella() {
    let record = csv::StringRecord::from(vec!["1", "POINT (1 2)"]);
    let mut buffer = Vec::new();

    let mut builder = BinaryBuilder::new();
    assert!(
        append_geometry(
            &mut builder,
            GeomSpec::Wkt(1),
            &record,
            &mut buffer,
            con_byte(64)
        )
        .is_ok(),
        "con un tetto capiente la cella passa"
    );

    let mut builder = BinaryBuilder::new();
    let errore = append_geometry(
        &mut builder,
        GeomSpec::Wkt(1),
        &record,
        &mut buffer,
        con_byte(10),
    )
    .expect_err("undici caratteri con tetto dieci devono fallire");
    assert_eq!(errore.code, plenora_io_model::IoErrorCode::LimitExceeded);
}

fn quote(righe: usize) -> QuoteInferenza {
    quote_con_cella(con_byte(4_096), righe)
}

/// I tetti del bordo con il solo cap in byte stretto: le sonde qui
/// provano quello, e gli altri due hanno le loro in `wkt_progressivo`.
fn con_byte(cella: usize) -> WkbLimits {
    WkbLimits {
        max_cell_bytes: cella,
        ..WkbLimits::default()
    }
}

fn quote_con_cella(cella_wkt: WkbLimits, righe: usize) -> QuoteInferenza {
    QuoteInferenza { cella_wkt, righe }
}

fn csv_con_righe(dir: &tempfile::TempDir, quante: u32) -> std::path::PathBuf {
    let percorso = dir.path().join("molte.csv");
    let righe: Vec<String> = (0..quante)
        .map(|indice| format!("{indice},\"POINT (1 2)\""))
        .collect();
    let contenuto = format!("id,geometry\n{}\n", righe.join("\n"));
    std::fs::write(&percorso, contenuto).expect("scrittura");
    percorso
}

/// Opzioni con quote WKB strette, per i test di S5.
fn opzioni_con_cella(byte: usize, righe: u64) -> ReadOptions {
    let limiti = plenora_io_model::budget::PipelineLimits::default()
        .with_max_wkb_cell_bytes(byte)
        .with_max_rows(righe);
    match plenora_io_model::budget::PipelineBudget::builder()
        .limits(limiti)
        .build()
    {
        Ok(bundle) => ReadOptions::from_read_parts(bundle.into_read_parts())
            .with_assume_crs("EPSG:4326")
            .with_format_option("wkt_column", "geometry"),
        Err(error) => unreachable!("limiti di test non validi: {error:?}"),
    }
}

/// Opzioni con il tetto sui **componenti** configurato, e nient'altro.
///
/// Il cap in byte resta quello del contratto: e' cio' che rende la prova
/// una prova di S12 invece di una di S5.
fn opzioni_con_componenti(componenti: usize) -> ReadOptions {
    let limiti =
        plenora_io_model::budget::PipelineLimits::default().with_max_wkb_components(componenti);
    match plenora_io_model::budget::PipelineBudget::builder()
        .limits(limiti)
        .build()
    {
        Ok(bundle) => ReadOptions::from_read_parts(bundle.into_read_parts())
            .with_assume_crs("EPSG:4326")
            .with_format_option("wkt_column", "geometry"),
        Err(error) => unreachable!("limiti di test non validi: {error:?}"),
    }
}

fn csv_con_wkt(dir: &tempfile::TempDir, wkt: &str) -> std::path::PathBuf {
    let percorso = dir.path().join("input.csv");
    std::fs::write(&percorso, format!("id,geometry\n1,\"{wkt}\"\n")).expect("scrittura");
    percorso
}

/// La capability `hostile_input_hardened`, provata dove S12 la sposta.
///
/// Non e' il cap in byte: quello esisteva prima del parser progressivo e
/// scatta **prima** di deserializzare, quindi un test che lo esercita
/// resterebbe verde anche rimettendo il parser vecchio. Prova nulla di
/// questo lotto.
///
/// Qui l'input sta comodamente sotto il cap in byte, e a fermarlo e' il
/// tetto sui **componenti** -- l'unita' che solo un'analisi che addebita
/// mentre consuma puo' applicare. Le tre condizioni stanno insieme
/// apposta:
///
///   * con il tetto stretto il rifiuto e' esattamente `LimitExceeded`;
///   * con il default lo stesso identico input passa, quindi il rifiuto
///     viene dal tetto e non dall'input;
///   * l'input e' molto piu' corto del cap in byte, che percio' non
///     c'entra.
///
/// E' la prova che `check_capability_input_ostile.py` esegue per questo
/// driver: cancellarla, rinominarla o indebolirla rende rossa la
/// capability nel catalogo.
#[test]
fn la_cella_wkt_e_rifiutata_per_componenti_sotto_il_cap_in_byte() {
    // Cinque coordinate, cioe' cinque componenti nell'unita' del bordo.
    const COMPONENTI: usize = 5;
    let wkt = "LINESTRING (0 0,1 1,2 2,3 3,4 4)";
    let dir = tempfile::tempdir().expect("tempdir");
    let percorso = csv_con_wkt(&dir, wkt);

    // Il cap in byte non c'entra: la cella e' minuscola accanto al default.
    assert!(
        wkt.len() < plenora_io_model::limits::WkbLimits::default().max_cell_bytes / 1_000,
        "l'input deve stare comodamente sotto il cap in byte"
    );

    // Con il tetto sui componenti al valore esatto, passa.
    assert!(
        CsvDriver
            .open(
                Source::Path(percorso.clone()),
                opzioni_con_componenti(COMPONENTI)
            )
            .is_ok(),
        "con {COMPONENTI} componenti di tetto la stessa cella deve passare"
    );

    // Con un componente in meno, e' rifiutata -- e per quota, non per
    // sintassi: il codice e' preteso esatto, senza alternative sul testo.
    let esito = CsvDriver.open(
        Source::Path(percorso),
        opzioni_con_componenti(COMPONENTI - 1),
    );
    match esito {
        Err(errore) => assert_eq!(
            errore.code,
            plenora_io_model::IoErrorCode::LimitExceeded,
            "il rifiuto deve venire dal tetto sui componenti: {}",
            errore.message
        ),
        Ok(_) => panic!("una cella oltre il tetto sui componenti deve fallire"),
    }
}

/// L'inferenza usa il tetto **configurato**, non il default del contratto.
///
/// Fino a S5 `infer_wkt_geometry` passava `WkbLimits::default()
/// .max_cell_bytes` — 64 MiB — quindi `--max-wkb-cell-bytes` non arrivava
/// alla passata di inferenza: una cella oltre la soglia richiesta veniva
/// parsata comunque, e l'AST wkt allocato. Il rifiuto arrivava piu' tardi
/// o non arrivava affatto.
///
/// Il test copre **entrambi** i lati della soglia con lo stesso tetto: se
/// coprisse solo il rifiuto, un tetto messo per sbaglio a zero lo
/// soddisferebbe.
#[test]
fn inference_uses_configured_wkt_cell_bytes_not_default() {
    let dir = tempfile::tempdir().expect("tempdir");
    let wkt = "POINT (1 2)";
    let percorso = csv_con_wkt(&dir, wkt);
    let soglia = wkt.len();

    // Sotto soglia: la cella ci sta, e l'apertura riesce.
    let sotto = CsvDriver
        .open(
            Source::Path(percorso.clone()),
            opzioni_con_cella(soglia, 1_000),
        )
        .expect("una cella dentro il tetto configurato deve passare");
    assert_eq!(sotto.layers().len(), 1);

    // Sopra soglia: un byte in meno di tetto e la stessa cella e'
    // rifiutata, prima di costruire l'AST.
    // `Box<dyn OpenDatasetHandle>` non implementa `Debug`, quindi niente
    // `expect_err`: si guarda direttamente il ramo di errore.
    let esito = CsvDriver.open(Source::Path(percorso), opzioni_con_cella(soglia - 1, 1_000));
    assert!(
        matches!(
            esito,
            Err(ref errore) if errore.code == plenora_io_model::IoErrorCode::LimitExceeded
        ),
        "una cella oltre il tetto configurato deve fallire per quota"
    );

    // E il default non salva: con 64 MiB di tetto la stessa cella
    // passerebbe, quindi il fallimento sopra viene davvero dal flag.
    assert!(
        soglia - 1 < plenora_io_model::limits::WkbLimits::default().max_cell_bytes,
        "la soglia del test deve stare sotto il default, o non distinguerebbe nulla"
    );
}

/// Il percorso completo — inferenza **e** lettura — gira sotto un tetto
/// non predefinito.
///
/// # Cosa questo test puo' e non puo' dimostrare
///
/// Le due passate parsano le stesse celle con la stessa quota, presa
/// dalle stesse opzioni: dall'API pubblica non c'e' modo di stringere la
/// seconda senza stringere la prima. Una cella oltre soglia viene percio'
/// sempre rifiutata dall'inferenza, e la lettura non la vede mai.
///
/// Di conseguenza una mutazione che riportasse **solo** la lettura al
/// default sopravviverebbe: non e' copertura mancante, e' ridondanza fra
/// due controlli sullo stesso dato. Cio' che il test dimostra e' che il
/// percorso completo funziona con un tetto configurato — se la lettura
/// usasse un valore incoerente con l'inferenza, un file accettato
/// all'apertura fallirebbe a meta' drenaggio, ed e' quello il difetto che
/// qui si esclude.
#[test]
fn il_percorso_completo_gira_sotto_un_tetto_non_predefinito() {
    // Il tetto governa **due** grandezze sullo stesso percorso: i byte
    // del testo WKT in inferenza e quelli del WKB codificato nella
    // validazione del batch. Un punto occupa 11 byte in testo e 21 in
    // WKB, quindi una soglia tarata sul solo testo farebbe fallire la
    // lettura per la ragione sbagliata — ed e' cosi' che questo test ha
    // fallito in prima stesura.
    const SOGLIA: usize = 64;

    let dir = tempfile::tempdir().expect("tempdir");
    let percorso = csv_con_wkt(&dir, "POINT (1 2)");
    assert!(
        SOGLIA < plenora_io_model::limits::WkbLimits::default().max_cell_bytes,
        "il tetto del test deve essere piu' stretto del default"
    );

    let dataset = CsvDriver
        .open(Source::Path(percorso), opzioni_con_cella(SOGLIA, 1_000))
        .expect("l'apertura deve riuscire: la cella sta nel tetto");
    let mut reader = dataset
        .open_layer_reader(&req(1_000))
        .expect("il reader si apre");
    let batch = reader
        .next_batch()
        .expect("la lettura non deve fallire a meta' drenaggio")
        .expect("un batch");
    assert_eq!(batch.num_rows(), 1);
}

/// Le passate di inferenza sono bounded sulle righe visitate.
///
/// # Perche' `max_rows` e non `max_input_entries`
///
/// `max_input_entries` governa l'enumerazione della **sorgente**, e il
/// preflight l'ha gia' applicata al file: riapplicarla ai record sarebbe
/// la stessa quota contata due volte. Il suo valore predefinito e'
/// calibrato sui file di una directory, e ai record rifiuterebbe un CSV di
/// dimensioni ordinarie — il benchmark del repository ne genera 200.000.
///
/// Che il tetto sulle entry sia applicato **prima** dell'inferenza lo
/// verifica il preflight in `plenora-io-core`
/// (`directory_scan_over_max_input_entries_rejects_with_typed_error`): un
/// driver non raggiunge l'inferenza se la sorgente ha gia' sforato.
#[test]
fn inference_respects_max_rows_before_materialising() {
    let dir = tempfile::tempdir().expect("tempdir");
    let percorso = dir.path().join("molte.csv");
    let righe: Vec<String> = (0..8_u32)
        .map(|indice| format!("{indice},\"POINT (1 2)\""))
        .collect();
    let contenuto = format!("id,geometry\n{}\n", righe.join("\n"));
    std::fs::write(&percorso, contenuto).expect("scrittura");

    // Otto righe con tetto otto: passa.
    assert!(CsvDriver
        .open(Source::Path(percorso.clone()), opzioni_con_cella(4_096, 8))
        .is_ok());

    // Le stesse otto con tetto sette: l'inferenza si ferma.
    let esito = CsvDriver.open(Source::Path(percorso), opzioni_con_cella(4_096, 7));
    let Err(errore) = esito else {
        unreachable!("l'inferenza deve fermarsi al tetto di righe");
    };
    assert_eq!(errore.code, plenora_io_model::IoErrorCode::LimitExceeded);
    assert!(
        errore.message.contains("inferenza"),
        "il messaggio deve dire dove ci si e' fermati: {}",
        errore.message
    );
}

fn opzioni_lettura() -> ReadOptions {
    match plenora_io_model::budget::PipelineBudget::builder().build() {
        Ok(bundle) => ReadOptions::from_read_parts(bundle.into_read_parts()),
        Err(error) => unreachable!("bundle di test non costruibile: {error:?}"),
    }
}

use plenora_io_core::request::{BatchTarget, ProjectionMode, ReadScope};
use plenora_io_core::WriteLayer;
use plenora_io_model::CancellationToken;
use std::collections::BTreeMap;

fn opts(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

fn read_opts(pairs: &[(&str, &str)]) -> ReadOptions {
    opzioni_lettura()
        .with_assume_crs("EPSG:4326")
        .with_format_options(opts(pairs))
}

fn req(max_rows: usize) -> ReadRequest {
    ReadRequest {
        layer: LayerId(0),
        projected_fields: None,
        projection_mode: ProjectionMode::BestEffort,
        pruning_predicate: None,
        spatial_pruning_hint: None,
        scope: ReadScope::default(),
        batch_target: BatchTarget {
            target_bytes: 8 * 1024 * 1024,
            max_rows,
        },
        cancellation: CancellationToken::default(),
    }
}

/// Un CSV di soli attributi e' un CSV, e il writer lo scrive.
///
/// # Il difetto che chiude
///
/// `DataContract::geometry` e' un `Option`, e il writer pretendeva
/// comunque una colonna geometrica: rifiutava con «nessuna colonna
/// geometria» un piano che il contratto ammette. Il costo si e' visto
/// quando il `FileGDB` ha smesso di rifiutare le tabelle non spaziali --
/// quelle che stanno accanto alle feature class in ogni GDB reale --
/// perche' il formato piu' adatto a riceverle era proprio quello che non
/// sapeva scriverle.
///
/// Il descrittore lo diceva gia': `csv` dichiara `CrsWriteSupport::None`,
/// cioe' che un CRS non lo porta. Un CRS si porta su una geometria, e un
/// formato che non ne ha uno non puo' pretenderla.
///
/// # La controprova
///
/// Nella stessa sonda, perche' e' il confronto a renderla leggibile: con
/// una geometria l'intestazione la porta e i valori pure. Senza,
/// «l'intestazione non ha `geometry`» sarebbe vero anche di un writer che
/// ha smesso di scrivere le geometrie.
/// Un punto XY in WKB, per la controprova con geometria.
fn punto_wkb() -> Vec<u8> {
    let mut byte = Vec::new();
    plenora_io_model::wkb::encode_wkb_into(
        &WkbGeometry {
            value: WkbValue::Point(WkbCoordinate {
                x: 1.0,
                y: 2.0,
                z: None,
                m: None,
            }),
            dimensions: plenora_io_model::contract::CoordinateDimensions::Xy,
            srid: None,
        },
        WkbFlavor::Iso,
        &mut byte,
    )
    .expect("un punto si codifica");
    byte
}

#[test]
fn n1_una_tabella_senza_geometria_diventa_un_csv_di_soli_attributi() {
    let dir = tempfile::tempdir().unwrap();
    let driver = CsvDriver;

    let schema_tabella: SchemaRef = Arc::new(Schema::new(vec![
        Field::new("nota", arrow_schema::DataType::Utf8, true),
        Field::new("peso", arrow_schema::DataType::Int64, true),
    ]));
    let lotto = RecordBatch::try_new(
        Arc::clone(&schema_tabella),
        vec![
            Arc::new(StringArray::from(vec!["primo", "secondo"])),
            Arc::new(arrow_array::Int64Array::from(vec![10_i64, 20])),
        ],
    )
    .unwrap();
    let piano = WritePlan {
        layers: vec![WriteLayer {
            name: "tabella".to_owned(),
            contract: DataContract {
                schema: schema_tabella,
                geometry: None,
            },
        }],
    };
    let uscita = dir.path().join("tabella.csv");
    let mut writer = driver
        .create(Sink::Path(uscita.clone()), &piano, &opzioni_scrittura())
        .expect("un piano senza geometria e' un piano valido per il CSV");
    writer.write(&lotto).expect("la tabella si scrive");
    writer.finish().expect("il CSV si pubblica");

    let testo = std::fs::read_to_string(&uscita).unwrap();
    assert_eq!(
        testo, "nota,peso\nprimo,10\nsecondo,20\n",
        "gli attributi, tutti, e nessuna colonna geometrica inventata"
    );

    // La controprova: con una geometria, l'intestazione la porta.
    let schema_spaziale: SchemaRef = Arc::new(Schema::new(vec![
        geometry_field("geometry", "EPSG:4326"),
        Field::new("nota", arrow_schema::DataType::Utf8, true),
    ]));
    let lotto = RecordBatch::try_new(
        Arc::clone(&schema_spaziale),
        vec![
            Arc::new(arrow_array::BinaryArray::from(vec![Some(
                punto_wkb().as_slice(),
            )])),
            Arc::new(StringArray::from(vec!["primo"])),
        ],
    )
    .unwrap();
    let piano = WritePlan {
        layers: vec![WriteLayer {
            name: "spaziale".to_owned(),
            contract: DataContract {
                schema: schema_spaziale,
                geometry: None,
            },
        }],
    };
    let uscita = dir.path().join("spaziale.csv");
    let mut writer = driver
        .create(Sink::Path(uscita.clone()), &piano, &opzioni_scrittura())
        .expect("il piano spaziale resta valido");
    writer.write(&lotto).expect("la riga si scrive");
    writer.finish().expect("il CSV si pubblica");

    let testo = std::fs::read_to_string(&uscita).unwrap();
    assert!(
        testo.starts_with("nota,geometry\n"),
        "con una geometria l'intestazione la porta, arrivata «{testo}»"
    );
    assert!(testo.contains("POINT"), "e il valore ci arriva: «{testo}»");
}

#[test]
fn round_trip_csv_xy() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("in.csv");
    std::fs::write(&src, "nome,lon,lat,pop\nA,12.5,45.9,100\nB,9.1,45.4,200\n").unwrap();

    let driver = CsvDriver;
    let ds = driver
        .open(
            Source::Path(src),
            read_opts(&[("x_column", "lon"), ("y_column", "lat")]),
        )
        .unwrap();
    let geom = ds.layers()[0].contract.geometry.as_ref().unwrap();
    assert_eq!(geom.crs.id(), Some("EPSG:4326"));
    let mut reader = ds.open_layer_reader(&req(65_536)).unwrap();
    let batch = reader.next_batch().unwrap().unwrap();
    assert_eq!(batch.num_rows(), 2);
    assert!(is_geometry_field(
        &batch.schema().field_with_name("geometry").unwrap().clone()
    ));
    let contract = ds.layers()[0].contract.clone();

    // scrivi come WKT e rileggi
    let out = dir.path().join("out.csv");
    let plan = WritePlan {
        layers: vec![WriteLayer {
            name: "l".to_owned(),
            contract,
        }],
    };
    let mut w = driver
        .create(Sink::Path(out.clone()), &plan, &opzioni_scrittura())
        .unwrap();
    w.write(&batch).unwrap();
    w.finish().unwrap();
    let text = std::fs::read_to_string(&out).unwrap();
    assert!(text.contains("POINT"));
    assert!(text.contains("nome"));
}

#[test]
fn integer_outside_i64_is_preserved_as_text() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("wide-integer.csv");
    std::fs::write(&source, "identifier,x,y\n18446744073709551615,12.5,45.9\n").unwrap();

    let dataset = CsvDriver
        .open(
            Source::Path(source),
            read_opts(&[("x_column", "x"), ("y_column", "y")]),
        )
        .unwrap();
    let mut reader = dataset.open_layer_reader(&req(65_536)).unwrap();
    let batch = reader.next_batch().unwrap().unwrap();
    let identifier = batch
        .column(batch.schema().index_of("identifier").unwrap())
        .as_any()
        .downcast_ref::<StringArray>()
        .unwrap();

    assert_eq!(identifier.value(0), "18446744073709551615");
}

#[test]
fn target_bytes_splits_streaming_batches() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("many.csv");
    let mut s = String::from("id,geom\n");
    for i in 0..10 {
        writeln!(s, "{i},\"POINT ({i} {i})\"").unwrap();
    }
    std::fs::write(&src, s).unwrap();

    let driver = CsvDriver;
    let ds = driver
        .open(Source::Path(src), read_opts(&[("wkt_column", "geom")]))
        .unwrap();
    let mut request = req(100);
    request.batch_target.target_bytes = 1;
    let mut reader = ds.open_layer_reader(&request).unwrap();
    let (mut total, mut batches) = (0, 0);
    while let Some(b) = reader.next_batch().unwrap() {
        total += b.num_rows();
        batches += 1;
    }
    assert_eq!(total, 10);
    assert_eq!(batches, 10, "target byte non applicato: {batches} batch");
}

#[test]
fn wkt_xyzm_round_trip_preserves_payload_and_contract() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("xyzm.csv");
    std::fs::write(
        &source,
        "id,geom\n1,\"MULTIPOLYGON ZM (((0 0 1 10,0 2 2 11,2 0 3 12,0 0 1 10)))\"\n",
    )
    .unwrap();

    let driver = CsvDriver;
    let dataset = driver
        .open(Source::Path(source), read_opts(&[("wkt_column", "geom")]))
        .unwrap();
    let contract = dataset.layers()[0].contract.clone();
    let geometry_contract = contract.geometry.as_ref().unwrap();
    assert_eq!(geometry_contract.dimensions, CoordinateDimensions::Xyzm);
    assert_eq!(
        geometry_contract.geometry_types,
        vec![GeometryType::MultiPolygon]
    );
    let mut reader = dataset.open_layer_reader(&req(65_536)).unwrap();
    let batch = reader.next_batch().unwrap().unwrap();
    let input_geometry = batch
        .column(0)
        .as_any()
        .downcast_ref::<BinaryArray>()
        .unwrap();
    let expected = decode_wkb(input_geometry.value(0), &WkbLimits::default()).unwrap();

    let output = dir.path().join("xyzm-out.csv");
    let plan = WritePlan {
        layers: vec![WriteLayer {
            name: "layer".to_owned(),
            contract,
        }],
    };
    let mut writer = driver
        .create(Sink::Path(output.clone()), &plan, &opzioni_scrittura())
        .unwrap();
    writer.write(&batch).unwrap();
    writer.finish().unwrap();

    let reopened = driver
        .open(
            Source::Path(output),
            read_opts(&[("wkt_column", "geometry")]),
        )
        .unwrap();
    assert_eq!(
        reopened.layers()[0]
            .contract
            .geometry
            .as_ref()
            .unwrap()
            .dimensions,
        CoordinateDimensions::Xyzm
    );
    let mut reader = reopened.open_layer_reader(&req(65_536)).unwrap();
    let batch = reader.next_batch().unwrap().unwrap();
    let output_geometry = batch
        .column(0)
        .as_any()
        .downcast_ref::<BinaryArray>()
        .unwrap();
    let actual = decode_wkb(output_geometry.value(0), &WkbLimits::default()).unwrap();
    assert_eq!(actual, expected);
}

#[test]
fn mixed_wkt_dimensions_are_declared_unknown_without_normalization() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("mixed.csv");
    std::fs::write(
        &source,
        "id,geom\n1,\"POINT Z (1 2 3)\"\n2,\"POINT M (4 5 6)\"\n",
    )
    .unwrap();
    let dataset = CsvDriver
        .open(Source::Path(source), read_opts(&[("wkt_column", "geom")]))
        .unwrap();
    assert_eq!(
        dataset.layers()[0]
            .contract
            .geometry
            .as_ref()
            .unwrap()
            .dimensions,
        CoordinateDimensions::Unknown
    );
    let mut reader = dataset.open_layer_reader(&req(65_536)).unwrap();
    let batch = reader.next_batch().unwrap().unwrap();
    let geometries = batch
        .column(0)
        .as_any()
        .downcast_ref::<BinaryArray>()
        .unwrap();
    assert_eq!(
        decode_wkb(geometries.value(0), &WkbLimits::default())
            .unwrap()
            .dimensions,
        CoordinateDimensions::Xyz
    );
    assert_eq!(
        decode_wkb(geometries.value(1), &WkbLimits::default())
            .unwrap()
            .dimensions,
        CoordinateDimensions::Xym
    );
}

#[test]
fn ragged_rows_are_rejected_instead_of_inventing_empty_cells() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("ragged.csv");
    std::fs::write(&source, "id,x,y\n1,12.5\n").unwrap();

    assert!(CsvDriver
        .open(
            Source::Path(source),
            read_opts(&[("x_column", "x"), ("y_column", "y")]),
        )
        .is_err());
}

#[test]
fn malformed_xy_is_rejected_instead_of_becoming_null_geometry() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("invalid-xy.csv");
    std::fs::write(&source, "id,x,y\n1,not-a-number,45.0\n").unwrap();
    let dataset = CsvDriver
        .open(
            Source::Path(source),
            read_opts(&[("x_column", "x"), ("y_column", "y")]),
        )
        .unwrap();
    let mut reader = dataset.open_layer_reader(&req(65_536)).unwrap();

    assert!(reader.next_batch().is_err());
}

#[test]
fn background_reader_preserves_wkb_error_variant() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("invalid-wkt-after-open.csv");
    std::fs::write(&source, "id,wkt\n1,POINT (12 45)\n").unwrap();
    let dataset = CsvDriver
        .open(
            Source::Path(source.clone()),
            read_opts(&[("wkt_column", "wkt")]),
        )
        .unwrap();

    std::fs::write(&source, "id,wkt\n1,NOT_A_GEOMETRY\n").unwrap();
    let mut reader = dataset.open_layer_reader(&req(65_536)).unwrap();

    assert!(matches!(
        reader.next_batch(),
        Err(error) if error.code == plenora_io_model::IoErrorCode::Wkb
    ));
}
