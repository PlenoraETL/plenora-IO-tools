//! Scrive un file Arrow IPC con una colonna a **dizionario**.
//!
//! Serve a coprire il punto che l'aggiornamento ad arrow 59.3.0 ha toccato: in
//! `arrow-ipc/src/reader.rs` sono cambiate le attese sui dizionari, e un campo
//! che il commento dichiarava «technically not legal to be null» e' diventato
//! non nullable. Il prodotto non **scrive** dizionari -- i nostri tipi di uscita
//! sono `Binary`, `Utf8`, `Int64`, `Float64` -- ma li **legge**, e li attraversa
//! in prevalidazione (`prevalida_arrow`, `header_as_dictionary_batch`). Un file
//! di terze parti puo' benissimo portarli.
//!
//! Le chiavi sono `Int8` perche' e' esattamente il tipo su cui le attese di
//! arrow sono cambiate, e c'e' un valore nullo perche' e' il caso che quella
//! modifica riguarda.

use std::sync::Arc;

use arrow_array::builder::StringDictionaryBuilder;
use arrow_array::types::Int8Type;
use arrow_array::RecordBatch;
use arrow_ipc::writer::FileWriter;
use arrow_schema::{DataType, Field, Schema};

fn main() {
    let percorso = std::env::args().nth(1).expect("percorso di uscita");

    // Sei righe, tre valori distinti, ripetizioni e un nullo: il dizionario ha
    // qualcosa da fare, e il nullo cade dove le attese sono cambiate.
    let mut costruttore = StringDictionaryBuilder::<Int8Type>::new();
    costruttore.append_value("alfa");
    costruttore.append_value("beta");
    costruttore.append_value("alfa");
    costruttore.append_null();
    costruttore.append_value("gamma");
    costruttore.append_value("beta");
    let colonna = costruttore.finish();

    let schema = Arc::new(Schema::new(vec![Field::new(
        "etichetta",
        DataType::Dictionary(Box::new(DataType::Int8), Box::new(DataType::Utf8)),
        true,
    )]));
    let batch = RecordBatch::try_new(Arc::clone(&schema), vec![Arc::new(colonna)])
        .expect("il batch si costruisce");

    let file = std::fs::File::create(&percorso).expect("il file si crea");
    let mut writer = FileWriter::try_new(file, &schema).expect("il writer si apre");
    writer.write(&batch).expect("il batch si scrive");
    writer.finish().expect("il file si chiude");
    println!("scritto {percorso}");
}
