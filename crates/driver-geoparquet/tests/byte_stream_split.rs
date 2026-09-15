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

use plenora_io_core::request::{BatchTarget, ProjectionMode, ReadRequest, ReadScope};
use plenora_io_core::FormatDriver;
use plenora_io_model::contract::LayerId;
use plenora_io_model::CancellationToken;

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

/// La richiesta minima: il panico non dipende da cosa si chiede.
fn richiesta() -> ReadRequest {
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

/// La lettura completa restituisce un errore, e i quattro assi sono quelli.
///
/// «Rifiutato» non basterebbe: un input puo' essere rifiutato dal ramo
/// sbagliato e la prova resterebbe verde mentre la barriera che pretende di
/// pinnare e' sparita. Qui si pretendono fase, categoria, ritentabilita' e il
/// messaggio curato -- e che il panico **non** sia arrivato fino al chiamante.
#[test]
fn la_lettura_completa_non_propaga_il_panico() {
    // `catch_unwind` qui e' dello strumento, non del prodotto: distingue «il
    // driver ha restituito un errore» da «il panico e' arrivato fin qui», che
    // e' precisamente cio' che si misura. Senza, il secondo caso apparirebbe
    // come un test rosso e basta, e non si saprebbe perche'.
    let esito = std::panic::catch_unwind(|| {
        let aperto = driver_geoparquet::GeoParquetDriver
            .open(plenora_io_core::Source::Path(fixture()), opzioni_lettura())
            .expect("l'apertura riesce: il footer e lo schema sono coerenti");
        let mut lettore = aperto
            .open_layer_reader(&richiesta())
            .expect("il lettore si costruisce");
        let mut batch = 0_usize;
        loop {
            match lettore.next_batch() {
                Ok(Some(_)) => {
                    batch += 1;
                    assert!(batch < 1024, "il file e' piccolo: non deve ciclare");
                }
                Ok(None) => return Ok(batch),
                Err(errore) => return Err(errore),
            }
        }
    });

    let errore = match esito {
        // Il carico del panico si riporta: se questa prova diventa rossa, il
        // messaggio dice **quale** panico e' passato, e cercarlo nei log del
        // processo sarebbe un passaggio in piu' per un'informazione che sta
        // gia' qui.
        Err(carico) => panic!(
            "il panico ha attraversato la lettura: «{}». La barriera di \
             `plenora-io-core/src/driver.rs` non copre piu' questo percorso, e \
             un consumatore della libreria crollerebbe invece di ricevere un \
             errore",
            descrizione_del_panico(&*carico)
        ),
        Ok(Ok(batch)) => panic!(
            "la lettura ha restituito {batch} batch senza errore: o il decoder \
             upstream e' stato corretto -- e allora questa prova va rivista \
             insieme al pin di `parquet` -- o l'input non e' piu' quello del \
             finding"
        ),
        Ok(Err(errore)) => errore,
    };

    // I quattro assi dell'errore. La categoria dice **di chi** e' il problema:
    // `DataMapping` attribuisce all'input la mancata rappresentazione, che e'
    // corretto -- il file dichiara un tipo e fornisce dati che non gli
    // corrispondono.
    assert_eq!(
        errore.phase,
        plenora_io_model::ErrorPhase::Read,
        "il panico avviene decodificando le pagine, quindi la fase e' Read: {errore}"
    );
    assert_eq!(
        errore.category,
        plenora_io_model::ErrorCategory::DataMapping,
        "l'input non e' rappresentabile: la categoria e' DataMapping, non un \
         errore interno o di configurazione: {errore}"
    );
    assert_eq!(
        errore.retry,
        plenora_io_model::RetryDisposition::Never,
        "rileggere lo stesso file dara' lo stesso esito: la ritentabilita' e' \n         Never: {errore}"
    );

    // Il messaggio e' **curato** e statico: non porta nulla derivato dal
    // payload. E' la promessa del bordo, e nominare il panico e' anche cio'
    // che distingue questo rifiuto da uno qualunque.
    let testo = errore.to_string();
    assert!(
        testo.contains("panico"),
        "se il messaggio non nomina il panico, non e' la barriera ad aver \
         prodotto questo errore, e la prova sta guardando un altro rifiuto: \
         {testo}"
    );
}

/// Senza la barriera il panico esce: la causa e' upstream, non nostra.
///
/// Chiude l'altra meta' della domanda. Se questa prova diventasse verde per la
/// via `Ok`, vorrebbe dire che `parquet` e' stato corretto a monte: allora la
/// prova di sopra va rivista insieme al pin, e non si toglie una barriera che
/// costa nulla.
#[test]
fn senza_la_barriera_il_decoder_upstream_panica() {
    let esito = std::panic::catch_unwind(|| {
        let file = std::fs::File::open(fixture()).expect("la fixture si apre");
        let costruttore =
            parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder::try_new(file)
                .expect("il footer si legge: il file non e' malformato in modo grossolano");
        let lettore = costruttore.build().expect("il lettore si costruisce");
        lettore
            .map(|batch| batch.expect("batch").num_rows())
            .sum::<usize>()
    });

    match esito {
        Err(carico) => {
            let messaggio = descrizione_del_panico(&*carico);
            assert!(
                messaggio.contains("index out of bounds"),
                "il panico atteso e' l'indicizzazione fuori limite del decoder \
                 BYTE_STREAM_SPLIT; questo e' un altro: {messaggio}"
            );
        }
        Ok(righe) => panic!(
            "`parquet` ha letto {righe} righe senza panicare. Se il pin e' \
             stato aggiornato, il difetto upstream e' chiuso: aggiornare questo \
             file invece di cancellarlo, perche' la barriera resta utile"
        ),
    }
}
