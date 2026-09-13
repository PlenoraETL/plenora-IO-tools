//! Le prove di [`super`], in un file loro.
//!
//! Restano un **modulo figlio**: vedono i privati del genitore
//! esattamente come quando stavano dentro di lui, e non allargano di
//! una riga la superficie pubblica del crate.

use super::{analizza, componenti_usati, consumato_prima_del_rifiuto};
use plenora_io_model::contract::CoordinateDimensions;
use plenora_io_model::limits::WkbLimits;
use plenora_io_model::wkb::{encode_wkb, inspect_wkb, WkbFlavor, WkbValue};

fn stretti(componenti: usize, profondita: usize) -> WkbLimits {
    WkbLimits {
        max_components: componenti,
        max_depth: profondita,
        ..WkbLimits::default()
    }
}

/// **Il fatto che il lotto S12 esiste per stabilire.**
///
/// Un tetto applicato a valle avrebbe consumato tutto l'input prima di
/// dire di no: l'albero si costruisce per intero e poi lo si misura. Qui
/// il rifiuto arriva alla coordinata che supera il tetto, e la posizione
/// raggiunta lo dimostra.
#[test]
fn il_rifiuto_arriva_prima_della_fine_del_testo() {
    let mut testo = String::from("LINESTRING (");
    for indice in 0..5_000 {
        if indice > 0 {
            testo.push(',');
        }
        testo.push_str("1 2");
    }
    testo.push(')');

    let limiti = stretti(10, 64);
    assert!(analizza(&testo, &limiti).is_err());

    let consumato = consumato_prima_del_rifiuto(&testo, &limiti);
    assert!(
        consumato < testo.len() / 100,
        "consumati {consumato} byte su {}: il rifiuto non e' arrivato presto",
        testo.len()
    );
}

/// Lo stesso fatto, visto da fuori e senza guardare la posizione.
///
/// La coda e' sintatticamente impossibile. Se l'analisi arrivasse in fondo
/// prima di misurare, il rifiuto sarebbe quello di sintassi; siccome
/// misura mentre legge, la coda non viene nemmeno guardata.
#[test]
fn il_rifiuto_e_quello_del_tetto_non_quello_della_coda() {
    let mut testo = String::from("LINESTRING (1 2,3 4,5 6,7 8,9 10");
    testo.push_str(", questa coda non e' un WKT e non deve essere letta");
    let errore = analizza(&testo, &stretti(3, 64)).unwrap_err();
    let reso = format!("{errore}");
    assert!(
        reso.contains("limite") || reso.contains("componenti"),
        "atteso il rifiuto del tetto, ottenuto: {reso}"
    );
}

/// L'annidamento e' un parametro della discesa, non una misura finale.
#[test]
fn l_annidamento_oltre_il_tetto_e_rifiutato() {
    let profondo = |livelli: usize| {
        let mut testo = String::new();
        for _ in 0..livelli {
            testo.push_str("GEOMETRYCOLLECTION (");
        }
        testo.push_str("POINT (1 2)");
        for _ in 0..livelli {
            testo.push(')');
        }
        testo
    };
    let limiti = stretti(1_000, 4);
    assert!(
        analizza(&profondo(4), &limiti).is_ok(),
        "quattro livelli stanno nel tetto"
    );
    assert!(analizza(&profondo(6), &limiti).is_err());
}

/// Il tetto sui componenti, provato **esattamente al confine**.
///
/// E' la sonda che deve reggere il peso: il target di fuzzing non arriva a
/// centomila coordinate -- non stanno in un input da quattro kilobyte --
/// quindi sotto fuzzing quel ramo non e' esercitato, e i registri delle
/// misure lo dicono. Qui si prova al confine e non «da qualche parte
/// sopra»: `n` passa, `n+1` no, per ogni forma che conta i componenti in
/// un modo diverso.
#[test]
fn il_tetto_sui_componenti_e_esatto() {
    // (testo, componenti che costa). I costi sono quelli del bordo:
    // una coordinata ciascuna, piu' una per ogni geometria figlia.
    let casi: [(&str, usize); 6] = [
        ("POINT (1 2)", 1),
        ("LINESTRING (0 0,1 1,2 2)", 3),
        ("POLYGON ((0 0,1 0,1 1,0 0))", 4),
        ("MULTIPOINT (1 2,3 4)", 4),
        ("MULTILINESTRING ((0 0,1 1),(2 2,3 3))", 6),
        ("GEOMETRYCOLLECTION (POINT (1 2),LINESTRING (0 0,1 1))", 5),
    ];
    for (testo, costo) in casi {
        let esatto = WkbLimits {
            max_components: costo,
            ..WkbLimits::default()
        };
        let stretto = WkbLimits {
            max_components: costo - 1,
            ..WkbLimits::default()
        };
        assert!(
            analizza(testo, &esatto).is_ok(),
            "{testo} costa {costo} componenti e con {costo} deve passare"
        );
        let errore = analizza(testo, &stretto)
            .expect_err(&format!("{testo} con {} deve fallire", costo - 1));
        assert_eq!(
            errore.code,
            plenora_io_model::IoErrorCode::LimitExceeded,
            "{testo}: al confine il rifiuto e' del tetto"
        );
        // E il costo dichiarato e' quello che l'analisi addebita davvero.
        assert_eq!(componenti_usati(testo, &esatto), costo, "{testo}");
    }
}

/// L'unita' di conteggio e' quella del bordo, e non «una simile».
///
/// E' la lezione del lotto S11: due tetti con lo stesso nome e due unita'
/// di misura diverse sono peggio di due tetti con nomi diversi. La sonda
/// non confronta il codice, confronta i **conteggi**: cio' che l'analisi
/// addebita leggendo il testo deve essere cio' che `inspect_wkb` conta
/// sulla stessa geometria in WKB.
#[test]
fn i_componenti_coincidono_con_quelli_del_parser_condiviso() {
    let campioni = [
        "POINT (1 2)",
        "LINESTRING (0 0,1 1,2 2)",
        "POLYGON ((0 0,1 0,1 1,0 0))",
        "POLYGON ((0 0,1 0,1 1,0 0),(0 0,1 0,1 1,0 0))",
        "MULTIPOINT (1 2,3 4)",
        "MULTIPOINT ((1 2),(3 4))",
        "MULTILINESTRING ((0 0,1 1),(2 2,3 3))",
        "MULTIPOLYGON (((0 0,1 0,1 1,0 0)))",
        "GEOMETRYCOLLECTION (POINT (1 2),LINESTRING (0 0,1 1))",
        "GEOMETRYCOLLECTION (GEOMETRYCOLLECTION (POINT (1 2)))",
        "POINT Z (1 2 3)",
        "LINESTRING ZM (0 0 0 0,1 1 1 1)",
    ];
    let limiti = WkbLimits::default();
    for testo in campioni {
        let nostri = componenti_usati(testo, &limiti);
        let geometria = analizza(testo, &limiti).expect("campione valido");
        let byte = encode_wkb(&geometria, WkbFlavor::Iso).expect("codificabile");
        let ispezione = inspect_wkb(&byte, &limiti).expect("ispezionabile");
        assert_eq!(
            nostri, ispezione.components,
            "{testo}: l'analisi addebita {nostri}, il parser condiviso conta {}",
            ispezione.components
        );
    }
}

/// La grammatica accettata e' quella di prima, e questa sonda la fissa.
///
/// Il lotto sposta *quando* si rifiuta, non *che cosa* si accetta: se
/// avesse allargato o stretto l'insieme, un file che smette di funzionare
/// non si saprebbe imputare a quale delle due cose.
#[test]
fn la_grammatica_accettata_e_quella_dichiarata() {
    let limiti = WkbLimits::default();
    let casi: [(&str, CoordinateDimensions); 8] = [
        ("POINT (1 2)", CoordinateDimensions::Xy),
        ("POINT (1 2 3)", CoordinateDimensions::Xyz),
        ("POINT Z (1 2 3)", CoordinateDimensions::Xyz),
        ("POINT M (1 2 3)", CoordinateDimensions::Xym),
        ("POINT ZM (1 2 3 4)", CoordinateDimensions::Xyzm),
        ("point (1 2)", CoordinateDimensions::Xy),
        ("  POINT   (  1   2  )  ", CoordinateDimensions::Xy),
        ("LINESTRING EMPTY", CoordinateDimensions::Xy),
    ];
    for (testo, attese) in casi {
        let geometria = analizza(testo, &limiti)
            .unwrap_or_else(|errore| panic!("{testo} doveva essere accettato: {errore}"));
        assert_eq!(geometria.dimensions, attese, "{testo}");
    }

    // Le due sintassi di MULTIPOINT sono lo stesso oggetto.
    assert_eq!(
        analizza("MULTIPOINT (1 2,3 4)", &limiti).unwrap(),
        analizza("MULTIPOINT ((1 2),(3 4))", &limiti).unwrap()
    );

    // I vuoti che il core WKB rappresenta.
    for testo in [
        "MULTIPOINT EMPTY",
        "MULTILINESTRING EMPTY",
        "MULTIPOLYGON EMPTY",
        "GEOMETRYCOLLECTION EMPTY",
        "POLYGON EMPTY",
    ] {
        let geometria = analizza(testo, &limiti)
            .unwrap_or_else(|errore| panic!("{testo} doveva essere accettato: {errore}"));
        let vuota = match &geometria.value {
            WkbValue::MultiPoint(figli)
            | WkbValue::MultiLineString(figli)
            | WkbValue::MultiPolygon(figli)
            | WkbValue::GeometryCollection(figli) => figli.is_empty(),
            WkbValue::Polygon(anelli) => anelli.is_empty(),
            _ => false,
        };
        assert!(vuota, "{testo}");
    }
}

/// I rifiuti che c'erano prima restano, con lo stesso significato.
#[test]
fn i_rifiuti_strutturali_restano() {
    let limiti = WkbLimits::default();
    let rifiutati = [
        ("POINT EMPTY", "il core WKB non ha un punto vuoto"),
        ("MULTIPOINT (EMPTY)", "ne' un punto vuoto annidato"),
        (
            "POINT (1 2 3 4 5)",
            "cinque ordinate non sono una coordinata",
        ),
        ("POINT Z (1 2)", "il tag dichiara tre ordinate"),
        (
            "LINESTRING (0 0,1 1 1)",
            "coordinate di dimensionalita' diversa",
        ),
        (
            "GEOMETRYCOLLECTION (POINT (1 2),POINT Z (1 2 3))",
            "figli di dimensionalita' diversa",
        ),
        ("POINT (1 2)POINT (3 4)", "testo residuo dopo la geometria"),
        (
            "CIRCULARSTRING (0 0,1 1,2 2)",
            "tipo fuori dall'insieme accettato",
        ),
        ("POINT (1 nan)", "coordinata non numerica"),
        ("POINT", "corpo assente"),
        ("", "testo vuoto"),
    ];
    for (testo, perche) in rifiutati {
        assert!(analizza(testo, &limiti).is_err(), "{testo}: {perche}");
    }
}

/// **La prova che la grammatica non e' cambiata.**
///
/// Sostituire un parser rompe l'insieme accettato senza rompere un test:
/// l'insieme e' molto piu' grande del corpus che lo descrive, e «il
/// workspace e' verde» dice soltanto che i due parser accettano le cose
/// che qualcuno ha gia' scritto.
///
/// Qui il confronto e' con il parser **precedente** -- la crate `wkt` piu'
/// il suo adattatore, conservati per questo -- su un corpus generato per
/// combinazione: sette tipi, quattro dimensionalita', le forme vuote, le
/// due sintassi di `MULTIPOINT`, gli spazi, le maiuscole, e una quarantina
/// di storpiature.
///
/// La regola del confronto ha tre righe e la terza e' quella che conta:
///
/// * accettano entrambi -> la geometria deve essere **la stessa**;
/// * rifiutano entrambi -> niente da dire;
/// * il vecchio accetta e il nuovo no -> ammesso **solo** se il nuovo
///   rifiuto e' un tetto, che e' cio' che il lotto aggiunge;
/// * il vecchio rifiuta e il nuovo accetta -> mai. Accettare piu' di prima
///   e' una regressione silenziosa, ed e' la direzione che nessun test
///   esistente avrebbe visto.
#[test]
fn accetta_esattamente_cio_che_accettava_il_parser_precedente() {
    let limiti = WkbLimits::default();
    let corpus = corpus_di_confronto();
    assert!(
        corpus.len() > 200,
        "corpus troppo piccolo: {}",
        corpus.len()
    );

    let mut concordi = 0_usize;
    let mut solo_per_i_tetti = 0_usize;
    let mut piu_stretti = 0_usize;
    for testo in &corpus {
        let prima = super::adattatore_storico::analizza_come_prima(testo);
        let dopo = crate::wkt_lossless::parse_wkt_bounded(testo, &limiti);
        match (prima, dopo) {
            (Ok(prima), Ok(dopo)) => {
                assert_eq!(prima, dopo, "geometrie diverse per «{testo}»");
                concordi += 1;
            }
            (Err(_), Err(_)) => concordi += 1,
            (Ok(_), Err(errore)) => {
                let per_un_tetto = errore.code == plenora_io_model::IoErrorCode::LimitExceeded;
                assert!(
                    per_un_tetto || per_il_testo_residuo(testo),
                    "«{testo}» era accettato e ora e' rifiutato senza che sia un tetto: {errore}"
                );
                if per_un_tetto {
                    solo_per_i_tetti += 1;
                } else {
                    piu_stretti += 1;
                }
            }
            (Err(errore), Ok(_)) => {
                panic!("«{testo}» era rifiutato ({errore}) e ora e' accettato");
            }
        }
    }
    assert_eq!(
        piu_stretti, 3,
        "le divergenze piu' strette sono tre e sono elencate: se cambiano, \
             la decisione sul testo residuo va ripresa"
    );
    assert_eq!(
        concordi + solo_per_i_tetti + piu_stretti,
        corpus.len(),
        "ogni caso deve ricadere in una delle righe della regola"
    );
}

/// I tre casi in cui l'analisi progressiva e' **piu' stretta**, elencati.
///
/// # La decisione, presa
///
/// La crate `wkt` ignorava cio' che segue la geometria: `POINT (1 2))` e
/// `POINT (1 2) POINT (3 4)` per lei erano un punto, e il resto non c'era.
/// L'analisi progressiva li rifiuta, e il rifiuto **non** viene da un
/// tetto: e' l'unico irrigidimento del lotto, ed e' deliberato -- una
/// cella WKT rappresenta una geometria completa, e ignorare il resto
/// nasconde un input malformato.
///
/// I tre casi restano qui **per nome**, e il loro numero e' asserito: non
/// perche' la decisione sia sospesa, ma perche' l'eccezione resti chiusa.
/// Se ne comparisse un quarto, questa sonda diventerebbe rossa invece di
/// allargarla in silenzio.
fn per_il_testo_residuo(testo: &str) -> bool {
    [
        "POINT (1 2))",
        "POINT (1 2) POINT (3 4)",
        include_str!("../../../../fuzz/seeds/wkt_parse/multipolygon-con-membro-vuoto.wkt"),
    ]
    .contains(&testo)
}

/// Il corpus del confronto, generato per combinazione.
///
/// Generato e non scritto a mano: un elenco scritto a mano contiene i casi
/// a cui si e' pensato, che sono esattamente quelli che i test esistenti
/// gia' coprono. La combinazione produce anche quelli a cui non si e'
/// pensato -- ed e' li' che una riscrittura sbaglia.
// Il corpus e' lungo perche' e' un elenco di casi, non una funzione con
// logica: dividerlo in tre pezzi renderebbe piu' difficile leggere che cosa
// copre, che e' la sola cosa che conta qui.
#[allow(clippy::too_many_lines)]
fn corpus_di_confronto() -> Vec<String> {
    let dimensioni = ["", " Z", " M", " ZM"];
    let coordinate = ["1 2", "1 2 3", "1 2 3", "1 2 3 4"];
    let mut corpus = Vec::new();

    for (indice, suffisso) in dimensioni.iter().enumerate() {
        let uno = coordinate[indice];
        let due = format!("{uno},{uno}");
        let quattro = format!("{uno},{uno},{uno},{uno}");
        let forme = [
                format!("POINT{suffisso} ({uno})"),
                format!("POINT{suffisso} EMPTY"),
                format!("LINESTRING{suffisso} ({due})"),
                format!("LINESTRING{suffisso} EMPTY"),
                format!("POLYGON{suffisso} (({quattro}))"),
                format!("POLYGON{suffisso} (({quattro}),({quattro}))"),
                format!("POLYGON{suffisso} EMPTY"),
                format!("POLYGON{suffisso} (EMPTY)"),
                format!("MULTIPOINT{suffisso} ({due})"),
                format!("MULTIPOINT{suffisso} (({uno}),({uno}))"),
                format!("MULTIPOINT{suffisso} EMPTY"),
                format!("MULTIPOINT{suffisso} (EMPTY)"),
                format!("MULTILINESTRING{suffisso} (({due}),({due}))"),
                format!("MULTILINESTRING{suffisso} EMPTY"),
                format!("MULTILINESTRING{suffisso} (EMPTY)"),
                format!("MULTIPOLYGON{suffisso} ((({quattro})))"),
                format!("MULTIPOLYGON{suffisso} EMPTY"),
                format!("MULTIPOLYGON{suffisso} (EMPTY)"),
                format!("GEOMETRYCOLLECTION{suffisso} (POINT{suffisso} ({uno}))"),
                format!(
                    "GEOMETRYCOLLECTION{suffisso} (POINT{suffisso} ({uno}),LINESTRING{suffisso} ({due}))"
                ),
                format!("GEOMETRYCOLLECTION{suffisso} EMPTY"),
                format!(
                    "GEOMETRYCOLLECTION{suffisso} (GEOMETRYCOLLECTION{suffisso} (POINT{suffisso} ({uno})))"
                ),
            ];
        for forma in forme {
            // Ogni forma in quattro vesti: com'e', minuscola, con spazi
            // dentro le parentesi, e senza lo spazio prima della parentesi.
            corpus.push(forma.clone());
            corpus.push(forma.to_lowercase());
            corpus.push(forma.replace('(', "(  ").replace(')', "  )"));
            corpus.push(forma.replace(" (", "("));
        }
    }

    // Le parentesi vuote, una per tipo. Mancavano, e la loro assenza ha
    // lasciato passare una regressione nella direzione vietata: il parser
    // nuovo accettava `MULTIPOINT ()` dove il precedente lo rifiutava. La
    // forma vuota e' `EMPTY`; `()` non e' WKT, e l'ha trovato la
    // diagnostica differenziale del livello 2, non questa sonda.
    for tipo in [
        "POINT",
        "LINESTRING",
        "POLYGON",
        "MULTIPOINT",
        "MULTILINESTRING",
        "MULTIPOLYGON",
        "GEOMETRYCOLLECTION",
    ] {
        corpus.push(format!("{tipo} ()"));
        corpus.push(format!("{tipo} (  )"));
        corpus.push(format!("{tipo}()"));
    }

    // Le storpiature: quelle che un file vero produce sbagliando, e quelle
    // che un input ostile produce apposta.
    for storpiatura in [
        "",
        " ",
        "POINT",
        "POINT (",
        "POINT ()",
        "POINT (1)",
        "POINT (1 2",
        "POINT 1 2)",
        "POINT (1 2))",
        "POINT (1 2) POINT (3 4)",
        "POINT (1 2 3 4 5)",
        "POINT (a b)",
        "POINT (1 due)",
        "POINT (1 2,3 4)",
        "POINT Z (1 2)",
        "POINT M (1 2)",
        "POINT ZM (1 2 3)",
        "LINESTRING (1 2)",
        "LINESTRING (,)",
        "LINESTRING (1 2,)",
        "LINESTRING (1 2,3 4 5)",
        "POLYGON (1 2,3 4)",
        "POLYGON ((1 2,3 4)",
        "MULTIPOINT (1 2,(3 4))",
        "MULTIPOINT ((1 2),3 4)",
        "MULTIPOLYGON (((1 2,3 4)),((5 6",
        "GEOMETRYCOLLECTION (POINT (1 2)",
        "GEOMETRYCOLLECTION (GEOMETRYCOLLECTION EMPTY)",
        "GEOMETRYCOLLECTION (POINT (1 2),POINT Z (1 2 3))",
        "CIRCULARSTRING (0 0,1 1,2 2)",
        "TRIANGLE ((0 0,1 0,1 1,0 0))",
        "TIN (((0 0,1 0,1 1,0 0)))",
        "POINT EMPTY EMPTY",
        "EMPTY",
        "POINT (1e2 -3E-1)",
        "POINT (+1 -2)",
        "POINT (1. .2)",
        "POINT (1..2 3)",
        "POINT (1 2)\n",
        "\tPOINT (1 2)",
        "POINT\n(1 2)",
        "PoInT (1 2)",
        "POINTZ (1 2 3)",
        "POINT Z(1 2 3)",
        "POINT  ZM  (1 2 3 4)",
        "POINT (1 2 nan)",
        "POINT (inf 2)",
        "MULTIPOINT (EMPTY,EMPTY)",
        "GEOMETRYCOLLECTION (EMPTY)",
    ] {
        corpus.push(storpiatura.to_owned());
    }

    // I semi versionati del target: sono il corpus che il fuzzer ha gia'
    // trovato interessante, ed e' il posto dove le divergenze si nascondono.
    for seme in [
        include_str!("../../../../fuzz/seeds/wkt_parse/polygon-con-anello-vuoto.wkt"),
        include_str!("../../../../fuzz/seeds/wkt_parse/multipolygon-con-membro-vuoto.wkt"),
    ] {
        corpus.push(seme.to_owned());
    }
    corpus
}

/// L'unico irrigidimento del lotto, provato nei due versi.
///
/// Lo spazio finale non e' testo e resta accettato; tutto il resto no. E'
/// un errore di **sintassi**, non di budget: dire «limite superato» a chi
/// ha una parentesi di troppo lo manderebbe ad allargare una quota che non
/// c'entra.
#[test]
fn la_coda_non_vuota_e_rifiutata_come_sintassi() {
    let limiti = WkbLimits::default();

    // Lo spazio finale, in tutte le sue forme, e' ammesso.
    for coda in ["", " ", "   ", "\t", "\n", "\r\n", " \t\r\n "] {
        let testo = format!("POINT (1 2){coda}");
        assert!(
            analizza(&testo, &limiti).is_ok(),
            "lo spazio finale non e' testo residuo: {testo:?}"
        );
    }

    for testo in [
        "POINT (1 2))",
        "POINT (1 2) POINT (3 4)",
        "POINT (1 2),",
        "POINT (1 2) 3",
        "LINESTRING (0 0,1 1) )",
        "GEOMETRYCOLLECTION (POINT (1 2)) EMPTY",
    ] {
        let errore = analizza(testo, &limiti).expect_err(&format!("{testo} deve essere rifiutato"));
        assert_eq!(
            errore.code,
            plenora_io_model::IoErrorCode::Wkb,
            "{testo}: il testo residuo e' un errore di sintassi, non di budget"
        );
    }
}

/// Cio' che i writer producono resta leggibile.
///
/// E' la meta' che l'irrigidimento potrebbe rompere senza farsi vedere: se
/// `format_wkt` emettesse uno spazio, una parentesi o un a capo di troppo,
/// il round-trip fallirebbe -- e fallirebbe in produzione, non qui.
#[test]
fn cio_che_scriviamo_resta_rileggibile() {
    let limiti = WkbLimits::default();
    for testo in [
        "POINT (1 2)",
        "POINT Z (1 2 3)",
        "POINT ZM (1 2 3 4)",
        "LINESTRING (0 0,1 1,2 2)",
        "LINESTRING M (0 0 5,1 1 6)",
        "POLYGON ((0 0,1 0,1 1,0 0))",
        "POLYGON ((0 0,1 0,1 1,0 0),(0 0,1 0,1 1,0 0))",
        "MULTIPOINT (1 2,3 4)",
        "MULTILINESTRING ((0 0,1 1),(2 2,3 3))",
        "MULTIPOLYGON (((0 0,1 0,1 1,0 0)))",
        "GEOMETRYCOLLECTION (POINT (1 2),LINESTRING (0 0,1 1))",
        "MULTIPOINT EMPTY",
        "GEOMETRYCOLLECTION EMPTY",
    ] {
        let geometria =
            analizza(testo, &limiti).unwrap_or_else(|errore| panic!("{testo}: {errore}"));
        let scritto = crate::wkt_lossless::format_wkt(&geometria)
            .unwrap_or_else(|errore| panic!("{testo}: {errore}"));
        let riletto = analizza(&scritto, &limiti).unwrap_or_else(|errore| {
            panic!("{testo} scritto come {scritto} non e' rileggibile: {errore}")
        });
        assert_eq!(geometria, riletto, "round-trip di {testo}");
    }
}

/// Una parola piu' lunga del vocabolario non alloca.
///
/// Il buffer delle parole e' fisso: un input che ne dichiarasse una da
/// megabyte otterrebbe un rifiuto, non la memoria che chiede.
#[test]
fn una_parola_smisurata_non_diventa_memoria() {
    let testo = "A".repeat(4_000_000);
    assert!(analizza(&testo, &WkbLimits::default()).is_err());
}
