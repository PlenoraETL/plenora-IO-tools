//! Le prove di [`super`], in un file loro.
//!
//! Restano un **modulo figlio**: vedono i privati del genitore
//! esattamente come quando stavano dentro di lui, e non allargano di
//! una riga la superficie pubblica del crate.

use super::*;

const SCHEMA: SchemaOpzioniFormato = SchemaOpzioniFormato::nuovo(&[
    OpzioneFormato {
        chiave: "wkt_column",
        fase: FaseOpzione::Lettura,
        valore: ValoreAmmesso::Testo,
        predefinito: None,
        descrizione: "colonna con la geometria WKT",
    },
    OpzioneFormato {
        chiave: "delimiter",
        fase: FaseOpzione::Entrambe,
        valore: ValoreAmmesso::Carattere,
        predefinito: Some(","),
        descrizione: "separatore di campo",
    },
    OpzioneFormato {
        chiave: "compression",
        fase: FaseOpzione::Scrittura,
        valore: ValoreAmmesso::Enumerato(&["snappy", "zstd", "none"]),
        predefinito: Some("snappy"),
        descrizione: "codec di compressione",
    },
    OpzioneFormato {
        chiave: "legacy",
        fase: FaseOpzione::Lettura,
        valore: ValoreAmmesso::Booleano,
        predefinito: Some("false"),
        descrizione: "compatibilita' storica",
    },
]);

fn opzioni(coppie: &[(&str, &str)]) -> BTreeMap<String, String> {
    coppie
        .iter()
        .map(|(chiave, valore)| ((*chiave).to_owned(), (*valore).to_owned()))
        .collect()
}

fn valida(coppie: &[(&str, &str)], fase: FaseOpzione) -> Result<()> {
    valida_opzioni("prova", SCHEMA, &opzioni(coppie), fase)
}

#[test]
fn una_chiave_sconosciuta_e_rifiutata_e_l_errore_elenca_quelle_valide() {
    let errore = valida(&[("wkt_colunm", "g")], FaseOpzione::Lettura)
        .expect_err("il refuso deve fermarsi qui");
    let testo = errore.to_string();
    assert!(testo.contains("wkt_colunm"), "{testo}");
    assert!(
        testo.contains("wkt_column"),
        "l'elenco deve guidare: {testo}"
    );
    assert!(testo.contains("delimiter"), "{testo}");
    // `compression` e' di sola scrittura: non compare fra le chiavi di
    // lettura, altrimenti l'elenco suggerirebbe una strada chiusa.
    assert!(!testo.contains("compression"), "{testo}");
}

#[test]
fn una_chiave_della_fase_sbagliata_e_rifiutata() {
    let errore = valida(&[("compression", "zstd")], FaseOpzione::Lettura)
        .expect_err("un'opzione di scrittura non vale in lettura");
    let testo = errore.to_string();
    assert!(testo.contains("compression"), "{testo}");
    assert!(testo.contains("scrittura"), "{testo}");
}

#[test]
fn gli_enumerati_sono_case_sensitive() {
    assert!(valida(&[("compression", "zstd")], FaseOpzione::Scrittura).is_ok());
    for variante in ["ZSTD", "Zstd", "zstd ", " zstd"] {
        assert!(
            valida(&[("compression", variante)], FaseOpzione::Scrittura).is_err(),
            "{variante} doveva essere rifiutato"
        );
    }
}

#[test]
fn i_booleani_ammettono_sei_forme_e_nessun_altra() {
    for vero in ["true", "1", "yes"] {
        assert!(
            valida(&[("legacy", vero)], FaseOpzione::Lettura).is_ok(),
            "{vero}"
        );
        assert!(booleano("prova", "legacy", vero).unwrap(), "{vero}");
    }
    for falso in ["false", "0", "no"] {
        assert!(
            valida(&[("legacy", falso)], FaseOpzione::Lettura).is_ok(),
            "{falso}"
        );
        assert!(!booleano("prova", "legacy", falso).unwrap(), "{falso}");
    }
    // Le forme che la ratifica esclude esplicitamente.
    for rifiutato in ["on", "off", "1.0", "", "True", "YES", "si"] {
        assert!(
            valida(&[("legacy", rifiutato)], FaseOpzione::Lettura).is_err(),
            "{rifiutato} doveva essere rifiutato"
        );
    }
}

#[test]
fn un_carattere_e_esattamente_uno_e_ascii() {
    for valido in [",", ";", "\t", "|"] {
        assert!(
            valida(&[("delimiter", valido)], FaseOpzione::Lettura).is_ok(),
            "{valido}"
        );
    }
    for rifiutato in ["", ";;", "ab", "€"] {
        assert!(
            valida(&[("delimiter", rifiutato)], FaseOpzione::Lettura).is_err(),
            "{rifiutato:?} doveva essere rifiutato"
        );
    }
}

/// Newline e caratteri di controllo non escono grezzi.
///
/// Il messaggio finisce dentro un envelope JSON e, prima ancora, dentro il
/// terminale di chi legge. Un `\n` grezzo spezza una riga di log in due
/// record; un `\r` riscrive quella che c'era; una virgoletta non scappata
/// arriva a chi fa il parsing come struttura invece che come testo. Nessuno
/// dei tre e' payload, e tutti e tre sono canali.
#[test]
fn il_token_non_lascia_uscire_controlli_grezzi() {
    let ostili = [
        ("a\nb", "a\\nb"),
        ("a\rb", "a\\rb"),
        ("a\tb", "a\\tb"),
        ("a\"b", "a\\\"b"),
        ("a\\b", "a\\\\b"),
        ("a\u{0}b", "a\\u{0000}b"),
        ("a\u{1b}[2Kb", "a\\u{001b}[2Kb"),
    ];
    for (grezzo, atteso) in ostili {
        let reso = RejectedOptionToken::conia(grezzo).to_string();
        assert_eq!(reso, atteso, "grezzo: {grezzo:?}");
        // Nessun controllo, in nessuna forma: dopo l'escape non ne resta
        // nemmeno uno.
        assert!(
            !reso.chars().any(char::is_control),
            "un controllo e' uscito grezzo da {grezzo:?}"
        );
        // Nessuna virgoletta **non scappata**. La forma resa la contiene,
        // ma sempre preceduta da un backslash: e' la differenza fra un
        // carattere e una delimitazione, ed e' cio' che conta per chi fa
        // il parsing di quel JSON.
        let mut precedente = '\0';
        for carattere in reso.chars() {
            if carattere == '"' {
                assert_eq!(
                    precedente, '\\',
                    "virgoletta non scappata in {reso:?} da {grezzo:?}"
                );
            }
            precedente = carattere;
        }
    }
}

/// Un token lunghissimo viene troncato, e sempre allo stesso modo.
///
/// Senza tetto, un'opzione da un megabyte diventerebbe un messaggio
/// d'errore da un megabyte: la redazione avrebbe chiuso il canale del
/// contenuto lasciando aperto quello della dimensione.
#[test]
fn il_token_lunghissimo_viene_troncato_in_modo_deterministico() {
    let lungo = "k".repeat(10_000);
    let primo = RejectedOptionToken::conia(&lungo).to_string();
    let secondo = RejectedOptionToken::conia(&lungo).to_string();
    assert_eq!(primo, secondo, "il troncamento deve essere deterministico");
    assert_eq!(
        primo.chars().count(),
        MASSIMO_TOKEN + 1,
        "64 caratteri piu' l'ellissi"
    );
    assert!(primo.ends_with('…'), "il troncamento e' visibile: {primo}");

    // Il tetto conta i **caratteri**, non i byte: un'opzione di soli
    // caratteri multibyte non deve poter uscire quattro volte piu' lunga.
    let multibyte = "à".repeat(10_000);
    let reso = RejectedOptionToken::conia(&multibyte).to_string();
    assert_eq!(reso.chars().count(), MASSIMO_TOKEN + 1);

    // Un token corto non viene toccato ne' marcato.
    let corto = RejectedOptionToken::conia("wkt_colunm").to_string();
    assert_eq!(corto, "wkt_colunm");
    assert!(!corto.ends_with('…'));

    // Il troncamento avviene **dopo** l'escape, quindi un input fatto di
    // soli controlli non sfonda il tetto espandendosi.
    let controlli = "\u{1}".repeat(10_000);
    let reso = RejectedOptionToken::conia(&controlli).to_string();
    assert!(
        reso.chars().count() <= MASSIMO_TOKEN * 8 + 1,
        "l'escape non deve rendere il tetto inutile: {} caratteri",
        reso.chars().count()
    );
}

/// L'eccezione arriva davvero all'errore, con il refuso dentro.
///
/// E' la proprieta' che S6 aveva ratificato e che l'eccezione conserva: chi
/// ha sbagliato a scrivere vede **cosa** ha scritto.
#[test]
fn il_refuso_arriva_all_errore_gia_reso_sicuro() {
    const SCHEMA: SchemaOpzioniFormato = SchemaOpzioniFormato::nuovo(&[OpzioneFormato {
        chiave: "wkt_column",
        fase: FaseOpzione::Lettura,
        valore: ValoreAmmesso::Testo,
        predefinito: None,
        descrizione: "colonna WKT",
    }]);

    let errore = valida_opzioni(
        "prova",
        SCHEMA,
        &opzioni(&[("wkt_colunm", "geom")]),
        FaseOpzione::Lettura,
    )
    .expect_err("il refuso e' rifiutato");
    assert!(errore.message.contains("wkt_colunm"), "{errore}");
    assert!(errore.message.contains("wkt_column"), "{errore}");

    // E un refuso ostile arriva **reso**, non grezzo.
    let errore = valida_opzioni(
        "prova",
        SCHEMA,
        &opzioni(&[("a\nb\"c", "x")]),
        FaseOpzione::Lettura,
    )
    .expect_err("rifiutato");
    assert!(errore.message.contains("a\\nb\\\"c"), "{errore}");
    assert!(
        !errore.message.contains('\n'),
        "newline grezzo nel messaggio"
    );
}

#[test]
fn un_intero_rispetta_gli_estremi_dichiarati() {
    const CON_INTERO: SchemaOpzioniFormato = SchemaOpzioniFormato::nuovo(&[OpzioneFormato {
        chiave: "limite",
        fase: FaseOpzione::Lettura,
        valore: ValoreAmmesso::Intero {
            minimo: 1,
            massimo: 64,
        },
        predefinito: Some("64"),
        descrizione: "esempi per diagnostica",
    }]);
    let prova = |valore: &str| {
        valida_opzioni(
            "prova",
            CON_INTERO,
            &opzioni(&[("limite", valore)]),
            FaseOpzione::Lettura,
        )
    };
    for valido in ["1", "8", "64"] {
        assert!(prova(valido).is_ok(), "{valido}");
    }
    // Fuori intervallo, non numerici, e le forme che `parse::<u64>`
    // rifiuta da sola: segno, spazi, decimali.
    for rifiutato in ["0", "65", "", "otto", "-1", " 8", "8.0", "+8"] {
        assert!(
            prova(rifiutato).is_err(),
            "{rifiutato:?} doveva essere rifiutato"
        );
    }
}

#[test]
fn il_testo_libero_non_puo_essere_vuoto() {
    assert!(valida(&[("wkt_column", "geom")], FaseOpzione::Lettura).is_ok());
    assert!(valida(&[("wkt_column", "")], FaseOpzione::Lettura).is_err());
}

#[test]
fn una_opzione_di_entrambe_le_fasi_vale_in_tutte_e_due() {
    assert!(valida(&[("delimiter", ";")], FaseOpzione::Lettura).is_ok());
    assert!(valida(&[("delimiter", ";")], FaseOpzione::Scrittura).is_ok());
}

#[test]
fn il_default_dichiarato_e_leggibile_dallo_schema() {
    assert_eq!(SCHEMA.predefinito("compression"), Some("snappy"));
    assert_eq!(SCHEMA.predefinito("delimiter"), Some(","));
    assert_eq!(SCHEMA.predefinito("wkt_column"), None);
}

#[test]
fn uno_schema_vuoto_rifiuta_qualunque_chiave() {
    let errore = valida_opzioni(
        "prova",
        SchemaOpzioniFormato::VUOTO,
        &opzioni(&[("qualunque", "cosa")]),
        FaseOpzione::Lettura,
    )
    .expect_err("senza opzioni dichiarate ogni chiave e' sconosciuta");
    assert!(errore.to_string().contains("nessuna"), "{errore}");
}

#[test]
fn nessuna_opzione_passa_sempre() {
    assert!(valida(&[], FaseOpzione::Lettura).is_ok());
    assert!(valida(&[], FaseOpzione::Scrittura).is_ok());
}
