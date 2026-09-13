//! Le prove di [`super`], in un file loro.
//!
//! Restano un **modulo figlio**: vedono i privati del genitore
//! esattamente come quando stavano dentro di lui, e non allargano di
//! una riga la superficie pubblica del crate.

use super::*;

/// `classe_sqlite` traduce ogni errore che sappiamo costruire.
///
/// Ventisette percorsi passano da `sql_err`, e la classe e' cio' che
/// distingue le loro cause dopo che il testo di `rusqlite` ha smesso di
/// uscire. La misura di copertura differenziale del checkpoint su
/// `effc4ab` ha trovato questo `match` **mai attraversato**: ventidue righe
/// cambiate e mai eseguite.
///
/// Non tutte le varianti di `rusqlite::Error` sono costruibili da fuori —
/// alcune portano tipi opachi del motore. Quelle che lo sono vengono
/// provate tutte; il ramo di riserva copre il resto per costruzione.
#[test]
fn la_classe_sqlite_traduce_ogni_variante_costruibile() {
    use rusqlite::Error as E;

    let campioni: Vec<(E, &str)> = vec![
        (E::SqliteSingleThreadedMode, "modalita' single-threaded"),
        (E::ExecuteReturnedResults, "execute ha restituito righe"),
        (E::QueryReturnedNoRows, "nessuna riga"),
        (E::InvalidColumnIndex(3), "indice di colonna non valido"),
        (
            E::InvalidColumnName("mancante".to_owned()),
            "nome di colonna non valido",
        ),
        (
            E::StatementChangedRows(2),
            "numero di righe modificate inatteso",
        ),
        (E::InvalidQuery, "query non valida"),
        (E::MultipleStatement, "piu' istruzioni in una sola"),
        (
            E::InvalidParameterCount(1, 2),
            "numero di parametri non valido",
        ),
        (
            E::InvalidParameterName(":assente".to_owned()),
            "nome di parametro non valido",
        ),
    ];

    let mut visti = std::collections::BTreeSet::new();
    for (errore, atteso) in &campioni {
        assert_eq!(classe_sqlite(errore), *atteso);
        visti.insert(*atteso);
    }
    assert_eq!(
        visti.len(),
        campioni.len(),
        "due varianti distinte non devono avere la stessa classe"
    );

    // La classe finisce nel messaggio, e il testo della dipendenza no.
    let errore = sql_err(E::QueryReturnedNoRows);
    assert_eq!(errore.driver.as_deref(), Some("gpkg"));
    assert!(errore.message.contains("nessuna riga"));
    assert!(
        !errore.message.contains("Query returned no rows"),
        "il Display di rusqlite non deve uscire: {}",
        errore.message
    );
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
use plenora_io_model::wkb::{encode_wkb, to_wkb, WkbCoordinate, WkbFlavor, WkbGeometry, WkbValue};
use plenora_io_model::CancellationToken;

/// Legge tutti i batch e restituisce il report di perdita accumulato.
fn read_all_and_loss(dataset: &dyn OpenDatasetHandle) -> LossReport {
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
    while reader.next_batch().unwrap().is_some() {}
    reader.loss_report()
}

/// Scrive un gpkg con una colonna dichiarata INTEGER, poi vi inserisce
/// valori REAL con SQL diretto. `SQLite` ha affinita', non vincoli: un REAL
/// non convertibile senza perdita resta REAL anche in colonna INTEGER, ed
/// e' esattamente il caso che un file legittimo puo' produrre.
#[test]
fn real_values_in_an_integer_column_are_reported_as_loss_not_silently_coerced() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("coercion.gpkg");
    let schema: SchemaRef = Arc::new(Schema::new(vec![
        geometry_field("geom", "EPSG:4326"),
        Field::new("id", DataType::Int64, false),
    ]));
    let geometry = to_wkb(&geo_types::Geometry::Point(geo_types::Point::new(0.0, 0.0))).unwrap();
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(BinaryArray::from(vec![Some(geometry.as_slice())])),
            Arc::new(Int64Array::from(vec![1])),
        ],
    )
    .unwrap();
    let plan = WritePlan {
        layers: vec![WriteLayer {
            name: "features".to_owned(),
            contract: DataContract {
                schema,
                geometry: None,
            },
        }],
    };
    let driver = GpkgDriver;
    let mut writer = driver
        .create(Sink::Path(path.clone()), &plan, &opzioni_scrittura())
        .unwrap();
    writer.write(&batch).unwrap();
    writer.finish().unwrap();

    let conn = Connection::open(&path).unwrap();
    // 7.0 viene convertito a INTEGER dall'affinita' (conversione lossless):
    // nessuna perdita. 1.5 e 1e300 restano REAL.
    conn.execute_batch(
        "UPDATE features SET id = 1.5 WHERE fid = 1;
             INSERT INTO features (fid, geom, id) SELECT 2, geom, 1e300 FROM features WHERE fid = 1;
             INSERT INTO features (fid, geom, id) SELECT 3, geom, 7.0 FROM features WHERE fid = 1;",
    )
    .unwrap();
    // Precondizione del test: l'affinita' si e' comportata come previsto.
    let kinds: Vec<String> = conn
        .prepare("SELECT typeof(id) FROM features ORDER BY fid")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .map(std::result::Result::unwrap)
        .collect();
    assert_eq!(kinds, vec!["real", "real", "integer"]);
    drop(conn);

    let dataset = driver.open(Source::Path(path), opzioni_lettura()).unwrap();
    let loss = read_all_and_loss(dataset.as_ref());

    assert_eq!(loss.counts.get(INTEGER_COLUMN_REAL_TRUNCATED), Some(&1));
    assert_eq!(loss.counts.get(INTEGER_COLUMN_REAL_SATURATED), Some(&1));
    // Il 7.0 convertito dall'affinita' non e' una perdita.
    assert_eq!(loss.counts.values().sum::<u64>(), 2);
    // Un esempio per coppia (campo, categoria), mai uno per riga.
    assert_eq!(loss.esempi_trattenuti(), 2);
    // Il nome della colonna non c'e' piu': al suo posto l'indice nello
    // schema, e i due esempi restano distinti per quello.
    assert!(loss.esempi_canonici().all(|example| {
        !example.context.contains("field=") && example.posizione.field_index.is_some()
    }));
}

/// Le due guardie difensive non sono raggiungibili da un file: `SQLite`
/// memorizza NaN come NULL e l'affinita' REAL converte gli interi in
/// virgola mobile prima che il driver li veda. Restano perche' rendono
/// esplicita un'invariante che oggi dipende dalle regole di affinita', e
/// si verificano al livello in cui sono scritte.
#[test]
fn non_finite_and_wide_integers_are_declared_never_silently_substituted() {
    let mut integer_column = ColBuilder::new(&DataType::Int64);
    assert_eq!(
        integer_column.append(ValueRef::Real(f64::NAN)),
        Some(Coercion::NonFiniteDiscarded)
    );
    assert_eq!(
        integer_column.append(ValueRef::Real(f64::INFINITY)),
        Some(Coercion::NonFiniteDiscarded)
    );
    // Un NaN non diventa mai lo zero plausibile e falso di `as i64`.
    let values = integer_column.finish();
    assert_eq!(values.null_count(), 2);

    let mut real_column = ColBuilder::new(&DataType::Float64);
    assert_eq!(
        real_column.append(ValueRef::Integer(9_007_199_254_740_993)),
        Some(Coercion::IntegerPrecisionUnverifiable)
    );
    assert_eq!(real_column.append(ValueRef::Integer(7)), None);
}

/// I valori gia' rappresentabili non producono perdita: il gate non deve
/// diventare rumoroso su file corretti.
#[test]
fn representable_values_produce_no_loss() {
    let mut integer_column = ColBuilder::new(&DataType::Int64);
    assert_eq!(integer_column.append(ValueRef::Integer(42)), None);
    // REAL senza parte frazionaria e dentro l'intervallo: esatto.
    assert_eq!(integer_column.append(ValueRef::Real(7.0)), None);
    assert_eq!(integer_column.append(ValueRef::Real(-7.0)), None);
    // Il limite negativo e' rappresentabile, quello positivo no.
    assert_eq!(
        integer_column.append(ValueRef::Real(-SATURATION_BOUND)),
        None
    );
    assert_eq!(
        integer_column.append(ValueRef::Real(SATURATION_BOUND)),
        Some(Coercion::RealSaturated)
    );
}

#[test]
fn fuzz_entrypoint_reports_the_envelope_offset_declared_by_the_flags() {
    let payload = to_wkb(&geo_types::Geometry::Point(geo_types::Point::new(1.0, 2.0))).unwrap();

    let mut senza_envelope = gpkg_header(4326, false).to_vec();
    senza_envelope.extend_from_slice(&payload);
    assert_eq!(__fuzz_gpkg_geometry(&senza_envelope).unwrap(), 8);

    // Envelope XY dichiarato nei flag: 32 byte fra header e payload.
    let mut con_envelope = gpkg_header(4326, false).to_vec();
    con_envelope[3] |= 0x02;
    con_envelope.extend_from_slice(&[0_u8; 32]);
    con_envelope.extend_from_slice(&payload);
    assert_eq!(__fuzz_gpkg_geometry(&con_envelope).unwrap(), 40);

    // Magic assente e blob troncato restano rifiuti, non panic.
    assert!(__fuzz_gpkg_geometry(b"XX\x00\x01\x00\x00\x00\x00").is_err());
    assert!(__fuzz_gpkg_geometry(&senza_envelope[..7]).is_err());
}

/// La forma con la quota predefinita.
///
/// Le sonde della forma non variano i tetti: ripetere `WkbLimits::default()`
/// su ogni riga direbbe che li stanno provando, e a provarli sono le due
/// sonde dei budget, che partono dai loro campi.
fn forma(payload: &[u8]) -> Result<WkbShape> {
    wkb_shape(payload, &WkbLimits::default())
}

// Helper solo per test: header WKB `little-endian` per un tipo dato.
fn wkb_le_header(geometry_type: u32) -> Vec<u8> {
    let mut buffer = vec![0x01_u8];
    buffer.extend_from_slice(&geometry_type.to_le_bytes());
    buffer
}

/// `POINT (x y)` little-endian.
fn punto(x: f64, y: f64) -> Vec<u8> {
    let mut buffer = wkb_le_header(1);
    buffer.extend_from_slice(&x.to_le_bytes());
    buffer.extend_from_slice(&y.to_le_bytes());
    buffer
}

/// `POINT EMPTY`, cioe' `POINT (NaN NaN)`.
fn punto_vuoto() -> Vec<u8> {
    punto(f64::NAN, f64::NAN)
}

/// Una collezione del tipo dato, con i figli gia' serializzati.
fn collezione(tipo: u32, figli: &[Vec<u8>]) -> Vec<u8> {
    let mut buffer = wkb_le_header(tipo);
    let quanti = u32::try_from(figli.len()).expect("figli oltre u32 in una fixture");
    buffer.extend_from_slice(&quanti.to_le_bytes());
    for figlio in figli {
        buffer.extend_from_slice(figlio);
    }
    buffer
}

/// `POINT (x y)` **big-endian**: stesso significato, byte al contrario.
///
/// Il byte order lo sceglie chi scrive, ed e' il primo byte del payload:
/// un lettore che sapesse leggere solo una delle due forme rifiuterebbe
/// meta' dei file legittimi -- o peggio, ne leggerebbe i conteggi al
/// contrario.
fn punto_be(x: f64, y: f64) -> Vec<u8> {
    let mut buffer = vec![0x00_u8];
    buffer.extend_from_slice(&1_u32.to_be_bytes());
    buffer.extend_from_slice(&x.to_be_bytes());
    buffer.extend_from_slice(&y.to_be_bytes());
    buffer
}

/// Una collezione big-endian con i figli gia' serializzati.
fn collezione_be(tipo: u32, figli: &[Vec<u8>]) -> Vec<u8> {
    let mut buffer = vec![0x00_u8];
    buffer.extend_from_slice(&tipo.to_be_bytes());
    let quanti = u32::try_from(figli.len()).expect("figli oltre u32 in una fixture");
    buffer.extend_from_slice(&quanti.to_be_bytes());
    for figlio in figli {
        buffer.extend_from_slice(figlio);
    }
    buffer
}

/// Una `LineString` XY con i punti indicati, coordinate a zero.
fn sequenza(punti: usize) -> Vec<u8> {
    let mut buffer = wkb_le_header(2);
    let quanti = u32::try_from(punti).expect("punti oltre u32 in una fixture");
    buffer.extend_from_slice(&quanti.to_le_bytes());
    buffer.extend_from_slice(&vec![0_u8; punti * 16]);
    buffer
}

/// Un poligono XY con gli anelli dichiarati, ciascuno con i punti indicati.
///
/// Le coordinate sono zeri: la forma di un poligono dipende dal **numero**
/// di punti, non dal loro valore, e uno zero non e' `NaN`.
fn poligono(anelli: &[usize]) -> Vec<u8> {
    let mut buffer = wkb_le_header(3);
    let quanti = u32::try_from(anelli.len()).expect("anelli oltre u32 in una fixture");
    buffer.extend_from_slice(&quanti.to_le_bytes());
    for punti in anelli {
        let quanti = u32::try_from(*punti).expect("punti oltre u32 in una fixture");
        buffer.extend_from_slice(&quanti.to_le_bytes());
        buffer.extend_from_slice(&vec![0_u8; punti * 16]);
    }
    buffer
}

#[test]
fn wkb_shape_riconosce_point_empty_come_nan_nan() {
    // Finding #12 follow-up review 2026-08-15: POINT EMPTY viene
    // codificato con coordinate tutte NaN. Il flag "empty" del header
    // GeoPackage deve rispecchiare questa condizione, altrimenti i
    // validator conformi rifiutano il file.
    let mut payload = wkb_le_header(1); // ISO WKB Point XY
    payload.extend_from_slice(&f64::NAN.to_le_bytes());
    payload.extend_from_slice(&f64::NAN.to_le_bytes());
    assert_eq!(forma(&payload).unwrap(), WkbShape::Empty);

    // Un Point con coordinate finite deve restare non-empty.
    let mut concreto = wkb_le_header(1);
    concreto.extend_from_slice(&1.5_f64.to_le_bytes());
    concreto.extend_from_slice(&2.5_f64.to_le_bytes());
    assert_eq!(forma(&concreto).unwrap(), WkbShape::NonEmpty);
}

#[test]
fn wkb_shape_riconosce_point_empty_anche_in_xyz_e_xyzm() {
    // POINT Z EMPTY: type = 1001, 3 doubles NaN.
    let mut xyz = wkb_le_header(1001);
    for _ in 0..3 {
        xyz.extend_from_slice(&f64::NAN.to_le_bytes());
    }
    assert_eq!(forma(&xyz).unwrap(), WkbShape::Empty);

    // POINT ZM EMPTY: type = 3001, 4 doubles NaN.
    let mut xyzm = wkb_le_header(3001);
    for _ in 0..4 {
        xyzm.extend_from_slice(&f64::NAN.to_le_bytes());
    }
    assert_eq!(forma(&xyzm).unwrap(), WkbShape::Empty);

    // Point Z con Z finita e X/Y NaN NON e' empty (basta una
    // coordinata finita per contare come non-empty).
    let mut mixed = wkb_le_header(1001);
    mixed.extend_from_slice(&f64::NAN.to_le_bytes());
    mixed.extend_from_slice(&f64::NAN.to_le_bytes());
    mixed.extend_from_slice(&42.0_f64.to_le_bytes());
    assert_eq!(forma(&mixed).unwrap(), WkbShape::NonEmpty);
}

#[test]
fn wkb_shape_supporta_ewkb_con_srid() {
    // EWKB Point con SRID (flag 0x2000_0000 | tipo 1): dopo il type
    // c'e' l'SRID (4 byte) e poi le coordinate. Un POINT (1,2) SRID=4326
    // deve risultare non-empty.
    let mut payload = vec![0x01_u8];
    let type_flags: u32 = 0x2000_0000 | 1;
    payload.extend_from_slice(&type_flags.to_le_bytes());
    payload.extend_from_slice(&4326_u32.to_le_bytes()); // SRID
    payload.extend_from_slice(&1.0_f64.to_le_bytes());
    payload.extend_from_slice(&2.0_f64.to_le_bytes());
    assert_eq!(forma(&payload).unwrap(), WkbShape::NonEmpty);

    // Stesso layout con NaN,NaN e' Point EMPTY.
    let mut empty = vec![0x01_u8];
    empty.extend_from_slice(&type_flags.to_le_bytes());
    empty.extend_from_slice(&4326_u32.to_le_bytes());
    empty.extend_from_slice(&f64::NAN.to_le_bytes());
    empty.extend_from_slice(&f64::NAN.to_le_bytes());
    assert_eq!(forma(&empty).unwrap(), WkbShape::Empty);

    // MULTIPOINT EMPTY EWKB con SRID: type 4 + SRID flag, poi
    // 4 byte SRID e 4 byte count=0.
    let mut multi_empty = vec![0x01_u8];
    let multi_type: u32 = 0x2000_0000 | 4;
    multi_empty.extend_from_slice(&multi_type.to_le_bytes());
    multi_empty.extend_from_slice(&4326_u32.to_le_bytes());
    multi_empty.extend_from_slice(&0_u32.to_le_bytes());
    assert_eq!(forma(&multi_empty).unwrap(), WkbShape::Empty);
}

#[test]
fn wkb_shape_fallisce_chiuso_sui_payload_ambigui() {
    // Header troncato: 5 byte minimi non presenti.
    assert!(forma(&[0x01, 0x01]).is_err());
    // Byte-order invalido.
    assert!(forma(&[0x02, 0x00, 0x00, 0x00, 0x01]).is_err());
    // Point ISO con coordinate mancanti (solo header, niente doubles).
    assert!(forma(&wkb_le_header(1)).is_err());
    // LineString ISO senza il conteggio.
    assert!(forma(&wkb_le_header(2)).is_err());
    // Flavor ISO invalido (type = 4123 → flavor 4 sconosciuto).
    assert!(forma(&wkb_le_header(4123)).is_err());
    // EWKB con SRID ma payload che finisce prima dell'SRID.
    let mut ewkb_troncato = vec![0x01_u8];
    let type_flags: u32 = 0x2000_0000 | 1;
    ewkb_troncato.extend_from_slice(&type_flags.to_le_bytes());
    // niente 4 byte SRID: il payload finisce qui
    assert!(forma(&ewkb_troncato).is_err());
}

/// Gli stessi rifiuti, un livello piu' sotto.
///
/// Prima di S11 nessuno di questi payload veniva guardato oltre il
/// conteggio: erano tutti «non vuoti» e passavano.
#[test]
fn wkb_shape_fallisce_chiuso_sui_figli_malformati() {
    // Un figlio troncato a meta' delle coordinate.
    let intero = collezione(7, &[punto(1.0, 2.0)]);
    assert!(forma(&intero[..intero.len() - 1]).is_err());

    // Un figlio con byte order invalido.
    let mut cattivo = punto(1.0, 2.0);
    cattivo[0] = 0x02;
    assert!(forma(&collezione(7, &[cattivo])).is_err());

    // Un anello che dichiara piu' punti di quanti il payload ne porti.
    let mut bugiardo = wkb_le_header(3);
    bugiardo.extend_from_slice(&1_u32.to_le_bytes());
    bugiardo.extend_from_slice(&1000_u32.to_le_bytes());
    assert!(forma(&bugiardo).is_err());

    // Un conteggio di punti che, moltiplicato per la coordinata, esce dal
    // payload di parecchi ordini di grandezza: nessun overflow, un
    // rifiuto.
    let mut enorme = wkb_le_header(2);
    enorme.extend_from_slice(&u32::MAX.to_le_bytes());
    assert!(forma(&enorme).is_err());

    // Una collezione che dichiara piu' figli di quanti ne contenga: il
    // secondo manca.
    let mut incompleta = wkb_le_header(7);
    incompleta.extend_from_slice(&2_u32.to_le_bytes());
    incompleta.extend_from_slice(&punto(1.0, 2.0));
    assert!(forma(&incompleta).is_err());
}

#[test]
fn wkb_shape_rileva_le_collezioni_vuote() {
    // MULTIPOINT EMPTY (ISO): type 4, count 0.
    let mut mp = wkb_le_header(4);
    mp.extend_from_slice(&0_u32.to_le_bytes());
    assert_eq!(forma(&mp).unwrap(), WkbShape::Empty);

    // MULTIPOINT con un punto vero: non-empty.
    assert_eq!(
        forma(&collezione(4, &[punto(1.0, 2.0)])).unwrap(),
        WkbShape::NonEmpty
    );

    // Un conteggio che dichiara un figlio assente **non** e' piu' un
    // non-empty per costruzione: fino a S11 bastava `count > 0`, e questo
    // payload -- cinque byte di header e un conteggio -- veniva
    // classificato come pieno. Ora e' troncamento, e fallisce chiuso.
    let mut senza_figlio = wkb_le_header(4);
    senza_figlio.extend_from_slice(&1_u32.to_le_bytes());
    assert!(forma(&senza_figlio).is_err());
}

/// **Il difetto che il lotto S11 chiude.**
///
/// Un contenitore e' vuoto quando non contiene niente di non vuoto, e
/// fino a `f7b6d79` la classificazione si fermava al conteggio di primo
/// livello: ognuno di questi payload dichiara almeno un figlio, e ognuno
/// e' semanticamente vuoto.
#[test]
fn wkb_shape_scende_nei_figli_delle_collection() {
    let casi: Vec<(&str, Vec<u8>, WkbShape)> = vec![
        (
            "GEOMETRYCOLLECTION (POINT EMPTY)",
            collezione(7, &[punto_vuoto()]),
            WkbShape::Empty,
        ),
        (
            "MULTIPOINT (EMPTY)",
            collezione(4, &[punto_vuoto()]),
            WkbShape::Empty,
        ),
        (
            "MULTIPOLYGON (POLYGON EMPTY)",
            collezione(6, &[poligono(&[])]),
            WkbShape::Empty,
        ),
        (
            "MULTIPOLYGON (POLYGON con un anello senza punti)",
            collezione(6, &[poligono(&[0])]),
            WkbShape::Empty,
        ),
        (
            "GEOMETRYCOLLECTION (CIRCULARSTRING EMPTY)",
            collezione(7, &[collezione(8, &[])]),
            WkbShape::Empty,
        ),
        // Le due asimmetriche provano che i fratelli vengono davvero
        // raggiunti: per leggere il secondo figlio bisogna aver misurato
        // il primo, e un offset sbagliato darebbe l'altra risposta.
        (
            "GEOMETRYCOLLECTION (POINT EMPTY, POINT (1 2))",
            collezione(7, &[punto_vuoto(), punto(1.0, 2.0)]),
            WkbShape::NonEmpty,
        ),
        (
            "GEOMETRYCOLLECTION (POINT (1 2), POINT EMPTY)",
            collezione(7, &[punto(1.0, 2.0), punto_vuoto()]),
            WkbShape::NonEmpty,
        ),
        (
            "GEOMETRYCOLLECTION (POINT EMPTY, POINT EMPTY)",
            collezione(7, &[punto_vuoto(), punto_vuoto()]),
            WkbShape::Empty,
        ),
        (
            "MULTIPOLYGON (POLYGON EMPTY, POLYGON con un anello)",
            collezione(6, &[poligono(&[]), poligono(&[4])]),
            WkbShape::NonEmpty,
        ),
    ];
    for (nome, payload, atteso) in casi {
        assert_eq!(forma(&payload).unwrap(), atteso, "{nome}");
    }
}

/// L'annidamento non e' un caso limite: e' la forma in cui il vuoto si
/// nasconde meglio.
#[test]
fn wkb_shape_scende_negli_annidamenti() {
    let profondo = collezione(7, &[collezione(7, &[collezione(7, &[punto_vuoto()])])]);
    assert_eq!(forma(&profondo).unwrap(), WkbShape::Empty);

    let con_contenuto = collezione(7, &[collezione(7, &[punto(1.0, 2.0)])]);
    assert_eq!(forma(&con_contenuto).unwrap(), WkbShape::NonEmpty);
}

/// Scendere significa ricorrere, e ricorrere su input ostile senza tetto
/// significa esaurire lo stack: nove byte per livello bastano.
#[test]
fn wkb_shape_fallisce_chiuso_quando_il_budget_di_profondita_finisce() {
    let limiti = WkbLimits::default();
    let mut annidato = punto_vuoto();
    for _ in 0..=limiti.max_depth {
        annidato = collezione(7, &[annidato]);
    }
    assert!(forma(&annidato).is_err());

    // Un livello sotto il tetto resta classificabile: il budget non e' una
    // scusa per rifiutare tutto.
    let mut ammesso = punto_vuoto();
    for _ in 0..(limiti.max_depth - 1) {
        ammesso = collezione(7, &[ammesso]);
    }
    assert_eq!(forma(&ammesso).unwrap(), WkbShape::Empty);
}

/// Il secondo budget, nell'unita' del parser condiviso: **coordinate e
/// geometrie figlie**, non geometrie visitate.
///
/// La prima stesura addebitava la radice e ogni anello, e a quota stretta i
/// due percorsi divergevano in entrambi i versi. Qui si prova a quota due,
/// invece che con un payload da centomila anelli: piu' piccolo, e prova in
/// piu' che il tetto e' quello **configurato**.
#[test]
fn wkb_shape_fallisce_chiuso_quando_il_budget_di_componenti_finisce() {
    let due = WkbLimits {
        max_components: 2,
        ..WkbLimits::default()
    };

    // Tre punti in una sequenza: tre coordinate, una in piu' del tetto.
    assert!(wkb_shape(&sequenza(3), &due).is_err());
    assert_eq!(
        wkb_shape(&sequenza(2), &due).unwrap(),
        WkbShape::NonEmpty,
        "due coordinate stanno nel tetto"
    );

    // Tre figli in una collezione: il conteggio si addebita **prima** di
    // visitarli, come in `inspect_geometry`.
    let tre_figli = collezione(4, &[punto(1.0, 2.0), punto(3.0, 4.0), punto(5.0, 6.0)]);
    assert!(wkb_shape(&tre_figli, &due).is_err());

    // Un poligono con due anelli **vuoti** non costa niente: a pagare sono
    // i punti, non gli anelli. Era il caso che divergeva dal parser
    // condiviso.
    assert_eq!(
        wkb_shape(&poligono(&[0, 0]), &due).unwrap(),
        WkbShape::Empty
    );
}

/// La contabilita' e' **la stessa** del parser condiviso, non una simile.
///
/// E' il rilievo che ha riaperto il lotto: due tetti con lo stesso nome e
/// due unita' di misura diverse. La sonda non confronta il codice, confronta
/// gli **esiti**: per ogni payload ben formato e per ogni quota da zero a
/// dodici, accettare o rifiutare deve coincidere con `inspect_wkb`.
#[test]
fn il_budget_dei_componenti_coincide_con_quello_del_parser_condiviso() {
    let payload: Vec<(&str, Vec<u8>)> = vec![
        ("POINT", punto(1.0, 2.0)),
        ("POINT EMPTY", punto_vuoto()),
        ("LINESTRING EMPTY", sequenza(0)),
        ("LINESTRING 3 punti", sequenza(3)),
        ("POLYGON EMPTY", poligono(&[])),
        ("POLYGON un anello di 4", poligono(&[4])),
        ("POLYGON due anelli vuoti", poligono(&[0, 0])),
        (
            "MULTIPOINT di 2",
            collezione(4, &[punto(1.0, 2.0), punto_vuoto()]),
        ),
        (
            "GEOMETRYCOLLECTION annidata",
            collezione(7, &[collezione(7, &[punto_vuoto()])]),
        ),
        (
            "MULTIPOLYGON di 2 poligoni",
            collezione(6, &[poligono(&[4]), poligono(&[])]),
        ),
    ];
    for (nome, byte) in payload {
        for quota in 0..=12_usize {
            let limiti = WkbLimits {
                max_components: quota,
                ..WkbLimits::default()
            };
            let nostro = wkb_shape(&byte, &limiti).is_ok();
            let condiviso = plenora_io_model::wkb::inspect_wkb(&byte, &limiti).is_ok();
            assert_eq!(
                nostro, condiviso,
                "{nome} a quota {quota}: la classificazione dice {nostro}, \
                     il parser condiviso dice {condiviso}"
            );
        }
    }
}

/// Il byte order non e' un dettaglio del punto: governa **ogni** intero
/// che la discesa legge.
///
/// La diagnostica differenziale del livello 2 su `1bd499d` ha trovato
/// queste righe mai eseguite: il parser ricorsivo era nuovo, e nessuna
/// sonda gli passava un payload big-endian. Un conteggio di figli letto
/// nell'endianess sbagliata non e' un errore che si nota: e' un numero
/// enorme o zero, cioe' un rifiuto o un «vuoto» falso.
#[test]
fn wkb_shape_scende_nei_figli_anche_in_big_endian() {
    let vuota = collezione_be(7, &[punto_be(f64::NAN, f64::NAN)]);
    assert_eq!(forma(&vuota).unwrap(), WkbShape::Empty);

    let piena = collezione_be(7, &[punto_be(f64::NAN, f64::NAN), punto_be(1.0, 2.0)]);
    assert_eq!(forma(&piena).unwrap(), WkbShape::NonEmpty);

    // Le due endianess convivono nello stesso albero: ogni figlio dichiara
    // la propria, ed e' cio' che rende sbagliato ereditarla dal padre.
    let mista = collezione(7, &[punto_be(f64::NAN, f64::NAN), punto_vuoto()]);
    assert_eq!(forma(&mista).unwrap(), WkbShape::Empty);

    assert!(forma(&collezione_be(7, &[punto_be(1.0, 2.0)])[..8]).is_err());
}

/// Il tetto per cella e' il primo dei tre, e viene dalla **configurazione**.
///
/// Provarlo con il default vorrebbe dire costruire una cella da 64 MiB;
/// provarlo con una quota stretta prova due cose in una riga -- che il
/// tetto si applica, e che e' quello passato e non quello predefinito.
#[test]
fn wkb_shape_rifiuta_una_cella_oltre_il_tetto_configurato() {
    let payload = collezione(7, &[punto(1.0, 2.0)]);
    let stretti = WkbLimits {
        max_cell_bytes: payload.len() - 1,
        ..WkbLimits::default()
    };
    assert!(wkb_shape(&payload, &stretti).is_err());

    let esatti = WkbLimits {
        max_cell_bytes: payload.len(),
        ..WkbLimits::default()
    };
    assert_eq!(wkb_shape(&payload, &esatti).unwrap(), WkbShape::NonEmpty);
}

/// Un aggregato non accetta qualunque figlio, e la regola e' **una sola**.
///
/// Un `MULTIPOINT` che contiene una `LINESTRING` non e' una geometria di cui
/// abbia senso chiedersi se sia vuota: `inspect_wkb` la rifiuta, e fino a
/// questo commit la classificazione la accettava e rispondeva. Il wrapper di
/// scrittura la intercettava prima -- quindi nessun file corrotto -- ma
/// l'invariante prometteva piu' del codice.
///
/// La tabella non e' riscritta qui: `membro_ammesso` e' la stessa che
/// `inspect_geometry` applica ai propri figli.
#[test]
fn wkb_shape_rifiuta_un_membro_di_tipo_non_ammesso() {
    let rifiutati = [
        (
            "MULTIPOINT con una LINESTRING",
            collezione(4, &[sequenza(2)]),
        ),
        (
            "MULTILINESTRING con un POINT",
            collezione(5, &[punto(1.0, 2.0)]),
        ),
        (
            "MULTIPOLYGON con un POINT",
            collezione(6, &[punto(1.0, 2.0)]),
        ),
        (
            "MULTIPOLYGON con una LINESTRING",
            collezione(6, &[sequenza(2)]),
        ),
    ];
    for (nome, payload) in rifiutati {
        assert!(forma(&payload).is_err(), "{nome}");
        assert!(
            plenora_io_model::wkb::inspect_wkb(&payload, &WkbLimits::default()).is_err(),
            "{nome}: e il parser condiviso lo rifiuta gia'"
        );
    }

    // `GEOMETRYCOLLECTION` accetta qualunque tipo, ed e' l'unico: una
    // regola che rifiutasse tutto sarebbe altrettanto sbagliata.
    let mista = collezione(7, &[punto_vuoto(), sequenza(0), poligono(&[])]);
    assert_eq!(forma(&mista).unwrap(), WkbShape::Empty);
    let mista_piena = collezione(7, &[punto_vuoto(), sequenza(2)]);
    assert_eq!(forma(&mista_piena).unwrap(), WkbShape::NonEmpty);
}

/// Il limite dichiarato: cio' che non sappiamo dimensionare non e' un
/// difetto del payload, e non viene rifiutato.
#[test]
fn wkb_shape_non_rifiuta_i_tipi_che_non_sa_dimensionare() {
    // Tipo 13 (Curve astratta): non trattato, safe-default non vuoto.
    assert_eq!(forma(&wkb_le_header(13)).unwrap(), WkbShape::NonEmpty);

    // Dentro una collezione il limite si propaga: senza la lunghezza del
    // figlio, i fratelli non sono raggiungibili, e la risposta
    // conservativa vale per l'intero contenitore.
    //
    // Il figlio porta un conteggio oltre l'header perche' nove byte sono il
    // minimo che un figlio puo' occupare: e' l'unita' con cui il parser
    // condiviso giudica plausibile un conteggio, e un header nudo di cinque
    // byte viene rifiutato prima -- correttamente, e per un'altra ragione.
    let ignoto = [wkb_le_header(13), 0_u32.to_le_bytes().to_vec()].concat();
    let con_ignoto = collezione(7, std::slice::from_ref(&ignoto));
    assert_eq!(forma(&con_ignoto).unwrap(), WkbShape::NonEmpty);

    let vuoto_poi_ignoto = collezione(7, &[punto_vuoto(), ignoto]);
    assert_eq!(forma(&vuoto_poi_ignoto).unwrap(), WkbShape::NonEmpty);
}

#[test]
fn last_change_gpkg_contents_e_not_null_con_default_valido() {
    // Finding #12 follow-up: la DDL deve dichiarare NOT NULL e un
    // default valido, non solo compilarlo nell'INSERT del driver. Un
    // consumer che scriva altri record in gpkg_contents senza
    // fornire last_change deve trovare comunque un valore ISO 8601.
    let conn = Connection::open_in_memory().unwrap();
    init_gpkg(&conn).unwrap();
    conn.execute(
        "INSERT INTO gpkg_contents (table_name, data_type, identifier, srs_id) \
             VALUES ('probe', 'features', 'probe', 4326)",
        [],
    )
    .unwrap();
    let last_change: String = conn
        .query_row(
            "SELECT last_change FROM gpkg_contents WHERE table_name = 'probe'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    // Formato dichiarato: `YYYY-MM-DDTHH:MM:SS.sssZ`.
    assert!(
        last_change.ends_with('Z') && last_change.contains('T'),
        "last_change atteso in formato ISO 8601 UTC: {last_change}"
    );
}

#[test]
fn integer_widths_are_exact_and_overflow_never_becomes_sql_null() {
    let values: ArrayRef = Arc::new(Int32Array::from(vec![7]));
    let overflow: ArrayRef = Arc::new(UInt64Array::from(vec![u64::MAX]));

    assert_eq!(
        arrow_cell_to_sql_ref(&values, 0).unwrap(),
        BorrowedSqlValue::Integer(7)
    );
    assert!(arrow_cell_to_sql_ref(&overflow, 0).is_err());
}

#[test]
fn undefined_and_dangling_srs_ids_fail_closed_with_raw_crs() {
    let conn = Connection::open_in_memory().unwrap();
    init_gpkg(&conn).unwrap();

    for srs_id in [0, -1, 999_999] {
        let error = crs_for(&conn, srs_id).unwrap_err();
        assert_eq!(error.code, plenora_io_model::IoErrorCode::CrsUnresolved);
        assert_eq!(error.driver.as_deref(), Some("gpkg"));
    }

    conn.execute("DELETE FROM gpkg_spatial_ref_sys WHERE srs_id = 4326", [])
        .unwrap();
    assert!(matches!(
        crs_for(&conn, 4326),
        Err(error) if error.code == plenora_io_model::IoErrorCode::CrsUnresolved
    ));
}

#[test]
fn writer_does_not_relabel_crs84_as_epsg_4326() {
    let conn = Connection::open_in_memory().unwrap();
    init_gpkg(&conn).unwrap();
    assert!(matches!(
        register_srs(&conn, Some("OGC:CRS84"), None),
        Err(error) if error.code == plenora_io_model::IoErrorCode::CrsUnresolved
    ));
}

#[test]
fn gpkg_epsg_axis_orders_are_explicit() {
    let conn = Connection::open_in_memory().unwrap();
    init_gpkg(&conn).unwrap();
    register_srs(&conn, Some("EPSG:3857"), None).unwrap();

    let geographic = crs_for(&conn, 4326).unwrap();
    let projected = crs_for(&conn, 3857).unwrap();
    assert_eq!(
        geographic.axis_order,
        plenora_io_model::crs::AxisOrder::LatitudeLongitude
    );
    assert_eq!(
        projected.axis_order,
        plenora_io_model::crs::AxisOrder::EastingNorthing
    );
}

fn ids_with_spatial_hint(dataset: &dyn OpenDatasetHandle, spatial_pruning_hint: Bbox) -> Vec<i64> {
    let mut reader = dataset
        .open_layer_reader(&ReadRequest {
            layer: LayerId(0),
            projected_fields: None,
            projection_mode: ProjectionMode::BestEffort,
            pruning_predicate: None,
            spatial_pruning_hint: Some(spatial_pruning_hint),
            scope: ReadScope::default(),
            batch_target: BatchTarget::default(),
            cancellation: CancellationToken::default(),
        })
        .unwrap();
    let mut ids = Vec::new();
    while let Some(batch) = reader.next_batch().unwrap() {
        ids.extend_from_slice(
            batch
                .column_by_name("id")
                .unwrap()
                .as_any()
                .downcast_ref::<Int64Array>()
                .unwrap()
                .values(),
        );
    }
    ids
}

// Il test copre in un solo scenario scrittura, registrazione dell'RTree e
// le tre letture (non registrato, registrato, hint invalido): spezzarlo
// duplicherebbe la fixture e ne perderebbe la sequenza.
#[allow(clippy::too_many_lines)]
#[test]
fn spatial_pruning_uses_only_registered_rtree_and_never_filters_exactly() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("rtree.gpkg");
    let geometries = [
        to_wkb(&geo_types::Geometry::Point(geo_types::Point::new(0.0, 0.0))).unwrap(),
        to_wkb(&geo_types::Geometry::LineString(
            geo_types::LineString::from(vec![(0.0, 0.0), (10.0, 10.0)]),
        ))
        .unwrap(),
        to_wkb(&geo_types::Geometry::Point(geo_types::Point::new(0.5, 9.5))).unwrap(),
        to_wkb(&geo_types::Geometry::Point(geo_types::Point::new(
            100.0, 100.0,
        )))
        .unwrap(),
    ];
    let schema: SchemaRef = Arc::new(Schema::new(vec![
        geometry_field("geom", "EPSG:4326"),
        Field::new("id", DataType::Int64, false),
    ]));
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(BinaryArray::from(
                geometries
                    .iter()
                    .map(|geometry| Some(geometry.as_slice()))
                    .collect::<Vec<_>>(),
            )),
            Arc::new(Int64Array::from(vec![1, 2, 3, 4])),
        ],
    )
    .unwrap();
    let plan = WritePlan {
        layers: vec![WriteLayer {
            name: "features".to_owned(),
            contract: DataContract {
                schema,
                geometry: None,
            },
        }],
    };
    let driver = GpkgDriver;
    let mut writer = driver
        .create(Sink::Path(path.clone()), &plan, &opzioni_scrittura())
        .unwrap();
    writer.write(&batch).unwrap();
    writer.finish().unwrap();

    let conn = Connection::open(&path).unwrap();
    conn.execute_batch(
        "CREATE TABLE gpkg_extensions (
                 table_name TEXT,
                 column_name TEXT,
                 extension_name TEXT NOT NULL,
                 definition TEXT NOT NULL,
                 scope TEXT NOT NULL
             );
             CREATE VIRTUAL TABLE rtree_features_geom
                 USING rtree(id, minx, maxx, miny, maxy);
             INSERT INTO rtree_features_geom VALUES (1, 0, 0, 0, 0);
             INSERT INTO rtree_features_geom VALUES (2, 0, 10, 0, 10);
             INSERT INTO rtree_features_geom VALUES (3, 0.5, 0.5, 9.5, 9.5);
             INSERT INTO rtree_features_geom VALUES (4, 100, 100, 100, 100);",
    )
    .unwrap();
    drop(conn);

    let hint = Bbox {
        minx: 0.0,
        miny: 9.0,
        maxx: 1.0,
        maxy: 10.0,
    };
    let unregistered = driver
        .open(Source::Path(path.clone()), opzioni_lettura())
        .unwrap();
    assert_eq!(
        unregistered.layers()[0]
            .contract
            .geometry
            .as_ref()
            .unwrap()
            .native_metadata["gpkg.rtree_index"],
        "false"
    );
    assert_eq!(
        ids_with_spatial_hint(unregistered.as_ref(), hint),
        vec![1, 2, 3, 4],
        "una tabella RTree non registrata deve essere ignorata"
    );
    drop(unregistered);

    let conn = Connection::open(&path).unwrap();
    conn.execute(
        "INSERT INTO gpkg_extensions
             VALUES (?1, ?2, 'gpkg_rtree_index',
                     'http://www.geopackage.org/spec/#extension_rtree', 'write-only')",
        rusqlite::params!["features", "geom"],
    )
    .unwrap();
    drop(conn);

    let indexed = driver.open(Source::Path(path), opzioni_lettura()).unwrap();
    assert_eq!(
        indexed.layers()[0]
            .contract
            .geometry
            .as_ref()
            .unwrap()
            .native_metadata["gpkg.rtree_index"],
        "true"
    );
    assert_eq!(
        ids_with_spatial_hint(indexed.as_ref(), hint),
        vec![2, 3],
        "il vero positivo deve restare e il falso positivo bbox è ammesso"
    );
    assert_eq!(
        ids_with_spatial_hint(
            indexed.as_ref(),
            Bbox {
                minx: 1.0,
                miny: 1.0,
                maxx: -1.0,
                maxy: -1.0,
            },
        ),
        vec![1, 2, 3, 4],
        "un hint invalido deve essere ignorato"
    );
}

/// Una colonna che oscura l'alias della chiave di riga rende la
/// paginazione non deterministica, e il driver deve rifiutare il file.
///
/// Non serve un file corrotto: e' un `GeoPackage` sintatticamente valido,
/// scritto qui dal driver stesso e poi esteso con `ALTER TABLE`. Prima di
/// questo controllo il driver paginava sulla colonna utente senza dirlo,
/// con righe saltate o ripetute a seconda dei valori.
#[test]
fn una_colonna_che_oscura_il_rowid_viene_rifiutata() {
    for nome in ["rowid", "_rowid_", "OID"] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("shadow.gpkg");
        let schema: SchemaRef = Arc::new(Schema::new(vec![
            geometry_field("geom", "EPSG:4326"),
            Field::new("id", DataType::Int64, false),
        ]));
        let geometry =
            to_wkb(&geo_types::Geometry::Point(geo_types::Point::new(0.0, 0.0))).unwrap();
        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![
                Arc::new(BinaryArray::from(vec![Some(geometry.as_slice())])),
                Arc::new(Int64Array::from(vec![1])),
            ],
        )
        .unwrap();
        let plan = WritePlan {
            layers: vec![WriteLayer {
                name: "features".to_owned(),
                contract: DataContract {
                    schema,
                    geometry: None,
                },
            }],
        };
        let driver = GpkgDriver;
        let mut writer = driver
            .create(Sink::Path(path.clone()), &plan, &opzioni_scrittura())
            .unwrap();
        writer.write(&batch).unwrap();
        writer.finish().unwrap();

        // Il file resta valido: aggiungiamo solo una colonna con un nome
        // che SQLite risolve come alias della chiave di riga.
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(&format!(
            "ALTER TABLE features ADD COLUMN \"{nome}\" INTEGER;"
        ))
        .unwrap();
        drop(conn);

        // `expect_err` richiederebbe `Debug` sul trait object restituito
        // in caso di successo, che non lo implementa.
        match driver.open(Source::Path(path), opzioni_lettura()) {
            Ok(_) => panic!("colonna {nome}: il file doveva essere rifiutato"),
            Err(errore) => assert!(
                errore.to_string().contains("oscura l'alias"),
                "colonna {nome}: messaggio inatteso {errore}"
            ),
        }
    }
}

/// Regressione sul file che il fuzzing ha usato per portare il reader
/// oltre 4 GiB residenti: 32 KiB di `GeoPackage` in cui 228 byte sono
/// stati scritti nello spazio libero delle pagine. `SQLite` legge la
/// chiave del b-tree, ma il `rowid` che restituisce viene dal record, e i
/// due divergono: la pagina successiva ripeteva la stessa riga senza mai
/// avanzare il cursore.
///
/// Il file e' versionato in `fuzz/seeds/`, quindi la campagna lo ricarica
/// a ogni run e questa regressione resta coperta su entrambi i fronti.
#[test]
fn un_gpkg_che_non_fa_avanzare_la_paginazione_termina_con_errore() {
    let seme = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fuzz/seeds/gpkg_reader/paginazione-bloccata.gpkg");
    // Copiato in una directory temporanea: aprendolo `SQLite` puo'
    // affiancargli journal o WAL, e l'albero versionato resta pulito.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("paginazione-bloccata.gpkg");
    std::fs::copy(&seme, &path).unwrap();

    let driver = GpkgDriver;
    let Ok(dataset) = driver.open(Source::Path(path), opzioni_lettura()) else {
        // Il file e' corrotto: se una verifica a monte lo rifiuta prima
        // ancora di leggerlo, il non-avanzamento e' comunque irraggiungibile.
        return;
    };
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

    // Il tetto non e' una soglia di merito: prima della correzione questo
    // ciclo non terminava. Un limite basso trasforma la non terminazione
    // in un fallimento immediato invece che in un test appeso.
    for _ in 0..1_000 {
        match reader.next_batch() {
            Ok(None) => return,
            Ok(Some(_)) => {}
            Err(errore) => {
                assert!(
                    errore.to_string().contains("paginazione bloccata"),
                    "messaggio inatteso: {errore}"
                );
                return;
            }
        }
    }
    panic!("la lettura non e' terminata entro 1000 batch: la paginazione non avanza");
}

#[test]
fn round_trip_gpkg() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("out.gpkg");
    let wkb = to_wkb(&geo_types::Geometry::Point(geo_types::Point::new(
        12.5, 45.9,
    )))
    .unwrap();
    let geom = BinaryArray::from(vec![Some(wkb.as_slice()), Some(wkb.as_slice())]);
    let ids = Int64Array::from(vec![1i64, 2]);
    let schema: SchemaRef = Arc::new(Schema::new(vec![
        geometry_field("geom", "EPSG:4326"),
        Field::new("id", DataType::Int64, false),
    ]));
    let batch = RecordBatch::try_new(schema.clone(), vec![Arc::new(geom), Arc::new(ids)]).unwrap();

    let driver = GpkgDriver;
    let plan = WritePlan {
        layers: vec![WriteLayer {
            name: "vani".to_owned(),
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
    assert_eq!(ds.layers().len(), 1);
    assert_eq!(ds.layers()[0].name, "vani");
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
    let gcol = out
        .column_by_name("geom")
        .unwrap()
        .as_any()
        .downcast_ref::<BinaryArray>()
        .unwrap();
    assert_eq!(gcol.value(0), wkb.as_slice());
    // id preservato come Int64
    let idcol = out
        .column_by_name("id")
        .unwrap()
        .as_any()
        .downcast_ref::<Int64Array>()
        .unwrap();
    assert_eq!(idcol.value(1), 2);
    assert!(reader.next_batch().unwrap().is_none());

    let mut attributes_only = ds
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
    assert!(attributes_only.contract().contract.geometry.is_none());
    assert_eq!(attributes_only.contract().contract.schema.fields().len(), 1);
    let projected = attributes_only.next_batch().unwrap().unwrap();
    assert_eq!(projected.num_rows(), 2);
    assert_eq!(projected.num_columns(), 1);
    assert_eq!(projected.schema().field(0).name(), "id");

    let mut no_columns = ds
        .open_layer_reader(&ReadRequest {
            layer: LayerId(0),
            projected_fields: Some(Vec::new()),
            projection_mode: ProjectionMode::Required,
            pruning_predicate: None,
            spatial_pruning_hint: None,
            scope: ReadScope::default(),
            batch_target: BatchTarget::default(),
            cancellation: CancellationToken::default(),
        })
        .unwrap();
    let projected = no_columns.next_batch().unwrap().unwrap();
    assert_eq!(projected.num_rows(), 2);
    assert_eq!(projected.num_columns(), 0);
}

#[test]
fn round_trip_gpkg_preserves_xyzm_payload_and_native_metadata() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("zm.gpkg");
    let wkb = encode_wkb(
        &WkbGeometry {
            value: WkbValue::Point(WkbCoordinate {
                x: 1.0,
                y: 2.0,
                z: Some(3.0),
                m: Some(4.0),
            }),
            dimensions: CoordinateDimensions::Xyzm,
            srid: None,
        },
        WkbFlavor::Iso,
    )
    .unwrap();
    let schema: SchemaRef = Arc::new(Schema::new(vec![geometry_field("geom", "EPSG:4326")]));
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![Arc::new(BinaryArray::from(vec![Some(wkb.as_slice())]))],
    )
    .unwrap();
    let mut geometry = GeometryColumnContract::wkb_passthrough(
        FieldId(0),
        "geom",
        ResolvedCrs::new(Some("EPSG:4326".to_owned()), CrsKind::Geographic, None),
        true,
    );
    geometry.dimensions = CoordinateDimensions::Xyzm;
    geometry.srid = Some(4326);
    geometry.set_exact_geometry_types(vec![GeometryType::Point]);
    let plan = WritePlan {
        layers: vec![WriteLayer {
            name: "zm".to_owned(),
            contract: DataContract {
                schema,
                geometry: Some(geometry),
            },
        }],
    };
    let driver = GpkgDriver;
    let mut writer = driver
        .create(Sink::Path(path.clone()), &plan, &opzioni_scrittura())
        .unwrap();
    writer.write(&batch).unwrap();
    writer.finish().unwrap();

    let dataset = driver.open(Source::Path(path), opzioni_lettura()).unwrap();
    let geometry = dataset.layers()[0].contract.geometry.as_ref().unwrap();
    assert_eq!(geometry.dimensions, CoordinateDimensions::Xyzm);
    assert_eq!(geometry.srid, Some(4326));
    assert_eq!(geometry.geometry_types, vec![GeometryType::Point]);
    assert_eq!(
        geometry
            .native_metadata
            .get("gpkg.geometry_type_name")
            .map(String::as_str),
        Some("POINT")
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
    let output = reader.next_batch().unwrap().unwrap();
    let geometry_array = output
        .column_by_name("geom")
        .unwrap()
        .as_any()
        .downcast_ref::<BinaryArray>()
        .unwrap();
    assert_eq!(geometry_array.value(0), wkb);
}

#[test]
fn write_two_layers_round_trip() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("multi.gpkg");
    let wkb = to_wkb(&geo_types::Geometry::Point(geo_types::Point::new(1.0, 2.0))).unwrap();

    let s0: SchemaRef = Arc::new(Schema::new(vec![
        geometry_field("geom", "EPSG:4326"),
        Field::new("id", DataType::Int64, false),
    ]));
    let b0 = RecordBatch::try_new(
        s0.clone(),
        vec![
            Arc::new(BinaryArray::from(vec![Some(wkb.as_slice())])),
            Arc::new(Int64Array::from(vec![10i64])),
        ],
    )
    .unwrap();

    let s1: SchemaRef = Arc::new(Schema::new(vec![
        geometry_field("geom", "EPSG:4326"),
        Field::new("nome", DataType::Utf8, true),
    ]));
    let b1 = RecordBatch::try_new(
        s1.clone(),
        vec![
            Arc::new(BinaryArray::from(vec![
                Some(wkb.as_slice()),
                Some(wkb.as_slice()),
            ])),
            Arc::new(StringArray::from(vec!["A", "B"])),
        ],
    )
    .unwrap();

    let driver = GpkgDriver;
    let plan = WritePlan {
        layers: vec![
            WriteLayer {
                name: "vani".to_owned(),
                contract: DataContract {
                    schema: s0,
                    geometry: None,
                },
            },
            WriteLayer {
                name: "strade".to_owned(),
                contract: DataContract {
                    schema: s1,
                    geometry: None,
                },
            },
        ],
    };
    let mut w = driver
        .create(Sink::Path(path.clone()), &plan, &opzioni_scrittura())
        .unwrap();
    w.write_to_layer(LayerId(0), &b0).unwrap();
    w.write_to_layer(LayerId(1), &b1).unwrap();
    w.finish().unwrap();

    let ds = driver.open(Source::Path(path), opzioni_lettura()).unwrap();
    assert_eq!(ds.layers().len(), 2);
    let names: Vec<&str> = ds.layers().iter().map(|l| l.name.as_str()).collect();
    assert!(
        names.contains(&"vani") && names.contains(&"strade"),
        "layer: {names:?}"
    );

    // Ogni layer ha il suo conteggio righe (instradamento corretto).
    for l in ds.layers() {
        let expected = if l.name == "vani" { 1 } else { 2 };
        let mut r = ds
            .open_layer_reader(&ReadRequest {
                layer: l.id,
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
        assert_eq!(rb.num_rows(), expected, "layer '{}'", l.name);
    }
}

#[test]
fn round_trip_non_wgs84_crs() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("m3857.gpkg");
    // Un punto in EPSG:3857 (Web Mercator).
    let wkb = to_wkb(&geo_types::Geometry::Point(geo_types::Point::new(
        1_113_194.0,
        5_621_521.0,
    )))
    .unwrap();
    let schema: SchemaRef = Arc::new(Schema::new(vec![
        geometry_field("geom", "EPSG:3857"),
        Field::new("id", DataType::Int64, false),
    ]));
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(BinaryArray::from(vec![Some(wkb.as_slice())])),
            Arc::new(Int64Array::from(vec![1i64])),
        ],
    )
    .unwrap();
    let driver = GpkgDriver;
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

    // Rilettura: il CRS NON è più 4326 fisso, è EPSG:3857.
    let ds = driver.open(Source::Path(path), opzioni_lettura()).unwrap();
    let crs = ds.layers()[0].contract.geometry.as_ref().unwrap().crs.id();
    assert_eq!(crs, Some("EPSG:3857"));
}
