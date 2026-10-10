//! I delta del fork di `parquet` che questo crate usa, provati sul decoder.
//!
//! Il fork (`vendor/parquet`, registro `assurance/registries/vendor-parquet-fork.json`)
//! esiste perche' `parquet` dimensiona prenotazioni e indici sui valori che il
//! file dichiara. Ogni prova qui costruisce **un** file con una dichiarazione
//! falsa, chiama `parquet` direttamente -- senza niente di nostro in mezzo -- e
//! pretende un `Err`: ne' un panico, ne' una prenotazione dal conteggio
//! dichiarato.
//!
//! Le prove non misurano la memoria: un allocatore che conta richiederebbe
//! `unsafe`, che il workspace vieta. Pretendono invece il messaggio del
//! controllo che scatta **prima** della prenotazione: se il controllo sparisse,
//! il messaggio cambierebbe anche su una macchina con memoria a sufficienza.

use std::path::Path;
use std::sync::Arc;

use arrow_array::{ArrayRef, FixedSizeBinaryArray, Float64Array, RecordBatch};
use arrow_schema::{DataType, Field, Schema};
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use parquet::arrow::ArrowWriter;
use parquet::basic::Encoding;
use parquet::file::properties::{WriterProperties, WriterVersion};
use parquet::file::reader::SerializedFileReader;

/// Legge un Parquet dai byte: i metadati, poi tutti i batch.
///
/// `catch_unwind` e' dello strumento: distingue l'errore dal panico, che e'
/// precisamente cio' che si misura.
fn leggi(byte: &[u8]) -> Result<Result<usize, String>, String> {
    let temporanea = tempfile::NamedTempFile::new().expect("file temporaneo");
    std::fs::write(temporanea.path(), byte).expect("il file si scrive");
    let percorso = temporanea.path().to_path_buf();
    std::panic::catch_unwind(move || {
        let file = std::fs::File::open(&percorso).expect("il file si apre");
        let lettore = ParquetRecordBatchReaderBuilder::try_new(file)
            .map_err(|e| e.to_string())?
            .build()
            .map_err(|e| e.to_string())?;
        let mut righe = 0;
        for batch in lettore {
            righe += batch.map_err(|e| e.to_string())?.num_rows();
        }
        Ok(righe)
    })
    .map_err(|carico| {
        carico
            .downcast_ref::<String>()
            .cloned()
            .or_else(|| carico.downcast_ref::<&str>().map(|s| (*s).to_owned()))
            .unwrap_or_else(|| "(payload non testuale)".to_owned())
    })
}

fn scrivi(schema: Arc<Schema>, colonna: ArrayRef, proprieta: WriterProperties) -> Vec<u8> {
    let batch = RecordBatch::try_new(Arc::clone(&schema), vec![colonna]).expect("batch");
    let mut byte = Vec::new();
    let mut scrittore = ArrowWriter::try_new(&mut byte, schema, Some(proprieta)).expect("writer");
    scrittore.write(&batch).expect("scrittura");
    scrittore.close().expect("chiusura");
    byte
}

/// L'inizio del footer: la lunghezza sta negli otto byte finali, prima di `PAR1`.
fn inizio_del_footer(byte: &[u8]) -> usize {
    let n = byte.len();
    let lunghezza = u32::from_le_bytes([byte[n - 8], byte[n - 7], byte[n - 6], byte[n - 5]]);
    n - 8 - usize::try_from(lunghezza).expect("u32 in usize")
}

/// Il caso della 4.1.1: 3966 byte, un elenco `key_value_metadata` di circa 48
/// milioni di elementi, `malloc(2315255472)` con `parquet` 59.3.
#[test]
fn il_footer_della_4_1_1_e_un_errore_e_non_una_prenotazione() {
    let caso = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../upstream/parquet-footer-thrift-list/caso.parquet");
    let file = std::fs::File::open(caso).expect("il caso e' versionato");
    let esito = std::panic::catch_unwind(|| SerializedFileReader::new(file).map(|_| ()));
    match esito {
        Ok(Err(errore)) => {
            let testo = errore.to_string();
            assert!(
                testo.contains("exceeds remaining input length"),
                "il rifiuto atteso e' il limite di `read_thrift_vec` ai byte \
                 residui, prima della prenotazione: {testo}"
            );
        }
        Ok(Ok(())) => panic!("il footer del caso si legge senza errore"),
        Err(_) => panic!("il footer del caso fa panicare il decoder"),
    }
}

/// Un elenco di row group che dichiara piu' elementi dei byte che restano.
///
/// `parquet` 60.0.0 prenota i row group con `Vec::with_capacity` dal conteggio
/// dichiarato, senza passare dal limite di `read_thrift_vec`: e' la variante che
/// #38 lasciava aperta anche dopo l'aggiornamento. Il fork la chiude.
#[test]
fn row_group_dichiarati_oltre_i_byte_sono_un_errore() {
    let schema = Arc::new(Schema::new(vec![Field::new("x", DataType::Float64, false)]));
    let originale = scrivi(
        Arc::clone(&schema),
        Arc::new(Float64Array::from(vec![1.0, 2.0])),
        WriterProperties::builder().build(),
    );
    let inizio = inizio_del_footer(&originale);
    // `row_groups` e' il campo 4 di `FileMetaData`, dopo `num_rows` (campo 3):
    // intestazione 0x19 (delta 1, elenco), poi l'intestazione dell'elenco 0x1C
    // (un elemento, struct).
    let sito = (inizio..originale.len() - 9)
        .find(|&i| originale[i] == 0x19 && originale[i + 1] == 0x1C)
        .expect("l'elenco dei row group si trova nel footer");
    // Forma lunga: 0xFC (conteggio esplicito, struct) e il varint di 2^31 - 1.
    let mut alterato = originale[..=sito].to_vec();
    alterato.extend_from_slice(&[0xFC, 0xFF, 0xFF, 0xFF, 0xFF, 0x07]);
    alterato.extend_from_slice(&originale[sito + 2..]);
    // Il footer e' cresciuto di cinque byte: la sua lunghezza va riscritta.
    let n = alterato.len();
    let lunghezza = u32::from_le_bytes([
        alterato[n - 8],
        alterato[n - 7],
        alterato[n - 6],
        alterato[n - 5],
    ]) + 5;
    alterato[n - 8..n - 4].copy_from_slice(&lunghezza.to_le_bytes());

    match leggi(&alterato) {
        Ok(Err(testo)) => assert!(
            testo.contains("exceeds remaining input length"),
            "il rifiuto atteso e' il limite del fork sul conteggio dichiarato: {testo}"
        ),
        Ok(Ok(righe)) => panic!("il footer alterato si legge, {righe} righe"),
        Err(panico) => panic!("il footer alterato fa panicare il decoder: {panico}"),
    }
}

/// Un `FIXED_LEN_BYTE_ARRAY` di larghezza 0, in PLAIN e in BYTE_STREAM_SPLIT.
///
/// Lo schema Parquet ammette la larghezza 0, e il lettore Arrow di `parquet`
/// 60.0.0 ci divide sopra: `attempt to divide by zero`. Il file si ottiene
/// scrivendo una colonna di larghezza 1 e riscrivendo `type_length` nello
/// schema; i siti candidati sono tutti quelli con lo stesso valore, e nessuno
/// deve panicare.
#[test]
fn una_larghezza_zero_e_un_errore_e_non_una_divisione_per_zero() {
    let schema = Arc::new(Schema::new(vec![Field::new(
        "z",
        DataType::FixedSizeBinary(1),
        false,
    )]));
    let colonna: ArrayRef = Arc::new(
        FixedSizeBinaryArray::try_from_iter(vec![vec![7_u8]; 3].into_iter()).expect("colonna"),
    );
    for bss in [false, true] {
        let mut proprieta = WriterProperties::builder().set_dictionary_enabled(false);
        if bss {
            proprieta = proprieta
                .set_writer_version(WriterVersion::PARQUET_2_0)
                .set_encoding(Encoding::BYTE_STREAM_SPLIT);
        }
        let originale = scrivi(Arc::clone(&schema), Arc::clone(&colonna), proprieta.build());
        let inizio = inizio_del_footer(&originale);
        // Un i32 di valore 1 dopo un campo contiguo: 0x15, zigzag(1) = 0x02.
        let siti: Vec<usize> = (inizio..originale.len() - 9)
            .filter(|&i| originale[i] == 0x15 && originale[i + 1] == 0x02)
            .collect();
        let mut rifiutati = 0;
        for sito in &siti {
            let mut alterato = originale.clone();
            alterato[sito + 1] = 0x00;
            match leggi(&alterato) {
                Err(panico) => panic!("bss={bss}, sito {sito}: panico «{panico}»"),
                Ok(Err(testo)) if testo.contains("FIXED_LEN_BYTE_ARRAY width 0") => rifiutati += 1,
                Ok(_) => {}
            }
        }
        assert!(
            rifiutati > 0,
            "bss={bss}: nessuno dei siti {siti:?} e' `type_length`: il layout del \
             writer e' cambiato, e la prova va rifatta"
        );
    }
}
