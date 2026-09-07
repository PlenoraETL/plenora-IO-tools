//! I due riproduttori delle correzioni difensive proposte a `shapefile-rs`.
//!
//! Da copiare in `examples/riproduttore.rs` di un albero del crate ed eseguire
//! con `cargo run --example riproduttore`. Costruisce i file in memoria: non
//! serve nessun dato su disco.
//!
//! Senza le patch **entrambi** i file vengono accettati, ed e' il difetto.
//! Con le patch entrambi sono rifiutati con `InvalidShapeRecordSize`.

use std::io::Cursor;

const SHP_HEADER_SIZE: usize = 100;

/// Un `.shp` con l'intestazione minima e un solo blocco di record.
fn shp_con_record(tipo_dichiarato: i32, record: &[u8]) -> Vec<u8> {
    let mut file = vec![0_u8; SHP_HEADER_SIZE];
    file[0..4].copy_from_slice(&9994_i32.to_be_bytes());
    // La lunghezza del file si dichiara in parole da sedici bit, non in byte.
    let parole = i32::try_from((SHP_HEADER_SIZE + record.len()) / 2).unwrap();
    file[24..28].copy_from_slice(&parole.to_be_bytes());
    file[28..32].copy_from_slice(&1000_i32.to_le_bytes());
    file[32..36].copy_from_slice(&tipo_dichiarato.to_le_bytes());
    for (indice, valore) in [0.0_f64, 0.0, 10.0, 10.0].into_iter().enumerate() {
        let inizio = 36 + indice * 8;
        file[inizio..inizio + 8].copy_from_slice(&valore.to_le_bytes());
    }
    file.extend_from_slice(record);
    file
}

/// Una testa di record: numero e lunghezza del contenuto, in parole.
fn testa_di_record(numero: i32, parole: i32) -> Vec<u8> {
    let mut testa = numero.to_be_bytes().to_vec();
    testa.extend_from_slice(&parole.to_be_bytes());
    testa
}

/// 1. Un record che dichiara una lunghezza **negativa**.
///
/// `checked_mul(2)` non lo ferma: `-1 * 2` non trabocca. La lunghezza prosegue
/// come `-2`, e `ShapeIterator::next` avanza poi di `record_size as usize * 2`,
/// un cast che di un negativo fa una posizione enorme.
fn lunghezza_negativa() -> Vec<u8> {
    let mut record = testa_di_record(1, -1);
    record.extend_from_slice(&0_i32.to_le_bytes());
    shp_con_record(1, &record)
}

/// 2. Un record **nullo** che dichiara piu' del proprio tag.
///
/// Il residuo e' costruito perche', letto come una testa di record, sia
/// **valido**: cosi' senza la patch la lettura non panica e non sbaglia
/// vistosamente -- termina con `Ok`, avendo letto meta' file e consegnandolo
/// come intero. E' la forma piu' pericolosa del difetto.
fn nullo_sovradimensionato() -> Vec<u8> {
    // Otto parole dichiarate, sedici byte di contenuto: il tag nullo, poi una
    // testa ben formata e il tag che quella testa promette.
    let mut record = testa_di_record(1, 8);
    record.extend_from_slice(&0_i32.to_le_bytes());
    record.extend_from_slice(&testa_di_record(2, 2));
    record.extend_from_slice(&0_i32.to_le_bytes());
    shp_con_record(1, &record)
}

/// La controprova positiva: un nullo **conforme** seguito da un punto.
///
/// Senza questa riga, «rifiuta il nullo sovradimensionato» sarebbe vero anche
/// di un reader che avesse smesso del tutto di leggere i record nulli.
fn nullo_conforme_e_un_punto() -> Vec<u8> {
    let mut record = testa_di_record(1, 2);
    record.extend_from_slice(&0_i32.to_le_bytes());
    record.extend_from_slice(&testa_di_record(2, 10));
    record.extend_from_slice(&1_i32.to_le_bytes());
    record.extend_from_slice(&3.0_f64.to_le_bytes());
    record.extend_from_slice(&4.0_f64.to_le_bytes());
    shp_con_record(1, &record)
}

fn prova(nome: &str, byte: Vec<u8>, atteso: &str) {
    let esito = shapefile::ShapeReader::new(Cursor::new(byte)).and_then(|r| r.read());
    let detto = match esito {
        Ok(forme) => format!("ACCETTATO, {} forme", forme.len()),
        Err(e) => format!("rifiutato: {e:?}"),
    };
    println!("{nome:38} {detto}   [atteso: {atteso}]");
}

fn main() {
    prova("lunghezza negativa", lunghezza_negativa(), "rifiutato");
    prova("nullo sovradimensionato", nullo_sovradimensionato(), "rifiutato");
    prova("nullo conforme + punto", nullo_conforme_e_un_punto(), "ACCETTATO, 2 forme");
    prova_nesima("negativa via read_nth_shape_as", "rifiutato");
}

// ---------------------------------------------------------------------------
// 3. Lo stesso record con lunghezza negativa, ma per la via che NON e' protetta.
//
// `ShapeIterator::next` fa `(hdr.record_size as usize).checked_mul(2)`: su un
// negativo il cast da' un numero enorme e la moltiplicazione trabocca, quindi
// quella via rifiuta gia' -- ed e' perche' il caso 1 qui sopra e' rifiutato
// anche senza patch.
//
// `ShapeReader::read_nth_shape_as` invece chiama `read_one_shape_as`
// direttamente, senza quella guardia. Li' la lunghezza negativa arriva a
// `Shape::read_from`, che le sottrae i quattro byte del tag e prosegue.

/// Un `.shx`: la stessa intestazione, e un record per forma (offset, lunghezza).
fn shx_per(offset_parole: i32, lunghezza_parole: i32) -> Vec<u8> {
    let mut file = vec![0_u8; SHP_HEADER_SIZE];
    file[0..4].copy_from_slice(&9994_i32.to_be_bytes());
    let parole = i32::try_from((SHP_HEADER_SIZE + 8) / 2).unwrap();
    file[24..28].copy_from_slice(&parole.to_be_bytes());
    file[28..32].copy_from_slice(&1000_i32.to_le_bytes());
    file[32..36].copy_from_slice(&1_i32.to_le_bytes());
    for (indice, valore) in [0.0_f64, 0.0, 10.0, 10.0].into_iter().enumerate() {
        let inizio = 36 + indice * 8;
        file[inizio..inizio + 8].copy_from_slice(&valore.to_le_bytes());
    }
    file.extend_from_slice(&offset_parole.to_be_bytes());
    file.extend_from_slice(&lunghezza_parole.to_be_bytes());
    file
}

fn prova_nesima(nome: &str, atteso: &str) {
    let mut record = testa_di_record(1, -1);
    record.extend_from_slice(&0_i32.to_le_bytes());
    let shp = shp_con_record(1, &record);
    // Il record comincia subito dopo l'intestazione: parola 50.
    let shx = shx_per(50, 2);
    let esito = shapefile::ShapeReader::with_shx(Cursor::new(shp), Cursor::new(shx))
        .and_then(|mut r| {
            r.read_nth_shape_as::<shapefile::Shape>(0)
                .unwrap_or(Err(shapefile::Error::MissingIndexFile))
        });
    let detto = match esito {
        Ok(forma) => format!("ACCETTATO: {}", forma.shapetype()),
        Err(e) => format!("rifiutato: {e:?}"),
    };
    println!("{nome:38} {detto}   [atteso: {atteso}]");
}
