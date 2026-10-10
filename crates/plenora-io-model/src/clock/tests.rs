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

#[test]
fn l_orologio_di_sistema_ignora_l_avanzamento() {
    let orologio = PipelineClock::system();
    assert!(!orologio.is_manual());
    let prima = orologio.now();
    orologio
        .advance(Duration::from_secs(3600))
        .expect("nessun effetto");
    assert!(orologio.now() < prima + Duration::from_secs(3600));
}
