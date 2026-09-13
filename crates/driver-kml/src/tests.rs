//! Le prove di [`super`], in un file loro.
//!
//! Restano un **modulo figlio**: vedono i privati del genitore
//! esattamente come quando stavano dentro di lui, e non allargano di
//! una riga la superficie pubblica del crate.

use super::*;

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
use plenora_io_model::wkb::to_wkb;

const SAMPLE: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
    <kml xmlns="http://www.opengis.net/kml/2.2"><Document>
      <Placemark><name>A</name><description>primo</description>
        <Point><coordinates>12.5,45.9,0</coordinates></Point></Placemark>
      <Placemark><name>B</name>
        <LineString><coordinates>0,0,0 1,1,0</coordinates></LineString></Placemark>
    </Document></kml>"#;

const FUZZ_TIMEOUT_REGRESSION: &[u8] = br#"<kml xmlns="http://www.opengis.net/kml/2.2"><Placemark><MultiGeomgis.net/kml/2.2"><Placemark><MultiGeometry>></LikeString></MultiGww.opengis.net/kml/2.2etry>></LikeString></MultiGww.opengis.net/kml/2.2"><>"#;
const FUZZ_EMPTY_POINT_REGRESSION: &[u8] =
    br#"<kml xmlns="httpw.opengis.net/kml/2.2"><Placemark><Point></Point></Placemark></kml>"#;

/// Il caso del 2026-09-09: 314 byte che la guardia leggeva in un'altra
/// codifica rispetto al parser che protegge.
///
/// I primi quattro byte sono `3C 00 3F 00`, cioe' `<?` in UTF-16LE, e
/// l'appendice F della specifica XML dice proprio di dedurne quella
/// codifica in assenza di BOM. Letto cosi' il documento e' due eventi con
/// lo stack vuoto, e la guardia lo lasciava passare; `Kml::from_str` lo
/// leggeva come UTF-8, ci trovava quattro `<xjA:Point>` mai chiusi, e non
/// tornava piu' -- oltre centodieci secondi in locale, contro i ventuno
/// del tetto della CI.
///
/// Come per la regressione qui sotto, il budget di un secondo e' la meta'
/// della prova: senza, un difetto di non terminazione non fa fallire il
/// test, lo fa scadere.
const FUZZ_UTF16_SNIFF_REGRESSION: &[u8] =
    b"<\x00?\x00<\x00?\x00:\x00\x00\x00%%%%(%%e%%%%%%%%%\x00\x00\x00\x00\
         \x00\x00\x00>5<j:k___________>\x00\x00\x00\x00\x00\x00\x00%%%%%%\
         \x00\x00\x00\x00\x00\x00\x00>5<j:k______xx<?<x?m?>?>^<?mx\x00\x00\
         \x00?>x<?mx>?>x<?mx)?>x-?mx__\x00\x00\x00\x00\x00\x00\x00\x00\x00\
         \x00\x00<xjA:Point>0j><xjA:Point>\x00\x00\x00<xjA:Point>0j><xjA:Po\
         int>\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\
         \x00\x00\x00\x00\x00\x00\x00\x00\x00\x10\x00\x00\x00\x00\x00\x00\
         \x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x10\x00\x00\x00\
         \x00\x00<___>\x00\x00\x00\x00\x00\x00\x00\x00\x00N\x00\x00\x00\x00\
         \x00\x00\x00)\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\
         \x00\x00\"$\" <>?\x00>\x00\"$\" i\x00?\x00>\x00\x00\x0d\"&;\x00\
         \x00/>\x00\x00\x0d";

#[test]
fn rejects_input_the_guard_and_the_parser_read_differently() {
    {
        let started = std::time::Instant::now();
        assert!(__fuzz_read_kml(FUZZ_UTF16_SNIFF_REGRESSION).is_err());
        assert!(started.elapsed() < std::time::Duration::from_secs(1));
    }
}

#[test]
fn rejects_malformed_xml_that_stalled_the_kml_parser() {
    let started = std::time::Instant::now();
    assert!(__fuzz_read_kml(FUZZ_TIMEOUT_REGRESSION).is_err());
    assert!(started.elapsed() < std::time::Duration::from_secs(1));
}

#[test]
fn rejects_empty_point_before_dependency_parser() {
    assert!(__fuzz_read_kml(FUZZ_EMPTY_POINT_REGRESSION).is_err());
    assert!(__fuzz_read_kml(
        br"<kml><Placemark><Point><coordinates> </coordinates></Point></Placemark></kml>"
    )
    .is_err());
    assert!(__fuzz_read_kml(
            br"<kml><Placemark><Point><coordinates><![CDATA[ ]]></coordinates></Point></Placemark></kml>"
        )
        .is_err());
}

fn event_parser_error(xml: &str) -> PlenoraIoError {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("unsupported.kml");
    std::fs::write(&path, xml).unwrap();
    let mut stream = PlacemarkStream::open(&path).unwrap();
    stream
        .next_placemark(&CancellationToken::new(), 0)
        .unwrap_err()
}

#[test]
fn event_parser_rejects_model_track_and_multitrack() {
    for geometry in [
        "<Model><Location/></Model>",
        "<gx:Track><when>2026-01-01T00:00:00Z</when></gx:Track>",
        "<gx:MultiTrack><gx:Track/></gx:MultiTrack>",
    ] {
        let xml = format!(
            r#"<kml xmlns="http://www.opengis.net/kml/2.2" xmlns:gx="http://www.google.com/kml/ext/2.2"><Placemark>{geometry}</Placemark></kml>"#
        );
        let error = event_parser_error(&xml);
        assert_eq!(error.category, plenora_io_model::ErrorCategory::DataMapping);
        assert!(error.message.contains("geometria KML non supportata"));
        let diagnostics = error.row_diagnostics.as_deref().unwrap();
        assert_eq!(diagnostics.examples[0].source_index, 0);
        assert_eq!(diagnostics.counts["kml.invalid_placemark"], 1);
        assert!(diagnostics.validate().is_ok());
    }
}

#[test]
fn event_parser_rejects_multiple_top_level_geometries() {
    let error = event_parser_error(
        r#"<kml xmlns="http://www.opengis.net/kml/2.2"><Placemark><Point><coordinates>1,2</coordinates></Point><Point><coordinates>3,4</coordinates></Point></Placemark></kml>"#,
    );

    assert_eq!(error.category, plenora_io_model::ErrorCategory::DataMapping);
    assert!(error.message.contains("piu geometrie top-level"));
    let diagnostics = error.row_diagnostics.as_deref().unwrap();
    assert_eq!(diagnostics.examples[0].source_index, 0);
    assert_eq!(diagnostics.counts["kml.invalid_placemark"], 1);
    assert!(diagnostics.validate().is_ok());
}

#[test]
fn reads_kml_placemarks() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("in.kml");
    std::fs::write(&path, SAMPLE).unwrap();
    let driver = KmlDriver;
    let ds = driver.open(Source::Path(path), opzioni_lettura()).unwrap();
    assert_eq!(
        ds.layers()[0]
            .contract
            .geometry
            .as_ref()
            .unwrap()
            .resolved_crs()
            .unwrap()
            .axis_order,
        plenora_io_model::crs::AxisOrder::LongitudeLatitude
    );
    let mut r = ds
        .open_layer_reader(&ReadRequest {
            layer: LayerId(0),
            projected_fields: None,
            projection_mode: ProjectionMode::BestEffort,
            pruning_predicate: None,
            spatial_pruning_hint: None,
            scope: ReadScope::default(),
            batch_target: BatchTarget {
                target_bytes: usize::MAX,
                max_rows: 1,
            },
            cancellation: CancellationToken::default(),
        })
        .unwrap();
    let batch = r.next_batch().unwrap().unwrap();
    assert_eq!(batch.num_rows(), 1);
    assert_eq!(batch.num_columns(), 3);
    assert_eq!(
        r.contract().contract.geometry.as_ref().unwrap().dimensions,
        CoordinateDimensions::Xyz
    );
    let geometries = batch
        .column(0)
        .as_any()
        .downcast_ref::<BinaryArray>()
        .unwrap();
    let point = decode_wkb(geometries.value(0), &WkbLimits::default()).unwrap();
    assert!(matches!(
        point.value,
        WkbValue::Point(WkbCoordinate { z: Some(0.0), .. })
    ));
    assert_eq!(r.next_batch().unwrap().unwrap().num_rows(), 1);
    assert!(r.next_batch().unwrap().is_none());
}

/// Il documento di prova delle codifiche: un nome con un carattere non ASCII.
///
/// `citta` con l'accento e' il carattere che distingue una codifica
/// dall'altra: in UTF-8 sono due byte, in ISO-8859-1 uno solo, e in UTF-16
/// tutto il documento cambia forma.
fn kml_con_accento(codifica_dichiarata: &str) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"{codifica_dichiarata}\"?>\
             <kml xmlns=\"http://www.opengis.net/kml/2.2\"><Document>\
             <Placemark><name>città</name>\
             <Point><coordinates>11.25,43.75</coordinates></Point>\
             </Placemark></Document></kml>"
    )
}

/// Legge un KML da byte e restituisce il nome del primo segnaposto.
fn primo_nome(byte: &[u8]) -> Result<Option<String>> {
    let dir = tempfile::tempdir().expect("tempdir");
    let percorso = dir.path().join("prova.kml");
    std::fs::write(&percorso, byte).expect("il file si scrive");
    let dataset = KmlDriver.open(
        Source::Path(percorso),
        opzioni_lettura().with_assume_crs("OGC:CRS84"),
    )?;
    let layer = dataset.layers()[0].clone();
    let mut lettore = dataset.open_layer_reader(&ReadRequest {
        layer: layer.id,
        projected_fields: None,
        projection_mode: ProjectionMode::BestEffort,
        pruning_predicate: None,
        spatial_pruning_hint: None,
        scope: ReadScope::default(),
        batch_target: BatchTarget::default(),
        cancellation: CancellationToken::default(),
    })?;
    let batch = lettore.next_batch()?.expect("un batch");
    let indice = batch.schema().index_of("name").expect("colonna name");
    let nomi = batch
        .column(indice)
        .as_any()
        .downcast_ref::<arrow_array::StringArray>()
        .expect("il nome e' una stringa");
    Ok(if nomi.is_null(0) {
        None
    } else {
        Some(nomi.value(0).to_owned())
    })
}

/// Le codifiche che il driver accetta, fissate una per una.
///
/// # Perche' questo test esiste
///
/// Il supporto alle codifiche non UTF-8 non e' dichiarato in nessun
/// documento del prodotto, e per un periodo non era nemmeno dichiarato nel
/// manifesto: arrivava dalla feature `encoding` di `quick-xml`, che nel
/// nostro grafo la accendeva **`calamine`**. Il risultato era che
/// `cargo test -p driver-kml` e il binario spedito dicevano cose diverse
/// sullo stesso file. Corretto il 2026-09-08 dichiarando la feature; questo
/// test e' cio' che impedisce che torni a succedere in silenzio.
///
/// # Che cosa fissa
///
/// Il comportamento **misurato**, non quello desiderato. Ogni riga qui e'
/// stata verificata contro il prodotto costruito, prima e dopo
/// l'aggiornamento a `quick-xml 0.42`. Se una va cambiata, e' una decisione
/// da prendere e da dichiarare.
///
/// # L'aggiornamento alla 0.42, e che cosa ha spostato
///
/// La `0.42` pretende UTF-8 dal lettore e sposta la transcodifica in
/// `DecodingReader`, che rileva dal BOM; la codifica **dichiarata** la
/// applica il chiamante, ed e' `applica_codifica_dichiarata`.
///
/// Confrontando i sette casi qui sotto contro la 0.41 e contro la 0.42:
/// nessun ingresso accettato prima e' stato perso, e **nessun valore e'
/// cambiato**. L'unica differenza e' un allargamento: UTF-16, con e senza
/// BOM, prima era rifiutato con «nome di elemento XML non valido» e ora si
/// legge, restituendo il valore giusto. E' un cambiamento, e sta scritto
/// qui perche' si veda.
#[test]
fn le_codifiche_accettate_restano_quelle() {
    // UTF-8: il caso di riferimento.
    let utf8 = kml_con_accento("UTF-8").into_bytes();
    assert_eq!(
        primo_nome(&utf8).expect("un KML UTF-8 si legge"),
        Some("città".to_owned())
    );

    // Con il BOM davanti, uguale.
    let mut utf8_bom = vec![0xEF, 0xBB, 0xBF];
    utf8_bom.extend_from_slice(&utf8);
    assert_eq!(
        primo_nome(&utf8_bom).expect("il BOM UTF-8 non cambia niente"),
        Some("città".to_owned())
    );

    // ISO-8859-1 dichiarata **e** reale: accettata, e il valore si
    // conserva. E' il caso che l'aggiornamento avrebbe potuto perdere, e
    // che `applica_codifica_dichiarata` tiene.
    let latin1: Vec<u8> = kml_con_accento("ISO-8859-1")
        .chars()
        .map(|c| u8::try_from(c as u32).expect("il documento sta in latin-1"))
        .collect();
    assert_eq!(
        primo_nome(&latin1).expect("un KML ISO-8859-1 si legge"),
        Some("città".to_owned()),
        "la codifica dichiarata viene onorata, e il valore si conserva"
    );

    // Una dichiarazione che mente: byte UTF-8, intestazione ISO-8859-1.
    // Passa, e il valore esce **trasformato** -- la dichiarazione viene
    // creduta. Non e' un difetto introdotto dall'aggiornamento: la 0.41
    // dava esattamente lo stesso, ed e' fissato qui perche' un giorno
    // qualcuno non lo scambi per una regressione.
    let mentitore = kml_con_accento("ISO-8859-1").into_bytes();
    assert_eq!(
        primo_nome(&mentitore).expect("byte validi in entrambe le letture si leggono"),
        Some("cittÃ\u{a0}".to_owned()),
        "la dichiarazione viene creduta, e il valore ne porta il segno"
    );

    // Una codifica dichiarata che nessuno conosce: si ripiega su UTF-8,
    // che e' cio' che faceva la 0.41. Un nome sconosciuto non e' un motivo
    // di rifiuto.
    let sconosciuta = kml_con_accento("X-INVENTATA").into_bytes();
    assert_eq!(
        primo_nome(&sconosciuta).expect("una codifica sconosciuta non ferma la lettura"),
        Some("città".to_owned())
    );

    // UTF-16: **allargamento**. Con la 0.41 era rifiutato; con la 0.42 e
    // `DecodingReader` si legge, e il valore torna giusto. Vale con e senza
    // BOM.
    let utf16: Vec<u8> = kml_con_accento("UTF-16")
        .encode_utf16()
        .flat_map(u16::to_le_bytes)
        .collect();
    assert_eq!(
        primo_nome(&utf16).expect("UTF-16 senza BOM si legge, dalla 0.42"),
        Some("città".to_owned()),
        "l'aggiornamento allarga gli ingressi accettati invece di restringerli"
    );
    let mut utf16_bom = vec![0xFF, 0xFE];
    utf16_bom.extend_from_slice(&utf16);
    assert_eq!(
        primo_nome(&utf16_bom).expect("UTF-16 con BOM si legge"),
        Some("città".to_owned())
    );
}

#[test]
fn event_stream_matches_legacy_document_traversal() {
    let text = r#"<?xml version="1.0" encoding="UTF-8"?>
        <kml:kml xmlns:kml="http://www.opengis.net/kml/2.2">
          <kml:Document>
            <kml:Folder>
              <kml:Placemark id="a"><kml:name>A &amp; B</kml:name>
                <kml:Point><kml:coordinates>12,45</kml:coordinates></kml:Point>
              </kml:Placemark>
            </kml:Folder>
            <Update>
              <kml:Placemark><kml:name>non attraversato</kml:name>
                <kml:Point><kml:coordinates>0,0</kml:coordinates></kml:Point>
              </kml:Placemark>
            </Update>
            <kml:Placemark><kml:description><![CDATA[testo <grezzo>]]></kml:description>
              <kml:LineString><kml:coordinates>0,0,0 1,1,0</kml:coordinates></kml:LineString>
            </kml:Placemark>
          </kml:Document>
        </kml:kml>"#;
    let document: Kml = text.parse().unwrap();
    let mut legacy_placemarks = Vec::new();
    collect(
        &document,
        &mut legacy_placemarks,
        &CancellationToken::new(),
        &mut 0,
    )
    .unwrap();
    let (legacy, _) = build_batch(&legacy_placemarks).unwrap();

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("semantic-equivalence.kml");
    std::fs::write(&path, text).unwrap();
    let dataset = KmlDriver
        .open(Source::Path(path), opzioni_lettura())
        .unwrap();
    let mut reader = dataset
        .open_layer_reader(&ReadRequest {
            layer: LayerId(0),
            projected_fields: None,
            projection_mode: ProjectionMode::BestEffort,
            pruning_predicate: None,
            spatial_pruning_hint: None,
            scope: ReadScope::default(),
            batch_target: BatchTarget::default(),
            cancellation: CancellationToken::new(),
        })
        .unwrap();
    let streamed = reader.next_batch().unwrap().unwrap();
    assert!(reader.next_batch().unwrap().is_none());
    assert_eq!(streamed.num_rows(), 2);
    assert_eq!(streamed.schema(), legacy.schema());
    for index in 0..streamed.num_columns() {
        assert_eq!(
            streamed.column(index).to_data(),
            legacy.column(index).to_data()
        );
    }
    assert_eq!(DESCRIPTOR.read_mode(), ReadMode::StreamingSequential);
}

#[test]
fn write_then_read_round_trip() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("out.kml");
    let wkb = to_wkb(&geo_types::Geometry::Point(geo_types::Point::new(
        12.5, 45.9,
    )))
    .unwrap();
    let schema: SchemaRef = Arc::new(Schema::new(vec![
        geometry_field(GEOMETRY, OGC_CRS84),
        Field::new("name", DataType::Utf8, true),
        Field::new("description", DataType::Utf8, true),
        Field::new("population", DataType::Int64, true),
    ]));
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(BinaryArray::from(vec![Some(wkb.as_slice())])),
            Arc::new(StringArray::from(vec!["Roma"])),
            Arc::new(StringArray::from(vec!["capitale"])),
            Arc::new(arrow_array::Int64Array::from(vec![2_800_000])),
        ],
    )
    .unwrap();

    let driver = KmlDriver;
    let plan = WritePlan {
        layers: vec![WriteLayer {
            name: "l".to_owned(),
            contract: DataContract {
                schema,
                geometry: None,
            },
        }],
    };
    let mut w = driver
        .create(Sink::Path(out.clone()), &plan, &opzioni_scrittura())
        .unwrap();
    w.write(&batch).unwrap();
    let published = w.finish().unwrap();
    assert_eq!(
        published.loss.counts.get("coercion tipo attributo"),
        Some(&1)
    );
    assert_eq!(
        published.fidelity.level,
        plenora_io_core::Fidelity::Approximating
    );

    let ds = driver.open(Source::Path(out), opzioni_lettura()).unwrap();
    let mut r = ds
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
    let rb = r.next_batch().unwrap().unwrap();
    assert_eq!(rb.num_rows(), 1);
    let name = rb
        .column_by_name("name")
        .unwrap()
        .as_any()
        .downcast_ref::<StringArray>()
        .unwrap();
    assert_eq!(name.value(0), "Roma");
}

#[test]
fn xyz_round_trip_preserves_altitude() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("xyz.kml");
    let geometry = WkbGeometry {
        value: WkbValue::Point(WkbCoordinate {
            x: 12.5,
            y: 45.9,
            z: Some(123.25),
            m: None,
        }),
        dimensions: CoordinateDimensions::Xyz,
        srid: None,
    };
    let wkb = encode_wkb(&geometry, WkbFlavor::Iso).unwrap();
    let mut geometry_contract =
        GeometryColumnContract::wkb_xy(FieldId(0), GEOMETRY, ResolvedCrs::wgs84(), true);
    geometry_contract.dimensions = CoordinateDimensions::Xyz;
    geometry_contract.set_exact_geometry_types(vec![GeometryType::Point]);
    let schema: SchemaRef = Arc::new(Schema::new(vec![
        with_geometry_contract_metadata(&geometry_field(GEOMETRY, OGC_CRS84), &geometry_contract),
        Field::new("name", DataType::Utf8, true),
        Field::new("description", DataType::Utf8, true),
    ]));
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(BinaryArray::from(vec![Some(wkb.as_slice())])),
            Arc::new(StringArray::from(vec!["Quota"])),
            Arc::new(StringArray::from(vec!["XYZ"])),
        ],
    )
    .unwrap();
    let plan = WritePlan {
        layers: vec![WriteLayer {
            name: "xyz".to_owned(),
            contract: DataContract {
                schema,
                geometry: Some(geometry_contract),
            },
        }],
    };

    let driver = KmlDriver;
    let mut writer = driver
        .create(Sink::Path(out.clone()), &plan, &opzioni_scrittura())
        .unwrap();
    writer.write(&batch).unwrap();
    writer.finish().unwrap();
    assert!(std::fs::read_to_string(&out)
        .unwrap()
        .contains("12.5,45.9,123.25"));

    let dataset = driver.open(Source::Path(out), opzioni_lettura()).unwrap();
    assert_eq!(
        dataset.layers()[0]
            .contract
            .geometry
            .as_ref()
            .unwrap()
            .dimensions,
        CoordinateDimensions::Xyz
    );
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
    let round_trip = reader.next_batch().unwrap().unwrap();
    let geometries = round_trip
        .column(0)
        .as_any()
        .downcast_ref::<BinaryArray>()
        .unwrap();
    let decoded = decode_wkb(geometries.value(0), &WkbLimits::default()).unwrap();
    assert!(matches!(
        decoded.value,
        WkbValue::Point(WkbCoordinate {
            z: Some(123.25),
            ..
        })
    ));
}

#[test]
fn xym_contract_is_rejected_before_output_creation() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("m.kml");
    let mut geometry =
        GeometryColumnContract::wkb_xy(FieldId(0), GEOMETRY, ResolvedCrs::wgs84(), true);
    geometry.dimensions = CoordinateDimensions::Xym;
    let schema: SchemaRef = Arc::new(Schema::new(vec![
        with_geometry_contract_metadata(&geometry_field(GEOMETRY, OGC_CRS84), &geometry),
        Field::new("name", DataType::Utf8, true),
        Field::new("description", DataType::Utf8, true),
    ]));
    let plan = WritePlan {
        layers: vec![WriteLayer {
            name: "m".to_owned(),
            contract: DataContract {
                schema,
                geometry: Some(geometry),
            },
        }],
    };
    let driver = KmlDriver;
    assert!(driver
        .create(Sink::Path(out.clone()), &plan, &opzioni_scrittura())
        .is_err());
    assert!(!out.exists());
}

#[test]
fn direct_conversion_preserves_xyz_multipolygon() {
    let coordinate = |x, y, z| WkbCoordinate {
        x,
        y,
        z: Some(z),
        m: None,
    };
    let polygon = WkbGeometry {
        value: WkbValue::Polygon(vec![vec![
            coordinate(0.0, 0.0, 10.0),
            coordinate(1.0, 0.0, 11.0),
            coordinate(1.0, 1.0, 12.0),
            coordinate(0.0, 0.0, 10.0),
        ]]),
        dimensions: CoordinateDimensions::Xyz,
        srid: None,
    };
    let geometry = WkbGeometry {
        value: WkbValue::MultiPolygon(vec![polygon]),
        dimensions: CoordinateDimensions::Xyz,
        srid: None,
    };
    let kml = kml_geometry_from_wkb(&geometry).unwrap();
    assert_eq!(wkb_geometry_from_kml(&kml).unwrap(), geometry);
}

#[test]
fn homogeneous_geometry_collection_is_rejected_as_ambiguous() {
    let point = |x| WkbGeometry {
        value: WkbValue::Point(WkbCoordinate {
            x,
            y: 2.0,
            z: None,
            m: None,
        }),
        dimensions: CoordinateDimensions::Xy,
        srid: None,
    };
    let geometry = WkbGeometry {
        value: WkbValue::GeometryCollection(vec![point(1.0), point(3.0)]),
        dimensions: CoordinateDimensions::Xy,
        srid: None,
    };
    assert!(kml_geometry_from_wkb(&geometry).is_err());
}

#[test]
fn writer_adapter_attributes_kml_specific_rejection_and_prevents_publish() {
    let directory = tempfile::tempdir().unwrap();
    let output = directory.path().join("ambiguous.kml");
    let contract = kml_contract(
        &BTreeSet::from([CoordinateDimensions::Xy]),
        BTreeSet::from([GeometryType::GeometryCollection]),
    );
    let point = |x| WkbGeometry {
        value: WkbValue::Point(WkbCoordinate {
            x,
            y: 1.0,
            z: None,
            m: None,
        }),
        dimensions: CoordinateDimensions::Xy,
        srid: None,
    };
    let bytes = encode_wkb(
        &WkbGeometry {
            value: WkbValue::GeometryCollection(vec![point(1.0), point(2.0)]),
            dimensions: CoordinateDimensions::Xy,
            srid: None,
        },
        WkbFlavor::Iso,
    )
    .unwrap();
    let batch = RecordBatch::try_new(
        contract.schema.clone(),
        vec![
            Arc::new(BinaryArray::from(vec![Some(bytes.as_slice())])),
            Arc::new(StringArray::from(vec![Some("name")])),
            Arc::new(StringArray::from(vec![None::<&str>])),
        ],
    )
    .unwrap();
    let plan = WritePlan {
        layers: vec![plenora_io_core::WriteLayer {
            name: "ambiguous".to_owned(),
            contract,
        }],
    };
    let mut writer = KmlDriver
        .create(Sink::Path(output.clone()), &plan, &opzioni_scrittura())
        .unwrap();
    writer.declare_input_total(LayerId(0), 1).unwrap();

    let error = writer.write(&batch).unwrap_err();
    let diagnostics = error.row_diagnostics.as_deref().unwrap();
    assert_eq!(diagnostics.examples[0].source_index, 0);
    assert_eq!(diagnostics.counts["kml.geometry_not_representable"], 1);
    assert!(diagnostics.validate().is_ok());
    assert!(writer.finish().is_err());
    assert!(!output.exists());
}

#[test]
fn empty_geometry_does_not_invent_xy_dimensions() {
    assert!(dimensions_for_kml_coords(&[]).is_err());
    let empty = WkbGeometry {
        value: WkbValue::GeometryCollection(vec![]),
        dimensions: CoordinateDimensions::Xy,
        srid: None,
    };
    assert!(kml_geometry_from_wkb(&empty).is_err());
}
