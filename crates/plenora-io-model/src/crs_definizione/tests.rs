//! Le prove di [`super`], in un file loro.

use super::*;

const WKT1_3003: &str = r#"PROJCS["Monte Mario / Italy zone 1",GEOGCS["Monte Mario",AUTHORITY["EPSG","4265"]],AUTHORITY["EPSG","3003"]]"#;
const WKT2_4326: &str = r#"GEOGCRS["WGS 84",ID["EPSG",4326]]"#;

#[test]
fn identificatori_di_primo_livello_nei_tre_formati() {
    assert_eq!(
        identificatori(WKT1_3003, CrsDefinitionFormat::Wkt),
        Ok(vec![("EPSG".to_owned(), "3003".to_owned())])
    );
    assert_eq!(
        identificatori(WKT2_4326, CrsDefinitionFormat::Wkt2),
        Ok(vec![("EPSG".to_owned(), "4326".to_owned())])
    );
    let projjson = r#"{"type":"GeographicCRS","ids":[{"authority":"EPSG","code":4326},{"authority":"OGC","code":"CRS84"}]}"#;
    assert_eq!(
        identificatori(projjson, CrsDefinitionFormat::Projjson),
        Ok(vec![
            ("EPSG".to_owned(), "4326".to_owned()),
            ("OGC".to_owned(), "CRS84".to_owned())
        ])
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
