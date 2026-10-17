//! La stessa classe di descrittori ostili nei contenitori e nei flussi.
use driver_common::prevalida_arrow::{valida_file_ipc, valida_flusso_ipc};

fn seme(nome: &str) -> Vec<u8> {
    std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fuzz/seeds/ipc_reader")
            .join(nome),
    )
    .unwrap()
}

// I messaggi del file sono gia' incapsulati: togliamo magic e footer,
// conservando schema, dizionari e tutti i record batch nell'ordine originale.
fn flusso(file: &[u8]) -> Vec<u8> {
    let n = file.len();
    let footer = i32::from_le_bytes(file[n - 10..n - 6].try_into().unwrap());
    let inizio = file.windows(4).position(|b| b == [255; 4]).unwrap();
    file[inizio..n - 10 - usize::try_from(footer).unwrap()].to_vec()
}

fn descrittore(byte: &[u8], dizionario: bool, successivo: bool) -> usize {
    let mut posizione = if byte.starts_with(b"ARROW1") {
        byte.windows(4).position(|b| b == [255; 4]).unwrap()
    } else {
        0
    };
    let mut saltato = false;
    loop {
        let n = i32::from_le_bytes(byte[posizione + 4..posizione + 8].try_into().unwrap());
        assert!(n > 0, "messaggio richiesto assente");
        let inizio = posizione + 8;
        let fine = inizio + usize::try_from(n).unwrap();
        let messaggio = arrow_ipc::root_as_message(&byte[inizio..fine]).unwrap();
        let batch = if dizionario {
            messaggio
                .header_as_dictionary_batch()
                .and_then(|d| d.data())
        } else {
            messaggio.header_as_record_batch()
        };
        if let Some(batch) = batch {
            if !successivo || saltato {
                if dizionario && successivo {
                    assert!(messaggio.header_as_dictionary_batch().unwrap().isDelta());
                }
                let buffer = batch.buffers().unwrap().get(0);
                return buffer.0.as_ptr() as usize - byte.as_ptr() as usize;
            }
            saltato = true;
        }
        posizione = fine + usize::try_from(messaggio.bodyLength()).unwrap();
    }
}

#[test]
fn descrittori_ostili_in_file_flussi_e_dizionari() {
    for dizionario in [false, true] {
        let nome = "dizionario-int8.arrow";
        let originale = seme(nome);
        for stream in [false, true] {
            let byte = if stream {
                flusso(&originale)
            } else {
                originale.clone()
            };
            let valida = if stream {
                valida_flusso_ipc
            } else {
                valida_file_ipc
            };
            let file = tempfile::NamedTempFile::new().unwrap();
            std::fs::write(file.path(), &byte).unwrap();
            valida("arrow", file.path()).expect("il controllo positivo passa");
            let offset = descrittore(&byte, dizionario, false);
            for (inizio, lunghezza, motivo) in [
                (-1_i64, 0_i64, "offset di buffer Arrow negativo"),
                (0, -1, "lunghezza di buffer Arrow negativa"),
                (
                    1,
                    0,
                    "buffer Arrow non allineato agli 8 byte richiesti dal formato",
                ),
                (
                    i64::MAX,
                    i64::MAX,
                    "buffer Arrow oltre la fine del corpo del messaggio",
                ),
                (
                    i64::MAX,
                    0,
                    "buffer Arrow oltre la fine del corpo del messaggio",
                ),
            ] {
                let mut ostile = byte.clone();
                ostile[offset..offset + 8].copy_from_slice(&inizio.to_le_bytes());
                ostile[offset + 8..offset + 16].copy_from_slice(&lunghezza.to_le_bytes());
                std::fs::write(file.path(), ostile).unwrap();
                let errore = valida("arrow", file.path()).unwrap_err();
                assert_eq!(errore.code, plenora_io_model::IoErrorCode::Format);
                assert!(
                    errore.message.contains(motivo),
                    "{nome}, stream={stream}: {errore}"
                );
            }
        }
    }
}

#[test]
fn anche_i_delta_del_dizionario_vengono_controllati() {
    use arrow_array::{builder::StringDictionaryBuilder, types::Int8Type, RecordBatch};
    use arrow_ipc::writer::{DictionaryHandling, IpcWriteOptions, StreamWriter};
    use std::sync::Arc;

    let mut builder = StringDictionaryBuilder::<Int8Type>::new();
    builder.append_value("primo");
    let batch = RecordBatch::try_from_iter(vec![(
        "valori",
        Arc::new(builder.finish_preserve_values()) as arrow_array::ArrayRef,
    )])
    .unwrap();
    let schema = batch.schema();
    let mut byte = Vec::new();
    let opzioni = IpcWriteOptions::default().with_dictionary_handling(DictionaryHandling::Delta);
    let mut writer = StreamWriter::try_new_with_options(&mut byte, &schema, opzioni).unwrap();
    writer.write(&batch).unwrap();
    builder.append_value("secondo");
    let batch =
        RecordBatch::try_new(schema, vec![Arc::new(builder.finish_preserve_values())]).unwrap();
    writer.write(&batch).unwrap();
    writer.finish().unwrap();
    drop(writer);
    let file = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(file.path(), &byte).unwrap();
    valida_flusso_ipc("arrow", file.path()).expect("delta conforme");
    let offset = descrittore(&byte, true, true);
    byte[offset..offset + 8].copy_from_slice(&i64::MAX.to_le_bytes());
    std::fs::write(file.path(), byte).unwrap();
    let errore = valida_flusso_ipc("arrow", file.path()).unwrap_err();
    assert!(
        errore
            .message
            .contains("buffer Arrow oltre la fine del corpo del messaggio"),
        "{errore}"
    );
}

#[test]
fn anche_un_batch_successivo_del_flusso_viene_controllato() {
    let originale = flusso(&seme("dizionario-int8.arrow"));
    let primo =
        8 + usize::try_from(i32::from_le_bytes(originale[4..8].try_into().unwrap())).unwrap();
    let mut byte = originale[..originale.len() - 8].to_vec();
    byte.extend_from_slice(&originale[primo..]);
    let offset = descrittore(&byte, false, true);
    byte[offset..offset + 8].copy_from_slice(&i64::MAX.to_le_bytes());
    let file = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(file.path(), byte).unwrap();
    let errore = valida_flusso_ipc("arrow", file.path()).unwrap_err();
    assert!(
        errore
            .message
            .contains("buffer Arrow oltre la fine del corpo del messaggio"),
        "{errore}"
    );
}
