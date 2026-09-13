//! Le prove di [`super`], in un file loro.
//!
//! Restano un **modulo figlio**: vedono i privati del genitore
//! esattamente come quando stavano dentro di lui, e non allargano di
//! una riga la superficie pubblica del crate.

// Il gestore vive in `segnali.rs` perche' la sonda dell'uscita al secondo
// segnale lo compila a sua volta; le sonde della **decisione** restano qui,
// dove il harness le elenca una volta sola.
use std::sync::atomic::AtomicBool;

use super::segnali::{reagisci_al_segnale, AzioneDelSegnale};

use super::*;
use plenora_io_model::budget::OperationCounter;

/// Opzioni di scrittura sul modello unificato, per i test.
fn opzioni_scrittura_di_prova() -> plenora_io_core::WriteOptions {
    match PipelineBudget::builder().build() {
        Ok(bundle) => plenora_io_core::WriteOptions::from_write_parts(bundle.into_write_parts()),
        Err(error) => unreachable!("bundle di test: {error:?}"),
    }
}

#[test]
fn i_flag_atterrano_nei_limiti_della_pipeline() {
    let cli = parse(&[
        "convert".to_owned(),
        "--max-rows".to_owned(),
        "7".to_owned(),
        "--max-columns".to_owned(),
        "5".to_owned(),
        "--max-output-bytes".to_owned(),
        "1024".to_owned(),
    ])
    .expect("flag validi");

    assert_eq!(cli.limits.max_rows(), 7);
    assert_eq!(cli.limits.max_columns(), 5);
    assert_eq!(cli.limits.max_output_bytes(), 1_024);
    // Le quote non esposte dalla CLI restano ai default del modello.
    assert_eq!(
        cli.limits.memory_bytes(),
        PipelineLimits::default().memory_bytes()
    );
}

#[test]
fn la_pipeline_rifiuta_flag_a_zero() {
    // `PipelineLimits::validate` rifiuta le quote nulle: la costruzione
    // propaga l'errore invece di degradare a un default silenzioso.
    let cli = parse(&[
        "convert".to_owned(),
        "--max-rows".to_owned(),
        "0".to_owned(),
    ])
    .expect("il parser accetta lo zero, e' il modello a rifiutarlo");
    assert!(PipelineBudget::builder()
        .limits(cli.limits)
        .build()
        .is_err());
}

/// Le due transizioni, nell'ordine, senza un segnale.
#[test]
fn il_primo_segnale_annulla_e_il_successivo_chiede_l_uscita() {
    let gia_chiesto = AtomicBool::new(false);
    let token = CancellationToken::new();
    assert!(!token.is_cancelled(), "premessa: il token parte armabile");

    assert_eq!(
        reagisci_al_segnale(&gia_chiesto, &token),
        AzioneDelSegnale::Annulla
    );
    assert!(
        token.is_cancelled(),
        "il primo segnale deve annullare, non solo dichiararlo"
    );

    assert_eq!(
        reagisci_al_segnale(&gia_chiesto, &token),
        AzioneDelSegnale::UsciSubito
    );
}

/// La decisione non torna indietro.
///
/// Un `swap` letto male — o sostituito da un `load` seguito da uno `store`
/// — potrebbe far rientrare la macchina nel primo stato, e un terzo Ctrl+C
/// annullerebbe di nuovo invece di uscire.
#[test]
fn dal_secondo_segnale_in_poi_la_decisione_resta_l_uscita() {
    let gia_chiesto = AtomicBool::new(false);
    let token = CancellationToken::new();
    assert_eq!(
        reagisci_al_segnale(&gia_chiesto, &token),
        AzioneDelSegnale::Annulla
    );
    for numero in 2..=5 {
        assert_eq!(
            reagisci_al_segnale(&gia_chiesto, &token),
            AzioneDelSegnale::UsciSubito,
            "il segnale numero {numero} e' tornato alla prima transizione"
        );
    }
}

/// Il ramo dell'uscita non tocca il token.
///
/// La sonda usa un token **diverso** alla seconda chiamata: se le due
/// transizioni fossero invertite, o se il ramo dell'uscita annullasse
/// comunque, quel token risulterebbe annullato. Con un token solo la
/// differenza non si vedrebbe, perche' era gia' annullato dalla prima.
#[test]
fn il_ramo_dell_uscita_non_annulla_nulla() {
    let gia_chiesto = AtomicBool::new(false);
    let primo = CancellationToken::new();
    assert_eq!(
        reagisci_al_segnale(&gia_chiesto, &primo),
        AzioneDelSegnale::Annulla
    );

    let secondo = CancellationToken::new();
    assert_eq!(
        reagisci_al_segnale(&gia_chiesto, &secondo),
        AzioneDelSegnale::UsciSubito
    );
    assert!(
        !secondo.is_cancelled(),
        "il ramo dell'uscita ha annullato un token: le transizioni sono invertite"
    );
}

#[test]
fn deadline_ms_ha_un_default_e_un_flag_che_lo_cambia() {
    // Il default e' quello che il modello applicava gia': il flag rende
    // raggiungibile un valore che c'era, non ne introduce uno nuovo.
    let senza = parse(&["read".to_owned(), "x".to_owned()]).expect("flag validi");
    assert_eq!(
        senza.limits.duration_ms(),
        PipelineLimits::default().duration_ms()
    );
    assert_eq!(senza.limits.duration_ms(), 30_000);

    let con = parse(&[
        "read".to_owned(),
        "x".to_owned(),
        "--deadline-ms".to_owned(),
        "1500".to_owned(),
    ])
    .expect("flag validi");
    assert_eq!(con.limits.duration_ms(), 1_500);
    // La deadline non tocca le altre quote.
    assert_eq!(
        con.limits.memory_bytes(),
        PipelineLimits::default().memory_bytes()
    );
}

#[test]
fn deadline_ms_rifiuta_zero_e_valori_non_rappresentabili() {
    for grezzo in ["0x10", "-1", "1.5", "18446744073709551616", ""] {
        assert!(
            parse(&[
                "read".to_owned(),
                "x".to_owned(),
                "--deadline-ms".to_owned(),
                grezzo.to_owned(),
            ])
            .is_err(),
            "'{grezzo}' doveva essere rifiutato dal parser"
        );
    }
    // Valore mancante.
    assert!(parse(&[
        "read".to_owned(),
        "x".to_owned(),
        "--deadline-ms".to_owned(),
    ])
    .is_err());
    // Lo zero passa dal parser e lo rifiuta il modello, come le altre
    // quote: e' li' che vive la regola, non nella CLI.
    let zero = parse(&[
        "read".to_owned(),
        "x".to_owned(),
        "--deadline-ms".to_owned(),
        "0".to_owned(),
    ])
    .expect("il parser accetta lo zero");
    assert!(PipelineBudget::builder()
        .limits(zero.limits)
        .build()
        .is_err());
    // Una deadline enorme non si avvolge. `build` somma i millisecondi a
    // `Instant::now()` con `checked_add` e fallisce chiuso se la scadenza
    // non e' rappresentabile; se invece la piattaforma la rappresenta --
    // ed e' il caso di Linux con `u64::MAX`, che sono centinaia di milioni
    // di anni -- l'operazione deve risultare **attiva**, non gia' scaduta.
    //
    // La sonda accetta entrambi gli esiti perche' entrambi sono corretti e
    // quale si presenti dipende dalla piattaforma. Cio' che non accetta e'
    // il terzo: una scadenza avvolta all'indietro, che farebbe fallire
    // subito un'operazione a cui e' stato concesso il tempo massimo.
    let enorme = parse(&[
        "read".to_owned(),
        "x".to_owned(),
        "--deadline-ms".to_owned(),
        u64::MAX.to_string(),
    ])
    .expect("il parser accetta il valore");
    if let Ok(bundle) = PipelineBudget::builder().limits(enorme.limits).build() {
        let opzioni = ReadOptions::from_read_parts(bundle.into_read_parts());
        assert!(
            opzioni.budget().context().ensure_active().is_ok(),
            "la deadline massima si e' avvolta in una scadenza gia' passata"
        );
    }
}

/// Il token che la CLI passa alla pipeline e' **quello** del processo.
///
/// Prima la `ReadRequest` portava `CancellationToken::default()`, cioe' un
/// token nuovo per ogni richiesta: nessuno poteva annullarlo, e il reader
/// interrogava un canale senza l'altro capo. La sonda confronta l'effetto,
/// non l'identita': annullare il token legato al `Cli` deve rendere
/// annullata la richiesta che ne nasce.
#[test]
fn la_richiesta_di_lettura_porta_il_token_legato_al_comando() {
    let mut cli = parse(&["read".to_owned(), "x".to_owned()]).expect("flag validi");
    let token = CancellationToken::new();
    cli.cancellazione = token.clone();

    let richiesta = read_request(&cli, 0, ReadScope::Complete);
    assert!(!richiesta.cancellation.is_cancelled());

    token.cancel();
    assert!(
        richiesta.cancellation.is_cancelled(),
        "la richiesta porta un token scollegato da quello del comando"
    );
}

/// E lo stesso token arriva al `PipelineContext` dei due rami.
#[test]
fn i_due_rami_di_convert_osservano_lo_stesso_token() {
    let mut cli = parse(&["convert".to_owned()]).expect("flag validi");
    let token = CancellationToken::new();
    cli.cancellazione = token.clone();

    let (ropts, wopts) = convert_pipeline(&cli).expect("pipeline costruita");
    assert!(ropts.budget().context().ensure_active().is_ok());
    assert!(wopts.budget().context().ensure_active().is_ok());

    token.cancel();
    assert!(ropts.budget().context().ensure_active().is_err());
    assert!(wopts.budget().context().ensure_active().is_err());
}

#[test]
fn memory_bytes_ha_un_default_e_un_flag_che_lo_cambia() {
    // Il default non si muove: chi non passa il flag ottiene ciò che
    // otteneva prima.
    let senza = parse(&["read".to_owned(), "x".to_owned()]).expect("flag validi");
    assert_eq!(
        senza.limits.memory_bytes(),
        PipelineLimits::default().memory_bytes()
    );

    let con = parse(&[
        "read".to_owned(),
        "x".to_owned(),
        "--memory-bytes".to_owned(),
        "134217728".to_owned(),
    ])
    .expect("flag validi");
    assert_eq!(con.limits.memory_bytes(), 134_217_728);
    // La memoria non tocca le altre quote: sono distinte apposta.
    assert_eq!(
        con.limits.max_input_bytes(),
        PipelineLimits::default().max_input_bytes()
    );
}

#[test]
fn memory_bytes_rifiuta_zero_e_valori_non_rappresentabili() {
    // Non rappresentabile: il parser si ferma prima del modello.
    for grezzo in ["0x10", "-1", "1.5", "18446744073709551616", ""] {
        assert!(
            parse(&[
                "read".to_owned(),
                "x".to_owned(),
                "--memory-bytes".to_owned(),
                grezzo.to_owned(),
            ])
            .is_err(),
            "'{grezzo}' doveva essere rifiutato dal parser"
        );
    }
    // Valore mancante.
    assert!(parse(&[
        "read".to_owned(),
        "x".to_owned(),
        "--memory-bytes".to_owned(),
    ])
    .is_err());

    // Zero: il parser lo accetta come intero, il **modello** lo rifiuta.
    // La divisione dei compiti è voluta — il parser sa cos'è un numero, il
    // modello sa quali numeri hanno senso insieme.
    let cli = parse(&[
        "read".to_owned(),
        "x".to_owned(),
        "--memory-bytes".to_owned(),
        "0".to_owned(),
    ])
    .expect("il parser accetta lo zero");
    assert!(PipelineBudget::builder()
        .limits(cli.limits)
        .build()
        .is_err());

    // Sotto `max_wkb_cell_bytes` senza abbassarla: rifiutato, perché una
    // cella non può valere più di tutta la memoria.
    let cli = parse(&[
        "read".to_owned(),
        "x".to_owned(),
        "--memory-bytes".to_owned(),
        "1024".to_owned(),
    ])
    .expect("il parser accetta il valore");
    assert!(PipelineBudget::builder()
        .limits(cli.limits)
        .build()
        .is_err());
}

#[test]
fn memory_bytes_arriva_a_lettura_e_scrittura_dallo_stesso_context() {
    let cli = parse(&[
        "convert".to_owned(),
        "a".to_owned(),
        "b".to_owned(),
        "--memory-bytes".to_owned(),
        "134217728".to_owned(),
    ])
    .expect("flag validi");
    let (ropts, wopts) = convert_pipeline(&cli).expect("pipeline valida");

    assert_eq!(
        ropts.budget().context().limits().memory_bytes(),
        134_217_728
    );
    assert_eq!(
        wopts.budget().context().limits().memory_bytes(),
        134_217_728
    );
    // Non due context uguali: **lo stesso**. È la proprietà che S4.d aveva
    // stabilito e che un flag nuovo potrebbe rompere passando da una strada
    // laterale.
    assert!(ropts
        .budget()
        .context()
        .is_same_pipeline(wopts.budget().context()));

    // E la lettura semplice lo riceve dallo stesso posto.
    let solo_lettura = read_pipeline(&cli).expect("pipeline valida");
    assert_eq!(
        solo_lettura.budget().context().limits().memory_bytes(),
        134_217_728
    );
}

/// Un `GeoParquet` con molte righe e celle minuscole.
///
/// La forma conta: la pagina deve essere grande **senza** che nessuna
/// singola cella lo sia, altrimenti abbassando la memoria scatterebbe
/// `max_wkb_cell_bytes` e il test misurerebbe un altro controllo.
fn scrivi_molte_geometrie_piccole(path: &std::path::Path) {
    use std::collections::HashMap;
    use std::sync::Arc;

    use arrow_array::{BinaryArray, RecordBatch};
    use arrow_schema::{DataType, Field, Schema, SchemaRef};
    use plenora_io_core::{FormatDriver, Sink, WriteLayer, WritePlan};
    use plenora_io_model::contract::{
        CoordinateDimensions, DataContract, FieldId, GeometryColumnContract, GeometryType,
    };
    use plenora_io_model::crs::{CrsKind, ResolvedCrs};
    use plenora_io_model::geometry::{
        ARROW_EXTENSION_NAME_KEY, GEOARROW_WKB_EXTENSION, GEO_CRS_KEY,
    };
    use plenora_io_model::wkb::{encode_wkb, WkbCoordinate, WkbFlavor, WkbGeometry, WkbValue};

    let schema: SchemaRef = Arc::new(Schema::new(vec![Field::new(
        "geometry",
        DataType::Binary,
        true,
    )
    .with_metadata(HashMap::from([
        (
            ARROW_EXTENSION_NAME_KEY.to_owned(),
            GEOARROW_WKB_EXTENSION.to_owned(),
        ),
        (GEO_CRS_KEY.to_owned(), "EPSG:4326".to_owned()),
    ]))]));
    let celle: Vec<Vec<u8>> = (0..50_000)
        .map(|i| {
            let x = f64::from(i);
            encode_wkb(
                &WkbGeometry {
                    value: WkbValue::Point(WkbCoordinate {
                        x,
                        y: x,
                        z: None,
                        m: None,
                    }),
                    dimensions: CoordinateDimensions::Xy,
                    srid: None,
                },
                WkbFlavor::Iso,
            )
            .unwrap()
        })
        .collect();
    let colonna = BinaryArray::from(celle.iter().map(|c| Some(c.as_slice())).collect::<Vec<_>>());
    let batch = RecordBatch::try_new(schema.clone(), vec![Arc::new(colonna)]).unwrap();

    let mut geometria = GeometryColumnContract::wkb_passthrough(
        FieldId(0),
        "geometry",
        ResolvedCrs::new(Some("EPSG:4326".to_owned()), CrsKind::Geographic, None),
        true,
    );
    geometria.set_exact_geometry_types(vec![GeometryType::Point]);
    let plan = WritePlan {
        layers: vec![WriteLayer {
            name: "l".to_owned(),
            contract: DataContract {
                schema,
                geometry: Some(geometria),
            },
        }],
    };
    let driver = driver_geoparquet::GeoParquetDriver;
    let mut writer = driver
        .create(
            Sink::Path(path.to_path_buf()),
            &plan,
            &convert_pipeline(&Cli::default()).unwrap().1,
        )
        .unwrap();
    writer.write(&batch).unwrap();
    writer.finish().unwrap();
}

/// Abbassare `--memory-bytes` abbassa il tetto sulla pagina `GeoParquet`.
///
/// È la ragione per cui il flag esiste: senza, dentro un container con meno
/// memoria del predefinito il tetto restava a 256 MiB — cioè proprio dove
/// andrebbe stretto non si poteva stringerlo.
///
/// Il file ha molte righe con celle minuscole, non una cella grande: così
/// la pagina è grande senza che `--max-wkb-cell-bytes` c'entri, e il rifiuto
/// che si osserva è quello sotto esame e non un altro.
#[test]
fn abbassare_memory_bytes_abbassa_il_tetto_della_pagina_geoparquet() {
    use plenora_io_core::{
        BatchTarget, FormatDriver, ProjectionMode, ReadRequest, ReadScope, Source,
    };
    use plenora_io_model::contract::LayerId;

    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("pagine.parquet");
    scrivi_molte_geometrie_piccole(&path);

    let richiesta = ReadRequest {
        layer: LayerId(0),
        projected_fields: None,
        projection_mode: ProjectionMode::BestEffort,
        pruning_predicate: None,
        spatial_pruning_hint: None,
        scope: ReadScope::default(),
        batch_target: BatchTarget::default(),
        cancellation: plenora_io_model::CancellationToken::default(),
    };
    let driver = driver_geoparquet::GeoParquetDriver;
    let leggi = |flag: &[&str]| -> Result<(), plenora_io_model::PlenoraIoError> {
        let mut argomenti = vec!["read".to_owned(), path.display().to_string()];
        argomenti.extend(flag.iter().map(|f| (*f).to_owned()));
        let cli = parse(&argomenti).expect("flag validi");
        let opzioni = read_pipeline(&cli).expect("pipeline valida");
        let dataset = driver
            .open(Source::Path(path.clone()), opzioni)
            .expect("il file sta sotto le quote di ingresso");
        match dataset.open_layer_reader(&richiesta) {
            Err(errore) => Err(errore),
            Ok(mut lettore) => lettore.next_batch().map(|_| ()),
        }
    };

    // Con il predefinito il tetto è 256 MiB e la pagina passa.
    leggi(&[]).expect("con la memoria predefinita il file si legge");

    // Abbassando la memoria il tetto scende sotto la pagina. Le celle sono
    // di ventuno byte, quindi `--max-wkb-cell-bytes` non è il vincolo che
    // scatta: è la memoria.
    let errore = leggi(&["--memory-bytes", "2000000", "--max-wkb-cell-bytes", "1000"])
        .expect_err("con meno memoria il tetto scende sotto la pagina");
    assert_eq!(
        errore.message,
        "pagina Parquet che dichiara piu' byte non compressi della memoria disponibile",
        "{errore}"
    );
}

#[test]
fn geometry_components_non_deriva_dal_wkb_per_cella() {
    // Follow-up review 2026-08-15: `--max-wkb-components` (per cella) NON
    // deve alimentare il contatore cumulativo `GeometryComponents`.
    // Dataset di molte geometrie piccole avrebbero altrimenti esaurito la
    // quota dopo 100k coordinate totali.
    let cli = parse(&[
        "convert".to_owned(),
        "--max-wkb-components".to_owned(),
        "42".to_owned(),
    ])
    .expect("flag validi");

    assert_eq!(
        cli.limits.max_wkb_components(),
        42,
        "il per-cella segue il flag"
    );
    assert_eq!(
        cli.limits.max_geometry_components(),
        PipelineLimits::default().max_geometry_components(),
        "il cumulativo non deve seguire il per-cella"
    );
}

#[test]
fn i_due_rami_di_convert_hanno_contatori_indipendenti_e_context_condiviso() {
    // Il finding #3 chiedeva contatori indipendenti: una riga non deve
    // consumare due volte la stessa quota. Fino a S4.d la CLI lo otteneva
    // con due budget **scollegati**, che pero' contavano due volte anche
    // memoria e spill e impedivano al writer di vedere l'input osservato
    // dal reader (INV-6). Ora i due rami escono dalle stesse parti.
    let cli = Cli {
        limits: PipelineLimits::default().with_max_rows(100),
        ..Cli::default()
    };
    let (ropts, wopts) = convert_pipeline(&cli).expect("pipeline valida");

    ropts
        .budget()
        .try_lease(OperationCounter::Rows, 60)
        .expect("lease")
        .commit(60)
        .expect("commit");
    assert_eq!(wopts.budget().remaining(OperationCounter::Rows), 100);
    assert!(!ropts.budget().shares_counters_with(wopts.budget()));

    // Context condiviso: memoria, spill e deadline sono gli stessi.
    assert!(ropts
        .budget()
        .context()
        .is_same_pipeline(wopts.budget().context()));
}

#[test]
fn opts_uniti_preserva_precedenza_direzionale() {
    // Finding #11 della review 2026-08-15: la precedenza deve essere
    // "direzionale sovrascrive comune" e non deve dipendere dall'ordine
    // sulla riga di comando. Il test blocca la regola in modo che una
    // regressione la rompa esplicitamente.
    let mut comuni = BTreeMap::new();
    comuni.insert("delim".to_owned(), ",".to_owned());
    comuni.insert("shared".to_owned(), "base".to_owned());
    let mut direzionali = BTreeMap::new();
    direzionali.insert("shared".to_owned(), "override".to_owned());
    direzionali.insert("only-out".to_owned(), "yes".to_owned());
    let uniti = opts_uniti(&comuni, &direzionali);
    assert_eq!(uniti.get("delim").map(String::as_str), Some(","));
    assert_eq!(uniti.get("shared").map(String::as_str), Some("override"));
    assert_eq!(uniti.get("only-out").map(String::as_str), Some("yes"));
    // Le mappe di ingresso restano invariate.
    assert_eq!(comuni.get("shared").map(String::as_str), Some("base"));
    assert_eq!(direzionali.get("delim"), None);
}

/// La sezione di perdita che il v2 deve produrre, scritta per esteso.
///
/// Nel v2 `counts` e' un **elenco** con un ordine e una lunghezza
/// dichiarati, e la sezione porta la propria dichiarazione di troncamento
/// anche quando non ha troncato niente: una dichiarazione che compare solo
/// quando serve non e' una garanzia.
fn perdita_v2_attesa(lossless: bool, conteggi: &[(&str, u64)], esempi: &[Value]) -> Value {
    let counts: Vec<Value> = conteggi
            .iter()
            .map(|(categoria, conteggio)| {
                serde_json::json!({"categoria": categoria, "conteggio": conteggio})
            })
            .collect();
    serde_json::json!({
        "lossless": lossless,
        "counts": counts,
        "esempi": esempi,
        "troncato": false,
        "omesse_esatte": true,
        "omesse": {
            "categorie_omesse": 0,
            "ragioni_omesse": 0,
            "esempi_omessi": 0,
            "omesse_per_byte": 0,
        },
    })
}

// --- il protocollo della busta ----------------------------------------
//
// Qui vivevano cinque sonde sul **doppio** protocollo: che il v2 fosse il
// predefinito, che il v1 restasse byte per byte quello che era, che le due
// forme differissero dove il protocollo lo diceva, che il v1 non si
// raggiungesse per distrazione, e che il suo avviso nominasse i due difetti
// senza finire nella busta.
//
// Erano sonde buone, e cio' che misuravano non esiste piu': l'artefatto
// serve un protocollo solo. Tenerle avrebbe voluto dire conservare il v1
// per poterlo provare -- cioe' tenere in vita la cosa che il profilo
// vieta, per amore della sua sonda.
//
// Cio' che di quelle sonde resta vero e' che la busta dichiara la propria
// versione e rispetta il manifesto, ed e' quanto verifica la sonda qui
// sotto. Che il flag legacy non sia piu' riconosciuto lo verifica il
// confine pubblico, in `check_public_contracts.py`, dove un flag
// sconosciuto fallisce chiuso come ogni altro.

/// Una conversione minima e deterministica, per guardare la busta.
fn busta_di_conversione() -> Value {
    let directory = tempfile::tempdir().unwrap();
    let input = directory.path().join("in.csv");
    let output = directory.path().join("out.geojson");
    std::fs::write(&input, "wkt,nome\nPOINT(1 2),alfa\n").unwrap();
    let argomenti = vec![
        input.to_string_lossy().into_owned(),
        output.to_string_lossy().into_owned(),
        // GeoJSON impone CRS84: dichiarare EPSG:4326 farebbe fallire la
        // conversione prima di produrre qualunque busta.
        "--assume-crs".to_owned(),
        "OGC:CRS84".to_owned(),
        "--in-opt".to_owned(),
        "wkt_column=wkt".to_owned(),
        "--from".to_owned(),
        "csv".to_owned(),
        "--to".to_owned(),
        "geojson".to_owned(),
    ];
    let cli = parse(&argomenti).unwrap();
    cmd_convert(&cli).unwrap()
}

#[test]
fn il_corpo_della_conversione_rispetta_il_manifesto() {
    let corpo = busta_di_conversione();
    // I campi dell'operazione stanno nel corpo, e la busta li avvolge: e'
    // `main` a costruirla, e qui si guarda cio' che l'operazione produce.
    assert!(corpo.get("read_loss").is_some());
    assert_eq!(corpo["read_loss"]["troncato"], serde_json::json!(false));
    assert!(
        corpo["read_loss"]["counts"].is_array(),
        "`counts` e' un elenco con un ordine dichiarato"
    );
    assert_candidate_envelope("convert", &corpo);
}

#[test]
fn la_busta_porta_identita_e_risultato() {
    let busta = busta_di_successo("convert", serde_json::json!({"x": 1}));
    assert_eq!(busta["status"], "ok");
    assert_eq!(busta["protocol_version"], serde_json::json!(2));
    assert_eq!(busta["component"], "plenora-io-tools");
    assert_eq!(busta["component_version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(busta["command"], "convert");
    assert_eq!(busta["contract"], "plenora-io-convert-v1");
    assert_eq!(busta["result"], serde_json::json!({"x": 1}));
    // E i dati **non** escono al primo livello: era il difetto.
    assert!(busta.get("x").is_none());
}

#[test]
fn ogni_categoria_proietta_il_codice_del_contratto() {
    // La tabella di CLI 2.0 §8, scritta qui per esteso: e' una proiezione
    // pubblica, e una sonda che la ricalcolasse dal codice non direbbe
    // niente.
    for (categoria, atteso) in [
        (ErrorCategory::InvalidPlan, 2),
        (ErrorCategory::InvalidConfiguration, 2),
        (ErrorCategory::Schema, 3),
        (ErrorCategory::DataMapping, 3),
        (ErrorCategory::Crs, 3),
        (ErrorCategory::Unsupported, 3),
        (ErrorCategory::ResourceLimit, 4),
        (ErrorCategory::Io, 5),
        (ErrorCategory::NotFound, 5),
        (ErrorCategory::Conflict, 5),
        (ErrorCategory::Protocol, 5),
        (ErrorCategory::Authentication, 5),
        (ErrorCategory::Authorization, 5),
        (ErrorCategory::Timeout, 5),
        (ErrorCategory::Transient, 5),
        (ErrorCategory::Execution, 6),
        (ErrorCategory::Internal, 70),
        (ErrorCategory::Cancelled, 130),
    ] {
        assert_eq!(
            uscita_della_categoria(categoria),
            atteso,
            "{categoria:?} deve proiettare {atteso}"
        );
    }
}

#[test]
fn nessuna_categoria_proietta_il_successo() {
    // La regola che il contratto scrive per esteso: una categoria nuova
    // proietta a 70, «never to success». Senza questa sonda, un ramo
    // aggiunto per distrazione potrebbe far uscire 0 su un fallimento.
    for categoria in [
        ErrorCategory::InvalidPlan,
        ErrorCategory::InvalidConfiguration,
        ErrorCategory::Schema,
        ErrorCategory::DataMapping,
        ErrorCategory::Crs,
        ErrorCategory::Unsupported,
        ErrorCategory::ResourceLimit,
        ErrorCategory::Io,
        ErrorCategory::NotFound,
        ErrorCategory::Conflict,
        ErrorCategory::Protocol,
        ErrorCategory::Authentication,
        ErrorCategory::Authorization,
        ErrorCategory::Timeout,
        ErrorCategory::Transient,
        ErrorCategory::Execution,
        ErrorCategory::Internal,
        ErrorCategory::Cancelled,
    ] {
        assert_ne!(uscita_della_categoria(categoria), 0, "{categoria:?}");
    }
}

/// La busta rispetta il manifesto del protocollo che dichiara.
///
/// Il manifesto e' uno, e la busta deve dichiarare la sua versione.
///
/// La stesura precedente sceglieva fra due manifesti leggendo
/// `protocol_version` dal documento. Serviva finche' l'artefatto ne serviva
/// due; ora una versione diversa da quella del manifesto non e' un
/// protocollo da cercare altrove, e' un difetto.
fn assert_candidate_envelope(name: &str, corpo_o_busta: &Value) {
    let manifest: Value =
        serde_json::from_str(include_str!("../../../release/cli-protocol-v2.json")).unwrap();
    // I comandi rendono il **corpo**, e `main` lo avvolge. Le sonde possono
    // passare l'uno o l'altra: avvolgere qui cio' che non e' gia' avvolto
    // evita di riscrivere venti chiamate per una differenza che non stanno
    // misurando.
    let avvolta;
    let document = if corpo_o_busta.get("protocol_version").is_some() {
        corpo_o_busta
    } else {
        avvolta = busta_di_successo(name, corpo_o_busta.clone());
        &avvolta
    };
    assert_eq!(
        document["protocol_version"].as_u64(),
        Some(busta::PROTOCOLLO),
        "la busta deve dichiarare il protocollo del manifesto"
    );
    let envelope = &manifest["envelopes"][name];
    assert_eq!(document["contract"], envelope["contract"]);
    // I campi dell'operazione stanno in `result`, e quelli della busta al
    // primo livello: il manifesto li elenca insieme perche' li' erano
    // insieme, e qui si cercano dove sono adesso.
    let corpo = document.get("result").unwrap_or(document);
    for field in envelope["required_top_level"].as_array().unwrap() {
        let field = field.as_str().unwrap();
        assert!(
            document.get(field).is_some() || corpo.get(field).is_some(),
            "{name}: campo {field} assente"
        );
    }
    if let Some(forbidden) = envelope["forbidden_legacy_fields"].as_array() {
        for field in forbidden {
            let field = field.as_str().unwrap();
            assert!(
                document.get(field).is_none() && corpo.get(field).is_none(),
                "{name}: campo legacy {field} presente"
            );
        }
    }
    if let Some(required) = envelope["current_producer"]["required_driver_fields"].as_array() {
        for driver in corpo["drivers"].as_array().unwrap() {
            for field in required {
                let field = field.as_str().unwrap();
                assert!(
                    driver.get(field).is_some(),
                    "{name}: campo driver {field} assente"
                );
            }
        }
    }
}

fn materialize_empty_ipc(directory: &tempfile::TempDir) -> PathBuf {
    use std::sync::Arc;

    use arrow_schema::{DataType, Field, Schema};

    let path = directory.path().join("input.arrow");
    let plan = WritePlan {
        layers: vec![WriteLayer {
            name: "layer".to_owned(),
            contract: DataContract {
                schema: Arc::new(Schema::new(vec![Field::new("id", DataType::Int64, false)])),
                geometry: None,
            },
        }],
    };
    let writer = driver_ipc::IpcDriver
        .create(
            Sink::Path(path.clone()),
            &plan,
            &opzioni_scrittura_di_prova(),
        )
        .unwrap();
    writer.finish().unwrap();
    path
}

/// Un punto WKB valido: little-endian, tipo 1, coordinate (0, 0).
///
/// Sta qui e non dentro una fixture perche' due la usano.
const VALID_POINT: &[u8] = &[
    1, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
];

fn materialize_multibatch_geometry_ipc(
    directory: &tempfile::TempDir,
    first_batch_valid: bool,
) -> PathBuf {
    use std::fs::File;
    use std::sync::Arc;

    use arrow_array::{BinaryArray, RecordBatch};
    use arrow_ipc::writer::FileWriter;
    use arrow_schema::{DataType, Field, Schema};
    use plenora_io_model::contract::{FieldId, GeometryColumnContract, GeometryType};
    use plenora_io_model::crs::CrsResolution;
    use plenora_io_model::geometry::{with_contract_version, with_geometry_contract_metadata};

    const INVALID_WKB: &[u8] = &[1, 1, 0];
    let path = directory.path().join(if first_batch_valid {
        "late-invalid.arrow"
    } else {
        "prefix-invalid.arrow"
    });
    let mut geometry =
        GeometryColumnContract::wkb_xy(FieldId(0), "geometry", CrsResolution::Missing, true);
    geometry.set_exact_geometry_types(vec![GeometryType::Point]);
    let field =
        with_geometry_contract_metadata(&Field::new("geometry", DataType::Binary, true), &geometry);
    let schema = with_contract_version(Arc::new(Schema::new(vec![field])));
    let first_values = (0..12)
        .map(|index| {
            Some(if first_batch_valid || index != 1 {
                VALID_POINT
            } else {
                INVALID_WKB
            })
        })
        .collect::<Vec<_>>();
    let first = RecordBatch::try_new(
        schema.clone(),
        vec![Arc::new(BinaryArray::from(first_values))],
    )
    .unwrap();
    let tail = RecordBatch::try_new(
        schema.clone(),
        vec![Arc::new(BinaryArray::from(vec![Some(INVALID_WKB)]))],
    )
    .unwrap();
    let mut writer = FileWriter::try_new(File::create(&path).unwrap(), &schema).unwrap();
    writer.write(&first).unwrap();
    writer.write(&tail).unwrap();
    writer.finish().unwrap();
    path
}

#[test]
fn read_limit_stops_before_invalid_tail_but_convert_remains_complete() {
    let directory = tempfile::tempdir().unwrap();
    let input = materialize_multibatch_geometry_ipc(&directory, true);
    let cli = parse(&[
        input.to_string_lossy().into_owned(),
        "--limit".to_owned(),
        "10".to_owned(),
    ])
    .unwrap();
    let summary = cmd_read(&cli).unwrap();
    assert_eq!(summary["rows_read"], 12);
    assert_eq!(summary["batches"], 1);
    assert_eq!(summary["truncated"], true);

    let output = directory.path().join("must-not-publish.arrow");
    let convert = parse(&[
        input.to_string_lossy().into_owned(),
        output.to_string_lossy().into_owned(),
        "--from".to_owned(),
        "ipc".to_owned(),
        "--to".to_owned(),
        "ipc".to_owned(),
    ])
    .unwrap();
    let error = cmd_convert(&convert).unwrap_err();
    assert_eq!(
        error.0,
        uscita_della_categoria(ErrorCategory::DataMapping),
        "{}",
        error.1
    );
    assert_eq!(error.1["error"]["code"], "FORMAT_ERROR");
    assert_eq!(
        error.1["error"]["row_diagnostics"]["examples"][0]["source_index"],
        12
    );
    assert!(!output.exists());
}

#[test]
fn zero_read_limit_keeps_the_frozen_summary_without_observing_tail() {
    let directory = tempfile::tempdir().unwrap();
    let input = materialize_multibatch_geometry_ipc(&directory, false);
    let cli = parse(&[
        input.to_string_lossy().into_owned(),
        "--limit".to_owned(),
        "0".to_owned(),
    ])
    .unwrap();

    let summary = cmd_read(&cli).unwrap();
    assert_eq!(summary["rows_read"], 0);
    assert_eq!(summary["batches"], 0);
    assert_eq!(summary["truncated"], true);
    // Il contratto sta nella **busta**, non nel corpo.
    assert_eq!(
        busta_di_successo("read", summary)["contract"],
        "plenora-io-read-result-v1"
    );
}

/// Due batch, tutti validi: dodici righe e una.
///
/// `materialize_multibatch_geometry_ipc` porta sempre una coda invalida --
/// e' cio' per cui esiste -- e le sonde sulla **semantica del limite** non
/// possono usarla: il file non si legge per intero, e senza quel conteggio
/// non c'e' niente con cui confrontare il conteggio troncato.
fn materialize_multibatch_valid_ipc(directory: &tempfile::TempDir) -> PathBuf {
    use std::fs::File;
    use std::sync::Arc;

    use arrow_array::{BinaryArray, RecordBatch};
    use arrow_ipc::writer::FileWriter;
    use arrow_schema::{DataType, Field, Schema};
    use plenora_io_model::contract::{FieldId, GeometryColumnContract, GeometryType};
    use plenora_io_model::crs::CrsResolution;
    use plenora_io_model::geometry::{with_contract_version, with_geometry_contract_metadata};

    let path = directory.path().join("all-valid.arrow");
    let mut geometry =
        GeometryColumnContract::wkb_xy(FieldId(0), "geometry", CrsResolution::Missing, true);
    geometry.set_exact_geometry_types(vec![GeometryType::Point]);
    let field =
        with_geometry_contract_metadata(&Field::new("geometry", DataType::Binary, true), &geometry);
    let schema = with_contract_version(Arc::new(Schema::new(vec![field])));

    let dodici = RecordBatch::try_new(
        schema.clone(),
        vec![Arc::new(BinaryArray::from(
            (0..12).map(|_| Some(VALID_POINT)).collect::<Vec<_>>(),
        ))],
    )
    .unwrap();
    let una = RecordBatch::try_new(
        schema.clone(),
        vec![Arc::new(BinaryArray::from(vec![Some(VALID_POINT)]))],
    )
    .unwrap();

    let mut writer = FileWriter::try_new(File::create(&path).unwrap(), &schema).unwrap();
    writer.write(&dodici).unwrap();
    writer.write(&una).unwrap();
    writer.finish().unwrap();
    path
}

/// `--limit` e' una soglia verificata **fra un batch e il successivo**.
///
/// Non taglia un batch a meta': il ciclo di lettura chiede il batch, lo
/// conta per intero, e solo allora guarda se il limite sia stato raggiunto.
/// `rows_read` puo' percio' superare il limite della parte residua del
/// batch corrente, e su un file letto in un colpo solo lo supera sempre.
///
/// E' una conseguenza dello streaming, non una svista: fermarsi a meta' di
/// un batch vorrebbe dire consegnarne uno parziale a chi lo conta, e il
/// conteggio non direbbe piu' quante righe il reader ha davvero decodificato.
///
/// La sonda fissa il fatto perche' il nome non lo suggerisce, e perche'
/// `release/cli-protocol-v2.json` lo ratifica in `envelopes.read.semantica`:
/// una clausola del contratto che nessuno esercita e' una promessa che si
/// legge come verificata.
#[test]
fn read_limit_is_checked_between_batches_so_rows_may_exceed_it() {
    let directory = tempfile::tempdir().unwrap();
    let input = materialize_multibatch_valid_ipc(&directory);

    // Senza limite: quante righe ha il primo batch, e quante il file.
    let intero = cmd_read(&parse(&[input.to_string_lossy().into_owned()]).unwrap())
        .expect("il file si legge per intero");
    let righe_totali = intero["rows_read"].as_u64().expect("un conteggio");
    assert!(righe_totali > 1, "serve piu' di una riga per dire qualcosa");

    // Con limite uno: si ferma dopo il **primo batch**, che ne porta piu'
    // di una.
    let cli = parse(&[
        input.to_string_lossy().into_owned(),
        "--limit".to_owned(),
        "1".to_owned(),
    ])
    .unwrap();
    let assaggio = cmd_read(&cli).expect("un limite basso non e' un errore");
    assert_eq!(assaggio["batches"], 1, "un batch solo, e intero");
    let lette = assaggio["rows_read"].as_u64().expect("un conteggio");
    assert!(
        lette > 1,
        "il limite si verifica fra i batch: `rows_read` = {lette} lo supera"
    );
    assert_eq!(assaggio["truncated"], true);
}

/// `truncated` dice **arresto per limite con EOF non accertato**.
///
/// Non dice che esistano altre righe: dice che la lettura si e' fermata
/// perche' il limite e' stato raggiunto, e che il reader non ha visto la
/// fine del file. Le due cose non coincidono, e la differenza si vede
/// esattamente qui -- un limite pari al numero di righe del file lascia
/// `truncated` vero pur avendole lette tutte.
///
/// Il significato **prudente** e' quello giusto: per sapere che non c'e'
/// altro bisognerebbe chiedere un batch in piu' e vederlo tornare vuoto,
/// cioe' leggere oltre il limite che il chiamante ha imposto. Affermare
/// l'EOF senza averlo osservato sarebbe la cosa peggiore delle due.
#[test]
fn read_truncated_means_stopped_at_the_limit_not_that_more_rows_exist() {
    let directory = tempfile::tempdir().unwrap();
    let input = materialize_multibatch_valid_ipc(&directory);
    let intero = cmd_read(&parse(&[input.to_string_lossy().into_owned()]).unwrap())
        .expect("il file si legge per intero");
    let righe = intero["rows_read"].as_u64().expect("un conteggio");
    assert_eq!(
        intero["truncated"], false,
        "senza limite la fine del file e' osservata"
    );

    // Il limite **esatto**: tutte le righe sono lette, e non c'e' altro.
    let esatto = cmd_read(
        &parse(&[
            input.to_string_lossy().into_owned(),
            "--limit".to_owned(),
            righe.to_string(),
        ])
        .unwrap(),
    )
    .expect("il file si legge");
    assert_eq!(esatto["rows_read"], righe);
    assert_eq!(
        esatto["truncated"], true,
        "vero pur non essendoci altro: il reader si e' fermato al limite \
             senza chiedere il batch che gli avrebbe mostrato la fine"
    );

    // Un limite oltre la fine: il reader arriva all'EOF e lo dice.
    let oltre = cmd_read(
        &parse(&[
            input.to_string_lossy().into_owned(),
            "--limit".to_owned(),
            (righe + 1).to_string(),
        ])
        .unwrap(),
    )
    .expect("il file si legge");
    assert_eq!(oltre["rows_read"], righe);
    assert_eq!(oltre["truncated"], false);
}

#[test]
fn read_limit_rejects_invalid_rows_inside_the_observed_prefix() {
    let directory = tempfile::tempdir().unwrap();
    let input = materialize_multibatch_geometry_ipc(&directory, false);
    let cli = parse(&[
        input.to_string_lossy().into_owned(),
        "--limit".to_owned(),
        "10".to_owned(),
    ])
    .unwrap();
    let error = cmd_read(&cli).unwrap_err();
    let diagnostics = &error.1["error"]["row_diagnostics"];
    assert_eq!(diagnostics["completeness"], "partial");
    assert_eq!(diagnostics["examples"][0]["source_index"], 1);
    assert_eq!(
        diagnostics["knowledge_limits"][0],
        "read_scope_row_limit_reached"
    );
}

#[test]
fn parse_flags_and_opts() {
    let args = [
        "--assume-crs",
        "EPSG:4326",
        "in.csv",
        "--opt",
        "wkt_column=g",
        "--durable",
        "--layer",
        "2",
        "--max-rows",
        "123",
        "--max-wkb-depth",
        "9",
    ]
    .map(String::from)
    .to_vec();
    let cli = parse(&args).unwrap();
    assert_eq!(cli.assume_crs.as_deref(), Some("EPSG:4326"));
    assert_eq!(cli.positionals, vec!["in.csv".to_owned()]);
    assert_eq!(cli.opts.get("wkt_column").map(String::as_str), Some("g"));
    assert!(cli.durable);
    assert_eq!(cli.layer, Some(2));
    assert_eq!(cli.limits.max_rows(), 123);
    assert_eq!(cli.limits.max_wkb_depth(), 9);
}

#[test]
fn kv_split() {
    assert_eq!(kv("a=b").unwrap(), ("a".to_owned(), "b".to_owned()));
    assert!(kv("nope").is_err());
}

#[test]
fn reader_busy_has_stable_cli_error() {
    let (exit, document) = map_err(plenora_io_model::PlenoraIoError::reader_busy("kml", 0));
    assert_eq!(exit, uscita_della_categoria(ErrorCategory::Conflict));
    assert_eq!(document["error"]["code"], "READER_BUSY");
    assert_eq!(document["error"]["category"], "conflict");
    assert_eq!(document["error"]["phase"], "prepare");
    assert_eq!(document["error"]["remote_effect"], "none");
    assert_eq!(
        document["error"]["retry"],
        serde_json::json!({"kind": "never"})
    );
    assert!(document["error"].get("row_diagnostics").is_none());
}

#[test]
fn output_exists_keeps_the_frozen_cli_exit_and_category() {
    let (exit, document) = map_err(PlenoraIoError::destinazione_esistente());
    assert_eq!(exit, uscita_della_categoria(ErrorCategory::Conflict));
    assert_eq!(document["error"]["code"], "OUTPUT_EXISTS");
    assert_eq!(document["error"]["category"], "conflict");
}

#[test]
fn row_diagnostics_are_preserved_in_the_cli_error_envelope() {
    let cause = "shapefile.inner_ring_without_outer".to_owned();
    let diagnostics = plenora_io_model::RowDiagnostics {
        contract: plenora_io_model::ROW_DIAGNOSTICS_CONTRACT.to_owned(),
        scope: plenora_io_model::RowDiagnosticScope::Read,
        index_basis: plenora_io_model::ROW_DIAGNOSTICS_INDEX_BASIS.to_owned(),
        completeness: plenora_io_model::RowDiagnosticsCompleteness::Complete,
        knowledge_limits: None,
        observed_total: 1,
        total: Some(1),
        input_total: None,
        counts: std::collections::BTreeMap::from([(cause.clone(), 1)]),
        examples_limit: 1,
        examples_truncated: false,
        examples: vec![plenora_io_model::RowDiagnosticExample {
            source_index: 17,
            cause,
            column: None,
            key: None,
            write_state: None,
        }],
        diagnostic_state_counts: None,
        write_outcome: None,
    };
    let error = PlenoraIoError::formato_redatto(
        "shp",
        &PublicMessage::Curated("riga Shapefile non valida"),
    )
    .with_row_diagnostics(diagnostics);

    let (exit, document) = map_err(error);

    assert_eq!(exit, uscita_della_categoria(ErrorCategory::DataMapping));
    assert_eq!(document["status"], "error");
    assert_eq!(document["protocol_version"], busta::PROTOCOLLO);
    assert_eq!(document["contract"], "plenora-error-v1");
    assert_eq!(document["error"]["code"], "FORMAT_ERROR");
    assert_eq!(document["error"]["category"], "data_mapping");
    assert_eq!(document["error"]["phase"], "read");
    assert_eq!(document["error"]["remote_effect"], "none");
    assert_eq!(
        document["error"]["retry"],
        serde_json::json!({"kind": "never"})
    );
    assert_eq!(
        document["error"]["row_diagnostics"]["contract"],
        plenora_io_model::ROW_DIAGNOSTICS_CONTRACT
    );
    assert_eq!(
        document["error"]["row_diagnostics"]["examples"][0]["source_index"],
        17
    );
}

/// `plenora-io-error-v1` ha esattamente questi campi, e non ne acquista
/// altri per sbaglio.
///
/// S9 ha riempito `PlenoraIoError::driver` e `PlenoraIoError::field` su
/// molti piu' errori di prima — `field` con un `ContractIdentifier`, che e'
/// il punto della migrazione. Nessuno dei due e' emesso da questo
/// envelope, e **non deve diventarlo per effetto collaterale**: aggiungere
/// un campo al wire e' un cambiamento di contratto, non una conseguenza di
/// un refactor interno.
///
/// Il test guarda l'insieme delle chiavi, non le singole: un `assert` per
/// campo assente si dimentica del campo che nessuno ha ancora inventato.
#[test]
fn il_wire_v1_ha_esattamente_i_campi_dichiarati_e_non_acquista_field() {
    use plenora_io_model::{ContractIdentifier, ErrorContext, PublicMessage};

    // Un errore con contesto ricco: driver, campo e ragione di capability.
    let schema = arrow_schema::Schema::new(vec![arrow_schema::Field::new(
        "geometry",
        arrow_schema::DataType::Binary,
        true,
    )]);
    let identificatore =
        ContractIdentifier::from_schema_field(&schema, plenora_io_model::contract::FieldId(0))
            .expect("il nome e' nominabile");
    let contesto = ErrorContext::nuovo()
        .con_driver("geoparquet")
        .con_identificatore(identificatore);
    let error = PlenoraIoError::schema_redatto(&PublicMessage::Curated(
        "campo esposto non presente nello schema fisico",
    ))
    .con_contesto(&contesto);

    // Il contesto e' arrivato nel tipo Rust...
    assert_eq!(error.driver.as_deref(), Some("geoparquet"));
    assert_eq!(error.field.as_deref(), Some("geometry"));

    // ...e non sul wire.
    let (_, document) = map_err(error);
    let campi: std::collections::BTreeSet<&str> = document["error"]
        .as_object()
        .expect("l'errore e' un oggetto")
        .keys()
        .map(String::as_str)
        .collect();
    let attesi: std::collections::BTreeSet<&str> = [
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
        campi, attesi,
        "plenora-io-error-v1 ha cambiato forma: {campi:?}"
    );

    let messaggio = document["error"]["message"]
        .as_str()
        .expect("il messaggio e' una stringa");
    assert!(
        !messaggio.contains("geometry"),
        "il nome del campo non deve rientrare dal messaggio: {messaggio}"
    );
}

/// La busta degli errori d'uso della CLI, che e' una **via diversa** da
/// `map_err`: passa per `usage_err` -> `local_err_doc` -> `err_doc`, e per
/// molto tempo nessun test ne ha verificato la forma. Due vie che
/// producono la stessa busta, e una sola provata, sono una busta provata a
/// meta'.
#[test]
fn la_busta_degli_errori_d_uso_ha_esattamente_le_sei_chiavi() {
    let (exit, documento) = usage_err(&PublicMessage::CuratedPair(
        "opzione sconosciuta; ammesse:",
        OPZIONI_AMMESSE,
    ));

    assert_eq!(
        exit,
        uscita_della_categoria(ErrorCategory::InvalidConfiguration),
        "l'exit viene dalla categoria, non da un numero scritto qui"
    );
    assert_eq!(documento["protocol_version"], busta::PROTOCOLLO);
    assert_eq!(documento["contract"], "plenora-error-v1");

    let campi: std::collections::BTreeSet<&str> = documento["error"]
        .as_object()
        .expect("l'errore e' un oggetto")
        .keys()
        .map(String::as_str)
        .collect();
    let attesi: std::collections::BTreeSet<&str> = [
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
        campi, attesi,
        "plenora-io-error-v1 ha cambiato forma sulla via d'uso: {campi:?}"
    );
    // I campi d'identita' stanno **fuori** dall'oggetto `error`, e li
    // aggiunge `main` quando conosce il comando: un errore d'uso non ne ha
    // uno, e la busta lo dira' `unknown` invece di inventarne uno.
    assert!(documento.get("component").is_none());

    // Il quartetto della via d'uso, che nessuno snapshot puo' vedere: il
    // `code` sul wire non viene da `IoErrorCode` ma dal letterale passato a
    // `err_doc`, quindi va fissato qui o non e' fissato da nessuna parte.
    let errore = &documento["error"];
    assert_eq!(errore["category"], "invalid_configuration");
    assert_eq!(errore["phase"], "validate");
    assert_eq!(errore["remote_effect"], "none");
    // `retry` sul wire e' un oggetto `{"kind": …}`, non una stringa nuda:
    // fissato sull'osservato, non su come me lo immaginavo.
    assert_eq!(errore["retry"]["kind"], "never");
    assert_eq!(errore["code"], "CLI_USAGE");
}

/// Nessun argomento della riga di comando finisce nella busta.
///
/// I due siti che lo facevano — il token di un'opzione sconosciuta e il
/// valore di `--opt` mal formato — passavano `argv` dentro `format!`. Il
/// test costruisce gli argomenti con un marcatore improbabile e verifica
/// che non compaia nel documento serializzato.
#[test]
fn nessun_argomento_della_riga_di_comando_entra_nella_busta() {
    const MARCATORE: &str = "zzMARCATORE-ARGVzz";

    let casi = vec![
        vec![format!("--{MARCATORE}")],
        vec!["--opt".to_owned(), MARCATORE.to_owned()],
        vec!["--in-opt".to_owned(), MARCATORE.to_owned()],
        vec!["--out-opt".to_owned(), MARCATORE.to_owned()],
        vec!["--layer".to_owned(), MARCATORE.to_owned()],
        vec!["--max-rows".to_owned(), MARCATORE.to_owned()],
    ];

    for argomenti in casi {
        let Err((_, documento)) = parse(&argomenti) else {
            panic!("{argomenti:?}: doveva essere rifiutato");
        };
        let testo = documento.to_string();
        assert!(
            !testo.contains(MARCATORE),
            "{argomenti:?}: l'argomento e' uscito nella busta: {testo}"
        );
    }
}

#[test]
fn cancellation_has_dedicated_exit_and_preserves_axes() {
    let error = PlenoraIoError::cancelled(ErrorPhase::Read, false);

    let (exit, document) = map_err(error);

    assert_eq!(exit, uscita_della_categoria(ErrorCategory::Cancelled));
    assert_eq!(document["status"], "error");
    assert_eq!(document["protocol_version"], busta::PROTOCOLLO);
    assert_eq!(document["contract"], "plenora-error-v1");
    assert_eq!(document["error"]["code"], "CANCELLED");
    assert_eq!(document["error"]["category"], "cancelled");
    assert_eq!(document["error"]["phase"], "read");
    assert_eq!(document["error"]["remote_effect"], "none");
    assert_eq!(
        document["error"]["retry"],
        serde_json::json!({"kind": "never"})
    );
    assert!(document["error"].get("row_diagnostics").is_none());
}

#[test]
fn data_mapping_changes_exit_only_and_preserves_frozen_error_codes() {
    for (error, expected_code) in [
        (
            PlenoraIoError::formato_redatto("shp", &PublicMessage::Curated("formato")),
            "FORMAT_ERROR",
        ),
        (
            PlenoraIoError::wkb_redatto(&PublicMessage::Curated("wkb")),
            "FORMAT_ERROR",
        ),
        (
            PlenoraIoError::Json(serde_json::from_str::<Value>("{").unwrap_err()),
            "FORMAT_ERROR",
        ),
    ] {
        let (exit, document) = map_err(error);
        assert_eq!(exit, uscita_della_categoria(ErrorCategory::DataMapping));
        assert_eq!(document["error"]["code"], expected_code);
        assert_eq!(document["error"]["category"], "data_mapping");
    }
}

#[test]
fn deadline_is_timeout_not_caller_cancellation() {
    let (exit, document) = map_err(PlenoraIoError::cancelled(ErrorPhase::Read, true));
    assert_eq!(exit, uscita_della_categoria(ErrorCategory::Timeout));
    assert_eq!(document["error"]["code"], "FORMAT_ERROR");
    assert_eq!(document["error"]["category"], "timeout");
}

#[test]
fn retry_after_keeps_delay_in_the_cli_envelope() {
    let error = PlenoraIoError::redatto(
        plenora_io_model::IoErrorCode::Generic,
        ErrorCategory::Transient,
        ErrorPhase::Connect,
        RemoteEffect::None,
        RetryDisposition::After(2_750),
        &PublicMessage::Curated("servizio temporaneamente non disponibile"),
    );
    let (exit, document) = map_err(error);

    assert_eq!(exit, uscita_della_categoria(ErrorCategory::Transient));
    // `main` aggiunge l'identita' quando conosce il comando: la sonda
    // guarda cio' che esce dal processo, non cio' che `map_err` produce
    // a meta' strada.
    let document = con_identita(document, "convert");
    assert_candidate_envelope("error", &document);
    assert_eq!(
        document,
        serde_json::json!({
            "status": "error",
            "protocol_version": 2,
            "component": "plenora-io-tools",
            "component_version": env!("CARGO_PKG_VERSION"),
            "contract": "plenora-error-v1",
            "command": "convert",
            "error": {
                "category": "transient",
                "phase": "connect",
                "remote_effect": "none",
                "retry": {"kind": "after", "delay_ms": 2_750},
                "code": "FORMAT_ERROR",
                "message": "servizio temporaneamente non disponibile",
            },
        })
    );
}

#[test]
fn usage_errors_also_expose_machine_readable_axes() {
    let (exit, document) = usage_err(&PublicMessage::Curated("argomento mancante"));
    assert_eq!(exit, 2);
    assert_eq!(document["error"]["category"], "invalid_configuration");
    assert_eq!(document["error"]["phase"], "validate");
    assert_eq!(
        document["error"]["retry"],
        serde_json::json!({"kind": "never"})
    );
    assert_eq!(document["error"]["message"], "argomento mancante");
}

#[test]
fn convert_observability_separates_read_write_and_end_to_end_fidelity() {
    let mut read_loss = LossReport::default();
    read_loss.record("inconsistent_crs_representations", 1);
    let read = FidelityAssessment::lossless().with_loss_report(&read_loss);
    let write = FidelityAssessment::lossless();
    let conversion = combined_fidelity(&read, &write);

    assert_eq!(conversion.level, Fidelity::Approximating);
    assert_eq!(
        loss_doc(&read, &read_loss).expect("budget"),
        serde_json::json!({
            "lossless": false,
            "counts": [{"categoria": "inconsistent_crs_representations", "conteggio": 1}],
            "esempi": [],
            "troncato": false,
            "omesse_esatte": true,
            "omesse": {
                "categorie_omesse": 0,
                "esempi_omessi": 0,
                "omesse_per_byte": 0,
                "ragioni_omesse": 0,
            },
        })
    );
    assert_eq!(
        loss_doc(&write, &LossReport::default()).expect("budget"),
        serde_json::json!({
            "lossless": true,
            "counts": [],
            "esempi": [],
            "troncato": false,
            "omesse_esatte": true,
            "omesse": {
                "categorie_omesse": 0,
                "esempi_omessi": 0,
                "omesse_per_byte": 0,
                "ragioni_omesse": 0,
            },
        })
    );
}

#[test]
fn combined_fidelity_uses_the_worst_level_and_bounds_reasons() {
    let mut read = FidelityAssessment::for_format("shp", Fidelity::Conditional);
    for index in 0..plenora_io_core::MAX_FIDELITY_REASONS {
        read.add_reason(
            plenora_io_core::FidelityReasonCode::FormatConstraint,
            format!("read-{index}"),
        );
    }
    let write = FidelityAssessment::for_format("dxf", Fidelity::Approximating);
    let combined = combined_fidelity(&read, &write);

    assert_eq!(combined.level, Fidelity::Approximating);
    assert_eq!(
        combined.ragioni_v1().len(),
        plenora_io_core::MAX_FIDELITY_REASONS
    );
}

#[test]
#[allow(clippy::too_many_lines)]
fn convert_exposes_reader_crs_inconsistency_without_writer_ambiguity() {
    use std::collections::HashMap;
    use std::sync::Arc;

    use arrow_schema::{DataType, Field, Schema};
    use plenora_io_model::contract::{FieldId, GeometryColumnContract, GeometryType};
    use plenora_io_model::crs::{CrsKind, CrsResolution, ResolvedCrs};
    use plenora_io_model::geometry::{
        with_geometry_contract_metadata, ARROW_EXTENSION_NAME_KEY, GEOARROW_WKB_EXTENSION,
    };

    let directory = tempfile::tempdir().unwrap();
    let input = directory.path().join("input.arrow");
    let output = directory.path().join("output.arrow");
    let mut geometry = GeometryColumnContract::wkb_xy(
        FieldId(0),
        "geometry",
        CrsResolution::resolved(ResolvedCrs::new(
            Some("EPSG:4326".to_owned()),
            CrsKind::Geographic,
            None,
        )),
        true,
    );
    geometry.srid = Some(3003);
    geometry.set_exact_geometry_types(vec![GeometryType::Point]);
    let base = Field::new("geometry", DataType::Binary, true).with_metadata(HashMap::from([(
        ARROW_EXTENSION_NAME_KEY.to_owned(),
        GEOARROW_WKB_EXTENSION.to_owned(),
    )]));
    let schema = Arc::new(Schema::new(vec![with_geometry_contract_metadata(
        &base, &geometry,
    )]));
    let plan = WritePlan {
        layers: vec![WriteLayer {
            name: "conflicting_crs".to_owned(),
            contract: DataContract {
                schema,
                geometry: Some(geometry),
            },
        }],
    };
    let driver = driver_ipc::IpcDriver;
    let writer = driver
        .create(
            Sink::Path(input.clone()),
            &plan,
            &opzioni_scrittura_di_prova(),
        )
        .unwrap();
    writer.finish().unwrap();

    let cli = parse(&[
        input.to_string_lossy().into_owned(),
        output.to_string_lossy().into_owned(),
        "--from".to_owned(),
        "ipc".to_owned(),
        "--to".to_owned(),
        "ipc".to_owned(),
    ])
    .unwrap();
    let document = cmd_convert(&cli).unwrap();

    assert_candidate_envelope("convert", &document);
    assert_eq!(
        document["read_loss"],
        perdita_v2_attesa(
            false,
            &[("inconsistent_crs_representations", 1)],
            // L'esempio esce **redatto**: nessun nome di layer o di
            // colonna, e i tre SRID -- che sono codici di autorita' e sono
            // la cosa che l'incoerenza deve dire.
            &[serde_json::json!({
                "category": "inconsistent_crs_representations",
                // La geometria e' il campo zero dello schema: l'esempio dice
                // **dove** senza dire come si chiama.
                "field_index": 0,
                "context": "definition_epsg=absent id_epsg=4326 srid=3003",
            })],
        )
    );
    assert_eq!(document["write_loss"], perdita_v2_attesa(true, &[], &[]));
    assert_eq!(document["conversion_fidelity"]["level"], "approximating");

    let reopened = driver
        .open(
            Source::Path(output),
            match plenora_io_model::budget::PipelineBudget::builder().build() {
                Ok(bundle) => {
                    plenora_io_core::ReadOptions::from_read_parts(bundle.into_read_parts())
                }
                Err(error) => unreachable!("bundle di test: {error:?}"),
            },
        )
        .unwrap();
    let reopened_geometry = reopened.layers()[0].contract.geometry.as_ref().unwrap();
    assert_eq!(reopened_geometry.crs.id(), Some("EPSG:4326"));
    assert_eq!(reopened_geometry.srid, Some(3003));

    let shapefile_output = directory.path().join("must_not_exist.shp");
    let cli = parse(&[
        input.to_string_lossy().into_owned(),
        shapefile_output.to_string_lossy().into_owned(),
        "--from".to_owned(),
        "ipc".to_owned(),
        "--to".to_owned(),
        "shp".to_owned(),
    ])
    .unwrap();
    let (exit, error) = cmd_convert(&cli).unwrap_err();
    assert_eq!(exit, uscita_della_categoria(ErrorCategory::Unsupported));
    assert_eq!(error["error"]["category"], "unsupported");
    assert_eq!(error["error"]["phase"], "validate");
    assert_eq!(error["error"]["remote_effect"], "none");
    assert_eq!(error["error"]["retry"]["kind"], "never");
    assert!(error["error"]["message"]
        .as_str()
        .unwrap()
        .contains("rappresentazioni CRS discordanti"));
    assert!(!shapefile_output.exists());
}

#[test]
fn convert_round_trips_declared_unresolved_srid_only_without_synthesis() {
    use std::collections::HashMap;
    use std::fs::File;
    use std::sync::Arc;

    use arrow_ipc::reader::FileReader;
    use arrow_ipc::writer::FileWriter;
    use arrow_schema::{DataType, Field, Schema};
    use plenora_io_model::geometry::{
        with_contract_version, ARROW_EXTENSION_NAME_KEY, GEOARROW_WKB_EXTENSION,
        PLENORA_AXIS_ORDER_KEY, PLENORA_CRS_DEFINITION_KEY, PLENORA_CRS_ID_KEY,
        PLENORA_CRS_RESOLUTION_KEY, PLENORA_DIMENSIONS_KEY, PLENORA_ENCODING_KEY,
        PLENORA_GEOMETRY_TYPES_KEY, PLENORA_SRID_KEY, PLENORA_TYPES_DECLARATION_KEY,
    };

    let directory = tempfile::tempdir().unwrap();
    let input = directory.path().join("srid-only-input.arrow");
    let output = directory.path().join("srid-only-output.arrow");
    let field = Field::new("geom", DataType::Binary, true).with_metadata(HashMap::from([
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
    ]));
    let schema = with_contract_version(Arc::new(Schema::new(vec![field])));
    FileWriter::try_new(File::create(&input).unwrap(), &schema)
        .unwrap()
        .finish()
        .unwrap();

    let cli = parse(&[
        input.to_string_lossy().into_owned(),
        output.to_string_lossy().into_owned(),
        "--from".to_owned(),
        "ipc".to_owned(),
        "--to".to_owned(),
        "ipc".to_owned(),
    ])
    .unwrap();
    // `cmd_convert` rende il **corpo**: lo `status` e' della busta, e chi
    // la costruisce e' `main`. Che la conversione sia riuscita lo dice gia'
    // l'`unwrap`.
    let document = cmd_convert(&cli).unwrap();
    assert!(document.get("publish_outcome").is_some());

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
fn projection_unsupported_has_stable_cli_error() {
    let (exit, document) = map_err(plenora_io_model::PlenoraIoError::projection_unsupported(
        "csv",
    ));
    assert_eq!(exit, uscita_della_categoria(ErrorCategory::Unsupported));
    assert_eq!(document["error"]["code"], "PROJECTION_UNSUPPORTED");
}

#[test]
fn unresolved_crs_has_stable_redacted_cli_error() {
    let raw = plenora_io_model::crs::RawCrs::new(
        "LOCAL_CS[\"survey-grid-secret\"]".to_owned(),
        Some("authority-secret".to_owned()),
    );
    let (_, document) = map_err(plenora_io_model::PlenoraIoError::crs_non_risolto_redatto(
        "shp", &raw,
    ));
    assert_eq!(document["error"]["code"], "CRS_UNRESOLVED");
    assert!(!document.to_string().contains("survey-grid-secret"));
    assert!(!document.to_string().contains("authority-secret"));
}

#[test]
fn ext_to_driver() {
    assert_eq!(
        driver_for_path(Path::new("x.geojson"))
            .unwrap()
            .descriptor()
            .id(),
        "geojson"
    );
    assert_eq!(
        driver_for_path(Path::new("x.gpkg"))
            .unwrap()
            .descriptor()
            .id(),
        "gpkg"
    );
    assert_eq!(
        driver_for_path(Path::new("x.parquet"))
            .unwrap()
            .descriptor()
            .id(),
        "geoparquet"
    );
    assert_eq!(
        driver_for_path(Path::new("x.shp.d"))
            .unwrap()
            .descriptor()
            .id(),
        "shp"
    );
    assert!(driver_for_path(Path::new("x.zzz")).is_err());
}

#[cfg(not(feature = "gdal-backend"))]
#[test]
fn default_catalog_marks_filegdb_unavailable_and_names_the_required_feature() {
    let document = cmd_catalog();
    let filegdb = document["drivers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|driver| driver["id"] == "filegdb")
        .expect("FileGDB deve restare individuabile nel catalogo");

    assert_eq!(filegdb["available"], false);
    assert_eq!(filegdb["required_feature"], "gdal-backend");
}

#[test]
fn catalog_fields_have_exact_types_and_semantics_for_every_driver() {
    let document = catalog_document_con(false);
    for driver in document["drivers"].as_array().unwrap() {
        assert!(
            driver["available"].is_boolean(),
            "{}: available",
            driver["id"]
        );
        if driver["id"] == "filegdb" {
            assert_eq!(driver["available"], false);
            assert_eq!(driver["required_feature"].as_str(), Some("gdal-backend"));
        } else {
            assert_eq!(driver["available"], true, "{}", driver["id"]);
            assert!(driver["required_feature"].is_null(), "{}", driver["id"]);
        }
    }
}

/// Snapshot **del solo legacy**: `read_mode`, driver per driver.
///
/// Sta da solo, separato da quello della tripla, perché prova una cosa
/// diversa: che S8 **non abbia toccato** un campo che `plenora-io-catalog-v1`
/// emette da sempre. Se i due snapshot fossero uno, una modifica al legacy
/// mascherata da aggiornamento della tripla passerebbe in una diff sola.
///
/// I valori sono quelli precedenti a S8, byte per byte. Non vanno
/// riallineati a `native_read_mode`: la divergenza fra i due **è**
/// l'informazione che lo split esiste per esporre.
#[test]
fn il_read_mode_legacy_e_preservato_driver_per_driver() {
    const ATTESI: &[(&str, &str)] = &[
        ("csv", "streaming_sequential"),
        ("dxf", "streaming_sequential"),
        ("filegdb", "materializing"),
        ("geojson", "streaming_sequential"),
        ("geoparquet", "streaming_columnar"),
        ("gpkg", "streaming_sequential"),
        ("ipc", "streaming_sequential"),
        ("kml", "streaming_sequential"),
        ("shp", "streaming_sequential"),
        ("xls", "streaming_sequential"),
    ];
    let document = catalog_document_con(false);
    let drivers = document["drivers"].as_array().unwrap();
    assert_eq!(drivers.len(), ATTESI.len(), "driver aggiunti o rimossi");
    for (id, atteso) in ATTESI {
        let driver = drivers
            .iter()
            .find(|driver| driver["id"] == *id)
            .unwrap_or_else(|| panic!("{id} assente dal catalogo"));
        assert_eq!(
            driver["read_mode"].as_str(),
            Some(*atteso),
            "{id}: il read_mode legacy e' cambiato"
        );
    }
}

/// Snapshot della **tripla dichiarativa** di INV-7.
///
/// Le due colonne che non variano — `operation_atomic` e
/// `adaptive_memory_then_disk` — non sono ridondanti: sono ciò che
/// `BudgetedReader` impone a *tutti*, e un driver che ne dichiarasse altre
/// starebbe descrivendo un comportamento che l'adapter non gli lascia
/// avere. È il caso che questo snapshot prende.
#[test]
fn la_tripla_di_inv7_e_quella_dichiarata_da_ogni_driver() {
    const ATTESI: &[(&str, &str)] = &[
        ("csv", "streaming_sequential"),
        ("dxf", "materialized"),
        ("filegdb", "streaming_sequential"),
        ("geojson", "streaming_sequential"),
        ("geoparquet", "streaming_random"),
        ("gpkg", "streaming_random"),
        ("ipc", "streaming_random"),
        ("kml", "materialized"),
        ("shp", "streaming_sequential"),
        ("xls", "materialized"),
    ];
    let document = catalog_document_con(false);
    let drivers = document["drivers"].as_array().unwrap();
    assert_eq!(drivers.len(), ATTESI.len(), "driver aggiunti o rimossi");
    for (id, nativo) in ATTESI {
        let driver = drivers
            .iter()
            .find(|driver| driver["id"] == *id)
            .unwrap_or_else(|| panic!("{id} assente dal catalogo"));
        assert_eq!(
            driver["native_read_mode"].as_str(),
            Some(*nativo),
            "{id}: native_read_mode"
        );
        assert_eq!(
            driver["effective_delivery"].as_str(),
            Some("operation_atomic"),
            "{id}: l'adapter comune drena prima del primo batch, per tutti"
        );
        assert_eq!(
            driver["buffering"].as_str(),
            Some("adaptive_memory_then_disk"),
            "{id}: lo spool dell'adapter comune vale per tutti"
        );
    }
}

/// La tripla è **completa** e il legacy non è derivato da essa.
///
/// Due proprietà in un test perché sono la stessa affermazione vista da due
/// lati: i tre campi ci sono per ogni driver, e il quarto — il legacy —
/// **non** si ricava dai primi tre. Se qualcuno un giorno derivasse
/// `read_mode` da `native_read_mode`, i sette driver che oggi divergono
/// tornerebbero a coincidere e il campo tornerebbe a non dire niente:
/// esattamente il difetto L0.4 che INV-7 chiude.
#[test]
fn ogni_driver_dichiara_la_tripla_e_il_legacy_puo_divergere() {
    let document = catalog_document_con(false);
    let drivers = document["drivers"].as_array().unwrap();
    let mut divergenti = 0;
    for driver in drivers {
        let id = &driver["id"];
        for campo in ["native_read_mode", "effective_delivery", "buffering"] {
            assert!(
                driver[campo].is_string(),
                "{id}: {campo} assente o non stringa"
            );
        }
        // I due valori non sono lo stesso vocabolario — `materializing` non
        // è `materialized`, `streaming_columnar` non esiste fra i nativi —
        // quindi la divergenza si conta sui casi in cui *nemmeno*
        // l'intenzione coincide.
        let legacy = driver["read_mode"].as_str().unwrap();
        let nativo = driver["native_read_mode"].as_str().unwrap();
        if legacy != nativo {
            divergenti += 1;
        }
    }
    assert_eq!(
        divergenti, 7,
        "sette driver su dieci divergono fra legacy e nativo (dxf, filegdb, \
             geoparquet, gpkg, ipc, kml, xls): e' la ragione per cui lo split \
             esiste. Se questo numero cambia, o e' cambiato un driver o qualcuno \
             sta derivando il legacy dalla tripla"
    );
}

/// La matrice di handoff versionata è quella che il codice produce oggi.
///
/// Snapshot, non ispezione: il file in `docs/contracts/` è ciò che chi
/// mantiene `plenora-contracts` legge, e se divergesse dal codice senza che
/// nessuno se ne accorgesse sarebbe peggio che non averlo — una matrice
/// sbagliata si usa con la stessa fiducia di una giusta.
///
/// Il test **non** rigenera il file da solo: fallisce e mostra la
/// differenza, così l'aggiornamento resta una decisione di chi cambia il
/// contratto invece di un effetto collaterale di `cargo test`.
#[test]
fn la_matrice_di_handoff_e_aggiornata() {
    let percorso = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../docs/contracts/handoff-plenora-error.json");
    let atteso =
        serde_json::to_string_pretty(&matrice_di_handoff()).expect("la matrice si serializza");
    let versionato = std::fs::read_to_string(&percorso)
        .unwrap_or_else(|error| panic!("{}: {error}", percorso.display()));
    assert_eq!(
        // Il confronto normalizza i soli CR: il file e' scritto con LF, ma
        // un checkout su Windows puo' restituirlo con CRLF, e la matrice
        // non e' diversa per questo.
        versionato.replace('\r', "").trim(),
        atteso.trim(),
        "la matrice versionata non corrisponde al codice: rigenerala"
    );
}

/// Ogni codice del vocabolario compare nella matrice.
///
/// È la proprietà che rende l'elenco generato invece che copiato: una
/// variante nuova di `IoErrorCode` che nessuno aggiunge a `TUTTI` viene
/// presa qui, non da un lettore attento tre mesi dopo.
#[test]
fn il_vocabolario_dei_codici_e_completo() {
    use plenora_io_model::IoErrorCode;

    let matrice = matrice_di_handoff();
    let elencati = matrice["vocabolari"]["code"].as_array().unwrap().len();
    assert_eq!(
        elencati,
        IoErrorCode::TUTTI.len(),
        "la matrice elenca {elencati} codici, il tipo ne ha {}",
        IoErrorCode::TUTTI.len()
    );
    // E `TUTTI` copre davvero l'enum: un codice assente non serializzerebbe
    // mai, quindi si verifica che ogni voce sia una stringa distinta.
    let mut viste = std::collections::BTreeSet::new();
    for codice in matrice["vocabolari"]["code"].as_array().unwrap() {
        let nome = codice.as_str().expect("codice come stringa");
        assert!(viste.insert(nome), "codice duplicato: {nome}");
    }
}

#[test]
fn feature_on_catalog_fails_closed_when_runtime_probe_is_unavailable() {
    let document = catalog_document_con(false);
    let filegdb = document["drivers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|driver| driver["id"] == "filegdb")
        .unwrap();
    assert!(!filegdb["available"].as_bool().unwrap());
}

#[test]
fn catalog_is_canonical_and_byte_for_byte_deterministic() {
    let first = serde_json::to_vec(&cmd_catalog()).unwrap();
    let second = serde_json::to_vec(&cmd_catalog()).unwrap();
    assert_eq!(first, second);

    let document: Value = serde_json::from_slice(&first).unwrap();
    assert_candidate_envelope("catalog", &document);
    assert_eq!(document["determinism"], "byte_for_byte");
    let ids: Vec<_> = document["drivers"]
        .as_array()
        .unwrap()
        .iter()
        .map(|driver| driver["id"].as_str().unwrap())
        .collect();
    assert_eq!(
        ids,
        vec![
            "csv",
            "dxf",
            "filegdb",
            "geojson",
            "geoparquet",
            "gpkg",
            "ipc",
            "kml",
            "shp",
            "xls",
        ]
    );
}

#[test]
fn inspect_layers_and_read_match_the_candidate_protocol_manifest() {
    let directory = tempfile::tempdir().unwrap();
    let input = materialize_empty_ipc(&directory);
    let cli = parse(&[input.to_string_lossy().into_owned()]).unwrap();

    assert_candidate_envelope("inspect", &cmd_inspect(&cli).unwrap());
    assert_candidate_envelope("layers", &cmd_layers(&cli).unwrap());
    assert_candidate_envelope("read", &cmd_read(&cli).unwrap());
}

#[test]
fn legacy_xls_extension_reports_the_explicit_capability_drop() {
    let Err(error) = driver_for_path(Path::new("legacy.xls")) else {
        panic!(".xls non deve essere instradato")
    };
    assert_eq!(error.0, uscita_della_categoria(ErrorCategory::Unsupported));
    assert_eq!(error.1["error"]["code"], "XLS_BINARY_UNSUPPORTED");
    assert!(error.1["error"]["message"]
        .as_str()
        .unwrap()
        .contains("BIFF .xls"));
}

/// Ciò che il trasferimento di un layer fa, nell'ordine in cui lo fa.
///
/// La memoria misurata non distingue un trasferimento in streaming da uno
/// che accumula: un `Vec` di batch sta sotto qualunque soglia se il caso di
/// prova è piccolo, e sopra qualunque soglia se è grande. A distinguere le
/// due implementazioni è la **sequenza**, e questo è il tipo che la rende
/// osservabile.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Evento {
    Lettura,
    Dichiarazione(u64),
    Scrittura(usize),
}

#[derive(Clone, Default)]
struct Diario(std::rc::Rc<std::cell::RefCell<Vec<Evento>>>);

impl Diario {
    fn annota(&self, evento: Evento) {
        self.0.borrow_mut().push(evento);
    }

    fn eventi(&self) -> Vec<Evento> {
        self.0.borrow().clone()
    }
}

fn batch_di_prova(righe: usize) -> arrow_array::RecordBatch {
    let schema = std::sync::Arc::new(arrow_schema::Schema::new(vec![arrow_schema::Field::new(
        "n",
        arrow_schema::DataType::Int64,
        false,
    )]));
    let quante = i64::try_from(righe).expect("righe rappresentabili");
    let colonna = std::sync::Arc::new(arrow_array::Int64Array::from(
        (0..quante).collect::<Vec<_>>(),
    ));
    match arrow_array::RecordBatch::try_new(schema, vec![colonna]) {
        Ok(batch) => batch,
        Err(errore) => unreachable!("batch di prova: {errore:?}"),
    }
}

/// Reader che imita il contratto dell'adapter operation-atomic: il totale
/// diventa noto al **primo** `next_batch` e non prima.
struct ReaderStrumentato {
    contratto: LayerContract,
    residui: std::collections::VecDeque<arrow_array::RecordBatch>,
    totale: u64,
    esaminato: bool,
    /// Se falso il reader non dichiara mai il totale: è il caso che deve
    /// far fallire chiuso il trasferimento.
    dichiara: bool,
    diario: Diario,
}

impl ReaderStrumentato {
    fn nuovo(righe_per_batch: &[usize], dichiara: bool, diario: Diario) -> Self {
        let residui: std::collections::VecDeque<_> = righe_per_batch
            .iter()
            .map(|righe| batch_di_prova(*righe))
            .collect();
        let totale = righe_per_batch
            .iter()
            .map(|righe| u64::try_from(*righe).expect("righe rappresentabili"))
            .sum::<u64>();
        let schema = residui.front().map_or_else(
            || std::sync::Arc::new(arrow_schema::Schema::empty()),
            arrow_array::RecordBatch::schema,
        );
        Self {
            contratto: LayerContract {
                id: plenora_io_model::contract::LayerId(0),
                name: "prova".to_owned(),
                contract: DataContract {
                    schema,
                    geometry: None,
                },
            },
            residui,
            totale,
            esaminato: false,
            dichiara,
            diario,
        }
    }
}

impl plenora_io_core::driver::LayerReader for ReaderStrumentato {
    fn contract(&self) -> &LayerContract {
        &self.contratto
    }

    fn next_batch(&mut self) -> plenora_io_model::Result<Option<arrow_array::RecordBatch>> {
        self.diario.annota(Evento::Lettura);
        self.esaminato = true;
        Ok(self.residui.pop_front())
    }

    fn accepted_total(&self) -> Option<u64> {
        (self.dichiara && self.esaminato).then_some(self.totale)
    }
}

struct WriterStrumentato {
    diario: Diario,
}

impl plenora_io_core::driver::FormatWriter for WriterStrumentato {
    fn declare_input_total(
        &mut self,
        _layer: plenora_io_model::contract::LayerId,
        total: u64,
    ) -> plenora_io_model::Result<()> {
        self.diario.annota(Evento::Dichiarazione(total));
        Ok(())
    }

    fn write(&mut self, batch: &arrow_array::RecordBatch) -> plenora_io_model::Result<()> {
        self.diario.annota(Evento::Scrittura(batch.num_rows()));
        Ok(())
    }

    fn finish(self: Box<Self>) -> plenora_io_model::Result<plenora_io_core::Published> {
        Ok(plenora_io_core::Published {
            bytes: 0,
            loss: LossReport::default(),
            fidelity: FidelityAssessment::lossless(),
            outcome: PublishOutcome::Published,
        })
    }
}

/// La sonda che l'implementazione precedente non passa.
///
/// Il codice che accumulava in `layer_batches` produceva
/// `Lettura`×4 → `Dichiarazione` → `Scrittura`×3: tutte le letture prima di
/// ogni scrittura. Qui l'ordine atteso è scritto per intero, quindi la
/// differenza non dipende da quanto sia grande il caso di prova né da
/// quanta memoria consumi — che è precisamente ciò che una prova «input più
/// grande del budget» non garantisce, perché quel codice non passava dai
/// contatori e poteva restare verde.
#[test]
fn il_trasferimento_alterna_lettura_e_scrittura_un_batch_per_volta() {
    let diario = Diario::default();
    let mut reader = ReaderStrumentato::nuovo(&[2, 3, 5], true, diario.clone());
    let mut writer = WriterStrumentato {
        diario: diario.clone(),
    };

    let (righe, batches) = trasferisci_layer(
        &mut reader,
        &mut writer,
        plenora_io_model::contract::LayerId(0),
    )
    .expect("trasferimento riuscito");

    assert_eq!(righe, 10);
    assert_eq!(batches, 3);
    assert_eq!(
        diario.eventi(),
        vec![
            Evento::Lettura,
            Evento::Dichiarazione(10),
            Evento::Scrittura(2),
            Evento::Lettura,
            Evento::Scrittura(3),
            Evento::Lettura,
            Evento::Scrittura(5),
            Evento::Lettura,
        ]
    );
}

/// L'ordine atteso, detto come invariante e non come elenco.
///
/// L'elenco esatto della sonda precedente si rompe se cambia il numero dei
/// batch; questa dice la proprietà per cui quell'elenco è quello — a ogni
/// lettura, i batch già consegnati sono tutti già scritti — e resta vera per
/// qualunque partizione della sorgente, compresa quella con batch vuoti.
#[test]
fn nessuna_lettura_precede_la_scrittura_del_batch_gia_consegnato() {
    for partizione in [vec![1_usize], vec![1, 1], vec![4, 1, 1, 9], vec![0, 7, 0]] {
        let diario = Diario::default();
        let mut reader = ReaderStrumentato::nuovo(&partizione, true, diario.clone());
        let mut writer = WriterStrumentato {
            diario: diario.clone(),
        };
        trasferisci_layer(
            &mut reader,
            &mut writer,
            plenora_io_model::contract::LayerId(0),
        )
        .expect("trasferimento riuscito");

        let (mut letture, mut scritture) = (0_usize, 0_usize);
        for evento in diario.eventi() {
            match evento {
                Evento::Lettura => {
                    assert_eq!(
                            letture, scritture,
                            "lettura chiesta con {letture} batch consegnati e {scritture} scritti, partizione {partizione:?}"
                        );
                    letture += 1;
                }
                Evento::Scrittura(_) => scritture += 1,
                Evento::Dichiarazione(_) => {}
            }
        }
    }
}

/// Sorgente vuota: il totale è `Some(0)`, non «non lo so».
#[test]
fn una_sorgente_vuota_dichiara_zero_e_non_scrive() {
    let diario = Diario::default();
    let mut reader = ReaderStrumentato::nuovo(&[], true, diario.clone());
    let mut writer = WriterStrumentato {
        diario: diario.clone(),
    };

    let (righe, batches) = trasferisci_layer(
        &mut reader,
        &mut writer,
        plenora_io_model::contract::LayerId(0),
    )
    .expect("trasferimento riuscito");

    assert_eq!((righe, batches), (0, 0));
    assert_eq!(
        diario.eventi(),
        vec![Evento::Lettura, Evento::Dichiarazione(0)]
    );
}

/// Fail-closed: senza totale non si converte, e non si riaccumula.
#[test]
fn senza_cardinalita_accettata_il_trasferimento_fallisce_chiuso() {
    let diario = Diario::default();
    let mut reader = ReaderStrumentato::nuovo(&[2, 3], false, diario.clone());
    let mut writer = WriterStrumentato {
        diario: diario.clone(),
    };

    let errore = trasferisci_layer(
        &mut reader,
        &mut writer,
        plenora_io_model::contract::LayerId(0),
    )
    .expect_err("senza totale il trasferimento deve fallire");

    assert_eq!(errore.category, ErrorCategory::InvalidPlan);
    // Nessuna dichiarazione e nessuna scrittura: la sola lettura è quella
    // che conclude l'esame dello scope.
    assert_eq!(diario.eventi(), vec![Evento::Lettura]);
}
