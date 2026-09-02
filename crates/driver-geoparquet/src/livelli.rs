//! Il flusso ibrido RLE/bit-packed dei livelli, verificato prima del decoder.
//!
//! # Che cosa panica, e perche' la prevalidazione precedente non bastava
//!
//! I livelli di definizione e ripetizione di una data page sono un flusso
//! **ibrido**: una sequenza di run, ciascuno introdotto da un varint. Bit zero
//! acceso significa bit-packed -- il resto del varint conta i gruppi da otto
//! valori -- e spento significa RLE, con il resto che conta le ripetizioni di un
//! valore scritto subito dopo.
//!
//! Con `max_def_level` uguale a uno, `parquet` non passa da un decoder generico:
//! usa `PackedDecoder`, che per un run bit-packed prende una fetta dei byte
//! rimanenti e la consegna a `BooleanBufferBuilder::append_packed_range`. Il
//! numero di bit lo dichiara l'header del run; i byte disponibili li ha il
//! buffer. Quando il primo e' piu' grande dei secondi, l'intervallo esce dal
//! buffer e arrow abbatte il processo -- `offset + len out of bounds`,
//! `arrow-buffer 59.1.0`, `util/bit_chunk_iterator.rs:224`.
//!
//! La prevalidazione che c'era guardava le **dimensioni** delle pagine e il bit
//! width degli indici di dizionario. Erano due difese vere e restavano fuori da
//! questa: una pagina puo' avere dimensioni coerenti in ogni campo del proprio
//! header, non essere a dizionario, e portare dentro la sezione dei livelli un
//! run che dichiara piu' valori di quanti byte la sezione contenga. Nessuna
//! delle due domande precedenti riguarda cio' che sta **dentro** la sezione.
//!
//! # Perche' non ci si affida alla barriera
//!
//! Un `catch_unwind` c'e' e trasforma il panico in errore tipizzato. Non e'
//! sufficiente per la stessa ragione gia' scritta per FZ-0.1: il panico e'
//! avvenuto. Nel profilo che spediamo l'unwinding attraversa codice di terzi che
//! non e' scritto per essere interrotto a meta', e sotto il fuzzer il panic hook
//! chiama `abort()` prima ancora di srotolare. Una difesa che dipende
//! dall'ordine in cui il panico viene intercettato e' una difesa che si spegne
//! quando cambia quell'ordine.
//!
//! # Che cosa questo modulo **non** afferma
//!
//! Che i livelli siano semanticamente corretti. Un flusso ben formato puo'
//! dichiarare livelli maggiori del massimo della colonna, e quello resta un
//! problema del decoder: qui si verifica soltanto che ogni run stia dentro i
//! propri byte e che l'insieme copra i valori dichiarati. E' il confine giusto,
//! perche' e' esattamente cio' che decide se il decoder legge dentro o fuori il
//! buffer.

use plenora_io_model::{PlenoraIoError, PublicMessage};

type Result<T> = std::result::Result<T, PlenoraIoError>;

/// I messaggi sono **statici**: nessun numero letto dal file esce di qui.
///
/// Un messaggio che riportasse il conteggio del run o i byte mancanti porterebbe
/// fuori un valore derivato dal payload, e `PlenoraIoError::message` dichiara di
/// non contenerne. Il valore che serve a correggere il file sta nel file.
pub const MSG_RUN_OLTRE_LA_SEZIONE: &str =
    "flusso dei livelli Parquet con un run oltre la fine della propria sezione";
pub const MSG_RUN_VUOTO: &str = "flusso dei livelli Parquet con un run che non copre valori";
pub const MSG_VARINT_NON_TERMINATO: &str =
    "flusso dei livelli Parquet con un varint che non termina";
pub const MSG_LIVELLI_INSUFFICIENTI: &str =
    "flusso dei livelli Parquet piu' corto dei valori dichiarati dalla pagina";
pub const MSG_LIVELLI_NON_RAPPRESENTABILI: &str =
    "flusso dei livelli Parquet con conteggi non rappresentabili";
pub const MSG_BIT_WIDTH_LIVELLI_NULLO: &str = "flusso dei livelli Parquet con bit width nullo";

/// Byte massimi di un varint che deve stare in `u32`.
///
/// Cinque gruppi da sette bit fanno trentacinque bit: il sesto byte non puo'
/// appartenere a un `u32`, e leggerlo sarebbe seguire un flusso che dichiara di
/// non essere quello che dice. `parquet` si ferma allo stesso punto.
const MAX_BYTE_VARINT: usize = 5;

/// Lo stesso costruttore del resto del driver: `Format`, fase `Read`, redatto.
///
/// Non un errore proprio di questo modulo: chi riceve deve vedere lo stesso
/// codice per «il file non e' leggibile», qualunque sia la difesa che l'ha
/// fermato. Un codice nuovo per ogni difesa costringerebbe chi integra a
/// inseguirle.
fn errore(messaggio: &'static str) -> PlenoraIoError {
    PlenoraIoError::formato_redatto("geoparquet", &PublicMessage::Curated(messaggio))
}

/// Legge un varint ULEB128 che deve stare in `u32`.
///
/// Restituisce il valore e quanti byte ha consumato.
fn varint(dati: &[u8], da: usize) -> Result<(u32, usize)> {
    let mut valore: u64 = 0;
    let mut spostamento: u32 = 0;
    for consumati in 0..MAX_BYTE_VARINT {
        let byte = *dati
            .get(da + consumati)
            .ok_or_else(|| errore(MSG_VARINT_NON_TERMINATO))?;
        valore |= u64::from(byte & 0x7F) << spostamento;
        if byte & 0x80 == 0 {
            let valore =
                u32::try_from(valore).map_err(|_| errore(MSG_LIVELLI_NON_RAPPRESENTABILI))?;
            return Ok((valore, consumati + 1));
        }
        spostamento += 7;
    }
    Err(errore(MSG_VARINT_NON_TERMINATO))
}

/// Verifica che il flusso ibrido copra `valori_attesi` senza uscire da `dati`.
///
/// # Errors
///
/// [`PlenoraIoError`] `Format` se un varint non termina dentro la sezione, se un
/// run dichiara piu' byte di quanti la sezione ne contenga, se un run non copre
/// alcun valore -- che non farebbe avanzare la lettura -- o se il flusso finisce
/// prima di aver coperto i valori che la pagina dichiara.
///
/// # Bounded
///
/// Non alloca e non trattiene: scorre `dati` una volta sola, e ogni giro
/// consuma almeno un byte. Il numero di giri e' quindi limitato dalla lunghezza
/// della sezione, non da cio' che la sezione dichiara.
pub fn valida_flusso(dati: &[u8], bit_width: u8, valori_attesi: u32) -> Result<()> {
    if valori_attesi == 0 {
        // Una pagina che non dichiara valori non ha livelli da coprire. La
        // sezione puo' essere vuota, e non e' un difetto.
        return Ok(());
    }
    if bit_width == 0 {
        // Non raggiungibile dai chiamanti, che chiamano solo con livello
        // massimo maggiore di zero. Rifiutato lo stesso: con bit width nullo un
        // run bit-packed consuma zero byte, e un ciclo che non consuma byte non
        // termina. Dedurre la terminazione da una precondizione del chiamante e'
        // esattamente il ragionamento che questo modulo esiste per non fare.
        return Err(errore(MSG_BIT_WIDTH_LIVELLI_NULLO));
    }

    let larghezza = u64::from(bit_width);
    // Il valore di un run RLE sta in `ceil(bit_width / 8)` byte.
    let byte_del_valore = usize::from(bit_width).div_ceil(8);
    let attesi = u64::from(valori_attesi);

    let mut posizione = 0usize;
    let mut coperti: u64 = 0;
    while coperti < attesi {
        let (intestazione, consumati) = varint(dati, posizione)?;
        posizione += consumati;

        let (byte_del_run, valori_del_run) = if intestazione & 1 == 1 {
            // Bit-packed: `intestazione >> 1` gruppi da otto valori, e ogni
            // gruppo occupa esattamente `bit_width` byte.
            let gruppi = u64::from(intestazione >> 1);
            let byte = gruppi
                .checked_mul(larghezza)
                .ok_or_else(|| errore(MSG_LIVELLI_NON_RAPPRESENTABILI))?;
            let valori = gruppi
                .checked_mul(8)
                .ok_or_else(|| errore(MSG_LIVELLI_NON_RAPPRESENTABILI))?;
            (byte, valori)
        } else {
            // RLE: `intestazione >> 1` ripetizioni di un valore solo.
            (byte_del_valore as u64, u64::from(intestazione >> 1))
        };

        if valori_del_run == 0 {
            // Un run che non copre valori non fa avanzare il conteggio. Senza
            // questo rifiuto un flusso di run vuoti girerebbe finche' i byte
            // bastano, e con `coperti` fermo il ciclo dipenderebbe soltanto
            // dalla lunghezza della sezione per fermarsi.
            return Err(errore(MSG_RUN_VUOTO));
        }

        // L'aritmetica torna in `usize` **prima** del confronto, e non con un
        // cast: un `as` su una macchina a puntatori stretti troncherebbe proprio
        // il numero grande, cioe' esattamente quello che questo confronto deve
        // respingere. `try_from` lo trasforma in un rifiuto.
        let byte_del_run =
            usize::try_from(byte_del_run).map_err(|_| errore(MSG_LIVELLI_NON_RAPPRESENTABILI))?;
        let fine = posizione
            .checked_add(byte_del_run)
            .ok_or_else(|| errore(MSG_LIVELLI_NON_RAPPRESENTABILI))?;
        if fine > dati.len() {
            // **Il difetto.** Il run dichiara piu' byte di quanti la sezione ne
            // porti: e' qui che `PackedDecoder` costruirebbe l'intervallo che
            // esce dal buffer di arrow.
            return Err(errore(MSG_RUN_OLTRE_LA_SEZIONE));
        }
        posizione = fine;
        coperti = coperti
            .checked_add(valori_del_run)
            .ok_or_else(|| errore(MSG_LIVELLI_NON_RAPPRESENTABILI))?;
    }
    Ok(())
}

/// Come [`valida_flusso`], ma si ferma anche quando il flusso finisce presto.
///
/// Separata perche' le due condizioni sono diverse e la prima e' quella che
/// panica: un run che sfora la sezione fa leggere fuori dal buffer, un flusso
/// che finisce prima fa leggere **dentro** ma meno del dovuto. La seconda e'
/// comunque un rifiuto -- il decoder proseguirebbe su byte che appartengono ai
/// valori -- e avere due messaggi distinti dice quale delle due si e' vista.
///
/// # Errors
///
/// [`PlenoraIoError`] `Format` come [`valida_flusso`], piu'
/// [`MSG_LIVELLI_INSUFFICIENTI`] se la sezione si esaurisce prima dei valori
/// dichiarati.
pub fn valida_sezione(dati: &[u8], bit_width: u8, valori_attesi: u32) -> Result<()> {
    match valida_flusso(dati, bit_width, valori_attesi) {
        // `valida_flusso` chiede un varint oltre la fine quando i run finiscono
        // prima dei valori: la sezione e' integra, e' corta.
        Err(e) if e.message == MSG_VARINT_NON_TERMINATO => Err(errore(MSG_LIVELLI_INSUFFICIENTI)),
        altro => altro,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Un run bit-packed con tutti i propri byte passa.
    #[test]
    fn un_flusso_bit_packed_completo_passa() {
        // Intestazione 0b11 = un gruppo bit-packed, otto valori, un byte a
        // `bit_width` uno.
        let flusso = [0b0000_0011, 0b1010_1010];
        valida_flusso(&flusso, 1, 8).expect("otto valori in un byte");
    }

    /// Il difetto, ridotto: l'intestazione promette un gruppo e i byte non ci
    /// sono.
    #[test]
    fn un_run_bit_packed_senza_i_propri_byte_e_rifiutato() {
        let flusso = [0b0000_0011];
        let errore = valida_flusso(&flusso, 1, 8).expect_err("il gruppo non ha il proprio byte");
        assert_eq!(errore.message, MSG_RUN_OLTRE_LA_SEZIONE);
    }

    /// Lo stesso con piu' gruppi: il conto e' `gruppi * bit_width` byte.
    #[test]
    fn i_byte_di_un_run_bit_packed_sono_gruppi_per_bit_width() {
        let intestazione = 0b0000_0101; // due gruppi
        let mut corto = vec![intestazione];
        corto.extend_from_slice(&[0xAA; 3]); // ne servirebbero quattro a bit width due
        let errore = valida_flusso(&corto, 2, 16).expect_err("manca un byte");
        assert_eq!(errore.message, MSG_RUN_OLTRE_LA_SEZIONE);

        let mut intero = vec![intestazione];
        intero.extend_from_slice(&[0xAA; 4]);
        valida_flusso(&intero, 2, 16).expect("quattro byte bastano");
    }

    /// Un run RLE porta il proprio valore in `ceil(bit_width / 8)` byte.
    #[test]
    fn un_run_rle_senza_il_proprio_valore_e_rifiutato() {
        let flusso = [0b0000_1000]; // quattro ripetizioni, valore assente
        let errore = valida_flusso(&flusso, 1, 4).expect_err("il valore non c'e'");
        assert_eq!(errore.message, MSG_RUN_OLTRE_LA_SEZIONE);

        valida_flusso(&[0b0000_1000, 0x01], 1, 4).expect("con il valore passa");
    }

    /// Un run che non copre valori non fa avanzare il conteggio.
    #[test]
    fn un_run_vuoto_e_rifiutato_invece_di_far_girare_il_ciclo() {
        for intestazione in [0b0000_0000u8, 0b0000_0001] {
            let errore = valida_flusso(&[intestazione, 0x00], 1, 8)
                .expect_err("un run vuoto non copre niente");
            assert_eq!(
                errore.message, MSG_RUN_VUOTO,
                "intestazione {intestazione:#b}"
            );
        }
    }

    /// Una sezione che finisce prima dei valori dichiarati e' un rifiuto, e
    /// **non** lo stesso rifiuto di un run che sfora.
    #[test]
    fn una_sezione_corta_e_un_rifiuto_diverso_da_un_run_che_sfora() {
        // Un run RLE completo da quattro valori, ma la pagina ne dichiara otto.
        let errore = valida_sezione(&[0b0000_1000, 0x01], 1, 8).expect_err("copre solo quattro");
        assert_eq!(errore.message, MSG_LIVELLI_INSUFFICIENTI);
    }

    /// Un varint che non termina dentro la sezione.
    #[test]
    fn un_varint_che_non_termina_e_rifiutato() {
        let errore = valida_flusso(&[0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF], 1, 8)
            .expect_err("il varint non chiude in cinque byte");
        assert_eq!(errore.message, MSG_VARINT_NON_TERMINATO);
    }

    /// L'ultimo run bit-packed puo' coprire piu' valori del necessario: il
    /// formato riempie fino al multiplo di otto, e rifiutarlo scarterebbe file
    /// leciti.
    #[test]
    fn un_ultimo_run_che_eccede_e_ammesso() {
        let flusso = [0b0000_0011, 0b1010_1010];
        valida_flusso(&flusso, 1, 5).expect("otto valori per cinque attesi");
    }

    /// Zero valori attesi: non c'e' niente da coprire, nemmeno con la sezione
    /// vuota.
    #[test]
    fn una_pagina_senza_valori_non_pretende_livelli() {
        valida_flusso(&[], 1, 0).expect("nessun valore, nessun livello");
    }

    /// Bit width nullo: irraggiungibile dai chiamanti, rifiutato lo stesso.
    ///
    /// Con zero byte per gruppo un run bit-packed non consuma niente, e il
    /// ciclo dipenderebbe da un invariante del chiamante per terminare.
    #[test]
    fn un_bit_width_nullo_e_rifiutato_invece_che_dedotto_impossibile() {
        let errore = valida_flusso(&[0b0000_0011], 0, 8).expect_err("bit width nullo");
        assert_eq!(errore.message, MSG_BIT_WIDTH_LIVELLI_NULLO);
    }

    /// Il conteggio dei giri e' limitato dai byte, non da cio' che i byte
    /// dichiarano: ogni giro ne consuma almeno uno.
    #[test]
    fn ogni_giro_consuma_almeno_un_byte() {
        // Quattro run RLE da un valore ciascuno: due byte per run.
        let mut flusso = Vec::new();
        for _ in 0..4 {
            flusso.push(0b0000_0010); // una ripetizione
            flusso.push(0x01);
        }
        valida_flusso(&flusso, 1, 4).expect("quattro run coprono quattro valori");
    }
}
