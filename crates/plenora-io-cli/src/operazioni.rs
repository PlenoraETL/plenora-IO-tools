//! La superficie Rust pubblica: le sei operazioni del catalogo, per nome.
//!
//! # Perché questo modulo esiste
//!
//! Il profilo io-tools richiede una superficie Rust con una mappatura
//! versionata verso le operazioni, verificata da un consumatore che importi
//! **solo** gli export documentati. Le operazioni erano implementate dentro il
//! binario, dove nessuno può importarle, e gli unici nomi disponibili erano
//! quelli del binding — `cmd_read`, `Cli` — cioè il vocabolario della riga di
//! comando.
//!
//! Qui i nomi sono quelli del catalogo comune, e l'ingresso è una `Richiesta`
//! con campi nominati invece di una struttura di argomenti con posizionali. La
//! differenza non è cosmetica: un consumatore Rust che dovesse sapere che la
//! sorgente è `positionals[0]` starebbe programmando contro la CLI, non contro
//! l'operazione.
//!
//! # Che cosa questa superficie **non** promette
//!
//! Un risultato tipizzato. Ogni operazione rende il documento JSON del proprio
//! contratto d'uscita — `plenora-io-read-result-v1` e compagni — e non una
//! struct Rust.
//!
//! È una scelta, e ha un costo: chi consuma da Rust legge `Value` e non campi.
//! In cambio l'equivalenza fra le due superfici, che SURF-017 e CLI-2.0 §10
//! pretendono, non è una promessa da verificare ma una conseguenza: la CLI e
//! questo modulo chiamano la stessa funzione e rendono lo stesso documento, e
//! gli schemi pubblicati in `contracts/schemas/` lo governano per entrambe.
//! Strutture tipizzate potranno arrivare **accanto** a questo, senza cambiare
//! la mappatura: sarebbero un'aggiunta, non una sostituzione.
//!
//! # L'errore
//!
//! Ogni operazione rende `Err(documento)` con la busta d'errore del contratto
//! comune: i quattro assi, il codice, la fase, l'effetto remoto e la politica
//! di ritentare. Il **codice d'uscita del processo** non c'è, e non è una
//! dimenticanza: è una proiezione del binding, e `uscita_della_categoria` la
//! applica nel binario. Un'API che rendesse un `i32` costringerebbe chi la usa
//! in libreria a ragionare su un numero che riguarda un processo che non ha
//! avviato.

use std::collections::BTreeMap;
use std::path::PathBuf;

use plenora_io_model::{budget::PipelineLimits, CancellationToken};
use serde_json::Value;

use crate::Cli;

/// L'ingresso di un'operazione, con i campi che i contratti dichiarano.
///
/// I nomi sono quelli degli schemi `*-input-v1` di `contracts/schemas/`, non
/// quelli dei flag: `source_format` e non `--from`. Chi programma contro
/// l'operazione legge lo schema; chi programma contro la CLI legge l'aiuto.
///
/// I campi che un'operazione non usa vengono ignorati da quella operazione. Non
/// è permissività: è che le sei condividono un ingresso perché condividono la
/// pipeline, e sei struct diverse avrebbero fatto sei conversioni identiche.
/// Che cosa ciascuna richiede lo dice il suo schema, e lo fa rispettare
/// l'operazione con un errore tipizzato -- non il tipo Rust.
///
/// # Come si costruisce, e perché non con un letterale
///
/// Con `sulla_sorgente` e i metodi `con_*`, che si concatenano. Il letterale
/// `Richiesta { .. }` **non** compila da fuori: la struct è
/// `#[non_exhaustive]`, e un campo aggiunto in futuro non deve rompere chi la
/// costruisce. È una proprietà che ho scoperto sbagliando — la prima stesura
/// documentava `..Default::default()` come idioma, e non compilava nemmeno
/// nelle prove d'integrazione di questo crate, che per Rust sono già "fuori".
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct Richiesta {
    /// La sorgente da leggere, ispezionare o convertire.
    pub source: Option<PathBuf>,
    /// La destinazione da produrre: consegna di `read`, sink di `write` e
    /// `convert`.
    pub destination: Option<PathBuf>,
    /// Il formato della sorgente, quando l'operazione lo richiede esplicito.
    pub source_format: Option<String>,
    /// Il formato del sink, quando l'operazione lo richiede esplicito.
    pub target_format: Option<String>,
    /// L'identificatore numerico del layer, come `io.layers` lo rende.
    pub layer: Option<u32>,
    /// Tetto alle righe **lette**. Non convive con `destination` in `io.read`.
    pub limit: Option<usize>,
    /// Il CRS da assumere quando la sorgente non lo dichiara.
    pub assume_crs: Option<String>,
    /// Opzioni comuni ai due driver.
    pub options: BTreeMap<String, String>,
    /// Opzioni del driver di lettura.
    pub input_options: BTreeMap<String, String>,
    /// Opzioni del driver di scrittura. Richiede `destination`.
    pub output_options: BTreeMap<String, String>,
    /// Chiede la durabilità alla chiusura. Richiede `destination`.
    pub durable: bool,
    /// I tetti di risorsa della pipeline.
    pub limits: PipelineLimits,
    /// Il canale con cui annullare l'operazione dall'esterno.
    pub cancellation: CancellationToken,
}

impl Richiesta {
    /// Una richiesta sulla sola sorgente: la forma di `inspect` e `layers`.
    #[must_use]
    pub fn sulla_sorgente(source: impl Into<PathBuf>) -> Self {
        Self {
            source: Some(source.into()),
            ..Self::default()
        }
    }

    /// La destinazione: consegna di `read`, sink di `write` e `convert`.
    #[must_use]
    pub fn con_destinazione(mut self, destination: impl Into<PathBuf>) -> Self {
        self.destination = Some(destination.into());
        self
    }

    /// Il formato della sorgente, per le operazioni che lo richiedono esplicito.
    #[must_use]
    pub fn con_formato_sorgente(mut self, formato: impl Into<String>) -> Self {
        self.source_format = Some(formato.into());
        self
    }

    /// Il formato del sink, per le operazioni che lo richiedono esplicito.
    #[must_use]
    pub fn con_formato_destinazione(mut self, formato: impl Into<String>) -> Self {
        self.target_format = Some(formato.into());
        self
    }

    /// Il layer da selezionare, per identificatore numerico.
    #[must_use]
    pub const fn con_layer(mut self, layer: u32) -> Self {
        self.layer = Some(layer);
        self
    }

    /// Il tetto alle righe **lette**.
    #[must_use]
    pub const fn con_limite(mut self, limit: usize) -> Self {
        self.limit = Some(limit);
        self
    }

    /// Il CRS da assumere quando la sorgente non lo dichiara.
    #[must_use]
    pub fn con_crs_assunto(mut self, crs: impl Into<String>) -> Self {
        self.assume_crs = Some(crs.into());
        self
    }

    /// Un'opzione comune ai due driver.
    #[must_use]
    pub fn con_opzione(mut self, chiave: impl Into<String>, valore: impl Into<String>) -> Self {
        self.options.insert(chiave.into(), valore.into());
        self
    }

    /// Un'opzione del driver di lettura.
    #[must_use]
    pub fn con_opzione_di_lettura(
        mut self,
        chiave: impl Into<String>,
        valore: impl Into<String>,
    ) -> Self {
        self.input_options.insert(chiave.into(), valore.into());
        self
    }

    /// Un'opzione del driver di scrittura. Richiede una destinazione.
    #[must_use]
    pub fn con_opzione_di_scrittura(
        mut self,
        chiave: impl Into<String>,
        valore: impl Into<String>,
    ) -> Self {
        self.output_options.insert(chiave.into(), valore.into());
        self
    }

    /// Chiede la durabilita' alla chiusura. Richiede una destinazione.
    #[must_use]
    pub const fn durevole(mut self) -> Self {
        self.durable = true;
        self
    }

    /// I tetti di risorsa della pipeline.
    #[must_use]
    pub const fn con_tetti(mut self, limits: PipelineLimits) -> Self {
        self.limits = limits;
        self
    }

    /// Il canale con cui annullare l'operazione dall'esterno.
    #[must_use]
    pub fn con_cancellazione(mut self, cancellation: CancellationToken) -> Self {
        self.cancellation = cancellation;
        self
    }
}

impl From<Richiesta> for Cli {
    fn from(richiesta: Richiesta) -> Self {
        // I posizionali sono la sola concessione al binding: le operazioni li
        // leggono in quell'ordine, e ricostruirli qui e' meno invasivo che
        // cambiare sei firme. Un consumatore non li vede.
        let mut positionals = Vec::new();
        if let Some(source) = &richiesta.source {
            positionals.push(source.display().to_string());
        }
        if let Some(destination) = &richiesta.destination {
            positionals.push(destination.display().to_string());
        }
        Self {
            positionals,
            // `read` distingue le due forme sul flag, non sul posizionale: la
            // destinazione va in entrambi i posti perche' `convert` e `write`
            // la leggono come posizionale e `read` come `--output`, e quale
            // delle due sia lo decide l'operazione invocata.
            output: richiesta.destination,
            assume_crs: richiesta.assume_crs,
            to: richiesta.target_format,
            from_: richiesta.source_format,
            layer: richiesta.layer,
            limit: richiesta.limit,
            durable: richiesta.durable,
            opts: richiesta.options,
            in_opts: richiesta.input_options,
            out_opts: richiesta.output_options,
            limits: richiesta.limits,
            cancellazione: richiesta.cancellation,
        }
    }
}

/// L'esito di un'operazione: il documento del contratto d'uscita, o la busta
/// d'errore del contratto comune.
pub type Esito = Result<Value, Value>;

fn senza_uscita(esito: crate::CliResult) -> Esito {
    // Il codice d'uscita e' del processo: qui si scarta, e il binario lo
    // riderivera' dalla categoria con `uscita_della_categoria`. Tenerlo
    // vorrebbe dire far ragionare una libreria su un numero che riguarda un
    // processo che non ha avviato.
    esito.map_err(|(_uscita, documento)| documento)
}

/// `io.catalog` — i formati e le capacità osservabili dell'artefatto.
///
/// Non prende ingresso: il contratto `plenora-io-catalog-query-v1` è vuoto,
/// perché l'operazione descrive l'artefatto che sta rispondendo e non c'è
/// niente da chiedere.
#[must_use]
pub fn catalog() -> Value {
    crate::cmd_catalog()
}

/// `io.inspect` — formato dichiarato, layer, schemi e metadati di fedeltà.
///
/// # Errors
///
/// `Err` porta la busta d'errore del contratto comune `plenora-error-v1`: i
/// quattro assi, il codice, la fase, l'effetto remoto e la politica di
/// ritentare. Non c'e' un codice d'uscita, che e' del processo: lo proietta il
/// binding con `uscita_della_categoria`.
pub fn inspect(richiesta: Richiesta) -> Esito {
    senza_uscita(crate::cmd_inspect(&richiesta.into()))
}

/// `io.layers` — i layer indirizzabili, riassunti.
///
/// # Errors
///
/// `Err` porta la busta d'errore del contratto comune `plenora-error-v1`: i
/// quattro assi, il codice, la fase, l'effetto remoto e la politica di
/// ritentare. Non c'e' un codice d'uscita, che e' del processo: lo proietta il
/// binding con `uscita_della_categoria`.
pub fn layers(richiesta: Richiesta) -> Esito {
    senza_uscita(crate::cmd_layers(&richiesta.into()))
}

/// `io.read` — legge un layer e, con `destination`, ne consegna il dataset
/// Arrow.
///
/// # Errors
///
/// `Err` porta la busta d'errore del contratto comune `plenora-error-v1`: i
/// quattro assi, il codice, la fase, l'effetto remoto e la politica di
/// ritentare. Non c'e' un codice d'uscita, che e' del processo: lo proietta il
/// binding con `uscita_della_categoria`.
pub fn read(richiesta: Richiesta) -> Esito {
    senza_uscita(crate::cmd_read(&richiesta.into()))
}

/// `io.write` — pubblica un dataset Arrow nel formato nominato da
/// `target_format`.
///
/// # Errors
///
/// `Err` porta la busta d'errore del contratto comune `plenora-error-v1`: i
/// quattro assi, il codice, la fase, l'effetto remoto e la politica di
/// ritentare. Non c'e' un codice d'uscita, che e' del processo: lo proietta il
/// binding con `uscita_della_categoria`.
pub fn write(richiesta: Richiesta) -> Esito {
    senza_uscita(crate::cmd_write(&richiesta.into()))
}

/// `io.convert` — converte fra i due formati nominati da `source_format` e
/// `target_format`.
///
/// # Errors
///
/// `Err` porta la busta d'errore del contratto comune `plenora-error-v1`: i
/// quattro assi, il codice, la fase, l'effetto remoto e la politica di
/// ritentare. Non c'e' un codice d'uscita, che e' del processo: lo proietta il
/// binding con `uscita_della_categoria`.
pub fn convert(richiesta: Richiesta) -> Esito {
    senza_uscita(crate::cmd_convert(&richiesta.into()))
}

/// Il documento capability dell'artefatto: quali operazioni espone, e quali no.
#[must_use]
pub fn capabilities() -> Value {
    crate::capabilities_document()
}
