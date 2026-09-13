//! Le prove di [`super`], in un file loro.
//!
//! Restano un **modulo figlio**: vedono i privati del genitore
//! esattamente come quando stavano dentro di lui, e non allargano di
//! una riga la superficie pubblica del crate.

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
        let errore =
            valida_flusso(&[intestazione, 0x00], 1, 8).expect_err("un run vuoto non copre niente");
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
