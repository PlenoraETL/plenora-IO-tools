#![no_main]
//! Coverage-guided sul parser DXF e sul walker DXF→WKB XY/XYZ.
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    // Err = DXF invalido o oltre i limiti; un panic è un finding.
    let _ = driver_dxf::__fuzz_read_dxf(data);
});
