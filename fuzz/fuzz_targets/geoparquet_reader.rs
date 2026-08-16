#![no_main]
//! Coverage-guided sul reader GeoParquet: footer/metadati Parquet, busta JSON
//! `geo` (colonna geometria, CRS, tipi dichiarati, bbox covering), retag dello
//! schema Arrow e drenaggio dei row group.
use libfuzzer_sys::fuzz_target;

mod harness;

fuzz_target!(|data: &[u8]| {
    let Some(file) = harness::spill(data, "input.parquet") else {
        return;
    };
    // Err = Parquet invalido o metadati `geo` non conformi; un panic è un finding.
    let _ = harness::read_all(
        &driver_geoparquet::GeoParquetDriver,
        file.path().to_path_buf(),
        harness::read_options(),
    );
});
