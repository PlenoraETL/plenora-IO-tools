//! Lettura bounded degli header di pagina Parquet, prima di ogni allocazione
//! (FZ-0.2).
//!
//! ## Perche' esiste
//!
//! `SerializedPageReader` decomprime una pagina allocando in un colpo solo la
//! dimensione che l'**header di pagina** dichiara (parquet 59.1.0,
//! `file/serialized_reader.rs:447`):
//!
//! ```text
//! let mut decompressed = Vec::with_capacity(uncompressed_page_size);
//! ```
//!
//! `verify_page_size` (stesso file, riga 900) controlla che
//! `compressed_page_size` stia nel residuo del chunk e che
//! `uncompressed_page_size` non sia negativo. Non controlla che sia **piccolo**:
//! un header che dichiara 2 GiB dentro un chunk che ne dichiara trenta passa,
//! e l'allocazione parte.
//!
//! Misurato con un seme costruito apposta, sotto `RLIMIT_AS` di 512 MiB:
//!
//! ```text
//! memory allocation of 2000000000 bytes failed
//! exit 134 (SIGABRT)
//! ```
//!
//! Non e' un panico: e' l'alloc error handler, che **aborta**. Nessun
//! `catch_unwind` lo vede, quindi la barriera arrow non lo trasforma in errore
//! tipizzato. L'unico modo di non subirlo e' non arrivarci.
//!
//! ## Perche' con l'API pubblica di `parquet` non si puo'
//!
//! * `PageMetadata` — l'unica cosa che `PageReader::peek_next_page`
//!   restituisce — porta `num_rows`, `num_levels` e `is_dict`, e **non** le
//!   dimensioni;
//! * `parquet::file::metadata::thrift`, dove vive `PageHeader`, e'
//!   `pub(crate)` in 59.1.0: il tipo non e' raggiungibile da fuori.
//!
//! Non resta che leggere l'header noi. Non e' replicare il decoder: sono due
//! campi `i32` di una struct Thrift compatta, senza allocazioni proporzionali
//! ai valori letti. Il decoder — dizionari, livelli, encoding — resta di
//! `parquet`.
//!
//! ## Cosa vuol dire «bounded», qui
//!
//! Una difesa contro l'esaurimento di risorse che si lasci esaurire e' un
//! secondo difetto, non un rimedio. Quindi, in ordine:
//!
//! * la finestra di lettura e' **fissa** e non dipende da niente che il file
//!   dichiari;
//! * ogni salto verifica di restare dentro la finestra, quindi il lavoro totale
//!   e' limitato dalla finestra e non dai numeri letti;
//! * gli elenchi (`list`, `set`, `map`) dichiarano quanti elementi hanno, e
//!   quel numero e' confrontato con i **byte residui** prima di entrare nel
//!   ciclo: ogni elemento ne consuma almeno uno, quindi un elenco piu' lungo
//!   dei byte disponibili non e' lungo, e' falso;
//! * i varint hanno il byte terminale verificato: un numero che non entra in
//!   `u64` non viene troncato in silenzio;
//! * la ricorsione ha una profondita' massima;
//! * la catena delle pagine deve finire **esattamente** sulla fine del chunk.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};

use plenora_io_model::{PlenoraIoError, Result};

/// Finestra massima per un singolo header di pagina.
///
/// Un `PageHeader` e' fatto di pochi interi piu' le statistiche di pagina, che
/// `parquet` tronca a 4096 byte per valore. 64 KiB e' largo abbondantemente,
/// ed e' **fisso**: la finestra non dipende da niente che il file dichiari,
/// quindi non e' un'altra allocazione guidata dall'input.
const FINESTRA_HEADER: usize = 64 * 1024;

/// Profondita' massima di annidamento delle struct nell'header.
///
/// Un `PageHeader` reale arriva a tre livelli. Sedici e' largo, e soprattutto
/// e' **finito**: una struct che si annida all'infinito e' un file ostile, non
/// un file profondo.
const PROFONDITA_MASSIMA: u32 = 16;

pub const MSG_HEADER_PAGINA_ILLEGGIBILE: &str = "header di pagina Parquet non leggibile";
pub const MSG_PAGINA_OLTRE_LA_MEMORIA: &str =
    "pagina Parquet che dichiara piu' byte non compressi della memoria disponibile";
pub const MSG_PAGINA_OLTRE_IL_CHUNK: &str =
    "pagina Parquet che dichiara piu' byte non compressi del proprio chunk";
pub const MSG_CATENA_PAGINE_NON_PROGREDISCE: &str = "catena di pagine Parquet che non avanza";
pub const MSG_CATENA_PAGINE_NON_CHIUDE: &str =
    "catena di pagine Parquet che non finisce sulla fine del chunk";

fn errore(messaggio: &'static str) -> PlenoraIoError {
    PlenoraIoError::redatto(
        plenora_io_model::IoErrorCode::Generic,
        plenora_io_model::ErrorCategory::DataMapping,
        plenora_io_model::ErrorPhase::Read,
        plenora_io_model::RemoteEffect::None,
        plenora_io_model::RetryDisposition::Never,
        &plenora_io_model::PublicMessage::Curated(messaggio),
    )
}

/// Dove sta un booleano Thrift compatto.
///
/// La differenza non e' teorica: dentro una struct il valore **e'** il tipo del
/// field header (`1` vero, `2` falso) e non c'e' niente da consumare; dentro un
/// elenco o una mappa ogni elemento e' un byte a se'. Trattarli allo stesso
/// modo — come faceva la prima stesura — legge un elenco di booleani senza
/// avanzare, e da li' in poi ogni offset e' sbagliato.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Posizione {
    Campo,
    Elemento,
}

/// Il minimo di Thrift compatto che serve a leggere un `PageHeader`.
///
/// Legge da una fetta gia' in memoria e non alloca: ogni salto avanza un
/// indice, e l'unico limite superiore e' la fine della fetta.
struct Compatto<'a> {
    dati: &'a [u8],
    i: usize,
}

impl<'a> Compatto<'a> {
    const fn nuovo(dati: &'a [u8]) -> Self {
        Self { dati, i: 0 }
    }

    const fn residui(&self) -> usize {
        self.dati.len() - self.i
    }

    fn byte(&mut self) -> Result<u8> {
        let b = *self
            .dati
            .get(self.i)
            .ok_or_else(|| errore(MSG_HEADER_PAGINA_ILLEGGIBILE))?;
        self.i += 1;
        Ok(b)
    }

    /// Un varint, con il byte terminale verificato.
    ///
    /// Dieci gruppi da sette bit coprono un `u64`, e il decimo ne puo' portare
    /// **uno solo**: un varint che continua oltre, o il cui ultimo byte porta
    /// bit che non entrano, non e' un numero grande — e' un numero che il file
    /// non poteva scrivere. Accettarlo con un troncamento silenzioso darebbe un
    /// valore diverso da quello che il decoder leggera'.
    fn varint(&mut self) -> Result<u64> {
        let mut valore: u64 = 0;
        for gruppo in 0..10_u32 {
            let b = self.byte()?;
            let carico = u64::from(b & 0x7F);
            if gruppo == 9 && carico > 1 {
                return Err(errore(MSG_HEADER_PAGINA_ILLEGGIBILE));
            }
            valore |= carico << (gruppo * 7);
            if b & 0x80 == 0 {
                return Ok(valore);
            }
        }
        Err(errore(MSG_HEADER_PAGINA_ILLEGGIBILE))
    }

    fn zigzag(&mut self) -> Result<i64> {
        let grezzo = self.varint()?;
        // Il cast e' voluto: zigzag mappa i negativi sui dispari.
        #[allow(clippy::cast_possible_wrap)]
        Ok(((grezzo >> 1) as i64) ^ -((grezzo & 1) as i64))
    }

    fn avanza(&mut self, quanti: u64) -> Result<()> {
        let quanti = usize::try_from(quanti).map_err(|_| errore(MSG_HEADER_PAGINA_ILLEGGIBILE))?;
        self.i = self
            .i
            .checked_add(quanti)
            .filter(|&fine| fine <= self.dati.len())
            .ok_or_else(|| errore(MSG_HEADER_PAGINA_ILLEGGIBILE))?;
        Ok(())
    }

    /// Quanti elementi dichiara l'intestazione di un elenco, verificati contro
    /// i byte residui.
    ///
    /// Ogni elemento consuma almeno un byte — anche un booleano, che dentro un
    /// elenco e' un byte a se' — quindi un elenco piu' lungo dei byte
    /// disponibili non puo' esistere. Senza questo controllo un elenco che
    /// dichiara `u64::MAX` elementi manda il ciclo a girare finche' il primo
    /// salto non fallisce: corretto nell'esito, arbitrario nel tempo.
    fn quanti_elementi(&self, dichiarati: u64) -> Result<usize> {
        let dichiarati =
            usize::try_from(dichiarati).map_err(|_| errore(MSG_HEADER_PAGINA_ILLEGGIBILE))?;
        if dichiarati > self.residui() {
            return Err(errore(MSG_HEADER_PAGINA_ILLEGGIBILE));
        }
        Ok(dichiarati)
    }

    /// Salta il valore di un campo o di un elemento del tipo indicato.
    fn salta(&mut self, tipo: u8, dove: Posizione, profondita: u32) -> Result<()> {
        if profondita == 0 {
            return Err(errore(MSG_HEADER_PAGINA_ILLEGGIBILE));
        }
        match tipo {
            1 | 2 => match dove {
                // Dentro una struct il valore sta nel field header.
                Posizione::Campo => Ok(()),
                // Dentro un elenco ogni booleano e' un byte.
                Posizione::Elemento => self.avanza(1),
            },
            3 => self.avanza(1),
            4..=6 => self.zigzag().map(|_| ()),
            7 => self.avanza(8),
            8 => {
                let quanti = self.varint()?;
                self.avanza(quanti)
            }
            9 | 10 => {
                let intestazione = self.byte()?;
                let elemento = intestazione & 0x0F;
                let dichiarati = u64::from(intestazione >> 4);
                let dichiarati = if dichiarati == 15 {
                    self.varint()?
                } else {
                    dichiarati
                };
                let quanti = self.quanti_elementi(dichiarati)?;
                for _ in 0..quanti {
                    self.salta(elemento, Posizione::Elemento, profondita - 1)?;
                }
                Ok(())
            }
            11 => {
                let dichiarati = self.varint()?;
                let quanti = self.quanti_elementi(dichiarati)?;
                if quanti > 0 {
                    let coppia = self.byte()?;
                    for _ in 0..quanti {
                        self.salta(coppia >> 4, Posizione::Elemento, profondita - 1)?;
                        self.salta(coppia & 0x0F, Posizione::Elemento, profondita - 1)?;
                    }
                }
                Ok(())
            }
            12 => self.salta_struct(profondita - 1),
            13 => self.avanza(16),
            _ => Err(errore(MSG_HEADER_PAGINA_ILLEGGIBILE)),
        }
    }

    /// Legge il prossimo field header, o `None` sullo STOP.
    fn prossimo_campo(&mut self, corrente: &mut i64) -> Result<Option<u8>> {
        let intestazione = self.byte()?;
        if intestazione == 0 {
            return Ok(None);
        }
        let delta = i64::from(intestazione >> 4);
        let tipo = intestazione & 0x0F;
        *corrente = if delta == 0 {
            self.zigzag()?
        } else {
            corrente
                .checked_add(delta)
                .ok_or_else(|| errore(MSG_HEADER_PAGINA_ILLEGGIBILE))?
        };
        Ok(Some(tipo))
    }

    fn salta_struct(&mut self, profondita: u32) -> Result<()> {
        if profondita == 0 {
            return Err(errore(MSG_HEADER_PAGINA_ILLEGGIBILE));
        }
        let mut corrente: i64 = 0;
        while let Some(tipo) = self.prossimo_campo(&mut corrente)? {
            self.salta(tipo, Posizione::Campo, profondita - 1)?;
        }
        Ok(())
    }
}

/// I due numeri di un `PageHeader` che governano l'allocazione e la catena,
/// piu' la lunghezza dell'header stesso.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Intestazione {
    pub non_compressi: i64,
    pub compressi: i64,
    pub byte_header: usize,
}

/// Legge un `PageHeader` da una fetta, senza allocare.
///
/// # Errors
///
/// `DataMapping` se l'header non si legge dentro la fetta, se un campo ha un
/// tipo Thrift che non esiste, se un varint non termina o non e'
/// rappresentabile, o se manca uno dei due campi obbligatori.
pub fn leggi_intestazione(dati: &[u8]) -> Result<Intestazione> {
    let mut lettore = Compatto::nuovo(dati);
    let mut non_compressi: Option<i64> = None;
    let mut compressi: Option<i64> = None;
    let mut corrente: i64 = 0;
    while let Some(tipo) = lettore.prossimo_campo(&mut corrente)? {
        match (corrente, tipo) {
            (2, 5) => non_compressi = Some(lettore.zigzag()?),
            (3, 5) => compressi = Some(lettore.zigzag()?),
            _ => lettore.salta(tipo, Posizione::Campo, PROFONDITA_MASSIMA)?,
        }
    }
    match (non_compressi, compressi) {
        (Some(non_compressi), Some(compressi)) => Ok(Intestazione {
            non_compressi,
            compressi,
            byte_header: lettore.i,
        }),
        _ => Err(errore(MSG_HEADER_PAGINA_ILLEGGIBILE)),
    }
}

/// Verifica le dimensioni dichiarate da ogni pagina di un chunk.
///
/// Cammina la catena delle pagine — la successiva comincia dove finisce
/// l'header della precedente, piu' i suoi byte compressi — e per ognuna
/// controlla, **prima** che il decoder la veda:
///
/// * `uncompressed_page_size` non maggiore di quanto il chunk dichiari in
///   tutto. E' l'invariante del formato, non una quota nostra: la somma delle
///   pagine non compresse **e'** il totale del chunk, quindi una sola pagina
///   non puo' superarlo;
/// * `uncompressed_page_size` dentro il tetto per pagina, che il chiamante
///   calcola dalla **memoria** dichiarata e passa qui.
///
/// Il secondo non e' un numero scelto qui, ed e' il controllo che conta. Una
/// pagina puo' essere coerente col proprio chunk e comunque troppo grande per
/// il processo che la legge: un chunk da 800 MiB con una pagina da 700 MiB
/// soddisfa il primo controllo e aborta lo stesso sotto mezzo gigabyte di
/// memoria.
///
/// La quota giusta e' quella della **memoria**, non quella dell'ingresso.
/// `max_input_bytes` governa la dimensione della sorgente; una pagina
/// decompressa e' memoria temporanea, e usare la prima per la seconda
/// confonderebbe due quote che il modello tiene distinte apposta — con
/// l'effetto pratico che alzare il tetto sul file alzerebbe anche quello sulla
/// memoria, che non e' cio' che chi lo alza sta chiedendo.
///
/// La catena deve finire **esattamente** sulla fine del chunk. Fermarsi a
/// «l'abbiamo superata» lascerebbe passare un'ultima pagina che sborda: i byte
/// oltre il chunk sono di un'altra colonna, e una pagina che li rivendica non
/// e' una pagina lunga, e' un chunk che mente.
///
/// # Errors
///
/// `DataMapping` con messaggio statico. Il file non viene letto oltre gli
/// header: nessuna pagina viene decompressa qui.
pub fn valida_chunk(
    sorgente: &File,
    primo_byte: u64,
    byte_compressi: u64,
    non_compressi_del_chunk: i64,
    tetto_pagina: u64,
) -> Result<()> {
    if byte_compressi == 0 {
        return Ok(());
    }
    let mut lettore = sorgente
        .try_clone()
        .map_err(|_| errore(MSG_HEADER_PAGINA_ILLEGGIBILE))?;
    let mut finestra = vec![0_u8; FINESTRA_HEADER];
    let fine = primo_byte
        .checked_add(byte_compressi)
        .ok_or_else(|| errore(MSG_HEADER_PAGINA_ILLEGGIBILE))?;
    let mut offset = primo_byte;

    while offset < fine {
        let residuo_chunk = fine - offset;
        lettore
            .seek(SeekFrom::Start(offset))
            .map_err(|_| errore(MSG_HEADER_PAGINA_ILLEGGIBILE))?;
        // Il minimo si prende in `u64` e **poi** si converte: cosi' il valore
        // convertito e' per costruzione <= FINESTRA_HEADER, e non serve un
        // default di ripiego per un `try_from` che non puo' fallire.
        let quanti = usize::try_from(residuo_chunk.min(FINESTRA_HEADER as u64))
            .map_err(|_| errore(MSG_HEADER_PAGINA_ILLEGGIBILE))?;
        let letti = leggi_al_piu(&mut lettore, &mut finestra[..quanti])?;
        let intestazione = leggi_intestazione(&finestra[..letti])?;

        if intestazione.non_compressi < 0 || intestazione.compressi < 0 {
            return Err(errore(MSG_HEADER_PAGINA_ILLEGGIBILE));
        }
        if intestazione.non_compressi > non_compressi_del_chunk {
            return Err(errore(MSG_PAGINA_OLTRE_IL_CHUNK));
        }
        // Il confronto con il tetto avviene in `u64`, convertendo il numero
        // **letto dal file** e non il tetto: convertire il tetto avrebbe
        // richiesto un ripiego per il caso in cui non entra in `i64`, e un
        // ripiego su un tetto e' un tetto che a volte non c'e'. Il segno e'
        // gia' stato verificato sopra, quindi qui la conversione non puo'
        // fallire — ma resta fallibile invece che assunta.
        let non_compressi = u64::try_from(intestazione.non_compressi)
            .map_err(|_| errore(MSG_HEADER_PAGINA_ILLEGGIBILE))?;
        if non_compressi > tetto_pagina {
            return Err(errore(MSG_PAGINA_OLTRE_LA_MEMORIA));
        }

        let passo = u64::try_from(intestazione.byte_header)
            .ok()
            .and_then(|header| {
                u64::try_from(intestazione.compressi)
                    .ok()
                    .and_then(|dati| header.checked_add(dati))
            })
            .ok_or_else(|| errore(MSG_HEADER_PAGINA_ILLEGGIBILE))?;
        if passo == 0 {
            // Senza questo, una pagina che dichiara zero byte e un header di
            // lunghezza zero — impossibile, ma il file lo puo' scrivere —
            // farebbe girare il ciclo per sempre.
            return Err(errore(MSG_CATENA_PAGINE_NON_PROGREDISCE));
        }
        if passo > residuo_chunk {
            return Err(errore(MSG_CATENA_PAGINE_NON_CHIUDE));
        }
        offset += passo;
    }
    if offset == fine {
        Ok(())
    } else {
        Err(errore(MSG_CATENA_PAGINE_NON_CHIUDE))
    }
}

/// Riempie il buffer per quanto il file consente, senza pretendere che sia
/// pieno: l'ultimo header di un chunk sta vicino alla fine del file.
fn leggi_al_piu(lettore: &mut File, buffer: &mut [u8]) -> Result<usize> {
    let mut letti = 0;
    while letti < buffer.len() {
        match lettore.read(&mut buffer[letti..]) {
            Ok(0) => break,
            Ok(n) => letti += n,
            Err(errore_io) if errore_io.kind() == std::io::ErrorKind::Interrupted => {}
            Err(_) => return Err(errore(MSG_HEADER_PAGINA_ILLEGGIBILE)),
        }
    }
    Ok(letti)
}
