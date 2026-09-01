//! PROJ non legge le config option di GDAL, e questa prova lo dimostra.
//!
//! # Perche' esiste
//!
//! La prima cura al difetto del relocation smoke Windows applicava le radici
//! dell'artefatto come **config option** di GDAL, per tutte e quattro. Sembrava
//! ragionevole: GDAL le consulta prima dell'ambiente. Per `GDAL_DATA` e' vero.
//! Per PROJ no -- GDAL non gli inoltra quella tabella -- e la corsa e' tornata
//! rossa con lo stesso identico messaggio, «Cannot find proj.db».
//!
//! Senza questa prova, la stessa regressione non si vedrebbe: nel container e
//! sul runner i dati di PROJ si trovano lo stesso, dal percorso cotto dentro la
//! libreria, e ogni conversione con un CRS resterebbe verde. Il difetto si
//! manifesta **solo** dove quel percorso non c'e' -- cioe' sulla macchina di chi
//! installa, che e' esattamente dove non lo vogliamo scoprire.
//!
//! # Perche' un binario di test tutto suo
//!
//! Avvelena l'ambiente del processo, che e' globale. Dentro il binario di test
//! della crate correrebbe insieme a tutto il resto, e la prova si porterebbe
//! dietro gli altri. Qui l'unico `#[test]` del file e' questo.

#![cfg(feature = "gdal-backend")]

use gdal::spatial_ref::{get_proj_search_paths, set_proj_search_paths, SpatialRef};

/// Avvelenare l'ambiente rompe PROJ, e `OSRSetPROJSearchPaths` lo ripara --
/// mentre la config option non lo ripara affatto.
#[test]
fn la_config_option_non_arriva_a_proj_e_l_api_dei_percorsi_si() {
    // I percorsi effettivi **prima** di toccare qualunque cosa: con nulla di
    // configurato GDAL riporta quelli con cui PROJ e' stato costruito, ed e' il
    // modo di sapere dove stanno i dati senza scriverne uno a mano.
    let veri = get_proj_search_paths();
    assert!(
        !veri.is_empty(),
        "GDAL non riporta nessun percorso di ricerca per PROJ: senza, questa \
         prova non saprebbe dove sono i dati e non proverebbe niente"
    );

    // L'ambiente avvelenato: su Linux `PROJ_DATA` **sostituisce** il default, e
    // il CRS smette di risolversi. E' la condizione che il relocation smoke crea
    // su Windows, riprodotta qui dove si puo' provare a ogni commit.
    let inesistente = "/percorso/che/non/esiste/mai/proj";
    std::env::set_var("PROJ_DATA", inesistente);
    std::env::set_var("PROJ_LIB", inesistente);

    assert!(
        SpatialRef::from_definition("EPSG:4326").is_err(),
        "con `PROJ_DATA` avvelenata il CRS si risolve lo stesso: la prova non \
         sta misurando quello che dice, e va rivista prima di fidarsene"
    );

    // La via che non funziona, provata perche' e' quella che ci ha ingannati.
    gdal::config::set_config_option("PROJ_DATA", &veri[veri.len() - 1]).unwrap();
    gdal::config::set_config_option("PROJ_LIB", &veri[veri.len() - 1]).unwrap();
    assert!(
        SpatialRef::from_definition("EPSG:4326").is_err(),
        "la config option `PROJ_DATA` ora arriva a PROJ: se GDAL ha cambiato \
         comportamento la cura si semplifica, ma va deciso -- non ereditato"
    );

    // La via che funziona.
    let percorsi: Vec<&str> = veri.iter().map(String::as_str).collect();
    set_proj_search_paths(&percorsi).unwrap();
    let srs = SpatialRef::from_definition("EPSG:4326")
        .expect("`OSRSetPROJSearchPaths` non ha riportato PROJ ai propri dati");
    assert_eq!(srs.auth_code().ok(), Some(4326));
}
