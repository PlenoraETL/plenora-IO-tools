#![no_main]
use libfuzzer_sys::fuzz_target;
use plenora_io_core::{Source, ReadOptions, LimitiArchivio, elenca_archivio};
use plenora_io_model::budget::PipelineBudget;
fuzz_target!(|data: &[u8]| {
    let src = Source::Bytes { nome: "fuzz.zip".into(), dati: data.into() };
    let limits = LimitiArchivio { max_voci: 32, max_byte_decompressi: 262_144, max_rapporto: 64 };
    if let Ok(entries) = elenca_archivio(&src, &limits) {
        if let Ok(bundle) = PipelineBudget::builder().build() {
            let opts = ReadOptions::from_read_parts(bundle.into_read_parts());
            for entry in entries.into_iter().filter(|e| !e.directory) {
                let _ = plenora_io_core::archivio::leggi_voce(&src, &entry.nome, &limits, &opts);
            }
        }
    }
});
