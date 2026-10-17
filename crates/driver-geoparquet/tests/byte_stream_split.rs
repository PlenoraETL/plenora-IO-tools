//! Un panico di `parquet` sul decoder `BYTE_STREAM_SPLIT` resta un errore tipizzato.
//!
//! # Da dove viene questo file
//!
//! Il 15 settembre 2026 lo smoke fuzz della corsa «Release checkout
//! qualification» 34988124226, sulla revisione pubblicata
//! `6beb410b9984f527f3e89bba420764ac9e24e436`, ha segnalato un crash su
//! `geoparquet_reader`. L'input e' conservato qui come
//! `fixtures/byte-stream-split-double-incoerente.parquet`,
//! `sha256=18cb2a7e0e394484e466f920bde064a455d5e0e7edee21b2a2ce3ae51a6ef985`,
//! 3.966 byte, identico all'artefatto della corsa.
//!
//! Il panico e' **upstream**, in
//! `parquet-59.3.0/src/encodings/decoding/byte_stream_split_decoder.rs:61`,
//! dentro `join_streams_const::<8>` chiamata da
//! `ByteStreamSplitDecoder<DoubleType>::get`: `index out of bounds: the len
//! is 2 but the index is 2`. Non e' prodotto da una conversione nostra -- una
//! sonda separata chiama `parquet` direttamente sullo stesso file, senza niente
//! di nostro in mezzo, e panica.
//!
//! # Che cosa cambia con `parquet` 60
//!
//! Il decoder del footer della 60 rifiuta il seme prima delle pagine: il seme
//! non raggiunge piu' `ByteStreamSplitDecoder`. Il difetto del decoder pero'
//! c'e' ancora, identico, in `parquet-60.0.0` (`join_streams_const` indicizza
//! senza confrontare con la lunghezza), e resta raggiungibile da un file in cui
//! l'header di pagina **e** i metadati di colonna dichiarano piu' valori dei
//! byte presenti. Il riproduttore e la prova della barriera su quel file stanno
//! in `byte_stream_split_sintetico.rs`; qui restano le prove sul seme.
//!
//! # Perche' il fuzzer dice «crash» e il prodotto no
//!
//! `libfuzzer-sys` installa un panic hook che chiama `abort()` prima
//! dell'unwinding. Qualunque barriera `catch_unwind` gli resta percio'
//! invisibile, e il target segnala un crash anche quando la barriera funziona.
//! Il finding **resta valido**: l'arresto anticipato impedisce di osservare il
//! recupero, non rende inesistente il difetto a monte.
//!
//! Il prodotto non ha `panic = "abort"` in nessun profilo, quindi fuori dal
//! fuzzer l'unwinding e' quello di default e la barriera di
//! `plenora-io-core/src/driver.rs` si osserva. Queste prove la osservano, ed e'
//! il motivo per cui esistono: la barriera era documentata e non trattenuta da
//! niente per **questo** input.
//!
//! # Perche' la fixture non sta fra i semi del fuzzer
//!
//! Perche' `scripts/fuzz-replay.sh` riesegue **tutte e tre** le cartelle di un
//! bersaglio -- `fuzz/seeds/`, `fuzz/corpus/` e `fuzz/artifacts/` -- e sotto
//! `libfuzzer-sys` questo input aborta. I semi che oggi fanno panicare arrow
//! sono innocui perche' la prevalidazione li rifiuta **prima** del decoder;
//! questo invece ci arriva, e metterlo li' renderebbe rosso il replay a ogni
//! corsa che quelle cartelle contengono.
//!
//! Che `fuzz/corpus/` e `fuzz/artifacts/` siano ignorati da Git **non** li
//! esclude dal replay: dice solo che una copia appena clonata parte senza. In
//! locale persistono, e il replay le legge. Oggi
//! `fuzz/artifacts/geoparquet_reader/` contiene tre input del 2026-08-17 che il
//! replay del livello 2 su `6beb410` ha rieseguito verdi -- non riproducono --
//! e due di essi misurano 3.966 byte, la stessa dimensione di questa fixture:
//! stessa famiglia, quattro digest diversi.
//!
//! # Che cosa queste prove non dicono
//!
//! Non dicono che ogni input che raggiunge quel decoder sia contenuto: dicono
//! che questo lo e'. E non giustificano una prevalidazione aggiuntiva: la
//! barriera contiene il caso, e anticiparlo richiederebbe di replicare la
//! logica del decoder per dedurre se andra' in panico.

use std::path::{Path, PathBuf};

use plenora_io_core::FormatDriver;

/// Il messaggio di un panico catturato, quando e' testuale.
///
/// `catch_unwind` restituisce un carico opaco: i panici costruiti con un
/// formato portano una `String`, quelli con un letterale un `&str`. Chi non e'
/// nessuno dei due si dichiara tale invece di sparire.
fn descrizione_del_panico(carico: &(dyn std::any::Any + Send)) -> &str {
    carico
        .downcast_ref::<String>()
        .map(String::as_str)
        .or_else(|| carico.downcast_ref::<&str>().copied())
        .unwrap_or("(payload non testuale)")
}

/// L'input del finding, fuori da ogni percorso che il fuzzer riesegue.
fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/byte-stream-split-double-incoerente.parquet")
}

/// Le opzioni di lettura non hanno un `default`: i budget si dichiarano.
fn opzioni_lettura() -> plenora_io_core::ReadOptions {
    match plenora_io_model::budget::PipelineBudget::builder().build() {
        Ok(bundle) => plenora_io_core::ReadOptions::from_read_parts(bundle.into_read_parts()),
        Err(errore) => unreachable!("bundle di prova non costruibile: {errore:?}"),
    }
}

/// Il seme ora e' rifiutato all'apertura, con un errore tipizzato.
///
/// Con `parquet` 60 il decoder del footer e' piu' severo: il seme del finding
/// porta nel footer un campo il cui tipo Thrift non corrisponde a quello dello
/// schema, e la lettura dei metadati lo rifiuta prima di arrivare alle pagine.
/// Il seme non raggiunge piu' il decoder `BYTE_STREAM_SPLIT`; la barriera, che
/// resta necessaria perche' il decoder e' ancora difettoso, e' provata su un
/// file costruito apposta in `byte_stream_split_sintetico.rs`.
///
/// Qui si pretende che l'esito sia un errore e non un panico, con i quattro
/// assi e il messaggio curato: «rifiutato» da solo non distinguerebbe il ramo.
#[test]
fn il_seme_e_rifiutato_all_apertura_senza_panico() {
    // `catch_unwind` qui e' dello strumento, non del prodotto: distingue «il
    // driver ha restituito un errore» da «il panico e' arrivato fin qui».
    let esito = std::panic::catch_unwind(|| {
        driver_geoparquet::GeoParquetDriver
            .open(plenora_io_core::Source::Path(fixture()), opzioni_lettura())
            .map(|_| ())
    });

    let errore = match esito {
        Err(carico) => panic!(
            "il panico ha attraversato l'apertura: «{}»",
            descrizione_del_panico(&*carico)
        ),
        Ok(Ok(())) => panic!(
            "il seme si apre: il decoder del footer di `parquet` non lo rifiuta \
             piu', e allora puo' tornare a raggiungere le pagine. Rivedere \
             questa prova insieme al pin"
        ),
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
        errore.to_string().contains("footer Parquet non valido"),
        "il rifiuto deve venire dalla lettura del footer, non da un altro ramo: \
         {errore}"
    );
}

/// Anche `parquet` da solo, senza niente di nostro in mezzo, rifiuta il seme.
///
/// E' la controprova che il rifiuto e' del decoder a monte e non di una nostra
/// prevalidazione: se questa diventasse verde per la via `Ok`, il seme
/// tornerebbe a raggiungere il decoder delle pagine.
#[test]
fn parquet_60_rifiuta_il_footer_del_seme() {
    let esito = std::panic::catch_unwind(|| {
        let file = std::fs::File::open(fixture()).expect("la fixture si apre");
        parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder::try_new(file).map(|_| ())
    });
    match esito {
        Err(carico) => panic!(
            "`parquet` panica sul footer del seme: «{}»",
            descrizione_del_panico(&*carico)
        ),
        Ok(Ok(())) => panic!(
            "`parquet` accetta il footer del seme: rivedere questa prova e \
             quella di sopra insieme al pin"
        ),
        Ok(Err(_)) => {}
    }
}
