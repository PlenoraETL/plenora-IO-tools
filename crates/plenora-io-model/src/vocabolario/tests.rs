//! Le prove di [`super`], in un file loro.

use std::collections::HashMap;

use arrow_schema::{DataType, Field, Schema};

use super::*;
use crate::error::ErrorCategory;

fn geometria_conforme() -> HashMap<String, String> {
    [
        ("plenora.field_id", "1"),
        ("ARROW:extension:name", "geoarrow.wkb"),
        ("plenora.geometry.encoding", "wkb"),
        ("plenora.geometry.dimensions", "xy"),
        ("plenora.geometry.spatial_semantics", "geometry"),
        ("plenora.geometry.precision", "float64"),
        ("plenora.geometry.types_declaration", "exact"),
        ("plenora.geometry.types", "point"),
        ("plenora.geometry.crs_resolution", "resolved"),
        ("plenora.geometry.crs_id", "EPSG:4326"),
        ("plenora.geometry.axis_order", "lon_lat"),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_owned(), v.to_owned()))
    .collect()
}

fn schema(versione: Option<&str>, geometria: HashMap<String, String>) -> Schema {
    let campi = vec![
        Field::new("id", DataType::Int64, false).with_metadata(HashMap::from([(
            "plenora.field_id".to_owned(),
            "0".to_owned(),
        )])),
        Field::new("geometry", DataType::Binary, true).with_metadata(geometria),
    ];
    let md: HashMap<String, String> = versione
        .map(|v| HashMap::from([("plenora.contract.version".to_owned(), v.to_owned())]))
        .unwrap_or_default();
    Schema::new_with_metadata(campi, md)
}

fn categoria(s: &Schema) -> Option<ErrorCategory> {
    verifica_schema(s).err().map(|e| e.category)
}

fn con(cambia: &[(&str, Option<&str>)]) -> Schema {
    let mut md = geometria_conforme();
    for (k, v) in cambia {
        match v {
            Some(v) => md.insert((*k).to_owned(), (*v).to_owned()),
            None => md.remove(*k),
        };
    }
    schema(Some("1"), md)
}

#[test]
fn conforme_e_arrow_estraneo_passano() {
    assert_eq!(categoria(&schema(Some("1"), geometria_conforme())), None);
    let estraneo = Schema::new(vec![Field::new("x", DataType::Int64, false)]);
    assert_eq!(categoria(&estraneo), None);
}

#[test]
fn versione() {
    assert_eq!(
        categoria(&schema(None, geometria_conforme())),
        Some(ErrorCategory::Schema)
    );
    for malformata in ["", "01", "1.0", "uno", "-1", "0"] {
        assert_eq!(
            categoria(&schema(Some(malformata), geometria_conforme())),
            Some(ErrorCategory::Schema),
            "{malformata}"
        );
    }
    assert_eq!(
        categoria(&schema(Some("2"), geometria_conforme())),
        Some(ErrorCategory::Unsupported)
    );
}

#[test]
fn vocabolario_e_schema() {
    for cambia in [
        vec![("plenora.geometry.precision", None)],
        vec![("plenora.geometry.spatial_semantics", None)],
        vec![("plenora.field_id", None)],
        vec![("plenora.field_id", Some("01"))],
        vec![("plenora.geometry.encoding", Some("wkt"))],
        vec![("plenora.geometry.types", Some("polygon,point"))],
        vec![("plenora.geometry.types", Some("point,point"))],
        vec![("plenora.geometry.types", Some(""))],
        vec![("plenora.geometry.types", None)],
        vec![("plenora.geometry.srid", Some("+1"))],
        vec![("plenora.geometry.srid", Some("2147483648"))],
        vec![("plenora.geometry.inventata", Some("x"))],
    ] {
        assert_eq!(
            categoria(&con(&cambia)),
            Some(ErrorCategory::Schema),
            "{cambia:?}"
        );
    }
    assert_eq!(
        categoria(&con(&[("plenora.geometry.native.qualcosa", Some("x"))])),
        None
    );
}

#[test]
fn crs_e_crs_anche_quando_manca() {
    for cambia in [
        vec![("plenora.geometry.crs_resolution", None)],
        vec![("plenora.geometry.axis_order", None)],
        vec![("plenora.geometry.axis_order", Some("nord_est"))],
        vec![("plenora.geometry.crs_id", Some("4326"))],
        vec![("plenora.geometry.crs_id", Some("POSTGIS:4326"))],
        vec![("plenora.geometry.crs_definition", Some(""))],
        vec![
            (
                "plenora.geometry.crs_definition",
                Some(r#"GEOGCRS["x",ID["EPSG",3857]]"#),
            ),
            ("plenora.geometry.crs_definition_format", Some("wkt2")),
        ],
        vec![
            ("plenora.geometry.crs_definition", Some("GEOGCRS[")),
            ("plenora.geometry.crs_definition_format", Some("wkt2")),
        ],
        vec![("plenora.geometry.crs_definition_format", Some("wkt2"))],
    ] {
        assert_eq!(
            categoria(&con(&cambia)),
            Some(ErrorCategory::Crs),
            "{cambia:?}"
        );
    }
}

#[test]
fn precedenza_versione_vocabolario_crs_supporto() {
    // versione prima del CRS
    let mut md = geometria_conforme();
    md.remove("plenora.geometry.axis_order");
    assert_eq!(
        categoria(&schema(Some("2"), md.clone())),
        Some(ErrorCategory::Unsupported)
    );
    // vocabolario prima del CRS
    md.remove("plenora.geometry.precision");
    assert_eq!(
        categoria(&schema(Some("1"), md)),
        Some(ErrorCategory::Schema)
    );
    // CRS prima del supporto
    let mut seconda = geometria_conforme();
    seconda.insert("plenora.field_id".to_owned(), "2".to_owned());
    seconda.remove("plenora.geometry.axis_order");
    let due = Schema::new_with_metadata(
        vec![
            Field::new("g1", DataType::Binary, true).with_metadata(geometria_conforme()),
            Field::new("g2", DataType::Binary, true).with_metadata(seconda.clone()),
        ],
        HashMap::from([("plenora.contract.version".to_owned(), "1".to_owned())]),
    );
    assert_eq!(categoria(&due), Some(ErrorCategory::Crs));
    seconda.insert(
        "plenora.geometry.axis_order".to_owned(),
        "lon_lat".to_owned(),
    );
    let due = Schema::new_with_metadata(
        vec![
            Field::new("g1", DataType::Binary, true).with_metadata(geometria_conforme()),
            Field::new("g2", DataType::Binary, true).with_metadata(seconda),
        ],
        HashMap::from([("plenora.contract.version".to_owned(), "1".to_owned())]),
    );
    assert_eq!(categoria(&due), None);
}

/// GEO-016: il codice di `crs_id` non ha spazi nel senso di `\s` di Python, che
/// comprende U+001C..U+001F; un carattere che Python non conta fra gli spazi
/// (U+200B, U+FEFF, U+180E) resta un carattere del codice.
#[test]
fn crs_id_con_gli_spazi_di_python() {
    for spazio in [
        '\u{1c}', '\u{1d}', '\u{1e}', '\u{1f}', '\u{85}', '\u{a0}', '\u{3000}',
    ] {
        let id = format!("EPSG:4326{spazio}");
        assert_eq!(
            categoria(&con(&[("plenora.geometry.crs_id", Some(&id))])),
            Some(ErrorCategory::Crs),
            "U+{:04X}",
            u32::from(spazio)
        );
    }
    for carattere in ['\u{200b}', '\u{feff}', '\u{180e}'] {
        let id = format!("EPSG:4326{carattere}");
        assert_eq!(
            categoria(&con(&[("plenora.geometry.crs_id", Some(&id))])),
            None,
            "U+{:04X}",
            u32::from(carattere)
        );
    }
}

/// Una definizione WKT con U+001F fra i token e' ben formata per l'oracolo, e
/// un PROJJSON con un numero oltre `f64` pure (GEO-005).
#[test]
fn definizioni_ben_formate_per_l_oracolo() {
    for (definizione, formato) in [
        ("GEOGCS[\"x\",\u{1f}AUTHORITY[\"EPSG\",4326]]", "wkt"),
        (
            r#"{"type":"GeographicCRS","x":1e400,"id":{"authority":"EPSG","code":4326}}"#,
            "projjson",
        ),
        (
            r#"{"type":"GeographicCRS","name":"\ud800","id":{"authority":"EPSG","code":4326}}"#,
            "projjson",
        ),
    ] {
        let schema = con(&[
            ("plenora.geometry.crs_definition", Some(definizione)),
            ("plenora.geometry.crs_definition_format", Some(formato)),
        ]);
        assert_eq!(categoria(&schema), None, "{definizione:.60}");
    }
}
