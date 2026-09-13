//! Il gestore dei segnali del processo, e la decisione che prende.
//!
//! # Perche' sta in un file proprio
//!
//! Il ramo che chiama `std::process::exit` non e' osservabile da un test in
//! processo: terminerebbe il harness. L'unico modo di provarlo e' un processo
//! che installi **questo** gestore e muoia davvero, e un processo del genere
//! deve poter compilare questo codice senza che sia il binario di produzione.
//!
//! Il file e' percio' incluso due volte: da `main.rs`, che e' la CLI, e da
//! `tests/sonda_dei_segnali.rs`, che e' la sonda. Non e' una copia -- e' lo
//! stesso file -- e la differenza conta: una copia sarebbe potuta divergere,
//! e la sonda avrebbe continuato a provare un gestore che non esiste piu'.
//!
//! Qui non c'e' un `mod tests`: il file viene compilato anche dal crate della
//! sonda, dove `cfg(test)` e' attivo, e le stesse unita' comparirebbero due
//! volte nell'elenco del harness. Le sonde della decisione stanno in `main.rs`,
//! che e' l'unico posto dove vengono elencate una volta sola.

use std::sync::atomic::{AtomicBool, Ordering};

use plenora_io_model::{
    CancellationToken, ErrorCategory, ErrorPhase, IoErrorCode, PlenoraIoError, PublicMessage,
    RemoteEffect, RetryDisposition,
};

/// L'exit code di un'operazione annullata: `128 + SIGINT`, come la shell si
/// aspetta. E' lo stesso che `map_err` assegna alla categoria `Cancelled`, e
/// resta lo stesso quando a uscire e' il secondo segnale invece della pipeline.
pub const EXIT_ANNULLATO: i32 = 130;

/// Il rifiuto quando la cancellazione non si puo' armare.
///
/// # Perche' non e' piu' un avviso su `stderr`
///
/// Lo era, e il ragionamento scritto qui diceva: un gestore che non si installa
/// non rende sbagliato nulla di cio' che il comando produce, toglie soltanto la
/// possibilita' di fermarlo con grazia; rifiutare di lavorare per questo
/// sarebbe un danno certo contro un rischio che riguarda una directory
/// temporanea.
///
/// Il ragionamento pesava le due cose sbagliate. La scelta non era fra
/// «avvisare» e «rifiutare»: era fra **rompere il contratto della busta** e
/// rifiutare. `stderr` porta un solo documento JSON, e un avviso testuale
/// stampato all'avvio finisce **davanti** alla busta quando il comando poi
/// fallisce -- cioe' proprio nel caso in cui l'avviso conta di piu', perche' e'
/// il caso in cui lo staging resta sul disco. Chi legge `stderr` con un parser
/// trova due documenti dove il contratto ne promette uno, e non li trova mai in
/// prova: il ramo non ha input, quindi nessuna sonda lo esercita.
///
/// Un errore tipizzato tiene un documento solo, e' leggibile da una macchina, e
/// dice la stessa cosa che diceva l'avviso. Il costo -- la CLI si rifiuta di
/// lavorare dove i segnali non si armano -- e' reale, e resta una decisione di
/// prodotto: se un giorno esistesse una piattaforma supportata dove
/// `set_handler` fallisce per davvero, la strada e' una diagnostica pubblica
/// **versionata**, non il ritorno alla riga di testo.
pub const RIFIUTO_SEGNALI: &str =
    "gestore dei segnali non installabile: senza, Ctrl+C terminerebbe il processo senza \
     annullare l'operazione e lo staging resterebbe sul disco";

/// Arma il token del processo al primo segnale, esce al secondo.
///
/// # La cancellazione e' cooperativa
///
/// Il token non interrompe niente: lo si osserva. La pipeline lo controlla ai
/// propri punti di verifica — fra un batch e il successivo, prima di ogni
/// scrittura, all'ingresso di ogni operazione — e da li' ritorna un errore
/// tipizzato che il consumatore vede come `CANCELLED`. Fra due punti di
/// verifica passa il tempo che passa.
///
/// Cio' che questo garantisce e cio' che non garantisce va detto insieme:
///
/// * **garantito** — il ritorno ordinato fa cadere lo staging e lo spool, che
///   sono `TempDir` e descrittori scollegati: si liberano con lo stack, in
///   errore come in successo, e la destinazione non viene pubblicata;
/// * **non garantito** — l'istante. Dentro una chiamata nativa che non torna —
///   una `OGR_*` di GDAL sul percorso `FileGDB` e' l'esempio vero — il token
///   non viene guardato da nessuno, e nessun codice in spazio utente puo'
///   farlo guardare senza abbandonare la libreria a meta'.
///
/// Il **secondo** segnale esiste per quel caso: chi lo manda ha gia' chiesto e
/// sta dicendo che non aspetta oltre. Il processo esce subito, e cio' che lo
/// staging aveva in corso resta dov'e'. E' la stessa cosa che farebbe la
/// disposizione predefinita di `SIGINT`, dichiarata invece che subita.
///
/// # Errors
///
/// [`PlenoraIoError`] se il gestore non si installa. Vedi [`RIFIUTO_SEGNALI`]
/// per la ragione per cui e' un rifiuto e non un avviso.
pub fn installa_gestore_dei_segnali() -> Result<CancellationToken, PlenoraIoError> {
    let token = CancellationToken::new();
    let armato = token.clone();
    let gia_chiesto = std::sync::Arc::new(AtomicBool::new(false));
    // `ctrlc` non esegue il gestore **dentro** il contesto del segnale: il
    // gestore del sistema si limita a svegliare un thread dedicato, che poi
    // chiama questa chiusura. E' cio' che rende lecito prendere un lock qui —
    // `cancel` ne prende due — dove un gestore vero ammetterebbe solo funzioni
    // async-signal-safe.
    //
    // Nella chiusura resta **una sola** decisione, e non e' una decisione: e'
    // l'uscita. Tutto cio' che si puo' provare senza un segnale sta in
    // [`reagisci_al_segnale`].
    if ctrlc::set_handler(move || {
        if reagisci_al_segnale(&gia_chiesto, &armato) == AzioneDelSegnale::UsciSubito {
            std::process::exit(EXIT_ANNULLATO);
        }
    })
    .is_err()
    {
        return Err(PlenoraIoError::redatto(
            IoErrorCode::Generic,
            ErrorCategory::InvalidConfiguration,
            ErrorPhase::Validate,
            RemoteEffect::None,
            RetryDisposition::Never,
            &PublicMessage::Curated(RIFIUTO_SEGNALI),
        ));
    }
    Ok(token)
}

/// Che cosa fare quando arriva un segnale.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AzioneDelSegnale {
    /// Primo segnale: si annulla in modo cooperativo e si lascia rientrare la
    /// pipeline, che liberera' staging e spool scendendo lo stack.
    Annulla,
    /// Segnale successivo: chi lo manda ha gia' chiesto e non aspetta oltre.
    UsciSubito,
}

/// La decisione primo/segnale-successivo, con il suo effetto sul token.
///
/// # Perche' sta fuori dalla chiusura
///
/// Per essere **provabile senza un segnale**. Dentro la chiusura le due
/// transizioni si sarebbero potute osservare solo mandando `SIGINT` a un
/// processo vero, e la seconda avrebbe richiesto di vincere una corsa contro il
/// rientro della prima: fra il primo segnale e l'uscita del processo passano
/// millisecondi. Una sonda costruita cosi' non prova la transizione, prova chi
/// arriva primo — ed e' esattamente il difetto che questa tranche ha gia'
/// trovato una volta, in una sonda tarata sul carico della macchina.
///
/// Qui le transizioni sono due chiamate e l'ordine lo decide il test.
///
/// # Che cosa resta fuori
///
/// La sola `std::process::exit`, che un test in processo non puo' osservare per
/// costruzione: terminerebbe il harness. E' censita in ASSURANCE-N1 invece di
/// essere dichiarata coperta.
///
/// # L'ordine dentro la funzione
///
/// `swap` prima di `cancel`, e non il contrario: due segnali che arrivassero
/// insieme devono vedere **uno solo** vincere la prima transizione, e a
/// deciderlo dev'essere l'operazione atomica, non l'ordine in cui i due thread
/// entrano in `cancel`.
pub fn reagisci_al_segnale(
    gia_chiesto: &AtomicBool,
    token: &CancellationToken,
) -> AzioneDelSegnale {
    if gia_chiesto.swap(true, Ordering::SeqCst) {
        return AzioneDelSegnale::UsciSubito;
    }
    token.cancel();
    AzioneDelSegnale::Annulla
}
