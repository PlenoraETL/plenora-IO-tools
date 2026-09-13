//! Le prove di [`super`], in un file loro.
//!
//! Restano un **modulo figlio**: vedono i privati del genitore
//! esattamente come quando stavano dentro di lui, e non allargano di
//! una riga la superficie pubblica del crate.

use super::*;
use std::collections::BTreeMap;

fn artefatto_completo(radice: &Path) {
    for r in RADICI {
        std::fs::create_dir_all(radice.join(r.relativo)).unwrap();
    }
    std::fs::create_dir_all(radice.join("bin")).unwrap();
}

fn mappa(piano: Vec<(&'static str, OsString)>) -> BTreeMap<String, String> {
    piano
        .into_iter()
        .map(|(k, v)| (k.to_owned(), v.to_string_lossy().into_owned()))
        .collect()
}

/// Ogni variabile della tabella punta dentro l'artefatto, e il catalogo XML
/// e' vuoto.
#[test]
fn ogni_radice_punta_dentro_l_artefatto() {
    let dir = tempfile::tempdir().unwrap();
    artefatto_completo(dir.path());
    let p = mappa(piano(dir.path()));

    assert_eq!(p.len(), RADICI.len() + 1, "manca una riga della tabella");
    for r in RADICI {
        let valore = p.get(r.variabile).expect(r.variabile);
        assert!(
            Path::new(valore).starts_with(dir.path()),
            "«{}» punta fuori dall'artefatto: {valore}",
            r.variabile
        );
    }
    assert_eq!(p.get(CATALOGO_XML).map(String::as_str), Some(""));
}

/// I due nomi di PROJ puntano alla stessa directory.
///
/// Non sono due cose: sono lo stesso dato con due nomi, perche' PROJ ha
/// cambiato quello che legge fra la 9.0 e la 9.1.
#[test]
fn i_due_nomi_di_proj_puntano_allo_stesso_posto() {
    let dir = tempfile::tempdir().unwrap();
    artefatto_completo(dir.path());
    let p = mappa(piano(dir.path()));
    assert_eq!(p.get("PROJ_DATA"), p.get("PROJ_LIB"));
}

/// Una variabile la cui directory non e' spedita non si imposta.
#[test]
fn cio_che_non_e_spedito_non_si_dichiara() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("share").join("gdal")).unwrap();
    let p = mappa(piano(dir.path()));

    assert!(p.contains_key("GDAL_DATA"));
    assert!(!p.contains_key("PROJ_DATA"), "share/proj non e' spedito");
    assert!(
        !p.contains_key("GDAL_DRIVER_PATH"),
        "lib/gdalplugins non e' spedito"
    );
    assert_eq!(p.get(CATALOGO_XML).map(String::as_str), Some(""));
}

/// Il layout completo e' riconosciuto.
#[test]
fn un_artefatto_completo_e_riconosciuto() {
    let dir = tempfile::tempdir().unwrap();
    artefatto_completo(dir.path());
    let binario = dir.path().join("bin").join("plenora-io");
    std::fs::write(&binario, b"").unwrap();
    assert_eq!(radice_da(&binario).as_deref(), Some(dir.path()));
}

/// Il layout di Windows: le DLL stanno in `bin/`, e `lib/` non c'e'.
///
/// E' il difetto che il relocation smoke ha trovato. Il criterio pretendeva
/// `lib/`, l'artefatto Windows non l'aveva, e il binario non riconosceva il
/// proprio layout.
#[test]
fn il_layout_di_windows_e_riconosciuto() {
    let dir = tempfile::tempdir().unwrap();
    let radice = dir.path();
    for percorso in ["bin", "share/gdal", "share/proj"] {
        std::fs::create_dir_all(radice.join(percorso)).unwrap();
    }
    std::fs::write(radice.join("bin").join("gdal.dll"), b"").unwrap();
    let binario = radice.join("bin").join("plenora-io.exe");
    std::fs::write(&binario, b"").unwrap();

    assert_eq!(
        radice_da(&binario).as_deref(),
        Some(radice),
        "senza `lib/` il layout resta quello di un artefatto"
    );
}

/// Un albero a meta' non e' un artefatto.
#[test]
fn un_layout_incompleto_non_e_un_artefatto() {
    for mancante in ["share/gdal", "share/proj"] {
        let dir = tempfile::tempdir().unwrap();
        for percorso in ["bin", "share/gdal", "share/proj"] {
            if percorso != mancante {
                std::fs::create_dir_all(dir.path().join(percorso)).unwrap();
            }
        }
        let binario = dir.path().join("bin").join("plenora-io");
        std::fs::write(&binario, b"").unwrap();
        assert!(
            radice_da(&binario).is_none(),
            "senza «{mancante}» non e' un artefatto"
        );
    }
}

/// L'albero di sviluppo non e' un artefatto.
#[test]
fn un_binario_fuori_da_un_artefatto_non_da_radici() {
    let dir = tempfile::tempdir().unwrap();
    let binario = dir.path().join("target").join("debug").join("plenora-io");
    std::fs::create_dir_all(binario.parent().unwrap()).unwrap();
    std::fs::write(&binario, b"").unwrap();
    assert!(radice_da(&binario).is_none());
}

/// `RADICE_DI_PROJ` nomina una riga che nella tabella c'e' davvero.
///
/// E' la costante che chi applica il piano tratta a parte, perche' PROJ non
/// legge le config option di GDAL. Se la riga sparisse, l'applicazione
/// smetterebbe di indirizzare PROJ **in silenzio**: continuerebbe a
/// impostare tutto il resto, e il difetto tornerebbe a somigliare a un dato
/// rotto invece che a una configurazione mancante.
#[test]
fn la_radice_di_proj_e_una_riga_della_tabella() {
    let dir = tempfile::tempdir().unwrap();
    artefatto_completo(dir.path());
    let p = mappa(piano(dir.path()));
    let valore = p
        .get(RADICE_DI_PROJ)
        .expect("`RADICE_DI_PROJ` non nomina nessuna riga della tabella");
    assert!(Path::new(valore).ends_with("proj"));
}

/// Un percorso senza nonno non fa panicare niente.
#[test]
fn un_percorso_troppo_corto_non_da_radici() {
    assert!(radice_da(Path::new("plenora-io")).is_none());
}
