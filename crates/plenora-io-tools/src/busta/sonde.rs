//! Le prove di [`super`], in un file loro.
//!
//! Restano un **modulo figlio**: vedono i privati del genitore
//! esattamente come quando stavano dentro di lui, e non allargano di
//! una riga la superficie pubblica del crate.

use super::*;
use plenora_io_model::ErrorCategory;
// Il tetto sul dettaglio lo applica la porta, in core: le sonde lo
// prendono da li' invece che dall'adattatore, che non lo nomina piu'.
use plenora_io_core::loss::{
    Posizione, MAX_BYTE_DETTAGLIO, MAX_ESEMPI_TRATTENUTI, MAX_RAGIONI_TRATTENUTE,
};

fn rapporto_con(categorie: usize, byte_per_id: usize) -> LossReport {
    let mut rapporto = LossReport::default();
    for i in 0..categorie {
        // Identificatori distinti e tutti della lunghezza voluta: il
        // prefisso numerico li rende diversi senza cambiarne i byte.
        let prefisso = format!("{i:04}_");
        let riempimento = "a".repeat(byte_per_id.saturating_sub(prefisso.len()));
        rapporto.record(&format!("{prefisso}{riempimento}"), u64::MAX);
    }
    rapporto
}

#[test]
fn il_caso_peggiore_dichiarato_entra_nei_dodici_kib() {
    // La promessa del contratto: sessantaquattro categorie da centoventotto
    // byte stanno in una sezione. Non si verifica sulla somma delle
    // lunghezze -- quella e' 8 KiB e direbbe di si' sbagliando -- ma sui
    // byte JSON che escono davvero, virgolette, chiavi e conteggi compresi.
    let rapporto = rapporto_con(MAX_CATEGORIE, MAX_BYTE_ID_CATEGORIA);
    let (documento, troncamento) =
        sezione_di_perdita(&rapporto, BYTE_PER_SEZIONE).expect("il budget basta");
    assert_eq!(
        troncamento,
        Troncamento::default(),
        "il caso peggiore dichiarato non deve troncare niente"
    );
    let byte = serde_json::to_string(&documento)
        .expect("serializzabile")
        .len();
    assert!(
        byte <= BYTE_PER_SEZIONE,
        "il caso peggiore occupa {byte} byte su {BYTE_PER_SEZIONE}"
    );
    assert_eq!(
        documento["counts"].as_array().map(Vec::len),
        Some(MAX_CATEGORIE)
    );
}

#[test]
fn la_sessantacinquesima_categoria_resta_fuori_ed_e_dichiarata() {
    let rapporto = rapporto_con(MAX_CATEGORIE + 1, 16);
    let (documento, troncamento) =
        sezione_di_perdita(&rapporto, BYTE_PER_SEZIONE).expect("il budget basta");
    assert_eq!(troncamento.categorie_omesse, 1);
    assert_eq!(
        troncamento.omesse_per_byte, 0,
        "non e' il budget ad averla tolta"
    );
    assert_eq!(documento["troncato"], json!(true));
    assert_eq!(
        documento["counts"].as_array().map(Vec::len),
        Some(MAX_CATEGORIE)
    );
}

#[test]
fn i_conteggi_pubblicati_restano_esatti() {
    // Si omette una voce intera, mai si riscrive un conteggio: un `12` al
    // posto di `300` sarebbe un valore che significa due cose.
    let mut rapporto = LossReport::default();
    rapporto.record("una", 300);
    let (documento, _) = sezione_di_perdita(&rapporto, BYTE_PER_SEZIONE).expect("budget");
    assert_eq!(documento["counts"][0]["conteggio"], json!(300_u64));
}

#[test]
fn il_taglio_non_dipende_dall_ordine_di_inserimento() {
    // Stesso insieme, ordini d'inserimento opposti, stesso documento: il
    // troncamento segue l'ordine canonico degli identificatori, non quello
    // in cui i driver hanno registrato le perdite.
    let mut avanti = LossReport::default();
    let mut indietro = LossReport::default();
    let identificatori: Vec<String> = (0..MAX_CATEGORIE + 10)
        .map(|i| format!("{i:04}_categoria"))
        .collect();
    for id in &identificatori {
        avanti.record(id, 1);
    }
    for id in identificatori.iter().rev() {
        indietro.record(id, 1);
    }
    let (uno, primo) = sezione_di_perdita(&avanti, BYTE_PER_SEZIONE).expect("budget");
    let (due, secondo) = sezione_di_perdita(&indietro, BYTE_PER_SEZIONE).expect("budget");
    assert_eq!(
        uno, due,
        "l'ordine d'inserimento non deve cambiare l'uscita"
    );
    assert_eq!(primo, secondo);
}

#[test]
fn il_budget_esaurito_e_una_causa_a_parte() {
    // Poche categorie, budget stretto: nessuna soglia di cardinalita' e'
    // superata, eppure qualcosa resta fuori. Le due cause non si mescolano.
    let rapporto = rapporto_con(8, 64);
    let (documento, troncamento) =
        sezione_di_perdita(&rapporto, 512).expect("la struttura minima entra");
    assert_eq!(troncamento.categorie_omesse, 0);
    assert!(troncamento.omesse_per_byte > 0);
    assert_eq!(documento["troncato"], json!(true));
    let byte = serde_json::to_string(&documento)
        .expect("serializzabile")
        .len();
    assert!(byte <= 512, "{byte} byte oltre il budget di 512");
}

#[test]
fn senza_spazio_per_la_dichiarazione_si_fallisce_chiusi() {
    // La sola cosa peggiore di una diagnostica troncata e' una troncata che
    // tace: se la dichiarazione non entra, non esce un documento.
    let rapporto = rapporto_con(1, 16);
    assert_eq!(
        sezione_di_perdita(&rapporto, 8),
        Err(BudgetInsufficiente),
        "otto byte non bastano nemmeno alla struttura minima"
    );
}

/// Una valutazione con le ragioni offerte nell'ordine dato.
fn valutazione_con(ordine: &[FidelityReason]) -> FidelityAssessment {
    let mut valutazione = FidelityAssessment::con_livello(Fidelity::Approximating);
    for ragione in ordine {
        valutazione.add_reason_redatta(
            ragione.code,
            ragione.detail.clone(),
            ragione.posizione,
            ragione.detail_v1(),
        );
    }
    valutazione
}

/// `n` ragioni distinte, discriminate dalla posizione e non dal testo.
fn ragioni_distinte(quante: u64) -> Vec<FidelityReason> {
    (0..quante)
        .map(|i| {
            FidelityReason::redatta(
                FidelityReasonCode::AttributeLoss,
                "l'attributo non e' nativo",
                Posizione {
                    layer_index: Some(0),
                    field_index: Some(i),
                    type_class: None,
                },
                format!("layer «uno»: attributo 'campo{i}' non nativo"),
            )
        })
        .collect()
}

/// Un esempio con posizione e contesto dati.
fn esempio(campo: u64, contesto: &str) -> LossExample {
    LossExample {
        category: "coercion tipo attributo".to_owned(),
        posizione: Posizione {
            layer_index: Some(0),
            field_index: Some(campo),
            type_class: None,
        },
        context: contesto.to_owned(),
    }
}

fn rapporto_con_esempi(esempi: &[LossExample]) -> LossReport {
    let mut rapporto = LossReport::default();
    rapporto.record("coercion tipo attributo", 1);
    for e in esempi {
        rapporto.add_example(e.clone());
    }
    rapporto
}

#[test]
fn un_esempio_fuori_misura_non_sfratta_un_esempio_valido() {
    // Stesso difetto delle ragioni, e stessa correzione: il filtro sta alla
    // porta, quindi le fuori misura non occupano il trattenimento.
    // Le fuori misura occupano le posizioni **canonicamente minori**, e le
    // valide quelle dopo. E' cio' che rende la sonda discriminante: con la
    // strategia difettosa -- trattieni prima, filtra dopo -- le
    // duecentocinquantasei minori riempirebbero il trattenimento e le
    // valide, essendo maggiori, verrebbero sfrattate. Con le valide alle
    // posizioni basse la sonda passerebbe anche col difetto, perche'
    // sopravvivrebbero comunque.
    let quante_fuori = u64::try_from(MAX_ESEMPI_TRATTENUTI).unwrap();
    let fuori: Vec<_> = (0..quante_fuori)
        .map(|i| esempio(i, &"x".repeat(MAX_BYTE_DETTAGLIO + 1)))
        .collect();
    let validi: Vec<_> = (quante_fuori..quante_fuori + u64::try_from(MAX_LOSS_EXAMPLES).unwrap())
        .map(|i| esempio(i, "il tipo dell'attributo richiede una coercizione"))
        .collect();

    for (nome, ordine) in [
        (
            "prima i fuori misura",
            [fuori.clone(), validi.clone()].concat(),
        ),
        ("prima i validi", [validi, fuori].concat()),
    ] {
        let (sezione, troncamento) =
            sezione_di_perdita(&rapporto_con_esempi(&ordine), BYTE_PER_SEZIONE).expect("budget");
        assert_eq!(
            sezione["esempi"].as_array().map(Vec::len),
            Some(MAX_LOSS_EXAMPLES),
            "{nome}: gli ammissibili devono uscire tutti"
        );
        assert_eq!(troncamento.esempi_omessi, 0, "{nome}");
        assert!(troncamento.omesse_per_byte > 0, "{nome}");
        assert_eq!(sezione["omesse_esatte"], json!(false), "{nome}");
    }
}

#[test]
fn una_categoria_fuori_misura_non_entra_nemmeno_in_un_esempio() {
    // Il tetto sull'identificatore vale **ovunque compaia**: limitarlo in
    // `counts` e non negli esempi vorrebbe dire che i dodici KiB li decide
    // la meta' senza tetto.
    let mut lungo = esempio(0, "contesto breve");
    lungo.category = "c".repeat(MAX_BYTE_ID_CATEGORIA + 1);
    let (sezione, troncamento) =
        sezione_di_perdita(&rapporto_con_esempi(&[lungo]), BYTE_PER_SEZIONE).expect("budget");
    assert_eq!(sezione["esempi"].as_array().map(Vec::len), Some(0));
    assert!(troncamento.omesse_per_byte > 0);
    assert_eq!(sezione["omesse_esatte"], json!(false));
}

#[test]
fn la_fusione_degli_esempi_e_componibile() {
    // `merge` concatenava e troncava, quindi fondere A con B dava un
    // risultato diverso da fondere B con A: la diagnostica cambiava a
    // seconda di come gli adattatori erano stati composti.
    let a: Vec<_> = (0..40).map(|i| esempio(i, "primo insieme")).collect();
    let b: Vec<_> = (30..80).map(|i| esempio(i, "secondo insieme")).collect();

    let mut ab = rapporto_con_esempi(&a);
    ab.merge(&rapporto_con_esempi(&b));
    let mut ba = rapporto_con_esempi(&b);
    ba.merge(&rapporto_con_esempi(&a));

    let (sezione_ab, tr_ab) = sezione_di_perdita(&ab, BYTE_PER_SEZIONE).expect("budget");
    let (sezione_ba, tr_ba) = sezione_di_perdita(&ba, BYTE_PER_SEZIONE).expect("budget");
    assert_eq!(
        sezione_ab, sezione_ba,
        "l'unione non deve dipendere dall'ordine"
    );
    assert_eq!(tr_ab, tr_ba);
}

#[test]
fn la_forma_di_un_esempio_coincide_col_derive() {
    // Come per le ragioni: l'adattatore resta scritto a mano e autorevole,
    // e la sonda impedisce al derive di divergere da lui.
    let vuoto = LossExample {
        category: "coercion tipo attributo".to_owned(),
        posizione: Posizione::default(),
        context: "con \"virgolette\" e accènti".to_owned(),
    };
    let pieno = LossExample {
        category: "coercion tipo attributo".to_owned(),
        posizione: Posizione {
            layer_index: Some(2),
            field_index: Some(5),
            type_class: Some(ArrowTypeClass::Temporal),
        },
        context: "il tipo dell'attributo richiede una coercizione".to_owned(),
    };
    for esempio in [&vuoto, &pieno] {
        assert_eq!(
            documento_dell_esempio(esempio),
            serde_json::to_value(esempio).expect("serializzabile"),
            "adattatore e derive divergono su {esempio:?}"
        );
    }
}

#[test]
fn la_sezione_v2_non_dipende_dall_ordine_di_inserimento() {
    // Il difetto che il trattenimento canonico toglie: la
    // sessantacinquesima veniva scartata **prima** che l'adattatore
    // ordinasse, quindi l'insieme pubblicato dipendeva da quali adattatori
    // fossero stati composti e in che ordine -- cioe' da qualcosa che ne'
    // chi fornisce il file ne' chi lo legge controlla.
    let avanti = ragioni_distinte(u64::try_from(MAX_FIDELITY_REASONS).unwrap() + 1);
    let indietro: Vec<_> = avanti.iter().rev().cloned().collect();

    let (v2_avanti, tr_avanti) =
        sezione_di_fedelta(&valutazione_con(&avanti), BYTE_PER_SEZIONE).expect("budget");
    let (v2_indietro, tr_indietro) =
        sezione_di_fedelta(&valutazione_con(&indietro), BYTE_PER_SEZIONE).expect("budget");

    assert_eq!(
        v2_avanti, v2_indietro,
        "due ordini di inserimento devono dare la stessa sezione v2"
    );
    assert_eq!(tr_avanti, tr_indietro);
    assert_eq!(
        tr_avanti.ragioni_omesse, 1,
        "sessantacinque distinte, sessantaquattro pubblicate"
    );
    assert_eq!(v2_avanti["omesse_esatte"], json!(true));

    // Qui c'era il controcanto: il v1 **doveva** differire fra i due
    // ordini, perche' pubblicava i primi sessantaquattro per inserimento.
    // Era una proprieta' del protocollo congelato, e con la 4.0.0 quel
    // protocollo non esiste piu' nell'artefatto. Cio' che l'asserzione
    // proteggeva -- che l'indipendenza dall'ordine fosse una scelta del v2
    // e non un caso -- resta detto dal confronto qui sopra, che pretende
    // l'uguaglianza invece di osservarla.
}

#[test]
fn una_voce_fuori_misura_non_sfratta_una_voce_valida() {
    // Con il filtro nell'adattatore invece che alla porta, le fuori misura
    // occupavano un posto nel trattenimento e le valide restavano fuori:
    // duecentocinquantasei canonicamente minori ma oltre il tetto
    // avrebbero fatto pubblicare **zero** ragioni.
    let fuori: Vec<_> = (0..u64::try_from(MAX_RAGIONI_TRATTENUTE).unwrap())
        .map(|i| {
            FidelityReason::redatta(
                FidelityReasonCode::AssessmentPending, // il codice minore: canonicamente prima
                "x".repeat(MAX_BYTE_DETTAGLIO + 1),
                Posizione {
                    layer_index: Some(0),
                    field_index: Some(i),
                    type_class: None,
                },
                "irrilevante",
            )
        })
        .collect();
    let valide = ragioni_distinte(u64::try_from(MAX_FIDELITY_REASONS).unwrap());

    for (nome, ordine) in [
        (
            "prima le fuori misura",
            [fuori.clone(), valide.clone()].concat(),
        ),
        ("prima le valide", [valide, fuori].concat()),
    ] {
        let (sezione, troncamento) =
            sezione_di_fedelta(&valutazione_con(&ordine), BYTE_PER_SEZIONE).expect("budget");
        assert_eq!(
            sezione["reasons"].as_array().map(Vec::len),
            Some(MAX_FIDELITY_REASONS),
            "{nome}: le ammissibili devono uscire tutte"
        );
        assert_eq!(troncamento.ragioni_omesse, 0, "{nome}");
        assert!(troncamento.omesse_per_byte > 0, "{nome}");
        assert_eq!(
            sezione["omesse_esatte"],
            json!(false),
            "{nome}: una voce respinta rende i contatori limiti inferiori"
        );
    }
}

/// `n` ragioni distinte e **fuori misura**, per saturare le respinte.
fn ragioni_fuori_misura(quante: u64) -> Vec<FidelityReason> {
    (0..quante)
        .map(|i| {
            FidelityReason::redatta(
                FidelityReasonCode::AttributeLoss,
                "x".repeat(MAX_BYTE_DETTAGLIO + 1),
                Posizione {
                    layer_index: Some(0),
                    field_index: Some(i),
                    type_class: None,
                },
                "irrilevante",
            )
        })
        .collect()
}

#[test]
fn anche_l_insieme_delle_respinte_e_limitato() {
    // Le respinte non conservano la stringa che le ha fatte respingere, ma
    // conservano una chiave: senza tetto, quella sarebbe una quota di
    // memoria decisa da chi fornisce il file -- cioe' il difetto che tutto
    // il lotto toglie, rientrato dalla porta di servizio.
    let valutazione = valutazione_con(&ragioni_fuori_misura(
        u64::try_from(MAX_RAGIONI_TRATTENUTE).unwrap() + 1,
    ));
    assert_eq!(valutazione.ragioni_trattenute(), 0);
    assert_eq!(
        valutazione.respinte_per_misura(),
        MAX_RAGIONI_TRATTENUTE as u64,
        "le respinte si fermano al proprio tetto"
    );
    assert!(!valutazione.omesse_esatte());
}

#[test]
fn anche_gli_esempi_respinti_sono_limitati() {
    let quanti = u64::try_from(MAX_ESEMPI_TRATTENUTI).unwrap() + 1;
    let fuori: Vec<_> = (0..quanti)
        .map(|i| esempio(i, &"x".repeat(MAX_BYTE_DETTAGLIO + 1)))
        .collect();
    let rapporto = rapporto_con_esempi(&fuori);
    assert_eq!(rapporto.esempi_trattenuti(), 0);
    assert_eq!(rapporto.respinti_per_misura(), MAX_ESEMPI_TRATTENUTI as u64);
    assert!(!rapporto.omesse_esatte());
}

#[test]
fn la_fusione_porta_con_se_le_respinte_e_le_limita() {
    // Fondere due valutazioni che hanno **entrambe** respinto: le chiavi si
    // uniscono, e l'unione resta sotto il proprio tetto. Se la fusione le
    // perdesse, `omesse_per_byte` scenderebbe fondendo -- cioe' la
    // diagnostica direbbe che si e' perso meno perche' si e' composto di
    // piu'.
    let meta = u64::try_from(MAX_RAGIONI_TRATTENUTE).unwrap();
    let mut prima = valutazione_con(&ragioni_fuori_misura(meta));
    let seconda = valutazione_con(
        &ragioni_fuori_misura(meta * 2)
            .into_iter()
            .skip(usize::try_from(meta).unwrap())
            .collect::<Vec<_>>(),
    );
    assert_eq!(prima.respinte_per_misura(), meta);
    prima.merge(&seconda);
    assert_eq!(
        prima.respinte_per_misura(),
        meta,
        "l'unione delle respinte resta sotto il tetto"
    );
    assert!(!prima.omesse_esatte());

    let mut rapporto = rapporto_con_esempi(
        &(0..meta)
            .map(|i| esempio(i, &"x".repeat(MAX_BYTE_DETTAGLIO + 1)))
            .collect::<Vec<_>>(),
    );
    let altro = rapporto_con_esempi(
        &(meta..meta * 2)
            .map(|i| esempio(i, &"x".repeat(MAX_BYTE_DETTAGLIO + 1)))
            .collect::<Vec<_>>(),
    );
    rapporto.merge(&altro);
    assert_eq!(rapporto.respinti_per_misura(), meta);
    assert!(!rapporto.omesse_esatte());
}

#[test]
fn l_ordine_canonico_e_una_relazione_e_non_solo_un_taglio() {
    // `Ord` decide dove la sezione taglia, e `PartialOrd`/`PartialEq` sono
    // la stessa relazione vista dagli operatori. Le collezioni usano
    // `cmp`, quindi senza questa sonda gli altri due esisterebbero solo
    // per soddisfare la gerarchia dei trait, mai eseguiti.
    let ragioni = ragioni_distinte(2);
    let (prima, seconda) = (&ragioni[0], &ragioni[1]);
    assert!(prima < seconda, "l'indice di campo minore viene prima");
    assert!(seconda > prima);
    assert_eq!(prima.partial_cmp(seconda), Some(std::cmp::Ordering::Less));
    assert_eq!(prima, &prima.clone());
    assert_ne!(prima, seconda);
}

#[test]
fn l_ordine_canonico_degli_esempi_e_quello_dei_tre_campi() {
    // La sonda gemella della precedente, e nasce con la stessa lacuna: gli
    // esempi **derivavano** `Ord`, quindi `eq` e `partial_cmp` non erano
    // righe di sorgente e non c'era niente da non coprire. Ora passano da
    // `chiave()` -- perche' il gate del protocollo legge l'ordine da li'
    // invece che dall'ordine di dichiarazione dei campi -- e sono codice
    // che qualcuno deve eseguire.
    //
    // La priorita' e' l'affermazione che conta: `category`, poi
    // `posizione`, poi `context`. E' cio' che il derive dava, ed e' cio'
    // che il contratto dichiara in `determinismo.ordine_canonico.esempi`:
    // scriverla a mano senza provarla avrebbe potuto cambiare il punto in
    // cui la sezione taglia senza che nulla lo dicesse.
    let alfa = LossExample {
        category: "alfa".to_owned(),
        posizione: Posizione {
            layer_index: Some(9),
            field_index: Some(9),
            type_class: None,
        },
        context: "zeta".to_owned(),
    };
    let beta = LossExample {
        category: "beta".to_owned(),
        posizione: Posizione::default(),
        context: "alfa".to_owned(),
    };
    assert!(
        alfa < beta,
        "la categoria decide per prima, anche contro posizione e contesto maggiori"
    );

    // A parita' di categoria decide la posizione, e non il contesto.
    let vicino = esempio(0, "zeta");
    let lontano = esempio(9, "alfa");
    assert!(vicino < lontano, "la posizione decide prima del contesto");

    // A parita' di categoria e posizione resta il contesto.
    assert!(esempio(3, "alfa") < esempio(3, "beta"));

    // Gli operatori sono la stessa relazione di `cmp`, che e' quella che le
    // collezioni usano: senza questa parte `partial_cmp` e `eq`
    // esisterebbero solo per soddisfare la gerarchia dei trait.
    assert!(lontano > vicino);
    assert_eq!(vicino.partial_cmp(&lontano), Some(std::cmp::Ordering::Less));
    assert_eq!(vicino, vicino.clone());
    assert_ne!(vicino, lontano);
}

#[test]
fn il_trattenimento_sfratta_la_maggiore_e_lo_dichiara() {
    // La meccanica centrale del trattenimento, e fino a questa sonda
    // nessuna la eseguiva: le altre si fermano al tetto, e lo sfratto parte
    // alla voce successiva. «Sfrattare la maggiore» invece di «rifiutare le
    // successive» e' cio' che rende il contenuto indipendente dall'ordine
    // di inserimento, quindi e' la proprieta' che merita di essere provata.
    let distinte = u64::try_from(MAX_RAGIONI_TRATTENUTE).unwrap() + 1;
    let tutte = ragioni_distinte(distinte);
    let avanti = valutazione_con(&tutte);
    let indietro = valutazione_con(&tutte.iter().rev().cloned().collect::<Vec<_>>());

    assert_eq!(avanti.ragioni_trattenute(), MAX_RAGIONI_TRATTENUTE);
    assert!(
        !avanti.omesse_esatte(),
        "un trattenimento saturo rende i contatori limiti inferiori"
    );

    // La sfrattata e' la **maggiore**, non l'ultima arrivata: le due
    // valutazioni trattengono lo stesso insieme pur avendo ricevuto le
    // voci in ordine opposto.
    let a: Vec<_> = avanti.ragioni_canoniche().collect();
    let b: Vec<_> = indietro.ragioni_canoniche().collect();
    assert_eq!(a, b, "lo sfratto non deve dipendere dall'ordine di arrivo");
    let ultima = tutte.last().expect("almeno una");
    assert!(
        !a.contains(&ultima),
        "la maggiore per chiave canonica deve essere quella sfrattata"
    );

    let (sezione, _) = sezione_di_fedelta(&avanti, BYTE_PER_SEZIONE).expect("budget");
    assert_eq!(sezione["omesse_esatte"], json!(false));
}

#[test]
fn il_trattenimento_degli_esempi_sfratta_il_maggiore_e_lo_dichiara() {
    let quanti = u64::try_from(MAX_ESEMPI_TRATTENUTI).unwrap() + 1;
    let tutti: Vec<_> = (0..quanti)
        .map(|i| esempio(i, "il tipo dell'attributo richiede una coercizione"))
        .collect();
    let rapporto = rapporto_con_esempi(&tutti);
    assert_eq!(rapporto.esempi_trattenuti(), MAX_ESEMPI_TRATTENUTI);
    assert!(!rapporto.omesse_esatte());

    let (sezione, troncamento) = sezione_di_perdita(&rapporto, BYTE_PER_SEZIONE).expect("budget");
    assert_eq!(sezione["omesse_esatte"], json!(false));
    assert_eq!(
        troncamento.esempi_omessi,
        (MAX_ESEMPI_TRATTENUTI - MAX_LOSS_EXAMPLES) as u64
    );
}

#[test]
fn la_fusione_non_supera_il_trattenimento() {
    // `merge` puo' portare l'insieme oltre il tetto, e li' lo sfratto deve
    // valere come alla porta: due meta' ciascuna sotto il tetto sommano a
    // qualcosa che lo supera.
    let meta = u64::try_from(MAX_RAGIONI_TRATTENUTE).unwrap();
    let prima = valutazione_con(&ragioni_distinte(meta));
    let seconda = valutazione_con(
        &(meta..meta * 2)
            .map(|i| ragioni_distinte(i + 1)[usize::try_from(i).unwrap()].clone())
            .collect::<Vec<_>>(),
    );
    let mut fusa = prima;
    fusa.merge(&seconda);
    assert_eq!(fusa.ragioni_trattenute(), MAX_RAGIONI_TRATTENUTE);
    assert!(!fusa.omesse_esatte());
}

#[test]
fn la_fusione_degli_esempi_non_supera_il_trattenimento() {
    let meta = u64::try_from(MAX_ESEMPI_TRATTENUTI).unwrap();
    let prima: Vec<_> = (0..meta).map(|i| esempio(i, "primo")).collect();
    let seconda: Vec<_> = (meta..meta * 2).map(|i| esempio(i, "secondo")).collect();
    let mut fuso = rapporto_con_esempi(&prima);
    fuso.merge(&rapporto_con_esempi(&seconda));
    assert_eq!(fuso.esempi_trattenuti(), MAX_ESEMPI_TRATTENUTI);
    assert!(!fuso.omesse_esatte());
}

#[test]
fn la_stessa_ragione_offerta_molte_volte_e_una_sola_omissione() {
    // Le offerte duplicate sono **deduplicate**, non occorrenze: una
    // ragione e' un fatto, e le occorrenze hanno la loro sede in `counts`.
    // Contare le offerte legherebbe i contatori a quante volte un driver
    // ha chiamato, cioe' a come sono stati composti gli adattatori.
    let una = &ragioni_distinte(1)[0];
    let ripetuta: Vec<_> = std::iter::repeat_n(una.clone(), MAX_FIDELITY_REASONS + 1).collect();
    let (sezione, troncamento) =
        sezione_di_fedelta(&valutazione_con(&ripetuta), BYTE_PER_SEZIONE).expect("budget");
    assert_eq!(sezione["reasons"].as_array().map(Vec::len), Some(1));
    assert_eq!(troncamento.ragioni_omesse, 0);
    assert_eq!(sezione["omesse_esatte"], json!(true));
}

#[test]
fn il_v1_deduplica_sulla_chiave_vecchia_e_il_v2_sulla_canonica() {
    // Due ragioni che il v2 distingue -- posizioni diverse -- e che il v1
    // considera la stessa, perche' la sua frase congelata coincide. Se il
    // v1 deduplicasse sull'`Eq` del v2, il protocollo congelato dipenderebbe
    // dall'identita' nuova, che e' esattamente cio' che non deve accadere.
    let a = FidelityReason::redatta(
        FidelityReasonCode::AttributeLoss,
        "l'attributo non e' nativo",
        Posizione {
            layer_index: Some(0),
            field_index: Some(1),
            type_class: None,
        },
        "layer «uno»: attributo non nativo",
    );
    let b = FidelityReason::redatta(
        FidelityReasonCode::AttributeLoss,
        "l'attributo non e' nativo",
        Posizione {
            layer_index: Some(0),
            field_index: Some(2),
            type_class: None,
        },
        "layer «uno»: attributo non nativo",
    );
    let valutazione = valutazione_con(&[a, b]);
    let (v2, _) = sezione_di_fedelta(&valutazione, BYTE_PER_SEZIONE).expect("budget");
    assert_eq!(
        v2["reasons"].as_array().map(Vec::len),
        Some(2),
        "gli indici distinguono cio' che i nomi distinguevano"
    );
    // Il confronto col v1 -- che ne deduplicava una sola, sulla frase --
    // misurava la differenza fra i due protocolli. Con un protocollo solo
    // non c'e' piu' una differenza da misurare, e cio' che conta e'
    // l'asserzione qui sopra: due ragioni che differiscono per **posizione**
    // restano due, e non una.
}

#[test]
fn le_ragioni_entrano_in_ordine_canonico_e_il_resto_e_dichiarato() {
    let mut valutazione = FidelityAssessment::con_livello(Fidelity::Approximating);
    for i in 0..MAX_FIDELITY_REASONS {
        valutazione.add_reason(FidelityReasonCode::AttributeLoss, format!("{i:04}"));
    }
    let (documento, troncamento) =
        sezione_di_fedelta(&valutazione, BYTE_PER_SEZIONE).expect("budget");
    assert_eq!(troncamento, Troncamento::default());
    let ragioni = documento["reasons"].as_array().expect("elenco");
    assert_eq!(ragioni.len(), MAX_FIDELITY_REASONS);
    let dettagli: Vec<&str> = ragioni
        .iter()
        .filter_map(|r| r["detail"].as_str())
        .collect();
    let mut ordinati = dettagli.clone();
    ordinati.sort_unstable();
    assert_eq!(dettagli, ordinati, "le ragioni escono in ordine canonico");
}

#[test]
fn una_sezione_di_fedelta_senza_spazio_fallisce_chiusa() {
    let valutazione = FidelityAssessment::con_livello(Fidelity::Lossless);
    assert_eq!(
        sezione_di_fedelta(&valutazione, 8),
        Err(BudgetInsufficiente)
    );
}

#[test]
fn un_identificatore_di_centoventinove_byte_resta_fuori() {
    // 128 byte entrano, 129 no. Il tetto e' sull'identificatore e non sullo
    // spazio: quella categoria resterebbe fuori anche in una sezione vuota.
    for (byte, atteso_fuori) in [(MAX_BYTE_ID_CATEGORIA, 0), (MAX_BYTE_ID_CATEGORIA + 1, 1)] {
        let mut rapporto = LossReport::default();
        rapporto.record(&"a".repeat(byte), 1);
        let (documento, troncamento) =
            sezione_di_perdita(&rapporto, BYTE_PER_SEZIONE).expect("budget");
        assert_eq!(
            troncamento.omesse_per_byte, atteso_fuori,
            "{byte} byte: {documento}"
        );
        assert_eq!(troncamento.categorie_omesse, 0, "non e' la cardinalita'");
    }
}

#[test]
fn un_identificatore_unicode_si_misura_in_byte() {
    // 64 «à» sono 128 byte e passano; 65 sono 130 e no, pur restando 65
    // caratteri. La voce esce **intera**: il taglio non spezza un carattere.
    for (caratteri, atteso_fuori) in [(64_usize, 0_u64), (65, 1)] {
        let identificatore = "à".repeat(caratteri);
        assert_eq!(identificatore.len(), caratteri * 2);
        let mut rapporto = LossReport::default();
        rapporto.record(&identificatore, 1);
        let (_, troncamento) = sezione_di_perdita(&rapporto, BYTE_PER_SEZIONE).expect("budget");
        assert_eq!(
            troncamento.omesse_per_byte,
            atteso_fuori,
            "{caratteri} caratteri = {} byte",
            identificatore.len()
        );
    }
}

#[test]
fn un_dettaglio_di_cinquecentotredici_byte_resta_fuori() {
    for (byte, dentro) in [(MAX_BYTE_DETTAGLIO, 1_usize), (MAX_BYTE_DETTAGLIO + 1, 0)] {
        let mut valutazione = FidelityAssessment::con_livello(Fidelity::Approximating);
        valutazione.add_reason(FidelityReasonCode::AttributeLoss, "x".repeat(byte));
        let (documento, troncamento) =
            sezione_di_fedelta(&valutazione, BYTE_PER_SEZIONE).expect("budget");
        assert_eq!(
            documento["reasons"].as_array().map(Vec::len),
            Some(dentro),
            "{byte} byte di dettaglio"
        );
        assert_eq!(troncamento.omesse_per_byte, 1 - dentro as u64);
    }
}

#[test]
fn il_documento_finale_sta_nel_budget_anche_dichiarando_il_troncamento() {
    // La dichiarazione fa parte della sezione, e la sua dimensione dipende
    // da **quanto** si e' tolto: riservare lo spazio dei contatori a zero e
    // poi scriverci numeri veri faceva uscire il documento oltre il budget
    // su cui era stato deciso il taglio.
    //
    // Non un budget scelto a mano: se ne provano molti, perche' il difetto
    // si vede solo dove la sezione finisce esattamente al bordo, e quel
    // punto dipende da quanto e' lunga la dichiarazione.
    let rapporto = rapporto_con(MAX_CATEGORIE + 500, 96);
    for budget in (300..4_000).step_by(37) {
        let Ok((documento, troncamento)) = sezione_di_perdita(&rapporto, budget) else {
            continue;
        };
        let byte = documento.to_string().len();
        assert!(
            byte <= budget,
            "budget {budget}: il documento finale ne occupa {byte}, e dichiara {troncamento:?}"
        );
        assert_eq!(
            documento["omesse"],
            troncamento.documento(),
            "la dichiarazione emessa deve essere quella calcolata"
        );
    }
}

#[test]
fn il_tetto_complessivo_e_verificato_sull_insieme() {
    // I nomi sono quelli veri della busta: il totale misura la
    // serializzazione **finale**, chiavi comprese, non la somma nominale
    // delle cinque quote -- quella direbbe sempre 61 440 e non guarderebbe
    // mai un byte di cio' che esce.
    const NOMI: [&str; SEZIONI] = [
        "read_fidelity",
        "write_fidelity",
        "conversion_fidelity",
        "read_loss",
        "write_loss",
    ];

    // Cinque sezioni ciascuna dentro i propri dodici KiB non dicono niente
    // sull'aggregato: e' la ragione per cui questo controllo esiste. Con le
    // cinque piene si sta dentro; con una sesta no -- e la sesta non esiste,
    // ma il controllo deve accorgersene se un giorno esistesse.
    let rapporto = rapporto_con(MAX_CATEGORIE, MAX_BYTE_ID_CATEGORIA);
    let (sezione, _) = sezione_di_perdita(&rapporto, BYTE_PER_SEZIONE).expect("budget");
    let cinque: Vec<(&str, &Value)> = NOMI.iter().map(|n| (*n, &sezione)).collect();
    let totale = diagnostica_entro_il_totale(&cinque).expect("cinque sezioni piene stanno");
    assert!(
        totale <= MAX_BYTE_BUSTA,
        "{totale} byte oltre il tetto di {MAX_BYTE_BUSTA}"
    );
    assert!(
        totale > SEZIONI * BYTE_PER_SEZIONE / 2,
        "il totale deve misurare qualcosa di reale, non un numero simbolico: {totale}"
    );

    let mut sei = cinque;
    sei.push(("una_sesta", &sezione));
    assert_eq!(
        diagnostica_entro_il_totale(&sei),
        Err(BudgetInsufficiente),
        "una sezione in piu' deve far fallire il totale, non passare in silenzio"
    );
}

#[test]
fn la_forma_scritta_a_mano_coincide_col_derive() {
    // La forma sul filo e' scritta a mano perche' i nomi sono contratto e
    // non devono seguire i nomi Rust per distrazione. Ma due scritture
    // divergono, e divergono in silenzio: questa sonda le lega.
    for livello in [
        Fidelity::Lossless,
        Fidelity::Conditional,
        Fidelity::Approximating,
    ] {
        assert_eq!(
            documento_del_livello(livello),
            serde_json::to_value(livello).expect("un enum semplice si serializza"),
            "{livello:?}"
        );
    }
    for codice in [
        FidelityReasonCode::AssessmentPending,
        FidelityReasonCode::FormatConstraint,
        FidelityReasonCode::GeometryApproximation,
        FidelityReasonCode::StructureChanged,
        FidelityReasonCode::AttributeLoss,
        FidelityReasonCode::TypeCoercion,
        FidelityReasonCode::PrecisionChanged,
        FidelityReasonCode::NullabilityChanged,
        FidelityReasonCode::NativeMetadataLoss,
        FidelityReasonCode::LossReported,
    ] {
        assert_eq!(
            documento_del_codice(codice),
            serde_json::to_value(codice).expect("un enum semplice si serializza"),
            "{codice:?}"
        );
    }
    // La forma scritta a mano e quella del derive devono **coincidere**, e
    // nei due casi: posizione vuota e posizione piena. E' cio' che
    // impedisce al derive di divergere dall'adattatore -- un campo aggiunto
    // al tipo senza `skip` e senza toccare l'adattatore diventa rosso qui --
    // e insieme prova che `dettaglio_v1` non esce dalla serializzazione.
    let vuota = FidelityReason::redatta(
        FidelityReasonCode::AttributeLoss,
        "con \"virgolette\" e accènti",
        Posizione::default(),
        "layer «segreto»: attributo 'riservato' non nativo",
    );
    let piena = FidelityReason::redatta(
        FidelityReasonCode::TypeCoercion,
        "il tipo dell'attributo richiede una coercizione",
        Posizione {
            layer_index: Some(3),
            field_index: Some(7),
            type_class: Some(ArrowTypeClass::Decimal),
        },
        "layer «segreto»: tipo Decimal128(38, 9) di 'riservato' richiede coercion",
    );
    for ragione in [&vuota, &piena] {
        let a_mano = documento_della_ragione_v2(ragione);
        assert_eq!(
            a_mano,
            serde_json::to_value(ragione).expect("serializzabile"),
            "adattatore e derive divergono su {ragione:?}"
        );
        let testo = a_mano.to_string();
        assert!(
            !testo.contains("segreto") && !testo.contains("riservato"),
            "il materiale riservato al v1 e' uscito dal v2: {testo}"
        );
    }
    assert_eq!(
        documento_della_ragione_v2(&piena)["type_class"],
        serde_json::json!("decimal"),
        "la classe di tipo esce col nome del filo"
    );
}

#[test]
fn i_tetti_dichiarati_sommano_al_tetto_della_busta() {
    // I numeri del contratto, verificati fra loro invece che ripetuti a
    // mano: 12 KiB per cinque sezioni piu' 4 KiB di struttura fanno 64 KiB.
    assert_eq!(
        SEZIONI * BYTE_PER_SEZIONE + BYTE_DELLA_STRUTTURA,
        MAX_BYTE_BUSTA
    );
    assert_eq!(MAX_BYTE_BUSTA, 64 * 1024);
}

/// Una busta d'errore che sta nel tetto non viene toccata.
///
/// E' il caso di **ogni** errore che il prodotto emette davvero: il
/// messaggio e' curato e `row_diagnostics` ha un tetto di 64 esempi, quindi
/// si sta nell'ordine dei chilobyte contro i cinquecento ammessi. Senza
/// questa sonda, le tre qui sotto proverebbero che la riduzione funziona
/// senza provare che non scatti quando non serve.
#[test]
fn una_busta_nel_tetto_resta_identica() {
    let busta = busta_d_errore_con_esempi(4);
    let (uscita, riduzione) = entro_il_tetto_dell_errore(busta.clone(), RISERVA_IDENTITA);
    assert_eq!(riduzione, RiduzioneDellErrore::Nessuna);
    assert_eq!(uscita, busta, "una busta che ci sta non si tocca");
}

/// Il confine: un byte sotto passa, un byte sopra riduce.
///
/// Si costruisce il messaggio alla lunghezza esatta invece di cercarla per
/// tentativi, perche' un confine provato «circa» non e' un confine.
#[test]
fn il_confine_del_tetto_e_esatto() {
    for (scarto, atteso) in [
        (0_i64, RiduzioneDellErrore::Nessuna),
        (-1, RiduzioneDellErrore::Nessuna),
        // Un byte sopra: basta togliere l'esempio, che e' la prima
        // riduzione. Avevo scritto `DiagnosticaDiRiga`, e la prova mi ha
        // corretto: si toglie il meno possibile, e un byte e' il meno.
        (1, RiduzioneDellErrore::EsempiDiRiga),
    ] {
        let busta = busta_alla_misura(MAX_BYTE_ERRORE, scarto);
        let misurata = serde_json::to_vec(&busta).expect("si serializza").len();
        assert_eq!(
            i64::try_from(misurata).expect("misura rappresentabile"),
            i64::try_from(MAX_BYTE_ERRORE).expect("tetto rappresentabile") + scarto,
            "la busta di prova deve avere la misura voluta"
        );
        let (_, riduzione) = entro_il_tetto_dell_errore(busta, 0);
        assert_eq!(
            riduzione, atteso,
            "scarto {scarto} dal tetto: riduzione attesa {atteso:?}"
        );
    }
}

/// La prima riduzione toglie gli esempi e lo dichiara.
///
/// `counts` e `observed_total` restano: sono cio' su cui una macchina
/// decide, e gli esempi sono illustrazioni. Una busta accorciata in
/// silenzio direbbe che la diagnostica non c'era, mentre la verita' e' che
/// non ci stava -- e quella si puo' chiedere di nuovo con un'invocazione
/// piu' stretta.
#[test]
fn oltre_il_tetto_cadono_prima_gli_esempi() {
    // Tanti esempi da sforare, ma con `counts` piccolo: togliendo gli
    // esempi si rientra.
    let busta = busta_d_errore_con_esempi(8_000);
    let (uscita, riduzione) = entro_il_tetto_dell_errore(busta, 0);

    assert_eq!(riduzione, RiduzioneDellErrore::EsempiDiRiga);
    let diagnostica = &uscita["error"]["row_diagnostics"];
    assert_eq!(
        diagnostica["examples"].as_array().map(Vec::len),
        Some(0),
        "gli esempi sono stati tolti"
    );
    assert_eq!(
        diagnostica["examples_truncated"], true,
        "e il documento lo dichiara"
    );
    assert!(
        diagnostica["counts"].is_object() || diagnostica["counts"].is_array(),
        "cio' su cui si decide resta: {diagnostica}"
    );
    assert_eq!(
        uscita["error"]["code"], "DIAGNOSTICS_TRUNCATED",
        "il codice dichiara la riduzione"
    );
    assert!(
        serde_json::to_vec(&uscita).expect("si serializza").len() <= MAX_BYTE_ERRORE,
        "e la busta ridotta sta nel tetto"
    );
}

/// Quando nemmeno togliere gli esempi basta, cade l'intera diagnostica.
#[test]
fn poi_cade_l_intera_diagnostica_di_riga() {
    let mut busta = busta_d_errore_con_esempi(1);
    // `counts` enorme: gli esempi non bastano a rientrare.
    let mut conteggi = serde_json::Map::new();
    for indice in 0..40_000 {
        conteggi.insert(format!("finto.causa_{indice}"), json!(1));
    }
    busta["error"]["row_diagnostics"]["counts"] = Value::Object(conteggi);

    let (uscita, riduzione) = entro_il_tetto_dell_errore(busta, 0);
    assert_eq!(riduzione, RiduzioneDellErrore::DiagnosticaDiRiga);
    assert!(
        uscita["error"].get("row_diagnostics").is_none(),
        "la diagnostica e' facoltativa per contratto, e cade per seconda"
    );
    for asse in ["category", "phase", "remote_effect", "retry"] {
        assert!(uscita["error"].get(asse).is_some(), "l'asse {asse} resta");
    }
    assert_eq!(uscita["error"]["code"], "DIAGNOSTICS_TRUNCATED");
}

/// E se non basta nemmeno quello, restano i quattro assi.
///
/// E' il caso che nessun percorso del prodotto puo' raggiungere -- il
/// messaggio e' curato e non porta testo della sorgente -- e si costruisce
/// a mano perche' una guardia che non scatta mai va comunque provata,
/// altrimenti si scopre che non funziona il giorno in cui serve.
#[test]
fn all_ultimo_restano_i_quattro_assi() {
    let mut busta = busta_d_errore_con_esempi(1);
    busta["error"]["message"] = json!("x".repeat(MAX_BYTE_ERRORE + 1_000));

    let (uscita, riduzione) = entro_il_tetto_dell_errore(busta, 0);
    assert_eq!(riduzione, RiduzioneDellErrore::SoloGliAssi);
    assert_eq!(uscita["error"]["code"], "ERROR_TRUNCATED");
    for asse in ["category", "phase", "remote_effect", "retry"] {
        assert!(
            uscita["error"].get(asse).is_some(),
            "l'asse {asse} non si toglie: e' cio' su cui una macchina deve \
                 poter decidere, e toglierlo renderebbe l'errore illeggibile \
                 invece che piu' corto"
        );
    }
    assert!(serde_json::to_vec(&uscita).expect("si serializza").len() <= MAX_BYTE_ERRORE);
}

/// La riduzione e' idempotente: applicarla due volte non cambia nulla.
///
/// Serve perche' il prodotto la applica **due volte** -- in `err_doc` con
/// la riserva, e su la busta completa nel binding -- e due passaggi che
/// dessero risultati diversi farebbero divergere le due superfici.
#[test]
fn la_riduzione_e_idempotente() {
    let busta = busta_d_errore_con_esempi(8_000);
    let (una_volta, _) = entro_il_tetto_dell_errore(busta, 0);
    let (due_volte, riduzione) = entro_il_tetto_dell_errore(una_volta.clone(), 0);
    assert_eq!(riduzione, RiduzioneDellErrore::Nessuna);
    assert_eq!(una_volta, due_volte);
}

/// La riduzione non tocca cio' su cui una macchina decide.
///
/// # Le due proprieta', e perche' stanno insieme
///
/// La prima: i cinque campi che `error-v1.schema.json` dichiara
/// obbligatori -- `category`, `phase`, `remote_effect`, `retry`, `message`
/// -- restano a ogni livello di riduzione. Una busta che per stare nel
/// tetto perdesse un campo obbligatorio sarebbe fuori contratto, ed e' il
/// modo piu' facile di sbagliare una riduzione: si toglie dal fondo finche'
/// ci sta.
///
/// La seconda: il codice d'uscita si deriva dalla **categoria**, e la
/// coppia (codice, documento) si compone in `local_err_doc` con la
/// categoria d'origine mentre il documento passa dalla riduzione. Se la
/// riduzione cambiasse `category`, il processo uscirebbe con un numero che
/// la busta contraddice -- due superfici che raccontano due esiti diversi
/// dello stesso errore, ed e' esattamente la classe che CLI-2.0 §8 chiude.
/// `retry` sta nello stesso gruppo: e' l'altro campo su cui un chiamante
/// decide senza leggere il testo.
///
/// Il `code` invece **cambia**, ed e' voluto: dichiara la riduzione. Qui si
/// pretende solo che resti una grafia valida per il contratto.
#[test]
fn la_riduzione_conserva_gli_assi_e_l_uscita() {
    // Un caso per livello di riduzione, cosi' che la proprieta' sia provata
    // dove la riduzione taglia di piu' e non solo dove non taglia.
    let casi = [
        (busta_d_errore_con_esempi(1), RiduzioneDellErrore::Nessuna),
        (
            busta_d_errore_con_esempi(20_000),
            RiduzioneDellErrore::EsempiDiRiga,
        ),
        (
            busta_d_errore_con_esempi(40_000),
            RiduzioneDellErrore::DiagnosticaDiRiga,
        ),
        (
            busta_alla_misura(MAX_BYTE_ERRORE, 1),
            RiduzioneDellErrore::EsempiDiRiga,
        ),
    ];

    for (busta, _) in casi {
        let categoria = busta["error"]["category"].clone();
        let riprova = busta["error"]["retry"].clone();
        let (uscita, riduzione) = entro_il_tetto_dell_errore(busta, 0);

        for campo in ["category", "phase", "remote_effect", "retry", "message"] {
            assert!(
                uscita["error"].get(campo).is_some_and(|v| !v.is_null()),
                "riduzione {riduzione:?}: manca il campo obbligatorio `{campo}`"
            );
        }

        assert_eq!(
                uscita["error"]["category"], categoria,
                "riduzione {riduzione:?}: la categoria e' cambiata, e con essa il                  codice d'uscita che il binding ha gia' calcolato"
            );
        assert_eq!(
            uscita["error"]["retry"], riprova,
            "riduzione {riduzione:?}: `retry` e' cambiato"
        );

        let codice = uscita["error"]["code"]
            .as_str()
            .expect("il codice e' una stringa")
            .to_owned();
        // La grafia di `error-v1.schema.json`: `^[A-Z][A-Z0-9_]{1,63}$`,
        // scritta a mano perche' una dipendenza di regex per tre righe
        // costerebbe piu' di quanto renda.
        assert!(
                (2..=64).contains(&codice.len())
                    && codice.starts_with(|c: char| c.is_ascii_uppercase())
                    && codice
                        .chars()
                        .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_'),
                "riduzione {riduzione:?}: `{codice}` non ha la grafia che                  `error-v1.schema.json` impone"
            );
    }
}

/// La stessa coerenza, presa dall'altro capo: dalla categoria della busta
/// ridotta si ricava lo stesso codice d'uscita della categoria d'origine.
///
/// Non e' la sonda di sopra detta due volte. Quella confronta due valori
/// JSON; questa fa il giro che il prodotto fa davvero -- deserializza la
/// categoria e la proietta -- e cade anche se la riduzione lasciasse una
/// categoria che il tipo non sa piu' leggere.
#[test]
fn dalla_busta_ridotta_si_ricava_lo_stesso_codice_d_uscita() {
    for categoria in [
        ErrorCategory::DataMapping,
        ErrorCategory::Io,
        ErrorCategory::Unsupported,
        ErrorCategory::NotFound,
        ErrorCategory::InvalidPlan,
        ErrorCategory::Internal,
    ] {
        let atteso = crate::uscita_della_categoria(categoria);

        let mut busta = busta_d_errore_con_esempi(40_000);
        busta["error"]["category"] =
            serde_json::to_value(categoria).expect("la categoria si serializza");

        let (uscita, riduzione) = entro_il_tetto_dell_errore(busta, 0);
        assert_ne!(
            riduzione,
            RiduzioneDellErrore::Nessuna,
            "il caso deve ridurre, altrimenti non prova niente"
        );

        let riletta: ErrorCategory = serde_json::from_value(uscita["error"]["category"].clone())
            .expect("la categoria della busta ridotta si rilegge");
        assert_eq!(
            crate::uscita_della_categoria(riletta),
            atteso,
            "{categoria:?}: la busta ridotta proietta un codice d'uscita diverso"
        );
    }
}

/// Una busta d'errore con `n` esempi di diagnostica di riga.
fn busta_d_errore_con_esempi(quanti: usize) -> Value {
    let esempi: Vec<Value> = (0..quanti)
        .map(|indice| {
            json!({
                "cause": "finto.riga_invalida",
                "column": "geometry",
                "source_index": indice,
            })
        })
        .collect();
    json!({
        "status": "error",
        "protocol_version": PROTOCOLLO,
        "contract": "plenora-error-v1",
        "error": {
            "category": "data_mapping",
            "phase": "read",
            "remote_effect": "none",
            "retry": {"kind": "never"},
            "code": "ROW_REJECTED",
            "message": "righe rifiutate",
            "row_diagnostics": {
                "contract": "plenora-row-diagnostics-v1",
                "scope": "read",
                "index_basis": "source_row_zero_based",
                "completeness": "partial",
                "observed_total": quanti,
                "counts": {"finto.riga_invalida": quanti},
                "examples_limit": 64,
                "examples_truncated": false,
                "examples": esempi,
            },
        },
    })
}

/// Una busta la cui codifica compatta misura **esattamente** `tetto + scarto`.
///
/// Il riempimento sta nel messaggio, e la lunghezza si calcola invece di
/// cercarla: si misura la busta vuota e si aggiunge la differenza.
fn busta_alla_misura(tetto: usize, scarto: i64) -> Value {
    let mut busta = busta_d_errore_con_esempi(1);
    busta["error"]["message"] = json!("");
    let base = serde_json::to_vec(&busta).expect("si serializza").len();
    let voluta = usize::try_from(i64::try_from(tetto).expect("tetto rappresentabile") + scarto)
        .expect("misura positiva");
    busta["error"]["message"] = json!("x".repeat(voluta - base));
    busta
}
