//! Riproduzione: il decoder del footer di `parquet` 59.3.0 prenota ogni elenco
//! Thrift dal conteggio che dichiara, senza legarlo ai byte del footer.
//!
//! `caso.parquet` e' l'input di 3966 byte trovato dallo smoke di
//! `geoparquet_reader` il 2026-10-08 (sha1
//! 0fe0b59475f1ce80fc5a2646bbea49f31939b073). Letto con l'API che decodifica
//! solo i metadati, arriva a
//!
//! ```text
//! parquet::parquet_thrift::read_thrift_vec   (parquet_thrift.rs:724)
//!     let mut res = Vec::with_capacity(list_ident.size as usize);
//! ```
//!
//! con circa 48 milioni di `KeyValue` dichiarati: un'allocazione da 2315255472
//! byte. Atteso: un `Err` del lettore. Osservato: il processo termina per
//! esaurimento di memoria, oppure, con memoria sufficiente, la prenotazione
//! riesce e l'errore arriva dopo.
//!
//! `cargo run --release` da questa cartella. Sotto un tetto di memoria
//! (`ulimit -v 1048576` su Linux) l'esito e' un abort.
//!
//! parquet 60.0.0 limita gli elenchi letti con `read_thrift_vec` ai byte
//! residui; la prenotazione dei row group (`file/metadata/thrift/mod.rs`, in
//! `parquet_metadata_from_bytes`) resta senza quel limite anche li'. E' la
//! correzione pianificata per la 4.2.0 di plenora-IO-tools.

use parquet::file::reader::SerializedFileReader;

fn main() {
    let percorso = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("caso.parquet");
    let file = std::fs::File::open(&percorso).expect("caso.parquet presente accanto al manifesto");
    match SerializedFileReader::new(file) {
        Ok(_) => println!("letto senza errore: il caso non riproduce piu'"),
        Err(errore) => println!("errore del lettore, come atteso: {errore}"),
    }
}
