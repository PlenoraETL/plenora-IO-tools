//! Le prove di [`super`], in un file loro.
//!
//! Restano un **modulo figlio**: vedono i privati del genitore
//! esattamente come quando stavano dentro di lui, e non allargano di
//! una riga la superficie pubblica del crate.

use super::*;

fn req() -> ReadRequest {
    ReadRequest {
        layer: LayerId(0),
        projected_fields: None,
        projection_mode: ProjectionMode::BestEffort,
        pruning_predicate: None,
        spatial_pruning_hint: None,
        scope: ReadScope::default(),
        batch_target: BatchTarget::default(),
        cancellation: CancellationToken::default(),
    }
}

/// Opzioni con tetto per cella e per feature configurabili.
fn opzioni_con_cella(byte: usize, feature: u64) -> ReadOptions {
    let limiti = plenora_io_model::budget::PipelineLimits::default()
        .with_max_wkb_cell_bytes(byte)
        .with_max_rows(feature);
    match plenora_io_model::budget::PipelineBudget::builder()
        .limits(limiti)
        .build()
    {
        Ok(bundle) => ReadOptions::from_read_parts(bundle.into_read_parts()),
        Err(error) => unreachable!("limiti di test non validi: {error:?}"),
    }
}

/// Opzioni con il tetto sui **componenti** configurato, e nient'altro.
fn opzioni_con_componenti(componenti: usize) -> ReadOptions {
    let limiti =
        plenora_io_model::budget::PipelineLimits::default().with_max_wkb_components(componenti);
    match plenora_io_model::budget::PipelineBudget::builder()
        .limits(limiti)
        .build()
    {
        Ok(bundle) => ReadOptions::from_read_parts(bundle.into_read_parts()),
        Err(error) => unreachable!("limiti di test non validi: {error:?}"),
    }
}

fn geojson_con_feature(dir: &tempfile::TempDir, quante: usize) -> std::path::PathBuf {
    let percorso = dir.path().join("input.geojson");
    let feature: Vec<String> = (0..quante)
            .map(|indice| {
                format!(
                    r#"{{"type":"Feature","properties":{{"id":{indice}}},"geometry":{{"type":"Point","coordinates":[1,2]}}}}"#
                )
            })
            .collect();
    std::fs::write(
        &percorso,
        format!(
            r#"{{"type":"FeatureCollection","features":[{}]}}"#,
            feature.join(",")
        ),
    )
    .expect("scrittura");
    percorso
}

/// Una `LineString` il cui JSON compatto e' piu' corto del WKB codificato.
///
/// Il JSON spende sei byte per punto (`[1,2],`), il WKB ne spende sedici:
/// due `f64`. Da tre punti in su la codifica supera il testo, e il
/// controllo sul testo grezzo smette di essere sufficiente.
fn geojson_con_linestring(dir: &tempfile::TempDir, punti: usize) -> std::path::PathBuf {
    let percorso = dir.path().join("linea.geojson");
    let coordinate: Vec<String> = (0..punti).map(|indice| format!("[{indice},2]")).collect();
    std::fs::write(
            &percorso,
            format!(
                r#"{{"type":"FeatureCollection","features":[{{"type":"Feature","properties":{{"id":0}},"geometry":{{"type":"LineString","coordinates":[{}]}}}}]}}"#,
                coordinate.join(",")
            ),
        )
        .expect("scrittura");
    percorso
}

/// Il JSON grezzo sta nel tetto, il WKB codificato no.
///
/// `GeoJSON` controlla `raw.get()` prima di deserializzare, ed e' il
/// controllo giusto per fermare un documento enorme senza costruire l'AST.
/// Ma non e' una maggiorazione della dimensione codificata: con dieci punti
/// il testo pesa meno del WKB, e fino a S5.1 il `Vec` cresceva comunque
/// oltre `max_wkb_cell_bytes`, con il rifiuto rimandato all'adapter.
#[test]
fn il_wkb_codificato_non_supera_il_tetto_anche_se_il_json_ci_sta() {
    const PUNTI: usize = 10;
    // Nove byte di intestazione WKB piu' sedici per punto.
    const WKB: usize = 9 + PUNTI * 16;

    let dir = tempfile::tempdir().expect("tempdir");
    let percorso = geojson_con_linestring(&dir, PUNTI);
    let documento = std::fs::read_to_string(&percorso).expect("lettura");
    let grezzo = documento
        .rfind(r#"{"type":"LineString""#)
        .map(|inizio| documento.len() - inizio - "}]}".len())
        .expect("la geometria e' nel documento");
    assert!(
        grezzo < WKB,
        "la premessa del test: il JSON ({grezzo}) deve stare sotto il WKB ({WKB})"
    );

    // Tetto fra le due grandezze: il testo passa, la codifica no.
    let soglia = WKB - 1;
    assert!(grezzo <= soglia, "il testo grezzo deve stare nel tetto");
    let dataset = GeoJsonDriver
        .open(Source::Path(percorso), opzioni_con_cella(soglia, 1_000))
        .expect("l'inferenza non tocca la codifica");
    let mut reader = dataset
        .open_layer_reader(&req())
        .expect("il reader si apre");
    let esito = reader.next_batch();
    assert!(
        matches!(
            esito,
            Err(ref errore) if errore.message.contains("oltre il limite")
        ),
        "la codifica WKB deve fermarsi al tetto: {esito:?}"
    );
}

/// Su errore il buffer resta vuoto, sul confine del driver.
///
/// Il test sopra osserva l'esito dal reader — un errore. Questo osserva lo
/// stato in cui il buffer viene lasciato, che e' la meta' del contratto
/// piu' facile da violare: un prefisso WKB parziale e' una sequenza ben
/// formata fino a dove arriva, quindi riutilizzabile per sbaglio.
///
/// Che il buffer non **cresca** oltre il tetto e' verificato sul sink, in
/// `plenora-io-model`, dove lo svuotamento non maschera la misura.
#[test]
fn wkb_from_gj_value_lascia_il_buffer_vuoto_su_errore() {
    let coordinate: Vec<geojson::Position> = (0..10)
        .map(|indice| geojson::Position::from([f64::from(indice), 2.0]))
        .collect();
    let valore = geojson::GeometryValue::LineString {
        coordinates: coordinate,
    };

    for soglia in [1_usize, 9, 40, 168] {
        let mut buffer = Vec::new();
        let esito = wkb_from_gj_value(&valore, &mut buffer, soglia);
        assert!(esito.is_err(), "tetto {soglia}: la codifica deve fallire");
        assert!(
            buffer.is_empty(),
            "tetto {soglia}: il buffer conserva {} byte di prefisso",
            buffer.len()
        );
    }

    // Anche il fallimento della conversione, prima di scrivere un byte,
    // lascia il buffer vuoto: la postcondizione non dipende da dove
    // l'errore e' nato.
    let mut buffer = vec![0xAA; 8];
    assert!(
        wkb_from_gj_value(
            &geojson::GeometryValue::LineString {
                coordinates: vec![]
            },
            &mut buffer,
            usize::MAX
        )
        .is_err(),
        "una LineString vuota e' rifiutata dalla conversione"
    );
    assert!(buffer.is_empty(), "il buffer preesistente non sopravvive");

    let mut buffer = Vec::new();
    wkb_from_gj_value(&valore, &mut buffer, 169).expect("al tetto esatto la codifica passa");
    assert_eq!(buffer.len(), 169);
}

/// La capability `hostile_input_hardened`, provata dove S12 la sposta.
///
/// Non e' il cap in byte: quello esisteva prima del parser progressivo e
/// scatta **prima** di deserializzare, quindi un test che lo esercita
/// resterebbe verde anche rimettendo il parser vecchio. Prova nulla di
/// questo lotto.
///
/// Qui l'input sta comodamente sotto il cap in byte, e a fermarlo e' il
/// tetto sui **componenti** -- l'unita' che solo un'analisi che addebita
/// mentre consuma puo' applicare. Le tre condizioni stanno insieme
/// apposta:
///
///   * con il tetto stretto il rifiuto e' esattamente `LimitExceeded`;
///   * con il default lo stesso identico input passa, quindi il rifiuto
///     viene dal tetto e non dall'input;
///   * l'input e' molto piu' corto del cap in byte, che percio' non
///     c'entra.
///
/// E' la prova che `check_capability_input_ostile.py` esegue per questo
/// driver: cancellarla, rinominarla o indebolirla rende rossa la
/// capability nel catalogo.
#[test]
fn la_geometria_e_rifiutata_per_componenti_sotto_il_cap_in_byte() {
    // Cinque posizioni, cioe' cinque componenti nell'unita' del bordo.
    const COMPONENTI: usize = 5;
    let geometria = r#"{"type":"LineString","coordinates":[[0,0],[1,1],[2,2],[3,3],[4,4]]}"#;

    let dir = tempfile::tempdir().expect("tempdir");
    let percorso = dir.path().join("linea.geojson");
    std::fs::write(
            &percorso,
            format!(
                r#"{{"type":"FeatureCollection","features":[{{"type":"Feature","properties":{{"id":1}},"geometry":{geometria}}}]}}"#
            ),
        )
        .expect("scrittura");

    assert!(
        geometria.len() < plenora_io_model::limits::WkbLimits::default().max_cell_bytes / 1_000,
        "l'input deve stare comodamente sotto il cap in byte"
    );

    let letto = |componenti: usize| {
        let dataset = GeoJsonDriver
            .open(
                Source::Path(percorso.clone()),
                opzioni_con_componenti(componenti),
            )
            .expect("l'inferenza non tocca il tetto sui componenti");
        let mut reader = dataset
            .open_layer_reader(&req())
            .expect("il reader si apre");
        reader.next_batch()
    };

    assert!(
        letto(COMPONENTI).is_ok(),
        "con {COMPONENTI} componenti di tetto la stessa geometria deve passare"
    );

    match letto(COMPONENTI - 1) {
        Err(errore) => assert_eq!(
            errore.code,
            plenora_io_model::IoErrorCode::LimitExceeded,
            "il rifiuto deve venire dal tetto sui componenti: {}",
            errore.message
        ),
        Ok(_) => panic!("una geometria oltre il tetto sui componenti deve fallire"),
    }
}

/// L'inferenza usa il tetto per cella **configurato**, non il default.
///
/// Fino a S5 la deserializzazione della geometria confrontava il testo
/// grezzo con `WkbLimits::default().max_cell_bytes` — 64 MiB — quindi
/// `--max-wkb-cell-bytes` non arrivava fin qui e una geometria oltre la
/// soglia richiesta veniva deserializzata comunque.
#[test]
fn inference_uses_configured_wkt_cell_bytes_not_default() {
    // Il testo grezzo della geometria e' `{"type":"Point","coordinates":[1,2]}`.
    const GEOMETRIA: usize = 38;

    let dir = tempfile::tempdir().expect("tempdir");
    let percorso = geojson_con_feature(&dir, 1);

    // Sotto soglia: passa.
    let dataset = GeoJsonDriver
        .open(
            Source::Path(percorso.clone()),
            opzioni_con_cella(GEOMETRIA + 64, 1_000),
        )
        .expect("una geometria dentro il tetto configurato deve passare");
    let mut reader = dataset
        .open_layer_reader(&req())
        .expect("il reader si apre");
    assert!(reader.next_batch().is_ok());

    // Sopra soglia: rifiutata prima di deserializzare.
    let dataset = GeoJsonDriver
        .open(Source::Path(percorso), opzioni_con_cella(8, 1_000))
        .expect("l'inferenza non tocca il testo grezzo della geometria");
    let mut reader = dataset
        .open_layer_reader(&req())
        .expect("il reader si apre");
    let esito = reader.next_batch();
    assert!(
        matches!(
            esito,
            Err(ref errore) if errore.code == plenora_io_model::IoErrorCode::LimitExceeded
                || errore.message.contains("oltre il limite")
        ),
        "una geometria oltre il tetto configurato deve fallire: {esito:?}"
    );
    assert!(
        8 < plenora_io_model::limits::WkbLimits::default().max_cell_bytes,
        "la soglia del test deve stare sotto il default, o non distinguerebbe nulla"
    );
}

/// La passata di inferenza e' bounded sulle feature visitate.
///
/// Non e' `max_input_entries`: quella governa l'enumerazione della
/// sorgente e il preflight l'ha gia' applicata al file. Vedi la nota su
/// `QuoteInferenza`.
#[test]
fn inference_respects_max_rows_before_materialising() {
    let dir = tempfile::tempdir().expect("tempdir");
    let percorso = geojson_con_feature(&dir, 8);

    assert!(GeoJsonDriver
        .open(Source::Path(percorso.clone()), opzioni_con_cella(4_096, 8))
        .is_ok());

    let esito = GeoJsonDriver.open(Source::Path(percorso), opzioni_con_cella(4_096, 7));
    let Err(errore) = esito else {
        unreachable!("l'inferenza deve fermarsi al tetto di feature");
    };
    assert!(
        errore.message.contains("inferenza"),
        "il messaggio deve dire dove ci si e' fermati: {}",
        errore.message
    );
}

/// Quote dell'inferenza per i test, dai limiti predefiniti della pipeline.
fn quote_di_prova() -> QuoteInferenza {
    QuoteInferenza::from_read_options(&opzioni_lettura())
}

/// Opzioni di lettura sul modello unificato.
///
/// Da S4.d il percorso di lettura vive interamente li': la memoria dei
/// batch e' una `InternalMemoryLease`, che esiste solo dentro un
/// `PipelineContext`. `opzioni_lettura()` costruisce ancora il ramo
/// legacy — sparira' in S4.e — e con quello `open` fallisce chiuso.
/// Opzioni di scrittura sul modello unificato.
///
/// `opzioni_scrittura()` non esiste piu' (S4.e): le opzioni portano un
/// `OperationBudget`, che nasce da una costruzione che puo' fallire.
fn opzioni_scrittura() -> WriteOptions {
    match plenora_io_model::budget::PipelineBudget::builder().build() {
        Ok(bundle) => WriteOptions::from_write_parts(bundle.into_write_parts()),
        Err(error) => unreachable!("bundle di test non costruibile: {error:?}"),
    }
}

fn opzioni_lettura() -> ReadOptions {
    match plenora_io_model::budget::PipelineBudget::builder().build() {
        Ok(bundle) => ReadOptions::from_read_parts(bundle.into_read_parts()),
        Err(error) => unreachable!("bundle di test non costruibile: {error:?}"),
    }
}

use plenora_io_core::request::{BatchTarget, ProjectionMode, ReadScope};
use plenora_io_core::WriteLayer;
use plenora_io_model::contract::{CoordinateDimensions, GeometryType};
use plenora_io_model::wkb::{from_wkb, WkbCoordinate, WkbValue};
use plenora_io_model::CancellationToken;
use std::fmt::Write as _;

fn read_all(driver: &GeoJsonDriver, path: &Path) -> (RecordBatch, LayerContract) {
    let ds = driver
        .open(Source::Path(path.to_owned()), opzioni_lettura())
        .unwrap();
    let layer = ds.layers()[0].clone();
    let mut reader = ds
        .open_layer_reader(&ReadRequest {
            layer: LayerId(0),
            projected_fields: None,
            projection_mode: ProjectionMode::BestEffort,
            pruning_predicate: None,
            spatial_pruning_hint: None,
            scope: ReadScope::default(),
            batch_target: BatchTarget::default(),
            cancellation: CancellationToken::default(),
        })
        .unwrap();
    let batch = reader.next_batch().unwrap().unwrap();
    (batch, layer)
}

#[test]
fn round_trip_geojson_recordbatch_geojson() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("in.geojson");
    std::fs::write(
            &src,
            r#"{"type":"FeatureCollection","features":[
            {"type":"Feature","geometry":{"type":"Point","coordinates":[12.5,45.9]},"properties":{"n":1,"s":"a","b":true}},
            {"type":"Feature","geometry":{"type":"LineString","coordinates":[[0,0],[1,1]]},"properties":{"n":2,"s":"b","b":false}}
            ]}"#,
        )
        .unwrap();

    let driver = GeoJsonDriver;
    let (batch, layer) = read_all(&driver, &src);
    assert_eq!(
        layer.contract.geometry.as_ref().unwrap().crs.id(),
        Some("OGC:CRS84")
    );
    assert_eq!(
        layer
            .contract
            .geometry
            .as_ref()
            .unwrap()
            .resolved_crs()
            .unwrap()
            .axis_order,
        plenora_io_model::crs::AxisOrder::LongitudeLatitude
    );
    assert_eq!(batch.num_rows(), 2);
    assert!(is_geometry_field(
        &batch.schema().field_with_name("geometry").unwrap().clone()
    ));

    // scrivi verso GeoJSON e rileggi
    let out = dir.path().join("out.geojson");
    let mut output_contract = layer.contract;
    output_contract
        .geometry
        .as_mut()
        .unwrap()
        .set_exact_geometry_types(vec![GeometryType::Point, GeometryType::LineString]);
    let plan = WritePlan {
        layers: vec![WriteLayer {
            name: "l".to_owned(),
            contract: output_contract,
        }],
    };
    let mut w = driver
        .create(Sink::Path(out.clone()), &plan, &opzioni_scrittura())
        .unwrap();
    w.write(&batch).unwrap();
    w.finish().unwrap();

    let features = parse_features(&std::fs::read_to_string(&out).unwrap()).unwrap();
    assert_eq!(features.len(), 2);
    assert_eq!(
        features[0].properties.as_ref().unwrap().get("s").unwrap(),
        "a"
    );
    match &features[0].geometry.as_ref().unwrap().value {
        geojson::GeometryValue::Point { coordinates } => {
            assert!((coordinates[0] - 12.5).abs() < 1e-9);
        }
        other => panic!("atteso Point, {other:?}"),
    }
}

#[test]
fn integer_outside_i64_is_preserved_as_text() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("wide-integer.geojson");
    std::fs::write(
        &source,
        r#"{"type":"FeatureCollection","features":[
            {"type":"Feature","geometry":null,"properties":{"identifier":18446744073709551615}}
            ]}"#,
    )
    .unwrap();

    let (batch, _) = read_all(&GeoJsonDriver, &source);
    let identifier = batch
        .column(batch.schema().index_of("identifier").unwrap())
        .as_any()
        .downcast_ref::<StringArray>()
        .unwrap();

    assert_eq!(identifier.value(0), "18446744073709551615");
}

#[test]
fn heterogeneous_features_align_columns() {
    // Feature con proprietà disomogenee: chiave mancante, properties null,
    // chiave sconosciuta, geometria null. Il deserializer custom deve
    // mantenere una append per builder per feature (colonne allineate).
    use arrow_array::{Int64Array, StringArray};
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("het.geojson");
    std::fs::write(
            &src,
            r#"{"type":"FeatureCollection","features":[
            {"type":"Feature","geometry":{"type":"Point","coordinates":[1,1]},"properties":{"a":1,"b":"x"}},
            {"type":"Feature","geometry":null,"properties":{"a":2}},
            {"type":"Feature","geometry":{"type":"Point","coordinates":[3,3]},"properties":null},
            {"type":"Feature","geometry":{"type":"Point","coordinates":[4,4]},"properties":{"b":"y","c":99}}
            ]}"#,
        )
        .unwrap();

    let driver = GeoJsonDriver;
    let (batch, _layer) = read_all(&driver, &src);
    assert_eq!(batch.num_rows(), 4);
    let schema = batch.schema();

    let geom = batch
        .column(0)
        .as_any()
        .downcast_ref::<BinaryArray>()
        .unwrap();
    assert!(!geom.is_null(0) && geom.is_null(1) && !geom.is_null(2) && !geom.is_null(3));

    let col = |name: &str| schema.index_of(name).unwrap();
    let a = batch
        .column(col("a"))
        .as_any()
        .downcast_ref::<Int64Array>()
        .unwrap();
    assert_eq!(a.value(0), 1);
    assert_eq!(a.value(1), 2);
    assert!(a.is_null(2) && a.is_null(3));

    let b = batch
        .column(col("b"))
        .as_any()
        .downcast_ref::<StringArray>()
        .unwrap();
    assert_eq!(b.value(0), "x");
    assert!(b.is_null(1) && b.is_null(2));
    assert_eq!(b.value(3), "y");

    let c = batch
        .column(col("c"))
        .as_any()
        .downcast_ref::<Int64Array>()
        .unwrap();
    assert!(c.is_null(0) && c.is_null(1) && c.is_null(2));
    assert_eq!(c.value(3), 99);
}

#[test]
fn source_property_order_does_not_change_inferred_schema_order() {
    let dir = tempfile::tempdir().unwrap();
    let za = dir.path().join("za.geojson");
    let az = dir.path().join("az.geojson");
    std::fs::write(
        &za,
        r#"{"type":"FeatureCollection","features":[
            {"type":"Feature","geometry":null,"properties":{"z":1,"a":"x"}}
            ]}"#,
    )
    .unwrap();
    std::fs::write(
        &az,
        r#"{"type":"FeatureCollection","features":[
            {"type":"Feature","geometry":null,"properties":{"a":"x","z":1}}
            ]}"#,
    )
    .unwrap();

    let (za_schema, za_columns, _) = infer_schema(&za, quote_di_prova()).unwrap();
    let (az_schema, az_columns, _) = infer_schema(&az, quote_di_prova()).unwrap();
    assert_eq!(za_schema, az_schema);
    assert_eq!(za_columns, az_columns);
    assert_eq!(
        za_schema
            .fields()
            .iter()
            .map(|field| field.name().as_str())
            .collect::<Vec<_>>(),
        ["geometry", "a", "z"]
    );
}

#[test]
fn duplicate_keys_are_rejected_instead_of_using_first_value_wins() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("dup.geojson");
    std::fs::write(
            &src,
            r#"{"type":"FeatureCollection","features":[
            {"type":"Feature","geometry":{"type":"Point","coordinates":[1,2]},"geometry":{"type":"Point","coordinates":[9,9]},"properties":{"c":1,"b":"x","c":true}}
            ]}"#,
        )
        .unwrap();
    let driver = GeoJsonDriver;
    let dataset = driver.open(Source::Path(src), opzioni_lettura()).unwrap();
    let mut reader = dataset
        .open_layer_reader(&ReadRequest {
            layer: LayerId(0),
            projected_fields: None,
            projection_mode: ProjectionMode::BestEffort,
            pruning_predicate: None,
            spatial_pruning_hint: None,
            scope: ReadScope::default(),
            batch_target: BatchTarget::default(),
            cancellation: CancellationToken::default(),
        })
        .unwrap();
    let error = reader.next_batch().unwrap_err();
    assert_eq!(error.category, plenora_io_model::ErrorCategory::DataMapping);
    assert!(error.message.contains("geometry duplicata"));
    let diagnostics = error.row_diagnostics.as_deref().unwrap();
    assert_eq!(diagnostics.examples[0].source_index, 0);
    assert_eq!(diagnostics.counts["geojson.invalid_feature"], 1);
    assert!(diagnostics.validate().is_ok());
}

#[test]
fn duplicate_property_outside_projection_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("dup-outside-projection.geojson");
    std::fs::write(
            &src,
            r#"{"type":"FeatureCollection","features":[
            {"type":"Feature","geometry":{"type":"Point","coordinates":[1,2]},"properties":{"kept":1,"outside":"first","outside":"second"}}
            ]}"#,
        )
        .unwrap();
    let driver = GeoJsonDriver;
    let dataset = driver.open(Source::Path(src), opzioni_lettura()).unwrap();
    let mut reader = dataset
        .open_layer_reader(&ReadRequest {
            layer: LayerId(0),
            projected_fields: Some(vec![FieldId(0)]),
            projection_mode: ProjectionMode::Required,
            pruning_predicate: None,
            spatial_pruning_hint: None,
            scope: ReadScope::default(),
            batch_target: BatchTarget::default(),
            cancellation: CancellationToken::default(),
        })
        .unwrap();

    let error = reader.next_batch().unwrap_err();
    assert_eq!(error.category, plenora_io_model::ErrorCategory::DataMapping);
    assert_eq!(error.phase, plenora_io_model::ErrorPhase::Read);
    assert!(error.message.contains("chiave duplicata nelle properties"));
    let diagnostics = error.row_diagnostics.as_deref().unwrap();
    assert_eq!(diagnostics.examples[0].source_index, 0);
    assert_eq!(diagnostics.counts["geojson.invalid_feature"], 1);
    assert!(diagnostics.validate().is_ok());
}

#[test]
fn polygon_and_multipolygon_round_trip() {
    // Esercita la conversione geometria DIRETTA in entrambe le direzioni:
    // lettura geojson→WKB (wkb_from_gj_value) e scrittura WKB→JSON
    // (write_geo_geojson), su Polygon-con-buco e MultiPolygon.
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("poly.geojson");
    std::fs::write(
            &src,
            r#"{"type":"FeatureCollection","features":[
            {"type":"Feature","geometry":{"type":"Polygon","coordinates":[[[0,0],[4,0],[4,4],[0,4],[0,0]],[[1,1],[2,1],[2,2],[1,2],[1,1]]]},"properties":{"k":1}},
            {"type":"Feature","geometry":{"type":"MultiPolygon","coordinates":[[[[0,0],[1,0],[1,1],[0,0]]],[[[5,5],[6,5],[6,6],[5,5]]]]},"properties":{"k":2}}
            ]}"#,
        )
        .unwrap();

    let driver = GeoJsonDriver;
    let (batch, layer) = read_all(&driver, &src);
    assert_eq!(batch.num_rows(), 2);

    // Lettura: geojson→WKB deve dare un WKB decodificabile da from_wkb.
    let geom = batch
        .column(0)
        .as_any()
        .downcast_ref::<BinaryArray>()
        .unwrap();
    let limits = WkbLimits::default();
    match from_wkb(geom.value(0), &limits).unwrap() {
        geo_types::Geometry::Polygon(pl) => {
            assert_eq!(pl.exterior().0.len(), 5);
            assert_eq!(pl.interiors().len(), 1);
            assert_eq!(pl.interiors()[0].0.len(), 5);
        }
        other => panic!("atteso Polygon, {other:?}"),
    }
    match from_wkb(geom.value(1), &limits).unwrap() {
        geo_types::Geometry::MultiPolygon(mp) => assert_eq!(mp.0.len(), 2),
        other => panic!("atteso MultiPolygon, {other:?}"),
    }

    // Scrittura: WKB→JSON diretto; rileggendo la geometria deve sopravvivere.
    let out = dir.path().join("poly-out.geojson");
    let mut output_contract = layer.contract;
    output_contract
        .geometry
        .as_mut()
        .unwrap()
        .set_exact_geometry_types(vec![GeometryType::Polygon, GeometryType::MultiPolygon]);
    let plan = WritePlan {
        layers: vec![WriteLayer {
            name: "l".to_owned(),
            contract: output_contract,
        }],
    };
    let mut w = driver
        .create(Sink::Path(out.clone()), &plan, &opzioni_scrittura())
        .unwrap();
    w.write(&batch).unwrap();
    w.finish().unwrap();

    let feats = parse_features(&std::fs::read_to_string(&out).unwrap()).unwrap();
    assert_eq!(feats.len(), 2);
    match &feats[0].geometry.as_ref().unwrap().value {
        geojson::GeometryValue::Polygon { coordinates: rings } => {
            assert_eq!(rings.len(), 2); // esterno + 1 buco
            assert_eq!(rings[0].len(), 5);
            assert_eq!(rings[1].len(), 5);
            assert!((rings[1][0][0] - 1.0).abs() < 1e-9); // il buco parte da x=1
        }
        other => panic!("atteso Polygon, {other:?}"),
    }
    match &feats[1].geometry.as_ref().unwrap().value {
        geojson::GeometryValue::MultiPolygon { coordinates: polys } => {
            assert_eq!(polys.len(), 2);
        }
        other => panic!("atteso MultiPolygon, {other:?}"),
    }
}

#[test]
fn xyz_round_trip_preserves_altitude() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("xyz.geojson");
    let output = dir.path().join("xyz-out.geojson");
    std::fs::write(
            &input,
            r#"{"type":"FeatureCollection","features":[
                {"type":"Feature","geometry":{"type":"Point","coordinates":[12.5,45.9,123.25]},"properties":{"name":"quota"}}
            ]}"#,
        )
        .unwrap();

    let driver = GeoJsonDriver;
    let (batch, layer) = read_all(&driver, &input);
    assert_eq!(
        layer.contract.geometry.as_ref().unwrap().dimensions,
        CoordinateDimensions::Unknown
    );
    let geometry = batch
        .column(0)
        .as_any()
        .downcast_ref::<BinaryArray>()
        .unwrap();
    let decoded = decode_wkb(geometry.value(0), &WkbLimits::default()).unwrap();
    assert_eq!(decoded.dimensions, CoordinateDimensions::Xyz);
    assert!(matches!(
        decoded.value,
        WkbValue::Point(WkbCoordinate {
            z: Some(123.25),
            ..
        })
    ));

    let mut output_contract = layer.contract;
    output_contract
        .geometry
        .as_mut()
        .unwrap()
        .set_exact_geometry_types(vec![GeometryType::Point]);
    let plan = WritePlan {
        layers: vec![WriteLayer {
            name: "xyz".to_owned(),
            contract: output_contract,
        }],
    };
    let mut writer = driver
        .create(Sink::Path(output.clone()), &plan, &opzioni_scrittura())
        .unwrap();
    writer.write(&batch).unwrap();
    writer.finish().unwrap();

    let text = std::fs::read_to_string(&output).unwrap();
    assert!(text.contains("[12.5,45.9,123.25]"));
    let (round_trip, _) = read_all(&driver, &output);
    let geometry = round_trip
        .column(0)
        .as_any()
        .downcast_ref::<BinaryArray>()
        .unwrap();
    let decoded = decode_wkb(geometry.value(0), &WkbLimits::default()).unwrap();
    assert_eq!(decoded.dimensions, CoordinateDimensions::Xyz);
}

#[test]
fn fourth_geojson_ordinate_is_rejected() {
    let mut output = Vec::new();
    let result = wkb_from_gj_value(
        &geojson::GeometryValue::Point {
            coordinates: geojson::Position::from(vec![1.0, 2.0, 3.0, 4.0]),
        },
        &mut output,
        usize::MAX,
    );
    assert!(result.is_err());
}

#[test]
fn empty_geometry_does_not_invent_xy_dimensions() {
    let mut output = Vec::new();
    assert!(wkb_from_gj_value(
        &geojson::GeometryValue::LineString {
            coordinates: vec![]
        },
        &mut output,
        usize::MAX
    )
    .is_err());
    assert!(wkb_from_gj_value(
        &geojson::GeometryValue::GeometryCollection { geometries: vec![] },
        &mut output,
        usize::MAX
    )
    .is_err());
}

#[test]
fn extreme_coordinate_survives_geojson_write() {
    // Regressione (trovato dal fuzzer): una coordinata f64 estrema deve
    // produrre JSON RI-LEGGIBILE (serde_json/ryu), non un decimale che
    // serde_json rifiuta in rilettura ("number out of range").
    let mut buf = Vec::new();
    let g = geo_types::Geometry::Point(geo_types::Point::new(f64::MAX, -1.5));
    write_geo_geojson(&mut buf, &g).unwrap();
    let parsed: geojson::Geometry = serde_json::from_slice(&buf).unwrap();
    match parsed.value {
        geojson::GeometryValue::Point { coordinates: c } => {
            // Il round-trip deve restituire f64::MAX identico bit a bit:
            // il confronto esatto è il contratto della regressione.
            #[allow(clippy::float_cmp)]
            {
                assert_eq!(c[0], f64::MAX);
            }
            assert!((c[1] + 1.5).abs() < 1e-9);
        }
        other => panic!("atteso Point, {other:?}"),
    }
}

#[test]
fn streams_multiple_batches() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("many.geojson");
    let mut s = String::from("{\"type\":\"FeatureCollection\",\"features\":[");
    for i in 0..10 {
        if i > 0 {
            s.push(',');
        }
        write!(
                s,
                "{{\"type\":\"Feature\",\"geometry\":{{\"type\":\"Point\",\"coordinates\":[{i},{i}]}},\"properties\":{{\"id\":{i}}}}}"
            )
            .unwrap();
    }
    s.push_str("]}");
    std::fs::write(&src, s).unwrap();

    let driver = GeoJsonDriver;
    let ds = driver.open(Source::Path(src), opzioni_lettura()).unwrap();
    let mut reader = ds
        .open_layer_reader(&ReadRequest {
            layer: LayerId(0),
            projected_fields: None,
            projection_mode: ProjectionMode::BestEffort,
            pruning_predicate: None,
            spatial_pruning_hint: None,
            scope: ReadScope::default(),
            batch_target: BatchTarget {
                target_bytes: 8 * 1024 * 1024,
                max_rows: 4,
            },
            cancellation: CancellationToken::default(),
        })
        .unwrap();
    let mut total = 0;
    let mut batches = 0;
    while let Some(b) = reader.next_batch().unwrap() {
        total += b.num_rows();
        batches += 1;
    }
    assert_eq!(total, 10);
    assert!(
        batches >= 3,
        "atteso streaming multi-batch, avuti {batches}"
    );
}
