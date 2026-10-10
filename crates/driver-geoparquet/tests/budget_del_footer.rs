//! Il tetto di memoria del decoder del footer copre **ogni** prenotazione, o
//! rifiuta cio' che non sa coprire.
//!
//! Due punti trovati in revisione su #47:
//!
//! * `RowGroupMetaDataBuilder::new` prenota un `ColumnChunkMetaData` per foglia
//!   dello schema prima di leggere un solo chunk, mentre l'addebito avveniva
//!   chunk per chunk: la capacita' intera passava fuori dal tetto. Ora si
//!   addebita tutta insieme, prima del costruttore, e non di nuovo per chunk;
//! * column index e offset index si decodificano con decoder senza tetto, e i
//!   loro byte si leggono prima: con un tetto la loro lettura e' rifiutata in
//!   modo esplicito, prima di chiedere un byte.

use std::sync::Arc;

use arrow_array::{ArrayRef, Int32Array, RecordBatch};
use arrow_schema::{DataType, Field, Schema};
use parquet::arrow::arrow_reader::{ArrowReaderOptions, ParquetRecordBatchReaderBuilder};
use parquet::arrow::ArrowWriter;
use parquet::file::metadata::{PageIndexPolicy, ParquetMetaDataOptions, ParquetMetaDataReader};

// --- Thrift compatto scritto a mano ------------------------------------------
//
// Il footer della prima prova non si ottiene da un writer: un row group senza
// `columns` non lo scrive nessuno. Lo si scrive campo per campo, con le sole
// forme che servono.

fn varint(mut n: u64, out: &mut Vec<u8>) {
    loop {
        let sette = u8::try_from(n & 0x7F).expect("sette bit");
        n >>= 7;
        if n == 0 {
            out.push(sette);
            return;
        }
        out.push(sette | 0x80);
    }
}

const fn zigzag(n: i64) -> u64 {
    ((n << 1) ^ (n >> 63)).cast_unsigned()
}

/// Intestazione di campo con delta corto: `delta << 4 | tipo`.
fn campo(delta: u8, tipo: u8, out: &mut Vec<u8>) {
    out.push((delta << 4) | tipo);
}

fn intero(delta: u8, tipo: u8, valore: i64, out: &mut Vec<u8>) {
    campo(delta, tipo, out);
    varint(zigzag(valore), out);
}

fn testo(delta: u8, valore: &str, out: &mut Vec<u8>) {
    campo(delta, 8, out);
    varint(valore.len() as u64, out);
    out.extend_from_slice(valore.as_bytes());
}

/// Intestazione di un elenco di struct (tipo elemento 12).
fn elenco_di_struct(quanti: u64, out: &mut Vec<u8>) {
    if quanti < 15 {
        out.push(u8::try_from(quanti << 4).expect("corto") | 0x0C);
    } else {
        out.push(0xFC);
        varint(quanti, out);
    }
}

/// `FileMetaData` con uno schema di `foglie` colonne INT32 e `gruppi` row
/// group **senza** il campo `columns`.
fn footer(foglie: u64, gruppi: u64) -> Vec<u8> {
    let mut out = Vec::new();
    intero(1, 5, 2, &mut out); // 1: version i32
    campo(1, 9, &mut out); // 2: schema list<SchemaElement>
    elenco_di_struct(foglie + 1, &mut out);
    // la radice: 4: name, 5: num_children
    testo(4, "schema", &mut out);
    intero(1, 5, i64::try_from(foglie).expect("foglie"), &mut out);
    out.push(0);
    for i in 0..foglie {
        intero(1, 5, 1, &mut out); // 1: type INT32
        intero(2, 5, 0, &mut out); // 3: repetition REQUIRED
        testo(1, &format!("c{i}"), &mut out); // 4: name
        out.push(0);
    }
    intero(1, 6, 0, &mut out); // 3: num_rows i64
    campo(1, 9, &mut out); // 4: row_groups list<RowGroup>
    elenco_di_struct(gruppi, &mut out);
    for _ in 0..gruppi {
        intero(2, 6, 0, &mut out); // 2: total_byte_size (columns assente)
        intero(1, 6, 0, &mut out); // 3: num_rows
        out.push(0);
    }
    out.push(0);
    out
}

fn decodifica(byte: &[u8], tetto: u64) -> Result<(), String> {
    let opzioni = ParquetMetaDataOptions::new().with_footer_memory_budget(tetto);
    ParquetMetaDataReader::decode_metadata_with_options(byte, Some(&opzioni))
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// Il tetto minimo con cui `byte` si decodifica, cercato per bisezione.
fn tetto_minimo(byte: &[u8]) -> u64 {
    let (mut basso, mut alto) = (0_u64, 1_u64 << 32);
    assert!(
        decodifica(byte, alto).is_ok(),
        "il footer si decodifica con 4 GiB"
    );
    while basso + 1 < alto {
        let medio = basso + (alto - basso) / 2;
        if decodifica(byte, medio).is_ok() {
            alto = medio;
        } else {
            basso = medio;
        }
    }
    alto
}

/// La capacita' del row group si addebita prima di essere prenotata.
///
/// Senza row group lo schema di 512 foglie si decodifica con il tetto
/// `minimo`. Un row group che non porta `columns` arriva al rifiuto
/// strutturale («columns» mancante) solo dopo che il decoder ha prenotato 512
/// `ColumnChunkMetaData`: il tetto minimo per arrivarci deve quindi superare
/// `minimo` di almeno quella prenotazione. Prima della correzione la superava
/// solo dei pochi byte del row group: la prenotazione stava fuori dal tetto.
#[test]
fn la_capacita_del_row_group_si_addebita_prima_del_costruttore() {
    let minimo = tetto_minimo(&footer(512, 0));
    let con_gruppo = footer(512, 1);
    let arriva_al_rifiuto_strutturale = |tetto: u64| match decodifica(&con_gruppo, tetto) {
        Ok(()) => panic!("un row group senza `columns` si decodifica"),
        Err(messaggio) => !messaggio.contains("exceeds the decoding memory budget"),
    };
    let (mut basso, mut alto) = (minimo, 1_u64 << 32);
    assert!(arriva_al_rifiuto_strutturale(alto));
    while basso + 1 < alto {
        let medio = basso + (alto - basso) / 2;
        if arriva_al_rifiuto_strutturale(medio) {
            alto = medio;
        } else {
            basso = medio;
        }
    }
    // 64 byte per chunk e' un minorante largo: `ColumnChunkMetaData` ne
    // occupa alcune centinaia.
    assert!(
        alto - minimo >= 512 * 64,
        "il row group arriva al decoder con {} byte di tetto oltre lo schema: la          prenotazione delle 512 colonne non e' stata addebitata",
        alto - minimo
    );
    let messaggio = decodifica(&con_gruppo, alto).expect_err("columns manca");
    assert!(messaggio.contains("columns"), "{messaggio}");
}

/// Niente doppio conteggio: un file vero con molte colonne si legge con lo
/// stesso tetto minimo che il suo footer richiede, una sola volta per chunk.
#[test]
fn un_file_vero_largo_si_legge_entro_il_tetto() {
    let campi: Vec<Field> = (0..64)
        .map(|i| Field::new(format!("c{i}"), DataType::Int32, false))
        .collect();
    let schema = Arc::new(Schema::new(campi));
    let colonne: Vec<ArrayRef> = (0..64)
        .map(|_| Arc::new(Int32Array::from(vec![1, 2, 3])) as ArrayRef)
        .collect();
    let batch = RecordBatch::try_new(Arc::clone(&schema), colonne).expect("batch");
    let mut byte = Vec::new();
    let mut scrittore = ArrowWriter::try_new(&mut byte, schema, None).expect("writer");
    scrittore.write(&batch).expect("scrittura");
    scrittore.close().expect("chiusura");
    let n = byte.len();
    let lunghezza = u32::from_le_bytes([byte[n - 8], byte[n - 7], byte[n - 6], byte[n - 5]]);
    let footer = &byte[n - 8 - lunghezza as usize..n - 8];
    let minimo = tetto_minimo(footer);
    assert!(decodifica(footer, minimo).is_ok());
    assert!(decodifica(footer, minimo - 1).is_err());
}

fn con_indici_di_pagina() -> Vec<u8> {
    let schema = Arc::new(Schema::new(vec![Field::new("x", DataType::Int32, false)]));
    let batch = RecordBatch::try_new(
        Arc::clone(&schema),
        vec![Arc::new(Int32Array::from((0..100).collect::<Vec<i32>>())) as ArrayRef],
    )
    .expect("batch");
    let mut byte = Vec::new();
    let mut scrittore = ArrowWriter::try_new(&mut byte, schema, None).expect("writer");
    scrittore.write(&batch).expect("scrittura");
    scrittore.close().expect("chiusura");
    byte
}

fn apri(byte: &[u8], opzioni: ArrowReaderOptions) -> Result<(), String> {
    let temporanea = tempfile::NamedTempFile::new().expect("file temporaneo");
    std::fs::write(temporanea.path(), byte).expect("il file si scrive");
    let file = std::fs::File::open(temporanea.path()).expect("il file si apre");
    ParquetRecordBatchReaderBuilder::try_new_with_options(file, opzioni)
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// Indici di pagina chiesti con un tetto: un errore esplicito, prima di
/// leggerli. Senza tetto si leggono come prima.
///
/// Prima della correzione la lettura riusciva: gli indici passavano da
/// `decode_column_index` e `decode_offset_index`, che non portano il tetto,
/// e i loro byte erano gia' letti in memoria.
#[test]
fn gli_indici_di_pagina_con_un_tetto_sono_rifiutati() {
    let byte = con_indici_di_pagina();
    for politica in [PageIndexPolicy::Optional, PageIndexPolicy::Required] {
        let senza_tetto = ArrowReaderOptions::new().with_page_index_policy(politica);
        assert_eq!(
            apri(&byte, senza_tetto),
            Ok(()),
            "senza tetto gli indici si leggono"
        );

        let con_tetto = ArrowReaderOptions::new()
            .with_page_index_policy(politica)
            .with_footer_memory_budget(1 << 30);
        let Err(messaggio) = apri(&byte, con_tetto) else {
            panic!("indici di pagina letti con un tetto che non li copre ({politica:?})");
        };
        assert!(
            messaggio.contains("page index requested with a footer memory budget"),
            "{messaggio}"
        );
    }
    // Solo l'offset index, la via veloce di `decode_offset_index`.
    let solo_offset = ArrowReaderOptions::new()
        .with_offset_index_policy(PageIndexPolicy::Required)
        .with_footer_memory_budget(1 << 30);
    assert!(apri(&byte, solo_offset).is_err());
    // Il driver li tiene a `Skip`: con il tetto la lettura resta possibile.
    let salta = ArrowReaderOptions::new()
        .with_page_index_policy(PageIndexPolicy::Skip)
        .with_footer_memory_budget(1 << 30);
    assert_eq!(apri(&byte, salta), Ok(()));
}

/// La stessa regola per il lettore di file seriale.
#[test]
fn anche_il_lettore_seriale_rifiuta_gli_indici_con_un_tetto() {
    use parquet::file::serialized_reader::{ReadOptionsBuilder, SerializedFileReader};

    let temporanea = tempfile::NamedTempFile::new().expect("file temporaneo");
    std::fs::write(temporanea.path(), con_indici_di_pagina()).expect("il file si scrive");
    let apri_file = || std::fs::File::open(temporanea.path()).expect("il file si apre");
    let opzioni = ReadOptionsBuilder::new()
        .with_page_index()
        .with_footer_memory_budget(1 << 30)
        .build();
    let esito = SerializedFileReader::new_with_options(apri_file(), opzioni).map(|_| ());
    let Err(errore) = esito else {
        panic!("indici di pagina letti con un tetto che non li copre");
    };
    assert!(
        errore
            .to_string()
            .contains("page index requested with a footer memory budget"),
        "{errore}"
    );
    let senza_tetto = ReadOptionsBuilder::new().with_page_index().build();
    assert!(SerializedFileReader::new_with_options(apri_file(), senza_tetto).is_ok());
}
