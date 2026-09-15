//! Il riproduttore upstream: quale campo e' incoerente, dimostrato costruendolo.
//!
//! # Perche' serve un file sintetico
//!
//! Il seme del finding panica, ma il panico da solo non dimostra **quale**
//! incoerenza del formato lo causi: mostra un'indicizzazione fuori limite, e
//! una descrizione del file che non sia stata provata resta una supposizione.
//! Qui l'incoerenza si costruisce: si scrive un file **valido**, si verifica
//! che si legga, si altera **un solo campo**, e si verifica che quello basti.
//!
//! # Il campo
//!
//! `num_values` nell'header della data page. In `parquet-59.3.0`,
//! `ByteStreamSplitDecoder::get` ricava due quantita' da due fonti diverse:
//!
//! ```text
//! let stride = self.encoded_bytes.len() / type_size;      // byte EFFETTIVI
//! let num_values = buffer.len().min(self.values_left());  // da total_num_values, DICHIARATO
//! join_streams_const::<8>(&self.encoded_bytes, raw_out_bytes, stride, self.values_decoded)
//! ```
//!
//! e `join_streams_const` indicizza `sub_src[i + j * stride]` senza confrontare
//! l'indice con la lunghezza. Se il dichiarato eccede l'effettivo, il ciclo
//! chiede byte che non ci sono: `index out of bounds`.
//!
//! # Che cosa questo file non afferma
//!
//! Non afferma che il seme del finding abbia **esattamente** questa alterazione:
//! quel file viene da un fuzzer e puo' essere incoerente in piu' modi insieme.
//! Afferma che questa incoerenza, da sola, produce lo stesso panico nello stesso
//! punto -- che e' cio' che serve a una segnalazione upstream, e che il seme da
//! solo non poteva dimostrare.

use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use parquet::arrow::ArrowWriter;
use parquet::basic::Encoding;
use parquet::file::properties::{WriterProperties, WriterVersion};

use arrow_array::{ArrayRef, Float64Array, RecordBatch};
use arrow_schema::{DataType, Field, Schema};
use std::sync::Arc;

/// Un Parquet valido: una colonna `DOUBLE`, codifica `BYTE_STREAM_SPLIT`.
fn parquet_valido(valori: &[f64]) -> Vec<u8> {
    let schema = Arc::new(Schema::new(vec![Field::new("x", DataType::Float64, false)]));
    let colonna: ArrayRef = Arc::new(Float64Array::from(valori.to_vec()));
    let batch =
        RecordBatch::try_new(Arc::clone(&schema), vec![colonna]).expect("il batch si costruisce");

    // `WriterVersion::PARQUET_2_0` e' necessaria: BYTE_STREAM_SPLIT non e'
    // ammessa dalla v1, e il writer ricadrebbe su un'altra codifica -- il che
    // renderebbe il riproduttore un file che non esercita il decoder.
    let proprieta = WriterProperties::builder()
        .set_writer_version(WriterVersion::PARQUET_2_0)
        .set_dictionary_enabled(false)
        .set_encoding(Encoding::BYTE_STREAM_SPLIT)
        .build();

    let mut byte = Vec::new();
    let mut scrittore =
        ArrowWriter::try_new(&mut byte, schema, Some(proprieta)).expect("il writer si costruisce");
    scrittore.write(&batch).expect("la scrittura riesce");
    scrittore.close().expect("la chiusura riesce");
    byte
}

fn legge(byte: &[u8]) -> Result<usize, String> {
    // Si passa per un file invece che per un buffer: `bytes::Bytes` non e' fra
    // le dipendenze dichiarate di questo crate, e un file e' anche il modo in
    // cui un consumatore riproduce il caso.
    let temporanea = tempfile::NamedTempFile::new().expect("file temporaneo");
    std::fs::write(temporanea.path(), byte).expect("il file si scrive");
    let percorso = temporanea.path().to_path_buf();
    let esito = std::panic::catch_unwind(move || {
        let file = std::fs::File::open(&percorso).expect("il file si apre");
        let lettore = ParquetRecordBatchReaderBuilder::try_new(file)
            .expect("il footer si legge")
            .build()
            .expect("il lettore si costruisce");
        lettore
            .map(|batch| batch.expect("batch").num_rows())
            .sum::<usize>()
    });
    match esito {
        Ok(righe) => Ok(righe),
        Err(carico) => Err(carico
            .downcast_ref::<String>()
            .map(String::as_str)
            .or_else(|| carico.downcast_ref::<&str>().copied())
            .unwrap_or("(payload non testuale)")
            .to_owned()),
    }
}

/// Il file valido si legge: e' il controfattuale, e va prima dell'alterazione.
///
/// Senza, un panico sul file alterato non direbbe se la causa e' il campo o la
/// codifica stessa.
#[test]
fn il_file_valido_con_byte_stream_split_si_legge() {
    let valori: Vec<f64> = (0..8).map(|i| f64::from(i) + 0.5).collect();
    match legge(&parquet_valido(&valori)) {
        Ok(righe) => assert_eq!(
            righe,
            valori.len(),
            "il file valido deve restituire tutte le righe"
        ),
        Err(messaggio) => panic!(
            "il file valido panica: la codifica non e' utilizzabile per il \
             riproduttore, e il seguito non misurerebbe il campo: {messaggio}"
        ),
    }
}

/// Dichiarare piu' valori di quanti la pagina ne porti basta a far panicare.
///
/// L'alterazione e' **una sola**: il campo `num_values` dell'header della data
/// page, cercato come varint nel thrift compatto e aumentato. Tutto il resto
/// del file resta quello che il writer ha prodotto.
#[test]
fn num_values_dichiarato_oltre_i_byte_effettivi_fa_panicare_il_decoder() {
    // Otto valori: la pagina porta 64 byte, quindi `stride` vale 8. Dichiararne
    // 63 fa chiedere al ciclo `sub_src[62 + 7 * 8]`, cioe' l'indice 118 su 64
    // byte -- e 63 e' il massimo che lo zigzag esprime in **un** byte, quindi
    // nessun offset del file si sposta.
    let valori: Vec<f64> = (0..8).map(|i| f64::from(i) + 0.5).collect();
    let originale = parquet_valido(&valori);

    // Il controfattuale deve valere prima di alterare, o l'alterazione non
    // spiegherebbe niente.
    assert!(
        legge(&originale).is_ok(),
        "il controfattuale non regge: il file di partenza non si legge"
    );

    let inizio_footer = inizio_del_footer(&originale);
    let siti = siti_di_num_values(&originale, valori.len());
    assert!(
        !siti.is_empty(),
        "nessun `num_values` con valore {} trovato nel thrift: il layout del \
         writer e' cambiato, e questo riproduttore va rifatto sul nuovo",
        valori.len()
    );

    let mut scattati = Vec::new();
    for sito in &siti {
        let mut alterato = originale.clone();
        // Una sola sostituzione, un solo byte, nessuno spostamento.
        alterato[sito + 1] = varint_zigzag(63)[0];
        assert_eq!(
            alterato.len(),
            originale.len(),
            "l'alterazione non deve cambiare la lunghezza del file"
        );
        let esito = legge(&alterato);
        let dove = if *sito < inizio_footer {
            "header di pagina (prima del footer)"
        } else {
            "metadati di colonna (dentro il footer)"
        };
        match &esito {
            Err(messaggio) if messaggio.contains("index out of bounds") => {
                scattati.push((*sito, dove));
                eprintln!("offset {sito} [{dove}]: {messaggio}");
            }
            Err(messaggio) => eprintln!("offset {sito} [{dove}]: altro esito -- {messaggio}"),
            Ok(righe) => eprintln!("offset {sito} [{dove}]: letto senza errore, {righe} righe"),
        }
    }

    assert!(
        !scattati.is_empty(),
        "nessuno dei {} siti di `num_values` produce l'indicizzazione fuori \
         limite. O il pin di `parquet` e' stato aggiornato e il difetto e' \
         chiuso, o l'incoerenza che causa il panico non e' questo campo: la \
         descrizione va corretta invece di essere ripetuta",
        siti.len()
    );
    for (sito, dove) in &scattati {
        assert!(
            *sito < inizio_footer,
            "il sito che scatta e' a offset {sito}, cioe' nei {dove}: allora il \
             campo incoerente non e' quello dell'header di pagina come descritto"
        );
    }
}

/// L'inizio del footer, dichiarato dal file: quattro byte prima del magic finale.
fn inizio_del_footer(byte: &[u8]) -> usize {
    let lunghezza = u32::from_le_bytes([
        byte[byte.len() - 8],
        byte[byte.len() - 7],
        byte[byte.len() - 6],
        byte[byte.len() - 5],
    ]);
    // `u32` in `usize` non perde nulla sui bersagli supportati, e scriverlo
    // con `from` invece di `as` lo rende una conversione dichiarata.
    byte.len() - 8 - usize::try_from(lunghezza).unwrap_or(usize::MAX)
}

/// Gli offset dove compare `0x15` seguito dal varint del valore atteso.
///
/// `0x15` e' l'intestazione di campo del thrift compatto per «delta 1, tipo
/// i32», e `num_values` e' il campo 1 sia di `DataPageHeader` sia di
/// `DataPageHeaderV2` sia di `ColumnMetaData`. Comparirne piu' di uno e' la
/// norma, e per questo si provano tutti.
fn siti_di_num_values(byte: &[u8], atteso: usize) -> Vec<usize> {
    let Ok(atteso) = i64::try_from(atteso) else {
        return Vec::new();
    };
    let cercato = varint_zigzag(atteso);
    if cercato.len() != 1 {
        return Vec::new();
    }
    (0..byte.len().saturating_sub(1))
        .filter(|i| byte[*i] == 0x15 && byte[i + 1] == cercato[0])
        .collect()
}

/// Varint zigzag del thrift compatto.
fn varint_zigzag(valore: i64) -> Vec<u8> {
    // Zigzag: i negativi si intrecciano ai positivi, e il risultato si legge
    // come senza segno. `cast_unsigned` lo dice invece di lasciarlo a un `as`.
    let mut n = ((valore << 1) ^ (valore >> 63)).cast_unsigned();
    let mut fuori = Vec::new();
    loop {
        let sette = u8::try_from(n & 0x7F).unwrap_or_default();
        if n < 0x80 {
            fuori.push(sette);
            return fuori;
        }
        fuori.push(sette | 0x80);
        n >>= 7;
    }
}
