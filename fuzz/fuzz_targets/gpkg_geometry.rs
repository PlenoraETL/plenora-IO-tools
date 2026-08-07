#![no_main]
//! Coverage-guided sull'header binario GeoPackage e sul WKB che lo segue: il
//! blob geometria è l'unico campo di un `.gpkg` che il driver interpreta byte
//! per byte invece di delegarlo a SQLite.
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    // Err = magic, flag envelope o WKB non validi: rifiuto legittimo.
    let Ok(offset) = driver_gpkg::__fuzz_gpkg_geometry(data) else {
        return;
    };
    // L'header è 8 byte più un envelope di dimensione fissa dichiarata nei
    // flag (0, 32, 48 o 64). Qualunque altro offset significa che il calcolo
    // dello scostamento ha ceduto su input ostile, non che l'input è invalido.
    assert!(
        matches!(offset, 8 | 40 | 56 | 72),
        "offset del payload GeoPackage fuori dai valori dichiarabili: {offset}"
    );
});
