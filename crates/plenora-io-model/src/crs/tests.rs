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
    // Il nome senza virgolette non e' un WKT ben formato per la grammatica del
    // contratto: nessuno SRID, e soprattutto nessun panic.
    let ostile = "PROJCS[\u{9f5a}\u{9f5a}\u{9f5a},AUTHORITY[\"EPSG\",\"3857\"]]";
    assert_eq!(
        definition_authority_srid(ostile, CrsDefinitionFormat::Wkt),
        None
    );
    let ben_formato = "PROJCS[\"\u{9f5a}\u{9f5a}\u{9f5a}\",AUTHORITY[\"EPSG\",\"3857\"]]";
    assert_eq!(
        definition_authority_srid(ben_formato, CrsDefinitionFormat::Wkt),
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

/// Il costruttore non deduce l'ordine dall'identificatore (GEO-002).
///
/// Prima `EPSG:4326` diventava `lat_lon`, l'ordine del registro, e
/// `EPSG:3857` `easting_northing` anche se nessuno lo aveva stabilito: la
/// prova si chiamava `crs84_and_epsg_4326_keep_distinct_axis_orders` e fissava
/// proprio la deduzione che il contratto vieta.
#[test]
fn il_costruttore_non_deduce_l_ordine_dal_registro() {
    for (id, kind) in [
        ("EPSG:4326", CrsKind::Geographic),
        ("OGC:CRS84", CrsKind::Geographic),
        ("EPSG:3857", CrsKind::Projected),
    ] {
        let crs = ResolvedCrs::new(Some(id.to_owned()), kind, None);
        assert_eq!(crs.axis_order, AxisOrder::Unknown, "{id}");
    }
    for raw in [
        RawCrs::new("GEOGCS[\"x\"]".to_owned(), Some("EPSG:4326".to_owned())),
        RawCrs::from_authority_hint("EPSG:4326".to_owned()),
    ] {
        assert_eq!(raw.axis_order, AxisOrder::Unknown);
    }
}

/// `EPSG:4326` memorizzato con la longitudine prima e' `lon_lat`, con o senza
/// la definizione che elenca la latitudine per prima (GEO-001).
#[test]
fn epsg_4326_memorizzato_est_prima_e_lon_lat() {
    let senza = ResolvedCrs::new(Some("EPSG:4326".to_owned()), CrsKind::Geographic, None)
        .stored_east_first();
    assert_eq!(senza.axis_order, AxisOrder::LongitudeLatitude);
    let wkt = r#"GEOGCS["WGS 84",DATUM["WGS_1984",SPHEROID["WGS 84",6378137,298.257223563]],PRIMEM["Greenwich",0],UNIT["degree",0.0174532925199433],AXIS["Latitude",NORTH],AXIS["Longitude",EAST],AUTHORITY["EPSG","4326"]]"#;
    let con = ResolvedCrs::new(
        Some("EPSG:4326".to_owned()),
        CrsKind::Geographic,
        Some(wkt.to_owned()),
    )
    .stored_east_first();
    assert_eq!(con.axis_order, AxisOrder::LongitudeLatitude);
    assert_eq!(
        ResolvedCrs::wgs84().axis_order,
        AxisOrder::LongitudeLatitude
    );
}

#[test]
fn proiettato_memorizzato_est_prima() {
    let utm = r#"PROJCS["WGS 84 / UTM zone 32N",GEOGCS["WGS 84",AUTHORITY["EPSG","4326"]],PROJECTION["Transverse_Mercator"],UNIT["metre",1],AXIS["Easting",EAST],AXIS["Northing",NORTH],AUTHORITY["EPSG","32632"]]"#;
    let crs = ResolvedCrs::new(
        Some("EPSG:32632".to_owned()),
        CrsKind::Projected,
        Some(utm.to_owned()),
    )
    .stored_east_first();
    assert_eq!(crs.axis_order, AxisOrder::EastingNorthing);
    // Gauss-Krueger: il registro elenca northing prima, il formato memorizza
    // comunque easting prima.
    let gk = r#"PROJCS["DHDN / 3-degree Gauss-Kruger zone 3",GEOGCS["DHDN"],PROJECTION["Transverse_Mercator"],UNIT["metre",1],AXIS["Northing",NORTH],AXIS["Easting",EAST],AUTHORITY["EPSG","31467"]]"#;
    assert_eq!(
        east_first_axis_order(CrsKind::Projected, Some((gk, CrsDefinitionFormat::Wkt))),
        AxisOrder::EastingNorthing
    );
}

/// Gli assi di EPSG:3031 puntano lungo i meridiani: `other` (GEO-001).
#[test]
fn proiezione_polare_e_other() {
    let gdal = r#"PROJCS["WGS 84 / Antarctic Polar Stereographic",GEOGCS["WGS 84"],PROJECTION["Polar_Stereographic"],PARAMETER["latitude_of_origin",-71],UNIT["metre",1],AXIS["Easting",NORTH],AXIS["Northing",NORTH],AUTHORITY["EPSG","3031"]]"#;
    let esri = r#"PROJCS["WGS_1984_Antarctic_Polar_Stereographic",GEOGCS["GCS_WGS_1984"],PROJECTION["Stereographic_South_Pole"],PARAMETER["standard_parallel_1",-71.0],UNIT["Meter",1.0]]"#;
    let wkt2 = r#"PROJCRS["WGS 84 / Antarctic Polar Stereographic",BASEGEOGCRS["WGS 84"],CONVERSION["x",METHOD["Polar Stereographic (variant B)"]],CS[Cartesian,2],AXIS["easting (X)",north,MERIDIAN[90,ANGLEUNIT["degree",0.0174532925199433]]],AXIS["northing (Y)",north,MERIDIAN[0,ANGLEUNIT["degree",0.0174532925199433]]],ID["EPSG",3031]]"#;
    let laea = r#"PROJCS["polar laea",GEOGCS["WGS 84"],PROJECTION["Lambert_Azimuthal_Equal_Area"],PARAMETER["latitude_of_center",90.000],UNIT["metre",1]]"#;
    for (testo, formato) in [
        (gdal, CrsDefinitionFormat::Wkt),
        (esri, CrsDefinitionFormat::Wkt),
        (wkt2, CrsDefinitionFormat::Wkt2),
        (laea, CrsDefinitionFormat::Wkt),
    ] {
        assert_eq!(
            east_first_axis_order(CrsKind::Projected, Some((testo, formato))),
            AxisOrder::Other,
            "{testo}"
        );
    }
}

/// Senza poter stabilire l'ordine, il produttore dichiara `unknown`.
#[test]
fn senza_elementi_l_ordine_e_unknown() {
    assert_eq!(
        east_first_axis_order(CrsKind::Projected, None),
        AxisOrder::Unknown,
        "un proiettato senza definizione potrebbe essere polare"
    );
    assert_eq!(
        east_first_axis_order(CrsKind::Unknown, None),
        AxisOrder::Unknown
    );
    for (testo, formato) in [
        ("PROJCS[", CrsDefinitionFormat::Wkt),
        (
            r#"COMPD_CS["x",PROJCS["y"],VERT_CS["z"]]"#,
            CrsDefinitionFormat::Wkt,
        ),
        (r#"PROJCRS["senza assi"]"#, CrsDefinitionFormat::Wkt2),
        (r#"{"type":"ProjectedCRS"}"#, CrsDefinitionFormat::Projjson),
    ] {
        assert_eq!(
            east_first_axis_order(CrsKind::Projected, Some((testo, formato))),
            AxisOrder::Unknown,
            "{testo}"
        );
    }
}

#[test]
fn projjson_epsg_4326_e_lon_lat() {
    let projjson = r#"{"type":"GeographicCRS","name":"WGS 84","coordinate_system":{"subtype":"ellipsoidal","axis":[{"name":"Geodetic latitude","abbreviation":"Lat","direction":"north","unit":"degree"},{"name":"Geodetic longitude","abbreviation":"Lon","direction":"east","unit":"degree"}]},"id":{"authority":"EPSG","code":4326}}"#;
    assert_eq!(
        east_first_axis_order(
            CrsKind::Geographic,
            Some((projjson, CrsDefinitionFormat::Projjson))
        ),
        AxisOrder::LongitudeLatitude
    );
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
    // `ID` in un WKT 1 non e' un identificatore per la grammatica del contratto
    // (GEO-005): la radice dichiara solo `AUTHORITY["EPSG","4326"]`.
    assert_eq!(
        definition_authority_srid(
            "GEOGCS[\"x\",AUTHORITY[\"EPSG\",\"4326\"],ID[\"EPSG\",4326]]",
            CrsDefinitionFormat::Wkt
        ),
        Some(4326)
    );
    // Dalla sesta seconda lettura: un CRS annidato fra parentesi tonde non
    // espone il suo identificatore come di radice, e `XID[` non e' `ID[`.
    assert_eq!(
        definition_authority_srid(
            "PROJCS[\"p\",GEOGCS(\"g\",DATUM[\"d\",SPHEROID[\"s\",6378388,297]],\
             PRIMEM[\"G\",0],UNIT[\"degree\",0.0174532925199433],AUTHORITY[\"EPSG\",\"4265\"]),\
             PROJECTION[\"Transverse_Mercator\"],UNIT[\"Meter\",1.0]]",
            CrsDefinitionFormat::Wkt
        ),
        None
    );
    assert_eq!(
        definition_authority_srid(
            "GEOGCRS[\"x\",XID[\"EPSG\",4326],ID[\"ESRI\",4265]]",
            CrsDefinitionFormat::Wkt2
        ),
        None
    );
}

/// GEO-005 confronta i codici come testo: un codice con zeri iniziali o con il
/// segno non e' il codice canonico, e non diventa uno SRID che lo farebbe
/// sembrare tale.
#[test]
fn authority_srid_solo_per_un_codice_canonico() {
    assert_eq!(authority_srid("EPSG:4326"), Some(4326));
    assert_eq!(authority_srid("epsg:0"), Some(0));
    for id in [
        "EPSG:04326",
        "EPSG:+4326",
        "EPSG:-4326",
        "EPSG:4326 ",
        "EPSG:",
        "EPSG:4326.0",
    ] {
        assert_eq!(authority_srid(id), None, "{id}");
    }
}

/// Nel WKT un codice **numerico** si legge come intero, come fa l'oracolo
/// (`+0043` vale 43); un codice **stringa** e' testo, e `"04326"` non e' 4326.
#[test]
fn definition_authority_srid_distingue_il_numero_dalla_stringa() {
    let wkt = |codice: &str| format!("GEOGCS[\"x\",AUTHORITY[\"EPSG\",{codice}]]");
    for (codice, atteso) in [
        ("\"4326\"", Some(4326)),
        ("4326", Some(4326)),
        ("04326", Some(4326)),
        ("+4326", Some(4326)),
        ("\"04326\"", None),
        ("\"+4326\"", None),
        ("4326.5", None),
        ("\"4326x\"", None),
    ] {
        assert_eq!(
            definition_authority_srid(&wkt(codice), CrsDefinitionFormat::Wkt),
            atteso,
            "{codice}"
        );
    }
}

/// Nel PROJJSON vale la stessa regola del WKT: un codice **numerico** e' un
/// intero, come per l'oracolo; un codice **stringa** e' testo. `"04326"` e
/// `"+4326"` davano 4326 per `str::parse`, mentre lo stesso codice in un WKT
/// dava `None`: due bordi, due verdetti sullo stesso testo.
#[test]
fn definition_authority_srid_projjson_distingue_il_numero_dalla_stringa() {
    let projjson = |codice: &str| format!(r#"{{"id":{{"authority":"EPSG","code":{codice}}}}}"#);
    for (codice, atteso) in [
        ("\"4326\"", Some(4326)),
        ("4326", Some(4326)),
        ("-0", Some(0)),
        ("\"04326\"", None),
        ("\"+4326\"", None),
        ("\"-4326\"", None),
        ("\" 4326\"", None),
        ("4326.0", None),
        ("-4326", None),
    ] {
        assert_eq!(
            definition_authority_srid(&projjson(codice), CrsDefinitionFormat::Projjson),
            atteso,
            "{codice}"
        );
    }
    // L'autorita' si confronta in ASCII: `EPſG` non e' EPSG.
    assert_eq!(
        definition_authority_srid(
            r#"{"id":{"authority":"EPſG","code":4326}}"#,
            CrsDefinitionFormat::Projjson
        ),
        None
    );
    // Un documento malformato per l'oracolo (chiave ripetuta) non da' SRID.
    assert_eq!(
        definition_authority_srid(
            r#"{"id":{"authority":"EPSG","code":4326,"code":3003}}"#,
            CrsDefinitionFormat::Projjson
        ),
        None
    );
}

/// GEO-005: l'autorita' si confronta senza maiuscole, il codice come testo, e
/// due autorita' diverse non sono mai lo stesso identificatore.
#[test]
fn stesso_identificatore_segue_geo_005() {
    assert!(stesso_identificatore("EPSG:3003", "epsg:3003"));
    assert!(stesso_identificatore("ogc:CRS84", "OGC:CRS84"));
    assert!(!stesso_identificatore("OGC:crs84", "OGC:CRS84"));
    assert!(!stesso_identificatore("EPSG:4326", "OGC:CRS84"));
    assert!(!stesso_identificatore("EPSG:04326", "EPSG:4326"));
    assert!(!stesso_identificatore("4326", "4326"));
}

/// Ogni stato del CRS dichiarato contro un'opzione: uguale passa, diverso,
/// non confrontabile o non applicato e' un errore `crs`, e il messaggio non
/// porta nessuno dei due identificatori.
#[test]
fn verifica_crs_assunto_rifiuta_cio_che_non_puo_confermare() {
    let risolto = CrsResolution::Resolved(ResolvedCrs::new(
        Some("EPSG:4326".to_owned()),
        CrsKind::Geographic,
        None,
    ));
    assert!(verifica_crs_assunto(None, &CrsResolution::Missing).is_ok());
    assert!(verifica_crs_assunto(Some("epsg:4326"), &risolto).is_ok());
    let senza_id = CrsResolution::Resolved(ResolvedCrs::new(None, CrsKind::Unknown, None));
    let grezzo =
        CrsResolution::DeclaredButUnresolved(RawCrs::from_authority_hint("EPSG:3003".to_owned()));
    assert!(verifica_crs_assunto(Some("EPSG:3003"), &grezzo).is_ok());
    for dichiarato in [&risolto, &senza_id, &grezzo, &CrsResolution::Missing] {
        let errore = verifica_crs_assunto(Some("EPSG:32632"), dichiarato).unwrap_err();
        assert_eq!(errore.category, crate::ErrorCategory::Crs);
        assert!(!errore.message.contains("32632"), "{}", errore.message);
        assert!(!errore.message.contains("4326"), "{}", errore.message);
    }
}

/// (g) Il formato WKT si decide con il lettore stretto, non con un prefisso
/// maiuscolo esatto: su a651844 `PROJCRS [`, `projcrs[` e `PROJECTEDCRS[`
/// erano `wkt`, e la verifica GEO-005 rifiutava una definizione ben formata.
#[test]
fn definition_format_decide_la_grammatica_con_il_lettore() {
    for wkt2 in [
        r#"PROJCRS ["x",ID["EPSG",3003]]"#,
        r#"projcrs["x"]"#,
        r#"PROJECTEDCRS["x"]"#,
        r#"GEOGRAPHICCRS("x")"#,
        r#" boundcrs [SOURCECRS[GEOGCRS["a"]],TARGETCRS[GEOGCRS["b"]]]"#,
    ] {
        assert_eq!(definition_format(wkt2), CrsDefinitionFormat::Wkt2, "{wkt2}");
    }
    for wkt1 in [r#"GEOGCS ["x"]"#, r#"projcs("x")"#] {
        assert_eq!(definition_format(wkt1), CrsDefinitionFormat::Wkt, "{wkt1}");
    }
    // Un testo malformato tiene l'etichetta del prefisso, come prima.
    assert_eq!(
        definition_format("PROJCRS[\"x\""),
        CrsDefinitionFormat::Wkt2
    );
    assert_eq!(
        definition_format("PROJCRS [\"x\""),
        CrsDefinitionFormat::Wkt
    );
}

/// La regola comune del genere: la definizione decide con la radice, un genere
/// ignoto della radice resta ignoto qualunque sia l'identificatore, senza
/// definizione decidono gli identificatori gia' noti, e un identificatore noto
/// di genere contrario alla radice e' `crs`. Il genere ignoto da' sempre un
/// ordine ignoto.
#[test]
fn il_genere_segue_la_radice_e_rifiuta_l_identificatore_contrario() {
    use CrsKind::{Geographic, Projected, Unknown};
    let utm =
        r#"PROJECTEDCRS["x",BASEGEOGCRS["y"],CS[Cartesian,2],AXIS["e",east],AXIS["n",north]]"#;
    for (id, definizione, atteso) in [
        (Some("EPSG:32632"), Some(utm), Projected),
        (Some("EPSG:4326"), None, Geographic),
        (Some("EPSG:3857"), None, Projected),
        (Some("EPSG:3003"), None, Unknown),
        (None, Some(r#"GEOGCS["x"]"#), Geographic),
        (Some("EPSG:4326"), Some(r#"GEOCCS["x"]"#), Unknown),
        (Some("EPSG:4326"), Some(r#"GEOGCS["x""#), Unknown),
        (Some("EPSG:3857"), Some(r#"PROJCS["x"]"#), Projected),
    ] {
        assert_eq!(
            crs_kind_for(id, definizione, "shp").ok(),
            Some(atteso),
            "{id:?} {definizione:?}"
        );
    }
    for (id, definizione) in [
        ("EPSG:4326", utm),
        ("OGC:CRS84", r#"PROJCS["x"]"#),
        ("EPSG:3857", r#"GEOGCRS["x"]"#),
    ] {
        let errore = crs_kind_for(Some(id), Some(definizione), "dxf").unwrap_err();
        assert_eq!(errore.code, crate::IoErrorCode::Crs);
        assert_eq!(errore.driver.as_deref(), Some("dxf"));
    }
    assert_eq!(
        east_first_axis_order(Unknown, Some((utm, CrsDefinitionFormat::Wkt2))),
        AxisOrder::Unknown
    );
}

/// GEO-005 e GEO-020 sul CRS emesso: una definizione malformata o contraddetta
/// e un catalogo locale risolto sono `crs`; senza definizione o concorde passa.
#[test]
fn verifica_crs_emesso_applica_geo_005() {
    let risolto = |id: &str, definizione: Option<&str>| {
        CrsResolution::Resolved(ResolvedCrs::new(
            Some(id.to_owned()),
            CrsKind::Unknown,
            definizione.map(str::to_owned),
        ))
    };
    assert!(verifica_crs_emesso(&CrsResolution::Missing).is_ok());
    assert!(verifica_crs_emesso(&risolto("EPSG:3003", None)).is_ok());
    assert!(verifica_crs_emesso(&risolto("EPSG:3003", Some(r#"PROJCS["x"]"#))).is_ok());
    for definizione in [
        "EPSG:3003",
        r#"PROJCS["x""#,
        r#"PROJCS["x",AUTHORITY["EPSG","3004"]]"#,
    ] {
        let errore = verifica_crs_emesso(&risolto("EPSG:3003", Some(definizione))).unwrap_err();
        assert_eq!(errore.code, crate::IoErrorCode::Crs, "{definizione}");
        assert!(!errore.message.contains("3004"), "{}", errore.message);
    }
    let non_risolto = CrsResolution::DeclaredButUnresolved(RawCrs::new(
        r#"PROJCS["x",AUTHORITY["EPSG","3004"]]"#.to_owned(),
        Some("EPSG:3003".to_owned()),
    ));
    assert!(verifica_crs_emesso(&non_risolto).is_err());
    // GEO-020: un catalogo locale si dichiara solo non risolto.
    assert!(verifica_crs_emesso(&risolto("postgis:4326", None)).is_err());
    let locale = CrsResolution::DeclaredButUnresolved(RawCrs::from_authority_hint(
        "POSTGIS:4326".to_owned(),
    ));
    assert!(verifica_crs_emesso(&locale).is_ok());
}
