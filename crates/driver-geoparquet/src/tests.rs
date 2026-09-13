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

/// Opzioni di lettura con l'opt-in al `crs` storico non conforme.
///
/// I semi di questo repository sono stati scritti prima di S10 e portano
/// `crs: {"id": {...}}`, che non e' un documento PROJJSON: lo schema
/// ufficiale li rifiuta, e per leggerli si dichiara di volerlo fare.
fn opzioni_lettura_legacy() -> ReadOptions {
    let mut opzioni = opzioni_lettura();
    opzioni
        .format_options
        .insert("accept_legacy_crs_id_only".to_owned(), "true".to_owned());
    opzioni
}

/// Un seme storico di questo repository: `crs` nella forma non conforme.
fn seme_storico() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fuzz/seeds/geoparquet_reader/bit-width-dizionario-fuori-intervallo.parquet")
}

// --- l'output del writer, contro l'autorita' --------------------------
//
// Queste due prove non passano dal modulo `metadati`: scrivono un Parquet
// vero e ne guardano il risultato con gli schemi ufficiali e con il
// descrittore Parquet. Un writer verificato dal proprio lettore non e'
// verificato: i due possono sbagliare insieme, ed e' esattamente cio' che
// e' successo -- emettevamo un `covering` piatto e lo leggevamo come
// valido.

/// Scrive un `GeoParquet` vero e ne restituisce il metadato `geo` grezzo.
fn geo_di_un_file_scritto(dir: &tempfile::TempDir) -> (String, std::path::PathBuf) {
    let percorso = dir.path().join("scritto.parquet");
    let punto: Vec<u8> = to_wkb(&Geometry::Point(Point::new(9.19, 45.46))).unwrap();
    let geom = BinaryArray::from(vec![Some(punto.as_slice())]);
    let ids = Int64Array::from(vec![1_i64]);
    let schema = Arc::new(Schema::new(vec![
        Field::new("geometry", DataType::Binary, true)
            .with_metadata(geometry_field_meta("EPSG:4326")),
        Field::new("id", DataType::Int64, false),
    ]));
    let batch = RecordBatch::try_new(schema.clone(), vec![Arc::new(geom), Arc::new(ids)]).unwrap();

    let plan = WritePlan {
        layers: vec![WriteLayer {
            name: "l".to_owned(),
            contract: DataContract {
                schema,
                geometry: None,
            },
        }],
    };
    let mut writer = GeoParquetDriver
        .create(Sink::Path(percorso.clone()), &plan, &opzioni_scrittura())
        .expect("il writer si apre");
    writer.write(&batch).expect("scrittura");
    writer.finish().expect("chiusura");

    let builder = ParquetRecordBatchReaderBuilder::try_new(File::open(&percorso).unwrap()).unwrap();
    let grezzo = builder
        .metadata()
        .file_metadata()
        .key_value_metadata()
        .expect("key_value_metadata")
        .iter()
        .find(|e| e.key == "geo")
        .and_then(|e| e.value.clone())
        .expect("metadato `geo`");
    (grezzo, percorso)
}

#[test]
fn il_metadato_scritto_rispetta_lo_schema_ufficiale() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (grezzo, _) = geo_di_un_file_scritto(&dir);
    let documento: serde_json::Value = serde_json::from_str(&grezzo).expect("il metadato e' JSON");

    // La versione dichiarata e' quella che si scrive, e lo schema contro cui
    // si valida e' il suo.
    assert_eq!(documento["version"].as_str(), Some("1.1.0"));
    schema_ufficiale::valida(&documento, "1.1.0")
        .expect("il metadato scritto rispetta GeoParquet 1.1.0 e il PROJJSON che referenzia");
}

#[test]
fn la_colonna_bbox_scritta_ha_la_forma_fisica_che_il_covering_designa() {
    // Lo schema JSON dice che il covering nomina `["bbox", "xmin"]`; non dice
    // che nel Parquet esista una colonna struct `bbox` con quel figlio, di
    // quel tipo e con quella nullabilita'. Se non esistesse, il documento
    // sarebbe conforme e il pruning non troverebbe niente: due verifiche
    // diverse, ed e' la ragione per cui questa sta accanto all'altra.
    let dir = tempfile::tempdir().expect("tempdir");
    let (_, percorso) = geo_di_un_file_scritto(&dir);
    let builder = ParquetRecordBatchReaderBuilder::try_new(File::open(&percorso).unwrap()).unwrap();
    let descrittore = builder.metadata().file_metadata().schema_descr();

    let foglie: Vec<Vec<String>> = (0..descrittore.num_columns())
        .map(|i| descrittore.column(i).path().parts().to_vec())
        .collect();

    for spigolo in BBOX_SPIGOLI {
        let atteso = vec![BBOX_STRUCT.to_owned(), spigolo.to_owned()];
        let Some(indice) = foglie.iter().position(|p| *p == atteso) else {
            panic!("manca la foglia {BBOX_STRUCT}.{spigolo}");
        };
        let colonna = descrittore.column(indice);
        assert_eq!(
            colonna.physical_type(),
            parquet::basic::Type::DOUBLE,
            "{spigolo} deve essere un FLOAT64"
        );
        assert_eq!(
            colonna.self_type().get_basic_info().repetition(),
            parquet::basic::Repetition::OPTIONAL,
            "{spigolo} segue la nullabilita' della geometria"
        );
    }

    // L'ordine e' quello dello schema e del pruning, e non e' un dettaglio:
    // un covering con gli spigoli incrociati darebbe al pruning i numeri
    // sbagliati senza che nulla lo dica.
    let ordine: Vec<&Vec<String>> = foglie
        .iter()
        .filter(|p| p.first().map(String::as_str) == Some(BBOX_STRUCT))
        .collect();
    assert_eq!(ordine.len(), 4, "quattro figli, non uno di piu'");
    for (posizione, spigolo) in BBOX_SPIGOLI.into_iter().enumerate() {
        assert_eq!(ordine[posizione][1], spigolo, "ordine dei figli");
    }

    // E le quattro colonne piatte di prima **non** ci sono piu'.
    for storica in BBOX_COLS {
        assert!(
            !foglie
                .iter()
                .any(|p| p.first().map(String::as_str) == Some(storica)),
            "{storica} non deve piu' essere emessa"
        );
    }
}

// --- le promesse del lotto, esercitate ---------------------------------

/// Un `Point M` in WKB ISO: tipo 2001, tre ordinate.
///
/// `to_wkb` di geo-types non sa produrlo -- il suo modello non ha la misura
/// M -- quindi si scrive a mano. E' l'unico modo di far arrivare al writer
/// una geometria che `GeoParquet` non sa rappresentare.
fn wkb_point_m(x: f64, y: f64, m: f64) -> Vec<u8> {
    let mut byte = vec![1_u8];
    byte.extend_from_slice(&2001_u32.to_le_bytes());
    byte.extend_from_slice(&x.to_le_bytes());
    byte.extend_from_slice(&y.to_le_bytes());
    byte.extend_from_slice(&m.to_le_bytes());
    byte
}

// --- cio' che il writer pubblica ---------------------------------------

#[test]
fn un_crs_che_non_e_projjson_non_produce_alcun_file() {
    // Il writer chiamava PROJJSON qualunque oggetto JSON gli arrivasse, e lo
    // copiava nel metadato senza chiedere niente a nessuno: un `crs` fatto
    // cosi' produceva un GeoParquet non conforme, pubblicato e indistin-
    // guibile da uno buono.
    //
    // La prova guarda il **filesystem**, non il codice d'errore soltanto: la
    // promessa e' che un file del genere non esista.
    let dir = tempfile::tempdir().expect("tempdir");
    let percorso = dir.path().join("mai_scritto.parquet");
    let crs = ResolvedCrs::new(
        Some("EPSG:4326".to_owned()),
        plenora_io_model::crs::CrsKind::Geographic,
        Some(r#"{"questo":"non e' PROJJSON"}"#.to_owned()),
    );
    let schema = Arc::new(Schema::new(vec![Field::new(
        "geometry",
        DataType::Binary,
        true,
    )
    .with_metadata(geometry_field_meta("EPSG:4326"))]));
    let mut geometria = GeometryColumnContract::wkb_xy(FieldId(0), "geometry", crs, true);
    geometria.set_exact_geometry_types(vec![GeometryType::Point]);
    let plan = WritePlan {
        layers: vec![WriteLayer {
            name: "l".to_owned(),
            contract: DataContract {
                schema,
                geometry: Some(geometria),
            },
        }],
    };
    let esito = GeoParquetDriver
        .create(Sink::Path(percorso.clone()), &plan, &opzioni_scrittura())
        .map(|_| ());
    assert!(
        matches!(
            esito,
            Err(ref errore) if errore.code == plenora_io_model::IoErrorCode::Unsupported
        ),
        "un `crs` che non e' PROJJSON non e' scrivibile in GeoParquet: {esito:?}"
    );
    assert!(!percorso.exists(), "e non deve restare niente sul disco");
}

#[test]
fn un_metadato_finale_non_conforme_non_viene_pubblicato() {
    // L'ultimo cancello, esercitato **dall'API pubblica**.
    //
    // Lo schema vuole `primary_column` di almeno un carattere. Una colonna
    // geometrica senza nome e' un contratto che Arrow accetta e che
    // GeoParquet non puo' rappresentare: il documento si costruisce, e non
    // e' conforme. Nessuna delle difese puntuali a monte guarda il nome
    // della colonna -- e' proprio il tipo di caso che nessuno prevede -- e
    // arriva quindi al controllo finale, che e' li' per questo.
    let dir = tempfile::tempdir().expect("tempdir");
    let percorso = dir.path().join("senza_nome.parquet");
    let punto: Vec<u8> = to_wkb(&Geometry::Point(Point::new(1.0, 2.0))).unwrap();
    let schema = Arc::new(Schema::new(vec![
        Field::new("", DataType::Binary, true).with_metadata(geometry_field_meta("OGC:CRS84"))
    ]));
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![Arc::new(BinaryArray::from(vec![Some(punto.as_slice())]))],
    )
    .unwrap();
    let plan = WritePlan {
        layers: vec![WriteLayer {
            name: "l".to_owned(),
            contract: DataContract {
                schema,
                geometry: None,
            },
        }],
    };
    let mut writer = GeoParquetDriver
        .create(Sink::Path(percorso.clone()), &plan, &opzioni_scrittura())
        .expect("il writer si apre: il difetto non e' visibile qui");
    writer
        .write(&batch)
        .expect("il batch si scrive nello staging");
    let esito = writer.finish().map(|_| ());
    assert!(
        matches!(
            esito,
            Err(ref errore) if errore.code == plenora_io_model::IoErrorCode::Format
        ),
        "un metadato che lo schema ufficiale rifiuta ferma la pubblicazione: {esito:?}"
    );
    assert!(
        !percorso.exists(),
        "e il file non arriva a destinazione: cio' che e' pubblicato non si ritira"
    );
}

#[test]
fn dove_la_geometria_e_nulla_il_covering_e_nullo() {
    // GeoParquet 1.1 chiede un riquadro **se e solo se** la geometria c'e'.
    //
    // La prima stesura metteva i nulli soltanto nei quattro figli e
    // costruiva la struct con `StructArray::from`, che la crea senza bitmap
    // di validita': il `bbox` risultava allora **presente** su ogni riga, e
    // su quelle senza geometria affermava che il riquadro esiste e non si
    // conosce. Lo schema Parquet non lo mostrava -- i campi erano gia'
    // marcati opzionali -- e solo i valori riletti lo dicono.
    let dir = tempfile::tempdir().expect("tempdir");
    let percorso = dir.path().join("con_nulli.parquet");
    let punto: Vec<u8> = to_wkb(&Geometry::Point(Point::new(1.0, 2.0))).unwrap();
    let schema = Arc::new(Schema::new(vec![Field::new(
        "geometry",
        DataType::Binary,
        true,
    )
    .with_metadata(geometry_field_meta("OGC:CRS84"))]));
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![Arc::new(BinaryArray::from(vec![
            Some(punto.as_slice()),
            None,
        ]))],
    )
    .unwrap();
    let plan = WritePlan {
        layers: vec![WriteLayer {
            name: "l".to_owned(),
            contract: DataContract {
                schema,
                geometry: None,
            },
        }],
    };
    let mut writer = GeoParquetDriver
        .create(Sink::Path(percorso.clone()), &plan, &opzioni_scrittura())
        .expect("il writer si apre");
    writer.write(&batch).expect("il batch si scrive");
    writer.finish().expect("il file si pubblica");

    // Si rilegge il Parquet **grezzo**: il driver toglie il covering dallo
    // schema esposto, quindi passando da `open` non lo si vedrebbe.
    let riletto = File::open(&percorso).expect("il file esiste");
    let mut lettore = ParquetRecordBatchReaderBuilder::try_new(riletto)
        .expect("Parquet valido")
        .build()
        .expect("lettore");
    let riga = lettore
        .next()
        .expect("almeno un batch")
        .expect("batch leggibile");
    let indice = riga
        .schema()
        .index_of(BBOX_STRUCT)
        .expect("la colonna del covering c'e'");
    let bbox = riga
        .column(indice)
        .as_any()
        .downcast_ref::<StructArray>()
        .expect("il covering e' una struct");
    assert!(!bbox.is_null(0), "dove la geometria c'e', il riquadro c'e'");
    assert!(
        bbox.is_null(1),
        "dove la geometria non c'e', il riquadro non c'e'"
    );
    // E i figli seguono la struct, invece di contraddirla.
    for spigolo in 0..4 {
        assert!(
            bbox.column(spigolo).is_null(1),
            "anche lo spigolo {spigolo} e' nullo sulla riga senza geometria"
        );
    }
}

#[test]
fn una_geometria_con_la_misura_m_non_e_scrivibile() {
    // La decisione del lotto e' che XYM e XYZM non si scrivono: il pattern
    // dello schema ammette `( Z)?` e nient'altro, e omettere il suffisso
    // direbbe che quelle geometrie sono XY, facendo sparire la misura M
    // senza che nessuno lo dichiari.
    //
    // La prova e' **end-to-end**, e mostra che le vie sono due e tutt'e due
    // chiuse. Non asserisce un codice d'errore solo: asserisce che da
    // nessuna delle due parti si riesce a scrivere.
    let dir = tempfile::tempdir().expect("tempdir");
    let punto = wkb_point_m(1.0, 2.0, 3.0);
    let schema = Arc::new(Schema::new(vec![Field::new(
        "geometry",
        DataType::Binary,
        true,
    )
    .with_metadata(geometry_field_meta("EPSG:4326"))]));
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![Arc::new(BinaryArray::from(vec![Some(punto.as_slice())]))],
    )
    .unwrap();

    // Via 1: il contratto **dichiara** XYM. Il formato lo rifiuta
    // all'apertura, prima ancora di ricevere un byte.
    let mut geometria =
        GeometryColumnContract::wkb_xy(FieldId(0), "geometry", ResolvedCrs::wgs84(), true);
    geometria.dimensions = CoordinateDimensions::Xym;
    let dichiarato = WritePlan {
        layers: vec![WriteLayer {
            name: "l".to_owned(),
            contract: DataContract {
                schema: schema.clone(),
                geometry: Some(geometria),
            },
        }],
    };
    let apertura = GeoParquetDriver.create(
        Sink::Path(dir.path().join("dichiarato.parquet")),
        &dichiarato,
        &opzioni_scrittura(),
    );
    assert!(
        matches!(
            apertura,
            Err(ref errore) if errore.category == plenora_io_model::ErrorCategory::Unsupported
        ),
        "un contratto che dichiara XYM non apre nemmeno un writer GeoParquet"
    );

    // Via 2: il contratto dichiara XY e i **dati** portano la misura M. La
    // riga e' rifiutata prima di essere scritta.
    let taciuto = WritePlan {
        layers: vec![WriteLayer {
            name: "l".to_owned(),
            contract: DataContract {
                schema,
                geometry: None,
            },
        }],
    };
    let mut writer = GeoParquetDriver
        .create(
            Sink::Path(dir.path().join("taciuto.parquet")),
            &taciuto,
            &opzioni_scrittura(),
        )
        .expect("con un contratto XY il writer si apre");
    let scrittura = writer.write(&batch);
    assert!(
        matches!(
            scrittura,
            Err(ref errore)
                if errore.capability_reason
                    == Some(plenora_io_model::CapabilityReason::CoordinateDimensions)
        ),
        "una geometria XYM sotto un contratto XY e' rifiutata prima della scrittura"
    );
}

#[test]
fn un_crs_mancante_si_scrive_null_e_si_rilegge_sconosciuto() {
    // `null` e' un'affermazione: dice che il CRS non c'e'. L'omissione ne
    // sarebbe un'altra -- direbbe CRS84 -- e le due non si scambiano.
    let geo = build_geo_metadata("geometry", &BTreeSet::new(), &CrsDaScrivere::Nullo)
        .expect("il documento si costruisce");
    let documento: serde_json::Value = serde_json::from_str(&geo).expect("JSON");
    assert_eq!(
        documento["columns"]["geometry"]["crs"],
        serde_json::Value::Null
    );
    schema_ufficiale::valida(&documento, "1.1.0").expect("conforme");

    // E in lettura `null` non diventa CRS84: diventa «non lo so».
    let letti = metadati::analizza(&geo, false).expect("conforme");
    assert_eq!(letti.primaria.crs, metadati::Crs::Nullo);
    let crs = crs_from(Some(&letti)).expect("risolto");
    assert_eq!(
        crs.id.as_deref(),
        None,
        "un CRS assente non ha identificatore"
    );
}

#[test]
fn una_definizione_projjson_si_scrive_per_intero() {
    // Non un riassunto: il documento che il contratto porta, tale e quale.
    // `{"id": ...}` non e' PROJJSON, ed e' cio' che questo writer emetteva.
    let projjson = serde_json::json!({
        "type": "GeographicCRS",
        "name": "WGS 84 (CRS84)",
        "datum": {
            "type": "GeodeticReferenceFrame",
            "name": "World Geodetic System 1984",
            "ellipsoid": {
                "name": "WGS 84",
                "semi_major_axis": 6_378_137,
                "inverse_flattening": 298.257_223_563
            }
        },
        "coordinate_system": {
            "subtype": "ellipsoidal",
            "axis": [
                {"name": "Geodetic longitude", "abbreviation": "Lon", "direction": "east", "unit": "degree"},
                {"name": "Geodetic latitude", "abbreviation": "Lat", "direction": "north", "unit": "degree"}
            ]
        }
    });
    let geo = build_geo_metadata(
        "geometry",
        &BTreeSet::new(),
        &CrsDaScrivere::Documento(projjson.to_string()),
    )
    .expect("il documento si costruisce");
    let documento: serde_json::Value = serde_json::from_str(&geo).expect("JSON");
    assert_eq!(documento["columns"]["geometry"]["crs"], projjson);
    // E il risultato e' conforme: e' il PROJJSON referenziato a dirlo.
    schema_ufficiale::valida(&documento, "1.1.0").expect("conforme");
}

#[test]
fn un_covering_in_un_documento_1_0_0_non_toglie_colonne() {
    // La specifica 1.0 non attribuisce significato a `covering`: quel campo
    // viene ignorato, e le colonne che nomina restano colonne utente.
    // Toglierle sarebbe attribuirgli noi un significato che la sua versione
    // non gli da'.
    let dir = tempfile::tempdir().expect("tempdir");
    let documento = concat!(
        r#"{"version":"1.0.0","primary_column":"geometry","columns":{"geometry":{"#,
        r#""encoding":"WKB","geometry_types":["Point"],"#,
        r#""covering":{"bbox":{"xmin":["bbox","xmin"],"ymin":["bbox","ymin"],"#,
        r#""xmax":["bbox","xmax"],"ymax":["bbox","ymax"]}}}}}"#,
    );
    let percorso = parquet_con_geo(&dir, Some(documento));
    let dataset = GeoParquetDriver
        .open(Source::Path(percorso), opzioni_lettura())
        .expect("un 1.0.0 con `covering` resta valido");
    let contratto = &dataset.layers()[0].contract;
    assert!(contratto.schema.index_of("id").is_ok());
    assert!(contratto.schema.index_of("geometry").is_ok());
}

/// Un `ARROW:schema` non decodificabile da base64 e' rifiutato, con il
/// messaggio curato e senza panico.
///
/// # Perche' esiste
///
/// `valida_schema_arrow_incorporato` decodifica il valore di `ARROW:schema`
/// dal footer Parquet con `base64::…::STANDARD.decode`, e da quel byte in
/// poi lo passa alla prevalidazione Arrow. Il ramo del **rifiuto** non
/// aveva nessun test: era coperto solo di riflesso, dal fatto che i file
/// che scriviamo noi hanno sempre un `ARROW:schema` valido.
///
/// Lo si e' scoperto alzando `base64` da 0.22.1 a 0.23.1, il 2026-09-07.
/// Il decoder e' riscritto -- 2062 righe, un motore SIMD nuovo -- e la
/// domanda «gli stessi ingressi danno gli stessi esiti?» non aveva un test
/// nel prodotto a cui rispondere. Le due versioni, misurate fuori dal
/// workspace su ventun casi limite, **decidono** allo stesso modo: quel che
/// decodificava decodifica agli stessi byte, quel che era rifiutato resta
/// rifiutato. A cambiare sono due **testi** d'errore della libreria, e qui
/// non escono: il `map_err` li scarta e mette il messaggio curato.
///
/// Che li scarti conta piu' di prima. Il testo nuovo di `base64` include il
/// carattere che ha fatto fallire il decode -- «Invalid last symbol 0x42
/// ('B') at offset 5» -- cioe' contenuto **derivato dal file**. Se qualcuno
/// un giorno propagasse l'errore della libreria invece di scartarlo, un
/// messaggio che dichiara di non portare payload ne porterebbe.
///
/// Il file di prova porta **due** voci `ARROW:schema`: quella valida che
/// `ArrowWriter` scrive da se', e una seconda indecifrabile aggiunta dopo.
/// Cosi' il test prova anche che il ciclo guarda ogni voce e non si ferma
/// alla prima buona.
#[test]
fn un_arrow_schema_non_decodificabile_e_rifiutato() {
    let dir = tempfile::tempdir().unwrap();
    let percorso = dir.path().join("arrow_schema_rotto.parquet");
    let punto: Vec<u8> = to_wkb(&Geometry::Point(Point::new(1.0, 2.0))).unwrap();
    let schema = Arc::new(Schema::new(vec![
        Field::new("geometry", DataType::Binary, true),
        Field::new("id", DataType::Int64, false),
    ]));
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(BinaryArray::from(vec![Some(punto.as_slice())])),
            Arc::new(Int64Array::from(vec![1_i64])),
        ],
    )
    .unwrap();
    let file = File::create(&percorso).unwrap();
    let mut writer = ArrowWriter::try_new(file, schema, None).unwrap();
    writer.write(&batch).unwrap();
    // Non e' base64: `*` e lo spazio sono fuori dall'alfabeto, e la
    // lunghezza non e' multipla di quattro.
    writer.append_key_value_metadata(KeyValue::new(
        "ARROW:schema".to_owned(),
        "non e' base64 *".to_owned(),
    ));
    writer.close().unwrap();

    let esito = GeoParquetDriver.open(Source::Path(percorso), opzioni_lettura());
    let Err(errore) = esito else {
        panic!("un ARROW:schema indecifrabile non deve aprirsi")
    };
    assert_eq!(errore.code, plenora_io_model::IoErrorCode::Format);
    let testo = errore.to_string();
    assert!(
        testo.contains("ARROW:schema non decodificabile da base64"),
        "il messaggio deve essere quello curato, e dice «{testo}»"
    );
    assert!(
        !testo.contains("in panico"),
        "il rifiuto deve precedere il panico, non seguirlo: {testo}"
    );
    // Il messaggio della libreria non esce: ne' l'offset, ne' il simbolo,
    // ne' i bit decodificati che `base64 0.23.1` mette nel proprio testo.
    for pezzo in ["Invalid", "offset", "symbol", "0x", "0b"] {
        assert!(
            !testo.contains(pezzo),
            "«{pezzo}» viene dall'errore della libreria e non deve uscire: {testo}"
        );
    }
}

/// La controprova positiva: l'`ARROW:schema` che scriviamo noi si legge.
///
/// Senza questa riga, una verifica che rifiutasse **ogni** `ARROW:schema`
/// passerebbe il test qui sopra e romperebbe ogni Parquet scritto da arrow
/// -- cioe' tutti, perche' `ArrowWriter` quella chiave la scrive sempre.
#[test]
fn l_arrow_schema_scritto_da_arrow_si_decodifica() {
    let dir = tempfile::tempdir().unwrap();
    let percorso = parquet_con_geo(&dir, None);
    let metadati = {
        use parquet::file::reader::FileReader as _;
        let file = File::open(&percorso).unwrap();
        let lettore = parquet::file::reader::SerializedFileReader::new(file).unwrap();
        lettore
            .metadata()
            .file_metadata()
            .key_value_metadata()
            .cloned()
    };
    let chiavi = metadati.expect("il footer porta metadati");
    assert!(
        chiavi.iter().any(|kv| kv.key == "ARROW:schema"),
        "ArrowWriter deve avere scritto ARROW:schema: senza, il test qui \
             sopra proverebbe il rifiuto di una chiave che nessuno mette"
    );
    GeoParquetDriver
        .open(Source::Path(percorso), opzioni_lettura())
        .expect("un ARROW:schema valido non deve essere rifiutato");
}

#[test]
fn un_file_storico_senza_opt_in_e_rifiutato() {
    // Il default e' il rifiuto, e deve restarlo: ogni GeoParquet che questo
    // repository ha scritto fino a S10 dichiara un `crs` che lo schema
    // ufficiale non ammette, e accettarlo in silenzio vorrebbe dire
    // chiamare conforme cio' che non lo e'.
    let esito = GeoParquetDriver.open(Source::Path(seme_storico()), opzioni_lettura());
    assert!(
        matches!(
            esito,
            Err(ref errore) if errore.code == plenora_io_model::IoErrorCode::Format
        ),
        "senza opt-in un file non conforme resta rifiutato"
    );
}

#[test]
fn un_file_storico_con_opt_in_si_apre_e_conserva_il_crs() {
    // Con l'opzione il file si apre, l'identificatore che dichiarava e'
    // conservato -- ignorarlo direbbe CRS84 di un dato che CRS84 non e' --
    // e il contratto **dichiara** che il file e' entrato da una via non
    // conforme: una deroga che non si vede diventa il comportamento
    // normale.
    let dataset = GeoParquetDriver
        .open(Source::Path(seme_storico()), opzioni_lettura_legacy())
        .expect("con l'opt-in il file storico si apre");
    let geometria = dataset.layers()[0]
        .contract
        .geometry
        .as_ref()
        .expect("colonna geometria");
    assert_eq!(geometria.crs.id(), Some("EPSG:4326"));
    assert_eq!(
        geometria
            .native_metadata
            .get("geoparquet.compatibilita")
            .map(String::as_str),
        Some("crs_storico_solo_identificatore_non_conforme")
    );
}

use arrow_array::Int64Array;
use arrow_schema::DataType;
use geo_types::{Geometry, Point};
use plenora_io_core::request::{BatchTarget, ReadScope};
use plenora_io_core::WriteLayer;
use plenora_io_model::wkb::{encode_wkb, to_wkb, WkbCoordinate, WkbFlavor, WkbGeometry, WkbValue};
use plenora_io_model::CancellationToken;

/// Opzioni di lettura con un budget di ingresso scelto.
/// Opzioni di lettura con memoria e ingresso scelti separatamente.
///
/// I due parametri sono distinti perche' e' esattamente cio' che i test di
/// FZ-0.2.1 devono poter muovere uno alla volta: il tetto sulla pagina
/// segue la memoria, non l'ingresso.
///
/// `max_wkb_cell_bytes` scende insieme alla memoria perche' il modello lo
/// pretende — una cella non puo' valere piu' di tutta la memoria — ma resta
/// **sopra** la geometria dei test, cosi' un rifiuto per cella non si
/// travesta da rifiuto per pagina.
fn opzioni_lettura_con(memoria: u64, ingresso: u64) -> ReadOptions {
    let limiti = plenora_io_model::budget::PipelineLimits::default()
        .with_memory_bytes(memoria)
        .with_max_input_bytes(ingresso)
        .with_max_wkb_cell_bytes(
            // `expect` e non un ripiego: il minimo con 64 MiB rende la
            // conversione impossibile da fallire, e degradare a `usize::MAX`
            // darebbe in silenzio un limite di cella assurdo proprio nel
            // test che lo sta scegliendo.
            usize::try_from(memoria.min(64 * 1024 * 1024))
                .expect("64 MiB stanno in un usize su qualunque piattaforma"),
        );
    match plenora_io_model::budget::PipelineBudget::builder()
        .limits(limiti)
        .build()
    {
        Ok(bundle) => ReadOptions::from_read_parts(bundle.into_read_parts()),
        Err(errore) => panic!("bundle non costruibile: {errore:?}"),
    }
}

/// La richiesta di lettura minima: nessuna projection, nessun pruning.
fn richiesta_semplice() -> ReadRequest {
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

/// Una riga rifiutata resta una riga rifiutata anche senza `input_total`.
///
/// Il report diagnostico pretende `input_total` positivo — è il contratto
/// `plenora-io-row-diagnostics-v1` — e chi scrive non è obbligato a
/// dichiararlo. Prima, in quel caso, l'errore diventava
/// `PlenoraIoError::Contract("input_total esatto richiesto …")`: la causa
/// primaria veniva sostituita da una condizione dell'infrastruttura
/// diagnostica, e chi leggeva vedeva un problema interno al posto del
/// proprio.
///
/// Il test fissa le quattro cose che devono restare vere: categoria, fase,
/// causa, e il fatto che il totale **non** venga inventato.
#[test]
fn una_riga_rifiutata_senza_input_total_conserva_causa_e_categoria() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("rifiutato.parquet");

    // Contratto con geometria **non nullable**, batch con un nullo: e' la
    // violazione piu' economica da produrre, e passa dallo stesso percorso
    // di tutte le altre.
    let schema: SchemaRef = Arc::new(Schema::new(vec![Field::new(
        "geometry",
        DataType::Binary,
        true,
    )
    .with_metadata(geometry_field_meta("EPSG:4326"))]));
    let colonna = BinaryArray::from(vec![None::<&[u8]>]);
    let batch = RecordBatch::try_new(schema.clone(), vec![Arc::new(colonna)]).unwrap();
    let mut geometria = GeometryColumnContract::wkb_passthrough(
        FieldId(0),
        "geometry",
        ResolvedCrs::new(Some("EPSG:4326".to_owned()), CrsKind::Geographic, None),
        false,
    );
    geometria.set_exact_geometry_types(vec![GeometryType::Point]);
    let plan = WritePlan {
        layers: vec![WriteLayer {
            name: "l".to_owned(),
            contract: DataContract {
                schema,
                geometry: Some(geometria),
            },
        }],
    };

    let mut writer = GeoParquetDriver
        .create(Sink::Path(path.clone()), &plan, &opzioni_scrittura())
        .expect("il piano e' valido: e' la riga a non esserlo");
    // Nessun `declare_input_total`: e' il caso in esame.
    let errore = writer
        .write(&batch)
        .expect_err("una geometria nulla su contratto non nullable e' rifiutata");

    assert_eq!(
        errore.category,
        plenora_io_model::ErrorCategory::DataMapping
    );
    assert_eq!(errore.phase, plenora_io_model::ErrorPhase::Write);
    assert!(
        errore.message.contains("contract.nullability"),
        "la causa primaria deve sopravvivere: {errore}"
    );
    assert!(
        !errore.message.contains("input_total"),
        "l'assenza del totale non e' la causa del rifiuto: {errore}"
    );
    // Il totale non viene inventato: senza `input_total` non c'e' report.
    assert!(
        errore.row_diagnostics.is_none(),
        "un report con un totale che nessuno ha dichiarato sarebbe peggio \
             di nessun report"
    );

    // Nessuna pubblicazione e nessun output parziale: lo staging non
    // diventa mai il file di destinazione.
    drop(writer);
    assert!(
        !path.exists(),
        "una scrittura rifiutata non deve lasciare il file: {}",
        path.display()
    );
    let residui: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|voce| voce.unwrap().file_name())
        .collect();
    assert!(
        residui.is_empty(),
        "la directory deve restare vuota, trovato: {residui:?}"
    );
}

/// Con `input_total` dichiarato il report c'e', e porta la stessa causa.
///
/// E' la controprova del test sopra: senza di essa «nessun report» potrebbe
/// voler dire «il report non funziona piu'» invece di «qui non e'
/// emettibile».
#[test]
fn una_riga_rifiutata_con_input_total_porta_il_report() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("rifiutato-con-totale.parquet");

    let schema: SchemaRef = Arc::new(Schema::new(vec![Field::new(
        "geometry",
        DataType::Binary,
        true,
    )
    .with_metadata(geometry_field_meta("EPSG:4326"))]));
    let colonna = BinaryArray::from(vec![None::<&[u8]>]);
    let batch = RecordBatch::try_new(schema.clone(), vec![Arc::new(colonna)]).unwrap();
    let mut geometria = GeometryColumnContract::wkb_passthrough(
        FieldId(0),
        "geometry",
        ResolvedCrs::new(Some("EPSG:4326".to_owned()), CrsKind::Geographic, None),
        false,
    );
    geometria.set_exact_geometry_types(vec![GeometryType::Point]);
    let plan = WritePlan {
        layers: vec![WriteLayer {
            name: "l".to_owned(),
            contract: DataContract {
                schema,
                geometry: Some(geometria),
            },
        }],
    };

    let mut writer = GeoParquetDriver
        .create(Sink::Path(path.clone()), &plan, &opzioni_scrittura())
        .unwrap();
    writer.declare_input_total(LayerId(0), 1).unwrap();
    let errore = writer.write(&batch).expect_err("la riga e' rifiutata");

    assert_eq!(
        errore.category,
        plenora_io_model::ErrorCategory::DataMapping
    );
    assert_eq!(errore.phase, plenora_io_model::ErrorPhase::Write);
    let report = errore
        .row_diagnostics
        .as_ref()
        .expect("con il totale dichiarato il report e' emettibile");
    assert_eq!(report.input_total, Some(1));
    assert!(
        report.counts.contains_key("contract.nullability"),
        "{report:?}"
    );

    drop(writer);
    assert!(!path.exists(), "nemmeno qui si pubblica niente");
}

/// Omettere `compression` equivale a dichiararla `snappy` (S6, correzione).
///
/// Il test trasversale `il_default_dichiarato_e_quello_applicato` confronta
/// il **contratto riletto**, che e' cio' che `DeterminismLevel::Semantic`
/// promette. Il codec non compare li': due file identici nel contratto
/// possono essere compressi in modo diverso, ed e' proprio il caso che S6
/// esisteva per chiudere.
///
/// Qui il codec si legge dove sta davvero, nel footer, e i tre casi sono
/// distinti: omesso, dichiarato al default, dichiarato diverso. Senza il
/// terzo, «omesso == snappy» potrebbe voler dire che il driver ignora
/// l'opzione del tutto.
#[test]
fn il_default_di_compression_e_snappy() {
    let dir = tempfile::tempdir().unwrap();
    let codec_di = |nome: &str, opzioni: WriteOptions| {
        let path = dir.path().join(format!("{nome}.parquet"));
        let schema: SchemaRef = Arc::new(Schema::new(vec![Field::new(
            "geometry",
            DataType::Binary,
            true,
        )
        .with_metadata(geometry_field_meta("EPSG:4326"))]));
        let wkb = encode_wkb(
            &WkbGeometry {
                value: WkbValue::Point(WkbCoordinate {
                    x: 1.0,
                    y: 2.0,
                    z: None,
                    m: None,
                }),
                dimensions: CoordinateDimensions::Xy,
                srid: None,
            },
            WkbFlavor::Iso,
        )
        .unwrap();
        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![Arc::new(BinaryArray::from(vec![Some(wkb.as_slice())]))],
        )
        .unwrap();
        let mut geometria = GeometryColumnContract::wkb_passthrough(
            FieldId(0),
            "geometry",
            ResolvedCrs::new(Some("EPSG:4326".to_owned()), CrsKind::Geographic, None),
            true,
        );
        geometria.set_exact_geometry_types(vec![GeometryType::Point]);
        let plan = WritePlan {
            layers: vec![WriteLayer {
                name: "l".to_owned(),
                contract: DataContract {
                    schema,
                    geometry: Some(geometria),
                },
            }],
        };
        let mut writer = GeoParquetDriver
            .create(Sink::Path(path.clone()), &plan, &opzioni)
            .unwrap();
        writer.write(&batch).unwrap();
        writer.finish().unwrap();
        let builder = ParquetRecordBatchReaderBuilder::try_new(File::open(&path).unwrap()).unwrap();
        builder.metadata().row_groups()[0].columns()[0].compression()
    };

    let mut esplicito = opzioni_scrittura();
    esplicito
        .format_options
        .insert("compression".to_owned(), "snappy".to_owned());
    let mut diverso = opzioni_scrittura();
    diverso
        .format_options
        .insert("compression".to_owned(), "zstd".to_owned());

    let omesso = codec_di("omesso", opzioni_scrittura());
    assert_eq!(omesso, codec_di("esplicito", esplicito));
    assert_eq!(omesso, Compression::SNAPPY);
    // Controprova: l'opzione non viene ignorata.
    assert_ne!(omesso, codec_di("diverso", diverso));
}

/// Il tetto per pagina si deriva dalla **memoria dichiarata**, e da nulla
/// altro (FZ-0.2.1).
///
/// L'ultimo caso e' il piu' importante e non e' un'omissione: dichiarare
/// quattro gigabyte su una macchina che ne ha mezzo produce un tetto da due
/// gigabyte, e la libreria **non** se ne accorge. E' un errore di
/// deployment, non qualcosa che questa funzione prometta di rilevare:
/// leggere la memoria reale del processo e' instabile e non portabile, e
/// una promessa del genere sarebbe falsa su qualche piattaforma.
#[test]
fn il_tetto_per_pagina_segue_la_memoria_dichiarata() {
    let tetto_di = |memoria: u64, ingresso: u64| {
        let opzioni = opzioni_lettura_con(memoria, ingresso);
        tetto_pagina(opzioni.budget().context())
    };

    // Meta' della memoria, quale che sia l'ingresso.
    assert_eq!(tetto_di(512 * 1024 * 1024, 1 << 20), 256 * 1024 * 1024);
    assert_eq!(tetto_di(512 * 1024 * 1024, 1 << 34), 256 * 1024 * 1024);
    // Meno memoria, meno tetto.
    assert_eq!(tetto_di(8 * 1024 * 1024, 1 << 30), 4 * 1024 * 1024);
    // Piu' memoria di quanta la macchina ne abbia: il tetto la segue
    // comunque, perche' e' la dichiarazione a governare.
    assert_eq!(
        tetto_di(4 * 1024 * 1024 * 1024, 1 << 20),
        2 * 1024 * 1024 * 1024
    );
    // I predefiniti sono quelli attesi: 512 MiB dichiarati, 256 di tetto.
    assert_eq!(
        tetto_pagina(opzioni_lettura().budget().context()),
        256 * 1024 * 1024
    );
}

/// Una pagina **coerente** col proprio chunk ma sopra il budget dichiarato
/// viene comunque rifiutata (FZ-0.2, revisione).
///
/// E' il caso che il solo controllo di coerenza non prende: un chunk da
/// 800 MiB con una pagina da 700 MiB non mente su niente, e aborta lo
/// stesso su un processo che di memoria ne ha mezza. Qui la stessa forma in
/// piccolo — un file molto comprimibile, il cui unico dato non compresso e'
/// piu' grande del budget che il chiamante ha dichiarato.
///
/// Il file **passa** il preflight: e' piu' piccolo del budget. E' la pagina
/// decompressa a non entrarci, ed e' esattamente cio' che il budget dice di
/// non potersi permettere.
#[test]
fn una_pagina_coerente_ma_sopra_il_budget_e_rifiutata() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("comprimibile.parquet");
    // **Una riga sola**, con una geometria grande.
    //
    // Le prime versioni usavano molte righe: o i valori si ripetevano — e
    // il dizionario riduceva la pagina piu' grande a venticinque byte — o
    // erano distinti, e allora portavano entropia sufficiente a rendere il
    // file piu' grande della pagina. In mezzo non c'e' spazio: per battere
    // il dizionario servono decine di migliaia di valori diversi, e quelli
    // non si comprimono.
    //
    // Un unico WKB da un megabyte scioglie la tensione: la pagina e' grande
    // per costruzione, le quattro colonne bbox del covering hanno una riga
    // sola e non pesano niente, e le coordinate quasi identiche si
    // comprimono benissimo. Il file resta di qualche decina di kilobyte.
    let vertici: Vec<WkbCoordinate> = (0..60_000)
        .map(|i| {
            let x = f64::from(i).mul_add(1e-12, 1.0);
            WkbCoordinate {
                x,
                y: x,
                z: None,
                m: None,
            }
        })
        .collect();
    let schema: SchemaRef = Arc::new(Schema::new(vec![Field::new(
        "geometry",
        DataType::Binary,
        true,
    )
    .with_metadata(geometry_field_meta("EPSG:4326"))]));
    let wkb = encode_wkb(
        &WkbGeometry {
            value: WkbValue::LineString(vertici),
            dimensions: CoordinateDimensions::Xy,
            srid: None,
        },
        WkbFlavor::Iso,
    )
    .unwrap();
    let colonna = BinaryArray::from(vec![Some(wkb.as_slice())]);
    let batch = RecordBatch::try_new(schema.clone(), vec![Arc::new(colonna)]).unwrap();
    let mut geometria = GeometryColumnContract::wkb_passthrough(
        FieldId(0),
        "geometry",
        ResolvedCrs::new(Some("EPSG:4326".to_owned()), CrsKind::Geographic, None),
        true,
    );
    geometria.set_exact_geometry_types(vec![GeometryType::LineString]);
    let plan = WritePlan {
        layers: vec![WriteLayer {
            name: "l".to_owned(),
            contract: DataContract {
                schema,
                geometry: Some(geometria),
            },
        }],
    };
    let mut writer = GeoParquetDriver
        .create(Sink::Path(path.clone()), &plan, &opzioni_scrittura())
        .unwrap();
    writer.write(&batch).unwrap();
    writer.finish().unwrap();

    let byte_su_disco = std::fs::metadata(&path).unwrap().len();
    let pagina_piu_grande = pagina_non_compressa_massima(&path);
    // La pagina decompressa deve essere piu' grande del file: e' la
    // condizione che rende il caso interessante — il file passa il
    // preflight, la memoria no. Verificata invece che sperata, perche' un
    // file poco comprimibile renderebbe il test verde senza provare niente.
    assert!(
        pagina_piu_grande > byte_su_disco,
        "file poco comprimibile: pagina {pagina_piu_grande} B, file {byte_su_disco} B"
    );

    // Memoria appena sotto il doppio della pagina: il tetto e' la meta',
    // quindi cade appena sotto la pagina.
    let memoria_stretta = (pagina_piu_grande - 1) * 2;
    let ingresso_largo = 1 << 30;
    let leggi = |memoria: u64, ingresso: u64| {
        let mut opzioni = opzioni_lettura_con(memoria, ingresso);
        opzioni.assume_crs = None;
        let dataset = GeoParquetDriver
            .open(Source::Path(path.clone()), opzioni)
            .expect("il file sta dentro l'ingresso: l'apertura riesce");
        match dataset.open_layer_reader(&richiesta_semplice()) {
            Err(errore) => Err(errore),
            Ok(mut lettore) => lettore.next_batch().map(|_| ()),
        }
    };

    // (1) Memoria stretta: rifiutato, e con il messaggio della memoria.
    let errore = leggi(memoria_stretta, ingresso_largo)
        .expect_err("la pagina non entra nella memoria dichiarata");
    assert_eq!(
        errore.message,
        pagine::MSG_PAGINA_OLTRE_LA_MEMORIA,
        "{errore}"
    );

    // (2) **Alzare l'ingresso non alza il tetto sulla pagina.** E' la
    // ragione per cui FZ-0.2.1 ha cambiato quota: con `max_input_bytes` un
    // ingresso piu' largo rendeva ammissibile una pagina piu' grande, cioe'
    // rispondeva a una domanda che nessuno aveva fatto.
    let errore =
        leggi(memoria_stretta, ingresso_largo * 4).expect_err("l'ingresso non governa la memoria");
    assert_eq!(
        errore.message,
        pagine::MSG_PAGINA_OLTRE_LA_MEMORIA,
        "{errore}"
    );

    // (3) Alzare la memoria lo alza: stessa pagina, stesso file.
    leggi(memoria_stretta * 4, ingresso_largo).expect("con piu' memoria la pagina entra");

    // (4) E con i valori predefiniti — 512 MiB di memoria, tetto 256 MiB —
    // il file si legge come qualunque altro.
    let mut opzioni = opzioni_lettura();
    opzioni.assume_crs = None;
    let dataset = GeoParquetDriver.open(Source::Path(path), opzioni).unwrap();
    let mut lettore = dataset.open_layer_reader(&richiesta_semplice()).unwrap();
    assert!(lettore.next_batch().unwrap().is_some());
}

/// La piu' grande `uncompressed_page_size` dichiarata dal file.
///
/// Serve al test sopra per **verificare** la premessa invece di assumerla:
/// un file poco comprimibile renderebbe il test verde senza provare niente.
fn pagina_non_compressa_massima(path: &std::path::Path) -> u64 {
    use std::io::{Read, Seek, SeekFrom};
    let sorgente = Arc::new(File::open(path).unwrap());
    let builder = ParquetRecordBatchReaderBuilder::try_new(sorgente.try_clone().unwrap()).unwrap();
    let mut massima = 0_u64;
    for gruppo in builder.metadata().row_groups() {
        for chunk in gruppo.columns() {
            let mut offset = u64::try_from(inizio_del_chunk(chunk).unwrap()).unwrap();
            let fine = offset + u64::try_from(chunk.compressed_size()).unwrap();
            let mut handle = sorgente.try_clone().unwrap();
            while offset < fine {
                handle.seek(SeekFrom::Start(offset)).unwrap();
                let quanti = usize::try_from((fine - offset).min(65_536)).unwrap();
                let mut buffer = vec![0_u8; quanti];
                let mut letti = 0;
                while letti < buffer.len() {
                    match handle.read(&mut buffer[letti..]) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => letti += n,
                    }
                }
                let testa = pagine::leggi_intestazione(&buffer[..letti]).unwrap();
                massima = massima.max(u64::try_from(testa.non_compressi).unwrap());
                offset += u64::try_from(testa.byte_header).unwrap()
                    + u64::try_from(testa.compressi).unwrap();
            }
        }
    }
    massima
}

/// Un chunk che dichiara la pagina di dizionario **dopo** le pagine dati
/// viene rifiutato invece di essere interpretato (FZ-0.2, revisione).
///
/// In parquet 59.1.0 ogni consumatore passa da `byte_range`, che sceglie il
/// dizionario quando c'e' — la stessa regola nostra — quindi oggi non c'e'
/// divergenza. Ma quell'accordo e' **condizionato**: dipende dal fatto che
/// la libreria continui a scegliere come noi. Su un chunk che non puo'
/// esistere — il dizionario precede sempre i dati che indicizza — rifiutare
/// e' piu' economico che restare d'accordo per fortuna.
#[test]
fn un_chunk_con_il_dizionario_dopo_i_dati_e_rifiutato() {
    use parquet::basic::{Encoding, Type as PhysicalType};
    use parquet::schema::types::{ColumnDescriptor, ColumnPath, Type as SchemaType};

    let tipo = Arc::new(
        SchemaType::primitive_type_builder("c", PhysicalType::INT32)
            .build()
            .unwrap(),
    );
    let descrittore = Arc::new(ColumnDescriptor::new(tipo, 0, 0, ColumnPath::new(vec![])));

    let coerente = parquet::file::metadata::ColumnChunkMetaData::builder(descrittore.clone())
        .set_encodings(vec![Encoding::PLAIN])
        .set_dictionary_page_offset(Some(100))
        .set_data_page_offset(200)
        .set_total_compressed_size(50)
        .set_total_uncompressed_size(50)
        .build()
        .unwrap();
    assert_eq!(inizio_del_chunk(&coerente).unwrap(), 100);

    let invertito = parquet::file::metadata::ColumnChunkMetaData::builder(descrittore.clone())
        .set_encodings(vec![Encoding::PLAIN])
        .set_dictionary_page_offset(Some(300))
        .set_data_page_offset(200)
        .set_total_compressed_size(50)
        .set_total_uncompressed_size(50)
        .build()
        .unwrap();
    let errore = inizio_del_chunk(&invertito).expect_err("gli offset invertiti sono rifiutati");
    assert_eq!(errore.message, MSG_DIZIONARIO_DOPO_I_DATI, "{errore}");

    // Senza dizionario si prende la prima pagina dati, come fa `byte_range`.
    let senza = parquet::file::metadata::ColumnChunkMetaData::builder(descrittore)
        .set_encodings(vec![Encoding::PLAIN])
        .set_data_page_offset(200)
        .set_total_compressed_size(50)
        .set_total_uncompressed_size(50)
        .build()
        .unwrap();
    assert_eq!(inizio_del_chunk(&senza).unwrap(), 200);
}

/// La catena delle pagine deve chiudere **esattamente** sulla fine del
/// chunk (FZ-0.2, revisione).
///
/// Fermarsi a «l'abbiamo superata» lascerebbe passare un'ultima pagina che
/// sborda: i byte oltre il chunk sono di un'altra colonna, e una pagina che
/// li rivendica non e' lunga, e' un chunk che mente.
#[test]
fn la_catena_delle_pagine_deve_chiudere_esatta() {
    const BUDGET: u64 = 1 << 20;

    let dir = tempfile::tempdir().unwrap();

    // Header valido: type=0, uncompressed=4, compressed=4, poi 4 byte di
    // dati. In tutto 11 byte.
    let mut pagina = vec![0x15, 0x00, 0x15, 0x08, 0x15, 0x08, 0x00];
    pagina.extend_from_slice(&[1, 2, 3, 4]);
    assert_eq!(pagina.len(), 11);

    let scrivi = |nome: &str, dati: &[u8]| {
        let path = dir.path().join(nome);
        std::fs::write(&path, dati).unwrap();
        File::open(path).unwrap()
    };

    // Chunk lungo esattamente quanto la pagina: chiude.
    let file = scrivi("esatto", &pagina);
    pagine::valida_chunk(&file, 0, 11, 64, BUDGET).expect("la catena chiude esatta");

    // Chunk dichiarato piu' corto della pagina: la pagina sborda.
    let file = scrivi("corto", &pagina);
    let errore = pagine::valida_chunk(&file, 0, 9, 64, BUDGET)
        .expect_err("una pagina che sborda dal chunk e' rifiutata");
    assert_eq!(
        errore.message,
        pagine::MSG_CATENA_PAGINE_NON_CHIUDE,
        "{errore}"
    );

    // Chunk dichiarato piu' lungo: resta un buco che nessuna pagina copre.
    let mut lungo = pagina.clone();
    lungo.extend_from_slice(&[0; 3]);
    let file = scrivi("lungo", &lungo);
    let errore = pagine::valida_chunk(&file, 0, 14, 64, BUDGET)
        .expect_err("un chunk con byte non coperti da pagine e' rifiutato");
    assert_eq!(
        errore.message,
        pagine::MSG_HEADER_PAGINA_ILLEGGIBILE,
        "{errore}"
    );

    // Il budget morde anche su una pagina coerente col chunk.
    let file = scrivi("budget", &pagina);
    let errore = pagine::valida_chunk(&file, 0, 11, 64, 2)
        .expect_err("una pagina sopra il budget e' rifiutata");
    assert_eq!(
        errore.message,
        pagine::MSG_PAGINA_OLTRE_LA_MEMORIA,
        "{errore}"
    );
}

/// Una pagina che dichiara piu' byte non compressi del proprio chunk viene
/// rifiutata **prima** che il decoder ne allochi la decompressione (FZ-0.2).
///
/// La verifica non e' "non va in panico", e non potrebbe esserlo: senza
/// questa prevalidazione l'esito non e' un panico ma un **abort**.
/// `Vec::with_capacity` che fallisce chiama l'alloc error handler, che
/// termina il processo senza unwinding — nessun `catch_unwind` lo vede, e
/// nessun test lo sopravvive per raccontarlo. Misurato sul seme sotto
/// `RLIMIT_AS` di 512 MiB:
///
/// ```text
/// memory allocation of 2000000000 bytes failed
/// exit 134 (SIGABRT)
/// ```
///
/// Questo test puo' quindi provare solo il verso positivo — il rifiuto
/// tipizzato — e lo dice invece di lasciar credere che copra l'abort. La
/// prova che senza la prevalidazione l'abort avviene sta in
/// `fuzz-findings/2026-08-18-parquet-uncompressed-page-size/dimostra.sh`,
/// che gira in sottoprocesso apposta perche' l'esito non e' catturabile.
#[test]
fn una_pagina_oltre_il_proprio_chunk_e_rifiutata_prima_dell_allocazione() {
    let seme = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fuzz/seeds/geoparquet_reader/pagina-oltre-il-chunk.parquet");
    assert!(seme.is_file(), "seme assente: {}", seme.display());

    let dataset = GeoParquetDriver
        // Seme storico: `crs` nella forma non conforme, quindi opt-in.
        .open(Source::Path(seme), opzioni_lettura_legacy())
        .expect("l'apertura legge i soli metadati e riesce");
    let richiesta = ReadRequest {
        layer: LayerId(0),
        projected_fields: None,
        projection_mode: ProjectionMode::BestEffort,
        pruning_predicate: None,
        spatial_pruning_hint: None,
        scope: ReadScope::default(),
        batch_target: BatchTarget::default(),
        cancellation: CancellationToken::default(),
    };

    let errore = match dataset.open_layer_reader(&richiesta) {
        Err(errore) => errore,
        Ok(mut lettore) => match lettore.next_batch() {
            Err(errore) => errore,
            Ok(_) => panic!("il file doveva essere rifiutato"),
        },
    };

    assert_eq!(errore.phase, plenora_io_model::ErrorPhase::Read);
    assert_eq!(
        errore.category,
        plenora_io_model::ErrorCategory::DataMapping
    );
    // Il messaggio e' una delle costanti statiche del modulo: non porta
    // niente che venga dal file.
    assert!(
        errore.message == pagine::MSG_PAGINA_OLTRE_LA_MEMORIA
            || errore.message == pagine::MSG_PAGINA_OLTRE_IL_CHUNK,
        "messaggio inatteso: {errore}"
    );
    assert!(
        !errore.message.contains("in panico"),
        "il rifiuto deve precedere il decoder: {errore}"
    );
}

/// Gli header di pagina si leggono senza allocare, e i casi ostili si
/// fermano invece di far girare il ciclo.
#[test]
fn il_lettore_di_header_di_pagina_regge_gli_input_ostili() {
    // Header minimo valido: campo 1 (type) = 0, campo 2 = 10, campo 3 = 12.
    let valido = [0x15, 0x00, 0x15, 0x14, 0x15, 0x18, 0x00];
    let letto = pagine::leggi_intestazione(&valido).expect("header valido");
    assert_eq!(letto.non_compressi, 10);
    assert_eq!(letto.compressi, 12);
    assert_eq!(letto.byte_header, valido.len());

    // Troncato a meta': non si inventa il resto.
    assert!(pagine::leggi_intestazione(&valido[..3]).is_err());
    // Senza STOP: la fetta finisce prima.
    assert!(pagine::leggi_intestazione(&valido[..valido.len() - 1]).is_err());
    // Campo 3 assente: mancherebbe il passo della catena.
    assert!(pagine::leggi_intestazione(&[0x15, 0x00, 0x15, 0x14, 0x00]).is_err());
    // Tipo Thrift inesistente (0x0F).
    assert!(pagine::leggi_intestazione(&[0x1F, 0x00, 0x00]).is_err());
    // Varint che non termina mai.
    assert!(pagine::leggi_intestazione(&[0x15, 0xFF, 0xFF, 0xFF, 0xFF]).is_err());
    // Struct annidate all'infinito: la profondita' e' limitata.
    let mut annidate = vec![0x15, 0x00, 0x15, 0x14, 0x15, 0x18];
    annidate.extend(std::iter::repeat_n(0x1C_u8, 64)); // campo struct, ripetuto
    assert!(pagine::leggi_intestazione(&annidate).is_err());

    // --- limiti che la revisione di FZ-0.2 ha imposto ------------------

    // Un elenco che dichiara piu' elementi dei byte residui non e' lungo,
    // e' falso: ogni elemento ne consuma almeno uno. Senza il controllo il
    // ciclo girerebbe finche' il primo salto non fallisce — esito giusto,
    // tempo arbitrario.
    let elenco_enorme = [
        0x15, 0x00, 0x15, 0x14, 0x15, 0x18, // campi 1, 2, 3
        0x19, 0xF5, // campo 4: lista di i32, conteggio in forma lunga
        0xFF, 0xFF, 0xFF, 0xFF, 0x0F, // 4 294 967 295 elementi
        0x00,
    ];
    assert!(pagine::leggi_intestazione(&elenco_enorme).is_err());

    // Un booleano **dentro un elenco** e' un byte a se'; dentro una struct
    // sta nel field header. Trattarli allo stesso modo legge l'elenco senza
    // avanzare, e da li' ogni offset e' sbagliato. Qui la lista dichiara
    // due booleani e li porta davvero: l'header deve leggersi.
    let booleani_in_lista = [
        0x15, 0x00, 0x15, 0x14, 0x15, 0x18, // campi 1, 2, 3
        0x19, 0x21, // campo 4: lista di 2 booleani
        0x01, 0x02, // i due byte degli elementi
        0x00,
    ];
    let letto = pagine::leggi_intestazione(&booleani_in_lista)
        .expect("i booleani in lista consumano un byte ciascuno");
    assert_eq!(letto.byte_header, booleani_in_lista.len());
    // Se gli elementi non fossero consumati, lo STOP finirebbe due byte
    // prima e l'header risulterebbe piu' corto: e' esattamente il difetto.
    let senza_i_due_byte = [0x15, 0x00, 0x15, 0x14, 0x15, 0x18, 0x19, 0x21, 0x00];
    assert!(pagine::leggi_intestazione(&senza_i_due_byte).is_err());

    // Un varint che non entra in u64: il byte terminale porta bit che il
    // file non poteva scrivere. Troncarlo in silenzio darebbe un valore
    // diverso da quello che il decoder leggera'.
    let varint_troppo_largo = [
        0x15, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x7F, 0x00,
    ];
    assert!(pagine::leggi_intestazione(&varint_troppo_largo).is_err());
}

/// Il seme che faceva panicare `parquet` sul bit width degli indici di
/// dizionario viene ora rifiutato **prima** che il decoder lo usi (FZ-0.1).
///
/// La verifica non e' "non va in panico": e' che la lettura si fermi con un
/// errore tipizzato e senza aver emesso righe. Un file che venisse accettato
/// e letto in silenzio sarebbe un esito peggiore del panico.
#[test]
fn un_bit_width_di_dizionario_fuori_intervallo_e_rifiutato_prima_del_decoder() {
    let seme = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fuzz/seeds/geoparquet_reader/bit-width-dizionario-fuori-intervallo.parquet");
    assert!(seme.is_file(), "seme assente: {}", seme.display());

    let dataset = GeoParquetDriver
        // Seme storico: `crs` nella forma non conforme, quindi opt-in.
        .open(Source::Path(seme), opzioni_lettura_legacy())
        .expect("l'apertura legge i soli metadati e riesce");
    let richiesta = ReadRequest {
        layer: LayerId(0),
        projected_fields: None,
        projection_mode: ProjectionMode::BestEffort,
        pruning_predicate: None,
        spatial_pruning_hint: None,
        scope: ReadScope::default(),
        batch_target: BatchTarget::default(),
        cancellation: CancellationToken::default(),
    };

    // Il rifiuto arriva da `open_layer_reader`, dove projection e pruning
    // sono noti: e' li' che la prevalidazione guarda i chunk selezionati.
    let errore = match dataset.open_layer_reader(&richiesta) {
        Err(errore) => errore,
        Ok(mut lettore) => match lettore.next_batch() {
            Err(errore) => errore,
            Ok(_) => panic!("il file doveva essere rifiutato"),
        },
    };

    assert_eq!(errore.phase, plenora_io_model::ErrorPhase::Read);
    assert_eq!(errore.code, plenora_io_model::IoErrorCode::Format);
    // Il panico e' *impedito*: se il messaggio venisse dalla barriera,
    // il decoder sarebbe stato raggiunto lo stesso.
    assert!(
        !errore.message.contains("in panico"),
        "il rifiuto deve precedere il decoder: {errore}"
    );
    assert!(
        errore.message.contains("bit width"),
        "l'errore deve dire cosa non va: {errore}"
    );
}

/// Il seme che faceva panicare arrow sui livelli di definizione viene ora
/// rifiutato **prima** del decoder.
///
/// # Che cosa faceva
///
/// Un run bit-packed dei livelli di definizione dichiarava piu' gruppi di
/// quanti byte la propria sezione ne portasse. Con `max_def_level` uguale a
/// uno `parquet` non passa da un decoder generico: usa `PackedDecoder`, che
/// consegna una fetta a `BooleanBufferBuilder::append_packed_range`, e
/// l'intervallo usciva dal buffer — `offset + len out of bounds`,
/// `arrow-buffer 59.1.0`, `util/bit_chunk_iterator.rs:224`.
///
/// # Perche' la prevalidazione che c'era non lo prendeva
///
/// Guardava le dimensioni delle pagine e il bit width degli indici di
/// dizionario, e questa pagina e' coerente in tutti e due: le sue dimensioni
/// tornano, e il run malformato sta **dentro** la sezione dei livelli, dove
/// nessuna delle due domande arrivava.
///
/// # Che cosa verifica
///
/// Non «non va in panico»: che la lettura si fermi con un errore tipizzato,
/// che il messaggio venga dalla prevalidazione e non dalla barriera — se
/// venisse dalla barriera il decoder sarebbe stato raggiunto lo stesso — e
/// che non porti fuori un byte del payload.
#[test]
fn un_run_dei_livelli_oltre_la_sezione_e_rifiutato_prima_del_decoder() {
    let seme = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fuzz/seeds/geoparquet_reader/livelli-run-oltre-la-sezione.parquet");
    assert!(seme.is_file(), "seme assente: {}", seme.display());

    let dataset = GeoParquetDriver
        .open(Source::Path(seme), opzioni_lettura_legacy())
        .expect("l'apertura legge i soli metadati e riesce");
    let richiesta = ReadRequest {
        layer: LayerId(0),
        projected_fields: None,
        projection_mode: ProjectionMode::BestEffort,
        pruning_predicate: None,
        spatial_pruning_hint: None,
        scope: ReadScope::default(),
        batch_target: BatchTarget::default(),
        cancellation: CancellationToken::default(),
    };

    let errore = match dataset.open_layer_reader(&richiesta) {
        Err(errore) => errore,
        Ok(mut lettore) => match lettore.next_batch() {
            Err(errore) => errore,
            Ok(_) => panic!("il file doveva essere rifiutato"),
        },
    };

    assert_eq!(errore.phase, plenora_io_model::ErrorPhase::Read);
    assert_eq!(errore.code, plenora_io_model::IoErrorCode::Format);
    assert!(
        !errore.message.contains("in panico"),
        "il rifiuto deve precedere il decoder: {errore}"
    );
    assert_eq!(
        errore.message,
        livelli::MSG_RUN_OLTRE_LA_SEZIONE,
        "il rifiuto deve venire dalla difesa che riguarda questo difetto"
    );
}

// --- i livelli, pagina per pagina ------------------------------------
//
// La diagnostica differenziale del checkpoint su `985e3ee` ha misurato 47
// righe cambiate e mai eseguite, e la meta' piu' grossa era
// `valida_livelli_v2`: il ramo V2 della barriera era scritto, compilato e
// mai percorso. Nessun seme versionato porta una `DataPageV2`, e nessuno
// arriva li' passando da un file.
//
// Una barriera contro un panico che nessun test attraversa e' una garanzia
// dichiarata, non una garanzia. Queste sonde la percorrono chiamando
// `valida_pagina` direttamente: e' la prevalidazione, cioe' il codice che
// gira **prima** del decoder, e provarla li' e' provarla dove agisce.

/// Un run bit-packed del flusso ibrido: intestazione piu' i propri byte.
///
/// `gruppi` gruppi da otto valori, `bit_width` byte ciascuno. Costruirlo qui
/// invece di scrivere le costanti a mano rende leggibile che cosa ogni sonda
/// sta rompendo: quasi tutte differiscono per un byte.
fn run_bit_packed(gruppi: u8, bit_width: usize) -> Vec<u8> {
    let mut flusso = vec![(gruppi << 1) | 1];
    flusso.extend(std::iter::repeat_n(0xAA, usize::from(gruppi) * bit_width));
    flusso
}

/// Una `DataPageV2` con le due sezioni dei livelli e un byte di valori.
fn pagina_v2(rep: &[u8], def: &[u8], num_values: u32) -> parquet::column::page::Page {
    let mut buf = Vec::new();
    buf.extend_from_slice(rep);
    buf.extend_from_slice(def);
    buf.push(0x00); // i valori: `PLAIN`, e nessuno li guarda qui
    parquet::column::page::Page::DataPageV2 {
        buf: buf.into(),
        num_values,
        encoding: parquet::basic::Encoding::PLAIN,
        num_nulls: 0,
        num_rows: num_values,
        def_levels_byte_len: u32::try_from(def.len()).unwrap(),
        rep_levels_byte_len: u32::try_from(rep.len()).unwrap(),
        is_compressed: false,
        statistics: None,
    }
}

/// L'errore di una prevalidazione: tipizzato, statico, senza payload.
fn rifiuto_statico(errore: &plenora_io_model::PlenoraIoError, atteso: &str) {
    assert_eq!(errore.code, plenora_io_model::IoErrorCode::Format);
    assert_eq!(errore.phase, plenora_io_model::ErrorPhase::Read);
    assert_eq!(errore.driver.as_deref(), Some("geoparquet"));
    assert_eq!(errore.message, atteso);
    // Il messaggio non porta fuori un byte del file: nessuna cifra
    // dell'intestazione del run, nessuna lunghezza letta.
    assert!(
        !errore.message.chars().any(|c| c.is_ascii_digit()),
        "il messaggio porta un numero derivato dal payload: {errore}"
    );
}

/// Una `DataPageV2` ben formata attraversa la prevalidazione.
///
/// E' la meta' positiva, e senza di lei le altre non direbbero niente: un
/// rifiuto che arriva sempre non distingue una pagina rotta da una sana.
#[test]
fn una_data_page_v2_ben_formata_passa_la_prevalidazione() {
    let livelli = run_bit_packed(1, 1); // otto valori a bit width uno
    valida_pagina(&pagina_v2(&livelli, &livelli, 8), 1, 1)
        .expect("le due sezioni coprono gli otto valori dichiarati");
}

/// Una sezione V2 che dichiara piu' byte di quanti la pagina ne porti.
///
/// E' il controllo che in V1 esisteva a meta' -- il prefisso poteva
/// dichiarare oltre la fine e l'unico limite era l'overflow -- e in V2 non
/// esisteva affatto: le lunghezze venivano dall'header e nessuno le
/// confrontava con il buffer.
#[test]
fn una_sezione_v2_troncata_e_rifiutata() {
    let livelli = run_bit_packed(1, 1);
    let parquet::column::page::Page::DataPageV2 { buf, .. } = pagina_v2(&livelli, &livelli, 8)
    else {
        unreachable!("costruita come V2")
    };
    // La sezione dei livelli di definizione dichiara dieci byte, e dopo
    // quelli di ripetizione ne restano tre.
    let pagina = parquet::column::page::Page::DataPageV2 {
        buf,
        num_values: 8,
        encoding: parquet::basic::Encoding::PLAIN,
        num_nulls: 0,
        num_rows: 8,
        def_levels_byte_len: 10,
        rep_levels_byte_len: u32::try_from(livelli.len()).unwrap(),
        is_compressed: false,
        statistics: None,
    };
    let errore = valida_pagina(&pagina, 1, 1).expect_err("la sezione esce dalla pagina");
    rifiuto_statico(&errore, MSG_LIVELLI_TRONCATI);
}

/// Un run V2 che promette piu' byte di quanti la propria sezione ne porti.
///
/// E' il difetto originale, sull'altra versione di pagina: l'intestazione
/// dichiara un gruppo, il byte del gruppo non c'e', e `PackedDecoder`
/// costruirebbe l'intervallo che esce dal buffer di arrow.
#[test]
fn un_run_v2_oltre_la_propria_sezione_e_rifiutato() {
    let sana = run_bit_packed(1, 1);
    let rotta = vec![sana[0]]; // l'intestazione senza il proprio byte
    let errore = valida_pagina(&pagina_v2(&sana, &rotta, 8), 1, 1)
        .expect_err("il run promette un byte che la sezione non ha");
    rifiuto_statico(&errore, livelli::MSG_RUN_OLTRE_LA_SEZIONE);
}

/// Anche la sezione di **ripetizione**, non solo quella di definizione.
///
/// Sono due sezioni e due iterazioni: una prevalidazione che guardasse solo
/// la seconda lascerebbe passare una pagina rotta nella prima, e il difetto
/// tornerebbe per l'altra meta' del formato. La sonda tiene la definizione
/// **sana** apposta, cosi' l'unico modo di essere rossa e' aver percorso il
/// ramo della ripetizione.
#[test]
fn anche_la_sezione_di_ripetizione_v2_e_verificata() {
    let sana = run_bit_packed(1, 1);
    let rotta = vec![sana[0]];
    let errore = valida_pagina(&pagina_v2(&rotta, &sana, 8), 1, 1)
        .expect_err("il run di ripetizione promette un byte che non c'e'");
    rifiuto_statico(&errore, livelli::MSG_RUN_OLTRE_LA_SEZIONE);
}

/// Una sezione V2 vuota per una pagina che dichiara valori.
///
/// Il flusso finisce prima di coprirli: il decoder proseguirebbe leggendo i
/// byte dei valori come se fossero livelli.
#[test]
fn una_sezione_v2_che_non_copre_i_valori_e_rifiutata() {
    let sana = run_bit_packed(1, 1);
    let errore =
        valida_pagina(&pagina_v2(&sana, &[], 8), 1, 1).expect_err("nessun livello per otto valori");
    rifiuto_statico(&errore, livelli::MSG_LIVELLI_INSUFFICIENTI);
}

/// La forma piu' comune: colonna piatta e nullable, quindi senza livelli di
/// ripetizione.
///
/// `max_rep_level` a zero significa che quella sezione non si verifica --
/// il decoder non la legge -- ma il salto si fa comunque, perche' la
/// lunghezza e' dichiarata nell'header e i valori cominciano dopo di essa.
/// E' il ramo che le altre sonde non toccano, ed e' quello che quasi tutti i
/// file percorrono.
#[test]
fn una_data_page_v2_senza_ripetizione_passa() {
    let def = run_bit_packed(1, 1);
    valida_pagina(&pagina_v2(&[], &def, 8), 0, 1)
        .expect("una colonna piatta e nullable non ha livelli di ripetizione");
}

/// Una codifica dei livelli che non e' ne' `RLE` ne' `BIT_PACKED`.
///
/// Ferma la lettura invece di far tirare a indovinare l'offset dei valori:
/// senza sapere quanto e' lunga la sezione, ogni byte successivo e'
/// interpretato a partire da un punto sbagliato.
#[test]
fn una_codifica_dei_livelli_ignota_ferma_la_prevalidazione() {
    let pagina = parquet::column::page::Page::DataPage {
        buf: vec![0xAA, 0x00].into(),
        num_values: 8,
        encoding: parquet::basic::Encoding::PLAIN,
        def_level_encoding: parquet::basic::Encoding::DELTA_BINARY_PACKED,
        rep_level_encoding: parquet::basic::Encoding::RLE,
        statistics: None,
    };
    let errore = valida_pagina(&pagina, 0, 1).expect_err("la codifica non e' riconosciuta");
    rifiuto_statico(&errore, MSG_CODIFICA_LIVELLI_IGNOTA);
}

/// Il ramo `BIT_PACKED` di V1: valido.
///
/// Deprecata dallo spec e ammessa, non e' il flusso ibrido -- e' un
/// impacchettamento piatto -- e la sua dimensione si calcola da
/// `num_values`. Era l'altro ramo che nessun test attraversava.
#[allow(deprecated)]
#[test]
fn il_ramo_bit_packed_di_v1_con_i_propri_byte_passa() {
    // Otto valori a bit width uno: un byte di livelli, poi i valori.
    let pagina = parquet::column::page::Page::DataPage {
        buf: vec![0xAA, 0x00].into(),
        num_values: 8,
        encoding: parquet::basic::Encoding::PLAIN,
        def_level_encoding: parquet::basic::Encoding::BIT_PACKED,
        rep_level_encoding: parquet::basic::Encoding::BIT_PACKED,
        statistics: None,
    };
    valida_pagina(&pagina, 0, 1).expect("un byte copre otto livelli a bit width uno");
}

/// Il ramo `BIT_PACKED` di V1: troncato.
///
/// La dimensione implicita dai valori dichiarati esce dalla pagina. Non c'e'
/// un run da attraversare -- non e' il flusso ibrido -- e cio' che va
/// verificato e' che quei byte ci siano: se la pagina finisce prima, il
/// decoder legge oltre il buffer per un'altra strada.
#[allow(deprecated)]
#[test]
fn il_ramo_bit_packed_di_v1_troncato_e_rifiutato() {
    // Sessantaquattro valori pretendono otto byte; la pagina ne ha due.
    let pagina = parquet::column::page::Page::DataPage {
        buf: vec![0xAA, 0x00].into(),
        num_values: 64,
        encoding: parquet::basic::Encoding::PLAIN,
        def_level_encoding: parquet::basic::Encoding::BIT_PACKED,
        rep_level_encoding: parquet::basic::Encoding::BIT_PACKED,
        statistics: None,
    };
    let errore = valida_pagina(&pagina, 0, 1).expect_err("otto byte di livelli non ci sono");
    rifiuto_statico(&errore, MSG_LIVELLI_TRONCATI);
}

/// I file che facevano panicare arrow decodificando lo schema devono
/// uscire come errore del driver, non abbattere il processo. Un `.parquet`
/// ci arriva perche' il footer puo' portare `ARROW:schema`, che viene
/// decodificato dallo stesso codice dell'IPC.
///
/// Il target di fuzzing non puo' verificarlo: `libfuzzer-sys` installa un
/// panic hook che chiama `abort()` prima dell'unwinding, quindi
/// `catch_unwind` non entra mai in gioco e il target continua a segnalare
/// un crash anche con la barriera al suo posto. Fuori dal fuzzer
/// l'unwinding e' quello di default — nel workspace non c'e' alcun
/// `panic = "abort"` — e la barriera si osserva.
#[test]
fn un_parquet_che_fa_panicare_arrow_diventa_un_errore_del_driver() {
    for nome in [
        "arrow-schema-che-fa-panicare.parquet",
        "arrow-schema-che-fa-panicare-2.parquet",
    ] {
        let seme = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fuzz/seeds/geoparquet_reader")
            .join(nome);
        match GeoParquetDriver.open(Source::Path(seme), opzioni_lettura()) {
            Ok(_) => panic!("{nome}: il file doveva essere rifiutato"),
            Err(errore) => {
                // FZ-0: il rifiuto precede arrow. Se il messaggio parlasse
                // di panico, la conversione sarebbe stata raggiunta lo
                // stesso e la prevalidazione non servirebbe a niente.
                assert!(
                    !errore.to_string().contains("in panico"),
                    "{nome}: il rifiuto deve precedere arrow: {errore}"
                );
                assert_eq!(errore.phase, plenora_io_model::ErrorPhase::Read);
            }
        }
    }
}

fn geometry_field_meta(crs: &str) -> HashMap<String, String> {
    let mut m = HashMap::new();
    m.insert(
        ARROW_EXTENSION_NAME_KEY.to_owned(),
        GEOARROW_WKB_EXTENSION.to_owned(),
    );
    m.insert(GEO_CRS_KEY.to_owned(), crs.to_owned());
    m
}

// Scrive un Parquet minimo senza metadata `geo` (simula un file legacy
// o esterno) con colonne: geometry (WKB pass-through), i 4 nomi
// convenzionali `_bbox_*` popolati con f64, e un attributo utente
// `id`. Il file NON dichiara covering `GeoParquet` 1.1: il driver deve
// trattare le `_bbox_*` come attributi utente per default.
// `minx`/`miny`/`maxx`/`maxy` sono le componenti canoniche di un
// bounding box: rinominarle per soddisfare `similar_names` peggiorerebbe
// la leggibilita' del test.
#[allow(clippy::similar_names)]
fn write_parquet_without_covering_metadata(path: &std::path::Path) {
    use arrow_array::Float64Array;
    let wkb: Vec<u8> = to_wkb(&Geometry::Point(Point::new(1.0, 2.0))).unwrap();
    let geom = BinaryArray::from(vec![Some(wkb.as_slice()), Some(wkb.as_slice())]);
    let minx = Float64Array::from(vec![1.0, 2.0]);
    let miny = Float64Array::from(vec![1.0, 2.0]);
    let maxx = Float64Array::from(vec![1.0, 2.0]);
    let maxy = Float64Array::from(vec![1.0, 2.0]);
    let ids = Int64Array::from(vec![1_i64, 2]);
    let schema = Arc::new(Schema::new(vec![
        Field::new("geometry", DataType::Binary, false),
        Field::new(BBOX_COLS[0], DataType::Float64, true),
        Field::new(BBOX_COLS[1], DataType::Float64, true),
        Field::new(BBOX_COLS[2], DataType::Float64, true),
        Field::new(BBOX_COLS[3], DataType::Float64, true),
        Field::new("id", DataType::Int64, false),
    ]));
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(geom),
            Arc::new(minx),
            Arc::new(miny),
            Arc::new(maxx),
            Arc::new(maxy),
            Arc::new(ids),
        ],
    )
    .unwrap();
    let file = File::create(path).unwrap();
    let mut writer = ArrowWriter::try_new(file, schema, None).unwrap();
    writer.write(&batch).unwrap();
    writer.close().unwrap();
}

// --- il driver davanti a un metadato `geo` -----------------------------
//
// Il modulo `metadati` e' provato campo per campo dalle sue sonde. Qui si
// prova l'altra meta': che il **driver** usi quella validazione, e che i
// comportamenti di punta del lotto -- un `geo` malformato rifiutato, un
// `geo` assente tollerato -- accadano aprendo un file vero.

/// Un parquet con una colonna geometria e il metadato `geo` che si vuole.
///
/// `None` scrive il file **senza** la chiave: e' il caso «Parquet
/// semplice», che resta legittimo e va distinto da un `geo` che c'e' e non
/// regge.
fn parquet_con_geo(dir: &tempfile::TempDir, geo: Option<&str>) -> std::path::PathBuf {
    let percorso = dir.path().join("con_geo.parquet");
    let punto: Vec<u8> = to_wkb(&Geometry::Point(Point::new(1.0, 2.0))).unwrap();
    let geom = BinaryArray::from(vec![Some(punto.as_slice())]);
    let ids = Int64Array::from(vec![1_i64]);
    let schema = Arc::new(Schema::new(vec![
        Field::new("geometry", DataType::Binary, true),
        Field::new("id", DataType::Int64, false),
    ]));
    let batch = RecordBatch::try_new(schema.clone(), vec![Arc::new(geom), Arc::new(ids)]).unwrap();
    let file = File::create(&percorso).unwrap();
    let mut writer = ArrowWriter::try_new(file, schema, None).unwrap();
    writer.write(&batch).unwrap();
    if let Some(documento) = geo {
        writer.append_key_value_metadata(KeyValue::new("geo".to_owned(), documento.to_owned()));
    }
    writer.close().unwrap();
    percorso
}

/// Come `parquet_con_geo`, ma con una **seconda colonna binaria** reale.
///
/// Serve alle colonne geometriche secondarie: dichiararne una che nel file
/// non c'e' non e' un contratto piu' ricco, e' un contratto falso.
fn parquet_con_due_binarie(dir: &tempfile::TempDir, geo: &str) -> std::path::PathBuf {
    let percorso = dir.path().join("due_binarie.parquet");
    let punto: Vec<u8> = to_wkb(&Geometry::Point(Point::new(1.0, 2.0))).unwrap();
    let schema = Arc::new(Schema::new(vec![
        Field::new("geometry", DataType::Binary, true),
        Field::new("altra", DataType::Binary, true),
        Field::new("id", DataType::Int64, false),
    ]));
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(BinaryArray::from(vec![Some(punto.as_slice())])),
            Arc::new(BinaryArray::from(vec![Some(punto.as_slice())])),
            Arc::new(Int64Array::from(vec![1_i64])),
        ],
    )
    .unwrap();
    let file = File::create(&percorso).unwrap();
    let mut writer = ArrowWriter::try_new(file, schema, None).unwrap();
    writer.write(&batch).unwrap();
    writer.append_key_value_metadata(KeyValue::new("geo".to_owned(), geo.to_owned()));
    writer.close().unwrap();
    percorso
}

/// Un file con una colonna `bbox` della **forma** che si chiede.
///
/// Forma vuol dire tutto cio' che la specifica prescrive e che il metadato
/// non porta: quali figli, in che ordine, di che tipo, e con quale
/// ripetizione rispetto alla geometria. Una fixture che sapesse costruire
/// solo la forma giusta non potrebbe provare nessuno dei rifiuti.
fn parquet_con_covering(
    dir: &tempfile::TempDir,
    geo: &str,
    figli: &[(&str, DataType)],
    bbox_nullable: bool,
    geometria_nullable: bool,
) -> std::path::PathBuf {
    let percorso = dir.path().join("con_covering.parquet");
    let punto: Vec<u8> = to_wkb(&Geometry::Point(Point::new(1.0, 2.0))).unwrap();
    let campi: Vec<Arc<Field>> = figli
        .iter()
        .map(|(nome, tipo)| Arc::new(Field::new(*nome, tipo.clone(), true)))
        .collect();
    let valori: Vec<ArrayRef> = figli
        .iter()
        .map(|(_, tipo)| valore_del_figlio(tipo))
        .collect();
    let bbox = StructArray::try_new(campi.clone().into(), valori, None).unwrap();
    let schema = Arc::new(Schema::new(vec![
        Field::new("geometry", DataType::Binary, geometria_nullable),
        Field::new(BBOX_STRUCT, DataType::Struct(campi.into()), bbox_nullable),
    ]));
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(BinaryArray::from(vec![Some(punto.as_slice())])),
            Arc::new(bbox),
        ],
    )
    .unwrap();
    let file = File::create(&percorso).unwrap();
    let mut writer = ArrowWriter::try_new(file, schema, None).unwrap();
    writer.write(&batch).unwrap();
    writer.append_key_value_metadata(KeyValue::new("geo".to_owned(), geo.to_owned()));
    writer.close().unwrap();
    percorso
}

/// Un valore per un figlio della struct, del tipo che il figlio dichiara.
///
/// Il caso `Struct` serve a costruire un figlio che **non** e' una colonna
/// di valori: la specifica vuole quattro numeri, e un gruppo annidato passa
/// il controllo dei nomi e dell'ordine senza essere un numero.
fn valore_del_figlio(tipo: &DataType) -> ArrayRef {
    match tipo {
        DataType::Float32 => {
            Arc::new(arrow_array::Float32Array::from(vec![Some(1.0_f32)])) as ArrayRef
        }
        DataType::Struct(figli) => {
            let dentro: Vec<ArrayRef> = figli
                .iter()
                .map(|figlio| valore_del_figlio(figlio.data_type()))
                .collect();
            Arc::new(StructArray::try_new(figli.clone(), dentro, None).unwrap()) as ArrayRef
        }
        DataType::Int64 => Arc::new(Int64Array::from(vec![Some(1_i64)])) as ArrayRef,
        _ => Arc::new(Float64Array::from(vec![Some(1.0)])) as ArrayRef,
    }
}

/// I quattro spigoli conformi, tutti `DOUBLE`.
fn spigoli_double() -> Vec<(&'static str, DataType)> {
    BBOX_SPIGOLI
        .into_iter()
        .map(|nome| (nome, DataType::Float64))
        .collect()
}

/// Un documento 1.1.0 con il `covering.bbox` costruito sui percorsi dati.
fn geo_con_covering(percorsi: [[&str; 2]; 4]) -> String {
    let spigoli: Vec<String> = BBOX_SPIGOLI
        .into_iter()
        .zip(percorsi)
        .map(|(spigolo, percorso)| {
            let (radice, foglia) = (percorso[0], percorso[1]);
            format!("\"{spigolo}\":[\"{radice}\",\"{foglia}\"]")
        })
        .collect();
    let elenco = spigoli.join(",");
    geo_con(&format!(",\"covering\":{{\"bbox\":{{{elenco}}}}}"))
}

/// Il documento minimo, con i campi che si vogliono in piu'.
fn geo_con(extra: &str) -> String {
    format!(
        r#"{{"version":"1.1.0","primary_column":"geometry","columns":{{"geometry":{{"encoding":"WKB","geometry_types":["Point"]{extra}}}}}}}"#
    )
}

#[test]
fn un_parquet_senza_geo_resta_leggibile_come_parquet_semplice() {
    // `geo` assente non e' un file rotto: e' un Parquet che non pretende di
    // essere GeoParquet, e la colonna geometria si riconosce dal nome.
    let dir = tempfile::tempdir().expect("tempdir");
    let percorso = parquet_con_geo(&dir, None);
    let dataset = GeoParquetDriver
        .open(Source::Path(percorso), opzioni_lettura())
        .expect("un Parquet semplice con una colonna `geometry` si apre");
    let contratto = &dataset.layers()[0].contract;
    assert!(contratto.schema.index_of("geometry").is_ok());
    // E non porta metadati nativi GeoParquet, perche' non ne ha dichiarati.
    let geometria = contratto.geometry.as_ref().expect("colonna geometria");
    assert!(!geometria.native_metadata.contains_key("geoparquet.version"));
}

#[test]
fn un_geo_malformato_ferma_l_apertura_invece_di_passare_per_assente() {
    // Il difetto che il lotto chiude: `serde_json::from_str(..).ok()`
    // faceva diventare `None` qualunque documento malformato, cioe' lo
    // rendeva indistinguibile da un file che `geo` non ce l'ha -- e il
    // driver passava a indovinare la colonna dal nome.
    let dir = tempfile::tempdir().expect("tempdir");
    for documento in [
        "{ non json",
        "[]",
        r#"{"primary_column":"geometry","columns":{}}"#,
        r#"{"version":"1.1.0","primary_column":"geometry"}"#,
    ] {
        let percorso = parquet_con_geo(&dir, Some(documento));
        let esito = GeoParquetDriver.open(Source::Path(percorso), opzioni_lettura());
        assert!(
            matches!(
                esito,
                Err(ref errore) if errore.code == plenora_io_model::IoErrorCode::Format
            ),
            "«{documento}» deve fermare l'apertura come metadato non conforme"
        );
    }
}

#[test]
fn un_geo_con_valore_vuoto_ferma_l_apertura() {
    // La chiave c'e' e il valore e' la stringa vuota: non e' JSON, e il
    // rifiuto viene da li'. E' il caso che un writer distratto produce.
    let dir = tempfile::tempdir().expect("tempdir");
    let percorso = parquet_con_geo(&dir, Some(""));
    // Il valore d'esito non e' `Debug`: si scarta, e resta l'errore.
    let esito = GeoParquetDriver
        .open(Source::Path(percorso), opzioni_lettura())
        .map(|_| ());
    assert!(
        matches!(
            esito,
            Err(ref errore) if errore.code == plenora_io_model::IoErrorCode::Format
        ),
        "una stringa vuota non e' un documento `geo`"
    );
}

#[test]
fn un_geo_dichiarato_senza_valore_ferma_l_apertura() {
    // Diverso dal precedente, e la copertura lo ha mostrato: nel formato
    // Parquet il valore di una `KeyValue` e' **opzionale**, quindi la
    // chiave `geo` puo' esserci senza alcun valore. La sonda di prima
    // scriveva la stringa vuota e passava dal ramo «non e' JSON»,
    // lasciando questo scoperto -- il nome prometteva una cosa e ne
    // provava un'altra.
    //
    // `KeyValue::new` non sa esprimerlo, quindi la voce si costruisce a
    // mano. E' un documento che si dichiara e non si scrive, e non un file
    // senza metadati: chi lo legge deve saperlo.
    let dir = tempfile::tempdir().expect("tempdir");
    let percorso = dir.path().join("geo_senza_valore.parquet");
    let punto: Vec<u8> = to_wkb(&Geometry::Point(Point::new(1.0, 2.0))).unwrap();
    let geom = BinaryArray::from(vec![Some(punto.as_slice())]);
    let schema = Arc::new(Schema::new(vec![Field::new(
        "geometry",
        DataType::Binary,
        true,
    )]));
    let batch = RecordBatch::try_new(schema.clone(), vec![Arc::new(geom)]).unwrap();
    let file = File::create(&percorso).unwrap();
    let mut writer = ArrowWriter::try_new(file, schema, None).unwrap();
    writer.write(&batch).unwrap();
    writer.append_key_value_metadata(KeyValue {
        key: "geo".to_owned(),
        value: None,
    });
    writer.close().unwrap();

    // Il valore d'esito non e' `Debug`: si scarta, e resta l'errore.
    let esito = GeoParquetDriver
        .open(Source::Path(percorso), opzioni_lettura())
        .map(|_| ());
    assert!(
        matches!(
            esito,
            Err(ref errore) if errore.code == plenora_io_model::IoErrorCode::Format
        ),
        "una chiave `geo` senza valore non e' un file senza metadati"
    );
}

#[test]
fn una_versione_non_supportata_ferma_l_apertura_con_il_proprio_codice() {
    // E il codice e' quello della funzionalita' non supportata, non quello
    // dei metadati non conformi: il file va bene, noi no.
    let dir = tempfile::tempdir().expect("tempdir");
    let documento = geo_con("").replace("1.1.0", "2.0.0");
    let percorso = parquet_con_geo(&dir, Some(&documento));
    // Il valore d'esito non e' `Debug`: si scarta, e resta l'errore.
    let esito = GeoParquetDriver
        .open(Source::Path(percorso), opzioni_lettura())
        .map(|_| ());
    assert!(
        matches!(
            esito,
            Err(ref errore) if errore.code == plenora_io_model::IoErrorCode::Unsupported
        ),
        "una 2.0.0 e' una versione che non leggiamo, non un file sbagliato"
    );
}

#[test]
fn una_primary_column_assente_dallo_schema_ferma_l_apertura() {
    // Prima nessuno lo verificava: il nome arrivava fino al retag dello
    // schema, dove non trovava niente da ri-etichettare, e la geometria
    // spariva senza un errore.
    let dir = tempfile::tempdir().expect("tempdir");
    let documento = geo_con("").replace("\"geometry\"", "\"non_c_e\"");
    let percorso = parquet_con_geo(&dir, Some(&documento));
    // Il valore d'esito non e' `Debug`: si scarta, e resta l'errore.
    let esito = GeoParquetDriver
        .open(Source::Path(percorso), opzioni_lettura())
        .map(|_| ());
    assert!(
        matches!(
            esito,
            Err(ref errore) if errore.code == plenora_io_model::IoErrorCode::Format
        ),
        "una colonna dichiarata e assente deve fermare l'apertura"
    );
}

#[test]
fn i_campi_validati_arrivano_al_contratto() {
    // Validare un campo e poi scartarlo sarebbe meta' del lavoro: chi legge
    // a valle non saprebbe che quel file dichiara bordi, orientamento,
    // epoca e riquadro di ingombro.
    let dir = tempfile::tempdir().expect("tempdir");
    let documento = geo_con(
        r#","edges":"planar","orientation":"counterclockwise","bbox":[0,0,1,1],"epoch":2021.5"#,
    );
    let percorso = parquet_con_geo(&dir, Some(&documento));
    let dataset = GeoParquetDriver
        .open(Source::Path(percorso), opzioni_lettura())
        .expect("il documento e' conforme");
    let geometria = dataset.layers()[0]
        .contract
        .geometry
        .as_ref()
        .expect("colonna geometria");
    let nativi = &geometria.native_metadata;
    assert_eq!(
        nativi.get("geoparquet.version").map(String::as_str),
        Some("1.1.0")
    );
    assert_eq!(
        nativi.get("geoparquet.edges").map(String::as_str),
        Some("planar")
    );
    assert_eq!(
        nativi.get("geoparquet.orientation").map(String::as_str),
        Some("counterclockwise")
    );
    assert_eq!(
        nativi.get("geoparquet.bbox").map(String::as_str),
        Some("0,0,1,1")
    );
    assert_eq!(
        nativi.get("geoparquet.epoch").map(String::as_str),
        Some("2021.5")
    );
}

#[test]
fn le_colonne_geometriche_secondarie_sono_nominate_dal_contratto() {
    // Un GeoParquet puo' averne piu' di una, e il contratto non nominava
    // quelle che non erano la primaria: un consumatore non aveva modo di
    // sapere che esistessero.
    let dir = tempfile::tempdir().expect("tempdir");
    let documento = concat!(
        r#"{"version":"1.1.0","primary_column":"geometry","columns":{"#,
        r#""geometry":{"encoding":"WKB","geometry_types":["Point"]},"#,
        r#""altra":{"encoding":"WKB","geometry_types":[]}}}"#,
    );
    // La colonna `altra` esiste **nel file**, ed e' binaria: prima questa
    // sonda la dichiarava e basta, e passava su un file che non ce l'aveva.
    let percorso = parquet_con_due_binarie(&dir, documento);
    let dataset = GeoParquetDriver
        .open(Source::Path(percorso), opzioni_lettura())
        .expect("il documento e' conforme");
    let geometria = dataset.layers()[0]
        .contract
        .geometry
        .as_ref()
        .expect("colonna geometria");
    assert_eq!(
        geometria
            .native_metadata
            .get("geoparquet.altre_colonne")
            .map(String::as_str),
        Some("altra")
    );
}

// --- il metadato confrontato col file ---------------------------------

/// Il documento con una colonna secondaria dichiarata sul nome dato.
fn geo_con_secondaria(nome: &str) -> String {
    format!(
        "{}\"{nome}\":{}",
        concat!(
            r#"{"version":"1.1.0","primary_column":"geometry","columns":{"#,
            r#""geometry":{"encoding":"WKB","geometry_types":["Point"]},"#,
        ),
        r#"{"encoding":"WKB","geometry_types":[]}}}"#,
    )
}

#[test]
fn una_secondaria_dichiarata_e_assente_ferma_l_apertura() {
    // Il metadato e' un'affermazione sul file, e nessuno la confrontava col
    // file. Una colonna geometrica inesistente veniva pubblicata nel
    // contratto come se ci fosse, e chi andava a cercarla non la trovava.
    let dir = tempfile::tempdir().expect("tempdir");
    let percorso = parquet_con_due_binarie(&dir, &geo_con_secondaria("inventata"));
    // Il valore d'esito non e' `Debug`: si scarta, e resta l'errore.
    let esito = GeoParquetDriver
        .open(Source::Path(percorso), opzioni_lettura())
        .map(|_| ());
    assert!(
        matches!(
            esito,
            Err(ref errore) if errore.code == plenora_io_model::IoErrorCode::Format
        ),
        "una secondaria inesistente deve fermare l'apertura: {esito:?}"
    );
}

#[test]
fn una_geometrica_dichiarata_su_una_colonna_non_binaria_ferma_l_apertura() {
    // `id` esiste, ed e' un intero. Il `WKB` sta nei byte: dichiararlo qui
    // e' un'incoerenza fra il metadato e i dati, non una variante.
    let dir = tempfile::tempdir().expect("tempdir");
    let percorso = parquet_con_due_binarie(&dir, &geo_con_secondaria("id"));
    // Il valore d'esito non e' `Debug`: si scarta, e resta l'errore.
    let esito = GeoParquetDriver
        .open(Source::Path(percorso), opzioni_lettura())
        .map(|_| ());
    assert!(
        matches!(
            esito,
            Err(ref errore) if errore.code == plenora_io_model::IoErrorCode::Format
        ),
        "una geometria dichiarata su una colonna di interi deve fermare l'apertura: {esito:?}"
    );
}

#[test]
fn un_covering_ben_formato_si_apre_e_toglie_la_sua_radice() {
    // Il verso positivo: quattro percorsi che esistono, in una struct sola,
    // con foglie `DOUBLE`. La colonna del covering non e' un dato utente e
    // sparisce dallo schema esposto; la geometria resta.
    let dir = tempfile::tempdir().expect("tempdir");
    let documento = geo_con_covering([
        ["bbox", "xmin"],
        ["bbox", "ymin"],
        ["bbox", "xmax"],
        ["bbox", "ymax"],
    ]);
    let percorso = parquet_con_covering(&dir, &documento, &spigoli_double(), true, true);
    let dataset = GeoParquetDriver
        .open(Source::Path(percorso), opzioni_lettura())
        .expect("un covering che corrisponde al file si apre");
    let contratto = &dataset.layers()[0].contract;
    assert!(contratto.schema.index_of("geometry").is_ok());
    assert!(
        contratto.schema.index_of("bbox").is_err(),
        "la radice del covering non e' una colonna utente"
    );
}

#[test]
fn un_covering_che_nomina_percorsi_assenti_ferma_l_apertura() {
    // E li ferma **prima** del retag. Era questo l'ordine sbagliato: le
    // radici del covering venivano tolte dallo schema esposto sulla fiducia,
    // e solo dopo qualcuno si chiedeva se esistessero. Un file che dichiara
    // un covering che non ha usciva con delle colonne in meno.
    let dir = tempfile::tempdir().expect("tempdir");
    let documento = geo_con_covering([
        ["riquadro", "xmin"],
        ["riquadro", "ymin"],
        ["riquadro", "xmax"],
        ["riquadro", "ymax"],
    ]);
    let percorso = parquet_con_covering(&dir, &documento, &spigoli_double(), true, true);
    // Il valore d'esito non e' `Debug`: si scarta, e resta l'errore.
    let esito = GeoParquetDriver
        .open(Source::Path(percorso), opzioni_lettura())
        .map(|_| ());
    assert!(
        matches!(
            esito,
            Err(ref errore) if errore.code == plenora_io_model::IoErrorCode::Format
        ),
        "un covering che nomina percorsi assenti deve fermare l'apertura: {esito:?}"
    );
}

#[test]
fn un_covering_sparso_su_strutture_diverse_ferma_l_apertura() {
    // La specifica vuole una bounding group sola. Due spigoli in una struct
    // e due in un'altra non sono un riquadro: sono quattro numeri.
    let dir = tempfile::tempdir().expect("tempdir");
    let documento = geo_con_covering([
        ["bbox", "xmin"],
        ["bbox", "ymin"],
        ["altrove", "xmax"],
        ["altrove", "ymax"],
    ]);
    let percorso = parquet_con_covering(&dir, &documento, &spigoli_double(), true, true);
    // Il valore d'esito non e' `Debug`: si scarta, e resta l'errore.
    let esito = GeoParquetDriver
        .open(Source::Path(percorso), opzioni_lettura())
        .map(|_| ());
    assert!(
        matches!(
            esito,
            Err(ref errore) if errore.code == plenora_io_model::IoErrorCode::Format
        ),
        "un covering sparso su piu' colonne deve fermare l'apertura: {esito:?}"
    );
}

#[test]
fn un_covering_con_i_figli_in_ordine_diverso_ferma_l_apertura() {
    // Cercare le foglie per percorso non dice niente sul loro **ordine**:
    // una struct `ymin, xmin, xmax, ymax` le contiene tutte e quattro, con i
    // nomi giusti, e ognuna si trova al proprio percorso. La specifica
    // pretende l'ordine perche' chi legge il riquadro senza rileggerne i
    // nomi -- e sono in molti a farlo -- scambierebbe le due ascisse con le
    // due ordinate.
    let dir = tempfile::tempdir().expect("tempdir");
    let documento = geo_con_covering([
        [BBOX_STRUCT, "xmin"],
        [BBOX_STRUCT, "ymin"],
        [BBOX_STRUCT, "xmax"],
        [BBOX_STRUCT, "ymax"],
    ]);
    let scambiati = [
        ("ymin", DataType::Float64),
        ("xmin", DataType::Float64),
        ("xmax", DataType::Float64),
        ("ymax", DataType::Float64),
    ];
    let percorso = parquet_con_covering(&dir, &documento, &scambiati, true, true);
    let esito = GeoParquetDriver
        .open(Source::Path(percorso), opzioni_lettura())
        .map(|_| ());
    assert!(
        matches!(
            esito,
            Err(ref errore) if errore.code == plenora_io_model::IoErrorCode::Format
        ),
        "i figli fuori ordine devono fermare l'apertura: {esito:?}"
    );
}

#[test]
fn un_covering_con_la_ripetizione_opposta_ferma_l_apertura() {
    // «Un riquadro se e solo se c'e' una geometria» e' un'affermazione sulla
    // **ripetizione**, e la prima stesura la cercava dove non stava: guardava
    // che le foglie non fossero ripetute, cioe' escludeva le liste, e
    // lasciava passare una geometria opzionale con un riquadro obbligatorio.
    // Un file cosi' promette un riquadro anche dove geometria non ce n'e'.
    let dir = tempfile::tempdir().expect("tempdir");
    let documento = geo_con_covering([
        [BBOX_STRUCT, "xmin"],
        [BBOX_STRUCT, "ymin"],
        [BBOX_STRUCT, "xmax"],
        [BBOX_STRUCT, "ymax"],
    ]);
    // geometria opzionale, riquadro obbligatorio
    let percorso = parquet_con_covering(&dir, &documento, &spigoli_double(), false, true);
    let esito = GeoParquetDriver
        .open(Source::Path(percorso), opzioni_lettura())
        .map(|_| ());
    assert!(
        matches!(
            esito,
            Err(ref errore) if errore.code == plenora_io_model::IoErrorCode::Format
        ),
        "una ripetizione diversa da quella della geometria deve fermare l'apertura: {esito:?}"
    );

    // E nell'altro verso: geometria obbligatoria, riquadro opzionale.
    let dir = tempfile::tempdir().expect("tempdir");
    let percorso = parquet_con_covering(&dir, &documento, &spigoli_double(), true, false);
    let esito = GeoParquetDriver
        .open(Source::Path(percorso), opzioni_lettura())
        .map(|_| ());
    assert!(
        matches!(
            esito,
            Err(ref errore) if errore.code == plenora_io_model::IoErrorCode::Format
        ),
        "e neppure il verso opposto e' ammesso: {esito:?}"
    );
}

#[test]
fn un_covering_con_figli_in_piu_o_a_meta_ferma_l_apertura() {
    // Due forme sono ammesse e nessun'altra. Un quinto figlio non e' un
    // campo utente da ignorare: la colonna del covering non e' una colonna
    // utente, e cio' che ci sta dentro deve essere il riquadro. E `zmin`
    // senza `zmax` non e' la forma tridimensionale a meta': non e' una forma.
    let dir = tempfile::tempdir().expect("tempdir");
    let documento = geo_con_covering([
        [BBOX_STRUCT, "xmin"],
        [BBOX_STRUCT, "ymin"],
        [BBOX_STRUCT, "xmax"],
        [BBOX_STRUCT, "ymax"],
    ]);

    let in_piu = [
        ("xmin", DataType::Float64),
        ("ymin", DataType::Float64),
        ("xmax", DataType::Float64),
        ("ymax", DataType::Float64),
        ("mmax", DataType::Float64),
    ];
    let percorso = parquet_con_covering(&dir, &documento, &in_piu, true, true);
    let esito = GeoParquetDriver
        .open(Source::Path(percorso), opzioni_lettura())
        .map(|_| ());
    assert!(
        matches!(
            esito,
            Err(ref errore) if errore.code == plenora_io_model::IoErrorCode::Format
        ),
        "un figlio in piu' deve fermare l'apertura: {esito:?}"
    );

    let solo_zmin = [
        ("xmin", DataType::Float64),
        ("ymin", DataType::Float64),
        ("zmin", DataType::Float64),
        ("xmax", DataType::Float64),
        ("ymax", DataType::Float64),
    ];
    let dir = tempfile::tempdir().expect("tempdir");
    let percorso = parquet_con_covering(&dir, &documento, &solo_zmin, true, true);
    let esito = GeoParquetDriver
        .open(Source::Path(percorso), opzioni_lettura())
        .map(|_| ());
    assert!(
        matches!(
            esito,
            Err(ref errore) if errore.code == plenora_io_model::IoErrorCode::Format
        ),
        "`zmin` senza `zmax` deve fermare l'apertura: {esito:?}"
    );
}

#[test]
fn un_covering_a_sei_figli_e_valido_e_si_apre() {
    // La forma tridimensionale e' conforme, e `zmin`/`zmax` vanno **in
    // mezzo**, non in coda. Rifiutarla sarebbe rifiutare un file che la
    // specifica dichiara valido, ed e' il verso in cui un lettore fa il
    // danno peggiore: dice che il file e' rotto quando rotto non e'.
    let dir = tempfile::tempdir().expect("tempdir");
    let documento = geo_con_covering([
        [BBOX_STRUCT, "xmin"],
        [BBOX_STRUCT, "ymin"],
        [BBOX_STRUCT, "xmax"],
        [BBOX_STRUCT, "ymax"],
    ]);
    let tridimensionale = [
        ("xmin", DataType::Float64),
        ("ymin", DataType::Float64),
        ("zmin", DataType::Float64),
        ("xmax", DataType::Float64),
        ("ymax", DataType::Float64),
        ("zmax", DataType::Float64),
    ];
    let percorso = parquet_con_covering(&dir, &documento, &tridimensionale, true, true);
    let dataset = GeoParquetDriver
        .open(Source::Path(percorso), opzioni_lettura())
        .expect("la forma a sei figli e' conforme");
    let contratto = &dataset.layers()[0].contract;
    assert!(contratto.schema.index_of("geometry").is_ok());
    assert!(
        contratto.schema.index_of(BBOX_STRUCT).is_err(),
        "e resta la colonna del covering, non una colonna utente"
    );
}

#[test]
fn un_covering_float_e_valido_e_serve_al_pruning() {
    // `FLOAT` **oppure** `DOUBLE`: la specifica ammette le due precisioni.
    // Pretendere `DOUBLE` classificava come malformato un file valido.
    //
    // E il comportamento non e' «accettato e ignorato»: le statistiche di un
    // `FLOAT` si allargano a `f64` senza perdere una cifra, quindi il
    // covering serve al pruning esattamente come quello a doppia precisione.
    // La sonda lo mostra sui row group saltati, non sull'apertura.
    let dir = tempfile::tempdir().expect("tempdir");
    let percorso = dir.path().join("covering_float.parquet");
    let righe = 400_usize;
    let meta = righe / 2;

    let punto = |x: f64| to_wkb(&Geometry::Point(Point::new(x, 45.0))).unwrap();
    // Prima meta' attorno a x=0, seconda attorno a x=100: due row group con
    // estensioni disgiunte.
    let ascisse: Vec<f64> = (0..righe)
        .map(|i| if i < meta { 0.0 } else { 100.0 })
        .collect();
    let geometrie: Vec<Vec<u8>> = ascisse.iter().map(|x| punto(*x)).collect();
    let spigoli: Vec<(&str, DataType)> = BBOX_SPIGOLI
        .into_iter()
        .map(|nome| (nome, DataType::Float32))
        .collect();
    let campi: Vec<Arc<Field>> = spigoli
        .iter()
        .map(|(nome, tipo)| Arc::new(Field::new(*nome, tipo.clone(), true)))
        .collect();
    // Un riquadro degenere per riga: il punto stesso. In `f32` i valori
    // scelti sono esatti.
    #[allow(clippy::cast_possible_truncation)]
    let colonne: Vec<ArrayRef> = [0_usize, 1, 2, 3]
        .into_iter()
        .map(|spigolo| {
            let valori: Vec<Option<f32>> = ascisse
                .iter()
                .map(|x| {
                    Some(if spigolo % 2 == 0 {
                        *x as f32
                    } else {
                        45.0_f32
                    })
                })
                .collect();
            Arc::new(arrow_array::Float32Array::from(valori)) as ArrayRef
        })
        .collect();
    let bbox = StructArray::try_new(campi.clone().into(), colonne, None).unwrap();
    let schema = Arc::new(Schema::new(vec![
        Field::new("geometry", DataType::Binary, true),
        Field::new(BBOX_STRUCT, DataType::Struct(campi.into()), true),
    ]));
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(BinaryArray::from(
                geometrie
                    .iter()
                    .map(|w| Some(w.as_slice()))
                    .collect::<Vec<_>>(),
            )),
            Arc::new(bbox),
        ],
    )
    .unwrap();
    let proprieta = parquet::file::properties::WriterProperties::builder()
        .set_max_row_group_row_count(Some(meta))
        .build();
    let file = File::create(&percorso).unwrap();
    let mut writer = ArrowWriter::try_new(file, schema, Some(proprieta)).unwrap();
    writer.write(&batch).unwrap();
    writer.append_key_value_metadata(KeyValue::new(
        "geo".to_owned(),
        geo_con_covering([
            [BBOX_STRUCT, "xmin"],
            [BBOX_STRUCT, "ymin"],
            [BBOX_STRUCT, "xmax"],
            [BBOX_STRUCT, "ymax"],
        ]),
    ));
    writer.close().unwrap();

    let dataset = GeoParquetDriver
        .open(Source::Path(percorso), opzioni_lettura())
        .expect("un covering `FLOAT` e' conforme");
    let mut lettore = dataset
        .open_layer_reader(&ReadRequest {
            layer: LayerId(0),
            projected_fields: None,
            projection_mode: ProjectionMode::BestEffort,
            pruning_predicate: None,
            spatial_pruning_hint: Some(plenora_io_core::request::Bbox {
                minx: 90.0,
                miny: 40.0,
                maxx: 110.0,
                maxy: 50.0,
            }),
            scope: ReadScope::default(),
            batch_target: BatchTarget::default(),
            cancellation: CancellationToken::default(),
        })
        .expect("lettore");
    let mut lette = 0;
    while let Some(batch) = lettore.next_batch().expect("batch") {
        lette += batch.num_rows();
    }
    assert!(
        lette < righe,
        "il covering `FLOAT` deve far saltare il row group lontano, lette {lette} su {righe}"
    );
    assert!(
        lette >= meta,
        "e non deve far perdere le righe che l'hint include, lette {lette}"
    );
}

#[test]
fn un_covering_con_gli_spigoli_di_precisione_diversa_ferma_l_apertura() {
    // «`FLOAT` oppure `DOUBLE`» non vuol dire «uno per spigolo»: la
    // specifica ammette le due precisioni e vieta di mescolarle. Un riquadro
    // con `xmin` a singola precisione e gli altri tre a doppia non e' un
    // riquadro piu' preciso su tre lati: e' un riquadro i cui lati non si
    // confrontano fra loro.
    let dir = tempfile::tempdir().expect("tempdir");
    let documento = geo_con_covering([
        [BBOX_STRUCT, "xmin"],
        [BBOX_STRUCT, "ymin"],
        [BBOX_STRUCT, "xmax"],
        [BBOX_STRUCT, "ymax"],
    ]);
    let mescolati = [
        ("xmin", DataType::Float32),
        ("ymin", DataType::Float64),
        ("xmax", DataType::Float64),
        ("ymax", DataType::Float64),
    ];
    let percorso = parquet_con_covering(&dir, &documento, &mescolati, true, true);
    let esito = GeoParquetDriver
        .open(Source::Path(percorso), opzioni_lettura())
        .map(|_| ());
    assert!(
        matches!(
            esito,
            Err(ref errore) if errore.code == plenora_io_model::IoErrorCode::Format
        ),
        "spigoli di precisione diversa devono fermare l'apertura: {esito:?}"
    );
}

#[test]
fn un_covering_con_uno_spigolo_annidato_ferma_l_apertura() {
    // Nomi giusti, ordine giusto, e `xmin` non e' un numero ma un gruppo.
    // Il controllo dei nomi non se ne accorge -- il nome e' quello -- e il
    // controllo del tipo fisico non esiste per un gruppo: e' il caso in cui
    // il covering ha la forma giusta e il contenuto no.
    let dir = tempfile::tempdir().expect("tempdir");
    let documento = geo_con_covering([
        [BBOX_STRUCT, "xmin"],
        [BBOX_STRUCT, "ymin"],
        [BBOX_STRUCT, "xmax"],
        [BBOX_STRUCT, "ymax"],
    ]);
    let annidato =
        DataType::Struct(vec![Arc::new(Field::new("valore", DataType::Float64, true))].into());
    let figli = [
        ("xmin", annidato),
        ("ymin", DataType::Float64),
        ("xmax", DataType::Float64),
        ("ymax", DataType::Float64),
    ];
    let percorso = parquet_con_covering(&dir, &documento, &figli, true, true);
    let esito = GeoParquetDriver
        .open(Source::Path(percorso), opzioni_lettura())
        .map(|_| ());
    assert!(
        matches!(
            esito,
            Err(ref errore) if errore.code == plenora_io_model::IoErrorCode::Format
        ),
        "uno spigolo che non e' un valore deve fermare l'apertura: {esito:?}"
    );
}

#[test]
fn un_covering_con_le_foglie_del_tipo_sbagliato_ferma_l_apertura() {
    // Percorsi giusti, struct giusta, tipo sbagliato. Lo schema JSON non
    // puo' vedere questo: conosce i percorsi, non i tipi fisici del file.
    // Il pruning che ne nascerebbe leggerebbe statistiche di interi come se
    // fossero coordinate.
    let dir = tempfile::tempdir().expect("tempdir");
    let documento = geo_con_covering([
        ["bbox", "xmin"],
        ["bbox", "ymin"],
        ["bbox", "xmax"],
        ["bbox", "ymax"],
    ]);
    let spigoli: Vec<(&str, DataType)> = BBOX_SPIGOLI
        .into_iter()
        .map(|nome| (nome, DataType::Int64))
        .collect();
    let percorso = parquet_con_covering(&dir, &documento, &spigoli, true, true);
    // Il valore d'esito non e' `Debug`: si scarta, e resta l'errore.
    let esito = GeoParquetDriver
        .open(Source::Path(percorso), opzioni_lettura())
        .map(|_| ());
    assert!(
        matches!(
            esito,
            Err(ref errore) if errore.code == plenora_io_model::IoErrorCode::Format
        ),
        "un covering con foglie non `DOUBLE` deve fermare l'apertura: {esito:?}"
    );
}

#[test]
fn legacy_bbox_names_are_preserved_by_default() {
    // Finding #4 follow-up follow-up review 2026-08-15: senza
    // metadata `covering.bbox` e senza opt-in, le colonne
    // `_bbox_minx/miny/maxx/maxy` devono restare esposte come dati
    // utente. Prima del fix il driver le nascondeva silenziosamente,
    // perdendo dati per i consumer che le usano legittimamente.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("legacy.parquet");
    write_parquet_without_covering_metadata(&path);

    let dataset = GeoParquetDriver
        .open(Source::Path(path), opzioni_lettura())
        .unwrap();
    let contract_schema = &dataset.layers()[0].contract.schema;
    // Tutte e 4 le colonne bbox restano visibili con i nomi originali.
    for name in BBOX_COLS {
        assert!(
            contract_schema.index_of(name).is_ok(),
            "colonna {name} deve restare esposta senza opt-in"
        );
    }
    // La colonna id resta visibile e la geometria resta la prima.
    assert!(contract_schema.index_of("id").is_ok());
    assert!(contract_schema.index_of("geometry").is_ok());
}

#[test]
fn legacy_bbox_names_are_hidden_with_explicit_opt_in() {
    // Simmetrica del test precedente: chi ha davvero un file scritto
    // da un writer plenora-io che non scriveva i covering metadata puo'
    // riattivare il vecchio comportamento via format_option esplicito.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("legacy_optin.parquet");
    write_parquet_without_covering_metadata(&path);

    let mut opts = opzioni_lettura();
    opts.format_options
        .insert("bbox_legacy_by_name".to_owned(), "true".to_owned());
    let dataset = GeoParquetDriver.open(Source::Path(path), opts).unwrap();
    let contract_schema = &dataset.layers()[0].contract.schema;
    // Con opt-in le 4 colonne bbox sono nascoste (fallback legacy attivo).
    for name in BBOX_COLS {
        assert!(
            contract_schema.index_of(name).is_err(),
            "colonna {name} deve essere nascosta con bbox_legacy_by_name=true"
        );
    }
    assert!(contract_schema.index_of("id").is_ok());
    assert!(contract_schema.index_of("geometry").is_ok());
}

#[test]
fn default_crs_is_crs84_with_longitude_latitude_axis_order() {
    let crs = crs_from(None).unwrap();
    assert_eq!(crs.id.as_deref(), Some("OGC:CRS84"));
    assert_eq!(
        crs.axis_order,
        plenora_io_model::crs::AxisOrder::LongitudeLatitude
    );
}

#[test]
fn un_crs_noto_solo_per_identificatore_non_e_scrivibile() {
    // Il caso che il writer risolveva scrivendo `{"id": {...}}`, che non e'
    // un documento PROJJSON: il file si dichiarava GeoParquet e non lo era.
    //
    // Le due scorciatoie sono peggio del rifiuto. Omettere il campo
    // direbbe CRS84, cioe' un altro sistema di riferimento; scrivere `null`
    // direbbe «non lo so», mentre noi lo sappiamo -- e sarebbe una perdita
    // semantica che nessuno ha dichiarato.
    let geometry = GeometryColumnContract::wkb_xy(
        FieldId(0),
        "geometry",
        ResolvedCrs::new(Some("EPSG:3003".to_owned()), CrsKind::Projected, None),
        true,
    );
    let errore = crs_da_scrivere(Some(&geometry), None).expect_err("non e' scrivibile");
    assert_eq!(errore.code, plenora_io_model::IoErrorCode::Unsupported);
    assert!(!errore.to_string().contains("EPSG:3003"));
}

#[test]
fn un_crs_con_definizione_projjson_si_scrive_per_intero() {
    // Un PROJJSON **completo**: `type`, `name`, `datum` e
    // `coordinate_system`, che e' cio' che lo schema 0.7 pretende. La prima
    // stesura di questa sonda ne usava uno di due campi, e passava: era il
    // writer a non guardare, non il documento a essere valido.
    let projjson = projjson_crs84();
    let geometry = GeometryColumnContract::wkb_xy(
        FieldId(0),
        "geometry",
        ResolvedCrs::new(
            Some("OGC:CRS84".to_owned()),
            CrsKind::Geographic,
            Some(projjson.clone()),
        ),
        true,
    );
    assert_eq!(
        crs_da_scrivere(Some(&geometry), None).expect("scrivibile"),
        CrsDaScrivere::Documento(projjson)
    );
}

/// Un PROJJSON 0.7 valido per CRS84, usato dove ne serve uno vero.
fn projjson_crs84() -> String {
    serde_json::json!({
            "type": "GeographicCRS",
            "name": "WGS 84 (CRS84)",
            "datum": {
                "type": "GeodeticReferenceFrame",
                "name": "World Geodetic System 1984",
                "ellipsoid": {
                    "name": "WGS 84",
                    "semi_major_axis": 6_378_137,
                    "inverse_flattening": 298.257_223_563
                }
            },
            "coordinate_system": {
                "subtype": "ellipsoidal",
                "axis": [
                    {"name": "Geodetic longitude", "abbreviation": "Lon", "direction": "east", "unit": "degree"},
                    {"name": "Geodetic latitude", "abbreviation": "Lat", "direction": "north", "unit": "degree"}
                ]
            }
        })
        .to_string()
}

#[test]
fn un_projjson_incompleto_non_e_scrivibile() {
    // Il contrario esatto della sonda sopra, e il caso che quella copriva
    // per sbaglio: un oggetto che si presenta come PROJJSON e non lo e'.
    // Non e' una questione di forma: `datum` e `coordinate_system` sono cio'
    // che rende un CRS utilizzabile da chi legge, e un documento che non li
    // porta non descrive alcun sistema di riferimento.
    let geometry = GeometryColumnContract::wkb_xy(
        FieldId(0),
        "geometry",
        ResolvedCrs::new(
            Some("OGC:CRS84".to_owned()),
            CrsKind::Geographic,
            Some(r#"{"type":"GeographicCRS","name":"WGS 84"}"#.to_owned()),
        ),
        true,
    );
    let esito = crs_da_scrivere(Some(&geometry), None);
    assert!(
        matches!(
            esito,
            Err(ref errore) if errore.code == plenora_io_model::IoErrorCode::Unsupported
        ),
        "un PROJJSON incompleto non e' un PROJJSON: {esito:?}"
    );
}

#[test]
fn crs84_omette_il_campo_e_un_crs_mancante_scrive_null() {
    // Assente vuol dire CRS84 per la specifica: ometterlo e' dirlo.
    let crs84 = GeometryColumnContract::wkb_xy(
        FieldId(0),
        "geometry",
        ResolvedCrs::new(Some("OGC:CRS84".to_owned()), CrsKind::Geographic, None),
        true,
    );
    assert_eq!(
        crs_da_scrivere(Some(&crs84), None).expect("scrivibile"),
        CrsDaScrivere::Omesso
    );
    // Nessun CRS: `null` e' un'affermazione, l'omissione ne sarebbe un'altra.
    assert_eq!(
        crs_da_scrivere(None, None).expect("scrivibile"),
        CrsDaScrivere::Nullo
    );
}

#[test]
fn projjson_without_identifier_is_a_typed_unresolved_crs() {
    // Il documento e' completo perche' ora passa dalla validazione: un
    // `crs` senza identificatore e' conforme alla specifica -- PROJJSON
    // ammette un oggetto senza `id` -- e a non poterlo risolvere siamo noi.
    let geo = metadati::analizza(
            &serde_json::json!({
                "version": "1.1.0",
                "primary_column": "geometry",
                "columns": {
                    "geometry": {
                        "encoding": "WKB",
                        "geometry_types": ["Point"],
                        "crs": {
                            "type": "ProjectedCRS",
                            "name": "survey-grid-secret",
                            "base_crs": {
                                "type": "GeographicCRS",
                                "name": "WGS 84",
                                "datum": {
                                    "type": "GeodeticReferenceFrame",
                                    "name": "World Geodetic System 1984",
                                    "ellipsoid": {
                                        "name": "WGS 84",
                                        "semi_major_axis": 6_378_137,
                                        "inverse_flattening": 298.257_223_563
                                    }
                                },
                                "coordinate_system": {
                                    "subtype": "ellipsoidal",
                                    "axis": [
                                        {"name": "Geodetic longitude", "abbreviation": "Lon", "direction": "east", "unit": "degree"},
                                        {"name": "Geodetic latitude", "abbreviation": "Lat", "direction": "north", "unit": "degree"}
                                    ]
                                }
                            },
                            "conversion": {
                                "name": "unnamed",
                                "method": {"name": "Transverse Mercator"},
                                "parameters": [
                                    {"name": "Latitude of natural origin", "value": 0, "unit": "degree"}
                                ]
                            },
                            "coordinate_system": {
                                "subtype": "Cartesian",
                                "axis": [
                                    {"name": "Easting", "abbreviation": "E", "direction": "east", "unit": "metre"},
                                    {"name": "Northing", "abbreviation": "N", "direction": "north", "unit": "metre"}
                                ]
                            }
                        }
                    }
                }
            })
            .to_string(),
            false,
        )
        .expect("il documento e' conforme");
    let error = crs_from(Some(&geo)).unwrap_err();
    assert_eq!(error.code, plenora_io_model::IoErrorCode::CrsUnresolved);
    assert_eq!(error.driver.as_deref(), Some("geoparquet"));
    assert!(!error.to_string().contains("survey-grid-secret"));
}

#[test]
fn pruning_predicates_preserve_integer_precision_and_fail_open() {
    let exact = 9_007_199_254_740_993_i64;
    assert_eq!(
        parse_opaque_predicate("id = 9007199254740993"),
        Some((
            "id".to_owned(),
            PruningComparison::Equal,
            PruningScalar::Int64(exact),
        ))
    );
    assert_eq!(
        range_matches(
            NumericRange::Int64(exact, exact),
            PruningComparison::Equal,
            PruningScalar::Int64(exact),
        ),
        Some(true)
    );
    // 2^53+1 NON è rappresentabile in f64: la perdita di precisione è
    // esattamente cio' che il test verifica (domini misti → fail-open).
    #[allow(clippy::cast_precision_loss)]
    let inexact = exact as f64;
    assert_eq!(
        range_matches(
            NumericRange::Int64(exact, exact),
            PruningComparison::Equal,
            PruningScalar::Float64(inexact),
        ),
        None,
        "domini numerici diversi devono tenere il row group"
    );
    assert_eq!(
        range_matches(
            NumericRange::Float64(0.0, 1.0),
            PruningComparison::GreaterThan,
            PruningScalar::Float64(f64::NAN),
        ),
        None,
        "un literal non finito deve tenere il row group"
    );
    assert!(parse_opaque_predicate("id > NaN").is_none());
    assert!(parse_opaque_predicate("espressione arbitraria").is_none());
}

#[test]
fn round_trip_file_recordbatch_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("out.parquet");

    // RecordBatch con colonna geometria geoarrow.wkb + una colonna int.
    let wkb: Vec<u8> = to_wkb(&Geometry::Point(Point::new(12.5, 45.9))).unwrap();
    let geom = BinaryArray::from(vec![Some(wkb.as_slice()), Some(wkb.as_slice())]);
    let ids = Int64Array::from(vec![1i64, 2]);
    let schema = Arc::new(Schema::new(vec![
        Field::new("geometry", DataType::Binary, true)
            .with_metadata(geometry_field_meta("EPSG:4326")),
        Field::new("id", DataType::Int64, false),
    ]));
    let batch = RecordBatch::try_new(schema.clone(), vec![Arc::new(geom), Arc::new(ids)]).unwrap();

    // create -> write -> finish
    let driver = GeoParquetDriver;
    let plan = WritePlan {
        layers: vec![WriteLayer {
            name: "l".to_owned(),
            contract: DataContract {
                schema,
                geometry: None,
            },
        }],
    };
    let mut writer = driver
        .create(Sink::Path(path.clone()), &plan, &opzioni_scrittura())
        .unwrap();
    writer.write(&batch).unwrap();
    let published = writer.finish().unwrap();
    assert!(published.bytes > 0);

    // open -> read back
    let ds = driver.open(Source::Path(path), opzioni_lettura()).unwrap();
    assert_eq!(ds.layers().len(), 1);
    let layer = &ds.layers()[0];
    let geom_c = layer.contract.geometry.as_ref().unwrap();
    assert_eq!(geom_c.name, "geometry");
    // Il dataset e' stato scritto dichiarando `EPSG:4326` e si rilegge
    // `OGC:CRS84`: e' la **canonicalizzazione**, non una perdita.
    //
    // GeoParquet impone alle coordinate che memorizza l'ordine
    // longitudine-latitudine, quindi dentro questo formato i due sistemi
    // sono lo stesso sistema: gli stessi numeri, nello stesso ordine. Il
    // writer lo esprime **omettendo** il campo `crs`, che per la specifica
    // vuol dire esattamente CRS84.
    //
    // L'alternativa sarebbe stata scrivere `{"id": {...}}`, che non e' un
    // documento PROJJSON e avrebbe reso il file non conforme, oppure
    // rifiutare la scrittura -- e allora quasi nessun dataset sarebbe
    // scrivibile in GeoParquet.
    assert_eq!(geom_c.crs.id(), Some("OGC:CRS84"));
    assert_eq!(
        geom_c.crs.as_resolved().map(|crs| crs.axis_order),
        Some(plenora_io_model::crs::AxisOrder::LongitudeLatitude),
        "l'ordine degli assi e' quello che rende equivalenti i due sistemi"
    );

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
    let out = reader.next_batch().unwrap().unwrap();
    assert_eq!(out.num_rows(), 2);
    // La geometria è marcata geoarrow.wkb nello schema effettivo.
    let field = out.schema().field_with_name("geometry").unwrap().clone();
    assert!(plenora_io_model::geometry::is_geometry_field(&field));
    // I byte WKB sono pass-through identici.
    let col = out
        .column_by_name("geometry")
        .unwrap()
        .as_any()
        .downcast_ref::<BinaryArray>()
        .unwrap();
    assert_eq!(col.value(0), wkb.as_slice());
    assert!(reader.next_batch().unwrap().is_none());
}

#[test]
fn geoparquet_preserves_xyz_and_emits_dimensional_metadata() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("z.parquet");
    let wkb = encode_wkb(
        &WkbGeometry {
            value: WkbValue::Point(WkbCoordinate {
                x: 12.5,
                y: 45.9,
                z: Some(123.0),
                m: None,
            }),
            dimensions: CoordinateDimensions::Xyz,
            srid: None,
        },
        WkbFlavor::Iso,
    )
    .unwrap();
    let schema: SchemaRef = Arc::new(Schema::new(vec![Field::new(
        "geometry",
        DataType::Binary,
        true,
    )
    .with_metadata(geometry_field_meta("EPSG:4326"))]));
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![Arc::new(BinaryArray::from(vec![Some(wkb.as_slice())]))],
    )
    .unwrap();
    let mut geometry = GeometryColumnContract::wkb_passthrough(
        FieldId(0),
        "geometry",
        ResolvedCrs::new(Some("EPSG:4326".to_owned()), CrsKind::Geographic, None),
        true,
    );
    geometry.dimensions = CoordinateDimensions::Xyz;
    geometry.set_exact_geometry_types(vec![GeometryType::Point]);
    let plan = WritePlan {
        layers: vec![WriteLayer {
            name: "z".to_owned(),
            contract: DataContract {
                schema,
                geometry: Some(geometry),
            },
        }],
    };
    let driver = GeoParquetDriver;
    let mut writer = driver
        .create(Sink::Path(path.clone()), &plan, &opzioni_scrittura())
        .unwrap();
    writer.write(&batch).unwrap();
    writer.finish().unwrap();

    assert_eq!(wkb_bbox(&wkb), Some([12.5, 45.9, 12.5, 45.9]));
    let dataset = driver.open(Source::Path(path), opzioni_lettura()).unwrap();
    let geometry = dataset.layers()[0].contract.geometry.as_ref().unwrap();
    assert_eq!(geometry.dimensions, CoordinateDimensions::Xyz);
    assert_eq!(geometry.geometry_types, vec![GeometryType::Point]);
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
    let output = reader.next_batch().unwrap().unwrap();
    let geometry_array = output
        .column_by_name("geometry")
        .unwrap()
        .as_any()
        .downcast_ref::<BinaryArray>()
        .unwrap();
    assert_eq!(geometry_array.value(0), wkb);
}

/// Conformità alla spec `GeoParquet` 1.0.0, verificata riaprendo il file
/// GREZZO col crate `parquet` (indipendente dal nostro reader): metadato
/// `geo` file-level + colonna geometria fisicamente `BYTE_ARRAY` (WKB).
#[test]
fn geoparquet_spec_conformance() {
    use parquet::basic::Type as PhysicalType;

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("conf.parquet");

    let wkb: Vec<u8> = to_wkb(&Geometry::Point(Point::new(9.19, 45.46))).unwrap();
    let schema = Arc::new(Schema::new(vec![
        Field::new("geometry", DataType::Binary, true)
            .with_metadata(geometry_field_meta("EPSG:4326")),
        Field::new("id", DataType::Int64, false),
    ]));
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(BinaryArray::from(vec![Some(wkb.as_slice())])),
            Arc::new(Int64Array::from(vec![7i64])),
        ],
    )
    .unwrap();

    let driver = GeoParquetDriver;
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
        .create(Sink::Path(path.clone()), &plan, &opzioni_scrittura())
        .unwrap();
    w.write(&batch).unwrap();
    w.finish().unwrap();

    // Riapri il file grezzo col crate parquet.
    let builder = ParquetRecordBatchReaderBuilder::try_new(File::open(&path).unwrap()).unwrap();

    // 1) metadato file-level "geo" conforme.
    let kv = builder
        .metadata()
        .file_metadata()
        .key_value_metadata()
        .expect("key_value_metadata assenti");
    let geo_raw = kv
        .iter()
        .find(|e| e.key == "geo")
        .and_then(|e| e.value.clone())
        .expect("metadato 'geo' assente (non è GeoParquet)");
    let geo: serde_json::Value = serde_json::from_str(&geo_raw).unwrap();

    // 1.1.0: il writer emette `covering`, che esiste da 1.1, e la
    // versione dichiarata dice quale documento e' questo.
    assert_eq!(geo["version"].as_str(), Some("1.1.0"));
    // E cio' che scriviamo si rilegge dal nostro stesso validatore: un
    // writer che emette un documento che il nostro lettore rifiuterebbe
    // sarebbe un guasto che nessuna delle due meta' vedrebbe da sola.
    let riletto = metadati::analizza(&geo_raw, false).expect("il documento scritto e' conforme");
    assert_eq!(riletto.versione, "1.1.0");
    assert_eq!(riletto.nome_primaria, "geometry");
    assert_eq!(geo["primary_column"].as_str(), Some("geometry"));
    let col = &geo["columns"]["geometry"];
    assert_eq!(col["encoding"].as_str(), Some("WKB"));
    let types: Vec<&str> = col["geometry_types"]
        .as_array()
        .expect("geometry_types deve essere un array")
        .iter()
        .filter_map(|v| v.as_str())
        .collect();
    assert!(
        types.contains(&"Point"),
        "geometry_types deve contenere 'Point', era {types:?}"
    );
    // `crs` **assente**: il dataset e' `EPSG:4326`, che in questo formato e'
    // CRS84, e la specifica dice che il campo assente vuol dire CRS84. Il
    // writer emetteva `{"id": {"authority": "EPSG", "code": 4326}}`, che non
    // e' un documento PROJJSON: lo schema pretende `oneOf: [PROJJSON,
    // null]`, e quel file si dichiarava GeoParquet senza esserlo.
    assert!(
        col.get("crs").is_none(),
        "un CRS equivalente a CRS84 si esprime omettendo il campo, era {:?}",
        col.get("crs")
    );
    // E il covering e' nella forma che lo schema 1.1.0 designa: due
    // segmenti per spigolo, il secondo uguale al nome dello spigolo.
    assert_eq!(
        col["covering"]["bbox"]["xmin"],
        serde_json::json!(["bbox", "xmin"])
    );
    assert_eq!(
        col["covering"]["bbox"]["ymax"],
        serde_json::json!(["bbox", "ymax"])
    );

    // 2) la colonna geometria è fisicamente BYTE_ARRAY (WKB) nel Parquet.
    let pschema = builder.metadata().file_metadata().schema_descr();
    let geom_col = (0..pschema.num_columns())
        .map(|i| pschema.column(i))
        .find(|c| c.name() == "geometry")
        .expect("colonna 'geometry' assente nello schema Parquet");
    assert_eq!(geom_col.physical_type(), PhysicalType::BYTE_ARRAY);
}

#[test]
fn projection_pushdown_reads_only_requested() {
    use plenora_io_model::contract::FieldId;

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("proj.parquet");
    let wkb: Vec<u8> = to_wkb(&Geometry::Point(Point::new(1.0, 2.0))).unwrap();
    let schema = Arc::new(Schema::new(vec![
        Field::new("geometry", DataType::Binary, true)
            .with_metadata(geometry_field_meta("EPSG:4326")),
        Field::new("id", DataType::Int64, false),
        Field::new("name", DataType::Utf8, false),
    ]));
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(BinaryArray::from(vec![Some(wkb.as_slice())])),
            Arc::new(Int64Array::from(vec![7i64])),
            Arc::new(arrow_array::StringArray::from(vec!["x"])),
        ],
    )
    .unwrap();

    let driver = GeoParquetDriver;
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
        .create(Sink::Path(path.clone()), &plan, &opzioni_scrittura())
        .unwrap();
    w.write(&batch).unwrap();
    w.finish().unwrap();

    // Proietta SOLO la colonna "id" (indice 1) in modalità Required.
    let ds = driver.open(Source::Path(path), opzioni_lettura()).unwrap();
    let mut reader = ds
        .open_layer_reader(&ReadRequest {
            layer: LayerId(0),
            projected_fields: Some(vec![FieldId(1)]),
            projection_mode: ProjectionMode::Required,
            pruning_predicate: None,
            spatial_pruning_hint: None,
            scope: ReadScope::default(),
            batch_target: BatchTarget::default(),
            cancellation: CancellationToken::default(),
        })
        .unwrap();
    // Il contratto del reader riflette la projection (1 colonna, niente geometria).
    assert_eq!(reader.contract().contract.schema.fields().len(), 1);
    assert!(reader.contract().contract.geometry.is_none());
    let out = reader.next_batch().unwrap().unwrap();
    assert_eq!(out.num_columns(), 1);
    assert_eq!(out.schema().field(0).name(), "id");
    assert_eq!(
        out.column(0)
            .as_any()
            .downcast_ref::<Int64Array>()
            .unwrap()
            .value(0),
        7
    );
}

#[test]
fn row_group_pruning_skips_blocks() {
    use plenora_io_core::request::PruningPredicate;

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("prune.parquet");
    let wkb: Vec<u8> = to_wkb(&Geometry::Point(Point::new(1.0, 2.0))).unwrap();
    let n = 200_000usize; // > 3 row group (65536/row group)
    let schema = Arc::new(Schema::new(vec![
        Field::new("geometry", DataType::Binary, true)
            .with_metadata(geometry_field_meta("EPSG:4326")),
        Field::new("id", DataType::Int64, false),
    ]));
    // `n` è la costante 200_000: il cast a i64 è esatto.
    #[allow(clippy::cast_possible_wrap)]
    let rows = n as i64;
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(BinaryArray::from(
                (0..n).map(|_| Some(wkb.as_slice())).collect::<Vec<_>>(),
            )),
            Arc::new(Int64Array::from((0..rows).collect::<Vec<_>>())),
        ],
    )
    .unwrap();

    let driver = GeoParquetDriver;
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
        .create(Sink::Path(path.clone()), &plan, &opzioni_scrittura())
        .unwrap();
    w.write(&batch).unwrap();
    w.finish().unwrap();

    // Pruning "id > 150000": salta i row group con max(id) <= 150000.
    let ds = driver
        .open(Source::Path(path.clone()), opzioni_lettura())
        .unwrap();
    let mut reader = ds
        .open_layer_reader(&ReadRequest {
            layer: LayerId(0),
            projected_fields: None,
            projection_mode: ProjectionMode::BestEffort,
            pruning_predicate: Some(PruningPredicate::NumericComparison {
                field: FieldId(1),
                comparison: PruningComparison::GreaterThan,
                value: PruningScalar::Int64(150_000),
            }),
            spatial_pruning_hint: None,
            scope: ReadScope::default(),
            batch_target: BatchTarget::default(),
            cancellation: CancellationToken::default(),
        })
        .unwrap();
    let mut total = 0;
    while let Some(b) = reader.next_batch().unwrap() {
        total += b.num_rows();
    }
    // Over-return: legge solo i row group che POSSONO contenere id>150000
    // (meno di tutte le 200k righe, ma tutte le righe matchanti sono incluse).
    assert!(
        total < n,
        "il pruning deve saltare row group, letti {total}"
    );
    assert!(total >= n - 150_000, "under-return vietato, letti {total}");

    let legacy = driver.open(Source::Path(path), opzioni_lettura()).unwrap();
    let mut legacy_reader = legacy
        .open_layer_reader(&ReadRequest {
            layer: LayerId(0),
            projected_fields: None,
            projection_mode: ProjectionMode::BestEffort,
            pruning_predicate: Some(PruningPredicate::Opaque("id > 150000".to_owned())),
            spatial_pruning_hint: None,
            scope: ReadScope::default(),
            batch_target: BatchTarget::default(),
            cancellation: CancellationToken::default(),
        })
        .unwrap();
    let mut legacy_total = 0;
    while let Some(batch) = legacy_reader.next_batch().unwrap() {
        legacy_total += batch.num_rows();
    }
    assert_eq!(
        legacy_total, total,
        "il formato Opaque v1 deve restare compatibile"
    );
}

#[test]
fn spatial_pruning_skips_blocks() {
    use plenora_io_core::request::Bbox;

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("sp.parquet");
    let n = 200_000usize;
    // Punti con x crescente (0..200), y=45 → row group con estensione x diversa.
    // `i` < 200_000 < 2^53: la conversione a f64 è esatta.
    #[allow(clippy::cast_precision_loss)]
    let wkb: Vec<Vec<u8>> = (0..n)
        .map(|i| to_wkb(&Geometry::Point(Point::new(i as f64 * 0.001, 45.0))).unwrap())
        .collect();
    let schema = Arc::new(Schema::new(vec![
        Field::new("geometry", DataType::Binary, true)
            .with_metadata(geometry_field_meta("EPSG:4326")),
        Field::new("id", DataType::Int64, false),
    ]));
    // `n` è la costante 200_000: il cast a i64 è esatto.
    #[allow(clippy::cast_possible_wrap)]
    let rows = n as i64;
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(BinaryArray::from(
                wkb.iter().map(|w| Some(w.as_slice())).collect::<Vec<_>>(),
            )),
            Arc::new(Int64Array::from((0..rows).collect::<Vec<_>>())),
        ],
    )
    .unwrap();

    let driver = GeoParquetDriver;
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
        .create(Sink::Path(path.clone()), &plan, &opzioni_scrittura())
        .unwrap();
    w.write(&batch).unwrap();
    w.finish().unwrap();

    let ds = driver.open(Source::Path(path), opzioni_lettura()).unwrap();
    // Il bbox covering NON è esposto: il contratto ha solo geometry + id.
    assert_eq!(ds.layers()[0].contract.schema.fields().len(), 2);

    // Hint spaziale x in [190,210]: interseca solo gli ultimi row group.
    let mut reader = ds
        .open_layer_reader(&ReadRequest {
            layer: LayerId(0),
            projected_fields: None,
            projection_mode: ProjectionMode::BestEffort,
            pruning_predicate: None,
            spatial_pruning_hint: Some(Bbox {
                minx: 190.0,
                miny: 40.0,
                maxx: 210.0,
                maxy: 50.0,
            }),
            scope: ReadScope::default(),
            batch_target: BatchTarget::default(),
            cancellation: CancellationToken::default(),
        })
        .unwrap();
    let mut total = 0;
    while let Some(b) = reader.next_batch().unwrap() {
        // Il batch NON contiene le colonne bbox interne.
        assert_eq!(b.num_columns(), 2);
        total += b.num_rows();
    }
    // Pruning: legge meno di tutto ma include tutte le ~10000 righe con x in [190,200].
    assert!(
        total < n,
        "spatial pruning deve saltare row group, letti {total}"
    );
    assert!(total >= 10_000, "under-return vietato, letti {total}");
}

/// Con **entrambi** i pruning attivi il driver legge l'intersezione, non
/// l'ultimo dei due.
///
/// Fino a FZ-0.1 `apply_spatial_pruning` iterava su tutti i row group e
/// chiamava `with_row_groups`, sovrascrivendo la selezione del pruning
/// numerico: con predicato e hint insieme il numerico andava perso. Non
/// erano righe sbagliate — l'over-return e' ammesso — ma lavoro fatto per
/// niente, e nessun test lo copriva perche' i due erano verificati
/// separatamente.
///
/// Il dataset ha 200.000 righe in row group da 65.536, con `x` e `id`
/// entrambi crescenti con l'indice. I due filtri sono scelti **disgiunti**:
/// l'hint spaziale tiene i row group finali, il predicato numerico quelli
/// iniziali. L'intersezione e' vuota, quindi la lettura non produce righe;
/// con la composizione vecchia ne produrrebbe decine di migliaia.
#[test]
fn entrambi_i_pruning_si_compongono_per_intersezione() {
    use plenora_io_core::request::{Bbox, PruningComparison, PruningPredicate, PruningScalar};

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("intersezione.parquet");
    let n = 200_000usize;
    // `i` < 200_000 < 2^53: la conversione a f64 e' esatta.
    #[allow(clippy::cast_precision_loss)]
    let wkb: Vec<Vec<u8>> = (0..n)
        .map(|i| to_wkb(&Geometry::Point(Point::new(i as f64 * 0.001, 45.0))).unwrap())
        .collect();
    let schema = Arc::new(Schema::new(vec![
        Field::new("geometry", DataType::Binary, true)
            .with_metadata(geometry_field_meta("EPSG:4326")),
        Field::new("id", DataType::Int64, false),
    ]));
    // `n` e' la costante 200_000: il cast a i64 e' esatto.
    #[allow(clippy::cast_possible_wrap)]
    let rows = n as i64;
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(BinaryArray::from(
                wkb.iter().map(|w| Some(w.as_slice())).collect::<Vec<_>>(),
            )),
            Arc::new(Int64Array::from((0..rows).collect::<Vec<_>>())),
        ],
    )
    .unwrap();

    let driver = GeoParquetDriver;
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
        .create(Sink::Path(path.clone()), &plan, &opzioni_scrittura())
        .unwrap();
    w.write(&batch).unwrap();
    w.finish().unwrap();

    let ds = driver.open(Source::Path(path), opzioni_lettura()).unwrap();

    let leggi = |predicato, hint| {
        let mut reader = ds
            .open_layer_reader(&ReadRequest {
                layer: LayerId(0),
                projected_fields: None,
                projection_mode: ProjectionMode::BestEffort,
                pruning_predicate: predicato,
                spatial_pruning_hint: hint,
                scope: ReadScope::default(),
                batch_target: BatchTarget::default(),
                cancellation: CancellationToken::default(),
            })
            .unwrap();
        let mut totale = 0;
        while let Some(b) = reader.next_batch().unwrap() {
            totale += b.num_rows();
        }
        totale
    };

    // `x` in [190, 210]: tiene i row group finali.
    let hint = Bbox {
        minx: 190.0,
        miny: 40.0,
        maxx: 210.0,
        maxy: 50.0,
    };
    // `id` < 70.000: tiene i row group iniziali. `id` e' il campo 1 dello
    // schema esposto.
    let predicato = PruningPredicate::NumericComparison {
        field: FieldId(1),
        comparison: PruningComparison::LessThan,
        value: PruningScalar::Int64(70_000),
    };

    let solo_spaziale = leggi(None, Some(hint));
    let solo_numerico = leggi(Some(predicato.clone()), None);
    let entrambi = leggi(Some(predicato), Some(hint));

    assert!(
        solo_spaziale > 0 && solo_spaziale < n,
        "il solo hint spaziale deve potare qualcosa: {solo_spaziale}"
    );
    assert!(
        solo_numerico > 0 && solo_numerico < n,
        "il solo predicato deve potare qualcosa: {solo_numerico}"
    );
    // I due insiemi sono disgiunti per costruzione: l'intersezione e'
    // vuota. Con la composizione vecchia si leggerebbe `solo_spaziale`.
    assert_eq!(
        entrambi, 0,
        "con entrambi i filtri si legge l'intersezione, non l'ultimo dei due"
    );
}

#[test]
fn zstd_round_trip() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("z.parquet");
    let wkb: Vec<u8> = to_wkb(&Geometry::Point(Point::new(12.5, 45.9))).unwrap();
    let schema = Arc::new(Schema::new(vec![
        Field::new("geometry", DataType::Binary, true)
            .with_metadata(geometry_field_meta("EPSG:4326")),
        Field::new("id", DataType::Int64, false),
    ]));
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(BinaryArray::from(vec![Some(wkb.as_slice())])),
            Arc::new(Int64Array::from(vec![42i64])),
        ],
    )
    .unwrap();

    let driver = GeoParquetDriver;
    let plan = WritePlan {
        layers: vec![WriteLayer {
            name: "l".to_owned(),
            contract: DataContract {
                schema,
                geometry: None,
            },
        }],
    };
    // Scrive compresso zstd.
    let wopts = opzioni_scrittura().with_format_option("compression", "zstd");
    let mut w = driver
        .create(Sink::Path(path.clone()), &plan, &wopts)
        .unwrap();
    w.write(&batch).unwrap();
    w.finish().unwrap();

    // Rilegge il file zstd (prima veniva RIFIUTATO).
    let ds = driver.open(Source::Path(path), opzioni_lettura()).unwrap();
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
    let out = reader.next_batch().unwrap().unwrap();
    assert_eq!(out.num_rows(), 1);
    let col = out
        .column_by_name("geometry")
        .unwrap()
        .as_any()
        .downcast_ref::<BinaryArray>()
        .unwrap();
    assert_eq!(col.value(0), wkb.as_slice());
}
