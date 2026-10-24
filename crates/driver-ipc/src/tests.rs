//! Le prove di [`super`], in un file loro.
//!
//! Restano un **modulo figlio**: vedono i privati del genitore
//! esattamente come quando stavano dentro di lui, e non allargano di
//! una riga la superficie pubblica del crate.

use super::*;

mod buffer_ipc;
mod layout_ipc;
mod parametri_schema_ipc;
mod schema_ipc;
mod tipi_geometrici;

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

use std::sync::Arc;

use arrow_array::{BinaryArray, Int64Array};
use arrow_schema::{DataType, Field, Schema, SchemaRef};
use plenora_io_core::request::{BatchTarget, ProjectionMode, ReadScope};
use plenora_io_core::WriteLayer;
use plenora_io_model::contract::{
    CoordinateDimensions, CoordinatePrecision, GeometryEncoding, GeometryType, SpatialSemantics,
};
use plenora_io_model::wkb::{encode_wkb, to_wkb, WkbCoordinate, WkbFlavor, WkbGeometry, WkbValue};
use plenora_io_model::CancellationToken;

/// Il file che faceva panicare arrow viene ora **rifiutato prima** che
/// arrow lo tocchi (FZ-0).
///
/// Prima di FZ-0 questo test osservava la barriera `catch_unwind`: il
/// panico avveniva e veniva convertito. Non bastava — un panico catturato
/// e' pur sempre un panico, e sotto `libfuzzer-sys` diventa `abort()`
/// prima dell'unwinding, quindi il target restava rosso e in quarantena.
///
/// Ora il difetto e' impedito: `valida_file_ipc` verifica schema e buffer
/// dichiarati contro il corpo del messaggio, e questo file dichiara un
/// buffer oltre la fine del proprio corpo. La verifica e' che l'errore
/// **non** venga dalla barriera.
#[test]
fn un_ipc_non_conforme_e_rifiutato_prima_di_arrow() {
    let seme = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fuzz/seeds/ipc_reader/schema-che-fa-panicare-arrow.arrow");

    let errore = match IpcDriver.open(Source::Path(seme), opzioni_lettura()) {
        Err(errore) => errore,
        Ok(dataset) => {
            let request = ReadRequest {
                layer: LayerId(0),
                projected_fields: None,
                projection_mode: ProjectionMode::BestEffort,
                pruning_predicate: None,
                spatial_pruning_hint: None,
                scope: ReadScope::default(),
                batch_target: BatchTarget::default(),
                cancellation: CancellationToken::default(),
            };
            match dataset.open_layer_reader(&request) {
                Err(errore) => errore,
                Ok(mut reader) => loop {
                    match reader.next_batch() {
                        Ok(Some(_)) => {}
                        Ok(None) => panic!("il file doveva essere rifiutato"),
                        Err(errore) => break errore,
                    }
                },
            }
        }
    };
    assert!(
        !errore.to_string().contains("in panico"),
        "il rifiuto deve precedere arrow, non seguirne il panico: {errore}"
    );
    assert_eq!(errore.phase, plenora_io_model::ErrorPhase::Read);
    assert_eq!(errore.code, plenora_io_model::IoErrorCode::Format);
}

#[test]
fn buffer_fuori_corpo_rifiutato_prima_di_arrow() {
    let seme = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fuzz/seeds/ipc_reader/buffer-fuori-corpo.arrow");

    let errore = match IpcDriver.open(Source::Path(seme), opzioni_lettura()) {
        Err(errore) => errore,
        Ok(dataset) => {
            let request = ReadRequest {
                layer: LayerId(0),
                projected_fields: None,
                projection_mode: ProjectionMode::BestEffort,
                pruning_predicate: None,
                spatial_pruning_hint: None,
                scope: ReadScope::default(),
                batch_target: BatchTarget::default(),
                cancellation: CancellationToken::default(),
            };
            match dataset.open_layer_reader(&request) {
                Err(errore) => errore,
                Ok(mut reader) => loop {
                    match reader.next_batch() {
                        Ok(Some(_)) => {}
                        Ok(None) => panic!("il file doveva essere rifiutato"),
                        Err(errore) => break errore,
                    }
                },
            }
        }
    };
    assert!(
        !errore.to_string().contains("in panico"),
        "il rifiuto deve precedere arrow, non seguirne il panico: {errore}"
    );
    assert!(
        errore
            .message
            .contains("buffer Arrow oltre la fine del corpo del messaggio"),
        "{errore}"
    );
    assert_eq!(errore.phase, plenora_io_model::ErrorPhase::Read);
    assert_eq!(errore.code, plenora_io_model::IoErrorCode::Format);
}

/// Un IPC conforme continua a essere letto: la prevalidazione non rifiuta
/// cio' che il formato ammette.
///
/// Senza questo, una verifica troppo severa passerebbe il test sopra e
/// romperebbe ogni file reale senza che nessuno se ne accorgesse qui.
#[test]
fn un_ipc_conforme_supera_la_prevalidazione() {
    let seme = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fuzz/seeds/ipc_reader/minimal.arrow");
    assert!(seme.is_file(), "seme assente: {}", seme.display());
    driver_common::prevalida_arrow::valida_file_ipc("arrow", &seme)
        .expect("un IPC conforme non deve essere rifiutato");
}

/// I metadati del footer con un campo assente sono rifiutati **prima** di
/// arrow, e i due campi si verificano separatamente.
///
/// # Il difetto
///
/// `arrow-ipc` legge i metadati del footer con `kv.key().unwrap()` e
/// `kv.value().unwrap()` (`reader.rs:1263-1264`). Nella grammatica
/// flatbuffer quei due campi sono **facoltativi**, e arrow stessa lo sa: in
/// `convert.rs` -- sui metadati di un campo e su quelli dello schema --
/// salta la voce incompleta invece di aprirla. Sono i soli due `unwrap` su
/// questi campi in tutto il crate, ed e' un'incoerenza interna ad arrow: la
/// stessa struttura, letta in modo sicuro in due punti e non nel terzo.
///
/// Trovato dalla campagna fuzz del 2026-09-07 su `ipc_reader`, con un input
/// di 2764 byte che portava dieci voci vuote nel footer. I due semi qui
/// sotto sono costruiti invece che ridotti: partono da un file che arrow
/// scrive, e ne azzerano **uno** slot del vtable -- che e' il modo in cui
/// flatbuffer omette un campo, non una corruzione. Cosi' ciascuno prova un
/// campo solo, cosa che il caso originale, con entrambi assenti, non
/// distingue.
///
/// # Perche' rifiutare, dato che c'e' `catch_unwind`
///
/// Perche' il panico avviene comunque. La barriera lo converte in un
/// errore, ma sotto `libfuzzer-sys` un panico diventa `abort()` prima
/// dell'unwinding: il bersaglio resta rosso e va in quarantena. E' la stessa
/// ragione di FZ-0, e questa e' la voce che quella verifica non guardava.
#[test]
fn un_metadato_del_footer_senza_un_campo_e_rifiutato_prima_di_arrow() {
    for (seme, atteso) in [
        ("footer-senza-chiave.arrow", "senza chiave"),
        ("footer-senza-valore.arrow", "senza valore"),
    ] {
        let percorso = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fuzz/seeds/ipc_reader")
            .join(seme);
        assert!(percorso.is_file(), "seme assente: {}", percorso.display());

        let Err(errore) = IpcDriver.open(Source::Path(percorso), opzioni_lettura()) else {
            panic!("{seme} doveva essere rifiutato")
        };
        assert!(
            !errore.to_string().contains("in panico"),
            "{seme}: il rifiuto deve precedere arrow, non seguirne il panico: {errore}"
        );
        assert!(
            errore.to_string().contains(atteso),
            "{seme}: l'errore deve dire quale campo manca, e dice «{errore}»"
        );
        assert_eq!(errore.phase, plenora_io_model::ErrorPhase::Read);
        assert_eq!(errore.code, plenora_io_model::IoErrorCode::Format);
    }
}

/// Un messaggio di dizionario **senza dati** e' rifiutato prima di arrow.
///
/// `DictionaryBatch.data` e' facoltativo nella grammatica flatbuffer e
/// obbligatorio nel formato. `get_dictionary_values` lo apre con `unwrap`
/// (`arrow-ipc/src/reader.rs:895`), quindi un messaggio che lo omette fa
/// panicare la libreria prima di arrivare a qualunque errore. Rifiutarlo non
/// e' piu' severo di arrow: e' la **stessa** assunzione, restituita come
/// errore invece che come panico.
///
/// Trovato dalla campagna fuzz della CI il 2026-09-07, corsa
/// [34155579047][corsa]. E' il terzo panico della famiglia -- dopo i
/// metadati del footer e la bitmap del dizionario -- e il terzo ha
/// suggerito di smettere di rincorrerli uno per uno.
///
/// # Gli `unwrap` di `arrow-ipc/src/reader.rs`, e chi li copre
///
/// | riga | che cosa apre | copertura |
/// |---|---|---|
/// | 572 | `child.as_ref().unwrap()` | irraggiungibile: `if child.is_none()` lo riempie due righe sopra |
/// | 636 | `skip_buffer`: `buffers.next().unwrap()` | `preleva_buffer` rifiuta «meno buffer dello schema», e la passeggiata consuma i buffer di **ogni** campo, anche quelli che una proiezione salterebbe |
/// | 895 | `batch.data().unwrap()` | **questa** verifica |
/// | 918-920 | lunghezze del blocco, `to_usize` e `checked_add` | `valida_blocco`: lunghezze negative rifiutate, e offset piu' lunghezza dentro il file |
/// | 950 | `buf[..4].try_into().unwrap()` | il minimo che pretendiamo dal file e' `MAGIC * 2 + 2 + 4` |
/// | 1098 | `header_as_dictionary_batch().unwrap()` | irraggiungibile: preceduto dal controllo del tipo di header |
/// | 1251 | `footer.schema().unwrap()` | «footer Arrow senza schema» |
/// | 1264-1265 | `kv.key()` e `kv.value()` | il finding sui metadati del footer, chiuso lo stesso giorno |
///
/// # Che cosa questo elenco **non** dice
///
/// Non dice che arrow non possa panicare leggendo un file. Copre gli otto
/// `unwrap()` di **questo** file in **questa** versione, e nient'altro.
/// Restano fuori gli `assert!`, le indicizzazioni dirette, le aritmetiche
/// che possono traboccare, e ogni panico nelle funzioni **chiamate** dal
/// reader, che stanno in altri crate.
///
/// La prova che non basta e' nella stessa giornata: il secondo finding del
/// 2026-09-07 -- la bitmap del dizionario piu' corta della lunghezza
/// dichiarata -- era un `assert!` in **`arrow-buffer`**, raggiunto
/// attraverso `arrow-ipc`. Un censimento degli `unwrap()` di `reader.rs`
/// non lo avrebbe trovato, e infatti non lo ha trovato: lo ha trovato il
/// fuzzing.
///
/// L'elenco chiude **una classe** -- `unwrap()` su un campo facoltativo del
/// flatbuffer, qui dentro -- e va rifatto a ogni aggiornamento di arrow.
/// Le altre forme restano dove sono sempre state: il fuzzing e la barriera.
///
/// [corsa]: https://github.com/PlenoraETL/plenora-IO-tools/actions/runs/34155579047
#[test]
fn un_dizionario_senza_dati_e_rifiutato_prima_di_arrow() {
    let seme = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fuzz/seeds/ipc_reader/dizionario-senza-dati.arrow");
    assert!(seme.is_file(), "seme assente: {}", seme.display());

    let Err(errore) = IpcDriver.open(Source::Path(seme), opzioni_lettura()) else {
        panic!("un dizionario senza dati non deve aprirsi")
    };
    let testo = errore.to_string();
    assert!(
        !testo.contains("in panico"),
        "il rifiuto deve precedere arrow, non seguirne il panico: {testo}"
    );
    assert!(
        testo.contains("senza dati"),
        "l'errore deve dire che cosa manca, e dice «{testo}»"
    );
    assert_eq!(errore.phase, plenora_io_model::ErrorPhase::Read);
    assert_eq!(errore.code, plenora_io_model::IoErrorCode::Format);
}

/// Il batch del **dizionario** con una bitmap piu' corta della lunghezza
/// dichiarata e' rifiutato prima di arrow.
///
/// # Il buco
///
/// La prevalidazione guardava il messaggio del dizionario solo con
/// `valida_buffer`, che verifica che ogni buffer stia dentro il corpo.
/// Nessuno accoppiava la lunghezza del nodo alla bitmap di validita', e
/// quel controllo e' proprio cio' che impedisce l'assert di
/// `BooleanBuffer::new`. Il record batch lo aveva; il dizionario no.
///
/// Il caso dichiara un nodo lungo 4 278 190 083 con **un byte** di bitmap:
///
/// ```text
/// arrow-buffer/src/buffer/boolean.rs:128:
/// buffer not large enough (bit_offset: 0, bit_len: 4278190083, buffer_len: 1)
/// ```
///
/// # Da dove viene, e che cosa non e'
///
/// Dalla campagna fuzz della CI su `ipc_reader`, il 2026-09-07, corsa
/// [34145597421][corsa]. **Non** e' una regressione dell'aggiornamento ad
/// arrow 59.3.0: lo stesso file panica identico contro la 59.1.0 -- provato
/// costruendo il binario alla revisione `3681d26` -- e l'`assert!` sta alla
/// stessa riga in entrambe le versioni. Era una lacuna nostra, che la
/// campagna ha raggiunto ora. Il seme a dizionario aggiunto poche ore prima
/// col salto ad arrow 59.3.0 ha probabilmente aiutato il fuzzer ad
/// arrivarci: e' cio' per cui i semi si aggiungono.
///
/// [corsa]: https://github.com/PlenoraETL/plenora-IO-tools/actions/runs/34145597421
#[test]
fn un_dizionario_con_la_bitmap_corta_e_rifiutato_prima_di_arrow() {
    let seme = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fuzz/seeds/ipc_reader/dizionario-bitmap-corta.arrow");
    assert!(seme.is_file(), "seme assente: {}", seme.display());

    let Err(errore) = IpcDriver.open(Source::Path(seme), opzioni_lettura()) else {
        panic!("un dizionario con la bitmap corta non deve aprirsi")
    };
    let testo = errore.to_string();
    assert!(
        !testo.contains("in panico"),
        "il rifiuto deve precedere arrow, non seguirne il panico: {testo}"
    );
    assert!(
        testo.contains("bitmap di validita'"),
        "l'errore deve dire che cosa non torna, e dice «{testo}»"
    );
    assert_eq!(errore.phase, plenora_io_model::ErrorPhase::Read);
    assert_eq!(errore.code, plenora_io_model::IoErrorCode::Format);
}

/// Una colonna a **dizionario** si rilegge con i valori giusti.
///
/// # Perche' questo test esiste
///
/// L'aggiornamento ad `arrow 59.3.0` cambia il comportamento del reader IPC
/// proprio sui dizionari: in `arrow-ipc/src/reader.rs` cambiano le attese
/// sulle chiavi `Int8` e un campo che il commento dichiarava «technically
/// not legal for this field to be null» diventa non nullable. E' l'unica
/// modifica di comportamento -- non di forma -- che quel salto di versione
/// porta sul percorso di lettura.
///
/// Il prodotto non **scrive** dizionari: i tipi che i nostri writer
/// emettono sono `Binary`, `Utf8`, `Int64`, `Float64`. Ma li **legge**, e li
/// attraversa in prevalidazione -- `header_as_dictionary_batch` in
/// `prevalida_arrow` -- perche' un file di terze parti puo' portarli. Senza
/// questo test l'aggiornamento avrebbe attraversato quella modifica senza
/// che niente la guardasse.
///
/// Il seme e' costruito da `driver-common/examples/scrivi_ipc_con_dizionario`:
/// chiavi `Int8` -- il tipo su cui le attese sono cambiate -- sei righe, tre
/// valori distinti con ripetizioni, e un nullo, che e' il caso che quella
/// modifica riguarda.
#[test]
fn una_colonna_a_dizionario_si_rilegge_con_i_valori_giusti() {
    use arrow_array::Array as _;

    let seme = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fuzz/seeds/ipc_reader/dizionario-int8.arrow");
    assert!(seme.is_file(), "seme assente: {}", seme.display());

    let ds = IpcDriver
        .open(Source::Path(seme), opzioni_lettura())
        .expect("un file con una colonna a dizionario si apre");
    let layer = ds.layers()[0].clone();
    let mut reader = ds
        .open_layer_reader(&ReadRequest {
            layer: layer.id,
            projected_fields: None,
            projection_mode: ProjectionMode::BestEffort,
            pruning_predicate: None,
            spatial_pruning_hint: None,
            scope: ReadScope::default(),
            batch_target: BatchTarget::default(),
            cancellation: CancellationToken::default(),
        })
        .expect("il reader si apre");

    let batch = reader.next_batch().unwrap().expect("un batch c'e'");
    assert_eq!(batch.num_rows(), 6);
    assert!(reader.next_batch().unwrap().is_none());

    // I valori, non solo il conteggio: e' la sostanza della modifica.
    let colonna = batch.column(0);
    let dizionario = colonna
        .as_any()
        .downcast_ref::<arrow_array::DictionaryArray<arrow_array::types::Int8Type>>()
        .expect("la colonna resta un dizionario Int8");
    let valori = dizionario
        .values()
        .as_any()
        .downcast_ref::<arrow_array::StringArray>()
        .expect("i valori del dizionario sono stringhe");

    let letti: Vec<Option<&str>> = (0..dizionario.len())
        .map(|i| {
            if dizionario.is_null(i) {
                None
            } else {
                // Le chiavi sono `Int8`: l'indice non puo' essere negativo,
                // e se lo fosse il file sarebbe corrotto e il test deve dirlo.
                let chiave = usize::try_from(dizionario.keys().value(i))
                    .expect("un indice del dizionario non puo' essere negativo");
                Some(valori.value(chiave))
            }
        })
        .collect();
    assert_eq!(
        letti,
        vec![
            Some("alfa"),
            Some("beta"),
            Some("alfa"),
            None,
            Some("gamma"),
            Some("beta"),
        ],
        "il dizionario deve rileggersi con gli stessi valori, nullo compreso"
    );
}

/// La controprova positiva: metadati **completi** nel footer si leggono.
///
/// Senza questa riga, una verifica che rifiutasse ogni metadato passerebbe
/// il test sopra e romperebbe ogni file che ne porta uno -- e i file reali
/// ne portano: `arrow-rs` scrive `ARROW:schema` nel footer Parquet, e i
/// nostri stessi artefatti ne dichiarano.
///
/// Il seme e' lo **stesso file** da cui i due ostili derivano, senza lo slot
/// azzerato: la differenza fra passare e non passare e' esattamente il campo
/// che manca, e nient'altro.
#[test]
fn un_footer_con_metadati_completi_supera_la_prevalidazione() {
    let seme = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fuzz/seeds/ipc_reader/footer-con-metadati-validi.arrow");
    assert!(seme.is_file(), "seme assente: {}", seme.display());
    driver_common::prevalida_arrow::valida_file_ipc("arrow", &seme)
        .expect("un footer con metadati completi non deve essere rifiutato");
}

/// I metadati di un campo geometria conforme ad Arrow Geometry Semantics 1.0:
/// l'estensione e le sette chiavi obbligatorie, con il CRS dichiarato
/// `missing`. Le fixture che non provano il vocabolario partono da qui e
/// cambiano solo cio' che provano.
fn metadati_geometria_conformi() -> arrow_schema::Metadata {
    use plenora_io_model::geometry::{
        ARROW_EXTENSION_NAME_KEY, GEOARROW_WKB_EXTENSION, PLENORA_CRS_RESOLUTION_KEY,
        PLENORA_DIMENSIONS_KEY, PLENORA_ENCODING_KEY, PLENORA_FIELD_ID_KEY, PLENORA_PRECISION_KEY,
        PLENORA_SPATIAL_SEMANTICS_KEY, PLENORA_TYPES_DECLARATION_KEY,
    };
    [
        (ARROW_EXTENSION_NAME_KEY, GEOARROW_WKB_EXTENSION),
        (PLENORA_FIELD_ID_KEY, "0"),
        (PLENORA_ENCODING_KEY, "wkb"),
        (PLENORA_DIMENSIONS_KEY, "xy"),
        (PLENORA_SPATIAL_SEMANTICS_KEY, "geometry"),
        (PLENORA_PRECISION_KEY, "float64"),
        (PLENORA_TYPES_DECLARATION_KEY, "mixed"),
        (PLENORA_CRS_RESOLUTION_KEY, "missing"),
    ]
    .into_iter()
    .map(|(chiave, valore)| (chiave.to_owned(), valore.to_owned()))
    .collect()
}

/// Un file IPC senza batch con lo schema dato, scritto da arrow e non da IO:
/// e' l'ingresso di un produttore qualunque.
fn ipc_vuoto(path: &std::path::Path, schema: &Schema) {
    let file = File::create(path).unwrap();
    let mut writer = FileWriter::try_new(file, schema).unwrap();
    writer.finish().unwrap();
}

/// La categoria del rifiuto all'apertura; `None` se il file si apre.
fn categoria_in_apertura(path: std::path::PathBuf) -> Option<plenora_io_model::ErrorCategory> {
    IpcDriver
        .open(Source::Path(path), opzioni_lettura())
        .err()
        .map(|errore| errore.category)
}

/// Una geometria senza CRS: un Arrow esterno con la sola estensione e uno
/// schema plenora che dichiara `crs_resolution=missing` si leggono `Missing`;
/// uno schema plenora con la sola estensione non e' un'assenza dichiarata ma
/// un vocabolario incompleto, e si rifiuta con `schema` (REJ-001).
#[test]
fn geometry_without_crs_metadata_is_explicitly_missing() {
    let dir = tempfile::tempdir().unwrap();
    let solo_estensione: arrow_schema::Metadata = std::iter::once((
        plenora_io_model::geometry::ARROW_EXTENSION_NAME_KEY.to_owned(),
        plenora_io_model::geometry::GEOARROW_WKB_EXTENSION.to_owned(),
    ))
    .collect();

    for (nome, schema) in [
        (
            "esterno",
            Schema::new(vec![Field::new("geometry", DataType::Binary, true)
                .with_metadata(solo_estensione.clone())]),
        ),
        (
            "plenora-missing",
            with_contract_version(Arc::new(Schema::new(vec![Field::new(
                "geometry",
                DataType::Binary,
                true,
            )
            .with_metadata(metadati_geometria_conformi())])))
            .as_ref()
            .clone(),
        ),
    ] {
        let path = dir.path().join(format!("{nome}.arrow"));
        ipc_vuoto(&path, &schema);
        let dataset = IpcDriver
            .open(Source::Path(path), opzioni_lettura())
            .unwrap_or_else(|errore| panic!("{nome}: {errore}"));
        assert!(
            matches!(
                &dataset.layers()[0].contract.geometry.as_ref().unwrap().crs,
                CrsResolution::Missing
            ),
            "{nome}"
        );
    }

    let path = dir.path().join("plenora-solo-estensione.arrow");
    let schema = with_contract_version(Arc::new(Schema::new(vec![Field::new(
        "geometry",
        DataType::Binary,
        true,
    )
    .with_metadata(solo_estensione)])));
    ipc_vuoto(&path, &schema);
    assert_eq!(
        categoria_in_apertura(path),
        Some(plenora_io_model::ErrorCategory::Schema)
    );
}

/// IPC non usa `--assume-crs`: con una geometria senza CRS l'opzione restava
/// senza effetto, e la lettura riusciva come se non fosse stata data. Ora e'
/// un rifiuto `crs`, come un'opzione che contraddice il CRS dichiarato.
#[test]
fn assume_crs_su_una_geometria_senza_crs_non_e_ignorato() {
    let dir = tempfile::tempdir().unwrap();
    let solo_estensione: arrow_schema::Metadata = std::iter::once((
        plenora_io_model::geometry::ARROW_EXTENSION_NAME_KEY.to_owned(),
        plenora_io_model::geometry::GEOARROW_WKB_EXTENSION.to_owned(),
    ))
    .collect();
    let path = dir.path().join("senza-crs.arrow");
    ipc_vuoto(
        &path,
        &Schema::new(vec![
            Field::new("geometry", DataType::Binary, true).with_metadata(solo_estensione)
        ]),
    );
    let Err(errore) = IpcDriver.open(
        Source::Path(path),
        opzioni_lettura().with_assume_crs("EPSG:3003"),
    ) else {
        panic!("--assume-crs ignorato su una geometria senza CRS");
    };
    assert_eq!(errore.category, plenora_io_model::ErrorCategory::Crs);
}

/// Il limite dichiarato in `docs/PRODUCT.md` («Limitazioni di prodotto», IPC):
/// il CRS che un produttore `GeoArrow` mette **solo** in
/// `ARROW:extension:metadata` non e' letto, e la geometria risulta senza CRS.
///
/// La prova fissa il comportamento di oggi perche' cambiarlo -- leggerlo, o
/// rifiutare il file -- sia una decisione visibile e non una deriva: quando il
/// limite rientra, questa prova cambia insieme alla riga del documento.
///
/// Il limite riguarda l'Arrow **esterno**, senza versione e senza chiavi
/// `plenora.*`, come lo scrive un produttore `GeoArrow`. Lo stesso campo in
/// uno schema che si dichiara plenora non e' un CRS non letto: e' un
/// vocabolario incompleto, e si rifiuta con `schema` (REJ-001).
#[test]
fn crs_only_in_geoarrow_extension_metadata_is_read_as_missing() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("crs-solo-geoarrow.arrow");
    let field = Field::new("geometry", DataType::Binary, true).with_metadata(
        [
            (
                plenora_io_model::geometry::ARROW_EXTENSION_NAME_KEY.to_owned(),
                plenora_io_model::geometry::GEOARROW_WKB_EXTENSION.to_owned(),
            ),
            (
                "ARROW:extension:metadata".to_owned(),
                r#"{"crs":"EPSG:4326","crs_type":"authority_code"}"#.to_owned(),
            ),
        ]
        .into_iter()
        .collect::<arrow_schema::Metadata>(),
    );
    let schema = Schema::new(vec![field.clone()]);
    ipc_vuoto(&path, &schema);

    let dataset = IpcDriver
        .open(Source::Path(path), opzioni_lettura())
        .unwrap();
    assert!(matches!(
        &dataset.layers()[0].contract.geometry.as_ref().unwrap().crs,
        CrsResolution::Missing
    ));

    let dichiarato = dir.path().join("crs-solo-geoarrow-plenora.arrow");
    ipc_vuoto(
        &dichiarato,
        &with_contract_version(Arc::new(Schema::new(vec![field]))),
    );
    assert_eq!(
        categoria_in_apertura(dichiarato),
        Some(plenora_io_model::ErrorCategory::Schema)
    );
}

#[test]
fn unresolved_authority_without_definition_is_preserved() {
    use plenora_io_model::geometry::{
        ARROW_EXTENSION_NAME_KEY, GEOARROW_WKB_EXTENSION, PLENORA_AXIS_ORDER_KEY,
        PLENORA_CRS_DEFINITION_FORMAT_KEY, PLENORA_CRS_DEFINITION_KEY, PLENORA_CRS_ID_KEY,
        PLENORA_CRS_RESOLUTION_KEY, PLENORA_DIMENSIONS_KEY, PLENORA_ENCODING_KEY,
        PLENORA_FIELD_ID_KEY, PLENORA_PRECISION_KEY, PLENORA_SPATIAL_SEMANTICS_KEY,
        PLENORA_TYPES_DECLARATION_KEY,
    };

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("unresolved-authority.arrow");
    let field = Field::new("geometry", DataType::Binary, true).with_metadata(
        [
            (
                ARROW_EXTENSION_NAME_KEY.to_owned(),
                GEOARROW_WKB_EXTENSION.to_owned(),
            ),
            (PLENORA_FIELD_ID_KEY.to_owned(), "0".to_owned()),
            (
                PLENORA_SPATIAL_SEMANTICS_KEY.to_owned(),
                "geometry".to_owned(),
            ),
            (PLENORA_PRECISION_KEY.to_owned(), "float64".to_owned()),
            (
                PLENORA_CRS_RESOLUTION_KEY.to_owned(),
                "declared_unresolved".to_owned(),
            ),
            (PLENORA_ENCODING_KEY.to_owned(), "wkb".to_owned()),
            (PLENORA_DIMENSIONS_KEY.to_owned(), "xy".to_owned()),
            (
                PLENORA_TYPES_DECLARATION_KEY.to_owned(),
                "unresolved".to_owned(),
            ),
            (PLENORA_CRS_ID_KEY.to_owned(), "EPSG:99999".to_owned()),
            (PLENORA_AXIS_ORDER_KEY.to_owned(), "unknown".to_owned()),
        ]
        .into_iter()
        .collect::<arrow_schema::Metadata>(),
    );
    let schema = with_contract_version(Arc::new(Schema::new(vec![field])));
    {
        let file = File::create(&path).unwrap();
        let mut writer = FileWriter::try_new(file, schema.as_ref()).unwrap();
        writer.finish().unwrap();
    }

    let dataset = IpcDriver
        .open(Source::Path(path), opzioni_lettura())
        .unwrap();
    let geometry = dataset.layers()[0].contract.geometry.as_ref().unwrap();
    let raw = geometry.crs.raw().unwrap();
    assert_eq!(raw.authority_hint.as_deref(), Some("EPSG:99999"));
    assert_eq!(raw.definition, None);
    assert_eq!(raw.definition_format, None);

    let emitted =
        with_geometry_contract_metadata(&Field::new("geometry", DataType::Binary, true), geometry);
    assert!(!emitted.metadata().contains_key(PLENORA_CRS_DEFINITION_KEY));
    assert!(!emitted
        .metadata()
        .contains_key(PLENORA_CRS_DEFINITION_FORMAT_KEY));
}

/// Uno SRID da solo non e' una dichiarazione di CRS: `declared_unresolved`
/// chiede `crs_id` o `crs_definition`, e `axis_order` (ARROW-GEOMETRY-SEMANTICS
/// 1.0 §4, `_crs` dell'oracolo). Un tempo il file si leggeva e si riscriveva
/// senza sintetizzare un `crs_id` dallo SRID; ora non si legge e non si
/// produce, con la categoria `crs` in entrambe le direzioni.
///
/// Il vocabolario del campo e' completo: l'unico difetto e' quello del CRS,
/// e il rifiuto e' quindi della sua classe e non di quella dello schema.
#[test]
fn declared_unresolved_srid_only_is_rejected_on_read_and_on_write() {
    use plenora_io_model::contract::GeometryType;
    use plenora_io_model::crs::{AxisOrder, RawCrs};
    use plenora_io_model::geometry::{
        PLENORA_CRS_RESOLUTION_KEY, PLENORA_GEOMETRY_TYPES_KEY, PLENORA_SRID_KEY,
        PLENORA_TYPES_DECLARATION_KEY,
    };

    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("srid-only-input.arrow");
    let output = dir.path().join("srid-only-output.arrow");
    let mut metadata = metadati_geometria_conformi();
    for (chiave, valore) in [
        (PLENORA_TYPES_DECLARATION_KEY, "exact"),
        (PLENORA_GEOMETRY_TYPES_KEY, "point"),
        (PLENORA_CRS_RESOLUTION_KEY, "declared_unresolved"),
        (PLENORA_SRID_KEY, "4326"),
    ] {
        metadata.insert(chiave.to_owned(), valore.to_owned());
    }
    let field = Field::new("geom", DataType::Binary, true).with_metadata(metadata);
    let schema = with_contract_version(Arc::new(Schema::new(vec![field])));
    {
        let values = BinaryArray::from(vec![Some(
            &[
                1_u8, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            ][..],
        )]);
        let batch = RecordBatch::try_new(schema.clone(), vec![Arc::new(values)]).unwrap();
        let mut writer = FileWriter::try_new(File::create(&input).unwrap(), &schema).unwrap();
        writer.write(&batch).unwrap();
        writer.finish().unwrap();
    }
    assert_eq!(
        categoria_in_apertura(input),
        Some(plenora_io_model::ErrorCategory::Crs)
    );

    // La stessa dichiarazione costruita dal lato del produttore: IO non la
    // scrive, e non la "ripara" inventando un `crs_id` dallo SRID.
    let mut geometry = GeometryColumnContract::wkb_xy(
        FieldId(0),
        "geom",
        CrsResolution::DeclaredButUnresolved(RawCrs {
            definition: None,
            authority_hint: None,
            definition_format: None,
            axis_order: AxisOrder::Unknown,
        }),
        true,
    );
    geometry.srid = Some(4326);
    geometry.set_exact_geometry_types(vec![GeometryType::Point]);
    let plan = WritePlan {
        layers: vec![WriteLayer {
            name: "layer".to_owned(),
            contract: DataContract {
                schema: Arc::new(Schema::new(vec![Field::new(
                    "geom",
                    DataType::Binary,
                    true,
                )
                .with_metadata(
                    std::iter::once((
                        plenora_io_model::geometry::ARROW_EXTENSION_NAME_KEY.to_owned(),
                        plenora_io_model::geometry::GEOARROW_WKB_EXTENSION.to_owned(),
                    ))
                    .collect::<arrow_schema::Metadata>(),
                )])),
                geometry: Some(geometry),
            },
        }],
    };
    let Err(errore) = IpcDriver.create(Sink::Path(output.clone()), &plan, &opzioni_scrittura())
    else {
        panic!("uno SRID senza crs_id ne' definizione non si scrive");
    };
    assert_eq!(errore.category, plenora_io_model::ErrorCategory::Crs);
    assert!(!output.exists());
}

#[test]
fn declared_unresolved_srid_only_with_axis_order_fails_at_open() {
    use plenora_io_model::geometry::{
        ARROW_EXTENSION_NAME_KEY, GEOARROW_WKB_EXTENSION, PLENORA_AXIS_ORDER_KEY,
        PLENORA_CRS_RESOLUTION_KEY, PLENORA_DIMENSIONS_KEY, PLENORA_ENCODING_KEY,
        PLENORA_GEOMETRY_TYPES_KEY, PLENORA_SRID_KEY, PLENORA_TYPES_DECLARATION_KEY,
    };

    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("srid-only-with-axis.arrow");
    let field = Field::new("geom", DataType::Binary, true).with_metadata(
        [
            (
                ARROW_EXTENSION_NAME_KEY.to_owned(),
                GEOARROW_WKB_EXTENSION.to_owned(),
            ),
            (PLENORA_ENCODING_KEY.to_owned(), "wkb".to_owned()),
            (PLENORA_DIMENSIONS_KEY.to_owned(), "xy".to_owned()),
            (PLENORA_TYPES_DECLARATION_KEY.to_owned(), "exact".to_owned()),
            (PLENORA_GEOMETRY_TYPES_KEY.to_owned(), "point".to_owned()),
            (
                PLENORA_CRS_RESOLUTION_KEY.to_owned(),
                "declared_unresolved".to_owned(),
            ),
            (PLENORA_SRID_KEY.to_owned(), "4326".to_owned()),
            (PLENORA_AXIS_ORDER_KEY.to_owned(), "lon_lat".to_owned()),
        ]
        .into_iter()
        .collect::<arrow_schema::Metadata>(),
    );
    let schema = with_contract_version(Arc::new(Schema::new(vec![field])));
    let values = BinaryArray::from(vec![Some(
        &[
            1_u8, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        ][..],
    )]);
    let batch = RecordBatch::try_new(schema.clone(), vec![Arc::new(values)]).unwrap();
    let mut writer = FileWriter::try_new(File::create(&input).unwrap(), &schema).unwrap();
    writer.write(&batch).unwrap();
    writer.finish().unwrap();

    assert!(IpcDriver
        .open(Source::Path(input), opzioni_lettura())
        .is_err());
}

/// Chiavi `plenora.geometry.*` su un campo senza `geoarrow.wkb`: un tempo il
/// campo si riconosceva come geometria dalle sole chiavi canoniche; ora e' un
/// difetto di vocabolario (REJ-001), `schema`, con la versione e senza. Senza
/// versione lo schema porta comunque chiavi `plenora.*`, quindi non e' un
/// Arrow esterno ma uno schema plenora a cui manca la versione.
#[test]
fn canonical_metadata_without_geoarrow_extension_is_rejected() {
    use plenora_io_model::geometry::{
        PLENORA_AXIS_ORDER_KEY, PLENORA_CRS_ID_KEY, PLENORA_CRS_RESOLUTION_KEY,
        PLENORA_DIMENSIONS_KEY, PLENORA_ENCODING_KEY, PLENORA_TYPES_DECLARATION_KEY,
    };

    let dir = tempfile::tempdir().unwrap();
    let field = Field::new("geometry", DataType::Binary, true).with_metadata(
        [
            (PLENORA_ENCODING_KEY.to_owned(), "wkb".to_owned()),
            (PLENORA_DIMENSIONS_KEY.to_owned(), "xy".to_owned()),
            (PLENORA_CRS_RESOLUTION_KEY.to_owned(), "resolved".to_owned()),
            (PLENORA_CRS_ID_KEY.to_owned(), "EPSG:4326".to_owned()),
            (PLENORA_AXIS_ORDER_KEY.to_owned(), "lat_lon".to_owned()),
            (PLENORA_TYPES_DECLARATION_KEY.to_owned(), "mixed".to_owned()),
        ]
        .into_iter()
        .collect::<arrow_schema::Metadata>(),
    );
    for (nome, schema) in [
        (
            "con-versione",
            with_contract_version(Arc::new(Schema::new(vec![field.clone()])))
                .as_ref()
                .clone(),
        ),
        ("senza-versione", Schema::new(vec![field])),
    ] {
        let path = dir.path().join(format!("{nome}.arrow"));
        ipc_vuoto(&path, &schema);
        assert_eq!(
            categoria_in_apertura(path),
            Some(plenora_io_model::ErrorCategory::Schema),
            "{nome}"
        );
    }
}

/// Identita' canonica incompleta o in conflitto: rifiuto della classe del
/// vocabolario, `schema` (REJ-001). Prima la categoria era quella del
/// contratto; ora la decide la verifica dello schema, che viene prima.
#[test]
fn incomplete_or_conflicting_canonical_identity_is_rejected() {
    use plenora_io_model::geometry::{
        ARROW_EXTENSION_NAME_KEY, GEOARROW_WKB_EXTENSION, PLENORA_CRS_RESOLUTION_KEY,
        PLENORA_DIMENSIONS_KEY, PLENORA_ENCODING_KEY, PLENORA_TYPES_DECLARATION_KEY,
    };

    let parziali = || {
        [
            (PLENORA_ENCODING_KEY.to_owned(), "wkb".to_owned()),
            (PLENORA_DIMENSIONS_KEY.to_owned(), "xy".to_owned()),
            (PLENORA_CRS_RESOLUTION_KEY.to_owned(), "missing".to_owned()),
            (PLENORA_TYPES_DECLARATION_KEY.to_owned(), "mixed".to_owned()),
        ]
        .into_iter()
        .collect::<arrow_schema::Metadata>()
    };
    let con = |chiave: &str, valore: &str| {
        let mut metadata = parziali();
        metadata.insert(chiave.to_owned(), valore.to_owned());
        metadata
    };
    let mut completi_senza_versione = metadati_geometria_conformi();
    completi_senza_versione.insert(PLENORA_TYPES_DECLARATION_KEY.to_owned(), "mixed".to_owned());

    for (name, metadata, versione) in [
        ("missing-version", parziali(), false),
        ("complete-without-version", completi_senza_versione, false),
        (
            "conflicting-extension",
            con(ARROW_EXTENSION_NAME_KEY, "vendor.opaque"),
            true,
        ),
        (
            "incomplete-with-extension",
            con(ARROW_EXTENSION_NAME_KEY, GEOARROW_WKB_EXTENSION),
            true,
        ),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(format!("{name}.arrow"));
        let field = Field::new("geometry", DataType::Binary, true).with_metadata(metadata);
        let schema = Arc::new(Schema::new(vec![field]));
        let schema = if versione {
            with_contract_version(schema)
        } else {
            schema
        };
        ipc_vuoto(&path, &schema);
        assert_eq!(
            categoria_in_apertura(path),
            Some(plenora_io_model::ErrorCategory::Schema),
            "{name}"
        );
    }
}

#[test]
fn multiple_geoarrow_fields_are_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("ambiguous.arrow");
    let geometry_field = |name| {
        Field::new(name, DataType::Binary, true).with_metadata(
            std::iter::once((
                plenora_io_model::geometry::ARROW_EXTENSION_NAME_KEY.to_owned(),
                plenora_io_model::geometry::GEOARROW_WKB_EXTENSION.to_owned(),
            ))
            .collect::<arrow_schema::Metadata>(),
        )
    };
    let schema = Schema::new(vec![
        geometry_field("geometry_a"),
        geometry_field("geometry_b"),
    ]);
    {
        let file = File::create(&path).unwrap();
        let mut writer = FileWriter::try_new(file, &schema).unwrap();
        writer.finish().unwrap();
    }

    assert!(matches!(
        IpcDriver.open(Source::Path(path), opzioni_lettura()),
        Err(error) if error.code == plenora_io_model::IoErrorCode::Contract
    ));
}

#[test]
fn round_trip_ipc_preserves_geometry_metadata() {
    use driver_common_geometry_field as geometry_field;

    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("t.arrow");
    let wkb = to_wkb(&geo_types::Geometry::Point(geo_types::Point::new(
        12.5, 45.9,
    )))
    .unwrap();
    let schema: SchemaRef = Arc::new(Schema::new(vec![
        geometry_field("geometry", "EPSG:4326"),
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

    // Il contratto geometrico esplicito, come lo porta un piano reale (il
    // contratto del reader): e' da li' che il writer completa il vocabolario.
    // Senza contratto il campo resterebbe con la sola estensione e il `crs`,
    // e la guardia del produttore lo rifiuta (prova
    // `ipc_writer_rejects_a_non_conforming_plenora_schema_before_any_effect`).
    let mut geometry = GeometryColumnContract::wkb_passthrough(
        FieldId(0),
        "geometry",
        ResolvedCrs::new(Some("EPSG:4326".to_owned()), CrsKind::Geographic, None),
        true,
    );
    geometry.set_exact_geometry_types(vec![GeometryType::Point]);
    let driver = IpcDriver;
    let plan = WritePlan {
        layers: vec![WriteLayer {
            name: "l".to_owned(),
            contract: DataContract {
                schema,
                geometry: Some(geometry),
            },
        }],
    };
    let mut w = driver
        .create(Sink::Path(out.clone()), &plan, &opzioni_scrittura())
        .unwrap();
    w.write(&batch).unwrap();
    w.finish().unwrap();

    let ds = driver.open(Source::Path(out), opzioni_lettura()).unwrap();
    // Il CRS e la geometria sopravvivono nei metadati Arrow (pass-through).
    let g = ds.layers()[0].contract.geometry.as_ref().unwrap();
    assert_eq!(g.name, "geometry");
    assert_eq!(g.crs.id(), Some("EPSG:4326"));
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
    assert!(is_geometry_field(
        &rb.schema().field_with_name("geometry").unwrap().clone()
    ));
    let col = rb
        .column_by_name("geometry")
        .unwrap()
        .as_any()
        .downcast_ref::<BinaryArray>()
        .unwrap();
    assert_eq!(col.value(0), wkb.as_slice());
    assert!(r.next_batch().unwrap().is_none());

    let mut projected = ds
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
    assert_eq!(projected.contract().contract.schema.fields().len(), 1);
    assert_eq!(projected.contract().contract.schema.field(0).name(), "id");
    assert!(projected.contract().contract.geometry.is_none());
    let projected_batch = projected.next_batch().unwrap().unwrap();
    assert_eq!(projected_batch.num_columns(), 1);
    assert_eq!(
        projected_batch
            .column(0)
            .as_any()
            .downcast_ref::<Int64Array>()
            .unwrap()
            .value(0),
        7
    );
}

/// La guardia del produttore: il writer IPC aggiunge `plenora.contract.version`
/// e `plenora.field_id`, quindi cio' che scrive si dichiara plenora e deve
/// essere conforme. Uno schema che IO stesso rifiuterebbe in lettura si
/// rifiuta in `create`, con la categoria della sua classe, prima dello
/// staging: nessuna destinazione e nessun residuo.
///
/// Senza la guardia `create` riusciva e il file si pubblicava; il rifiuto
/// arrivava solo a chi lo rileggeva.
#[test]
fn ipc_writer_rejects_a_non_conforming_plenora_schema_before_any_effect() {
    use driver_common_geometry_field as geometry_field;
    use plenora_io_model::geometry::PLENORA_ENCODING_KEY;

    let casi = [
        // Un campo `geoarrow.wkb` senza contratto geometrico: il vocabolario
        // resta quello del campo, senza le chiavi obbligatorie.
        (
            "geometria-senza-contratto",
            Schema::new(vec![
                geometry_field("geometry", "EPSG:4326"),
                Field::new("id", DataType::Int64, false),
            ]),
        ),
        // Una chiave `plenora.geometry.*` su un campo che non e' geometria.
        (
            "chiave-geometrica-su-attributo",
            Schema::new(vec![Field::new("id", DataType::Int64, false)
                .with_metadata(
                    std::iter::once((PLENORA_ENCODING_KEY.to_owned(), "wkb".to_owned()))
                        .collect::<arrow_schema::Metadata>(),
                )]),
        ),
    ];
    for (nome, schema) in casi {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join(format!("{nome}.arrow"));
        let plan = WritePlan {
            layers: vec![WriteLayer {
                name: "l".to_owned(),
                contract: DataContract {
                    schema: Arc::new(schema),
                    geometry: None,
                },
            }],
        };
        let Err(errore) = IpcDriver.create(Sink::Path(out.clone()), &plan, &opzioni_scrittura())
        else {
            panic!("{nome}: uno schema plenora non conforme non si scrive");
        };
        assert_eq!(
            errore.category,
            plenora_io_model::ErrorCategory::Schema,
            "{nome}: {errore}"
        );
        assert!(!out.exists(), "{nome}: nessuna destinazione");
        assert_eq!(
            std::fs::read_dir(dir.path()).unwrap().count(),
            0,
            "{nome}: nessun residuo di staging"
        );
    }
}

#[test]
fn batch_target_slices_file_defined_ipc_batches() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("batch-target.arrow");
    let schema: SchemaRef = Arc::new(Schema::new(vec![Field::new("id", DataType::Int64, false)]));
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![Arc::new(Int64Array::from(vec![0, 1, 2, 3, 4]))],
    )
    .unwrap();
    let plan = WritePlan {
        layers: vec![WriteLayer {
            name: "layer".to_owned(),
            contract: DataContract {
                schema,
                geometry: None,
            },
        }],
    };
    let driver = IpcDriver;
    let mut writer = driver
        .create(Sink::Path(out.clone()), &plan, &opzioni_scrittura())
        .unwrap();
    writer.write(&batch).unwrap();
    writer.finish().unwrap();

    let dataset = driver.open(Source::Path(out), opzioni_lettura()).unwrap();
    let mut reader = dataset
        .open_layer_reader(&ReadRequest {
            layer: LayerId(0),
            projected_fields: None,
            projection_mode: ProjectionMode::BestEffort,
            pruning_predicate: None,
            spatial_pruning_hint: None,
            scope: ReadScope::default(),
            batch_target: BatchTarget {
                target_bytes: 16,
                max_rows: 100,
            },
            cancellation: CancellationToken::default(),
        })
        .unwrap();
    let mut sizes = Vec::new();
    while let Some(batch) = reader.next_batch().unwrap() {
        sizes.push(batch.num_rows());
    }
    assert_eq!(sizes, vec![2, 2, 1]);
}

#[test]
fn round_trip_ipc_preserves_ewkb_zm_contract_and_bytes() {
    use driver_common_geometry_field as geometry_field;

    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("zm.arrow");
    let ewkb = encode_wkb(
        &WkbGeometry {
            value: WkbValue::Point(WkbCoordinate {
                x: 1.0,
                y: 2.0,
                z: Some(3.0),
                m: Some(4.0),
            }),
            dimensions: CoordinateDimensions::Xyzm,
            srid: Some(4326),
        },
        WkbFlavor::Ewkb,
    )
    .unwrap();
    let schema: SchemaRef = Arc::new(Schema::new(vec![geometry_field("geometry", "EPSG:4326")]));
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![Arc::new(BinaryArray::from(vec![Some(ewkb.as_slice())]))],
    )
    .unwrap();
    let mut geometry = GeometryColumnContract::wkb_passthrough(
        FieldId(0),
        "geometry",
        ResolvedCrs::new(Some("EPSG:4326".to_owned()), CrsKind::Geographic, None),
        true,
    );
    geometry.encoding = GeometryEncoding::Ewkb;
    geometry.dimensions = CoordinateDimensions::Xyzm;
    geometry.spatial_semantics = SpatialSemantics::Geography;
    geometry.srid = Some(4326);
    geometry.precision = CoordinatePrecision::Native;
    geometry.set_exact_geometry_types(vec![GeometryType::Point]);
    geometry.native_metadata.insert(
        "postgis.typmod".to_owned(),
        "geography(PointZM,4326)".to_owned(),
    );
    let plan = WritePlan {
        layers: vec![WriteLayer {
            name: "l".to_owned(),
            contract: DataContract {
                schema,
                geometry: Some(geometry),
            },
        }],
    };
    let driver = IpcDriver;
    let mut writer = driver
        .create(Sink::Path(out.clone()), &plan, &opzioni_scrittura())
        .unwrap();
    writer.write(&batch).unwrap();
    writer.finish().unwrap();

    let dataset = driver.open(Source::Path(out), opzioni_lettura()).unwrap();
    let geometry = dataset.layers()[0].contract.geometry.as_ref().unwrap();
    assert_eq!(geometry.encoding, GeometryEncoding::Ewkb);
    assert_eq!(geometry.dimensions, CoordinateDimensions::Xyzm);
    assert_eq!(geometry.spatial_semantics, SpatialSemantics::Geography);
    assert_eq!(geometry.srid, Some(4326));
    assert_eq!(geometry.precision, CoordinatePrecision::Native);
    assert_eq!(geometry.geometry_types, vec![GeometryType::Point]);
    assert_eq!(
        geometry
            .native_metadata
            .get("postgis.typmod")
            .map(String::as_str),
        Some("geography(PointZM,4326)")
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
    let read = reader.next_batch().unwrap().unwrap();
    let geometry_array = read
        .column(0)
        .as_any()
        .downcast_ref::<BinaryArray>()
        .unwrap();
    assert_eq!(geometry_array.value(0), ewkb);
}

/// Uno schema plenora a cui mancano chiavi obbligatorie -- qui
/// `plenora.field_id`, `spatial_semantics` e `precision` -- non si completa:
/// si rifiuta con `schema` (REJ-001). Un tempo IO le aggiungeva in lettura;
/// il completamento resta del solo Arrow esterno, e un passaggio fra
/// componenti che si dichiara plenora vale per il contratto intero.
#[test]
fn incomplete_canonical_metadata_is_rejected_not_completed() {
    use plenora_io_model::geometry::{
        PLENORA_FIELD_ID_KEY, PLENORA_PRECISION_KEY, PLENORA_SPATIAL_SEMANTICS_KEY,
    };

    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("physical.arrow");
    let mut metadata = metadati_geometria_conformi();
    for chiave in [
        PLENORA_FIELD_ID_KEY,
        PLENORA_PRECISION_KEY,
        PLENORA_SPATIAL_SEMANTICS_KEY,
    ] {
        assert!(metadata.remove(chiave).is_some(), "{chiave}");
    }
    ipc_vuoto(
        &source,
        &with_contract_version(Arc::new(Schema::new(vec![Field::new(
            "geometry",
            DataType::Binary,
            false,
        )
        .with_metadata(metadata)]))),
    );
    assert_eq!(
        categoria_in_apertura(source),
        Some(plenora_io_model::ErrorCategory::Schema)
    );
}

/// Un ingresso conforme attraversa lettura e scrittura IPC con i valori e il
/// vocabolario invariati: ogni chiave `plenora.*` e ogni metadato del
/// produttore escono come sono entrati, e i byte WKB sono gli stessi.
#[test]
fn conforming_canonical_metadata_is_published_without_changing_values() {
    use plenora_io_model::geometry::{PLENORA_GEOMETRY_TYPES_KEY, PLENORA_TYPES_DECLARATION_KEY};

    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("physical.arrow");
    let output = dir.path().join("canonical.arrow");
    let mut metadata = metadati_geometria_conformi();
    metadata.insert(PLENORA_TYPES_DECLARATION_KEY.to_owned(), "exact".to_owned());
    metadata.insert(PLENORA_GEOMETRY_TYPES_KEY.to_owned(), "point".to_owned());
    metadata.insert("producer.normative".to_owned(), "retained".to_owned());
    let physical_schema = with_contract_version(Arc::new(Schema::new(vec![Field::new(
        "geometry",
        DataType::Binary,
        false,
    )
    .with_metadata(metadata.clone())])));
    let values = [
        geo_types::Point::new(1.0, 2.0),
        geo_types::Point::new(3.0, 4.0),
        geo_types::Point::new(5.0, 6.0),
    ]
    .map(|point| to_wkb(&geo_types::Geometry::Point(point)).unwrap());
    let physical_batch = RecordBatch::try_new(
        physical_schema.clone(),
        vec![Arc::new(BinaryArray::from_iter_values(
            values.iter().map(Vec::as_slice),
        ))],
    )
    .unwrap();
    {
        let file = File::create(&source).unwrap();
        let mut writer = FileWriter::try_new(file, physical_schema.as_ref()).unwrap();
        writer.write(&physical_batch).unwrap();
        writer.finish().unwrap();
    }

    let driver = IpcDriver;
    let dataset = driver
        .open(Source::Path(source), opzioni_lettura())
        .unwrap();
    let layer = dataset.layers()[0].clone();
    assert_eq!(layer.contract.schema.field(0).metadata(), &metadata);
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
    let read = reader.next_batch().unwrap().unwrap();
    assert_eq!(read.schema(), layer.contract.schema);

    let plan = WritePlan {
        layers: vec![WriteLayer {
            name: layer.name,
            contract: layer.contract,
        }],
    };
    let mut writer = driver
        .create(Sink::Path(output.clone()), &plan, &opzioni_scrittura())
        .unwrap();
    writer.write(&read).unwrap();
    writer.finish().unwrap();

    let mut published = FileReader::try_new(File::open(&output).unwrap(), None).unwrap();
    assert_eq!(published.schema().field(0).metadata(), &metadata);
    let published_batch = published.next().unwrap().unwrap();
    assert_eq!(published_batch.num_rows(), 3);
    let published_values = published_batch
        .column(0)
        .as_any()
        .downcast_ref::<BinaryArray>()
        .unwrap();
    for (row, expected) in values.iter().enumerate() {
        assert_eq!(published_values.value(row), expected);
    }
    // Cio' che IO pubblica, IO lo rilegge.
    assert_eq!(categoria_in_apertura(output), None);
}

// geometry_field locale (evita la dipendenza driver-common nei test).
fn driver_common_geometry_field(name: &str, crs: &str) -> Field {
    use std::collections::HashMap;
    let mut md = HashMap::new();
    md.insert(
        plenora_io_model::geometry::ARROW_EXTENSION_NAME_KEY.to_owned(),
        plenora_io_model::geometry::GEOARROW_WKB_EXTENSION.to_owned(),
    );
    md.insert(GEO_CRS_KEY.to_owned(), crs.to_owned());
    Field::new(name, DataType::Binary, true).with_metadata(md)
}
