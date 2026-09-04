//! Le fixture canoniche Parquet e Arrow. **Non gira in CI.**
//!
//! # Perche' non le genera lo script Python
//!
//! `scripts/genera-fixture-canoniche.py` produce le altre otto con testo
//! scritto a mano, `zipfile` e OGR. Per Parquet e Arrow servirebbe pyarrow, che
//! non e' fra le dipendenze del container dei gate; qui invece i crate upstream
//! `arrow` e `parquet` ci sono gia', alla versione che il workspace fissa.
//!
//! # Perche' un binario e non un test
//!
//! Vale la stessa ragione dello script: una fixture rigenerata a ogni corsa
//! renderebbe l'atteso una funzione dello strumento del giorno. Si lancia a
//! mano, e `scripts/check-fixture-canoniche.py` risponde dei byte prodotti.
//!
//! # Che indipendenza e' questa, esattamente
//!
//! Indipendenza dal **codice del prodotto**: nessun byte di queste due fixture
//! passa da `driver-geoparquet` o da `driver-ipc`. Lo schema, i metadati `geo`
//! e il WKB sono scritti qui, e il WKB e' scritto **a mano** invece di
//! chiamare `plenora_io_model::wkb`: un encoder condiviso fra la fixture e il
//! lettore che deve rileggerla renderebbe invisibile un difetto simmetrico.
//!
//! Non e' invece una prova di interoperabilita' con un'implementazione
//! esterna: `arrow-rs` resta la libreria su cui il driver si appoggia. Provare
//! l'interoperabilita' con pyarrow sarebbe un esercizio distinto, e non e'
//! questo.
//!
//! # Uso
//!
//!     cargo run --bin genera-fixture-arrow -- \
//!         --destinazione crates/plenora-io-cli/tests/fixtures/canoniche

use std::collections::HashMap;
use std::fs::File;
use std::sync::Arc;

use arrow_array::{
    ArrayRef, BinaryArray, BooleanArray, Date32Array, Float64Array, Int32Array, Int64Array,
    RecordBatch, StringArray,
};
use arrow_schema::{DataType, Field, Schema};

/// Il CRS della variante proiettata, come lo scrivono le altre fixture.
const CRS_ID: &str = "EPSG:3003";

/// Le chiavi di metadato del contratto geometrico, scritte per valore.
///
/// Sono le stesse costanti che `plenora-io-model` dichiara, e qui **non**
/// vengono importate da li': se un giorno il prodotto cambiasse la chiave, la
/// fixture continuerebbe a portare quella vecchia e il driver fallirebbe a
/// rileggerla -- che e' precisamente il segnale voluto. Una fixture che segue
/// il prodotto non lo mette alla prova.
const CHIAVE_ESTENSIONE: &str = "ARROW:extension:name";
const ESTENSIONE_WKB: &str = "geoarrow.wkb";
const CHIAVE_CRS: &str = "crs";

/// Il registro da cui viene il PROJJSON: congelato li', non ricalcolato qui.
const REGISTRO: &str = "assurance/registries/fixture-canoniche.json";

// --- il contenuto canonico ---------------------------------------------------
//
// Le stesse cinque righe di `scripts/genera-fixture-canoniche.py`, e la
// duplicazione e' voluta: i due generatori non condividono codice, quindi non
// possono sbagliare insieme. A tenerli allineati e' l'oracle delle conversioni,
// che confronta ogni uscita contro il dataset canonico e diventa rosso se una
// delle due famiglie di fixture si scosta.

struct Riga {
    id: &'static str,
    codice: &'static str,
    etichetta: Option<&'static str>,
    intero_largo: i64,
    conteggio: Option<i32>,
    misura: Option<f64>,
    attivo: Option<bool>,
    istante: (i32, u32, u32),
    geometria: Option<Geometria>,
}

/// Le quattro forme del dataset, nella variante proiettata.
enum Geometria {
    Punto,
    Linea,
    Poligono,
    PuntoZ,
}

const RIGHE: &[Riga] = &[
    Riga {
        id: "r1",
        codice: "A-1",
        etichetta: Some("città"),
        // 2^53+1: il primo intero che un float64 non rappresenta. Un formato
        // che lo facesse passare da un double lo restituirebbe come
        // 9007199254740992, e la differenza di uno e' cio' che un confronto
        // per valore trova e uno per ordine di grandezza no.
        intero_largo: 9_007_199_254_740_993,
        conteggio: Some(7),
        misura: Some(1.5),
        attivo: Some(true),
        istante: (2026, 1, 15),
        geometria: Some(Geometria::Punto),
    },
    Riga {
        id: "r2",
        codice: "B-2",
        etichetta: None,
        intero_largo: -9_007_199_254_740_993,
        conteggio: None,
        misura: Some(-0.125),
        attivo: Some(false),
        istante: (2026, 2, 28),
        geometria: Some(Geometria::Linea),
    },
    Riga {
        id: "r3",
        codice: "Ç-3",
        etichetta: Some("naïve"),
        intero_largo: 0,
        conteggio: Some(0),
        misura: None,
        attivo: None,
        istante: (2026, 3, 1),
        geometria: Some(Geometria::Poligono),
    },
    Riga {
        id: "r4",
        // La stringa vuota **non** e' un null, ed e' l'unica riga che lo dice.
        codice: "D-4",
        etichetta: Some(""),
        intero_largo: 1,
        conteggio: Some(-3),
        misura: Some(std::f64::consts::PI),
        attivo: Some(true),
        istante: (2026, 12, 31),
        geometria: Some(Geometria::PuntoZ),
    },
    Riga {
        id: "r5",
        codice: "E-5",
        etichetta: Some("senza geometria"),
        intero_largo: 9_007_199_254_740_992,
        conteggio: Some(42),
        misura: Some(0.0),
        attivo: Some(false),
        istante: (2026, 6, 30),
        geometria: None,
    },
];

// --- WKB scritto a mano ------------------------------------------------------

/// I tipi WKB nel dialetto **ISO**: la terza dimensione somma mille al tipo.
///
/// Non EWKB, che la marcherebbe con un bit alto: entrambi sono leggibili dal
/// codec del prodotto, e sceglierne uno solo rende la fixture una domanda
/// precisa invece di due domande insieme.
const WKB_PUNTO: u32 = 1;
const WKB_LINEA: u32 = 2;
const WKB_POLIGONO: u32 = 3;
const WKB_PUNTO_Z: u32 = 1001;

struct Scrittore(Vec<u8>);

impl Scrittore {
    fn nuovo(tipo: u32) -> Self {
        let mut byte = Vec::new();
        // 1 = little-endian, per ogni geometria di questa fixture.
        byte.push(1_u8);
        byte.extend_from_slice(&tipo.to_le_bytes());
        Self(byte)
    }

    fn conteggio(&mut self, quanti: u32) -> &mut Self {
        self.0.extend_from_slice(&quanti.to_le_bytes());
        self
    }

    fn coordinata(&mut self, valori: &[f64]) -> &mut Self {
        for valore in valori {
            self.0.extend_from_slice(&valore.to_le_bytes());
        }
        self
    }

    fn byte(&self) -> Vec<u8> {
        self.0.clone()
    }
}

/// Le coordinate proiettate: gli stessi numeri del generatore Python.
fn wkb(forma: &Geometria) -> Vec<u8> {
    match forma {
        Geometria::Punto => Scrittore::nuovo(WKB_PUNTO)
            .coordinata(&[1_650_000.0, 4_850_000.0])
            .byte(),
        Geometria::Linea => Scrittore::nuovo(WKB_LINEA)
            .conteggio(2)
            .coordinata(&[1_650_000.0, 4_850_000.0, 1_650_100.0, 4_850_100.0])
            .byte(),
        Geometria::Poligono => Scrittore::nuovo(WKB_POLIGONO)
            .conteggio(1)
            .conteggio(5)
            .coordinata(&[
                1_651_000.0,
                4_851_000.0, //
                1_651_100.0,
                4_851_000.0, //
                1_651_100.0,
                4_851_100.0, //
                1_651_000.0,
                4_851_100.0, //
                1_651_000.0,
                4_851_000.0,
            ])
            .byte(),
        Geometria::PuntoZ => Scrittore::nuovo(WKB_PUNTO_Z)
            .coordinata(&[1_652_000.0, 4_852_000.0, 125.5])
            .byte(),
    }
}

// --- lo schema e il batch ----------------------------------------------------

/// Giorni dall'epoca Unix, dall'algoritmo `days_from_civil` di Howard Hinnant.
///
/// Scritto qui invece di prendere `chrono`: una dipendenza in piu' nel
/// workspace costa cinque misure di profondita' fuzz, e questo e' un
/// calendario proletticamente gregoriano di dodici righe.
fn giorni_dall_epoca(anno: i32, mese: u32, giorno: u32) -> i32 {
    let anno = anno - i32::from(mese <= 2);
    let era = if anno >= 0 { anno } else { anno - 399 } / 400;
    let anno_dell_era = anno - era * 400;
    let mese = i32::try_from(mese).expect("mese entro i32");
    let giorno = i32::try_from(giorno).expect("giorno entro i32");
    let giorno_dell_anno = (153 * (mese + if mese > 2 { -3 } else { 9 }) + 2) / 5 + giorno - 1;
    let giorno_dell_era =
        anno_dell_era * 365 + anno_dell_era / 4 - anno_dell_era / 100 + giorno_dell_anno;
    era * 146_097 + giorno_dell_era - 719_468
}

/// Il contratto geometrico, scritto nei metadati del campo.
///
/// L'IPC e' gia' Arrow: non ha un posto suo dove dichiarare i tipi geometrici,
/// e li dichiara nelle stesse chiavi che il prodotto documenta. Senza,
/// `ipc -> geoparquet` viene rifiutata -- «il formato richiede una
/// dichiarazione preventiva dei tipi geometrici» -- e il rifiuto e' corretto:
/// `GeoParquet` scrive `geometry_types` nel proprio metadato `geo`, e non puo'
/// scoprirli mentre scrive.
///
/// `dimensions` e' `unknown` e non `xyz`: la colonna porta quattro geometrie
/// 2D e una 3D, e nessuna etichetta singola e' vera per tutte. E' lo stesso
/// valore a cui arriva il lettore `GeoParquet` sulla fixture gemella, che quando
/// le dimensioni sono piu' d'una non ne sceglie una.
fn campo_geometrico(projjson: &str) -> Field {
    let mut metadati = HashMap::new();
    metadati.insert(CHIAVE_ESTENSIONE.to_owned(), ESTENSIONE_WKB.to_owned());
    metadati.insert(CHIAVE_CRS.to_owned(), CRS_ID.to_owned());
    metadati.insert("plenora.field_id".to_owned(), "8".to_owned());
    metadati.insert("plenora.geometry.encoding".to_owned(), "wkb".to_owned());
    metadati.insert(
        "plenora.geometry.dimensions".to_owned(),
        "unknown".to_owned(),
    );
    metadati.insert(
        "plenora.geometry.spatial_semantics".to_owned(),
        "geometry".to_owned(),
    );
    metadati.insert(
        "plenora.geometry.precision".to_owned(),
        "float64".to_owned(),
    );
    metadati.insert(
        "plenora.geometry.types_declaration".to_owned(),
        "exact".to_owned(),
    );
    // La Z non e' un tipo: e' una dimensione. I tipi sono tre, e il punto Z e'
    // uno dei tre con una coordinata in piu'.
    metadati.insert(
        "plenora.geometry.types".to_owned(),
        "point,linestring,polygon".to_owned(),
    );
    // Il CRS, nella forma namespaced. Non e' una ripetizione di `crs`: quella
    // chiave e' la convenzione GeoArrow e porta il solo identificatore, questa
    // dice **come** il CRS e' stato risolto, e senza di lei il campo non e' un
    // contratto canonico valido -- `encoding`, `dimensions`, `crs_resolution` e
    // `types_declaration` vanno dichiarate tutte e quattro o nessuna.
    metadati.insert(
        "plenora.geometry.crs_resolution".to_owned(),
        "resolved".to_owned(),
    );
    metadati.insert("plenora.geometry.crs_id".to_owned(), CRS_ID.to_owned());
    // Proiettato, quindi est-nord: e' l'ordine assi che l'identificatore
    // implica, e per una risoluzione `resolved` va dichiarato.
    metadati.insert(
        "plenora.geometry.axis_order".to_owned(),
        "easting_northing".to_owned(),
    );
    // La definizione, e non il solo identificatore: senza, `ipc -> geoparquet`
    // viene rifiutata -- «il CRS e' noto solo per identificatore e GeoParquet
    // pretende un documento PROJJSON». Il rifiuto e' corretto e il posto giusto
    // per chiuderlo e' qui: e' la fixture a dover portare il CRS per intero,
    // perche' l'IPC ha dove metterlo.
    metadati.insert(
        "plenora.geometry.crs_definition".to_owned(),
        projjson.to_owned(),
    );
    metadati.insert(
        "plenora.geometry.crs_definition_format".to_owned(),
        "projjson".to_owned(),
    );
    Field::new("geometry", DataType::Binary, true).with_metadata(metadati)
}

fn schema(projjson: &str) -> Schema {
    let mut metadati = HashMap::new();
    metadati.insert("plenora.contract.version".to_owned(), "1".to_owned());
    Schema::new_with_metadata(
        vec![
            Field::new("id", DataType::Utf8, false),
            Field::new("codice", DataType::Utf8, false),
            Field::new("etichetta", DataType::Utf8, true),
            Field::new("intero_largo", DataType::Int64, false),
            Field::new("conteggio", DataType::Int32, true),
            Field::new("misura", DataType::Float64, true),
            Field::new("attivo", DataType::Boolean, true),
            Field::new("istante", DataType::Date32, false),
            campo_geometrico(projjson),
        ],
        metadati,
    )
}

fn batch(schema: &Arc<Schema>) -> RecordBatch {
    batch_da(schema, &RIGHE.iter().collect::<Vec<_>>())
}

/// Le sole righe che portano una geometria.
///
/// Serve alla conversione verso DXF, che la riga senza geometria **rifiuta**,
/// e giustamente: un'entita' di disegno senza geometria non esiste. Il rifiuto
/// resta un caso a se'; questa variante permette di provare l'altra meta' --
/// che le geometrie, gli attributi e le approssimazioni dichiarate arrivino --
/// senza scartare in silenzio la riga che il rifiuto riguarda.
fn righe_con_geometria() -> Vec<&'static Riga> {
    RIGHE.iter().filter(|r| r.geometria.is_some()).collect()
}

fn batch_da(schema: &Arc<Schema>, righe: &[&Riga]) -> RecordBatch {
    let geometrie: Vec<Option<Vec<u8>>> = righe
        .iter()
        .map(|r| r.geometria.as_ref().map(wkb))
        .collect();
    let colonne: Vec<ArrayRef> = vec![
        Arc::new(StringArray::from_iter_values(righe.iter().map(|r| r.id))),
        Arc::new(StringArray::from_iter_values(
            righe.iter().map(|r| r.codice),
        )),
        Arc::new(righe.iter().map(|r| r.etichetta).collect::<StringArray>()),
        Arc::new(Int64Array::from_iter_values(
            righe.iter().map(|r| r.intero_largo),
        )),
        Arc::new(righe.iter().map(|r| r.conteggio).collect::<Int32Array>()),
        Arc::new(righe.iter().map(|r| r.misura).collect::<Float64Array>()),
        Arc::new(righe.iter().map(|r| r.attivo).collect::<BooleanArray>()),
        Arc::new(Date32Array::from_iter_values(righe.iter().map(|r| {
            let (anno, mese, giorno) = r.istante;
            giorni_dall_epoca(anno, mese, giorno)
        }))),
        Arc::new(
            geometrie
                .iter()
                .map(std::option::Option::as_deref)
                .collect::<BinaryArray>(),
        ),
    ];
    RecordBatch::try_new(Arc::clone(schema), colonne).expect("il batch canonico si costruisce")
}

// --- i metadati `geo` di GeoParquet ------------------------------------------

/// Il documento `geo`, con il PROJJSON preso dal registro.
///
/// Il PROJJSON non e' scritto qui: sta congelato in
/// `assurance/registries/fixture-canoniche.json` insieme a strumento, versione
/// e comando che l'hanno prodotto, come le coordinate della variante
/// geografica. Ricalcolarlo a ogni generazione legherebbe la fixture alla
/// versione di PROJ installata quel giorno.
/// Il PROJJSON congelato nel registro.
///
/// Letto una volta e usato da entrambe le fixture: il metadato `geo` del
/// Parquet e il contratto geometrico dell'Arrow portano lo **stesso**
/// documento, perche' e' lo stesso CRS.
fn projjson(radice: &std::path::Path) -> serde_json::Value {
    // `let ... else` e non `unwrap_or_else(|_| panic!(...))`: qui non c'e'
    // nessun fallback -- si ferma in entrambi i rami -- e la forma
    // `unwrap_or*(` e' quella che il registro dei fallback conta. Contarne uno
    // che non esiste renderebbe il registro meno leggibile proprio dove serve
    // che sia esatto.
    let Ok(registro) = std::fs::read_to_string(radice.join(REGISTRO)) else {
        panic!("registro {REGISTRO} leggibile dalla radice del repository")
    };
    let registro: serde_json::Value =
        serde_json::from_str(&registro).expect("il registro e' JSON valido");
    let projjson = registro["provenienza"]["crs_proiettato"]["projjson"].clone();
    assert!(
        projjson.is_object(),
        "il registro non porta `provenienza.crs_proiettato.projjson`: senza, ne' la \
         fixture GeoParquet ne' quella Arrow avrebbero un CRS conforme"
    );
    projjson
}

fn metadato_geo(projjson: &serde_json::Value) -> String {
    serde_json::json!({
        "version": "1.1.0",
        "primary_column": "geometry",
        "columns": {
            "geometry": {
                "encoding": "WKB",
                // Dichiarati tutti e quattro, Z compreso: un elenco piu' corto
                // di cio' che il file contiene e' un file non conforme, e
                // sarebbe la fixture a mentire invece del driver a sbagliare.
                "geometry_types": ["Point", "LineString", "Polygon", "Point Z"],
                "crs": projjson.clone(),
            }
        }
    })
    .to_string()
}

// --- le due scritture --------------------------------------------------------

fn scrivi_arrow(percorso: &std::path::Path, schema: &Arc<Schema>, batch: &RecordBatch) {
    let file = File::create(percorso).expect("fixture Arrow creabile");
    let mut scrittore =
        arrow_ipc::writer::FileWriter::try_new(file, schema).expect("writer IPC costruibile");
    scrittore.write(batch).expect("batch scritto");
    scrittore.finish().expect("fixture Arrow completata");
}

fn scrivi_parquet(
    percorso: &std::path::Path,
    schema: &Arc<Schema>,
    batch: &RecordBatch,
    geo: String,
) {
    use parquet::file::metadata::KeyValue;
    use parquet::file::properties::WriterProperties;

    let proprieta = WriterProperties::builder()
        .set_key_value_metadata(Some(vec![KeyValue::new("geo".to_owned(), geo)]))
        .build();
    let file = File::create(percorso).expect("fixture Parquet creabile");
    let mut scrittore =
        parquet::arrow::ArrowWriter::try_new(file, Arc::clone(schema), Some(proprieta))
            .expect("writer Parquet costruibile");
    scrittore.write(batch).expect("batch scritto");
    scrittore.close().expect("fixture Parquet completata");
}

fn main() {
    let mut argomenti = std::env::args().skip(1);
    let mut destinazione: Option<std::path::PathBuf> = None;
    let mut radice = std::path::PathBuf::from(".");
    while let Some(argomento) = argomenti.next() {
        match argomento.as_str() {
            "--destinazione" => {
                destinazione = argomenti.next().map(std::path::PathBuf::from);
            }
            "--radice" => {
                radice = argomenti
                    .next()
                    .map_or_else(|| std::path::PathBuf::from("."), std::path::PathBuf::from);
            }
            altro => {
                eprintln!("argomento sconosciuto: {altro}");
                std::process::exit(2);
            }
        }
    }
    let Some(destinazione) = destinazione else {
        eprintln!("uso: genera-fixture-arrow --destinazione <dir> [--radice <repo>]");
        std::process::exit(2);
    };
    std::fs::create_dir_all(&destinazione).expect("destinazione creabile");

    let crs = projjson(&radice);
    let crs_compatto = serde_json::to_string(&crs).expect("il PROJJSON si riserializza");
    let schema = Arc::new(schema(&crs_compatto));
    let batch = batch(&schema);

    let arrow = destinazione.join("canonico.arrow");
    scrivi_arrow(&arrow, &schema, &batch);
    let parquet = destinazione.join("canonico.parquet");
    scrivi_parquet(&parquet, &schema, &batch, metadato_geo(&crs));

    // La variante senza la riga priva di geometria, per i bersagli che una
    // feature senza geometria la rifiutano -- il DXF, dove un'entita' di
    // disegno senza geometria non esiste. Il rifiuto resta un caso a se': qui
    // si prova l'altra meta', cioe' che tutto il resto arrivi.
    let pieno = destinazione.join("canonico_pieno.parquet");
    let batch_pieno = batch_da(&schema, &righe_con_geometria());
    scrivi_parquet(&pieno, &schema, &batch_pieno, metadato_geo(&crs));

    println!("  canonico.arrow");
    println!("  canonico.parquet");
    println!("  canonico_pieno.parquet");
    println!("3 fixture generate in {}", destinazione.display());
    println!("Ora aggiorna il registro: scripts/check-fixture-canoniche.py --mostra-manifesto");
}
