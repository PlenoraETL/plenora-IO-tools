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
//!
//! # Con `parquet` 60 e il fork
//!
//! Il difetto del decoder c'e' ancora, identico, in `parquet` 60.0.0 a monte.
//! Il fork che questo crate usa (`vendor/parquet`) lo chiude in due punti:
//!
//! * una pagina non dichiara piu' valori del suo column chunk (delta del fork
//!   `-eof` di plenora-data-tools): l'alterazione del solo header di pagina e'
//!   un errore prima del decoder;
//! * il decoder confronta i valori chiesti con i byte della pagina (delta di
//!   questo repository): l'alterazione **concorde** di header di pagina,
//!   metadati di colonna e righe, che il primo controllo non vede, e' un errore
//!   invece di un panico.
//!
//! Il riproduttore resta la descrizione del difetto a monte, che
//! `upstream/arrow-rs-byte-stream-split/` porta ad arrow-rs.

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

/// Lo stesso file, ma GeoParquet: una colonna `geometry` di punti WKB e il
/// metadato `geo`, perche' il driver lo apra e la lettura arrivi alle pagine.
///
/// `x` resta la prima colonna, quindi i primi `num_values` dell'header di
/// pagina e dei metadati di colonna sono i suoi.
fn geoparquet_valido(valori: &[f64]) -> Vec<u8> {
    use arrow_array::BinaryArray;
    use parquet::file::metadata::KeyValue;
    use parquet::schema::types::ColumnPath;

    let schema = Arc::new(Schema::new(vec![
        Field::new("x", DataType::Float64, false),
        Field::new("geometry", DataType::Binary, false),
    ]));
    let punti: Vec<Vec<u8>> = valori
        .iter()
        .map(|v| {
            let mut wkb = vec![1_u8, 1, 0, 0, 0];
            wkb.extend_from_slice(&v.to_le_bytes());
            wkb.extend_from_slice(&v.to_le_bytes());
            wkb
        })
        .collect();
    let colonne: Vec<ArrayRef> = vec![
        Arc::new(Float64Array::from(valori.to_vec())),
        Arc::new(BinaryArray::from_iter_values(punti.iter())),
    ];
    let batch = RecordBatch::try_new(Arc::clone(&schema), colonne).expect("il batch si costruisce");
    let geo = r#"{"version":"1.1.0","primary_column":"geometry","columns":{"geometry":{"encoding":"WKB","geometry_types":["Point"]}}}"#;
    let proprieta = WriterProperties::builder()
        .set_writer_version(WriterVersion::PARQUET_2_0)
        .set_dictionary_enabled(false)
        .set_column_encoding(ColumnPath::from("x"), Encoding::BYTE_STREAM_SPLIT)
        .set_key_value_metadata(Some(vec![KeyValue::new("geo".to_owned(), geo.to_owned())]))
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

/// Il file con `num_values` gonfiato nell'header di pagina, e lo stesso con le
/// dichiarazioni concordi: header di pagina, metadati di colonna e righe.
fn parquet_con_num_values_gonfiato() -> (Vec<u8>, Vec<u8>) {
    let valori: Vec<f64> = (0..8).map(|i| f64::from(i) + 0.5).collect();
    let originale = parquet_valido(&valori);
    assert!(
        legge(&originale).is_ok(),
        "il controfattuale non regge: il file di partenza non si legge"
    );
    let (solo_pagina, concordi) = gonfia(&originale, valori.len());
    (solo_pagina, concordi)
}

/// Le due alterazioni di un file con `quanti` valori, in un byte ciascuna.
///
/// Nell'header di pagina `num_values` e' un i32 (campo 1, intestazione 0x15);
/// nel footer `num_values` dei metadati di colonna e `num_rows` sono i64 dopo
/// un campo contiguo (intestazione 0x16). Otto valori: la pagina porta 64 byte,
/// quindi `stride` vale 8; dichiararne 63 -- il massimo che lo zigzag esprime
/// in **un** byte, cosi' nessun offset del file si sposta -- fa chiedere al
/// decoder l'indice 118 su 64 byte.
fn gonfia(originale: &[u8], quanti: usize) -> (Vec<u8>, Vec<u8>) {
    let inizio_footer = inizio_del_footer(originale);
    let pagina: Vec<usize> = siti_di_num_values(originale, quanti)
        .into_iter()
        .filter(|s| *s < inizio_footer)
        .collect();
    let footer: Vec<usize> = siti_con_intestazione(originale, 0x16, quanti)
        .into_iter()
        .filter(|s| *s >= inizio_footer)
        .collect();
    assert!(
        !pagina.is_empty() && !footer.is_empty(),
        "servono un `num_values` nell'header di pagina e uno nel footer: il \
         layout del writer e' cambiato, e il riproduttore va rifatto"
    );
    let mut solo_pagina = originale.to_vec();
    let mut concordi = originale.to_vec();
    for sito in &pagina {
        solo_pagina[sito + 1] = varint_zigzag(63)[0];
        concordi[sito + 1] = varint_zigzag(63)[0];
    }
    for sito in &footer {
        concordi[sito + 1] = varint_zigzag(63)[0];
    }
    (solo_pagina, concordi)
}

/// Gonfiare il solo header di pagina e' un errore, non un panico: una pagina
/// non dichiara piu' valori del suo column chunk.
#[test]
fn num_values_della_sola_pagina_oltre_il_chunk_e_un_errore() {
    let (solo_pagina, _) = parquet_con_num_values_gonfiato();
    match legge(&solo_pagina) {
        Err(messaggio) => {
            assert!(
                !messaggio.contains("index out of bounds"),
                "il decoder ha panicato: {messaggio}"
            );
            assert!(
                messaggio.contains("page value count exceeds the column chunk value count"),
                "il rifiuto atteso e' quello del fork sul conteggio della pagina: {messaggio}"
            );
        }
        Ok(righe) => panic!("il file alterato si legge, {righe} righe"),
    }
}

/// Con le dichiarazioni concordi il decoder `BYTE_STREAM_SPLIT` di `parquet`
/// 60.0.0 a monte panica (`index out of bounds`); quello del fork restituisce
/// un errore.
#[test]
fn num_values_concordi_oltre_i_byte_sono_un_errore_del_decoder() {
    let (_, concordi) = parquet_con_num_values_gonfiato();
    match legge(&concordi) {
        Err(messaggio) => {
            assert!(
                !messaggio.contains("index out of bounds"),
                "il decoder ha panicato: il controllo del fork sui byte della \
                 pagina non c'e' piu'. {messaggio}"
            );
            assert!(
                messaggio.contains("values requested beyond"),
                "il rifiuto atteso e' quello del decoder del fork: {messaggio}"
            );
        }
        Ok(righe) => panic!("il file alterato si legge, {righe} righe"),
    }
}

/// Lo stesso caso dal driver: un errore tipizzato con i quattro assi.
///
/// Non e' piu' la barriera a produrlo -- il decoder non panica -- e il
/// messaggio non nomina un panico: la barriera ha le sue prove in
/// `plenora-io-core`.
#[test]
fn il_driver_rifiuta_le_dichiarazioni_concordi_con_un_errore_tipizzato() {
    use plenora_io_core::FormatDriver as _;

    let valori: Vec<f64> = (0..8).map(|i| f64::from(i) + 0.5).collect();
    let (_, concordi) = gonfia(&geoparquet_valido(&valori), valori.len());
    let temporanea = tempfile::NamedTempFile::new().expect("file temporaneo");
    std::fs::write(temporanea.path(), &concordi).expect("il file si scrive");
    let percorso = temporanea.path().to_path_buf();

    // `catch_unwind` qui e' dello strumento, non del prodotto.
    let esito = std::panic::catch_unwind(move || {
        let aperto = driver_geoparquet::GeoParquetDriver
            .open(plenora_io_core::Source::Path(percorso), opzioni_lettura())?;
        let mut lettore = aperto.open_layer_reader(&richiesta())?;
        let mut batch = 0_usize;
        while lettore.next_batch()?.is_some() {
            batch += 1;
            assert!(batch < 1024, "il file e' piccolo: non deve ciclare");
        }
        Ok::<usize, plenora_io_model::PlenoraIoError>(batch)
    });

    let errore = match esito {
        Err(_) => panic!("un panico ha attraversato la lettura"),
        Ok(Ok(batch)) => panic!("la lettura ha restituito {batch} batch senza errore"),
        Ok(Err(errore)) => errore,
    };
    assert_eq!(errore.phase, plenora_io_model::ErrorPhase::Read, "{errore}");
    assert_eq!(
        errore.category,
        plenora_io_model::ErrorCategory::DataMapping,
        "{errore}"
    );
    assert_eq!(
        errore.retry,
        plenora_io_model::RetryDisposition::Never,
        "{errore}"
    );
    assert!(
        !errore.to_string().contains("panico"),
        "il decoder del fork non deve panicare: {errore}"
    );
}

/// Le opzioni di lettura non hanno un `default`: i budget si dichiarano.
fn opzioni_lettura() -> plenora_io_core::ReadOptions {
    match plenora_io_model::budget::PipelineBudget::builder().build() {
        Ok(bundle) => plenora_io_core::ReadOptions::from_read_parts(bundle.into_read_parts()),
        Err(errore) => unreachable!("bundle di prova non costruibile: {errore:?}"),
    }
}

/// La richiesta minima: il panico non dipende da cosa si chiede.
fn richiesta() -> plenora_io_core::request::ReadRequest {
    use plenora_io_core::request::{BatchTarget, ProjectionMode, ReadRequest, ReadScope};
    ReadRequest {
        layer: plenora_io_model::contract::LayerId(0),
        projected_fields: None,
        projection_mode: ProjectionMode::BestEffort,
        pruning_predicate: None,
        spatial_pruning_hint: None,
        scope: ReadScope::default(),
        batch_target: BatchTarget::default(),
        cancellation: plenora_io_model::CancellationToken::default(),
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
    // con `try_from` invece che con `as` lo rende una conversione dichiarata.
    //
    // `expect` e non `unwrap_or`: il ripiego era `usize::MAX`, che non produce
    // un offset sbagliato in silenzio -- la sottrazione andrebbe sotto zero --
    // ma nemmeno dice che cosa si stava assumendo. Un bersaglio a 16 bit lo
    // farebbe fallire qui, con la ragione scritta.
    byte.len()
        - 8
        - usize::try_from(lunghezza).expect("un u32 sta in usize sui bersagli supportati")
}

/// Gli offset dove compare `0x15` seguito dal varint del valore atteso.
///
/// `0x15` e' l'intestazione di campo del thrift compatto per «delta 1, tipo
/// i32», e `num_values` e' il campo 1 sia di `DataPageHeader` sia di
/// `DataPageHeaderV2`. Comparirne piu' di uno e' la norma, e per questo si
/// provano tutti.
fn siti_di_num_values(byte: &[u8], atteso: usize) -> Vec<usize> {
    siti_con_intestazione(byte, 0x15, atteso)
}

/// Gli offset dove compare l'intestazione di campo indicata seguita dal varint
/// del valore atteso, quando il varint sta in un byte.
fn siti_con_intestazione(byte: &[u8], intestazione: u8, atteso: usize) -> Vec<usize> {
    let Ok(atteso) = i64::try_from(atteso) else {
        return Vec::new();
    };
    let cercato = varint_zigzag(atteso);
    if cercato.len() != 1 {
        return Vec::new();
    }
    (0..byte.len().saturating_sub(1))
        .filter(|i| byte[*i] == intestazione && byte[i + 1] == cercato[0])
        .collect()
}

/// Varint zigzag del thrift compatto.
fn varint_zigzag(valore: i64) -> Vec<u8> {
    // Zigzag: i negativi si intrecciano ai positivi, e il risultato si legge
    // come senza segno. `cast_unsigned` lo dice invece di lasciarlo a un `as`.
    let mut n = ((valore << 1) ^ (valore >> 63)).cast_unsigned();
    let mut fuori = Vec::new();
    loop {
        // La maschera tiene il valore sotto 128, quindi la conversione non
        // puo' fallire. Il ripiego a zero, se mai scattasse, corromperebbe il
        // varint in silenzio: `expect` dice l'invariante invece di coprirlo.
        let sette = u8::try_from(n & 0x7F).expect("la maschera 0x7F tiene il valore sotto 128");
        if n < 0x80 {
            fuori.push(sette);
            return fuori;
        }
        fuori.push(sette | 0x80);
        n >>= 7;
    }
}
