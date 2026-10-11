//! Le prove di [`super`]: l'orologio manuale si muove solo quando lo si muove.

use super::*;

#[test]
fn un_orologio_manuale_e_fermo_finche_non_avanza() {
    let inizio = Instant::now();
    let orologio = PipelineClock::manual(inizio);
    assert!(orologio.is_manual());
    assert_eq!(orologio.now(), inizio);
    orologio
        .advance(Duration::from_millis(7))
        .expect("avanzamento rappresentabile");
    assert_eq!(orologio.now(), inizio + Duration::from_millis(7));
}

#[test]
fn le_copie_condividono_lo_stesso_orologio() {
    let inizio = Instant::now();
    let orologio = PipelineClock::manual(inizio);
    let copia = orologio.clone();
    copia.advance(Duration::from_secs(1)).expect("avanzamento");
    assert_eq!(orologio.now(), inizio + Duration::from_secs(1));
}

#[test]
fn un_avanzamento_oltre_instant_e_un_errore_e_l_orologio_resta_fermo() {
    let inizio = Instant::now();
    let orologio = PipelineClock::manual(inizio);
    assert!(orologio.advance(Duration::MAX).is_err());
    assert_eq!(orologio.now(), inizio);
}

/// L'orologio di sistema non ha stato: `advance` non ha dove scrivere.
///
/// La prova guarda lo **stato**, non il tempo. Era `now() < prima + 1 h`, cioe'
/// una scommessa sul fatto che il test durasse meno di un'ora: vera in pratica,
/// ma dipendente dal tempo reale (revisione di #52). Ora si verifica che dopo
/// un avanzamento, anche di `Duration::MAX`, l'orologio resti senza stato
/// manuale e una sua copia pure; l'unica lettura del tempo reale rimasta e' la
/// monotonia, che `Instant` garantisce.
#[test]
fn l_orologio_di_sistema_ignora_l_avanzamento() {
    let orologio = PipelineClock::system();
    let copia = orologio.clone();
    assert!(!orologio.is_manual());
    let prima = orologio.now();
    for passo in [Duration::from_secs(3600), Duration::MAX] {
        orologio
            .advance(passo)
            .expect("nessun effetto, nessun errore");
    }
    assert!(
        orologio.manuale.is_none(),
        "nessuno stato in cui l'avanzamento resti"
    );
    assert!(copia.manuale.is_none());
    assert!(orologio.now() >= prima, "Instant e' monotono");
}
