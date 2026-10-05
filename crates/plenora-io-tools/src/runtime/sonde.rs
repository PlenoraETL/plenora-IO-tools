//! Le sonde delle parti private della superficie runtime: la composizione
//! dei tetti, la scadenza, il nome dell'artefatto risolto, gli spazi ECMA.

use std::path::PathBuf;
use std::time::{Duration, Instant, UNIX_EPOCH};

use serde_json::json;

use super::{
    istante_della_scadenza, millisecondi_rimasti, nome_coerente, spazio_ecma, tetti, Campi,
};
use plenora_io_model::budget::PipelineLimits;

fn campi(valore: serde_json::Value) -> Campi {
    serde_json::from_value(valore).expect("i campi si leggono")
}

/// Una scadenza sola governa la durata: l'assoluta (con il margine che la
/// lascia al token), oppure `deadline_ms`, oppure il default. Le due insieme
/// non arrivano qui: le rifiuta l'ammissione.
#[test]
fn la_durata_segue_una_scadenza_sola() {
    let predefinita = PipelineLimits::default().duration_ms();
    assert_eq!(tetti(&campi(json!({})), None).duration_ms(), predefinita);
    assert_eq!(
        tetti(&campi(json!({})), Some(1_000)).duration_ms(),
        1_000 + super::MARGINE_DELLA_DURATA_MS
    );
    assert_eq!(
        tetti(&campi(json!({})), Some(predefinita + 5_000)).duration_ms(),
        predefinita + 5_000 + super::MARGINE_DELLA_DURATA_MS,
        "la scadenza assoluta non e' tagliata dal default"
    );
    assert_eq!(
        tetti(&campi(json!({"deadline_ms": 90_000})), None).duration_ms(),
        90_000
    );
    assert_eq!(
        tetti(&campi(json!({})), Some(u64::MAX)).duration_ms(),
        u64::MAX,
        "nessun trabocco"
    );
}

/// Ogni quota del payload finisce nel campo omonimo, e in nessun altro.
#[test]
fn ogni_budget_va_nel_suo_campo() {
    let limiti = tetti(
        &campi(json!({"budgets": {
            "memory_bytes": 101,
            "max_rows": 102,
            "max_columns": 103,
            "max_input_bytes": 104,
            "max_input_entries": 105,
            "max_output_bytes": 106,
            "max_vertices": 107,
            "max_wkb_cell_bytes": 108,
            "max_wkb_components": 109,
            "max_wkb_depth": 110,
        }})),
        None,
    );
    assert_eq!(limiti.memory_bytes(), 101);
    assert_eq!(limiti.max_rows(), 102);
    assert_eq!(limiti.max_columns(), 103);
    assert_eq!(limiti.max_input_bytes(), 104);
    assert_eq!(limiti.max_input_entries(), 105);
    assert_eq!(limiti.max_output_bytes(), 106);
    assert_eq!(limiti.max_vertices(), 107);
    assert_eq!(limiti.max_wkb_cell_bytes(), 108);
    assert_eq!(limiti.max_wkb_components(), 109);
    assert_eq!(limiti.max_wkb_depth(), 110);
    // Una quota assente resta quella del modello.
    let soli = tetti(&campi(json!({"budgets": {"max_rows": 7}})), None);
    let predefiniti = PipelineLimits::default();
    assert_eq!(soli.max_rows(), 7);
    assert_eq!(soli.memory_bytes(), predefiniti.memory_bytes());
    assert_eq!(soli.max_columns(), predefiniti.max_columns());
}

#[test]
fn i_millisecondi_rimasti_arrotondano_per_difetto() {
    assert!(millisecondi_rimasti(Instant::now()).is_err());
    let lontano = Instant::now() + Duration::from_secs(60);
    let rimasti = millisecondi_rimasti(lontano).expect("ne restano");
    assert!(rimasti <= 60_000 && rimasti > 59_000, "{rimasti}");
}

#[test]
fn l_orologio_prima_dell_epoca_non_allenta_la_scadenza() {
    let prima = UNIX_EPOCH - Duration::from_secs(1);
    let errore = istante_della_scadenza("2030-01-01T00:00:00Z", prima)
        .expect_err("un orologio prima dell'epoca non si legge");
    assert_eq!(errore["code"], "RUNTIME_CLOCK_INVALID");
}

#[test]
fn una_scadenza_passata_e_quella_della_pipeline() {
    let adesso = UNIX_EPOCH + Duration::from_hours(497_544);
    for passata in [
        "2026-10-04T23:59:59Z",
        "2026-10-05T00:00:00Z",
        "1969-12-31T23:59:59Z",
    ] {
        let errore = istante_della_scadenza(passata, adesso).expect_err(passata);
        assert_eq!(errore["code"], "DEADLINE_EXCEEDED", "{passata}");
        assert_eq!(errore["category"], "timeout", "{passata}");
        assert_eq!(errore["phase"], "validate", "{passata}");
        assert_eq!(errore["retry"]["kind"], "never", "{passata}");
    }
    assert!(istante_della_scadenza("2026-10-05T00:00:01Z", adesso).is_ok());
    let invalida = istante_della_scadenza("1969-02-30T00:00:00Z", adesso).expect_err("data");
    assert_eq!(invalida["code"], "RUNTIME_DEADLINE_INVALID");
}

#[test]
fn il_nome_risolto_e_quello_del_riferimento() {
    let base = PathBuf::from("deposito");
    let casi = [
        ("artifact://input/strade", "strade.geojson", true),
        ("artifact://input/strade.geojson", "strade.geojson", true),
        ("s3://bucket/dati/strade.gpkg", "strade.gpkg", true),
        ("urn:plenora:strade", "strade.csv", true),
        ("artifact://input/strade.shp.d", "strade.shp.d", true),
        ("artifact://input/strade", "tmp-81f3a2.geojson", false),
        ("artifact://input/strade", "Strade.geojson", false),
        ("artifact://input/", "strade.geojson", false),
    ];
    for (riferimento, file, atteso) in casi {
        let esito = nome_coerente(riferimento, base.join(file));
        assert_eq!(esito.is_ok(), atteso, "{riferimento} -> {file}");
        if let Err(errore) = esito {
            assert_eq!(errore["code"], "RUNTIME_ARTIFACT_NAME_MISMATCH");
            assert!(
                !errore.to_string().contains(file),
                "il nome non entra nel messaggio"
            );
        }
    }
}

#[test]
fn gli_spazi_sono_quelli_di_ecma() {
    for spazio in [
        '\u{0020}', '\u{0009}', '\u{00A0}', '\u{2007}', '\u{FEFF}', '\u{3000}',
    ] {
        assert!(spazio_ecma(spazio), "{spazio:?}");
    }
    // U+0085 e' `White_Space` per Unicode ma non `\s` per ECMA-262.
    for non_spazio in ['\u{0085}', 'a', '/', '\u{200B}'] {
        assert!(!spazio_ecma(non_spazio), "{non_spazio:?}");
    }
}
