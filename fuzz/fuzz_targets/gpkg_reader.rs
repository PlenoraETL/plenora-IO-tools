#![no_main]
//! Coverage-guided sul reader GeoPackage completo: apertura del database,
//! interpretazione di `gpkg_contents`/`gpkg_geometry_columns`/`gpkg_spatial_ref_sys`,
//! costruzione dello schema Arrow dai tipi SQLite dichiarati e drenaggio a pagine.
//! È la superficie dove un file ostile controlla i *metadati*, non solo i dati.
use libfuzzer_sys::fuzz_target;

mod harness;

fuzz_target!(|data: &[u8]| {
    let Some(file) = harness::spill(data, "input.gpkg") else {
        return;
    };
    // Err = file non-SQLite, catalogo GeoPackage assente o incoerente, tipi non
    // mappabili; un panic è un finding.
    let _ = harness::read_all(
        &driver_gpkg::GpkgDriver,
        file.path().to_path_buf(),
        harness::read_options(),
    );
});
