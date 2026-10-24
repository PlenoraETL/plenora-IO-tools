//! Le prove di [`super`], in un file loro.

use super::*;

const WKT1_3003: &str = r#"PROJCS["Monte Mario / Italy zone 1",GEOGCS["Monte Mario",AUTHORITY["EPSG","4265"]],AUTHORITY["EPSG","3003"]]"#;
const WKT2_4326: &str = r#"GEOGCRS["WGS 84",ID["EPSG",4326]]"#;

fn coppia(autorita: &str, codice: &str) -> (TestoDefinizione, TestoDefinizione) {
    (
        TestoDefinizione::Unicode(autorita.to_owned()),
        TestoDefinizione::Unicode(codice.to_owned()),
    )
}

#[test]
fn identificatori_di_primo_livello_nei_tre_formati() {
    assert_eq!(
        identificatori(WKT1_3003, CrsDefinitionFormat::Wkt),
        Ok(vec![coppia("EPSG", "3003")])
    );
    assert_eq!(
        identificatori(WKT2_4326, CrsDefinitionFormat::Wkt2),
        Ok(vec![coppia("EPSG", "4326")])
    );
    let projjson = r#"{"type":"GeographicCRS","ids":[{"authority":"EPSG","code":4326},{"authority":"OGC","code":"CRS84"}]}"#;
    assert_eq!(
        identificatori(projjson, CrsDefinitionFormat::Projjson),
        Ok(vec![coppia("EPSG", "4326"), coppia("OGC", "CRS84")])
    );
}

#[test]
fn l_identificatore_annidato_non_conta() {
    // Il PROJCS EPSG:3003 contiene un GEOGCS: dichiarato EPSG:4265 o EPSG:4326,
    // conta solo l'AUTHORITY della radice.
    assert_eq!(
        contraddice(Some("EPSG:3003"), WKT1_3003, CrsDefinitionFormat::Wkt),
        Ok(false)
    );
    assert_eq!(
        contraddice(Some("EPSG:4265"), WKT1_3003, CrsDefinitionFormat::Wkt),
        Ok(true)
    );
}

#[test]
fn contraddizione_solo_nella_stessa_autorita() {
    assert_eq!(
        contraddice(Some("EPSG:4326"), WKT2_4326, CrsDefinitionFormat::Wkt2),
        Ok(false)
    );
    assert_eq!(
        contraddice(Some("epsg:4326"), WKT2_4326, CrsDefinitionFormat::Wkt2),
        Ok(false)
    );
    assert_eq!(
        contraddice(Some("EPSG:3857"), WKT2_4326, CrsDefinitionFormat::Wkt2),
        Ok(true)
    );
    // un'autorita' diversa non si confronta
    assert_eq!(
        contraddice(Some("OGC:CRS84"), WKT2_4326, CrsDefinitionFormat::Wkt2),
        Ok(false)
    );
    // senza crs_id non c'e' niente da contraddire
    assert_eq!(
        contraddice(None, WKT2_4326, CrsDefinitionFormat::Wkt2),
        Ok(false)
    );
    // il codice numerico si scrive come intero decimale
    let con_zeri = r#"GEOGCRS["WGS 84",ID["EPSG",+004326]]"#;
    assert_eq!(
        contraddice(Some("EPSG:4326"), con_zeri, CrsDefinitionFormat::Wkt2),
        Ok(false)
    );
}

#[test]
fn definizioni_malformate() {
    for (testo, formato) in [
        (
            r#"GEOGCRS["WGS 84",ID["EPSG",4326])"#,
            CrsDefinitionFormat::Wkt2,
        ),
        (
            r#"GEOGCRS["WGS 84",ID["EPSG",4326]] dopo"#,
            CrsDefinitionFormat::Wkt2,
        ),
        (r#"GEOGCRS["WGS 84""#, CrsDefinitionFormat::Wkt2),
        (r#"GEOGCS["WGS 84"]"#, CrsDefinitionFormat::Wkt2),
        (r#"GEOGCRS["WGS 84"]"#, CrsDefinitionFormat::Wkt),
        (
            r#"GEOGCRS["WGS 84",ID["EPSG",4326.5]]"#,
            CrsDefinitionFormat::Wkt2,
        ),
        (r#"GEOGCRS["WGS 84",ID[4326]]"#, CrsDefinitionFormat::Wkt2),
        (
            "GEOGCRS[\"WGS 84\",ID[\"EPSG\",\u{0664}]]",
            CrsDefinitionFormat::Wkt2,
        ),
        (
            r#"{"id":{"authority":"EPSG","code":4326},"id":{"authority":"EPSG","code":3857}}"#,
            CrsDefinitionFormat::Projjson,
        ),
        (
            r#"{"id":{"authority":"EPSG","code":4326},"ids":[]}"#,
            CrsDefinitionFormat::Projjson,
        ),
        (
            r#"{"id":{"authority":"EPSG","code":true}}"#,
            CrsDefinitionFormat::Projjson,
        ),
        (
            r#"{"id":{"authority":"EPSG","code":4326.0}}"#,
            CrsDefinitionFormat::Projjson,
        ),
        ("[]", CrsDefinitionFormat::Projjson),
        ("", CrsDefinitionFormat::Wkt),
    ] {
        assert_eq!(identificatori(testo, formato), Err(Malformata), "{testo}");
        assert_eq!(
            contraddice(None, testo, formato),
            Err(Malformata),
            "{testo}"
        );
    }
}

#[test]
fn profondita_oltre_il_limite() {
    let mut testo = String::from("GEOGCRS[\"x\"");
    for _ in 0..PROFONDITA_MASSIMA {
        testo.push_str(",A[\"y\"");
    }
    testo.push_str(&"]".repeat(PROFONDITA_MASSIMA + 1));
    assert!(identificatori(&testo, CrsDefinitionFormat::Wkt2).is_ok());
    let piu_profonda = testo.replacen("GEOGCRS[\"x\"", "GEOGCRS[\"x\",B[\"z\"", 1) + "]";
    assert_eq!(
        identificatori(&piu_profonda, CrsDefinitionFormat::Wkt2),
        Err(Malformata)
    );
}

#[test]
fn autorita_dei_cataloghi_locali() {
    for id in ["POSTGIS:4326", "postgis:4326", "MySQL:4326", "SQLITE:1"] {
        assert!(autorita_locale(id), "{id}");
    }
    for id in ["EPSG:4326", "OGC:CRS84", "POSTGIS", "POSTGISX:4326"] {
        assert!(!autorita_locale(id), "{id}");
    }
}

/// `str.isspace()` di Python, carattere per carattere: i 29 spazi che Python 3.11
/// conta, e nessun altro. Sono il `White_Space` di Unicode piu' U+001C..U+001F.
#[test]
fn lo_spazio_e_quello_di_python() {
    const PYTHON: [u32; 29] = [
        0x9, 0xa, 0xb, 0xc, 0xd, 0x1c, 0x1d, 0x1e, 0x1f, 0x20, 0x85, 0xa0, 0x1680, 0x2000, 0x2001,
        0x2002, 0x2003, 0x2004, 0x2005, 0x2006, 0x2007, 0x2008, 0x2009, 0x200a, 0x2028, 0x2029,
        0x202f, 0x205f, 0x3000,
    ];
    for punto in 0..=0x10_ffff_u32 {
        let Some(c) = char::from_u32(punto) else {
            continue;
        };
        assert_eq!(spazio_python(c), PYTHON.contains(&punto), "U+{punto:04X}");
        let separatore = (0x1c..=0x1f).contains(&punto);
        assert_eq!(
            spazio_python(c),
            c.is_whitespace() || separatore,
            "U+{punto:04X}"
        );
    }
}

/// Il WKT si spoglia degli spazi di Python: U+001C..U+001F, U+0085 e U+3000
/// separano i token, U+200B no (e' un carattere inatteso).
#[test]
fn il_wkt_separa_i_token_con_gli_spazi_di_python() {
    for spazio in ['\u{1c}', '\u{1f}', '\u{85}', '\u{3000}'] {
        let testo = format!("GEOGCS[\"x\",{spazio}AUTHORITY[\"EPSG\",4326]]{spazio}");
        assert_eq!(
            identificatori(&testo, CrsDefinitionFormat::Wkt),
            Ok(vec![coppia("EPSG", "4326")]),
            "U+{:04X}",
            u32::from(spazio)
        );
    }
    let testo = "GEOGCS[\"x\",\u{200b}AUTHORITY[\"EPSG\",4326]]";
    assert_eq!(
        identificatori(testo, CrsDefinitionFormat::Wkt),
        Err(Malformata)
    );
}

/// I PROJJSON che `json.loads` legge e `serde_json` (senza le sue feature
/// opzionali) rifiutava: un numero oltre `f64`, un surrogato isolato, piu' di
/// 128 livelli, un codice `-0` o oltre `u64`.
#[test]
fn projjson_come_lo_legge_l_oracolo() {
    let con = |membro: &str, codice: &str| {
        format!(r#"{{"type":"GeographicCRS",{membro}"id":{{"authority":"EPSG","code":{codice}}}}}"#)
    };
    for testo in [
        con(r#""x":1e400,"#, "4326"),
        con(r#""x":-1e-400,"#, "4326"),
        con(r#""name":"\ud800","#, "4326"),
        con(r#""name":"\udc00\ud800x","#, "4326"),
        con(
            &format!(r#""x":{}{},"#, "[".repeat(2000), "]".repeat(2000)),
            "4326",
        ),
        con(
            &format!(r#""x":{}1{},"#, r#"{"a":"#.repeat(500), "}".repeat(500)),
            "4326",
        ),
        // Caratteri crudi di due, tre e quattro byte.
        con("\"name\":\"Citt\u{e0} \u{2713} \u{1f600}\",", "4326"),
    ] {
        assert_eq!(
            identificatori(&testo, CrsDefinitionFormat::Projjson),
            Ok(vec![coppia("EPSG", "4326")]),
            "{testo:.80}"
        );
    }
    // `-0` e' l'intero 0: `str(int)` lo scrive `0`.
    let zero = con("", "-0");
    assert_eq!(
        identificatori(&zero, CrsDefinitionFormat::Projjson),
        Ok(vec![coppia("EPSG", "0")])
    );
    assert_eq!(
        contraddice(Some("EPSG:0"), &zero, CrsDefinitionFormat::Projjson),
        Ok(false)
    );
    assert_eq!(
        contraddice(Some("EPSG:-0"), &zero, CrsDefinitionFormat::Projjson),
        Ok(true)
    );
    let grande = con("", "184467440737095516160000");
    assert_eq!(
        identificatori(&grande, CrsDefinitionFormat::Projjson),
        Ok(vec![coppia("EPSG", "184467440737095516160000")])
    );
    let negativo = con("", "-4326");
    assert_eq!(
        contraddice(Some("EPSG:-4326"), &negativo, CrsDefinitionFormat::Projjson),
        Ok(false)
    );
}

/// I surrogati: una coppia si unisce, un isolato si conserva e non e' uguale a
/// nessun codice; una chiave scritta con un escape e' la stessa chiave.
#[test]
fn projjson_surrogati_ed_escape() {
    let coppia_unita = r#"{"id":{"authority":"EPSG","code":"\ud83d\ude00"}}"#;
    assert_eq!(
        identificatori(coppia_unita, CrsDefinitionFormat::Projjson),
        Ok(vec![coppia("EPSG", "\u{1f600}")])
    );
    let isolato = r#"{"id":{"authority":"EPSG","code":"\ud800"}}"#;
    assert_eq!(
        identificatori(isolato, CrsDefinitionFormat::Projjson),
        Ok(vec![(
            TestoDefinizione::Unicode("EPSG".to_owned()),
            TestoDefinizione::SurrogatiIsolati
        )])
    );
    assert_eq!(
        contraddice(
            Some("EPSG:\u{fffd}"),
            isolato,
            CrsDefinitionFormat::Projjson
        ),
        Ok(true),
        "un surrogato isolato non e' il carattere di sostituzione"
    );
    let escape = r#"{"\u0069d":{"\u0061uthority":"EPSG","code":4326}}"#;
    assert_eq!(
        identificatori(escape, CrsDefinitionFormat::Projjson),
        Ok(vec![coppia("EPSG", "4326")])
    );
}

/// Cio' che `json.loads`, con i ganci dell'oracolo, rifiuta.
#[test]
fn projjson_malformati_come_per_l_oracolo() {
    let id = r#""id":{"authority":"EPSG","code":4326}"#;
    for testo in [
        format!(r#"{{"x":NaN,{id}}}"#),
        format!(r#"{{"x":Infinity,{id}}}"#),
        format!(r#"{{"x":-Infinity,{id}}}"#),
        format!(r#"{{"x":{{"a":1,"a":2}},{id}}}"#),
        format!(r#"{{"\u0069d":1,{id}}}"#),
        format!("{{\"name\":\"a\tb\",{id}}}"),
        format!(r#"{{"name":"a\xb",{id}}}"#),
        format!(r#"{{"name":"\u12",{id}}}"#),
        format!(r#"{{"x":01,{id}}}"#),
        format!(r#"{{"x":1.,{id}}}"#),
        format!(r#"{{"x":.5,{id}}}"#),
        format!(r#"{{"x":1e,{id}}}"#),
        format!(r#"{{"x":+1,{id}}}"#),
        format!(r#"{{"x":[1,],{id}}}"#),
        format!(r#"{{"x":tru,{id}}}"#),
        format!("{{{id}}}\u{b}"),
        format!("\u{feff}{{{id}}}"),
        format!("{{{id}}} {{}}"),
        format!("{{{id},}}"),
        r#"{"id":{"authority":"EPSG","code":4326e0}}"#.to_owned(),
        r#"{"id":{"authority":"EPSG","code":null}}"#.to_owned(),
        r#"{"id":{"authority":"EPSG"}}"#.to_owned(),
        r#"{"id":{"authority":1,"code":4326}}"#.to_owned(),
        r#"{"id":[{"authority":"EPSG","code":4326}]}"#.to_owned(),
        r#"{"ids":{"authority":"EPSG","code":4326}}"#.to_owned(),
        r#"{"ids":[1]}"#.to_owned(),
        "\"EPSG\"".to_owned(),
        String::new(),
        "{".repeat(100),
    ] {
        assert_eq!(
            identificatori(&testo, CrsDefinitionFormat::Projjson),
            Err(Malformata),
            "{testo:.80}"
        );
    }
}

/// Gli spazi di JSON sono quattro, e soltanto quattro.
#[test]
fn projjson_spazi_di_json() {
    let testo = " \t\n\r{ \"id\" :\r{\"authority\" : \"EPSG\" , \"code\" : 4326 } }\n";
    assert_eq!(
        identificatori(testo, CrsDefinitionFormat::Projjson),
        Ok(vec![coppia("EPSG", "4326")])
    );
}

/// L'autorita' si confronta ASCII case-insensitive (GEO-005, GEO-020): la s
/// lunga U+017F e la i senza punto U+0131 non sono `S` e `I`, quindi `EPſG` e'
/// un'altra autorita' e non contraddice `EPSG:4326`.
#[test]
fn l_autorita_si_confronta_solo_in_ascii() {
    let projjson = "{\"id\":{\"authority\":\"EP\u{17f}G\",\"code\":3857}}";
    assert_eq!(
        contraddice(Some("EPSG:4326"), projjson, CrsDefinitionFormat::Projjson),
        Ok(false)
    );
    let wkt = "GEOGCS[\"x\",AUTHORITY[\"EP\u{17f}G\",3857]]";
    assert_eq!(
        contraddice(Some("EPSG:4326"), wkt, CrsDefinitionFormat::Wkt),
        Ok(false)
    );
    let ascii = "GEOGCS[\"x\",AUTHORITY[\"epsg\",3857]]";
    assert_eq!(
        contraddice(Some("EPSG:4326"), ascii, CrsDefinitionFormat::Wkt),
        Ok(true)
    );
    assert!(!autorita_locale("POSTG\u{131}S:4326"));
    assert!(autorita_locale("postgis:4326"));
}

/// Il WGS 84 nella forma ESRI che il writer Shapefile emette, senza autorita'.
const ESRI_WGS84: &str = r#"GEOGCS["WGS 84",DATUM["WGS_1984",SPHEROID["WGS 84",6378137,298.257223563]],PRIMEM["Greenwich",0],UNIT["degree",0.0174532925199433]]"#;

/// La forma ESRI di WGS 84 con un nodo in piu' alla radice, prima della `]`
/// finale.
fn esri_con(nodo: &str) -> String {
    format!("{},{nodo}]", &ESRI_WGS84[..ESRI_WGS84.len() - 1])
}

/// Un `PROJCS` di Monte Mario senza identificatori di radice, con un nodo in
/// piu' alla radice (vuoto: nessuno).
fn projcs_con(nodo: &str) -> String {
    let separatore = if nodo.is_empty() { "" } else { "," };
    format!(
        r#"PROJCS["Monte Mario / Italy zone 1",GEOGCS["Monte Mario",DATUM["Monte_Mario",SPHEROID["International 1924",6378388,297]],PRIMEM["Greenwich",0],UNIT["degree",0.0174532925199433],AUTHORITY["EPSG","4265"]],PROJECTION["Transverse_Mercator"],UNIT["metre",1]{separatore}{nodo}]"#
    )
}

const BOUNDCRS: &str = r#"BOUNDCRS[SOURCECRS[GEOGCRS["Monte Mario",DATUM["Monte Mario",ELLIPSOID["International 1924",6378388,297]],CS[ellipsoidal,2],AXIS["latitude",north],AXIS["longitude",east],ANGLEUNIT["degree",0.0174532925199433],ID["EPSG",4265]]],TARGETCRS[GEOGCRS["WGS 84",DATUM["World Geodetic System 1984",ELLIPSOID["WGS 84",6378137,298.257223563]],CS[ellipsoidal,2],AXIS["latitude",north],AXIS["longitude",east],ANGLEUNIT["degree",0.0174532925199433],ID["EPSG",4326]]],ABRIDGEDTRANSFORMATION["t",METHOD["Geocentric translations"]]]"#;

type Atteso = Result<Option<&'static str>, RifiutoIdentita>;

fn verifica(casi: Vec<(&str, String, Option<&str>, Atteso)>) {
    for (nome, testo, assunto, atteso) in casi {
        assert_eq!(
            identificatore_wkt(&testo, assunto),
            atteso.map(|id| id.map(str::to_owned)),
            "{nome}, opzione {assunto:?}"
        );
    }
}

// La regola unica dell'identificatore di un WKT, caso per caso, senza e con
// `--assume-crs`, in quattro prove. Le righe marcate (a)..(f) sono le
// etichette sbagliate trovate dalle riletture fino al 24/10 sulle letture
// parziali dei driver: ognuna, sul codice di a651844, dava un identificatore o
// accettava un'opzione dove qui c'e' un rifiuto o un altro identificatore
// (sonda sui driver, prove `il_prj_segue_la_regola_unica_dell_identificatore`
// e `il_wkt_del_geodata_segue_la_regola_unica`).

/// 6. Nessuna dichiarazione: la forma ESRI di WGS 84, decisa sull'albero; (a) un `ID` alla radice e' una dichiarazione e la deduzione non c'e'.
#[test]
fn regola_wkt_senza_dichiarazioni_e_forma_esri() {
    use RifiutoIdentita::Contraddetta;
    let esri_estensione = r#"GEOGCS["WGS 84",DATUM["D_Monte_Mario",SPHEROID["International_1924",6378388,297]],PRIMEM["Greenwich",0],UNIT["degree",0.0174532925199433],EXTENSION["x",DATUM["WGS_1984"]]]"#.to_owned();
    let esri_due_datum = esri_con(r#"DATUM["Monte_Mario"]"#);
    let esri_minuscolo =
        r#"geogcs ( "wgs 84" , datum ( "wgs_1984" , SPHEROID["WGS 84",6378137,298.257223563] ) )"#
            .to_owned();
    verifica(vec![
        // 6. Nessuna dichiarazione, forma ESRI di WGS 84.
        ("esri", ESRI_WGS84.to_owned(), None, Ok(Some("EPSG:4326"))),
        (
            "esri",
            ESRI_WGS84.to_owned(),
            Some("OGC:CRS84"),
            Ok(Some("OGC:CRS84")),
        ),
        (
            "esri",
            ESRI_WGS84.to_owned(),
            Some("epsg:4326"),
            Ok(Some("epsg:4326")),
        ),
        (
            "esri",
            ESRI_WGS84.to_owned(),
            Some("EPSG:3003"),
            Err(Contraddetta),
        ),
        (
            "esri minuscolo",
            esri_minuscolo,
            None,
            Ok(Some("EPSG:4326")),
        ),
        // (e) Il `DATUM` di WGS 84 dentro un `EXTENSION` non e' quello della
        // radice: prima bastava trovarlo nel testo.
        (
            "(e) datum annidato",
            esri_estensione.clone(),
            None,
            Ok(None),
        ),
        (
            "(e) datum annidato",
            esri_estensione,
            Some("EPSG:3003"),
            Ok(Some("EPSG:3003")),
        ),
        ("due datum", esri_due_datum.clone(), None, Ok(None)),
        (
            "due datum",
            esri_due_datum,
            Some("EPSG:3003"),
            Ok(Some("EPSG:3003")),
        ),
        (
            "D_WGS_1984",
            ESRI_WGS84.replace("WGS_1984", "D_WGS_1984"),
            None,
            Ok(None),
        ),
        // (a) Un `ID` con la parola dell'altra grammatica alla radice e' una
        // dichiarazione: niente deduzione.
        (
            "(a)",
            esri_con(r#"ID["EPSG",4258]"#),
            None,
            Ok(Some("EPSG:4258")),
        ),
        (
            "(a)",
            esri_con(r#"ID["EPSG",4258]"#),
            Some("EPSG:4326"),
            Err(Contraddetta),
        ),
        (
            "(a)",
            esri_con(r#"ID["EPSG",4258]"#),
            Some("OGC:CRS84"),
            Err(Contraddetta),
        ),
    ]);
}

/// 5. Una dichiarazione, con l'una o l'altra parola chiave; (d) `ID` in WKT 1 e `AUTHORITY` in WKT 2.
#[test]
fn regola_wkt_con_una_dichiarazione() {
    use RifiutoIdentita::Contraddetta;
    verifica(vec![
        // Un'autorita' non EPSG dichiarata non si risolve, e non si rietichetta.
        (
            "esri 104000",
            esri_con(r#"AUTHORITY["ESRI","104000"]"#),
            None,
            Ok(None),
        ),
        (
            "esri 104000",
            esri_con(r#"AUTHORITY["ESRI","104000"]"#),
            Some("EPSG:4326"),
            Err(Contraddetta),
        ),
        (
            "esri 104000",
            esri_con(r#"AUTHORITY["ESRI","104000"]"#),
            Some("esri:104000"),
            Ok(Some("ESRI:104000")),
        ),
        (
            "ogc crs84 di radice",
            esri_con(r#"AUTHORITY["OGC","CRS84"]"#),
            None,
            Ok(None),
        ),
        (
            "ogc crs84 di radice",
            esri_con(r#"AUTHORITY["OGC","CRS84"]"#),
            Some("OGC:CRS84"),
            Ok(Some("OGC:CRS84")),
        ),
        // 5. Una dichiarazione.
        (
            "3003",
            projcs_con(r#"AUTHORITY["EPSG","3003"]"#),
            None,
            Ok(Some("EPSG:3003")),
        ),
        (
            "3003",
            projcs_con(r#"AUTHORITY["epsg","3003"]"#),
            None,
            Ok(Some("EPSG:3003")),
        ),
        (
            "3003",
            projcs_con(r#"AUTHORITY["epsg","3003"]"#),
            Some("EPSG:3003"),
            Ok(Some("epsg:3003")),
        ),
        (
            "3003",
            projcs_con(r#"AUTHORITY["EPSG","3003"]"#),
            Some("EPSG:4326"),
            Err(Contraddetta),
        ),
        (
            "3003 numero",
            projcs_con(r#"AUTHORITY["EPSG",+03003]"#),
            None,
            Ok(Some("EPSG:3003")),
        ),
        (
            "codice non canonico",
            projcs_con(r#"AUTHORITY["EPSG","03003"]"#),
            None,
            Ok(None),
        ),
        (
            "codice non canonico",
            projcs_con(r#"AUTHORITY["EPSG","03003"]"#),
            Some("EPSG:3003"),
            Err(Contraddetta),
        ),
        (
            "codice non canonico",
            projcs_con(r#"AUTHORITY["EPSG","03003"]"#),
            Some("EPSG:03003"),
            Ok(Some("EPSG:03003")),
        ),
        (
            "duplicato",
            projcs_con(r#"AUTHORITY["EPSG","3003"],ID["epsg",3003]"#),
            None,
            Ok(Some("EPSG:3003")),
        ),
        (
            "duplicato",
            projcs_con(r#"AUTHORITY["EPSG","3003"],ID["epsg",3003]"#),
            Some("epsg:3003"),
            Ok(Some("EPSG:3003")),
        ),
    ]);
}

/// (d) `ID` in un WKT 1 e `AUTHORITY` in un WKT 2 dichiarano; la radice si
/// legge con ogni spaziatura, maiuscola e parentesi che la grammatica ammette.
#[test]
fn regola_wkt_con_l_altra_parola_chiave() {
    use RifiutoIdentita::Contraddetta;
    verifica(vec![
        // (d) `ID` in un WKT 1, `AUTHORITY` in un WKT 2: dichiarano.
        (
            "(d) id in wkt1",
            projcs_con(r#"ID["EPSG",3003]"#),
            None,
            Ok(Some("EPSG:3003")),
        ),
        (
            "(d) id in wkt1",
            projcs_con(r#"ID["EPSG",3003]"#),
            Some("EPSG:4326"),
            Err(Contraddetta),
        ),
        (
            "(d) authority in wkt2",
            r#"PROJCRS["x",AUTHORITY["EPSG","3003"]]"#.to_owned(),
            Some("EPSG:4326"),
            Err(Contraddetta),
        ),
        (
            "(d) authority in wkt2",
            r#"PROJCRS["x",AUTHORITY["EPSG","3003"]]"#.to_owned(),
            None,
            Ok(Some("EPSG:3003")),
        ),
        (
            "wkt2",
            r#"geogcrs [ "WGS 84" , ID ( "EPSG" , 4326 ) ]"#.to_owned(),
            None,
            Ok(Some("EPSG:4326")),
        ),
        (
            "tonde",
            r#"GEOGCS("WGS 84",AUTHORITY("EPSG","4326"))"#.to_owned(),
            None,
            Ok(Some("EPSG:4326")),
        ),
    ]);
}

/// 4. Due dichiarazioni diverse (b) e 3. `BOUNDCRS`, letto dalla parola della radice; senza dichiarazioni l'opzione nomina.
#[test]
fn regola_wkt_ambigua_o_legata() {
    use RifiutoIdentita::{Ambigua, Legata};
    let bound_spazi = BOUNDCRS.replacen("BOUNDCRS[", "boundcrs [", 1);
    let bound_con_id = format!("{},ID[\"EPSG\",4326]]", &BOUNDCRS[..BOUNDCRS.len() - 1]);
    verifica(vec![
        // 4. (b) Due dichiarazioni diverse.
        (
            "(b)",
            projcs_con(r#"AUTHORITY["EPSG","3003"],ID["EPSG",3004]"#),
            None,
            Err(Ambigua),
        ),
        (
            "(b)",
            projcs_con(r#"AUTHORITY["EPSG","3003"],ID["EPSG",3004]"#),
            Some("EPSG:3003"),
            Err(Ambigua),
        ),
        (
            "due authority",
            projcs_con(r#"AUTHORITY["EPSG","3003"],AUTHORITY["EPSG","3004"]"#),
            None,
            Err(Ambigua),
        ),
        (
            "due autorita",
            projcs_con(r#"AUTHORITY["EPSG","3003"],AUTHORITY["ESRI","3003"]"#),
            Some("EPSG:3003"),
            Err(Ambigua),
        ),
        // 3. `BOUNDCRS`, letto dalla parola della radice.
        ("boundcrs", BOUNDCRS.to_owned(), None, Ok(None)),
        (
            "boundcrs",
            BOUNDCRS.to_owned(),
            Some("EPSG:4326"),
            Err(Legata),
        ),
        (
            "(f) boundcrs con spazi",
            bound_spazi.clone(),
            None,
            Ok(None),
        ),
        (
            "(f) boundcrs con spazi",
            bound_spazi,
            Some("EPSG:4265"),
            Err(Legata),
        ),
        ("boundcrs con id", bound_con_id.clone(), None, Ok(None)),
        (
            "boundcrs con id",
            bound_con_id,
            Some("EPSG:4326"),
            Err(Legata),
        ),
        // Nessuna dichiarazione, nessuna forma: l'opzione nomina.
        ("anonimo", projcs_con(""), None, Ok(None)),
        (
            "anonimo",
            projcs_con(""),
            Some("EPSG:3003"),
            Ok(Some("EPSG:3003")),
        ),
        ("local", r#"LOCAL_CS["griglia"]"#.to_owned(), None, Ok(None)),
    ]);
}

/// 1. e 2. Radice o nodo identificatore malformati: (c) e (f), con e senza opzione.
#[test]
fn regola_wkt_malformata() {
    use RifiutoIdentita::Malformata as Mal;
    verifica(vec![
        // 1. e 2. Malformati: radice o nodo identificatore.
        (
            "(c) id senza virgolette",
            projcs_con("ID[EPSG,3003]"),
            None,
            Err(Mal),
        ),
        (
            "(c) id senza virgolette",
            projcs_con("ID[EPSG,3003]"),
            Some("EPSG:3003"),
            Err(Mal),
        ),
        (
            "(c) id senza codice",
            projcs_con(r#"ID["EPSG"]"#),
            Some("EPSG:3003"),
            Err(Mal),
        ),
        (
            "(c) id decimale",
            projcs_con(r#"ID["EPSG",3003.5]"#),
            Some("EPSG:3003"),
            Err(Mal),
        ),
        (
            "authority senza codice",
            projcs_con(r#"AUTHORITY["EPSG"]"#),
            None,
            Err(Mal),
        ),
        (
            "wkt2 authority malformata",
            r#"PROJCRS["x",AUTHORITY[EPSG,3003]]"#.to_owned(),
            Some("EPSG:3003"),
            Err(Mal),
        ),
        (
            "(f) codice decimale",
            esri_con(r#"AUTHORITY["EPSG",4258.0]"#),
            None,
            Err(Mal),
        ),
        (
            "(f) codice decimale",
            esri_con(r#"AUTHORITY["EPSG",4258.0]"#),
            Some("OGC:CRS84"),
            Err(Mal),
        ),
        (
            "(f) senza ] finale",
            ESRI_WGS84[..ESRI_WGS84.len() - 1].to_owned(),
            None,
            Err(Mal),
        ),
        (
            "(f) senza ] finale",
            ESRI_WGS84[..ESRI_WGS84.len() - 1].to_owned(),
            Some("EPSG:4326"),
            Err(Mal),
        ),
        (
            "(f) testo in coda",
            format!("{ESRI_WGS84} coda"),
            None,
            Err(Mal),
        ),
        (
            "(f) testo in coda",
            format!("{ESRI_WGS84} coda"),
            Some("OGC:CRS84"),
            Err(Mal),
        ),
        ("non wkt", "WGS 84".to_owned(), None, Err(Mal)),
        ("non wkt", "WGS 84".to_owned(), Some("EPSG:4326"), Err(Mal)),
        ("vuoto", String::new(), Some("EPSG:4326"), Err(Mal)),
        (
            "radice ignota",
            r#"FOOCS["x",AUTHORITY["EPSG","3003"]]"#.to_owned(),
            None,
            Err(Mal),
        ),
    ]);
}

#[test]
fn i_rifiuti_dell_identificatore_sono_errori_crs_senza_dati() {
    for rifiuto in [
        RifiutoIdentita::Malformata,
        RifiutoIdentita::Ambigua,
        RifiutoIdentita::Legata,
        RifiutoIdentita::Contraddetta,
    ] {
        let errore = rifiuto.errore("shp");
        assert_eq!(errore.category, crate::ErrorCategory::Crs);
        assert_eq!(errore.driver.as_deref(), Some("shp"));
        assert!(
            !errore.message.bytes().any(|b| b.is_ascii_digit()),
            "{}",
            errore.message
        );
    }
}

/// `sembra_wkt` decide quali definizioni vanno alla regola: le stesse due
/// condizioni con cui il lettore apre la radice, mai di meno.
#[test]
fn sembra_wkt_come_il_lettore_apre_la_radice() {
    for wkt in ["GEOGCS[", " \u{1c}geogcs (", "_x [", "BOUNDCRS\t["] {
        assert!(sembra_wkt(wkt), "{wkt:?}");
    }
    for altro in [
        "EPSG:3003",
        "<xml>",
        "OGC:CRS84",
        "",
        "1GEOGCS[",
        "\"GEOGCS\"[",
    ] {
        assert!(!sembra_wkt(altro), "{altro:?}");
    }
    // Un testo che il lettore legge ben formato sembra sempre un WKT.
    for testo in [ESRI_WGS84, BOUNDCRS, WKT1_3003, WKT2_4326] {
        assert!(grammatica_wkt(testo).is_some() && sembra_wkt(testo));
    }
}
