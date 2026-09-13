//! driver-geoparquet — `GeoParquet` ⇄ `RecordBatch` (Fase 1). La geometria è WKB:
//! in lettura la colonna binaria viene ri-etichettata `geoarrow.wkb` + `crs`
//! SENZA decodifica (pass-through, V4); in scrittura si emette il metadato `geo`
//! dal contratto. Compressione configurabile (`format_options["compression"]`:
//! snappy default, oppure zstd/gzip/brotli/lz4) — zstd via zstd-sys.
#![forbid(unsafe_code)]

use std::collections::{BTreeSet, HashMap};
use std::fs::File;
use std::path::PathBuf;
use std::sync::Arc;

use arrow_array::{
    new_null_array, Array, ArrayRef, BinaryArray, Float64Array, LargeBinaryArray, RecordBatch,
    RecordBatchOptions, StructArray,
};
use arrow_buffer::NullBuffer;
use arrow_schema::{DataType, Field, Schema, SchemaRef};
use parquet::arrow::arrow_reader::{ParquetRecordBatchReader, ParquetRecordBatchReaderBuilder};
use parquet::arrow::{ArrowWriter, ProjectionMask};
use parquet::basic::{BrotliLevel, Compression, GzipLevel, ZstdLevel};
use parquet::file::metadata::KeyValue;
use parquet::file::properties::WriterProperties;
use parquet::file::statistics::Statistics;

use plenora_io_core::descriptor::{
    CrsHandling, Direction, Fidelity, FormatDescriptor, ReadMode, ReaderConcurrency, Runtime,
    WriteMode,
};
use plenora_io_core::driver::{
    FormatDriver, FormatWriter, LayerReader, OpenDatasetHandle, Published, ReadOptions, Sink,
    Source, WriteOptions,
};
use plenora_io_core::loss::LossReport;
use plenora_io_core::publish::StagedFile;
use plenora_io_core::request::{
    Bbox, ProjectionMode, PruningComparison, PruningPredicate, PruningScalar, ReadRequest,
};
use plenora_io_core::{
    validate_write, with_write_validation, AttributeWriteSupport, CrsDerivation,
    CrsRepresentationCapabilities, CrsRepresentationState, CrsWriteSupport,
    FormatWriteCapabilities, NullabilitySupport, SinkPathConstraint, TypeCoercionPolicy, WritePlan,
    ALL_ARROW_TYPES, UTF8_FIELD_NAMES, WKB_PASSTHROUGH_GEOMETRY,
};
use plenora_io_model::contract::{
    CoordinateDimensions, DataContract, FieldId, GeometryColumnContract, GeometryType,
    LayerContract, LayerId,
};
use plenora_io_model::crs::{
    crs_kind_for_authority_id, CrsKind, CrsResolution, RawCrs, ResolvedCrs,
};
use plenora_io_model::geometry::{ARROW_EXTENSION_NAME_KEY, GEOARROW_WKB_EXTENSION, GEO_CRS_KEY};
use plenora_io_model::limits::WkbLimits;
use plenora_io_model::wkb::inspect_wkb;
use plenora_io_model::{
    ContractIdentifier, ErrorContext, NumeroStrutturale, PlenoraIoError, PublicMessage, Result,
};

/// Verifica lo schema Arrow incorporato nel footer Parquet (FZ-0).
///
/// Un `.parquet` puo' portare la chiave `ARROW:schema` fra i metadati del
/// footer: e' un messaggio Arrow IPC in base64, deserializzato dalla stessa
/// conversione infallibile che fa panicare `arrow-ipc` su un `.arrow` ostile.
/// Il footer Thrift viene quindi letto **prima**, con l'API che non tocca
/// arrow, e lo schema incorporato viene verificato prima che la libreria lo
/// converta.
///
/// # Fail-closed
///
/// Un footer illeggibile, un base64 non decodificabile o uno schema non
/// conforme fermano la lettura. Un file senza `ARROW:schema` passa: non c'e'
/// niente da convertire, e la chiave e' opzionale nel formato.
/// Tetto sulla dimensione non compressa **dichiarata dai metadati** di un
/// chunk di colonna.
///
/// # Cosa limita, e cosa no
///
/// Limita il totale che il *footer* dichiara per il chunk. **Non** limita
/// l'allocazione che la decompressione compie: `SerializedPageReader` alloca
/// con `Vec::with_capacity(uncompressed_page_size)` (parquet 59.1.0,
/// `file/serialized_reader.rs:447`) usando il valore dell'**header di
/// pagina**, che e' un `i32` indipendente dal totale del chunk. Un file che
/// dichiari un chunk piccolo e una pagina enorme supera questo tetto e fa
/// comunque chiedere fino a circa 2 GiB per pagina.
///
/// Era il residuo di FZ-0.1: questo tetto, da solo, **non** e' protezione dagli
/// header di pagina incoerenti — e' un filtro sui metadati che scarta il caso
/// grossolano prima di aprire il lettore di pagine.
///
/// FZ-0.2 l'ha chiuso: `pagine::valida_chunk` verifica ogni header **prima**
/// che il decoder allochi, e in particolare che nessuna pagina dichiari piu'
/// byte non compressi del proprio chunk. Da quel momento questo tetto e' una
/// garanzia effettiva sull'allocazione, perche' non esiste piu' una pagina che
/// lo scavalchi. Le due cose vanno insieme: descriverlo come garanzia **senza**
/// la prevalidazione di pagina sarebbe di nuovo falso.
///
/// L'API pubblica di `parquet` non basta a farlo — `PageMetadata` non porta le
/// dimensioni e `PageHeader` e' `pub(crate)` — quindi l'header lo leggiamo noi,
/// bounded, in `pagine`.
///
/// Il valore e' assoluto e volutamente largo: un chunk che ne dichiara di piu'
/// non e' un chunk grande, e' una dichiarazione su cui rifiutiamo di agire. Un
/// tetto sul **rapporto** di decompressione — come quello che il contenitore
/// XLSX applica — rifiuterebbe anche file leciti molto comprimibili, e sarebbe
/// un restringimento del contratto invece di una difesa.
mod metadati;
mod schema_ufficiale;
use metadati::MetadatiGeo;
mod livelli;
mod pagine;

const MAX_BYTE_CHUNK_ISPEZIONATO: i64 = 1 << 30;

/// Messaggi pubblici della prevalidazione Parquet: **statici**.
///
/// Un messaggio che riportasse il bit width letto, la codifica trovata o il
/// testo dell'errore della libreria porterebbe fuori un valore derivato dal
/// payload, e `PlenoraIoError::message` dichiara di non contenerne. Il valore
/// che serve a correggere il file sta nel file, non nell'errore.
const MSG_BIT_WIDTH_OLTRE_MASSIMO: &str =
    "bit width degli indici di dizionario Parquet oltre il massimo del formato";
const MSG_PAGINE_NON_LEGGIBILI: &str = "pagine Parquet non leggibili";
const MSG_SEZIONE_VALORI_ASSENTE: &str = "data page a dizionario Parquet senza sezione valori";
const MSG_CODIFICA_LIVELLI_IGNOTA: &str = "codifica dei livelli Parquet non riconosciuta";
const MSG_LIVELLI_TRONCATI: &str = "data page Parquet troncata sui livelli";
const MSG_SEZIONE_NON_RAPPRESENTABILE: &str = "sezione dei livelli Parquet non rappresentabile";
const MSG_FOOTER_NON_VALIDO: &str = "footer Parquet non valido";
const MSG_DIZIONARIO_DOPO_I_DATI: &str =
    "chunk di colonna Parquet con pagina di dizionario dopo le pagine dati";
const MSG_CHUNK_OLTRE_TETTO: &str =
    "chunk di colonna Parquet oltre il tetto di dimensione non compressa";

/// Bit width massimo per un indice di dizionario letto come `i32`.
///
/// E' il limite del **formato**: gli indici stanno in `i32`, quindi
/// trentadue bit li esauriscono. Non e' una regola nostra.
const MAX_BIT_WIDTH_INDICI: u8 = 32;

/// Impedisce il panico di `parquet` sul bit width degli indici (FZ-0.1).
///
/// `DictIndexDecoder::new` prende il bit width dal primo byte della sezione
/// valori di una data page a dizionario e lo passa a `RleDecoder` senza
/// controllarne l'intervallo (`arrow/decoder/dictionary_index.rs:46`). Un
/// valore oltre trentadue arriva a `BitReader::get_batch::<i32>` e fa panicare
/// la libreria: al `debug_assert!` sotto fuzzing, all'aritmetica non
/// controllata nel profilo release che spediamo. La barriera lo converte in
/// errore tipizzato, ma il panico e' avvenuto.
///
/// # Perimetro: solo cio' che verra' letto davvero
///
/// La verifica non scorre il file: guarda i soli chunk che **projection e
/// pruning hanno gia' selezionato**, e fra quelli solo i dictionary-encoded.
/// Un file senza dizionario non paga niente, perche' `encodings()` lo dice dai
/// metadati senza toccare una pagina; un file con dizionario paga la lettura
/// delle sole colonne proiettate nei soli row group sopravvissuti al pruning.
///
/// Lo snapshot dei metadati e' **quello della lettura**, passato dal
/// chiamante: se ne rileggesse uno proprio, validazione e lettura potrebbero
/// guardare due file diversi.
///
/// # Bounded
///
/// Le pagine sono lette una per volta dal `PageReader` della libreria e non
/// vengono trattenute; la validazione guarda un byte per pagina. Nessuna
/// allocazione proporzionale al file.
fn valida_bit_width_dizionario(
    sorgente: &Arc<File>,
    metadati: &parquet::file::metadata::ParquetMetaData,
    maschera: &ProjectionMask,
    gruppi: Option<&[usize]>,
    tetto_pagina: u64,
) -> Result<()> {
    use parquet::column::page::PageReader as _;
    let tutti: Vec<usize> = (0..metadati.num_row_groups()).collect();
    // `None` significa "nessun pruning applicato", cioe' tutti i row group:
    // non e' un default di ripiego, e' il significato dell'assenza.
    let gruppi = match gruppi {
        Some(selezionati) => selezionati,
        None => &tutti,
    };

    for &indice_gruppo in gruppi {
        let blocco = metadati.row_groups().get(indice_gruppo).ok_or_else(|| {
            fmt_err(&PublicMessage::Curated(
                "indice di row group Parquet fuori intervallo",
            ))
        })?;
        for (foglia, chunk) in blocco.columns().iter().enumerate() {
            if !maschera.leaf_included(foglia) {
                continue;
            }
            let descrittore = chunk.column_descr();
            let (max_rep, max_def) = (descrittore.max_rep_level(), descrittore.max_def_level());
            // Filtro dai metadati: si apre un chunk solo se c'e' qualcosa da
            // guardarci dentro.
            //
            // Erano i soli chunk a dizionario, e con la sola domanda sul bit
            // width degli indici era il filtro giusto. Ora ce n'e' una seconda:
            // i livelli, che stanno in ogni data page di una colonna annidata o
            // nullable, a dizionario o no. Un chunk piatto e obbligatorio --
            // livelli massimi entrambi a zero -- non ha ne' indici ne' livelli,
            // e continua a non pagare niente.
            //
            // Il costo cambia, e va detto: un file di sole colonne nullable
            // senza dizionario prima non faceva aprire una pagina, ora le legge
            // tutte. E' il prezzo della domanda, non un'inefficienza: la domanda
            // non si puo' rispondere dai metadati, perche' il difetto sta nei
            // byte della sezione.
            let ha_livelli = max_rep > 0 || max_def > 0;
            if !chunk.encodings().any(e_a_dizionario) && !ha_livelli {
                continue;
            }
            // Tetto prima dell'allocazione: la decompressione avviene dentro
            // `get_next_page`, quindi il rifiuto deve precedere la chiamata.
            let non_compressi = chunk.uncompressed_size();
            if !(0..=MAX_BYTE_CHUNK_ISPEZIONATO).contains(&non_compressi) {
                return Err(fmt_err(&PublicMessage::Curated(MSG_CHUNK_OLTRE_TETTO)));
            }
            let righe = usize::try_from(blocco.num_rows()).map_err(|_| {
                fmt_err(&PublicMessage::Curated(
                    "numero di righe del row group Parquet negativo",
                ))
            })?;
            // FZ-0.2: le dimensioni di **questo** chunk si verificano qui, non
            // solo nel passaggio d'insieme che precede la costruzione del
            // reader. Questa funzione legge le pagine con `get_next_page`, che
            // alloca quanto l'header dichiara: dedurre la protezione dal
            // chiamante e' esattamente cio' che FZ-0 ha mostrato non reggere,
            // ed e' quello che il gate anti-chiamata-nuda verifica.
            pagine::valida_chunk(
                sorgente,
                u64::try_from(inizio_del_chunk(chunk)?).map_err(|_| {
                    fmt_err(&PublicMessage::Curated(
                        "chunk di colonna Parquet non rappresentabile",
                    ))
                })?,
                u64::try_from(chunk.compressed_size()).map_err(|_| {
                    fmt_err(&PublicMessage::Curated(
                        "chunk di colonna Parquet non rappresentabile",
                    ))
                })?,
                non_compressi,
                tetto_pagina,
            )?;

            let mut lettore_pagine = parquet::file::serialized_reader::SerializedPageReader::new(
                Arc::clone(sorgente),
                chunk,
                righe,
                None,
            )
            .map_err(|_| fmt_err(&PublicMessage::Curated(MSG_PAGINE_NON_LEGGIBILI)))?;
            while let Some(pagina) = lettore_pagine
                .get_next_page()
                .map_err(|_| fmt_err(&PublicMessage::Curated(MSG_PAGINE_NON_LEGGIBILI)))?
            {
                valida_pagina(&pagina, max_rep, max_def)?;
            }
        }
    }
    Ok(())
}

/// Verifica le dimensioni dichiarate dagli header di pagina, prima che il
/// decoder ne allochi una (FZ-0.2).
///
/// Cammina gli stessi chunk di `valida_bit_width_dizionario` — quelli che
/// projection e pruning porteranno davvero al decoder — ma **tutti**, non solo
/// quelli a dizionario: l'allocazione della decompressione non dipende dalla
/// codifica.
///
/// Va eseguita **prima** di `valida_bit_width_dizionario`, che per leggere le
/// pagine usa `get_next_page` e quindi e' esposta alla stessa allocazione che
/// questa impedisce. La prevalidazione non e' al riparo per il fatto di essere
/// prevalidazione: passa dallo stesso lettore.
///
/// # Errors
///
/// `DataMapping` con messaggio statico se una pagina dichiara piu' byte non
/// compressi del tetto o del proprio chunk.
fn valida_dimensioni_pagine(
    sorgente: &Arc<File>,
    metadati: &parquet::file::metadata::ParquetMetaData,
    maschera: &ProjectionMask,
    gruppi: Option<&[usize]>,
    tetto_pagina: u64,
) -> Result<()> {
    let tutti: Vec<usize> = (0..metadati.num_row_groups()).collect();
    let gruppi = match gruppi {
        Some(selezionati) => selezionati,
        None => &tutti,
    };
    for &indice_gruppo in gruppi {
        let blocco = metadati.row_groups().get(indice_gruppo).ok_or_else(|| {
            fmt_err(&PublicMessage::Curated(
                "indice di row group Parquet fuori intervallo",
            ))
        })?;
        for (foglia, chunk) in blocco.columns().iter().enumerate() {
            if !maschera.leaf_included(foglia) {
                continue;
            }
            // Gli offset e le lunghezze sono gia' passati da
            // `valida_metadati_thrift`: non negativi, rappresentabili e dentro
            // il file. Qui restano da convertire, non da verificare.
            let inizio = inizio_del_chunk(chunk)?;
            let primo_byte = u64::try_from(inizio).map_err(|_| {
                fmt_err(&PublicMessage::Curated(
                    "chunk di colonna Parquet non rappresentabile",
                ))
            })?;
            let byte_compressi = u64::try_from(chunk.compressed_size()).map_err(|_| {
                fmt_err(&PublicMessage::Curated(
                    "chunk di colonna Parquet non rappresentabile",
                ))
            })?;
            pagine::valida_chunk(
                sorgente,
                primo_byte,
                byte_compressi,
                chunk.uncompressed_size(),
                tetto_pagina,
            )?;
        }
    }
    Ok(())
}

/// Il tetto per una singola pagina non compressa (FZ-0.2.1).
///
/// **Meta' della capacita' di memoria effettiva**, cioe' del minimo fra il
/// limite della pipeline e quello del pool. Con i valori predefiniti sono 256
/// MiB, meta' dei 512 MiB dichiarati.
///
/// Tre scelte, tutte deliberate.
///
/// *Memoria e non ingresso.* FZ-0.2 aveva usato `max_input_bytes`: funzionava,
/// ma confondeva due quote che il modello tiene distinte apposta.
/// `max_input_bytes` governa quanto e' grande la **sorgente**; una pagina
/// decompressa e' **memoria temporanea**. Con quella quota, alzare il tetto sul
/// file alzava anche quello sulla memoria — che non e' cio' che chi lo alza sta
/// chiedendo.
///
/// *Capacita' effettiva e non `PipelineLimits::memory_bytes`.* Con un pool piu'
/// stretto del limite locale, una soglia calcolata sul solo limite locale
/// sarebbe irraggiungibile: e' la stessa ragione per cui il modello espone
/// `effective_memory_capacity`, ed e' scritta li'.
///
/// *Meta' e non tutta.* La pagina decompressa non e' sola in memoria: accanto
/// ci sono la pagina compressa da cui viene, i buffer del decoder e gli array
/// Arrow che ne escono. Concedere l'intera capacita' a una sola allocazione
/// significherebbe dichiararla l'unica, e non lo e'.
///
/// La libreria **non** misura la memoria reale del processo, e non promette di
/// farlo: leggerla e' instabile e non portabile. Configurare quattro gigabyte
/// su una macchina che ne ha mezzo resta un errore di deployment, che questa
/// funzione non puo' vedere ne' correggere.
fn tetto_pagina(context: &plenora_io_model::budget::PipelineContext) -> u64 {
    context.effective_memory_capacity() / 2
}

/// Le due prevalidazioni che precedono il decoder, **in ordine**.
///
/// L'ordine e' la sostanza: `valida_bit_width_dizionario` legge le pagine con
/// `get_next_page`, che alloca quanto l'header dichiara, quindi e' esposta alla
/// stessa cosa da cui difende. Prima si verificano le dimensioni degli header,
/// poi si guarda dentro le pagine.
///
/// Entrambe guardano i chunk che projection e pruning hanno appena selezionato,
/// sullo stesso snapshot di metadati della lettura: se ne leggessero uno
/// proprio, validazione e lettura potrebbero guardare due file diversi.
///
/// # Errors
///
/// Il primo rifiuto delle due, con il proprio messaggio statico.
fn prevalida_cio_che_il_decoder_leggera(
    sorgente: &Arc<File>,
    metadati: &parquet::file::metadata::ParquetMetaData,
    maschera: &ProjectionMask,
    gruppi: Option<&[usize]>,
    tetto_pagina: u64,
) -> Result<()> {
    // FZ-0.2
    valida_dimensioni_pagine(sorgente, metadati, maschera, gruppi, tetto_pagina)?;
    // FZ-0.1
    valida_bit_width_dizionario(sorgente, metadati, maschera, gruppi, tetto_pagina)
}

/// Il primo byte di un chunk di colonna.
///
/// **E' la regola del formato, non un default di ripiego**: un chunk comincia
/// con la pagina di dizionario se c'e', altrimenti con la prima pagina dati.
/// E' anche la regola di `parquet`: in 59.1.0 ogni consumatore — il lettore di
/// pagine e quello arrow — passa da `ColumnChunkMetaData::byte_range`, che fa
/// esattamente questa scelta. Non possiamo chiamarla perche' asserisce su
/// offset negativi invece di restituirli.
///
/// Sta in una funzione sola perche' la usano tre prevalidazioni: ripeterla
/// significherebbe tre occorrenze da giustificare nel registro dei fallback per
/// una regola che e' una.
///
/// # Errors
///
/// Se il file dichiara una pagina di dizionario **dopo** la prima pagina dati.
/// Nessun chunk legittimo lo fa — il dizionario precede sempre i dati che
/// indicizza — e la coerenza fra cio' che verifichiamo e cio' che il decoder
/// legge dipende oggi dal fatto che `parquet` scelga come noi. Rifiutarlo
/// rende quell'accordo strutturale invece che condizionato: se domani `parquet`
/// prendesse il minore dei due, su questi file non ci sarebbe piu' un file su
/// cui divergere.
fn inizio_del_chunk(chunk: &parquet::file::metadata::ColumnChunkMetaData) -> Result<i64> {
    let Some(dizionario) = chunk.dictionary_page_offset() else {
        return Ok(chunk.data_page_offset());
    };
    if dizionario > chunk.data_page_offset() {
        return Err(fmt_err(&PublicMessage::Curated(MSG_DIZIONARIO_DOPO_I_DATI)));
    }
    Ok(dizionario)
}

const fn e_a_dizionario(codifica: parquet::basic::Encoding) -> bool {
    matches!(
        codifica,
        parquet::basic::Encoding::RLE_DICTIONARY | parquet::basic::Encoding::PLAIN_DICTIONARY
    )
}

/// Verifica una singola pagina: i livelli sempre, il bit width se e' a
/// dizionario.
///
/// # Perche' le due cose stanno insieme
///
/// Perche' condividono l'attraversamento. Il bit width degli indici e' il primo
/// byte **dopo** le sezioni dei livelli, quindi per trovarlo bisogna gia'
/// averle percorse; e percorrerle e' esattamente cio' che serve a verificarle.
/// Due funzioni farebbero due volte lo stesso lavoro sullo stesso buffer, con
/// due copie della stessa aritmetica.
///
/// La verifica dei livelli **non** e' condizionata alla codifica a dizionario.
/// La stesura precedente usciva subito su una pagina non a dizionario, ed era
/// giusto finche' l'unica domanda era sul bit width degli indici: una pagina
/// senza indici non ne ha. I livelli invece ci sono in ogni data page di una
/// colonna annidata o nullable, qualunque sia la codifica dei valori.
fn valida_pagina(pagina: &parquet::column::page::Page, max_rep: i16, max_def: i16) -> Result<()> {
    use parquet::column::page::Page;

    let (buffer, inizio_valori, codifica) = match pagina {
        // La pagina di dizionario porta i **valori**, non gli indici, e non ha
        // livelli: il suo primo byte non e' un bit width e guardarlo sarebbe un
        // errore.
        Page::DictionaryPage { .. } => return Ok(()),
        Page::DataPage {
            buf,
            num_values,
            encoding,
            def_level_encoding,
            rep_level_encoding,
            ..
        } => {
            let inizio = inizio_valori_v1(
                buf,
                *num_values,
                max_rep,
                max_def,
                *rep_level_encoding,
                *def_level_encoding,
            )?;
            (buf, inizio, *encoding)
        }
        Page::DataPageV2 {
            buf,
            num_values,
            encoding,
            def_levels_byte_len,
            rep_levels_byte_len,
            ..
        } => {
            let inizio = valida_livelli_v2(
                buf,
                *rep_levels_byte_len,
                *def_levels_byte_len,
                *num_values,
                max_rep,
                max_def,
            )?;
            (buf, inizio, *encoding)
        }
    };

    if !e_a_dizionario(codifica) {
        return Ok(());
    }
    let bit_width = buffer
        .get(inizio_valori)
        .ok_or_else(|| fmt_err(&PublicMessage::Curated(MSG_SEZIONE_VALORI_ASSENTE)))?;
    if *bit_width > MAX_BIT_WIDTH_INDICI {
        return Err(fmt_err(&PublicMessage::Curated(
            MSG_BIT_WIDTH_OLTRE_MASSIMO,
        )));
    }
    Ok(())
}

/// Dove cominciano i valori in una data page V1, **validando i livelli**.
///
/// Prima dei valori stanno le sezioni dei livelli, presenti solo se il livello
/// massimo della colonna e' maggiore di zero. `RLE` porta un prefisso di
/// quattro byte con la lunghezza; `BIT_PACKED` — deprecata ma ammessa dallo
/// spec — non lo porta, e la sua dimensione si calcola da `num_values` e dai
/// bit necessari al livello massimo.
///
/// Una codifica diversa da queste due ferma la lettura invece di far tirare a
/// indovinare l'offset.
///
/// # Perche' calcola e verifica insieme
///
/// Perche' il posto in cui si calcola dove finisce una sezione e' lo stesso in
/// cui si sa quanti byte quella sezione ha davvero. Separare le due cose vuol
/// dire scrivere due volte lo stesso attraversamento, e due attraversamenti
/// dello stesso formato divergono: uno avanza di quattro byte e l'altro no, uno
/// tratta `BIT_PACKED` e l'altro l'ha dimenticata, e la divergenza non fa rosso
/// da nessuna parte perche' ciascuno e' coerente con se'.
///
/// Due cose si verificano qui e non stavano prima:
///
/// * la sezione dichiarata sta **dentro** il buffer della pagina -- il prefisso
///   di quattro byte poteva dichiarare piu' byte di quanti la pagina ne portasse,
///   e l'unico controllo era l'assenza di overflow aritmetico;
/// * il flusso ibrido **dentro** la sezione e' ben formato, che e' il difetto di
///   questo lotto. Vedi [`livelli`].
#[allow(deprecated)]
fn inizio_valori_v1(
    buffer: &[u8],
    num_values: u32,
    max_rep: i16,
    max_def: i16,
    rep_level_encoding: parquet::basic::Encoding,
    def_level_encoding: parquet::basic::Encoding,
) -> Result<usize> {
    use parquet::basic::Encoding;

    let mut inizio = 0usize;
    for (livello_massimo, codifica) in
        [(max_rep, rep_level_encoding), (max_def, def_level_encoding)]
    {
        if livello_massimo <= 0 {
            continue;
        }
        inizio = match codifica {
            Encoding::RLE => {
                let dopo_prefisso = inizio.checked_add(4).ok_or_else(|| {
                    fmt_err(&PublicMessage::Curated(MSG_SEZIONE_NON_RAPPRESENTABILE))
                })?;
                let prefisso = buffer
                    .get(inizio..dopo_prefisso)
                    .ok_or_else(|| fmt_err(&PublicMessage::Curated(MSG_LIVELLI_TRONCATI)))?;
                let lunghezza =
                    u32::from_le_bytes([prefisso[0], prefisso[1], prefisso[2], prefisso[3]])
                        as usize;
                let fine = dopo_prefisso.checked_add(lunghezza).ok_or_else(|| {
                    fmt_err(&PublicMessage::Curated(MSG_SEZIONE_NON_RAPPRESENTABILE))
                })?;
                // La sezione dichiarata deve stare dentro la pagina. Senza
                // questo, un prefisso che dichiara piu' byte di quanti ce ne
                // siano passava: l'unico controllo era che la somma non
                // traboccasse.
                let sezione = buffer
                    .get(dopo_prefisso..fine)
                    .ok_or_else(|| fmt_err(&PublicMessage::Curated(MSG_LIVELLI_TRONCATI)))?;
                livelli::valida_sezione(sezione, bit_dei_livelli(livello_massimo)?, num_values)?;
                fine
            }
            Encoding::BIT_PACKED => {
                let livello = u64::try_from(livello_massimo).map_err(|_| {
                    fmt_err(&PublicMessage::Curated("livello massimo Parquet negativo"))
                })?;
                let bit_totali = (num_values as usize)
                    .checked_mul(bit_necessari(livello) as usize)
                    .ok_or_else(|| {
                        fmt_err(&PublicMessage::Curated(MSG_SEZIONE_NON_RAPPRESENTABILE))
                    })?;
                let fine = inizio.checked_add(bit_totali.div_ceil(8)).ok_or_else(|| {
                    fmt_err(&PublicMessage::Curated(MSG_SEZIONE_NON_RAPPRESENTABILE))
                })?;
                // `BIT_PACKED` non e' il flusso ibrido: e' un impacchettamento
                // piatto, senza run e senza intestazioni, e non ha niente da
                // attraversare. Cio' che va verificato e' che i byte che la sua
                // dimensione implica ci siano: se la pagina finisce prima, il
                // decoder legge oltre il buffer per la stessa ragione, per
                // un'altra strada.
                if fine > buffer.len() {
                    return Err(fmt_err(&PublicMessage::Curated(MSG_LIVELLI_TRONCATI)));
                }
                fine
            }
            _ => {
                return Err(fmt_err(&PublicMessage::Curated(
                    MSG_CODIFICA_LIVELLI_IGNOTA,
                )))
            }
        };
    }
    Ok(inizio)
}

/// Bit necessari a rappresentare un valore, come fa `parquet` per i livelli.
const fn bit_necessari(valore: u64) -> u32 {
    u64::BITS - valore.leading_zeros()
}

/// Il bit width con cui i livelli di una colonna sono codificati.
///
/// E' quello del **livello massimo dichiarato dallo schema**, non un valore
/// letto dalla pagina: `parquet` codifica e decodifica i livelli con questa
/// larghezza, e prenderla da altrove significherebbe verificare un flusso
/// diverso da quello che il decoder leggera'.
///
/// # Errors
///
/// [`PlenoraIoError`] se il livello massimo e' negativo — non lo e' in uno
/// schema valido — o se non sta in `u8`, che per un livello di annidamento
/// significa uno schema che non descrive un documento.
fn bit_dei_livelli(livello_massimo: i16) -> Result<u8> {
    let livello = u64::try_from(livello_massimo)
        .map_err(|_| fmt_err(&PublicMessage::Curated("livello massimo Parquet negativo")))?;
    u8::try_from(bit_necessari(livello))
        .map_err(|_| fmt_err(&PublicMessage::Curated(MSG_SEZIONE_NON_RAPPRESENTABILE)))
}

/// Le due sezioni dei livelli di una data page V2, verificate.
///
/// In V2 le lunghezze non si deducono: stanno nell'header, e i livelli sono
/// **sempre** `RLE` senza prefisso — la lunghezza e' gia' quella dichiarata.
/// Restituisce dove cominciano i valori, che e' la somma delle due.
///
/// # Errors
///
/// [`PlenoraIoError`] se le lunghezze non sono rappresentabili o non stanno
/// dentro il buffer della pagina, o se uno dei due flussi ibridi e' malformato.
fn valida_livelli_v2(
    buffer: &[u8],
    rep_levels_byte_len: u32,
    def_levels_byte_len: u32,
    num_values: u32,
    max_rep: i16,
    max_def: i16,
) -> Result<usize> {
    let mut inizio = 0usize;
    for (livello_massimo, lunghezza) in [
        (max_rep, rep_levels_byte_len),
        (max_def, def_levels_byte_len),
    ] {
        let fine = inizio
            .checked_add(lunghezza as usize)
            .ok_or_else(|| fmt_err(&PublicMessage::Curated(MSG_SEZIONE_NON_RAPPRESENTABILE)))?;
        let sezione = buffer
            .get(inizio..fine)
            .ok_or_else(|| fmt_err(&PublicMessage::Curated(MSG_LIVELLI_TRONCATI)))?;
        // Il salto si fa comunque, anche con livello massimo zero: la lunghezza
        // e' dichiarata dall'header e i valori cominciano dopo di essa. Il
        // flusso invece si verifica solo dove esiste — con livello massimo zero
        // il decoder non lo legge affatto, e pretendere che copra `num_values`
        // rifiuterebbe una pagina corretta.
        if livello_massimo > 0 {
            livelli::valida_sezione(sezione, bit_dei_livelli(livello_massimo)?, num_values)?;
        }
        inizio = fine;
    }
    Ok(inizio)
}

/// Verifica offset, lunghezze e somme dichiarati dai metadati Thrift.
///
/// Il lettore li usa senza controlli: `ColumnChunkMetaData::byte_range`
/// asserisce `col_start >= 0 && col_len >= 0` (parquet 59.1.0,
/// `file/metadata/mod.rs:1063`), quindi un footer con un offset negativo
/// abbatte il processo prima di leggere un solo byte di dati.
fn valida_metadati_thrift(
    metadati: &parquet::file::metadata::ParquetMetaData,
    dimensione: u64,
) -> Result<()> {
    // usa senza controlli: `ColumnChunkMetaData::byte_range` asserisce
    // `col_start >= 0 && col_len >= 0` (parquet 59.1.0,
    // `file/metadata/mod.rs:1063`), quindi un footer con un offset negativo
    // abbatte il processo prima di leggere un solo byte di dati.
    for gruppo in metadati.row_groups() {
        if gruppo.num_rows() < 0 || gruppo.total_byte_size() < 0 {
            return Err(fmt_err(&PublicMessage::Curated(
                "gruppo di righe Parquet con conteggio o dimensione negativi",
            )));
        }
        for colonna in gruppo.columns() {
            let inizio = inizio_del_chunk(colonna)?;
            let lunghezza = colonna.compressed_size();
            if inizio < 0 || lunghezza < 0 || colonna.data_page_offset() < 0 {
                return Err(fmt_err(&PublicMessage::Curated(
                    "chunk di colonna Parquet con offset o lunghezza negativi",
                )));
            }
            // Rappresentabili e dentro il file: un chunk che dichiara byte
            // oltre la fine non e' un chunk corto, e' un chunk che non c'e'.
            let primo_byte = u64::try_from(inizio).map_err(|_| {
                fmt_err(&PublicMessage::Curated(
                    "chunk di colonna Parquet non rappresentabile",
                ))
            })?;
            let byte_dichiarati = u64::try_from(lunghezza).map_err(|_| {
                fmt_err(&PublicMessage::Curated(
                    "chunk di colonna Parquet non rappresentabile",
                ))
            })?;
            let oltre_il_chunk = primo_byte.checked_add(byte_dichiarati).ok_or_else(|| {
                fmt_err(&PublicMessage::Curated(
                    "chunk di colonna Parquet non rappresentabile",
                ))
            })?;
            if oltre_il_chunk > dimensione {
                return Err(fmt_err(&PublicMessage::Curated(
                    "chunk di colonna Parquet oltre la fine del file",
                )));
            }
            if colonna.num_values() < 0 {
                return Err(fmt_err(&PublicMessage::Curated(
                    "chunk di colonna Parquet con conteggio negativo",
                )));
            }
        }
    }

    Ok(())
}

fn valida_schema_arrow_incorporato(file: File, dimensione: u64) -> Result<()> {
    use base64::Engine as _;
    use parquet::file::reader::FileReader as _;

    const CHIAVE: &str = "ARROW:schema";

    // `SerializedFileReader` legge il footer Thrift e si ferma li': non
    // costruisce lo schema Arrow ne' i lettori di colonna, quindi non
    // raggiunge ne' la conversione che panica ne' `byte_range`. E' l'unico
    // ingresso nella libreria che precede la prevalidazione, perche' **e'** la
    // lettura dei metadati da validare; sta comunque sotto la barriera.
    let lettore = plenora_io_core::driver::leggendo_arrow("parquet", || {
        // Statico come gli altri messaggi della prevalidazione: il testo
        // dell'errore della libreria e' derivato dal file, e `message`
        // dichiara di non contenere payload.
        parquet::file::reader::SerializedFileReader::new(file)
            .map_err(|_| fmt_err(&PublicMessage::Curated(MSG_FOOTER_NON_VALIDO)))
    })?;
    let metadati = lettore.metadata();

    valida_metadati_thrift(metadati, dimensione)?;

    let Some(chiavi) = metadati.file_metadata().key_value_metadata() else {
        return Ok(());
    };
    for voce in chiavi {
        if voce.key != CHIAVE {
            continue;
        }
        let Some(valore) = voce.value.as_ref() else {
            return Err(fmt_err(&PublicMessage::Curated(
                "chiave ARROW:schema priva di valore",
            )));
        };
        let byte = base64::engine::general_purpose::STANDARD
            .decode(valore)
            .map_err(|_| {
                fmt_err(&PublicMessage::Curated(
                    "ARROW:schema non decodificabile da base64",
                ))
            })?;
        driver_common::prevalida_arrow::valida_messaggio_schema("parquet", &byte)?;
    }
    Ok(())
}

/// Mappa ogni campo esposto sulla sua posizione nello schema Parquet fisico.
///
/// Un campo esposto senza corrispondente fisico e' un errore di contratto —
/// mai atteso, ma fail-closed.
///
/// Il nome del campo **non entra nel messaggio**: esce dal campo `field`
/// dell'errore come [`ContractIdentifier`], cioe' nel posto dove i consumatori
/// lo trovano senza doverlo estrarre da una frase. Quando il nome non e'
/// nominabile — vuoto, o oltre il tetto — l'errore resta senza campo invece di
/// portarne uno inventato, e l'indice nel messaggio identifica comunque il
/// punto.
fn mappa_campi_fisici(out_schema: &SchemaRef, parquet_schema: &SchemaRef) -> Result<Vec<usize>> {
    let mut visible_to_physical: Vec<usize> = Vec::with_capacity(out_schema.fields().len());
    for (posizione, field) in out_schema.fields().iter().enumerate() {
        let index = parquet_schema.index_of(field.name()).map_err(|_| {
            let errore = fmt_err(&PublicMessage::CuratedWith(
                "campo esposto non presente nello schema Parquet fisico, indice",
                NumeroStrutturale::Indice(driver_common::saturating_u64(posizione)),
            ));
            u32::try_from(posizione)
                .ok()
                .and_then(|indice| {
                    ContractIdentifier::from_schema_field(
                        out_schema.as_ref(),
                        plenora_io_model::contract::FieldId(indice),
                    )
                })
                .map_or_else(
                    || errore.clone(),
                    |identificatore| {
                        errore
                            .clone()
                            .con_contesto(&ErrorContext::nuovo().con_identificatore(identificatore))
                    },
                )
        })?;
        visible_to_physical.push(index);
    }
    Ok(visible_to_physical)
}

/// Un `field id` richiesto in projection `Required` che lo schema non ha.
///
/// Esce l'indice, che viene dalla richiesta del chiamante ed e' un numero
/// strutturale; nient'altro.
fn campo_fuori_range(fid: plenora_io_model::contract::FieldId) -> PlenoraIoError {
    PlenoraIoError::non_supportato_redatto(&PublicMessage::CuratedWith(
        "projection Required: field id fuori range,",
        NumeroStrutturale::Indice(u64::from(fid.0)),
    ))
}

fn fmt_err(reason: &PublicMessage) -> PlenoraIoError {
    PlenoraIoError::formato_redatto("geoparquet", reason)
}

use plenora_io_model::format_options::{
    FaseOpzione, OpzioneFormato, SchemaOpzioniFormato, ValoreAmmesso,
};

/// Le `format_options` interpretate dal driver `GeoParquet` (L0.7, S6).
///
/// `uncompressed` e `none` sono due nomi dello stesso esito e restano
/// entrambi: erano gia' accettati, toglierne uno sarebbe una rottura di
/// contratto travestita da pulizia dello schema.
const SCHEMA_OPZIONI: SchemaOpzioniFormato = SchemaOpzioniFormato::nuovo(&[
    OpzioneFormato {
        chiave: "accept_legacy_crs_id_only",
        fase: FaseOpzione::Lettura,
        valore: ValoreAmmesso::Booleano,
        predefinito: Some("false"),
        descrizione:
            "opt-in alla lettura del `crs` storico non conforme scritto da plenora fino a S10",
    },
    OpzioneFormato {
        chiave: "bbox_legacy_by_name",
        fase: FaseOpzione::Lettura,
        valore: ValoreAmmesso::Booleano,
        predefinito: Some("false"),
        descrizione: "opt-in al riconoscimento per nome delle colonne bbox legacy",
    },
    OpzioneFormato {
        chiave: "compression",
        fase: FaseOpzione::Scrittura,
        valore: ValoreAmmesso::Enumerato(&[
            "brotli",
            "gzip",
            "lz4",
            "none",
            "snappy",
            "uncompressed",
            "zstd",
        ]),
        predefinito: Some("snappy"),
        descrizione: "codec di compressione delle pagine",
    },
]);

static DESCRIPTOR: FormatDescriptor = FormatDescriptor::const_new(
    "geoparquet",
    Direction::Bidirectional,
    ReadMode::StreamingColumnar,
    // INV-7: row group indirizzabili, `SeekFrom::Start` sugli offset del footer.
    plenora_io_core::NativeReadMode::StreamingRandom,
    // Il drenaggio e lo spool sono dell'adapter comune, non di
    // questo driver: `BudgetedReader` li impone a tutti.
    plenora_io_core::DeliverySemantics::OperationAtomic,
    plenora_io_core::BufferingStrategy::AdaptiveMemoryThenDisk,
    plenora_io_core::DeterminismLevel::Semantic,
    Some(WriteMode::Streaming),
    Some(plenora_io_core::DeterminismLevel::Semantic),
    false,
    false,
    ReaderConcurrency::MultipleIndependentReaders, // Parquet è seekable
    plenora_io_core::ProjectionSupport::Exact,
    plenora_io_core::PredicatePruningSupport::NumericMinMaxStatistics,
    plenora_io_core::SpatialPruningSupport::BoundingBoxStatistics,
    CrsHandling::Embedded,
    Fidelity::Lossless,
    Runtime::PureRust,
    // `hostile_input_hardened`: non dichiarato: l'input e' binario, con prevalidazione delle pagine.
    false,
    // `spec_version_supported`: GeoParquet 1.1.0. Il validatore accetta
    // esattamente 1.0.0 e 1.1.0 -- i due valori che gli schemi ufficiali
    // fissano -- e oltre rifiuta come funzionalita' non supportata. Il campo
    // dice al consumatore dove il supporto finisce, invece di lasciarglielo
    // dedurre dal fatto che il formato compare nel catalogo.
    Some("1.1.0"),
    Some(FormatWriteCapabilities {
        field_names: UTF8_FIELD_NAMES,
        allowed_types: ALL_ARROW_TYPES,
        type_coercion: TypeCoercionPolicy::Reject,
        attributes: AttributeWriteSupport::All,
        geometry: WKB_PASSTHROUGH_GEOMETRY,
        crs: CrsWriteSupport::Embedded,
        crs_representations: CrsRepresentationCapabilities::new(
            CrsRepresentationState::Preserved,
            // L'SRID non viene scritto: il metadato `geo` non ha un posto dove
            // metterlo. E' pero' **ricavabile**, perche' l'identificatore c'e'
            // e il lettore ne toglie il prefisso `EPSG:`. E' la stessa forma
            // del GeoPackage, e la categoria giusta e' quella derivata:
            // `absent` direbbe a chi automatizza che l'SRID va riportato da
            // fuori, e non e' vero.
            CrsRepresentationState::Derived(CrsDerivation::FromIdentifier),
            // La definizione **si conserva**. Il writer scrive nel metadato
            // `geo` il PROJJSON che il contratto porta, verbatim: una
            // definizione che non sia PROJJSON viene rifiutata e non degradata,
            // e un CRS noto per solo identificatore ferma la scrittura invece
            // di produrre un file senza CRS. Quindi ogni scrittura che riesce
            // ha scritto la definizione che le e' arrivata.
            CrsRepresentationState::Preserved,
        ),
        nullability: NullabilitySupport::Preserve,
        multi_layer: false,
        sink_path: SinkPathConstraint::Free,
    }),
    SCHEMA_OPZIONI,
    &["parquet"],
    1,
    5,
    9,
);

pub struct GeoParquetDriver;

/// Preflight della sorgente per il percorso di lettura.
///
/// Estratto perche' `open` sfiorava il tetto di righe, ma anche perche' in
/// S4.d il cambio semantico del preflight — enumerazione via il modello
/// unificato e rimozione dei controlli legacy — dovra' avvenire in un punto
/// solo per driver, non sparso nel corpo di `open`.
/// L'opt-in al riconoscimento per nome delle colonne bbox legacy.
///
/// La forma booleana e' quella dello schema, non una lista scritta a mano qui:
/// prima "false" e "pippo" erano indistinguibili — entrambi "non vero" — e un
/// opt-in scritto male taceva invece di correggersi.
/// L'opt-in alla lettura del `crs` storico, non conforme.
///
/// Spento per default, e deve restarlo: ogni `GeoParquet` che questo repository
/// ha scritto fino a S10 dichiara `crs: {"id": {...}}`, che **non e'** un
/// documento PROJJSON e che lo schema ufficiale rifiuta in entrambe le
/// versioni. Quei file non sono conformi, e accettarli in silenzio vorrebbe
/// dire chiamare conforme cio' che non lo e'.
///
/// Chi ha quei dati accende l'opzione e sa che cosa sta accettando: il
/// contratto lo dichiara, e la via non entra nel supporto `GeoParquet` dichiarato
/// conforme.
fn opt_in_crs_storico(format_options: &std::collections::BTreeMap<String, String>) -> Result<bool> {
    format_options
        .get("accept_legacy_crs_id_only")
        .map_or(Ok(false), |valore| {
            plenora_io_model::format_options::booleano(
                "geoparquet",
                "accept_legacy_crs_id_only",
                valore,
            )
        })
}

fn opt_in_bbox_legacy(format_options: &std::collections::BTreeMap<String, String>) -> Result<bool> {
    format_options
        .get("bbox_legacy_by_name")
        .map_or(Ok(false), |valore| {
            plenora_io_model::format_options::booleano("geoparquet", "bbox_legacy_by_name", valore)
        })
}

fn percorso_verificato(source: Source, opts: &mut ReadOptions) -> Result<PathBuf> {
    plenora_io_core::preflight_source(&DESCRIPTOR, source, opts)
}

impl FormatDriver for GeoParquetDriver {
    fn descriptor(&self) -> &FormatDescriptor {
        &DESCRIPTOR
    }

    fn open(&self, source: Source, mut opts: ReadOptions) -> Result<Box<dyn OpenDatasetHandle>> {
        let path = percorso_verificato(source, &mut opts)?;
        // Il footer Parquet puo' portare la chiave `ARROW:schema`, che e' un
        // messaggio Arrow IPC deserializzato qui dentro: un `.parquet` ostile
        // raggiunge quindi lo stesso panico di un `.arrow`. Vedi
        // `leggendo_arrow`.
        // Una sola apertura, poi handle clonati: due `open` distinti possono
        // cadere su due file diversi se il percorso viene sostituito fra l'uno
        // e l'altro, e la verifica varrebbe per un file che non e' quello
        // letto.
        let sorgente = File::open(&path)?;
        let dimensione = sorgente.metadata()?.len();
        valida_schema_arrow_incorporato(sorgente.try_clone()?, dimensione)?;
        let builder = plenora_io_core::driver::leggendo_arrow("parquet", || {
            ParquetRecordBatchReaderBuilder::try_new(sorgente.try_clone()?)
                .map_err(|_| fmt_err(&PublicMessage::Curated("Parquet non valido")))
        })?;
        let parquet_schema = builder.schema().clone();
        let geo = read_geo_meta(&builder, opt_in_crs_storico(&opts.format_options)?)?;
        let (geom_name, crs) = resolve_geometry_and_crs(&parquet_schema, geo.as_ref())?;
        // **Prima** di toccare lo schema esposto: il metadato viene confrontato
        // col file, e se non regge il file e' rifiutato. Farlo dopo il retag
        // vorrebbe dire aver gia' tolto colonne sulla fiducia.
        if let Some(geo) = geo.as_ref() {
            riconcilia_con_lo_schema_fisico(&parquet_schema, builder.parquet_schema(), geo)?;
        }
        // Finding #4 follow-up follow-up review 2026-08-15: il fallback
        // legacy per-nome (accettare `_bbox_minx/miny/maxx/maxy` come
        // covering anche in assenza di metadata `covering.bbox`) e' stato
        // rimosso dal percorso predefinito perche' un GeoParquet esterno
        // con attributi utente omonimi veniva silenziosamente trattato
        // come covering — perdendo colonne o applicando pruning
        // sbagliato. Il fallback e' ora un opt-in esplicito via
        // `format_options["bbox_legacy_by_name"] = "true"`: chi ha file
        // scritti prima del covering GeoParquet 1.1 lo abilita
        // esplicitamente, prendendosi responsabilita' del comportamento
        // documentato.
        let legacy_by_name_opt_in = opt_in_bbox_legacy(&opts.format_options)?;
        // Il covering dichiarato -- quattro **percorsi**, gia' validati nella
        // forma dello schema 1.1.0 -- oppure, su richiesta esplicita, le
        // quattro colonne piatte storiche.
        //
        // I due casi non si mescolano e l'ordine e' quello: un covering
        // dichiarato vale, e il riconoscimento per nome resta cio' che era, un
        // opt-in per i file scritti prima che il covering avesse una forma.
        let covering: Option<[Vec<String>; 4]> = covering_bbox_paths(geo.as_ref()).or_else(|| {
            legacy_by_name_opt_in
                .then(|| BBOX_COLS.map(|nome| vec![nome.to_owned()]))
                .filter(|percorsi| {
                    percorsi
                        .iter()
                        .all(|p| percorso_presente(&parquet_schema, p))
                })
        });
        // Retag toglie dallo schema esposto **solo** le colonne radice che il
        // covering occupa: con la forma conforme e' la colonna struct, con
        // quella storica sono le quattro piatte. Un file senza covering e senza
        // opt-in conserva tutte le colonne utente esattamente come sono.
        let strip_names: Option<Vec<String>> = covering.as_ref().map(radici_del_covering);
        let out_schema = retag_schema(&parquet_schema, &geom_name, &crs, strip_names.as_deref());
        // Il covering utilizzabile per il pruning e' quello che si e' ottenuto:
        // se e' dichiarato, la riconciliazione ha gia' preteso che i quattro
        // percorsi esistano -- e se non esistevano non siamo arrivati fin qui --
        // mentre la via storica per nome ha filtrato la presenza sopra. Un
        // ulteriore filtro qui sarebbe un ramo che non puo' scattare.
        let bbox_covering: Option<[Vec<String>; 4]> = covering;
        // Mappa logico → fisico prima di consumare `out_schema` per il
        // contratto. Ogni campo esposto viene localizzato per nome nello
        // schema Parquet originale. Un campo esposto senza corrispondente
        // fisico e' un errore di contratto (mai atteso, ma fail-closed).
        let visible_to_physical = mappa_campi_fisici(&out_schema, &parquet_schema)?;
        let geom_idx = out_schema
            .index_of(&geom_name)
            .map_err(|_| fmt_err(&PublicMessage::Curated("colonna geometria non leggibile")))?;
        // Indice di colonna di uno schema Parquet: limitato a poche migliaia di
        // campi, il cast a u32 non puo' troncare.
        #[allow(clippy::cast_possible_truncation)]
        let geometry_field_id = FieldId(geom_idx as u32);
        let mut geometry = GeometryColumnContract::wkb_passthrough(
            geometry_field_id,
            geom_name,
            crs,
            out_schema.field(geom_idx).is_nullable(),
        );
        apply_geo_column_metadata(&mut geometry, geo.as_ref());
        let contract = DataContract::new(out_schema, Some(geometry));
        // `DataContract::new` rende i metadati geometrici del contratto
        // autoritativi; anche i batch runtime devono essere retaggati con
        // quello stesso schema, non con la versione intermedia.
        let out_schema = contract.schema.clone();
        let name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("layer")
            .to_owned();
        let layer = LayerContract {
            id: LayerId(0),
            name,
            contract,
        };
        Ok(plenora_io_core::with_read_budget(
            Box::new(GeoParquetDataset {
                path,
                out_schema,
                bbox_covering,
                visible_to_physical,
                layers: vec![layer],
                tetto_pagina: tetto_pagina(opts.budget().context()),
            }),
            &opts,
            true,
        ))
    }

    fn create(
        &self,
        sink: Sink,
        plan: &WritePlan,
        opts: &WriteOptions,
    ) -> Result<Box<dyn FormatWriter>> {
        validate_write(
            self.descriptor(),
            plan,
            opts.max_columns(),
            &opts.format_options,
        )?;
        let Sink::Path(path) = sink;
        if path.exists() {
            return Err(PlenoraIoError::destinazione_esistente());
        }
        // Nessun controllo sul suffisso della destinazione.
        //
        // Ce n'era uno, e rifiutava una destinazione il cui nome non portasse
        // l'estensione attesa. Non era un requisito del formato: dopo il
        // controllo l'estensione non veniva usata per **niente** -- ne' per
        // derivare un nome, ne' per scegliere un comportamento -- ed era una
        // convenzione travestita da vincolo. Il costo era che il formato
        // esplicito non bastava a scegliere la destinazione: chi pubblicava su
        // un percorso di staging, o su un nome generato, veniva rifiutato per
        // il nome invece che per i dati.
        //
        // Il suffisso resta rilevante per chi **rilegge** senza dichiarare il
        // formato, e quella parte e' dichiarata e non taciuta:
        // `recognised_suffixes` del descrittore la rende, e `io.catalog` la
        // pubblica. Scrivere e riconoscere sono due cose, e ora si vedono
        // entrambe.
        if plan.layers.len() != 1 {
            return Err(PlenoraIoError::non_supportato_redatto(
                &PublicMessage::Curated("GeoParquet: un solo layer per dataset nella v1"),
            ));
        }
        let layer = &plan.layers[0];
        let schema = layer.contract.schema.clone();
        // Finding #4 review 2026-08-15: prima del fix il writer aggiungeva
        // sempre le 4 colonne bbox interne alle colonne utente, senza
        // controllare che non esistessero gia' con quei nomi. Il risultato
        // era una sovrascrittura silenziosa che alterava il contratto
        // dichiarato dall'utente. Fail-closed qui rifiuta il piano prima
        // di aprire il sink: e' la stessa policy di collisione applicata
        // dagli altri driver ai propri metadati interni.
        if schema
            .fields()
            .iter()
            .any(|field| is_bbox_col(field.name()))
        {
            // Il nome utente non entra nel testo: viene dal piano, e chi
            // legge l'errore ha il piano. L'elenco delle colonne bbox e'
            // nostro ed e' costante: sta nella documentazione del driver, non
            // in ogni messaggio.
            return Err(fmt_err(&PublicMessage::Curated(
                "GeoParquet: una colonna utente entrerebbe in collisione con le colonne bbox \
                 interne del covering spaziale; rinominare la colonna utente prima della \
                 scrittura",
            )));
        }
        let (geom_idx, geom_name, legacy_crs_meta) = geometry_field(&schema)?;
        // Il CRS si decide **all'apertura**, non alla chiusura: se non e'
        // scrivibile in modo conforme, e' meglio saperlo prima di aver scritto
        // un file intero.
        let crs_meta = crs_da_scrivere(layer.contract.geometry.as_ref(), legacy_crs_meta)?;
        // Schema di scrittura = utente + la colonna struct `bbox` del covering.
        //
        // Una colonna sola, con quattro figli: e' la forma che il covering
        // GeoParquet 1.1 designa, e la nullabilita' segue quella della
        // geometria.
        let geometria_nullable = schema.field(geom_idx).is_nullable();
        let mut aug_fields: Vec<Field> =
            schema.fields().iter().map(|f| f.as_ref().clone()).collect();
        aug_fields.push(bbox_field(geometria_nullable));
        let write_schema: SchemaRef = Arc::new(Schema::new_with_metadata(
            aug_fields,
            schema.metadata().clone(),
        ));
        let staging = StagedFile::new(&path, opts.durable, opts.max_output_bytes())?;
        // Row group da 64k righe: statistiche min/max abbastanza granulari da
        // rendere efficace il row-group pruning in lettura (Fase 2C).
        let props = WriterProperties::builder()
            .set_compression(compression_from(opts)?)
            .set_max_row_group_row_count(Some(65_536))
            .build();
        let writer = ArrowWriter::try_new(staging.reopen()?, write_schema.clone(), Some(props))
            .map_err(|_| {
                fmt_err(&PublicMessage::Curated(
                    "apertura del writer Parquet fallita",
                ))
            })?;
        with_write_validation(
            Box::new(GeoParquetWriter {
                staging,
                writer: Some(writer),
                write_schema,
                geom_idx,
                geom_name,
                crs_meta,
                geometry_types: BTreeSet::new(),
                wkb_limits: opts.wkb_limits(),
                geometria_nullable,
            }),
            self.descriptor(),
            plan,
            opts,
        )
    }
}

/// Che cosa il writer mette nel campo `crs` della colonna.
///
/// Tre esiti, e nessun quarto silenzioso. La prima stesura ne aveva uno solo --
/// `{"id": {...}}`, che **non e' un documento PROJJSON** -- e in piu' aveva un
/// buco: se l'identificatore non aveva la forma `AUTH:CODE`, il campo non
/// veniva scritto affatto, e un lettore lo avrebbe interpretato come «assente»,
/// cioe' come CRS84. Un file in un altro sistema di riferimento si dichiarava
/// WGS84 senza che nessuno lo avesse deciso.
#[derive(Clone, Debug, PartialEq, Eq)]
enum CrsDaScrivere {
    /// CRS84: il campo si omette, ed e' la specifica a dire che assente vuol
    /// dire questo.
    Omesso,
    /// Nessun CRS: `crs: null`. E' un'affermazione, non un'omissione.
    Nullo,
    /// Un documento PROJJSON, emesso per intero.
    Documento(String),
}

/// Gli identificatori che, dentro `GeoParquet`, **sono** CRS84.
///
/// `OGC:CRS84` lo e' per definizione. `EPSG:4326` lo e' per una ragione che sta
/// nel formato e non nel registro EPSG: `GeoParquet` impone l'ordine degli assi
/// longitudine-latitudine alle coordinate che memorizza, quindi un dataset
/// dichiarato `EPSG:4326` e scritto qui ha le stesse coordinate, nello stesso
/// ordine, di uno dichiarato `OGC:CRS84`. I due sono equivalenti **in questo
/// formato**, e non lo sarebbero altrove.
///
/// La canonicalizzazione e' percio' a CRS84, e si esprime **omettendo** il
/// campo: la specifica dice che assente vuol dire CRS84. Non e' una perdita --
/// il dato che esce e' identico al dato che entra -- ed e' l'unico modo di
/// tenere scrivibile il caso comune senza sintetizzare un PROJJSON che nessuno
/// ci ha dato.
const EQUIVALENTI_A_CRS84: [&str; 2] = ["OGC:CRS84", "EPSG:4326"];

/// La versione che questo writer dichiara, e contro cui valida cio' che scrive.
const VERSIONE_SCRITTA: &str = "1.1.0";

fn crs_da_scrivere(
    geometry: Option<&GeometryColumnContract>,
    legacy_crs_meta: Option<String>,
) -> Result<CrsDaScrivere> {
    let risoluzione = geometry.map(|geometria| &geometria.crs);
    let definizione = match risoluzione {
        Some(CrsResolution::Resolved(crs)) => crs.definition.as_deref(),
        _ => None,
    };
    // Una definizione che e' un oggetto JSON e' **candidata** a essere il
    // PROJJSON che il contratto porta. Candidata, non tale: chiamare PROJJSON
    // qualunque oggetto era il difetto, e produceva file non conformi con la
    // stessa disinvoltura con cui ne produceva di conformi. Chi decide e' lo
    // schema, in scrittura come in lettura.
    if let Some(testo) = definizione {
        if let Ok(serde_json::Value::Object(oggetto)) =
            serde_json::from_str::<serde_json::Value>(testo)
        {
            let documento = serde_json::Value::Object(oggetto);
            if schema_ufficiale::e_projjson(&documento)? {
                return Ok(CrsDaScrivere::Documento(testo.to_owned()));
            }
            // Si nomina il campo, mai il contenuto: il documento arriva dal
            // contratto e non entra in un messaggio pubblico.
            return Err(PlenoraIoError::non_supportato_redatto(
                &PublicMessage::Curated(
                    "definizione CRS che non e' un documento PROJJSON valido: `GeoParquet` non ha un modo conforme di scriverla",
                ),
            ));
        }
    }

    let identificatore = match risoluzione {
        Some(CrsResolution::Resolved(crs)) => crs.id.as_deref(),
        Some(CrsResolution::DeclaredButUnresolved(raw)) => raw.authority_hint.as_deref(),
        Some(CrsResolution::Missing) | None => None,
    }
    .map(str::to_owned)
    .or(legacy_crs_meta);

    match identificatore {
        None => Ok(CrsDaScrivere::Nullo),
        Some(id) if EQUIVALENTI_A_CRS84.contains(&id.as_str()) => Ok(CrsDaScrivere::Omesso),
        // Conosciuto per identificatore e non rappresentabile: rifiuto. La
        // tentazione sarebbe scrivere `null` -- «CRS sconosciuto» -- ma noi lo
        // conosciamo, e dichiarare di non saperlo sarebbe una perdita semantica
        // che nessuno ha dichiarato. Anche `{"id": ...}` sarebbe una scorciatoia:
        // non e' PROJJSON, e il file non sarebbe conforme.
        Some(_) => Err(PlenoraIoError::non_supportato_redatto(
            &PublicMessage::Curated(
                "il CRS e' noto solo per identificatore e GeoParquet pretende un documento PROJJSON: \
                 nessuna definizione PROJJSON disponibile per questa scrittura",
            ),
        )),
    }
}

struct GeoParquetDataset {
    path: PathBuf,
    out_schema: SchemaRef,
    /// Nomi delle colonne bbox del covering spaziale, se il file dichiara
    /// `covering.bbox` nel metadata `GeoParquet` 1.1 o, in fallback, se sono
    /// presenti tutti e quattro i nomi convenzionali `_bbox_minx`/... .
    /// Il pruning spaziale legge min/max da queste colonne (finding #4
    /// review 2026-08-15 + follow-up).
    bbox_covering: Option<[Vec<String>; 4]>,
    /// Mappa dagli indici logici dello schema esposto (`out_schema`, senza
    /// le colonne bbox interne) agli indici fisici root dello schema
    /// `Parquet`. Prima del follow-up review 2026-08-15 la CLI passava
    /// direttamente `0..out_schema.len()` a `ProjectionMask::roots`, che
    /// coincide col fisico solo se le colonne rimosse sono in coda: un
    /// `GeoParquet` esterno con bbox intercalate produceva colonne
    /// sbagliate o errore di schema. Ora la mappa e' calcolata una volta
    /// all'apertura e ogni projection la usa per tradurre.
    visible_to_physical: Vec<usize>,
    layers: Vec<LayerContract>,
    /// Il tetto per una singola pagina non compressa, derivato all'apertura.
    ///
    /// Viene dallo stesso snapshot di opzioni con cui il dataset e' stato
    /// aperto, non ricalcolato a ogni `open_layer_reader`: due letture sullo
    /// stesso handle non devono poter usare quote diverse. Vedi
    /// [`tetto_pagina`].
    tetto_pagina: u64,
}

impl GeoParquetDataset {
    /// Apre il file **una sola volta** e ne restituisce l'handle condiviso
    /// insieme al builder, con lo schema Arrow incorporato gia' verificato.
    ///
    /// Il file viene riaperto a ogni `open_layer_reader`, quindi riverificato:
    /// fra l'apertura del dataset e questa chiamata il contenuto su disco puo'
    /// essere cambiato. Gli handle sono cloni della stessa apertura, cosi'
    /// verifica e lettura non possono finire su due file diversi — cosa che due
    /// `open` distinti non garantiscono.
    fn apri_verificato(&self) -> Result<(Arc<File>, ParquetRecordBatchReaderBuilder<File>)> {
        let sorgente = Arc::new(File::open(&self.path)?);
        let dimensione = sorgente.metadata()?.len();
        valida_schema_arrow_incorporato(sorgente.try_clone()?, dimensione)?;
        let per_builder = sorgente.try_clone()?;
        let builder = plenora_io_core::driver::leggendo_arrow("parquet", move || {
            ParquetRecordBatchReaderBuilder::try_new(per_builder)
                .map_err(|_| fmt_err(&PublicMessage::Curated("Parquet non valido")))
        })?;
        Ok((sorgente, builder))
    }

    /// I row group che la lettura toccherà davvero, dopo entrambi i pruning.
    ///
    /// I due restituiscono la selezione invece di applicarla al builder: e' lo
    /// stesso valore che alimenta la lettura e la prevalidazione, quindi le due
    /// non possono guardare insiemi diversi.
    ///
    /// I due pruning si compongono per **intersezione**: un row group viene
    /// letto solo se entrambi i criteri lo tengono. Fino a FZ-0.1 lo spaziale
    /// sostituiva il numerico — `with_row_groups` veniva chiamato due volte e
    /// la seconda vinceva — e con predicato e hint insieme il pruning numerico
    /// andava perso. Non erano righe sbagliate, perche' l'over-return e'
    /// dichiarato ammesso: era lavoro fatto per niente.
    fn gruppi_da_leggere(
        &self,
        builder: &ParquetRecordBatchReaderBuilder<File>,
        request: &ReadRequest,
    ) -> Option<Vec<usize>> {
        let numerici = gruppi_dopo_pruning(
            builder.metadata(),
            request.pruning_predicate.as_ref(),
            self.out_schema.as_ref(),
            builder.parquet_schema(),
        );
        let spaziali = gruppi_dopo_pruning_spaziale(
            builder.metadata(),
            builder.parquet_schema(),
            request.spatial_pruning_hint.as_ref(),
            self.bbox_covering.as_ref(),
        );
        match (numerici, spaziali) {
            // Un row group va letto solo se **entrambi** i pruning lo tengono.
            // Ognuno esclude cio' che il proprio criterio ha gia' escluso, e
            // l'intersezione e' l'unica composizione che li rispetta entrambi.
            (Some(numerici), Some(spaziali)) => Some(
                numerici
                    .into_iter()
                    .filter(|gruppo| spaziali.contains(gruppo))
                    .collect(),
            ),
            (Some(soli), None) | (None, Some(soli)) => Some(soli),
            (None, None) => None,
        }
    }
}

impl OpenDatasetHandle for GeoParquetDataset {
    fn layers(&self) -> &[LayerContract] {
        &self.layers
    }

    fn fidelity_assessment(&self) -> plenora_io_core::FidelityAssessment {
        plenora_io_core::FidelityAssessment::for_format(
            DESCRIPTOR.id(),
            DESCRIPTOR.fidelity_class(),
        )
    }

    fn open_layer_reader(&self, request: &ReadRequest) -> Result<Box<dyn LayerReader>> {
        plenora_io_core::validate_read_projection(&DESCRIPTOR, request)?;
        let (sorgente, builder) = self.apri_verificato()?;

        // Projection pushdown (Fase 2C): se richiesto, leggi SOLO quelle colonne.
        // Con bbox covering, le colonne bbox interne sono SEMPRE proiettate via.
        //
        // Finding #4 follow-up review 2026-08-15: `ProjectionMask::roots`
        // interpreta gli indici come fisici (root Parquet). Gli indici
        // logici dello schema esposto NON coincidono col fisico quando le
        // colonne bbox interne sono intercalate — cosa che i nostri writer
        // non producono ma un GeoParquet esterno puo'. `visible_to_physical`
        // fa la traduzione una volta all'apertura e ogni projection la usa.
        // La maschera esce dal `match` insieme al builder: serve anche alla
        // prevalidazione, che deve guardare le stesse colonne che verranno
        // lette. Ricalcolarla la' sarebbe una seconda verita' che diverge.
        let (builder, out_schema, layer, maschera) = match &request.projected_fields {
            None if self.bbox_covering.is_some() => {
                let mask = ProjectionMask::roots(
                    builder.parquet_schema(),
                    self.visible_to_physical.iter().copied(),
                );
                (
                    builder.with_projection(mask.clone()),
                    self.out_schema.clone(),
                    self.layers[0].clone(),
                    mask,
                )
            }
            None => (
                builder,
                self.out_schema.clone(),
                self.layers[0].clone(),
                ProjectionMask::all(),
            ),
            Some(field_ids) => {
                let ncols = self.out_schema.fields().len();
                let mut logical_idx: Vec<usize> = Vec::new();
                for fid in field_ids {
                    let i = fid.0 as usize;
                    if i >= ncols {
                        if request.projection_mode == ProjectionMode::Required {
                            return Err(campo_fuori_range(*fid));
                        }
                        continue;
                    }
                    if !logical_idx.contains(&i) {
                        logical_idx.push(i);
                    }
                }
                logical_idx.sort_unstable();
                // Schema proiettato: sottoinsieme in ordine originale (geometria già
                // ri-etichettata geoarrow.wkb se presente fra le colonne scelte).
                let fields: Vec<Field> = logical_idx
                    .iter()
                    .map(|&i| self.out_schema.field(i).as_ref().clone())
                    .collect();
                let projected: SchemaRef = Arc::new(Schema::new_with_metadata(
                    fields,
                    self.out_schema.metadata().clone(),
                ));
                // Traduce gli indici logici richiesti nei corrispondenti
                // indici fisici prima di costruire la mask.
                let physical_idx: Vec<usize> = logical_idx
                    .iter()
                    .map(|&i| self.visible_to_physical[i])
                    .collect();
                let mask = ProjectionMask::roots(builder.parquet_schema(), physical_idx);
                let mut layer = self.layers[0].clone();
                layer.contract = DataContract {
                    schema: projected.clone(),
                    geometry: layer.contract.geometry.and_then(|g| {
                        projected.index_of(&g.name).ok().map(|i| {
                            // Indice di colonna di uno schema Arrow: il
                            // cast a u32 non puo' troncare.
                            #[allow(clippy::cast_possible_truncation)]
                            let field_id = FieldId(i as u32);
                            GeometryColumnContract { field_id, ..g }
                        })
                    }),
                };
                (
                    builder.with_projection(mask.clone()),
                    projected,
                    layer,
                    mask,
                )
            }
        };

        // Batch sizing adattivo: combina il tetto righe con target_bytes.
        let batch_size =
            plenora_io_core::effective_batch_rows(out_schema.as_ref(), request.batch_target);
        let builder = builder.with_batch_size(batch_size);
        // Row-group pruning (2C): salta i row group esclusi dalle statistiche
        // min/max (mai filtering riga-per-riga; over-return, mai under-return).
        //
        // I due pruning restituiscono la selezione invece di applicarla: e' lo
        // stesso valore che alimenta il builder e la prevalidazione, quindi le
        // due non possono guardare insiemi diversi. La composizione e' per
        // **intersezione**: un row group viene letto solo se entrambi i criteri
        // lo tengono.
        let gruppi = self.gruppi_da_leggere(&builder, request);

        prevalida_cio_che_il_decoder_leggera(
            &sorgente,
            builder.metadata(),
            &maschera,
            gruppi.as_deref(),
            self.tetto_pagina,
        )?;

        let builder = match gruppi {
            Some(gruppi) => builder.with_row_groups(gruppi),
            None => builder,
        };

        let reader = builder
            .build()
            .map_err(|_| fmt_err(&PublicMessage::Curated("lettura Parquet fallita")))?;
        let reader: Box<dyn LayerReader> = Box::new(GeoParquetReader {
            reader,
            out_schema,
            layer,
        });
        Ok(plenora_io_core::with_cancellation(
            reader,
            request.cancellation.clone(),
        ))
    }
}

struct GeoParquetReader {
    reader: ParquetRecordBatchReader,
    out_schema: SchemaRef,
    layer: LayerContract,
}

impl LayerReader for GeoParquetReader {
    fn contract(&self) -> &LayerContract {
        &self.layer
    }

    fn next_batch(&mut self) -> Result<Option<RecordBatch>> {
        // La barriera copre la decodifica dei buffer, non solo lo schema letto
        // all'apertura: arrow decodifica il batch a ogni `next()`, e un offset
        // oltre la lunghezza dichiarata panica invece di dare un errore.
        //
        // Dopo un panico catturato il reader resta in uno stato non definito.
        // Non e' un problema: il chiamante riceve un errore e il contratto di
        // `LayerReader` non prevede di proseguire dopo un errore.
        let reader = &mut self.reader;
        let prossimo =
            plenora_io_core::driver::leggendo_arrow("parquet", move || match reader.next() {
                None => Ok(None),
                Some(Err(_)) => Err(fmt_err(&PublicMessage::Curated(
                    "batch Parquet non leggibile",
                ))),
                Some(Ok(batch)) => Ok(Some(batch)),
            })?;
        match prossimo {
            None => Ok(None),
            Some(batch) => {
                // Ri-etichetta lo schema (geometria -> geoarrow.wkb) senza toccare
                // i buffer: pass-through delle colonne.
                let options = RecordBatchOptions::new().with_row_count(Some(batch.num_rows()));
                let retagged = RecordBatch::try_new_with_options(
                    self.out_schema.clone(),
                    batch.columns().to_vec(),
                    &options,
                )
                .map_err(|_| fmt_err(&PublicMessage::Curated("re-tag dello schema fallito")))?;
                Ok(Some(retagged))
            }
        }
    }
}

struct GeoParquetWriter {
    staging: StagedFile,
    writer: Option<ArrowWriter<File>>,
    write_schema: SchemaRef,
    geom_idx: usize,
    geom_name: String,
    crs_meta: CrsDaScrivere,
    geometry_types: BTreeSet<(GeometryType, CoordinateDimensions)>,
    wkb_limits: WkbLimits,
    /// La nullabilita' della geometria, che la colonna `bbox` segue.
    geometria_nullable: bool,
}

impl FormatWriter for GeoParquetWriter {
    fn write(&mut self, batch: &RecordBatch) -> Result<()> {
        let geom = batch.column(self.geom_idx);
        accumulate_geometry_types(geom, &mut self.geometry_types, &self.wkb_limits)?;
        // Aggiunge la colonna struct `bbox` del covering, per il pruning.
        let bbox_cols: Vec<ArrayRef> = match geom.as_any().downcast_ref::<BinaryArray>() {
            Some(binaria) => build_bbox_columns(binaria, self.geometria_nullable)?,
            None => vec![new_null_array(
                bbox_field(self.geometria_nullable).data_type(),
                batch.num_rows(),
            )],
        };
        let mut cols: Vec<ArrayRef> = batch.columns().to_vec();
        cols.extend(bbox_cols);
        let aug = RecordBatch::try_new(self.write_schema.clone(), cols).map_err(|_| {
            fmt_err(&PublicMessage::Curated(
                "calcolo delle colonne bbox fallito",
            ))
        })?;
        self.writer
            .as_mut()
            .ok_or_else(|| fmt_err(&PublicMessage::Curated("writer Parquet non disponibile")))?
            .write(&aug)
            .map_err(|_| fmt_err(&PublicMessage::Curated("scrittura Parquet fallita")))
    }

    fn finish(mut self: Box<Self>) -> Result<Published> {
        let mut writer = self.writer.take().ok_or_else(|| {
            fmt_err(&PublicMessage::Curated(
                "writer Parquet non disponibile al finish",
            ))
        })?;
        let geo = build_geo_metadata(&self.geom_name, &self.geometry_types, &self.crs_meta)?;

        // Ultimo cancello prima della pubblicazione, e lo tiene l'autorita'.
        //
        // Le difese a monte sono puntuali -- il CRS qui, i tipi geometrici la',
        // il covering altrove -- e ciascuna copre il caso che conosce. Questa
        // copre il documento **intero**, cioe' anche le combinazioni che nessuno
        // ha previsto. E sta qui, non dopo: un file pubblicato non si ritira, e
        // se il metadato non e' conforme quel file non deve esistere.
        let documento: serde_json::Value = serde_json::from_str(&geo).map_err(|_| {
            fmt_err(&PublicMessage::Curated(
                "metadato `geo` costruito e non rileggibile come JSON",
            ))
        })?;
        schema_ufficiale::valida(&documento, VERSIONE_SCRITTA)?;

        writer.append_key_value_metadata(KeyValue::new("geo".to_owned(), geo));
        writer.close().map_err(|_| {
            fmt_err(&PublicMessage::Curated(
                "chiusura del writer Parquet fallita",
            ))
        })?;
        let (bytes, outcome) = self.staging.publish()?;
        Ok(Published {
            bytes,
            loss: LossReport::default(),
            fidelity: plenora_io_core::FidelityAssessment::lossless(),
            outcome,
        })
    }
}

// --- helpers ---------------------------------------------------------------

// --- row-group pruning (2C) ------------------------------------------------

#[derive(Clone, Copy)]
enum NumericRange {
    Int64(i64, i64),
    Float64(f64, f64),
}

/// Predicato opaco "colonna OP valore" (OP: >, >=, <, <=, =/==).
fn parse_opaque_predicate(s: &str) -> Option<(String, PruningComparison, PruningScalar)> {
    for (symbol, comparison) in [
        (">=", PruningComparison::GreaterThanOrEqual),
        ("<=", PruningComparison::LessThanOrEqual),
        ("==", PruningComparison::Equal),
        (">", PruningComparison::GreaterThan),
        ("<", PruningComparison::LessThan),
        ("=", PruningComparison::Equal),
    ] {
        if let Some((left, right)) = s.split_once(symbol) {
            let column = left.trim().to_owned();
            if column.is_empty() {
                return None;
            }
            let literal = right.trim();
            let value = literal
                .parse::<i64>()
                .map(PruningScalar::Int64)
                .or_else(|_| literal.parse::<f64>().map(PruningScalar::Float64))
                .ok()?;
            if matches!(value, PruningScalar::Float64(value) if !value.is_finite()) {
                return None;
            }
            return Some((column, comparison, value));
        }
    }
    None
}

fn stat_range(stats: &Statistics) -> Option<NumericRange> {
    match stats {
        Statistics::Int32(stats) => {
            let min = i64::from(*stats.min_opt()?);
            let max = i64::from(*stats.max_opt()?);
            (min <= max).then_some(NumericRange::Int64(min, max))
        }
        Statistics::Int64(stats) => {
            let min = *stats.min_opt()?;
            let max = *stats.max_opt()?;
            (min <= max).then_some(NumericRange::Int64(min, max))
        }
        Statistics::Float(stats) => {
            let min = f64::from(*stats.min_opt()?);
            let max = f64::from(*stats.max_opt()?);
            (min.is_finite() && max.is_finite() && min <= max)
                .then_some(NumericRange::Float64(min, max))
        }
        Statistics::Double(stats) => {
            let min = *stats.min_opt()?;
            let max = *stats.max_opt()?;
            (min.is_finite() && max.is_finite() && min <= max)
                .then_some(NumericRange::Float64(min, max))
        }
        _ => None,
    }
}

fn stat_f64_range(stats: &Statistics) -> Option<(f64, f64)> {
    let NumericRange::Float64(min, max) = stat_range(stats)? else {
        return None;
    };
    Some((min, max))
}

fn range_matches(
    range: NumericRange,
    comparison: PruningComparison,
    value: PruningScalar,
) -> Option<bool> {
    match (range, value) {
        (NumericRange::Int64(min, max), PruningScalar::Int64(value)) => {
            Some(comparison_matches(min, max, comparison, value))
        }
        (NumericRange::Float64(min, max), PruningScalar::Float64(value)) if value.is_finite() => {
            Some(comparison_matches(min, max, comparison, value))
        }
        _ => None,
    }
}

fn comparison_matches<T: PartialOrd + Copy>(
    min: T,
    max: T,
    comparison: PruningComparison,
    value: T,
) -> bool {
    match comparison {
        PruningComparison::GreaterThan => max > value,
        PruningComparison::GreaterThanOrEqual => max >= value,
        PruningComparison::LessThan => min < value,
        PruningComparison::LessThanOrEqual => min <= value,
        PruningComparison::Equal => min <= value && value <= max,
    }
}

/// Seleziona i row group che POSSONO soddisfare il predicato (over-return: se le
/// statistiche mancano o il predicato non è riconosciuto, tiene tutto).
fn gruppi_dopo_pruning(
    metadati: &parquet::file::metadata::ParquetMetaData,
    pred: Option<&PruningPredicate>,
    arrow_schema: &Schema,
    schema: &parquet::schema::types::SchemaDescriptor,
) -> Option<Vec<usize>> {
    let predicate = pred?;
    let resolved = match predicate {
        PruningPredicate::NumericComparison {
            field,
            comparison,
            value,
        } => arrow_schema
            .fields()
            .get(field.0 as usize)
            .map(|field| (field.name().clone(), *comparison, *value)),
        PruningPredicate::Opaque(expression) => parse_opaque_predicate(expression),
    };
    let (column, comparison, value) = resolved?;
    let mut matching =
        (0..schema.num_columns()).filter(|&i| schema.column(i).name() == column.as_str());
    let cidx = matching.next()?;
    if matching.next().is_some() {
        return None;
    }
    let md = metadati;
    let mut keep = Vec::new();
    for rg in 0..md.num_row_groups() {
        let keep_it = md
            .row_group(rg)
            .column(cidx)
            .statistics()
            .and_then(stat_range)
            .is_none_or(|range| range_matches(range, comparison, value).unwrap_or(true));
        if keep_it {
            keep.push(rg);
        }
    }
    Some(keep)
}

/// Spatial pruning: tiene i row group il cui estensione bbox interseca l'hint.
/// Over-return (stats mancanti → tiene); mai under-return.
// I nomi `cminx`/`cminy` e `minx`/`miny` sono le componenti canoniche di un
// bounding box: rinominarle per soddisfare `similar_names` renderebbe il codice
// meno leggibile, non più.
#[allow(clippy::similar_names)]
fn gruppi_dopo_pruning_spaziale(
    metadati: &parquet::file::metadata::ParquetMetaData,
    schema: &parquet::schema::types::SchemaDescriptor,
    hint: Option<&Bbox>,
    // Finding #4 follow-up: i nomi delle 4 colonne bbox (xmin, ymin, xmax,
    // ymax) sono passati dal chiamante, che li ha risolti da
    // `covering.bbox` GeoParquet 1.1 o dal fallback storico su `BBOX_COLS`.
    // Non piu' hard-coded: un covering con nomi personalizzati viene ora
    // realmente usato dal pruning.
    covering: Option<&[Vec<String>; 4]>,
) -> Option<Vec<usize>> {
    let (Some(q), Some(covering)) = (hint, covering) else {
        return None;
    };
    // Le foglie si risolvono per **percorso**, non per nome. Con il covering
    // conforme a 1.1 la foglia si chiama `xmin` e vive dentro la colonna struct
    // `bbox`: cercarla per nome la troverebbe -- e troverebbe anche una colonna
    // utente che si chiama `xmin` e non c'entra niente.
    let foglia = |percorso: &[String]| indice_della_foglia(schema, percorso);
    let (Some(cminx), Some(cminy), Some(cmaxx), Some(cmaxy)) = (
        foglia(&covering[0]),
        foglia(&covering[1]),
        foglia(&covering[2]),
        foglia(&covering[3]),
    ) else {
        return None;
    };
    let md = metadati;
    let mut keep = Vec::new();
    for rg in 0..md.num_row_groups() {
        let g = md.row_group(rg);
        // Estensione del row group: min(minx),min(miny) .. max(maxx),max(maxy).
        let ext = (
            g.column(cminx)
                .statistics()
                .and_then(stat_f64_range)
                .map(|(a, _)| a),
            g.column(cminy)
                .statistics()
                .and_then(stat_f64_range)
                .map(|(a, _)| a),
            g.column(cmaxx)
                .statistics()
                .and_then(stat_f64_range)
                .map(|(_, b)| b),
            g.column(cmaxy)
                .statistics()
                .and_then(stat_f64_range)
                .map(|(_, b)| b),
        );
        // Un'estensione con un minimo oltre il proprio massimo non e' un
        // errore: e' un insieme di geometrie che attraversa l'antimeridiano, e
        // la semplice intersezione di rettangoli la leggerebbe al contrario --
        // **escludendo** row group che servono. Un'estensione cosi' non supera
        // la guardia e cade sul ramo `_`, che tiene il gruppo: il pruning si
        // spegne per quel gruppo, e si legge di piu', mai di meno.
        //
        // E' la stessa ragione per cui la conformita' non rifiuta piu' un
        // `bbox` invertito: lo schema lo ammette, e a non saperlo usare siamo
        // noi.
        let keep_it = match ext {
            (Some(minx), Some(miny), Some(maxx), Some(maxy))
                if metadati::interpretabile_per_il_pruning(&[minx, miny, maxx, maxy]) =>
            {
                // Interseca l'hint? (nessuna intersezione = fuori da un lato)
                !(maxx < q.minx || minx > q.maxx || maxy < q.miny || miny > q.maxy)
            }
            _ => true,
        };
        if keep_it {
            keep.push(rg);
        }
    }
    Some(keep)
}

/// Compressione dal `format_options["compression"]` (default snappy). zstd via
/// zstd-sys (unica dep C oltre a GDAL/filegdb), sia in lettura che scrittura.
fn compression_from(opts: &WriteOptions) -> Result<Compression> {
    // Nessun ramo `_`. Prima, un valore fuori elenco diventava snappy in
    // silenzio: chi scriveva `compression=zstsd` otteneva un file valido,
    // compresso in un altro modo, senza mai saperlo. Ora l'unica assenza
    // ammessa e' l'opzione non specificata, che vale il default dichiarato
    // nello schema; ogni altro caso e' gia' stato respinto da `validate_write`,
    // e il ramo finale lo riafferma invece di assorbirlo.
    let Some(valore) = opts.format_options.get("compression").map(String::as_str) else {
        return Ok(Compression::SNAPPY);
    };
    match valore {
        "snappy" => Ok(Compression::SNAPPY),
        "zstd" => Ok(Compression::ZSTD(ZstdLevel::default())),
        "gzip" => Ok(Compression::GZIP(GzipLevel::default())),
        "brotli" => Ok(Compression::BROTLI(BrotliLevel::default())),
        "lz4" => Ok(Compression::LZ4),
        "none" | "uncompressed" => Ok(Compression::UNCOMPRESSED),
        // Il valore non esce: lo schema dichiara `compression` come
        // `Enumerato`, quindi un valore diverso e' gia' stato respinto da
        // `valida_opzioni` con il suo token bounded. Questo ramo e' difensivo.
        _ => Err(PlenoraIoError::redatto(
            plenora_io_model::IoErrorCode::Generic,
            plenora_io_model::ErrorCategory::InvalidConfiguration,
            plenora_io_model::ErrorPhase::Validate,
            plenora_io_model::RemoteEffect::None,
            plenora_io_model::RetryDisposition::Never,
            &PublicMessage::Curated("geoparquet: compressione non riconosciuta"),
        )),
    }
}

/// I metadati `geo` del file, validati per intero.
///
/// `Ok(None)` vuol dire **una cosa sola**: la chiave `geo` non c'e', cioe' il
/// file e' un Parquet semplice e non pretende di essere `GeoParquet`. Per quel
/// caso il driver continua a indovinare la colonna geometria dal nome, che e'
/// un comportamento legittimo e documentato.
///
/// Prima `Ok(None)` ne voleva dire due, e la seconda era il difetto: un `geo`
/// **presente e malformato** finiva anche lui li', e un `GeoParquet` corrotto
/// veniva letto come Parquet semplice -- con la colonna indovinata per nome,
/// che poteva non essere quella dichiarata da `primary_column`. Ora un
/// documento presente viene validato, e se non regge il file e' rifiutato.
fn read_geo_meta(
    builder: &ParquetRecordBatchReaderBuilder<File>,
    accetta_crs_storico: bool,
) -> Result<Option<MetadatiGeo>> {
    let Some(kv) = builder.metadata().file_metadata().key_value_metadata() else {
        return Ok(None);
    };
    let Some(voce) = kv.iter().find(|e| e.key == "geo") else {
        return Ok(None);
    };
    let Some(grezzo) = voce.value.as_deref() else {
        // La chiave c'e' e il valore no: e' un documento che si dichiara e non
        // si scrive, non un file senza metadati.
        return Err(fmt_err(&PublicMessage::Curated(
            "metadato `geo` GeoParquet dichiarato e vuoto",
        )));
    };
    metadati::analizza(grezzo, accetta_crs_storico).map(Some)
}

/// Nome colonna geometria + CRS risolto dai metadati `geo`.
fn resolve_geometry_and_crs(
    schema: &Schema,
    geo: Option<&MetadatiGeo>,
) -> Result<(String, ResolvedCrs)> {
    if let Some(geo) = geo {
        // La colonna che i metadati dichiarano deve esistere nel file. Prima
        // nessuno lo verificava: un `primary_column` che nominava una colonna
        // assente arrivava fino al retag dello schema, dove non trovava niente
        // da ri-etichettare e la geometria spariva senza un errore.
        if schema.index_of(&geo.nome_primaria).is_err() {
            return Err(fmt_err(&PublicMessage::Curated(
                "metadato `geo` GeoParquet che dichiara una `primary_column` assente dallo schema",
            )));
        }
        let crs = crs_from(Some(geo))?;
        return Ok((geo.nome_primaria.clone(), crs));
    }

    // Nessun metadato `geo`: e' un Parquet semplice, e la colonna geometria si
    // riconosce dal nome. Resta il comportamento di sempre.
    let primary = ["geometry", "geom", "wkb"]
        .iter()
        .find(|n| schema.index_of(n).is_ok())
        .map(std::string::ToString::to_string)
        .ok_or_else(|| {
            fmt_err(&PublicMessage::Curated(
                "nessuna colonna geometria: non è GeoParquet",
            ))
        })?;
    Ok((primary, ResolvedCrs::wgs84()))
}

/// Il CRS della colonna primaria, nei tre stati che la specifica distingue.
///
/// * **assente** -- lo schema dice `OGC:CRS84`, ed e' l'unico caso in cui il
///   driver puo' assumerlo;
/// * **`null`** -- il file dichiara di **non** avere un CRS. Trasformarlo in
///   CRS84, come faceva la prima stesura, e' mettere in bocca a chi ha scritto
///   il file un'affermazione che non ha fatto: chi legge crederebbe di avere
///   coordinate in WGS84 dove nessuno lo ha detto;
/// * **documento** -- il PROJJSON dichiarato.
fn crs_from(geo: Option<&MetadatiGeo>) -> Result<ResolvedCrs> {
    let Some(geo) = geo else {
        // Nessun metadato `geo`: e' un Parquet semplice, e la colonna geometria
        // e' stata riconosciuta dal nome. Vale cio' che valeva.
        return Ok(ResolvedCrs::wgs84());
    };
    match &geo.primaria.crs {
        metadati::Crs::Assente => Ok(ResolvedCrs::wgs84()),
        // Nessun identificatore, nessuna definizione, natura ignota: e' la
        // rappresentazione di «non lo so», e non ne esiste un'altra.
        metadati::Crs::Nullo => Ok(ResolvedCrs::new(None, CrsKind::Unknown, None)),
        // Accettato per compatibilita': l'identificatore e' cio' che quel file
        // dichiarava, e si conserva. Il contratto dice altrove che il file non
        // era conforme, cosi' chi legge non scambia la cortesia per conformita'.
        metadati::Crs::StoricoSoloIdentificatore(id) => {
            let genere = crs_kind_for_authority_id(id);
            Ok(ResolvedCrs::new(Some(id.clone()), genere, None))
        }
        metadati::Crs::Documento(v) => {
            let id = v.get("id").and_then(|i| {
                let a = i.get("authority").and_then(|a| a.as_str())?;
                let code = i.get("code").map(|c| match c {
                    serde_json::Value::Number(n) => n.to_string(),
                    serde_json::Value::String(s) => s.clone(),
                    other => other.to_string(),
                })?;
                Some(format!("{a}:{code}"))
            });
            let definition = v.to_string();
            let Some(id) = id else {
                let raw = RawCrs::new(
                    definition,
                    v.get("id")
                        .and_then(|i| i.get("authority"))
                        .and_then(|a| a.as_str())
                        .map(str::to_owned),
                );
                return Err(PlenoraIoError::crs_non_risolto_redatto("geoparquet", &raw));
            };
            let kind = if crs_kind_for_authority_id(&id) == CrsKind::Geographic
                || v.get("type").and_then(|t| t.as_str()) == Some("GeographicCRS")
            {
                CrsKind::Geographic
            } else if v.get("type").and_then(|t| t.as_str()) == Some("ProjectedCRS") {
                CrsKind::Projected
            } else {
                CrsKind::Unknown
            };
            Ok(ResolvedCrs::new(Some(id), kind, Some(definition)))
        }
    }
}

fn apply_geo_column_metadata(contract: &mut GeometryColumnContract, geo: Option<&MetadatiGeo>) {
    let Some(geo) = geo else {
        return;
    };
    let colonna = &geo.primaria;
    contract
        .native_metadata
        .insert("geoparquet.column".to_owned(), colonna.grezza.to_string());
    // La versione che il documento dichiara, validata: e' l'informazione che
    // dice a valle con quali regole quel metadato va letto, e non c'era.
    contract
        .native_metadata
        .insert("geoparquet.version".to_owned(), geo.versione.to_owned());
    // Se il file e' entrato dalla via di compatibilita', il contratto lo dice.
    // Una deroga che non si vede diventa il comportamento normale.
    if geo.conformita == metadati::Conformita::CrsStoricoSoloIdentificatore {
        contract.native_metadata.insert(
            "geoparquet.compatibilita".to_owned(),
            "crs_storico_solo_identificatore_non_conforme".to_owned(),
        );
    }
    // Le altre colonne geometriche del file. Un GeoParquet puo' averne piu' di
    // una, e finora il contratto non nominava quelle che non erano la primaria:
    // un consumatore non aveva modo di sapere che esistessero.
    if !geo.secondarie.is_empty() {
        let altre: Vec<&str> = geo.secondarie.keys().map(String::as_str).collect();
        contract
            .native_metadata
            .insert("geoparquet.altre_colonne".to_owned(), altre.join(","));
    }
    // I campi che la validazione legge arrivano **fino al contratto**.
    //
    // Validarli e poi buttarli via sarebbe stata la meta' del lavoro: chi legge
    // a valle non saprebbe che quel file dichiara bordi planari, un
    // orientamento degli anelli, un'epoca delle coordinate o un riquadro di
    // ingombro. `bordi` e' sempre `Planari` -- gli altri valori non arrivano
    // qui, perche' il rifiuto li ferma a monte -- e proprio per questo dirlo ha
    // senso: e' l'unica cosa che quel campo puo' valere in un file che abbiamo
    // accettato.
    contract.native_metadata.insert(
        "geoparquet.edges".to_owned(),
        match colonna.bordi {
            metadati::Bordi::Planari => "planar".to_owned(),
        },
    );
    if let Some(orientamento) = colonna.orientamento {
        contract.native_metadata.insert(
            "geoparquet.orientation".to_owned(),
            match orientamento {
                metadati::Orientamento::Antiorario => "counterclockwise".to_owned(),
            },
        );
    }
    if let Some(riquadro) = colonna.bbox.as_ref() {
        let numeri: Vec<String> = riquadro.iter().map(ToString::to_string).collect();
        contract
            .native_metadata
            .insert("geoparquet.bbox".to_owned(), numeri.join(","));
    }
    if let Some(epoca) = colonna.epoch {
        contract
            .native_metadata
            .insert("geoparquet.epoch".to_owned(), epoca.to_string());
    }
    let mut dimensions = BTreeSet::new();
    // I tipi arrivano gia' validati: un'etichetta fuori dalla specifica ha
    // fermato il file, invece di sparire da un `filter_map` e lasciare il
    // contratto piu' povero senza che nulla lo dicesse.
    for (geometry_type, dimension) in &colonna.tipi {
        if !contract.geometry_types.contains(geometry_type) {
            contract.geometry_types.push(*geometry_type);
        }
        dimensions.insert(*dimension);
    }
    if !contract.geometry_types.is_empty() {
        let geometry_types = std::mem::take(&mut contract.geometry_types);
        contract.set_exact_geometry_types(geometry_types);
    }
    if dimensions.len() == 1 {
        contract.dimensions = dimensions
            .first()
            .copied()
            .unwrap_or(CoordinateDimensions::Unknown);
    }
    contract.srid = contract
        .crs
        .id()
        .and_then(|id| id.strip_prefix("EPSG:"))
        .and_then(|code| code.parse().ok());
}

// --- bbox covering (spatial pruning, 2C) -----------------------------------

/// Colonne bbox interne (covering "plenora"): 4 f64 flat per row, con statistiche
/// min/max Parquet per row group → pruning spaziale.
/// L'indice della foglia Parquet che quel percorso designa.
///
/// Il confronto e' sui **segmenti** del percorso della foglia, non sul suo
/// nome: `["bbox", "xmin"]` designa la foglia `xmin` dentro la colonna struct
/// `bbox`, e non una qualunque colonna che si chiami `xmin`.
fn indice_della_foglia(
    schema: &parquet::schema::types::SchemaDescriptor,
    percorso: &[String],
) -> Option<usize> {
    (0..schema.num_columns()).find(|&i| schema.column(i).path().parts() == percorso)
}

/// Riconcilia il metadato `geo` con lo schema **fisico** del file.
///
/// Il metadato e' un'affermazione sul file, e finora nessuno la confrontava col
/// file: si verificava che la `primary_column` esistesse, e li' finiva. Una
/// colonna secondaria inventata, una geometria dichiarata su una colonna di
/// interi, un `covering` che nomina percorsi inesistenti o sparsi su strutture
/// diverse -- tutto passava, e le radici del covering venivano perfino tolte
/// dallo schema esposto **prima** che qualcuno controllasse che esistessero.
///
/// La specifica chiede il contrario: i percorsi devono esistere, appartenere
/// alla stessa bounding group e avere la forma fisica che il § *Bounding Box
/// Columns* prescrive -- nomi, ordine, tipo e ripetizione. Un file che dice di
/// se' qualcosa che non e' vero e' un file malformato, e va rifiutato prima di
/// esporne uno schema che nessuno potra' leggere.
///
/// # Errors
///
/// `Format` se una colonna dichiarata non esiste o non e' binaria, o se il
/// `covering` non regge il confronto con lo schema fisico.
fn riconcilia_con_lo_schema_fisico(
    arrow: &Schema,
    parquet: &parquet::schema::types::SchemaDescriptor,
    geo: &MetadatiGeo,
) -> Result<()> {
    // La primaria e le secondarie si trattano allo stesso modo: sono colonne
    // geometriche dichiarate, e una dichiarazione falsa non e' meno falsa
    // perche' riguarda una colonna secondaria.
    for (nome, colonna) in
        std::iter::once((&geo.nome_primaria, &geo.primaria)).chain(geo.secondarie.iter())
    {
        colonna_geometrica_presente(arrow, nome)?;
        if let Some(percorsi) = colonna.covering.as_ref() {
            covering_riconciliato(parquet, nome, percorsi)?;
        }
    }
    Ok(())
}

/// Una colonna geometrica dichiarata esiste, ed e' binaria.
///
/// L'unica codifica che questo driver accetta e' `WKB`, e `WKB` sta in una
/// colonna di byte. Dichiararla su una colonna di interi o di stringhe e'
/// un'incoerenza fra il metadato e i dati, non una variante da tollerare.
fn colonna_geometrica_presente(arrow: &Schema, nome: &str) -> Result<()> {
    let indice = arrow.index_of(nome).map_err(|_| {
        fmt_err(&PublicMessage::Curated(
            "metadato `geo` GeoParquet che dichiara una colonna geometrica assente dallo schema del file",
        ))
    })?;
    if !matches!(
        arrow.field(indice).data_type(),
        DataType::Binary | DataType::LargeBinary | DataType::BinaryView
    ) {
        return Err(fmt_err(&PublicMessage::Curated(
            "colonna dichiarata geometrica `WKB` e non binaria nello schema del file",
        )));
    }
    Ok(())
}

/// Gli spigoli di una bounding group, nell'ordine che la specifica prescrive.
///
/// Sono due forme e non una: `zmin`/`zmax` sono ammessi, e vanno **in mezzo**,
/// non in coda. Un file tridimensionale conforme non e' un file da rifiutare.
const FORME_DEL_COVERING: [&[&str]; 2] = [
    &["xmin", "ymin", "xmax", "ymax"],
    &["xmin", "ymin", "zmin", "xmax", "ymax", "zmax"],
];

/// Il `covering` regge il confronto con lo schema fisico del file?
///
/// Qui si verifica cio' che lo schema JSON non puo' vedere. Lo schema conosce i
/// **percorsi** dichiarati nel metadato; il file ha una struct vera, con dei
/// figli veri, in un ordine vero, di un tipo vero, e con una ripetizione sua.
/// La specifica 1.1 pretende (§ *Bounding Box Columns*):
///
/// * la colonna sta **alla radice**, e non dentro un altro gruppo;
/// * i figli si chiamano `xmin, ymin, xmax, ymax`, **in quest'ordine**, oppure
///   `xmin, ymin, zmin, xmax, ymax, zmax` se c'e' la terza dimensione: niente
///   figli in piu', niente `zmin` da solo, niente ordine diverso;
/// * i figli sono `FLOAT` **oppure** `DOUBLE`, e tutti dello stesso tipo;
/// * la ripetizione della colonna e' **quella della geometria**: un riquadro
///   se e solo se c'e' una geometria.
///
/// La prima stesura guardava solo che i percorsi esistessero, che avessero la
/// stessa radice, che le foglie fossero `DOUBLE` e non ripetute. Cercare le
/// foglie per percorso non dice niente sul loro ordine -- una struct ordinata
/// `ymin, xmin, xmax, ymax` le contiene tutte -- `max_rep_level` esclude le
/// liste ma lascia passare una geometria opzionale con un riquadro
/// obbligatorio, e pretendere `DOUBLE` rifiutava come malformato un file che la
/// specifica dichiara valido.
fn covering_riconciliato(
    parquet: &parquet::schema::types::SchemaDescriptor,
    nome_geometria: &str,
    percorsi: &[Vec<String>; 4],
) -> Result<()> {
    use parquet::basic::Type as TipoFisico;

    let malformato = |messaggio: &'static str| fmt_err(&PublicMessage::Curated(messaggio));

    // I quattro percorsi dichiarati designano la stessa colonna, e ciascuno
    // designa il proprio spigolo. Sono due segmenti perche' lo schema 1.1 non
    // ne ammette altri, ed e' anche il modo in cui la specifica dice «alla
    // radice, non dentro un altro gruppo».
    let mut radice: Option<&String> = None;
    for (percorso, spigolo) in percorsi.iter().zip(BBOX_SPIGOLI) {
        let [prima, seconda] = percorso.as_slice() else {
            return Err(malformato(
                "covering GeoParquet con un percorso che non ha due segmenti: la colonna sta alla radice",
            ));
        };
        if seconda != spigolo {
            return Err(malformato(
                "covering GeoParquet con uno spigolo che designa una foglia di un altro nome",
            ));
        }
        match radice {
            None => radice = Some(prima),
            Some(attesa) if attesa == prima => {}
            Some(_) => {
                return Err(malformato(
                    "covering GeoParquet con gli spigoli su colonne diverse: la specifica ne vuole una sola",
                ))
            }
        }
    }
    let Some(radice) = radice else {
        return Err(malformato("covering GeoParquet senza spigoli"));
    };

    // La struct vera, presa dalla radice dello schema Parquet: e' li' che
    // stanno l'ordine dei figli, il loro tipo e la ripetizione, che il
    // metadato non porta e la ricerca per percorso non guarda.
    let radice_dello_schema = parquet.root_schema();
    let campo = |nome: &str| {
        radice_dello_schema
            .get_fields()
            .iter()
            .find(|campo| campo.name() == nome)
    };
    let Some(gruppo) = campo(radice) else {
        return Err(malformato(
            "covering GeoParquet che dichiara una colonna assente dalla radice dello schema del file",
        ));
    };
    if !gruppo.is_group() {
        return Err(malformato(
            "covering GeoParquet che designa una colonna che non e' un gruppo",
        ));
    }

    // Nomi **e ordine**, insieme: sono la stessa affermazione, e verificarne
    // uno solo la lascia mezza vera.
    let figli: Vec<&str> = gruppo.get_fields().iter().map(|f| f.name()).collect();
    if !FORME_DEL_COVERING.contains(&figli.as_slice()) {
        return Err(malformato(
            "covering GeoParquet con figli diversi da `xmin, ymin, xmax, ymax` -- o dalla forma con `zmin` e `zmax` -- nell'ordine prescritto",
        ));
    }

    // `FLOAT` **oppure** `DOUBLE`, e tutti lo stesso: la specifica ammette le
    // due precisioni e vieta di mescolarle. Le statistiche di un `FLOAT` si
    // allargano a `f64` senza perdere niente, quindi il pruning le usa come
    // quelle di un `DOUBLE`.
    let mut tipo_comune: Option<TipoFisico> = None;
    for figlio in gruppo.get_fields() {
        if !figlio.is_primitive() {
            return Err(malformato(
                "spigolo del covering GeoParquet che non e' una colonna di valori",
            ));
        }
        let tipo = figlio.get_physical_type();
        if tipo != TipoFisico::FLOAT && tipo != TipoFisico::DOUBLE {
            return Err(malformato(
                "spigolo del covering GeoParquet che non e' `FLOAT` ne' `DOUBLE`",
            ));
        }
        match tipo_comune {
            None => tipo_comune = Some(tipo),
            Some(atteso) if atteso == tipo => {}
            Some(_) => {
                return Err(malformato(
                    "covering GeoParquet con spigoli di precisione diversa: la specifica li vuole tutti dello stesso tipo",
                ))
            }
        }
    }

    // «Un riquadro se e solo se c'e' una geometria» e' un'affermazione sulla
    // ripetizione, non sui valori: se la geometria e' opzionale e il riquadro
    // obbligatorio, il file promette un riquadro anche dove geometria non ce
    // n'e'. Confrontarle e' l'unico modo di accorgersene leggendo lo schema.
    let Some(geometria) = campo(nome_geometria) else {
        return Err(malformato(
            "colonna geometria assente dalla radice dello schema del file",
        ));
    };
    // `None` soltanto per la radice dello schema, che non ha ripetizione: qui
    // sono due suoi figli, e un ripiego a `REQUIRED` sarebbe un valore inventato
    // proprio nel confronto che deve dire la verita'. Il caso impossibile cade
    // nello stesso rifiuto invece di aprirsi un ramo suo.
    let ripetizione = |campo: &parquet::schema::types::Type| {
        let info = campo.get_basic_info();
        info.has_repetition().then(|| info.repetition())
    };
    match (ripetizione(gruppo), ripetizione(geometria)) {
        (Some(del_covering), Some(della_geometria)) if del_covering == della_geometria => {}
        _ => {
            return Err(malformato(
                "covering GeoParquet con una ripetizione diversa da quella della geometria: il riquadro c'e' se e solo se c'e' la geometria",
            ))
        }
    }
    Ok(())
}

/// Quel percorso esiste nello schema Parquet?
fn percorso_presente(schema: &Schema, percorso: &[String]) -> bool {
    let Some((radice, resto)) = percorso.split_first() else {
        return false;
    };
    let Ok(indice) = schema.index_of(radice) else {
        return false;
    };
    let mut campo = schema.field(indice).clone();
    for segmento in resto {
        let DataType::Struct(figli) = campo.data_type() else {
            return false;
        };
        let Some(trovato) = figli.iter().find(|f| f.name() == segmento) else {
            return false;
        };
        campo = (**trovato).clone();
    }
    true
}

/// Le quattro colonne piatte che questo writer emetteva **prima** di S10.
///
/// Restano qui perche' i file scritti allora esistono, e perche'
/// `bbox_legacy_by_name` -- l'opt-in che gia' c'era -- serve a riconoscerle. Il
/// writer non le produce piu': non erano un covering `GeoParquet` 1.1 valido.
const BBOX_COLS: [&str; 4] = ["_bbox_minx", "_bbox_miny", "_bbox_maxx", "_bbox_maxy"];

/// Il nome della colonna struct che porta il covering conforme.
const BBOX_STRUCT: &str = "bbox";

/// Gli spigoli, nell'ordine in cui lo schema e il pruning li nominano.
const BBOX_SPIGOLI: [&str; 4] = ["xmin", "ymin", "xmax", "ymax"];

fn is_bbox_col(name: &str) -> bool {
    name == BBOX_STRUCT || BBOX_COLS.contains(&name)
}

/// La colonna `bbox` del covering: una struct con quattro figli `FLOAT64`.
///
/// # Perche' una struct
///
/// Lo schema 1.1.0 pretende che ogni spigolo del `covering.bbox` sia un
/// percorso di **due** segmenti, il secondo uguale al nome dello spigolo:
/// `["bbox", "xmin"]`. Quattro colonne piatte non possono esprimerlo, e i file
/// che questo writer produceva -- `["_bbox_minx"]` -- non erano `GeoParquet` 1.1
/// validi, benche' il documento si dichiarasse tale.
///
/// # Nullabilita'
///
/// Segue la geometria: dove non c'e' geometria non c'e' riquadro, e dichiarare
/// non-nullo un campo che sara' nullo sarebbe una promessa che i dati smentono
/// alla prima riga senza geometria.
fn bbox_field(geometria_nullable: bool) -> Field {
    Field::new(
        BBOX_STRUCT,
        DataType::Struct(
            BBOX_SPIGOLI
                .iter()
                .map(|spigolo| Field::new(*spigolo, DataType::Float64, geometria_nullable))
                .collect(),
        ),
        geometria_nullable,
    )
}

fn upd(bb: &mut [f64; 4], x: f64, y: f64) {
    if x < bb[0] {
        bb[0] = x;
    }
    if y < bb[1] {
        bb[1] = y;
    }
    if x > bb[2] {
        bb[2] = x;
    }
    if y > bb[3] {
        bb[3] = y;
    }
}

fn rd_u32(b: &[u8], off: &mut usize, le: bool) -> Option<u32> {
    let s = b.get(*off..*off + 4)?;
    *off += 4;
    let a = [s[0], s[1], s[2], s[3]];
    Some(if le {
        u32::from_le_bytes(a)
    } else {
        u32::from_be_bytes(a)
    })
}

fn rd_f64(b: &[u8], off: &mut usize, le: bool) -> Option<f64> {
    let s = b.get(*off..*off + 8)?;
    *off += 8;
    let mut a = [0u8; 8];
    a.copy_from_slice(s);
    Some(if le {
        f64::from_le_bytes(a)
    } else {
        f64::from_be_bytes(a)
    })
}

fn scan_wkb(bytes: &[u8], off: &mut usize, bb: &mut [f64; 4], depth: u32) -> Option<()> {
    if depth > 32 {
        return None;
    }
    let le = match *bytes.get(*off)? {
        0 => false,
        1 => true,
        _ => return None,
    };
    *off += 1;
    let raw = rd_u32(bytes, off, le)?;
    let (base, has_z, has_m, srid) = if raw & 0xE000_0000 != 0 {
        (
            raw & 0x1FFF_FFFF,
            raw & 0x8000_0000 != 0,
            raw & 0x4000_0000 != 0,
            raw & 0x2000_0000 != 0,
        )
    } else {
        let dimension = raw / 1000;
        if dimension > 3 {
            return None;
        }
        (
            raw % 1000,
            dimension == 1 || dimension == 3,
            dimension == 2 || dimension == 3,
            false,
        )
    };
    if srid {
        rd_u32(bytes, off, le)?;
    }
    let scan_coord = |off: &mut usize, bb: &mut [f64; 4]| -> Option<()> {
        let x = rd_f64(bytes, off, le)?;
        let y = rd_f64(bytes, off, le)?;
        if has_z {
            rd_f64(bytes, off, le)?;
        }
        if has_m {
            rd_f64(bytes, off, le)?;
        }
        upd(bb, x, y);
        Some(())
    };
    match base {
        1 => {
            scan_coord(off, bb)?;
        }
        2 => {
            let count = rd_u32(bytes, off, le)?;
            for _ in 0..count {
                scan_coord(off, bb)?;
            }
        }
        3 => {
            let rings = rd_u32(bytes, off, le)?;
            for _ in 0..rings {
                let npts = rd_u32(bytes, off, le)?;
                for _ in 0..npts {
                    scan_coord(off, bb)?;
                }
            }
        }
        4..=7 => {
            let count = rd_u32(bytes, off, le)?;
            for _ in 0..count {
                scan_wkb(bytes, off, bb, depth + 1)?;
            }
        }
        _ => return None,
    }
    Some(())
}

/// Bounding box 2D da WKB senza costruire geometrie. `None` se non-2D o malformato
/// (robusto al fuzzing: nessun panic, nessun loop illimitato).
#[doc(hidden)] // esposto solo per il fuzzer (plenora-fuzz)
#[must_use]
pub fn wkb_bbox(bytes: &[u8]) -> Option<[f64; 4]> {
    let mut bb = [
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    ];
    let mut off = 0usize;
    scan_wkb(bytes, &mut off, &mut bb, 0)?;
    if off != bytes.len() {
        return None;
    }
    if bb.iter().all(|v| v.is_finite()) {
        Some(bb)
    } else {
        None
    }
}

/// Costruisce le 4 colonne bbox per un batch, dalla colonna geometria WKB.
// `minx`/`miny` e `maxx`/`maxy` sono le componenti canoniche di un bounding
// box: rinominarle per soddisfare `similar_names` peggiorerebbe la leggibilità.
#[allow(clippy::similar_names)]
fn build_bbox_columns(geom: &BinaryArray, geometria_nullable: bool) -> Result<Vec<ArrayRef>> {
    let rows = geom.len();
    // Un array di quattro colonne, non quattro variabili in una tupla: sono i
    // quattro spigoli nell'ordine di `BBOX_SPIGOLI`, e il tipo lo dice.
    let mut colonne: [Vec<Option<f64>>; 4] = std::array::from_fn(|_| Vec::with_capacity(rows));
    for row in 0..rows {
        let bbox = if geom.is_null(row) {
            None
        } else {
            wkb_bbox(geom.value(row))
        };
        for (spigolo, valori) in colonne.iter_mut().enumerate() {
            valori.push(bbox.map(|bbox| bbox[spigolo]));
        }
    }
    // I quattro figli entrano in una struct sola, e la struct porta **la bitmap
    // di validita' della geometria**.
    //
    // Metterla soltanto nei figli non basta, ed era il difetto: `StructArray::from`
    // costruisce la struct con `nulls: None`, cosi' il `bbox` risultava
    // **presente** anche sulle righe senza geometria. GeoParquet 1.1 chiede un
    // riquadro se e solo se la geometria c'e', e un `bbox` presente con quattro
    // figli nulli afferma un'altra cosa: afferma che il riquadro esiste e non si
    // conosce.
    let campi: Vec<Arc<Field>> = BBOX_SPIGOLI
        .into_iter()
        .map(|spigolo| Arc::new(Field::new(spigolo, DataType::Float64, geometria_nullable)))
        .collect();
    let valori: Vec<ArrayRef> = colonne
        .into_iter()
        .map(|valori| Arc::new(Float64Array::from(valori)) as ArrayRef)
        .collect();
    let validita = geometria_nullable.then(|| {
        (0..rows)
            .map(|row| !geom.is_null(row))
            .collect::<NullBuffer>()
    });
    let struttura = StructArray::try_new(campi.into(), valori, validita).map_err(|_| {
        fmt_err(&PublicMessage::Curated(
            "costruzione della colonna del covering fallita",
        ))
    })?;
    Ok(vec![Arc::new(struttura)])
}

/// Estrae dai metadati `geo.columns.<primary>.covering.bbox` la lista dei
/// nomi delle colonne bbox del covering spaziale (finding #4 review
/// 2026-08-15). Un file scritto dal writer post-fix dichiara esplicitamente
/// il mapping; per i file legacy (anche quelli emessi dai nostri writer
/// precedenti) il chiamante puo' fare fallback ai nomi convenzionali
/// `BBOX_COLS`.
/// I quattro **percorsi** del covering dichiarato, gia' validati da `metadati`.
///
/// Percorsi e non nomi: lo schema 1.1.0 pretende due segmenti per spigolo, cioe'
/// una colonna struct. `metadati` restituisce `None` quando il covering non c'e'
/// e quando sta in un documento 1.0.0, dove la specifica non gli attribuisce
/// significato.
fn covering_bbox_paths(geo: Option<&MetadatiGeo>) -> Option<[Vec<String>; 4]> {
    geo.and_then(|g| g.primaria.covering.clone())
}

/// I nomi di colonna **radice** che un covering occupa.
///
/// Sono cio' che il retag toglie dallo schema esposto: con la forma conforme e'
/// un nome solo -- la colonna struct -- e con quella storica sono quattro.
fn radici_del_covering(percorsi: &[Vec<String>; 4]) -> Vec<String> {
    let mut radici: Vec<String> = Vec::with_capacity(4);
    for percorso in percorsi {
        if let Some(radice) = percorso.first() {
            if !radici.contains(radice) {
                radici.push(radice.clone());
            }
        }
    }
    radici
}

/// Ricostruisce lo schema marcando la geometria come `geoarrow.wkb`+`crs` ed
/// ESCLUDENDO le colonne dichiarate come covering in `covering_names`.
///
/// Finding #4 follow-up follow-up review 2026-08-15: la funzione non ha
/// piu' un fallback per-nome implicito. Il caller (`open()`) decide se
/// includere i nomi convenzionali `BBOX_COLS` sulla base di
/// `format_options["bbox_legacy_by_name"]`. `None` significa "non
/// strippare nulla" — cosi' un file esterno con colonne omonime NON
/// perde dati.
fn retag_schema(
    schema: &Schema,
    geom_name: &str,
    crs: &ResolvedCrs,
    covering_names: Option<&[String]>,
) -> SchemaRef {
    let is_internal = |name: &str| -> bool {
        covering_names.is_some_and(|names| names.iter().any(|declared| declared == name))
    };
    let fields: Vec<Field> = schema
        .fields()
        .iter()
        .filter(|f| !is_internal(f.name()))
        .map(|f| {
            if f.name() == geom_name {
                let mut md = f.metadata().clone();
                md.insert(
                    ARROW_EXTENSION_NAME_KEY.to_owned(),
                    GEOARROW_WKB_EXTENSION.to_owned(),
                );
                if let Some(id) = &crs.id {
                    md.insert(GEO_CRS_KEY.to_owned(), id.clone());
                }
                f.as_ref().clone().with_metadata(md)
            } else {
                f.as_ref().clone()
            }
        })
        .collect();
    Arc::new(Schema::new_with_metadata(fields, schema.metadata().clone()))
}

/// Trova la colonna geometria (`geoarrow.wkb`) in uno schema in scrittura.
fn geometry_field(schema: &Schema) -> Result<(usize, String, Option<String>)> {
    for (i, f) in schema.fields().iter().enumerate() {
        if plenora_io_model::geometry::is_geometry_field(f) {
            let crs = f.metadata().get(GEO_CRS_KEY).cloned();
            return Ok((i, f.name().clone(), crs));
        }
    }
    Err(fmt_err(&PublicMessage::Curated(
        "nessuna colonna geometria geoarrow.wkb nel contratto",
    )))
}

fn geometry_type_name(geometry_type: GeometryType) -> Result<&'static str> {
    match geometry_type {
        GeometryType::Point => Ok("Point"),
        GeometryType::LineString => Ok("LineString"),
        GeometryType::Polygon => Ok("Polygon"),
        GeometryType::MultiPoint => Ok("MultiPoint"),
        GeometryType::MultiLineString => Ok("MultiLineString"),
        GeometryType::MultiPolygon => Ok("MultiPolygon"),
        GeometryType::GeometryCollection => Ok("GeometryCollection"),
        other => Err(fmt_err(&PublicMessage::CuratedPair(
            "tipo geometrico non supportato dal profilo GeoParquet corrente:",
            other.canonical_name(),
        ))),
    }
}

fn geometry_type_label(
    geometry_type: GeometryType,
    dimensions: CoordinateDimensions,
) -> Result<String> {
    // Il pattern dello schema, in entrambe le versioni, e'
    // `...( Z)?$`: **`" M"` e `" ZM"` non esistono in GeoParquet**. Questo
    // writer li emetteva, cioe' produceva documenti che lo schema ufficiale
    // rifiuta -- e il nostro lettore li accettava, chiudendo il cerchio su
    // un'incompatibilita' che nessuno dei due vedeva.
    //
    // Il rifiuto e' di **funzionalita' non supportata**, non di formato: i dati
    // sono corretti, e a non saperli rappresentare in questo formato siamo noi.
    // Omettere il suffisso sarebbe peggio: il file direbbe che quelle geometrie
    // sono XY, e la misura M sparirebbe senza che nessuno lo dichiari.
    let suffix = match dimensions {
        CoordinateDimensions::Xy => "",
        CoordinateDimensions::Xyz => " Z",
        // Difesa di ultima istanza, e **irraggiungibile** dall'API pubblica: chi
        // dichiara XYM nel contratto non apre nemmeno il writer -- lo ferma la
        // capability del formato -- e chi lo tace si vede rifiutare la riga dal
        // controllo di dimensionalita' del contratto. La sonda
        // `una_geometria_con_la_misura_m_non_e_scrivibile` prova tutt'e due le
        // vie; questa riga resta perche' il `match` deve trattare il caso, e
        // trattarlo restituendo un'etichetta che lo schema rifiuta sarebbe
        // peggio.
        CoordinateDimensions::Xym | CoordinateDimensions::Xyzm => {
            return Err(PlenoraIoError::non_supportato_redatto(
                &PublicMessage::CuratedPair(
                    "GeoParquet non rappresenta la misura M: sono scrivibili soltanto",
                    "XY e XYZ",
                ),
            ))
        }
        CoordinateDimensions::Unknown => {
            return Err(fmt_err(&PublicMessage::Curated(
                "dimensionalità WKB ignota",
            )))
        }
    };
    Ok(format!("{}{suffix}", geometry_type_name(geometry_type)?))
}

fn accumulate_geometry_types(
    col: &dyn Array,
    out: &mut BTreeSet<(GeometryType, CoordinateDimensions)>,
    limits: &WkbLimits,
) -> Result<()> {
    if let Some(a) = col.as_any().downcast_ref::<BinaryArray>() {
        for i in 0..a.len() {
            if !a.is_null(i) {
                let inspection = inspect_wkb(a.value(i), limits)?;
                out.insert((inspection.geometry_type, inspection.dimensions));
            }
        }
    } else if let Some(a) = col.as_any().downcast_ref::<LargeBinaryArray>() {
        for i in 0..a.len() {
            if !a.is_null(i) {
                let inspection = inspect_wkb(a.value(i), limits)?;
                out.insert((inspection.geometry_type, inspection.dimensions));
            }
        }
    }
    Ok(())
}

fn build_geo_metadata(
    geom_name: &str,
    types: &BTreeSet<(GeometryType, CoordinateDimensions)>,
    crs: &CrsDaScrivere,
) -> Result<String> {
    let mut geometry_types = types
        .iter()
        .map(|(geometry_type, dimensions)| geometry_type_label(*geometry_type, *dimensions))
        .collect::<Result<Vec<_>>>()?;
    // Mantiene l'ordine lessicografico emesso dal precedente BTreeSet<String>:
    // l'ottimizzazione non deve cambiare neppure incidentalmente il metadato.
    geometry_types.sort_unstable();
    // Finding #4: covering GeoParquet 1.1 dichiarato in modo esplicito. Il
    // lettore usa questa dichiarazione per identificare le colonne bbox
    // interne invece di dipendere dai soli nomi. La forma segue lo schema
    // pubblico `covering.bbox.<edge>` di GeoParquet
    // (https://geoparquet.org/releases/v1.1.0/) ed e' additiva rispetto ai
    // consumer che ignorano l'attributo.
    let mut column = serde_json::json!({
        "encoding": "WKB",
        "geometry_types": geometry_types,
        // Percorsi di **due** segmenti, il secondo uguale al nome dello
        // spigolo: e' l'unica forma che lo schema 1.1.0 ammette, e designa i
        // figli della colonna struct `bbox`.
        "covering": {
            "bbox": {
                "xmin": [BBOX_STRUCT, BBOX_SPIGOLI[0]],
                "ymin": [BBOX_STRUCT, BBOX_SPIGOLI[1]],
                "xmax": [BBOX_STRUCT, BBOX_SPIGOLI[2]],
                "ymax": [BBOX_STRUCT, BBOX_SPIGOLI[3]],
            }
        },
    });
    match crs {
        // Assente: la specifica dice CRS84, ed e' cio' che intendiamo.
        CrsDaScrivere::Omesso => {}
        // Presente e nullo: dichiariamo di non avere un CRS.
        CrsDaScrivere::Nullo => {
            column["crs"] = serde_json::Value::Null;
        }
        CrsDaScrivere::Documento(testo) => {
            let documento: serde_json::Value = serde_json::from_str(testo).map_err(|_| {
                fmt_err(&PublicMessage::Curated(
                    "definizione PROJJSON non rileggibile al momento di scriverla",
                ))
            })?;
            column["crs"] = documento;
        }
    }
    let mut columns = HashMap::new();
    columns.insert(geom_name.to_owned(), column);
    Ok(serde_json::json!({
        // 1.1.0, non 1.0.0: `covering` esiste **da** 1.1, e questo writer lo
        // emette. Dichiarare 1.0.0 mentre si scrive un metadato 1.1 era una
        // contraddizione dentro lo stesso documento, e il lettore che
        // credesse alla versione avrebbe letto un file con le regole di
        // un'altra. La dichiarazione segue cio' che si scrive.
        "version": VERSIONE_SCRITTA,
        "primary_column": geom_name,
        "columns": columns,
    })
    .to_string())
}

#[cfg(test)]
mod tests;
