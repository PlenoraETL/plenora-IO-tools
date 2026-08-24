#![no_main]
//! Coverage-guided sul **reader Shapefile reale**: apertura del dataset,
//! header `.shp`, tabella `.dbf`, `.prj`, inferenza dello schema e drenaggio
//! delle geometrie.
//!
//! # Perche' `shp_wkb` non bastava
//!
//! `shp_wkb` converte fra WKB e forme ESRI in memoria. E' una superficie utile
//! e non e' il parsing del formato: non apre un file, non legge un header, non
//! interpreta una tabella DBF e non attraversa l'entry point del driver.
//! Presentarlo come copertura di `.shp`/`.dbf` sarebbe falso, ed e' la ragione
//! per cui `fuzz.reader-shapefile` e' rimasto un blocco mentre `shp_wkb`
//! esisteva.
//!
//! # Perche' qui non c'e' quasi niente
//!
//! La divisione del bundle e la sua materializzazione vivono in `driver-shp`,
//! accanto al codice che esercitano. Non e' una questione di stile: li' sono
//! **provabili**, e le sonde del driver le chiamano sui semi committati per
//! verificare che raggiungano davvero il parsing. Una build che compila e un
//! replay senza crash non lo dimostrano.
//!
//! Il target resta quindi la sola dichiarazione della superficie coperta, come
//! `shp_wkb` accanto a lui.
//!
//! # Che cosa `Err` significa qui
//!
//! Un rifiuto tipizzato e' l'esito atteso sulla quasi totalita' degli input: un
//! blob casuale non e' uno Shapefile, e gli errori d'ambiente — directory non
//! creabile, scrittura fallita — sono anch'essi `Err` e non panici. Il finding
//! e' il **panico** dentro il parsing, o un output parziale consegnato prima di
//! un errore terminale.
use libfuzzer_sys::fuzz_target;

mod harness;

fuzz_target!(|data: &[u8]| {
    // `assume_crs` e' dichiarato perche' senza `.prj` l'apertura fallirebbe sul
    // CRS prima di leggere un solo record, e il target coprirebbe la
    // risoluzione del CRS invece del parsing. Con un `.prj` presente il driver
    // lo legge comunque: le due strade restano entrambe raggiungibili.
    let _ = driver_shp::__fuzz_leggi_bundle(
        data,
        harness::read_options().with_assume_crs("EPSG:4326"),
    );
});
