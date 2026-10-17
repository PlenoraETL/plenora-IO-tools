//! L'orologio da cui la pipeline legge il tempo.
//!
//! # Perche' un tipo, e non `Instant::now()`
//!
//! La scadenza di una pipeline (`--deadline-ms`) si calcola da un istante di
//! partenza e si verifica a ogni punto di controllo. Con `Instant::now()`
//! scritto nei due posti, l'unico modo di provare «la scadenza ferma il
//! replay» era una scadenza di un millisecondo e una `sleep`: una prova che
//! dipende da quanto il passo precedente ci mette. Sotto strumentazione
//! (`coverage`) la scadenza arrivava gia' durante la `push` e la prova falliva
//! nel punto sbagliato.
//!
//! Qui il tempo si legge da un [`PipelineClock`]. In produzione e' quello di
//! sistema, e nient'altro cambia; una prova ne costruisce uno manuale, lo fa
//! avanzare quando vuole, e la scadenza scatta **esattamente** li'.
//!
//! Anche [`crate::CancellationToken::with_clock`] accetta questo orologio:
//! i figli lo condividono e osservano la scadenza dopo la fine della pipeline.
//! I token costruiti con `new` o `with_deadline` usano il tempo di sistema.

use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use crate::error::{PlenoraIoError, PublicMessage};

const OROLOGIO_OLTRE_INSTANT: &str = "l'orologio manuale avanzerebbe oltre Instant";

/// Il tempo della pipeline: di sistema, o manuale nelle prove.
///
/// Clonarlo condivide lo stesso orologio: un orologio manuale avanzato da una
/// copia avanza per tutte.
#[derive(Clone, Debug, Default)]
pub struct PipelineClock {
    manuale: Option<Arc<Mutex<Instant>>>,
}

impl PipelineClock {
    /// L'orologio di sistema: `Instant::now()`.
    #[must_use]
    pub const fn system() -> Self {
        Self { manuale: None }
    }

    /// Un orologio fermo su `inizio`, che si muove solo con [`Self::advance`].
    #[must_use]
    pub fn manual(inizio: Instant) -> Self {
        Self {
            manuale: Some(Arc::new(Mutex::new(inizio))),
        }
    }

    /// L'istante corrente secondo questo orologio.
    #[must_use]
    pub fn now(&self) -> Instant {
        self.manuale
            .as_ref()
            .map_or_else(Instant::now, |istante| *leggi(istante))
    }

    /// Fa avanzare un orologio manuale; su quello di sistema non fa niente.
    ///
    /// # Errors
    ///
    /// Un avanzamento che porterebbe l'istante oltre cio' che `Instant`
    /// rappresenta e' un errore, e l'orologio resta dov'era.
    pub fn advance(&self, di: Duration) -> Result<(), PlenoraIoError> {
        let Some(istante) = &self.manuale else {
            return Ok(());
        };
        let mut corrente = leggi(istante);
        let nuovo = corrente.checked_add(di).ok_or_else(|| {
            PlenoraIoError::limite_redatto(&PublicMessage::Curated(OROLOGIO_OLTRE_INSTANT))
        })?;
        *corrente = nuovo;
        drop(corrente);
        Ok(())
    }

    /// Vero per un orologio manuale.
    #[must_use]
    pub const fn is_manual(&self) -> bool {
        self.manuale.is_some()
    }
}

/// Il lock dell'orologio manuale. Un `Instant` non ha stati intermedi: un
/// thread che e' andato in panico tenendo il lock non puo' averlo lasciato a
/// meta', quindi il valore dopo un avvelenamento e' ancora quello giusto.
fn leggi(istante: &Mutex<Instant>) -> std::sync::MutexGuard<'_, Instant> {
    match istante.lock() {
        Ok(guardia) => guardia,
        Err(avvelenato) => PoisonError::into_inner(avvelenato),
    }
}

#[cfg(test)]
mod tests;
