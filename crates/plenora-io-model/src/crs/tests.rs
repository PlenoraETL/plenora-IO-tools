//! Le prove di [`super`], in un file loro.
//!
//! Restano un **modulo figlio**: vedono i privati del genitore
//! esattamente come quando stavano dentro di lui, e non allargano di
//! una riga la superficie pubblica del crate.

use super::*;

/// Un WKT con caratteri multi-byte non deve far abortire il processo.
///
/// Trovato dal fuzzer (`gpkg_reader`) durante il checkpoint di livello 2
/// del 2026-08-20, su `8d6883f`. `wkt_root_epsg` scorreva **indici di
/// byte** e poi affettava la **stringa**: quando l'indice cadeva dentro un
/// carattere multi-byte, `upper[index..]` panicava con «is not a char
/// boundary». Nel bordo I/O un panic e' un abort — `libfuzzer-sys` lo
/// mostra come «deadly signal» — e la definizione di un CRS arriva dal
/// file, quindi era raggiungibile da un input ostile.
///
/// Il caso non e' esotico: basta un ideogramma dentro le parentesi di
/// primo livello, prima di dove un `AUTHORITY[` potrebbe comparire.
#[test]
fn una_definizione_wkt_multibyte_non_fa_panicare_il_bordo() {
    // L'ideogramma dell'input del fuzzer: tre byte, e l'indice ci finisce
    // dentro mentre si scorre il contenuto di primo livello.
    let ostile = "PROJCS[\u{9f5a}\u{9f5a}\u{9f5a},AUTHORITY[\"EPSG\",\"3857\"]]";
    assert_eq!(
        definition_authority_srid(ostile, CrsDefinitionFormat::Wkt),
        Some(3857),
        "l'identificatore radice resta leggibile anche con caratteri multi-byte"
    );

    // Un ideogramma spezzato a meta' dall'indice, senza identificatore:
    // deve dare `None`, non abortire.
    for definizione in [
        "PROJCS[\u{9f5a}]",
        "PROJCS[\u{9f5a}AUTHORITY]",
        "GEOGCS[\u{1f600}\u{1f600},ID[\"EPSG\",4326]]",
    ] {
        let _ = definition_authority_srid(definizione, CrsDefinitionFormat::Wkt);
    }
}

#[test]
fn authority_kind_classifies_wgs84_aliases_case_insensitively() {
    for id in ["OGC:CRS84", "ogc:crs84", "EPSG:4326", "epsg:4326"] {
        assert_eq!(crs_kind_for_authority_id(id), CrsKind::Geographic);
    }
}

#[test]
fn authority_kind_leaves_other_identifiers_unknown() {
    for id in ["EPSG:3857", "EPSG:3003", ""] {
        assert_eq!(crs_kind_for_authority_id(id), CrsKind::Unknown);
    }
}

#[test]
fn crs84_and_epsg_4326_keep_distinct_axis_orders() {
    let crs84 = ResolvedCrs::wgs84();
    let epsg4326 = ResolvedCrs::new(Some("EPSG:4326".to_owned()), CrsKind::Geographic, None);

    assert_eq!(crs84.axis_order, AxisOrder::LongitudeLatitude);
    assert_eq!(epsg4326.axis_order, AxisOrder::LatitudeLongitude);
    assert_ne!(crs84, epsg4326);
}

#[test]
fn projected_crs_declares_easting_northing() {
    let crs = ResolvedCrs::new(Some("EPSG:3857".to_owned()), CrsKind::Projected, None);
    assert_eq!(crs.axis_order, AxisOrder::EastingNorthing);
}

#[test]
fn raw_resolution_is_not_operational() {
    let raw = RawCrs::new("LOCAL_CS[\"private\"]".to_owned(), Some("LOCAL".to_owned()));
    let resolution = CrsResolution::DeclaredButUnresolved(raw.clone());
    assert_eq!(resolution.raw(), Some(&raw));
    assert!(resolution.as_resolved().is_none());
}

#[test]
fn unresolved_authority_does_not_require_or_invent_a_definition() {
    let raw = RawCrs::from_authority_hint("EPSG:99999".to_owned());
    let resolution = CrsResolution::DeclaredButUnresolved(raw.clone());

    assert_eq!(raw.authority_hint.as_deref(), Some("EPSG:99999"));
    assert_eq!(raw.definition, None);
    assert_eq!(raw.definition_format, None);
    assert_eq!(raw.axis_order, AxisOrder::Unknown);
    assert!(resolution.as_resolved().is_none());
}

#[test]
fn authority_srid_only_resolves_numeric_epsg_ids() {
    assert_eq!(authority_srid("EPSG:4326"), Some(4326));
    assert_eq!(authority_srid("epsg:3003"), Some(3003));
    assert_eq!(authority_srid("OGC:CRS84"), None);
    assert_eq!(authority_srid("EPSG:not-a-code"), None);
    assert_eq!(authority_srid("EPSG:4326:extra"), None);
}

#[test]
fn definition_format_is_explicit_and_not_inferred_by_consumers() {
    let wkt = ResolvedCrs::new(
        None,
        CrsKind::Geographic,
        Some("GEOGCS[\"WGS 84\"]".to_owned()),
    );
    let wkt2 = ResolvedCrs::new(
        None,
        CrsKind::Geographic,
        Some("GEOGCRS[\"WGS 84\"]".to_owned()),
    );
    let projjson = ResolvedCrs::new(
        None,
        CrsKind::Geographic,
        Some("{\"type\":\"GeographicCRS\"}".to_owned()),
    );
    assert_eq!(wkt.definition_format, Some(CrsDefinitionFormat::Wkt));
    assert_eq!(wkt2.definition_format, Some(CrsDefinitionFormat::Wkt2));
    assert_eq!(
        projjson.definition_format,
        Some(CrsDefinitionFormat::Projjson)
    );
}

#[test]
fn definition_epsg_uses_root_identifier_not_nested_base_crs() {
    let projected = concat!(
        "PROJCS[\"Monte Mario / Italy zone 1\",",
        "GEOGCS[\"Monte Mario\",AUTHORITY[\"EPSG\",\"4265\"]],",
        "AUTHORITY[\"EPSG\",\"3003\"]]"
    );
    assert_eq!(
        definition_authority_srid(projected, CrsDefinitionFormat::Wkt),
        Some(3003)
    );

    let wkt2 = concat!(
        "PROJCRS[\"WGS 84 / UTM zone 32N\",",
        "BASEGEOGCRS[\"WGS 84\",ID[\"EPSG\",4326]],",
        "ID[\"EPSG\",32632]]"
    );
    assert_eq!(
        definition_authority_srid(wkt2, CrsDefinitionFormat::Wkt2),
        Some(32632)
    );
}

#[test]
fn definition_epsg_reads_projjson_root_id_only() {
    let definition = r#"{
            "type":"ProjectedCRS",
            "base_crs":{"id":{"authority":"EPSG","code":4326}},
            "id":{"authority":"EPSG","code":3003}
        }"#;
    assert_eq!(
        definition_authority_srid(definition, CrsDefinitionFormat::Projjson),
        Some(3003)
    );
    assert_eq!(
        definition_authority_srid(
            r#"{"id":{"authority":"OGC","code":"CRS84"}}"#,
            CrsDefinitionFormat::Projjson
        ),
        None
    );
}

#[test]
fn definition_epsg_rejects_ambiguous_or_nested_only_ids() {
    assert_eq!(
        definition_authority_srid(
            "GEOGCS[\"unnamed\",DATUM[\"x\",AUTHORITY[\"EPSG\",\"6326\"]]]",
            CrsDefinitionFormat::Wkt
        ),
        None
    );
    assert_eq!(
        definition_authority_srid(
            "GEOGCS[\"x\",AUTHORITY[\"EPSG\",\"4326\"],ID[\"EPSG\",4326]]",
            CrsDefinitionFormat::Wkt
        ),
        None
    );
}
