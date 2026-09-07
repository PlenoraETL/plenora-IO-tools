//! Scrive un file Arrow IPC valido con un metadato nel footer.
//!
//! Serve a costruire i due casi che isolano `key` e `value`: si parte da cio'
//! che arrow scrive davvero, e se ne toglie un campo solo. Cosi' il resto del
//! file resta quello che arrow considera corretto, e la differenza fra il caso
//! valido e quello ostile e' esattamente lo slot azzerato.

use std::sync::Arc;

use arrow_array::{Int32Array, RecordBatch};
use arrow_ipc::writer::FileWriter;
use arrow_schema::{DataType, Field, Schema};

fn main() {
    let percorso = std::env::args().nth(1).expect("percorso di uscita");

    let schema = Arc::new(Schema::new(vec![Field::new("n", DataType::Int32, false)]));
    let batch = RecordBatch::try_new(
        Arc::clone(&schema),
        vec![Arc::new(Int32Array::from(vec![1, 2, 3]))],
    )
    .expect("il batch si costruisce");

    let file = std::fs::File::create(&percorso).expect("il file si crea");
    let mut writer = FileWriter::try_new(file, &schema).expect("il writer si apre");

    // Il metadato del footer: e' la voce su cui il caso ostile toglie un campo.
    writer.write_metadata("plenora.prova", "valore");

    writer.write(&batch).expect("il batch si scrive");
    writer.finish().expect("il file si chiude");
    println!("scritto {percorso}");
}
