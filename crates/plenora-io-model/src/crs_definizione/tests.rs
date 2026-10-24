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
