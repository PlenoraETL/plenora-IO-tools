//! Prevalidazione dello schema e del corpo Arrow IPC (FZ-0).
//!
//! `arrow-ipc` 59.1.0 converte lo schema `FlatBuffer` in `arrow_schema::Schema`
//! con una funzione **infallibile**: `fb_to_schema` e `get_data_type` non
//! restituiscono `Result`, e dove l'input non e' quello che si aspettano
//! chiamano `panic!`, `unimplemented!` o `unwrap()` — una ventina di siti in
//! `convert.rs`. Il decoder del corpo fa lo stesso: `read_buffer` affetta il
//! body con `Buffer::slice_with_length`, che asserisce che offset e lunghezza
//! stiano dentro (`arrow-buffer/src/buffer/immutable.rs:288`).
//!
//! I driver leggono file esterni non fidati per mestiere. Una barriera
//! `catch_unwind` converte il panico in errore tipizzato e resta come difesa
//! in profondita', ma **non chiude il difetto**: il panico e' avvenuto, e
//! sotto `libfuzzer-sys` diventa `abort()` prima che l'unwinding cominci.
//! Qui il panico non avviene, perche' l'input che lo produrrebbe viene
//! rifiutato prima.
//!
//! # Cosa verifica, e perche' non e' una copia di `arrow`
//!
//! Il criterio e' la **conformita' al formato Arrow**, non l'elenco di cio'
//! che questa versione della libreria sa digerire:
//!
//! * un valore di enum fuori dai valori definiti (`variant_name()` che
//!   restituisce `None`) non e' un tipo che non supportiamo: e' un tipo che
//!   non esiste nel formato;
//! * `Type::NONE` e' il sentinella dell'unione `FlatBuffer`, non un tipo di
//!   campo: uno schema che lo dichiara e' malformato;
//! * un tag di unione che non corrisponde alla tabella presente
//!   (`type_type()` dice `Int`, `type_as_int()` restituisce `None`) e' una
//!   incoerenza interna del `FlatBuffer`;
//! * un buffer che dichiara `offset + length` oltre il corpo del messaggio
//!   descrive byte che non ci sono.
//!
//! Nessuno di questi rifiuti riguarda un file valido, quindi la
//! prevalidazione non restringe cio' che i driver leggono oggi. Le
//! combinazioni interne — larghezza di un intero, unita' di un tempo,
//! larghezza di un decimale — sono invece prese **una per una** da
//! `convert.rs` della versione pinnata: sono le sole in cui un file conforme
//! al formato puo' comunque far panicare la libreria, e il pin esatto e'
//! sorvegliato da `scripts/check_dependency_pins.py`.
//!
//! # Bounded e fail-closed
//!
//! Ogni percorso ha un tetto — campi visitati, profondita' di annidamento,
//! byte di metadati letti, blocchi del footer — e ogni condizione che non si
//! riesce a verificare ferma la lettura con un errore tipizzato. Non c'e' un
//! ramo che, non riuscendo a controllare, prosegua lo stesso.

use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use arrow_ipc::{
    root_as_footer, root_as_message, Endianness, Field as FbField, Precision, Schema as FbSchema,
    TimeUnit as FbTimeUnit, Type, UnionMode,
};
use plenora_io_model::PublicMessage;
use plenora_io_model::{PlenoraIoError, Result};

/// Campi totali visitati in uno schema, figli annidati compresi.
const MAX_CAMPI: usize = 65_536;

/// Profondita' massima di annidamento dei tipi.
const MAX_PROFONDITA: usize = 64;

/// Tetto sui byte di metadati letti per un singolo messaggio o per il footer.
/// I metadati IPC sono descrittori, non dati: qualche decina di KiB in un file
/// reale.
const MAX_BYTE_METADATI: usize = 16 * 1024 * 1024;

/// Tetto sui blocchi dichiarati dal footer.
const MAX_BLOCCHI: usize = 1_048_576;

/// Byte iniziali e finali di un file Arrow IPC.
const MAGIC: &[u8] = b"ARROW1";

fn errore(driver: &'static str, motivo: &'static str) -> PlenoraIoError {
    PlenoraIoError::formato_redatto(driver, &PublicMessage::Curated(motivo))
}

/// Verifica uno schema Arrow IPC gia' deserializzato in `FlatBuffer`.
///
/// # Errors
///
/// Restituisce `PlenoraIoError::format` — fase `Read` — se lo schema non e'
/// conforme al formato o contiene una combinazione che la conversione di
/// `arrow-ipc` non sa gestire senza panicare.
pub fn valida_schema(driver: &'static str, schema: FbSchema<'_>) -> Result<()> {
    // `fb_to_schema` fa `fb.fields().unwrap()`: uno schema senza il vettore
    // dei campi non e' vuoto, e' malformato.
    let campi = schema
        .fields()
        .ok_or_else(|| errore(driver, "schema Arrow senza vettore dei campi"))?;

    // `fb_to_schema` rifiuta i decimali big-endian con `unimplemented!`. Il
    // formato dichiara l'endianness a livello di schema: la rifiutiamo qui,
    // per qualunque tipo, perche' il resto della pipeline assume little-endian.
    if schema.endianness() != Endianness::Little {
        return Err(errore(driver, "schema Arrow non little-endian"));
    }

    let mut visitati = 0usize;
    for indice in 0..campi.len() {
        valida_campo(driver, campi.get(indice), 0, &mut visitati)?;
    }
    Ok(())
}

/// Verifica un messaggio IPC che porta uno schema, dai byte serializzati.
///
/// E' la forma in cui lo schema arriva dai metadati `ARROW:schema` di un
/// Parquet: un messaggio IPC incorporato nel footer.
///
/// # Errors
///
/// Come [`valida_schema`], piu' il caso in cui i byte non siano un messaggio
/// `FlatBuffer` valido o non portino uno schema.
pub fn valida_messaggio_schema(driver: &'static str, byte: &[u8]) -> Result<()> {
    if byte.len() > MAX_BYTE_METADATI {
        return Err(errore(driver, "metadati dello schema Arrow oltre il tetto"));
    }
    // Forma incapsulata: `[0xFFFFFFFF][i32 lunghezza][flatbuffer]`. E' come
    // `arrow-rs` scrive `ARROW:schema` nel footer Parquet, ed e' la stessa
    // distinzione che fa il suo lettore prima di decodificare. Un messaggio
    // nudo, senza marcatore, resta ammesso: lo scrivevano le versioni
    // precedenti del formato.
    let byte = if byte.len() >= 8 && byte[0..4] == [0xFF, 0xFF, 0xFF, 0xFF] {
        byte.get(8..)
            .ok_or_else(|| errore(driver, "messaggio Arrow IPC troncato"))?
    } else {
        byte
    };
    let messaggio = root_as_message(byte)
        .map_err(|_| errore(driver, "messaggio Arrow IPC non decodificabile"))?;
    let schema = messaggio
        .header_as_schema()
        .ok_or_else(|| errore(driver, "messaggio Arrow IPC senza schema"))?;
    valida_schema(driver, schema)
}

/// Verifica un file Arrow IPC: schema del footer e coerenza di ogni messaggio.
///
/// Legge solo i metadati — footer e intestazioni dei messaggi — mai il corpo.
///
/// # Errors
///
/// Restituisce `PlenoraIoError::format` se il contenitore, il footer, lo
/// schema o i buffer dichiarati non sono conformi; propaga gli errori di I/O.
pub fn valida_file_ipc(driver: &'static str, percorso: &Path) -> Result<()> {
    let mut file = std::fs::File::open(percorso)?;
    let dimensione = file.seek(SeekFrom::End(0))?;

    // Contenitore: `ARROW1` in testa, `ARROW1` piu' la lunghezza del footer in
    // coda. Sotto questa soglia non c'e' nemmeno un file.
    let minimo = (MAGIC.len() * 2 + 2 + 4) as u64;
    if dimensione < minimo {
        return Err(errore(driver, "file Arrow IPC troppo corto per il formato"));
    }

    let mut magic = [0_u8; 6];
    file.seek(SeekFrom::Start(0))?;
    file.read_exact(&mut magic)?;
    if magic != MAGIC {
        return Err(errore(driver, "file Arrow IPC senza firma iniziale"));
    }

    // In coda: [footer][i32 lunghezza footer][ARROW1].
    let mut coda = [0_u8; 10];
    file.seek(SeekFrom::End(-10))?;
    file.read_exact(&mut coda)?;
    if &coda[4..] != MAGIC {
        return Err(errore(driver, "file Arrow IPC senza firma finale"));
    }
    let lunghezza_footer = i32::from_le_bytes([coda[0], coda[1], coda[2], coda[3]]);
    let lunghezza_footer = usize::try_from(lunghezza_footer)
        .map_err(|_| errore(driver, "lunghezza del footer Arrow negativa"))?;
    if lunghezza_footer == 0 || lunghezza_footer > MAX_BYTE_METADATI {
        return Err(errore(
            driver,
            "lunghezza del footer Arrow fuori dai limiti",
        ));
    }
    let inizio_footer = dimensione
        .checked_sub(10_u64 + lunghezza_footer as u64)
        .ok_or_else(|| errore(driver, "footer Arrow oltre la dimensione del file"))?;

    let mut byte_footer = vec![0_u8; lunghezza_footer];
    file.seek(SeekFrom::Start(inizio_footer))?;
    file.read_exact(&mut byte_footer)?;
    let footer = root_as_footer(&byte_footer)
        .map_err(|_| errore(driver, "footer Arrow non decodificabile"))?;

    let schema = footer
        .schema()
        .ok_or_else(|| errore(driver, "footer Arrow senza schema"))?;
    valida_schema(driver, schema)?;

    for blocchi in [footer.dictionaries(), footer.recordBatches()] {
        let Some(blocchi) = blocchi else { continue };
        if blocchi.len() > MAX_BLOCCHI {
            return Err(errore(driver, "troppi blocchi dichiarati nel footer Arrow"));
        }
        for indice in 0..blocchi.len() {
            let blocco = blocchi.get(indice);
            valida_blocco(
                driver,
                schema,
                &mut file,
                dimensione,
                blocco.offset(),
                blocco.metaDataLength(),
                blocco.bodyLength(),
            )?;
        }
    }
    Ok(())
}

/// Verifica un blocco: sta nel file, e i buffer che dichiara stanno nel corpo.
fn valida_blocco(
    driver: &'static str,
    schema: FbSchema<'_>,
    file: &mut std::fs::File,
    dimensione: u64,
    offset: i64,
    lunghezza_metadati: i32,
    lunghezza_corpo: i64,
) -> Result<()> {
    let offset = u64::try_from(offset).map_err(|_| errore(driver, "offset di blocco negativo"))?;
    let lunghezza_metadati = usize::try_from(lunghezza_metadati)
        .map_err(|_| errore(driver, "lunghezza dei metadati negativa"))?;
    let lunghezza_corpo = u64::try_from(lunghezza_corpo)
        .map_err(|_| errore(driver, "lunghezza del corpo negativa"))?;
    if lunghezza_metadati > MAX_BYTE_METADATI {
        return Err(errore(driver, "metadati di blocco oltre il tetto"));
    }

    let oltre_il_blocco = offset
        .checked_add(lunghezza_metadati as u64)
        .and_then(|parziale| parziale.checked_add(lunghezza_corpo))
        .ok_or_else(|| errore(driver, "blocco Arrow con estensione non rappresentabile"))?;
    if oltre_il_blocco > dimensione {
        return Err(errore(driver, "blocco Arrow oltre la fine del file"));
    }

    // I metadati cominciano dopo il marcatore di continuazione e la lunghezza.
    if lunghezza_metadati < 8 {
        return Err(errore(driver, "metadati di blocco troppo corti"));
    }
    let mut intestazione = vec![0_u8; lunghezza_metadati];
    file.seek(SeekFrom::Start(offset))?;
    file.read_exact(&mut intestazione)?;

    // [0xFFFFFFFF][i32 lunghezza][flatbuffer]: la continuazione e' opzionale
    // nei file scritti da implementazioni vecchie, quindi si accettano
    // entrambe le forme e si rifiuta tutto il resto.
    let corpo_flat = if intestazione[0..4] == [0xFF, 0xFF, 0xFF, 0xFF] {
        let dichiarata = i32::from_le_bytes([
            intestazione[4],
            intestazione[5],
            intestazione[6],
            intestazione[7],
        ]);
        let dichiarata = usize::try_from(dichiarata)
            .map_err(|_| errore(driver, "lunghezza dei metadati negativa"))?;
        intestazione
            .get(8..8 + dichiarata)
            .ok_or_else(|| errore(driver, "metadati di blocco troncati"))?
    } else {
        let dichiarata = i32::from_le_bytes([
            intestazione[0],
            intestazione[1],
            intestazione[2],
            intestazione[3],
        ]);
        let dichiarata = usize::try_from(dichiarata)
            .map_err(|_| errore(driver, "lunghezza dei metadati negativa"))?;
        intestazione
            .get(4..4 + dichiarata)
            .ok_or_else(|| errore(driver, "metadati di blocco troncati"))?
    };

    let messaggio = root_as_message(corpo_flat)
        .map_err(|_| errore(driver, "messaggio Arrow IPC non decodificabile"))?;

    if let Some(schema_del_messaggio) = messaggio.header_as_schema() {
        valida_schema(driver, schema_del_messaggio)?;
    }

    // Un corpo compresso non e' ispezionabile qui, ed e' irrilevante: la
    // versione pinnata di `arrow-ipc` e' compilata senza `lz4` e senza `zstd`
    // (`default = []`), quindi rifiuta il codec con un errore prima di
    // decodificare. Rifiutarlo anche noi non restringe niente e tiene il
    // controllo fail-closed: se un domani i codec venissero abilitati, questa
    // riga imporrebbe di decidere come validare i buffer decompressi invece di
    // lasciarli passare senza verifica.
    let versione_v4 = messaggio.version().0 < arrow_ipc::MetadataVersion::V5.0;
    if let Some(batch) = messaggio.header_as_record_batch() {
        if batch.compression().is_some() {
            return Err(errore(driver, "corpo Arrow compresso non verificabile"));
        }
        valida_batch(driver, schema, batch, versione_v4, lunghezza_corpo)?;
    }
    if let Some(dizionario) = messaggio.header_as_dictionary_batch() {
        if let Some(batch) = dizionario.data() {
            if batch.compression().is_some() {
                return Err(errore(driver, "corpo Arrow compresso non verificabile"));
            }
            valida_buffer(driver, batch, lunghezza_corpo)?;
        }
    }
    Ok(())
}

/// Ogni buffer dichiarato deve stare dentro il corpo del proprio messaggio.
///
/// E' il controllo che impedisce l'assert di `Buffer::slice_with_length`: il
/// decoder affetta il corpo agli offset che il messaggio dichiara, e un offset
/// oltre la fine non e' un buffer vuoto, e' un buffer che non c'e'.
fn valida_buffer(
    driver: &'static str,
    batch: arrow_ipc::RecordBatch<'_>,
    lunghezza_corpo: u64,
) -> Result<()> {
    let Some(buffer) = batch.buffers() else {
        return Ok(());
    };
    for indice in 0..buffer.len() {
        let descrittore = buffer.get(indice);
        let offset = u64::try_from(descrittore.offset())
            .map_err(|_| errore(driver, "offset di buffer Arrow negativo"))?;
        let lunghezza = u64::try_from(descrittore.length())
            .map_err(|_| errore(driver, "lunghezza di buffer Arrow negativa"))?;
        let fine = offset
            .checked_add(lunghezza)
            .ok_or_else(|| errore(driver, "buffer Arrow con estensione non rappresentabile"))?;
        if fine > lunghezza_corpo {
            return Err(errore(
                driver,
                "buffer Arrow oltre la fine del corpo del messaggio",
            ));
        }
    }
    Ok(())
}

/// Classe di layout di un campo: quanti nodi e buffer il decoder consuma, in
/// quale ordine, e quali figli attraversa.
///
/// E' la parte **dichiarativa** della validazione: invece di una raccolta di
/// controlli legati ai singoli crash, il tipo determina il layout e il layout
/// determina i controlli. Un tipo Arrow nuovo si aggiunge qui, in un posto
/// solo, e ne eredita le verifiche.
///
/// L'ordine rispecchia `arrow_ipc::reader::ArrayReader::create_array` della
/// versione pinnata: se non lo rispecchiasse, i controlli finirebbero sul
/// buffer sbagliato — che e' peggio di non farli, perche' sembrerebbero fatti.
#[derive(Clone, Copy)]
enum Layout {
    /// Nessun buffer, nessun figlio.
    Nulla,
    /// Validita' + dati.
    Primitiva,
    /// Validita' + offset + dati. `larghezza` e' la dimensione di un offset.
    Binaria { larghezza: u64 },
    /// Conteggio variadico, poi `conteggio + 2` buffer, poi il nodo.
    Vista,
    /// Validita' + offset, un figlio.
    ListaOffset { larghezza: u64 },
    /// Validita' + offset + dimensioni, un figlio.
    ListaVista { larghezza: u64 },
    /// Sola validita', un figlio.
    ListaFissa,
    /// Sola validita', N figli.
    Struttura,
    /// Nessun buffer, due figli.
    SequenzeRipetute,
    /// Identificatori di tipo (+ offset se densa), N figli.
    Unione { densa: bool },
    /// Indici: validita' + dati. I valori arrivano dal messaggio dizionario,
    /// quindi i figli non vengono attraversati qui.
    Dizionario,
}

impl Layout {
    /// Deriva il layout dal tipo dichiarato nel `FlatBuffer`.
    fn per_campo(campo: &FbField<'_>) -> Option<Self> {
        if campo.dictionary().is_some() {
            return Some(Self::Dizionario);
        }
        Some(match campo.type_type() {
            Type::Null => Self::Nulla,
            Type::Utf8 | Type::Binary => Self::Binaria { larghezza: 4 },
            Type::LargeBinary | Type::LargeUtf8 => Self::Binaria { larghezza: 8 },
            Type::BinaryView | Type::Utf8View => Self::Vista,
            Type::List | Type::Map => Self::ListaOffset { larghezza: 4 },
            Type::LargeList => Self::ListaOffset { larghezza: 8 },
            Type::ListView => Self::ListaVista { larghezza: 4 },
            Type::LargeListView => Self::ListaVista { larghezza: 8 },
            Type::FixedSizeList => Self::ListaFissa,
            Type::Struct_ => Self::Struttura,
            Type::RunEndEncoded => Self::SequenzeRipetute,
            Type::Union => Self::Unione {
                densa: campo
                    .type_as_union()
                    .is_some_and(|unione| unione.mode() == UnionMode::Dense),
            },
            Type::Bool
            | Type::Int
            | Type::FloatingPoint
            | Type::Decimal
            | Type::Date
            | Type::Time
            | Type::Timestamp
            | Type::Interval
            | Type::Duration
            | Type::FixedSizeBinary => Self::Primitiva,
            _ => return None,
        })
    }

    /// Quanti figli il decoder attraversa per questo layout.
    const fn figli_attraversati(self, dichiarati: usize) -> usize {
        match self {
            Self::Nulla
            | Self::Primitiva
            | Self::Binaria { .. }
            | Self::Vista
            | Self::Dizionario => 0,
            Self::ListaOffset { .. } | Self::ListaVista { .. } | Self::ListaFissa => 1,
            Self::SequenzeRipetute => 2,
            Self::Struttura | Self::Unione { .. } => dichiarati,
        }
    }
}

/// Stato della passeggiata: dove siamo nei vettori di nodi, buffer e
/// conteggi variadici che il messaggio dichiara.
struct Cursori {
    nodo: usize,
    buffer: usize,
    variadico: usize,
    profondita: usize,
}

/// Verifica un batch: la passeggiata sullo schema consuma nodi e buffer nello
/// stesso ordine del decoder, e applica a ognuno le regole del proprio tipo.
///
/// I controlli sono quelli che impediscono un **panico**, e nessuno e' piu'
/// severo di cio' che `arrow` stesso pretende:
///
/// * la bitmap di validita' deve bastare alla lunghezza dichiarata quando ci
///   sono null. `ArrayData::try_new` lo verifica e restituisce `Err`, ma
///   `create_struct_array` costruisce `BooleanBuffer::new` **prima** di
///   passare dal costruttore fallibile, e li' l'assert diventa un panico;
/// * gli identificatori di tipo di un'unione vengono affettati a `len` byte e
///   gli offset densi a `len * 4` senza controlli, con una moltiplicazione che
///   trabocca prima ancora di affettare;
/// * il conteggio variadico di una vista viene sommato a 2 senza controlli;
/// * ogni buffer usato deve stare dentro il corpo del messaggio.
///
/// Cio' che `arrow` valida gia' in modo fallibile — contenuto degli offset,
/// monotonia, ultimo offset, UTF-8, dimensioni dei buffer di dati — non viene
/// duplicato qui: `try_new` chiama `validate_data`, quindi quei casi
/// producono un errore, non un panico, e riscriverli fuori significherebbe
/// solo poterli sbagliare in modo diverso.
fn valida_batch(
    driver: &'static str,
    schema: FbSchema<'_>,
    batch: arrow_ipc::RecordBatch<'_>,
    versione_v4: bool,
    lunghezza_corpo: u64,
) -> Result<()> {
    let campi = schema
        .fields()
        .ok_or_else(|| errore(driver, "schema Arrow senza vettore dei campi"))?;
    let mut cursori = Cursori {
        nodo: 0,
        buffer: 0,
        variadico: 0,
        profondita: 0,
    };
    for indice in 0..campi.len() {
        valida_campo_del_batch(
            driver,
            campi.get(indice),
            batch,
            versione_v4,
            lunghezza_corpo,
            &mut cursori,
        )?;
    }
    Ok(())
}

fn valida_campo_del_batch(
    driver: &'static str,
    campo: FbField<'_>,
    batch: arrow_ipc::RecordBatch<'_>,
    versione_v4: bool,
    lunghezza_corpo: u64,
    cursori: &mut Cursori,
) -> Result<()> {
    if cursori.profondita > MAX_PROFONDITA {
        return Err(errore(driver, "schema Arrow annidato oltre il tetto"));
    }
    let layout = Layout::per_campo(&campo)
        .ok_or_else(|| errore(driver, "campo Arrow con tipo non decodificabile"))?;

    // Le viste consumano i buffer **prima** del nodo: e' l'ordine del decoder,
    // e invertirlo sposterebbe ogni controllo successivo di una posizione.
    if matches!(layout, Layout::Vista) {
        let conteggio = conteggio_variadico(driver, batch, cursori)?;
        let quanti = conteggio
            .checked_add(2)
            .ok_or_else(|| errore(driver, "conteggio variadico Arrow non rappresentabile"))?;
        for _ in 0..quanti {
            let _ = preleva_buffer(driver, batch, lunghezza_corpo, cursori)?;
        }
        let _ = preleva_nodo(driver, batch, cursori)?;
        return Ok(());
    }

    let (lunghezza, conteggio_null) = preleva_nodo(driver, batch, cursori)?;

    consuma_buffer(
        driver,
        layout,
        batch,
        versione_v4,
        lunghezza_corpo,
        lunghezza,
        conteggio_null,
        cursori,
    )?;

    let dichiarati = campo.children().map_or(0, |figli| figli.len());
    let da_attraversare = layout.figli_attraversati(dichiarati);
    if da_attraversare > dichiarati {
        return Err(errore(
            driver,
            "contenitore Arrow con meno figli di quanti il tipo ne richieda",
        ));
    }
    if let Some(figli) = campo.children() {
        cursori.profondita += 1;
        for indice in 0..da_attraversare {
            valida_campo_del_batch(
                driver,
                figli.get(indice),
                batch,
                versione_v4,
                lunghezza_corpo,
                cursori,
            )?;
        }
        cursori.profondita -= 1;
    }
    Ok(())
}

/// Consuma i buffer che la classe di layout prevede, applicando a ognuno la
/// propria regola.
///
/// Sta a parte dalla passeggiata sui campi per una ragione di leggibilita':
/// qui c'e' la tabella dei layout, li' c'e' la ricorsione sull'albero.
#[allow(clippy::too_many_arguments)]
fn consuma_buffer(
    driver: &'static str,
    layout: Layout,
    batch: arrow_ipc::RecordBatch<'_>,
    versione_v4: bool,
    lunghezza_corpo: u64,
    lunghezza: u64,
    conteggio_null: u64,
    cursori: &mut Cursori,
) -> Result<()> {
    match layout {
        Layout::Nulla | Layout::SequenzeRipetute => {}
        Layout::Primitiva | Layout::Dizionario => {
            let (_, validita) = preleva_buffer(driver, batch, lunghezza_corpo, cursori)?;
            verifica_validita(driver, validita, lunghezza, conteggio_null)?;
            let _ = preleva_buffer(driver, batch, lunghezza_corpo, cursori)?;
        }
        Layout::Binaria { larghezza } => {
            let (_, validita) = preleva_buffer(driver, batch, lunghezza_corpo, cursori)?;
            verifica_validita(driver, validita, lunghezza, conteggio_null)?;
            let (_, offset) = preleva_buffer(driver, batch, lunghezza_corpo, cursori)?;
            verifica_buffer_tipizzato(driver, offset, larghezza)?;
            let _ = preleva_buffer(driver, batch, lunghezza_corpo, cursori)?;
        }
        Layout::ListaOffset { larghezza } => {
            let (_, validita) = preleva_buffer(driver, batch, lunghezza_corpo, cursori)?;
            verifica_validita(driver, validita, lunghezza, conteggio_null)?;
            let (_, offset) = preleva_buffer(driver, batch, lunghezza_corpo, cursori)?;
            verifica_buffer_tipizzato(driver, offset, larghezza)?;
        }
        Layout::ListaVista { larghezza } => {
            let (_, validita) = preleva_buffer(driver, batch, lunghezza_corpo, cursori)?;
            verifica_validita(driver, validita, lunghezza, conteggio_null)?;
            for _ in 0..2 {
                let (_, tipizzato) = preleva_buffer(driver, batch, lunghezza_corpo, cursori)?;
                verifica_buffer_tipizzato(driver, tipizzato, larghezza)?;
            }
        }
        Layout::ListaFissa | Layout::Struttura => {
            let (_, validita) = preleva_buffer(driver, batch, lunghezza_corpo, cursori)?;
            verifica_validita(driver, validita, lunghezza, conteggio_null)?;
        }
        Layout::Unione { densa } => {
            // In V4 l'unione porta ancora una bitmap di validita', che il
            // decoder preleva e scarta.
            if versione_v4 {
                let _ = preleva_buffer(driver, batch, lunghezza_corpo, cursori)?;
            }
            let (_, identificatori) = preleva_buffer(driver, batch, lunghezza_corpo, cursori)?;
            if identificatori < lunghezza {
                return Err(errore(
                    driver,
                    "buffer degli identificatori di unione Arrow piu' corto della lunghezza dichiarata",
                ));
            }
            if densa {
                let necessari = lunghezza.checked_mul(4).ok_or_else(|| {
                    errore(
                        driver,
                        "lunghezza di unione densa Arrow non rappresentabile",
                    )
                })?;
                let (_, offset) = preleva_buffer(driver, batch, lunghezza_corpo, cursori)?;
                if offset < necessari {
                    return Err(errore(
                        driver,
                        "buffer degli offset di unione densa Arrow piu' corto del necessario",
                    ));
                }
            }
        }
        // La vista consuma i propri buffer prima del nodo, quindi non arriva
        // qui. Se ci arrivasse sarebbe un difetto nostro, e un difetto nostro
        // ferma la lettura invece di abbattere il processo: il gate anti-panic
        // vieta `unreachable!` proprio perche' un ramo "impossibile" non lo e'
        // per sempre.
        Layout::Vista => {
            return Err(errore(driver, "layout Arrow attraversato fuori ordine"));
        }
    }

    Ok(())
}

/// La bitmap deve bastare alla lunghezza dichiarata quando ci sono null.
///
/// E' la stessa condizione di `ArrayData::try_new` — `ceil(len / 8)` byte — e
/// vale solo con `null_count > 0`, perche' senza null il decoder scarta il
/// buffer senza guardarlo. Piu' severa rifiuterebbe file che oggi si leggono.
fn verifica_validita(
    driver: &'static str,
    disponibili: u64,
    lunghezza: u64,
    conteggio_null: u64,
) -> Result<()> {
    if conteggio_null == 0 {
        return Ok(());
    }
    let necessari = lunghezza
        .checked_add(7)
        .ok_or_else(|| errore(driver, "lunghezza del nodo Arrow non rappresentabile"))?
        / 8;
    if disponibili < necessari {
        return Err(errore(
            driver,
            "bitmap di validita' Arrow piu' corta della lunghezza dichiarata",
        ));
    }
    Ok(())
}

/// Un buffer che `arrow` legge come sequenza tipizzata deve avere una
/// lunghezza multipla dell'elemento.
///
/// `ArrayData::validate_offsets` legge gli offset con `Buffer::typed_data`,
/// che fa `align_to::<T>()` e asserisce che **non restino code**: una coda
/// c'e' ogni volta che la lunghezza non e' un multiplo dell'elemento, e
/// l'assert e' un panico dentro la validazione stessa della libreria. E' il
/// caso in cui validare non protegge, e serve arrivare prima.
///
/// Non e' piu' severo del formato: un buffer di offset contiene offset interi,
/// e uno spezzato a meta' non descrive niente. Il padding IPC e' a multipli di
/// 8, quindi ogni scrittore conforme lo rispetta gia'.
fn verifica_buffer_tipizzato(driver: &'static str, lunghezza: u64, larghezza: u64) -> Result<()> {
    if !lunghezza.is_multiple_of(larghezza) {
        return Err(errore(
            driver,
            "buffer di offset Arrow con lunghezza non multipla dell'elemento",
        ));
    }
    Ok(())
}

/// Preleva il nodo successivo e ne verifica la rappresentabilita'.
fn preleva_nodo(
    driver: &'static str,
    batch: arrow_ipc::RecordBatch<'_>,
    cursori: &mut Cursori,
) -> Result<(u64, u64)> {
    let nodi = batch
        .nodes()
        .ok_or_else(|| errore(driver, "batch Arrow senza vettore dei nodi"))?;
    if cursori.nodo >= nodi.len() {
        return Err(errore(driver, "batch Arrow con meno nodi dello schema"));
    }
    let descrittore = nodi.get(cursori.nodo);
    cursori.nodo += 1;
    let lunghezza = u64::try_from(descrittore.length())
        .map_err(|_| errore(driver, "lunghezza di nodo Arrow negativa"))?;
    let conteggio_null = u64::try_from(descrittore.null_count())
        .map_err(|_| errore(driver, "conteggio dei null Arrow negativo"))?;
    if conteggio_null > lunghezza {
        return Err(errore(
            driver,
            "nodo Arrow con piu' null della propria lunghezza",
        ));
    }
    Ok((lunghezza, conteggio_null))
}

/// Preleva il buffer successivo, ne verifica i limiti nel corpo e ne
/// restituisce la lunghezza.
fn preleva_buffer(
    driver: &'static str,
    batch: arrow_ipc::RecordBatch<'_>,
    lunghezza_corpo: u64,
    cursori: &mut Cursori,
) -> Result<(u64, u64)> {
    let buffer = batch
        .buffers()
        .ok_or_else(|| errore(driver, "batch Arrow senza vettore dei buffer"))?;
    if cursori.buffer >= buffer.len() {
        return Err(errore(driver, "batch Arrow con meno buffer dello schema"));
    }
    let descrittore = buffer.get(cursori.buffer);
    cursori.buffer += 1;
    let offset = u64::try_from(descrittore.offset())
        .map_err(|_| errore(driver, "offset di buffer Arrow negativo"))?;
    let lunghezza = u64::try_from(descrittore.length())
        .map_err(|_| errore(driver, "lunghezza di buffer Arrow negativa"))?;
    let fine = offset
        .checked_add(lunghezza)
        .ok_or_else(|| errore(driver, "buffer Arrow con estensione non rappresentabile"))?;
    if fine > lunghezza_corpo {
        return Err(errore(
            driver,
            "buffer Arrow oltre la fine del corpo del messaggio",
        ));
    }
    // Allineamento a 8 byte. Non e' una precauzione nostra: il formato IPC lo
    // impone — i buffer nel corpo sono paddati a multipli di 8 — e ogni
    // scrittore conforme, `arrow` compreso, lo rispetta.
    //
    // Senza questo controllo un buffer disallineato fa panicare `arrow`
    // **mentre lo valida**: `validate_offsets` legge gli offset con
    // `typed_data`, che asserisce l'allineamento invece di restituire un
    // errore. E' il caso in cui la validazione della libreria non protegge da
    // se' stessa, e serve che il controllo venga prima.
    if !offset.is_multiple_of(8) {
        return Err(errore(
            driver,
            "buffer Arrow non allineato agli 8 byte richiesti dal formato",
        ));
    }
    Ok((offset, lunghezza))
}

/// Preleva il conteggio variadico successivo.
fn conteggio_variadico(
    driver: &'static str,
    batch: arrow_ipc::RecordBatch<'_>,
    cursori: &mut Cursori,
) -> Result<u64> {
    let conteggi = batch
        .variadicBufferCounts()
        .ok_or_else(|| errore(driver, "batch Arrow senza conteggi variadici"))?;
    if cursori.variadico >= conteggi.len() {
        return Err(errore(driver, "conteggi variadici Arrow insufficienti"));
    }
    let conteggio = conteggi.get(cursori.variadico);
    cursori.variadico += 1;
    u64::try_from(conteggio).map_err(|_| errore(driver, "conteggio variadico Arrow negativo"))
}

/// Verifica un campo e, ricorsivamente, i suoi figli.
#[allow(clippy::too_many_lines)]
fn valida_campo(
    driver: &'static str,
    campo: FbField<'_>,
    profondita: usize,
    visitati: &mut usize,
) -> Result<()> {
    if profondita > MAX_PROFONDITA {
        return Err(errore(driver, "schema Arrow annidato oltre il tetto"));
    }
    *visitati = visitati.saturating_add(1);
    if *visitati > MAX_CAMPI {
        return Err(errore(driver, "schema Arrow con troppi campi"));
    }

    let tipo = campo.type_type();
    if tipo.variant_name().is_none() {
        return Err(errore(driver, "campo Arrow con tipo sconosciuto"));
    }
    if tipo == Type::NONE {
        return Err(errore(driver, "campo Arrow senza tipo dichiarato"));
    }

    // Un campo dizionario porta il tipo dell'indice: `get_data_type` lo estrae
    // con `unwrap()` e ne accetta solo otto combinazioni.
    if let Some(dizionario) = campo.dictionary() {
        let indice = dizionario
            .indexType()
            .ok_or_else(|| errore(driver, "dizionario Arrow senza tipo dell'indice"))?;
        if !matches!(indice.bitWidth(), 8 | 16 | 32 | 64) {
            return Err(errore(
                driver,
                "indice di dizionario Arrow non rappresentabile",
            ));
        }
    }

    // Il tag dell'unione deve corrispondere alla tabella presente: `arrow`
    // fa `type_as_*().unwrap()` fidandosi del tag.
    let coerente = match tipo {
        Type::Null => campo.type_as_null().is_some(),
        Type::Bool => campo.type_as_bool().is_some(),
        Type::Binary => campo.type_as_binary().is_some(),
        Type::LargeBinary => campo.type_as_large_binary().is_some(),
        Type::BinaryView => campo.type_as_binary_view().is_some(),
        Type::Utf8 => campo.type_as_utf_8().is_some(),
        Type::LargeUtf8 => campo.type_as_large_utf_8().is_some(),
        Type::Utf8View => campo.type_as_utf_8_view().is_some(),
        Type::Struct_ => campo.type_as_struct_().is_some(),
        Type::Int => valida_intero(driver, &campo)?,
        Type::FloatingPoint => valida_virgola_mobile(driver, &campo)?,
        Type::FixedSizeBinary => campo.type_as_fixed_size_binary().is_some(),
        Type::Date => valida_data(driver, &campo)?,
        Type::Time => valida_ora(driver, &campo)?,
        Type::Timestamp => valida_marca_temporale(driver, &campo)?,
        Type::Interval => valida_intervallo(driver, &campo)?,
        Type::Duration => valida_durata(driver, &campo)?,
        Type::Decimal => valida_decimale(driver, &campo)?,
        Type::Union => valida_unione(driver, &campo)?,
        Type::List => campo.type_as_list().is_some(),
        Type::LargeList => campo.type_as_large_list().is_some(),
        Type::ListView => campo.type_as_list_view().is_some(),
        Type::LargeListView => campo.type_as_large_list_view().is_some(),
        Type::FixedSizeList => campo.type_as_fixed_size_list().is_some(),
        Type::Map => campo.type_as_map().is_some(),
        Type::RunEndEncoded => campo.type_as_run_end_encoded().is_some(),
        _ => false,
    };
    if !coerente {
        return Err(errore(
            driver,
            "campo Arrow con tipo dichiarato e contenuto incoerenti",
        ));
    }

    // Conteggio dei figli: `get_data_type` panica sui contenitori che non ne
    // hanno esattamente il numero atteso.
    let figli = campo.children().map_or(0, |figli| figli.len());
    let attesi: Option<usize> = match tipo {
        Type::List
        | Type::LargeList
        | Type::ListView
        | Type::LargeListView
        | Type::FixedSizeList
        | Type::Map => Some(1),
        Type::RunEndEncoded => Some(2),
        _ => None,
    };
    if let Some(attesi) = attesi {
        if figli != attesi {
            return Err(errore(
                driver,
                "contenitore Arrow con un numero di figli non conforme",
            ));
        }
    }

    if let Some(figli) = campo.children() {
        for indice in 0..figli.len() {
            valida_campo(driver, figli.get(indice), profondita + 1, visitati)?;
        }
    }
    Ok(())
}

fn valida_intero(driver: &'static str, campo: &FbField<'_>) -> Result<bool> {
    let Some(intero) = campo.type_as_int() else {
        return Ok(false);
    };
    if !matches!(intero.bitWidth(), 8 | 16 | 32 | 64) {
        return Err(errore(
            driver,
            "intero Arrow con larghezza non rappresentabile",
        ));
    }
    Ok(true)
}

fn valida_virgola_mobile(driver: &'static str, campo: &FbField<'_>) -> Result<bool> {
    let Some(reale) = campo.type_as_floating_point() else {
        return Ok(false);
    };
    if !matches!(
        reale.precision(),
        Precision::HALF | Precision::SINGLE | Precision::DOUBLE
    ) {
        return Err(errore(
            driver,
            "precisione Arrow in virgola mobile non definita",
        ));
    }
    Ok(true)
}

fn valida_data(driver: &'static str, campo: &FbField<'_>) -> Result<bool> {
    let Some(data) = campo.type_as_date() else {
        return Ok(false);
    };
    if data.unit().variant_name().is_none() {
        return Err(errore(driver, "unita' di data Arrow non definita"));
    }
    Ok(true)
}

fn valida_ora(driver: &'static str, campo: &FbField<'_>) -> Result<bool> {
    let Some(ora) = campo.type_as_time() else {
        return Ok(false);
    };
    let ammessa = matches!(
        (ora.bitWidth(), ora.unit()),
        (32, FbTimeUnit::SECOND | FbTimeUnit::MILLISECOND)
            | (64, FbTimeUnit::MICROSECOND | FbTimeUnit::NANOSECOND)
    );
    if !ammessa {
        return Err(errore(
            driver,
            "combinazione di larghezza e unita' dell'ora Arrow non conforme",
        ));
    }
    Ok(true)
}

fn valida_marca_temporale(driver: &'static str, campo: &FbField<'_>) -> Result<bool> {
    let Some(marca) = campo.type_as_timestamp() else {
        return Ok(false);
    };
    if marca.unit().variant_name().is_none() {
        return Err(errore(
            driver,
            "unita' di marca temporale Arrow non definita",
        ));
    }
    Ok(true)
}

fn valida_intervallo(driver: &'static str, campo: &FbField<'_>) -> Result<bool> {
    let Some(intervallo) = campo.type_as_interval() else {
        return Ok(false);
    };
    if intervallo.unit().variant_name().is_none() {
        return Err(errore(driver, "unita' di intervallo Arrow non definita"));
    }
    Ok(true)
}

fn valida_durata(driver: &'static str, campo: &FbField<'_>) -> Result<bool> {
    let Some(durata) = campo.type_as_duration() else {
        return Ok(false);
    };
    if durata.unit().variant_name().is_none() {
        return Err(errore(driver, "unita' di durata Arrow non definita"));
    }
    Ok(true)
}

fn valida_decimale(driver: &'static str, campo: &FbField<'_>) -> Result<bool> {
    let Some(decimale) = campo.type_as_decimal() else {
        return Ok(false);
    };
    if !matches!(decimale.bitWidth(), 32 | 64 | 128 | 256) {
        return Err(errore(driver, "larghezza del decimale Arrow non conforme"));
    }
    // `get_data_type` converte con `try_into().unwrap()`: precisione in u8 e
    // scala in i8.
    if u8::try_from(decimale.precision()).is_err() || i8::try_from(decimale.scale()).is_err() {
        return Err(errore(
            driver,
            "precisione o scala del decimale Arrow fuori intervallo",
        ));
    }
    Ok(true)
}

fn valida_unione(driver: &'static str, campo: &FbField<'_>) -> Result<bool> {
    let Some(unione) = campo.type_as_union() else {
        return Ok(false);
    };
    if !matches!(unione.mode(), UnionMode::Dense | UnionMode::Sparse) {
        return Err(errore(driver, "modo di unione Arrow non definito"));
    }
    // `UnionFields::try_new` viene chiamato con `.expect`: gli id devono
    // stare in `i8` ed essere tanti quanti i figli.
    if let Some(identificatori) = unione.typeIds() {
        let figli = campo.children().map_or(0, |figli| figli.len());
        if identificatori.len() != figli {
            return Err(errore(
                driver,
                "unione Arrow con identificatori e figli in numero diverso",
            ));
        }
        for indice in 0..identificatori.len() {
            if i8::try_from(identificatori.get(indice)).is_err() {
                return Err(errore(driver, "identificatore di unione Arrow fuori da i8"));
            }
        }
    }
    Ok(true)
}
