#![no_main]
//! Coverage-guided sul deserializer GeoJSON completo (pass-1 + pass-2 sincrono):
//! nessun panic né disallineamento colonne su input arbitrario.
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    // Err = JSON invalido (rifiuto legittimo). Un panic (es. colonne
    // disallineate) fa crashare libFuzzer → finding.
    let _ = driver_geojson::__fuzz_read_geojson(data);
});
