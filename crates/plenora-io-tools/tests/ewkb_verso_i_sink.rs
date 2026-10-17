//! `io.write` e `io.convert` di un dataset EWKB verso un sink che scrive WKB ISO.
//!
//! EWKB e' l'unica forma che plenora-database-tools emette leggendo `PostGIS`, e
//! `ARROW-VOCABULARY-1.0` la ammette. Fino alla 4.1.1 ogni sink tranne Arrow
//! IPC la rifiutava con `encoding: ewkb` (la catena database -> IO della suite
//! di interoperabilita'). Ora si converte in WKB ISO little-endian con gli
//! stessi valori delle coordinate, bit per bit, se il SRID e' quello del CRS;
//! altrimenti e' un errore esplicito e la destinazione non esiste.
//!
//! Gli attesi sono costruiti **a mano**, byte per byte, e non con l'encoder del
//! prodotto: un encoder che sbagliasse allo stesso modo nei due versi
//! renderebbe la prova verde per costruzione.

use std::collections::HashMap;
use std::path::Path;
use std::process::Command;
use std::sync::Arc;

use arrow_array::{ArrayRef, BinaryArray, Int64Array, RecordBatch};
use arrow_schema::{DataType, Field, Schema};
use serde_json::Value;

const BINARIO: &str = env!("CARGO_BIN_EXE_plenora-io");

fn esegui(argomenti: &[&str]) -> (bool, Value) {
    let uscita = Command::new(BINARIO)
        .args(argomenti)
        .args(["--format", "json"])
        .output()
        .expect("il binario parte");
    let documento = serde_json::from_slice(&uscita.stdout).expect("stdout e' JSON");
    (uscita.status.success(), documento)
}

// --- byte scritti a mano ------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Ordine {
    Little,
    Big,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Dim {
    Xy,
    Xyz,
    Xym,
    Xyzm,
}

impl Dim {
    const fn ordinate(self) -> usize {
        match self {
            Self::Xy => 2,
            Self::Xyz | Self::Xym => 3,
            Self::Xyzm => 4,
        }
    }

    const fn nome(self) -> &'static str {
        match self {
            Self::Xy => "xy",
            Self::Xyz => "xyz",
            Self::Xym => "xym",
            Self::Xyzm => "xyzm",
        }
    }

    /// I flag EWKB di `PostGIS`.
    const fn flag_ewkb(self) -> u32 {
        match self {
            Self::Xy => 0,
            Self::Xyz => 0x8000_0000,
            Self::Xym => 0x4000_0000,
            Self::Xyzm => 0xC000_0000,
        }
    }

    /// Lo scarto ISO.
    const fn scarto_iso(self) -> u32 {
        match self {
            Self::Xy => 0,
            Self::Xyz => 1000,
            Self::Xym => 2000,
            Self::Xyzm => 3000,
        }
    }
}

fn u32_in(valore: u32, ordine: Ordine, out: &mut Vec<u8>) {
    out.extend_from_slice(&match ordine {
        Ordine::Little => valore.to_le_bytes(),
        Ordine::Big => valore.to_be_bytes(),
    });
}

fn f64_in(valore: f64, ordine: Ordine, out: &mut Vec<u8>) {
    out.extend_from_slice(&match ordine {
        Ordine::Little => valore.to_le_bytes(),
        Ordine::Big => valore.to_be_bytes(),
    });
}

/// Valori scelti perche' un arrotondamento si vedrebbe: -0.0, un subnormale,
/// un valore non rappresentabile esattamente in decimale.
const ORDINATE: [f64; 4] = [12.000_000_000_000_002, -0.0, 5e-324, 125.5];

/// L'intestazione di una geometria: ordine dei byte, parola di tipo, SRID.
fn testa(base: u32, dim: Dim, srid: Option<u32>, ordine: Ordine, iso: bool, out: &mut Vec<u8>) {
    out.push(match ordine {
        Ordine::Little => 1,
        Ordine::Big => 0,
    });
    let tipo = if iso {
        base + dim.scarto_iso()
    } else {
        base | dim.flag_ewkb() | if srid.is_some() { 0x2000_0000 } else { 0 }
    };
    u32_in(tipo, ordine, out);
    if let (Some(srid), false) = (srid, iso) {
        u32_in(srid, ordine, out);
    }
}

fn coordinata(dim: Dim, spostamento: f64, ordine: Ordine, out: &mut Vec<u8>) {
    for (i, valore) in ORDINATE.iter().take(dim.ordinate()).enumerate() {
        let valore = if i == 0 {
            valore + spostamento
        } else {
            *valore
        };
        f64_in(valore, ordine, out);
    }
}

/// Un punto, EWKB (con SRID) o ISO.
fn punto(dim: Dim, srid: Option<u32>, ordine: Ordine, iso: bool, spostamento: f64) -> Vec<u8> {
    let mut out = Vec::new();
    testa(1, dim, srid, ordine, iso, &mut out);
    coordinata(dim, spostamento, ordine, &mut out);
    out
}

/// Un multipunto di due punti: il padre porta `srid_padre`, i figli
/// `srid_figlio` (EWKB) o nulla (ISO).
fn multipunto(
    dim: Dim,
    srid_padre: Option<u32>,
    srid_figlio: Option<u32>,
    ordine: Ordine,
    iso: bool,
) -> Vec<u8> {
    let mut out = Vec::new();
    testa(4, dim, srid_padre, ordine, iso, &mut out);
    u32_in(2, ordine, &mut out);
    for spostamento in [0.0, 1.0] {
        testa(1, dim, srid_figlio, ordine, iso, &mut out);
        coordinata(dim, spostamento, ordine, &mut out);
    }
    out
}

// --- i dataset ----------------------------------------------------------------

/// Il dataset come lo consegna database: `encoding=ewkb`, SRID e CRS, una
/// geometria per riga, uno o piu' batch.
fn dataset(
    percorso: &Path,
    dim: Dim,
    tipi: &str,
    srid_metadati: &str,
    crs_id: &str,
    batch_di_geometrie: &[Vec<Option<Vec<u8>>>],
) {
    let metadati: HashMap<String, String> = [
        ("ARROW:extension:name", "geoarrow.wkb"),
        ("plenora.field_id", "2"),
        ("plenora.geometry.encoding", "ewkb"),
        ("plenora.geometry.dimensions", dim.nome()),
        ("plenora.geometry.spatial_semantics", "geometry"),
        ("plenora.geometry.precision", "float64"),
        ("plenora.geometry.types_declaration", "exact"),
        ("plenora.geometry.types", tipi),
        ("plenora.geometry.crs_resolution", "resolved"),
        ("plenora.geometry.crs_id", crs_id),
        ("plenora.geometry.axis_order", "lon_lat"),
        ("plenora.geometry.srid", srid_metadati),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_owned(), v.to_owned()))
    .collect();
    let id_md: HashMap<String, String> =
        HashMap::from([("plenora.field_id".to_owned(), "1".to_owned())]);
    let schema = Arc::new(Schema::new_with_metadata(
        vec![
            Field::new("id", DataType::Int64, false).with_metadata(id_md),
            Field::new("geom", DataType::Binary, true).with_metadata(metadati),
        ],
        HashMap::from([("plenora.contract.version".to_owned(), "1".to_owned())]),
    ));
    let file = std::fs::File::create(percorso).expect("file");
    let mut scrittore = arrow_ipc::writer::FileWriter::try_new(file, &schema).expect("writer");
    let mut prossimo = 1_i64;
    for geometrie in batch_di_geometrie {
        let quanti = i64::try_from(geometrie.len()).expect("pochi");
        let colonne: Vec<ArrayRef> = vec![
            Arc::new(Int64Array::from(
                (prossimo..prossimo + quanti).collect::<Vec<_>>(),
            )),
            Arc::new(BinaryArray::from_iter(
                geometrie.iter().map(Option::as_deref),
            )),
        ];
        prossimo += quanti;
        let batch = RecordBatch::try_new(Arc::clone(&schema), colonne).expect("batch");
        scrittore.write(&batch).expect("scrittura");
    }
    scrittore.finish().expect("chiusura");
}

/// Le geometrie di un file riletto da `plenora-io read --output`.
fn geometrie_rilette(sorgente: &Path, cartella: &Path) -> Vec<Option<Vec<u8>>> {
    let riletto = cartella.join("riletto.arrow");
    let _ = std::fs::remove_file(&riletto);
    let (riuscita, documento) = esegui(&[
        "read",
        sorgente.to_str().unwrap(),
        "--output",
        riletto.to_str().unwrap(),
    ]);
    assert!(riuscita, "{documento}");
    let file = std::fs::File::open(&riletto).expect("riletto");
    let lettore = arrow_ipc::reader::FileReader::try_new(file, None).expect("ipc");
    let schema = lettore.schema();
    let indice = schema
        .fields()
        .iter()
        .position(|f| f.metadata().get("plenora.geometry.encoding").is_some())
        .expect("colonna geometria");
    let mut valori = Vec::new();
    for batch in lettore {
        let batch = batch.expect("batch");
        let colonna = batch
            .column(indice)
            .as_any()
            .downcast_ref::<BinaryArray>()
            .expect("binaria")
            .clone();
        valori.extend(colonna.iter().map(|v| v.map(<[u8]>::to_vec)));
    }
    valori
}

type Comando = fn(&Path, &Path) -> (bool, Value);

fn scrivi(ingresso: &Path, uscita: &Path) -> (bool, Value) {
    esegui(&[
        "write",
        ingresso.to_str().unwrap(),
        uscita.to_str().unwrap(),
        "--to",
        "gpkg",
    ])
}

fn converti(ingresso: &Path, uscita: &Path) -> (bool, Value) {
    esegui(&[
        "convert",
        ingresso.to_str().unwrap(),
        uscita.to_str().unwrap(),
        "--from",
        "ipc",
        "--to",
        "gpkg",
    ])
}

const COMANDI: [(&str, Comando); 2] = [("write", scrivi), ("convert", converti)];

// --- le prove -------------------------------------------------------------------

/// XY, XYZ, XYM e XYZM, in little e big endian: l'uscita e' WKB ISO
/// little-endian con gli **stessi valori** delle coordinate, bit per bit.
///
/// Da un EWKB big-endian cambia anche l'ordine dei byte: i valori si
/// conservano, i byte no. L'atteso e' scritto a mano.
#[test]
fn ogni_dimensione_e_ogni_ordine_dei_byte_arrivano_con_gli_stessi_valori() {
    let temporanea = tempfile::tempdir().expect("directory temporanea");
    for dim in [Dim::Xy, Dim::Xyz, Dim::Xym, Dim::Xyzm] {
        for ordine in [Ordine::Little, Ordine::Big] {
            let caso = format!("{}-{ordine:?}", dim.nome());
            let ingresso = temporanea.path().join(format!("{caso}.arrow"));
            let geometrie = vec![
                Some(punto(dim, Some(4326), ordine, false, 0.0)),
                None,
                Some(punto(dim, Some(4326), ordine, false, 1.0)),
            ];
            dataset(&ingresso, dim, "point", "4326", "EPSG:4326", &[geometrie]);
            let gpkg = temporanea.path().join(format!("{caso}.gpkg"));
            let (riuscita, documento) = scrivi(&ingresso, &gpkg);
            assert!(riuscita, "{caso}: {documento}");
            assert_eq!(documento["result"]["rows_written"], 3, "{caso}");
            assert_eq!(
                geometrie_rilette(&gpkg, temporanea.path()),
                vec![
                    Some(punto(dim, None, Ordine::Little, true, 0.0)),
                    None,
                    Some(punto(dim, None, Ordine::Little, true, 1.0)),
                ],
                "{caso}: WKB ISO little-endian con gli stessi valori"
            );
        }
    }
}

/// I figli di un aggregato portano il proprio SRID (o nessuno): se coincide
/// con quello dichiarato si toglie, e l'aggregato arriva ISO.
#[test]
fn un_aggregato_con_lo_srid_ripetuto_nei_figli_arriva_iso() {
    let temporanea = tempfile::tempdir().expect("directory temporanea");
    for (indice, srid_figlio) in [None, Some(4326)].into_iter().enumerate() {
        let ingresso = temporanea.path().join(format!("multi-{indice}.arrow"));
        let ewkb = multipunto(Dim::Xyz, Some(4326), srid_figlio, Ordine::Big, false);
        dataset(
            &ingresso,
            Dim::Xyz,
            "multipoint",
            "4326",
            "EPSG:4326",
            &[vec![Some(ewkb)]],
        );
        let gpkg = temporanea.path().join(format!("multi-{indice}.gpkg"));
        let (riuscita, documento) = scrivi(&ingresso, &gpkg);
        assert!(riuscita, "{documento}");
        assert_eq!(
            geometrie_rilette(&gpkg, temporanea.path()),
            vec![Some(multipunto(Dim::Xyz, None, None, Ordine::Little, true))]
        );
    }
}

/// Un figlio con un SRID diverso sotto un padre corretto: errore, e nessuna
/// destinazione, per `write` e per `convert`.
#[test]
fn un_figlio_con_un_altro_srid_e_un_errore() {
    let temporanea = tempfile::tempdir().expect("directory temporanea");
    let ingresso = temporanea.path().join("figlio.arrow");
    let ewkb = multipunto(Dim::Xy, Some(4326), Some(3003), Ordine::Little, false);
    dataset(
        &ingresso,
        Dim::Xy,
        "multipoint",
        "4326",
        "EPSG:4326",
        &[vec![Some(ewkb)]],
    );
    for (nome, comando) in COMANDI {
        let gpkg = temporanea.path().join(format!("figlio-{nome}.gpkg"));
        let (riuscita, documento) = comando(&ingresso, &gpkg);
        assert!(!riuscita, "{nome}: {documento}");
        assert!(!gpkg.exists(), "{nome}: nessuna destinazione su un rifiuto");
    }
}

/// Un fallimento a meta': il primo batch e' valido e viene scritto, il
/// secondo porta un SRID diverso. La pubblicazione e' atomica: nessuna
/// destinazione, neppure parziale, e nessun residuo accanto.
#[test]
fn un_fallimento_dopo_batch_gia_scritti_non_lascia_la_destinazione() {
    let temporanea = tempfile::tempdir().expect("directory temporanea");
    let ingresso = temporanea.path().join("tardivo.arrow");
    let buono = (0..3)
        .map(|i| {
            Some(punto(
                Dim::Xy,
                Some(4326),
                Ordine::Little,
                false,
                f64::from(i),
            ))
        })
        .collect();
    let cattivo = vec![Some(punto(Dim::Xy, Some(3003), Ordine::Little, false, 0.0))];
    dataset(
        &ingresso,
        Dim::Xy,
        "point",
        "4326",
        "EPSG:4326",
        &[buono, cattivo],
    );
    for (nome, comando) in COMANDI {
        let gpkg = temporanea.path().join(format!("tardivo-{nome}.gpkg"));
        let (riuscita, documento) = comando(&ingresso, &gpkg);
        assert!(!riuscita, "{nome}: {documento}");
        assert!(
            !gpkg.exists(),
            "{nome}: nessuna destinazione dopo batch gia' scritti"
        );
        let residui: Vec<String> = std::fs::read_dir(temporanea.path())
            .expect("cartella")
            .filter_map(Result::ok)
            .map(|voce| voce.file_name().to_string_lossy().into_owned())
            .filter(|nome_file| nome_file.contains(&format!("tardivo-{nome}")))
            .collect();
        assert!(residui.is_empty(), "{nome}: residui di staging {residui:?}");
    }
}

/// SRID dei metadati diverso dal CRS: rifiuto esplicito, per `write` e per
/// `convert`, prima di creare la destinazione.
#[test]
fn un_srid_diverso_dal_crs_e_un_errore_esplicito() {
    let temporanea = tempfile::tempdir().expect("directory temporanea");
    let ingresso = temporanea.path().join("discorde.arrow");
    let geometrie = vec![Some(punto(Dim::Xy, Some(3003), Ordine::Little, false, 0.0))];
    dataset(
        &ingresso,
        Dim::Xy,
        "point",
        "3003",
        "EPSG:4326",
        &[geometrie],
    );
    for (nome, comando) in COMANDI {
        let gpkg = temporanea.path().join(format!("discorde-{nome}.gpkg"));
        let (riuscita, documento) = comando(&ingresso, &gpkg);
        assert!(!riuscita, "{nome}: {documento}");
        assert!(!gpkg.exists(), "{nome}: nessuna destinazione su un rifiuto");
        let messaggio = documento["error"]["message"]
            .as_str()
            .expect("la busta d'errore porta un messaggio");
        assert!(
            messaggio.contains("SRID EWKB diverso dal CRS"),
            "{nome}: il rifiuto nomina la discordanza: {documento}"
        );
    }
}

/// Il payload porta un SRID diverso da quello dei metadati: errore, per
/// `write` e per `convert`.
#[test]
fn un_payload_con_un_altro_srid_e_un_errore_esplicito() {
    let temporanea = tempfile::tempdir().expect("directory temporanea");
    let ingresso = temporanea.path().join("payload.arrow");
    let geometrie = vec![Some(punto(Dim::Xy, Some(3003), Ordine::Big, false, 0.0))];
    dataset(
        &ingresso,
        Dim::Xy,
        "point",
        "4326",
        "EPSG:4326",
        &[geometrie],
    );
    for (nome, comando) in COMANDI {
        let gpkg = temporanea.path().join(format!("payload-{nome}.gpkg"));
        let (riuscita, documento) = comando(&ingresso, &gpkg);
        assert!(!riuscita, "{nome}: {documento}");
        assert!(!gpkg.exists(), "{nome}: nessuna destinazione su un rifiuto");
    }
}

/// `convert` da un IPC EWKB big-endian XYZM arriva con gli stessi valori.
#[test]
fn convert_da_ipc_ewkb_verso_geopackage() {
    let temporanea = tempfile::tempdir().expect("directory temporanea");
    let ingresso = temporanea.path().join("ewkb.arrow");
    let geometrie = vec![Some(punto(Dim::Xyzm, Some(4326), Ordine::Big, false, 0.0))];
    dataset(
        &ingresso,
        Dim::Xyzm,
        "point",
        "4326",
        "EPSG:4326",
        &[geometrie],
    );
    let gpkg = temporanea.path().join("convertito.gpkg");
    let (riuscita, documento) = converti(&ingresso, &gpkg);
    assert!(riuscita, "{documento}");
    assert_eq!(
        geometrie_rilette(&gpkg, temporanea.path()),
        vec![Some(punto(Dim::Xyzm, None, Ordine::Little, true, 0.0))]
    );
}
