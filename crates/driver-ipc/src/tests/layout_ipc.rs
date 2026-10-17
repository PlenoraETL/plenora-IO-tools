//! Regressioni delle assunzioni infallibili di Arrow IPC 60.
use arrow_ipc::*;
use arrow_schema::{DataType, Field as NativeField, Schema as NativeSchema};
use driver_common::prevalida_arrow::{valida_file_ipc, valida_flusso_ipc};
use flatbuffers::FlatBufferBuilder;
use plenora_io_model::{ErrorPhase, IoErrorCode};

use super::schema_ipc::incapsula;

fn fixture(
    tipo: DataType,
    nodi: &[FieldNode],
    lunghezze: &[i64],
    stream: bool,
    dizionario: Option<bool>,
) -> Vec<u8> {
    let run_ends = matches!(tipo, DataType::RunEndEncoded(_, _));
    let tipo = if dizionario.is_some() {
        DataType::Dictionary(Box::new(DataType::Int8), Box::new(tipo))
    } else {
        tipo
    };
    let schema = NativeSchema::new(vec![NativeField::new("valori", tipo, true)]);
    let mut b = FlatBufferBuilder::new();
    let s = convert::IpcSchemaEncoder::new()
        .with_dictionary_tracker(&mut writer::DictionaryTracker::new(false))
        .schema_to_fb_offset(&mut b, &schema);
    let m = Message::create(
        &mut b,
        &MessageArgs {
            version: MetadataVersion::V5,
            header_type: MessageHeader::Schema,
            header: Some(s.as_union_value()),
            ..Default::default()
        },
    );
    b.finish(m, None);
    let dict_id = root_as_message(b.finished_data())
        .unwrap()
        .header_as_schema()
        .unwrap()
        .fields()
        .unwrap()
        .get(0)
        .dictionary()
        .map_or(0, |d| d.id());
    let mut byte = if stream {
        vec![]
    } else {
        b"ARROW1\0\0".to_vec()
    };
    incapsula(b.finished_data(), &[], &mut byte);
    b.reset();
    let (mut corpo, buffers) = corpo_e_buffer(lunghezze);
    if run_ends && !corpo.is_empty() {
        corpo[0] = 1;
    }
    let buffers = b.create_vector(&buffers);
    let nodes = b.create_vector(nodi);
    let variadici = b.create_vector(&[0_i64]);
    let batch = RecordBatch::create(
        &mut b,
        &RecordBatchArgs {
            length: nodi[0].length(),
            nodes: Some(nodes),
            buffers: Some(buffers),
            variadicBufferCounts: Some(variadici),
            ..Default::default()
        },
    );
    let (header_type, header) = dizionario.map_or_else(
        || (MessageHeader::RecordBatch, batch.as_union_value()),
        |delta| {
            let d = DictionaryBatch::create(
                &mut b,
                &DictionaryBatchArgs {
                    id: dict_id,
                    data: Some(batch),
                    isDelta: delta,
                },
            );
            (MessageHeader::DictionaryBatch, d.as_union_value())
        },
    );
    let body_length = i64::try_from(corpo.len()).unwrap();
    let m = Message::create(
        &mut b,
        &MessageArgs {
            version: MetadataVersion::V5,
            header_type,
            header: Some(header),
            bodyLength: body_length,
            ..Default::default()
        },
    );
    b.finish(m, None);
    let offset = i64::try_from(byte.len()).unwrap();
    let n = incapsula(b.finished_data(), &corpo, &mut byte);
    if !stream {
        termina_file(
            &mut byte,
            &schema,
            Block::new(offset, n, body_length),
            dizionario.is_some(),
        );
    }
    byte
}

fn corpo_e_buffer(lunghezze: &[i64]) -> (Vec<u8>, Vec<Buffer>) {
    let mut corpo = Vec::new();
    let buffers: Vec<_> = lunghezze
        .iter()
        .map(|&n| {
            let offset = i64::try_from(corpo.len()).unwrap();
            corpo.resize(
                corpo.len() + usize::try_from(n).unwrap().next_multiple_of(8),
                0,
            );
            Buffer::new(offset, n)
        })
        .collect();
    (corpo, buffers)
}

fn termina_file(byte: &mut Vec<u8>, schema: &NativeSchema, blocco: Block, dizionario: bool) {
    let mut b = FlatBufferBuilder::new();
    let s = convert::IpcSchemaEncoder::new()
        .with_dictionary_tracker(&mut writer::DictionaryTracker::new(false))
        .schema_to_fb_offset(&mut b, schema);
    let blocchi = [blocco];
    let vuoti = b.create_vector::<Block>(&[]);
    let blocchi = b.create_vector(&blocchi);
    let footer = Footer::create(
        &mut b,
        &FooterArgs {
            version: MetadataVersion::V5,
            schema: Some(s),
            dictionaries: Some(if dizionario { blocchi } else { vuoti }),
            recordBatches: Some(if dizionario { vuoti } else { blocchi }),
            ..Default::default()
        },
    );
    b.finish(footer, None);
    byte.extend_from_slice(b.finished_data());
    byte.extend_from_slice(
        &i32::try_from(b.finished_data().len())
            .unwrap()
            .to_le_bytes(),
    );
    byte.extend_from_slice(b"ARROW1");
}

fn verifica(tipo: &DataType, nodi: &[FieldNode], buffer: &[i64], motivo: Option<&str>) {
    for stream in [false, true] {
        for dizionario in [None, Some(false), Some(true)] {
            let byte = fixture(tipo.clone(), nodi, buffer, stream, dizionario);
            let file = tempfile::NamedTempFile::new().unwrap();
            std::fs::write(file.path(), byte).unwrap();
            let valida = if stream {
                valida_flusso_ipc
            } else {
                valida_file_ipc
            };
            // Chiamata diretta: nessun decoder e nessun catch_unwind.
            let esito = valida("arrow", file.path());
            if let Some(motivo) = motivo {
                let e = esito.unwrap_err();
                assert_eq!(e.code, IoErrorCode::Format);
                assert_eq!(e.phase, ErrorPhase::Read);
                assert!(
                    e.message.contains(motivo),
                    "{tipo:?}, stream={stream}, dizionario={dizionario:?}: {e}"
                );
            } else {
                esito.unwrap();
            }
        }
    }
}

#[test]
fn bitmap_delle_viste_in_file_flussi_dizionari_e_delta() {
    for tipo in [DataType::Utf8View, DataType::BinaryView] {
        verifica(
            &tipo,
            &[FieldNode::new(1, 1)],
            &[0, 16],
            Some("bitmap di validita'"),
        );
        verifica(&tipo, &[FieldNode::new(1, 1)], &[1, 16], None);
        verifica(&tipo, &[FieldNode::new(1, 0)], &[0, 16], None);
    }
}

#[test]
fn viste_con_elemento_incompleto() {
    for tipo in [DataType::Utf8View, DataType::BinaryView] {
        verifica(
            &tipo,
            &[FieldNode::new(1, 0)],
            &[0, 17],
            Some("non multipla"),
        );
    }
}

#[test]
fn indici_di_dizionario_con_elemento_incompleto() {
    for (tipo, byte) in [
        (DataType::Int16, 3),
        (DataType::Int32, 5),
        (DataType::Int64, 9),
    ] {
        // Anche dizionari annidati nei valori: il layout interno resta quello degli indici.
        let tipo = DataType::Dictionary(Box::new(tipo), Box::new(DataType::Null));
        let tipo = DataType::Struct(vec![NativeField::new("indici", tipo, true)].into());
        verifica(
            &tipo,
            &[FieldNode::new(1, 0), FieldNode::new(1, 0)],
            &[0, 0, byte],
            Some("non multipla"),
        );
    }
}

#[test]
fn lista_fissa_con_prodotto_non_rappresentabile() {
    let tipo = DataType::FixedSizeList(
        std::sync::Arc::new(NativeField::new("item", DataType::Null, true)),
        3,
    );
    verifica(
        &tipo,
        &[FieldNode::new(i64::MAX, 0), FieldNode::new(0, 0)],
        &[0],
        Some("lista fissa Arrow non rappresentabile"),
    );
}

#[test]
fn run_ends_devono_coprire_la_lunghezza_logica() {
    for (intero, larghezza) in [
        (DataType::Int16, 2),
        (DataType::Int32, 4),
        (DataType::Int64, 8),
    ] {
        let tipo = DataType::RunEndEncoded(
            std::sync::Arc::new(NativeField::new("run_ends", intero, false)),
            std::sync::Arc::new(NativeField::new("values", DataType::Null, true)),
        );
        verifica(
            &tipo,
            &[
                FieldNode::new(2, 0),
                FieldNode::new(1, 0),
                FieldNode::new(1, 1),
            ],
            &[0, larghezza],
            Some("lunghezza logica"),
        );
        verifica(
            &tipo,
            &[
                FieldNode::new(1, 0),
                FieldNode::new(0, 0),
                FieldNode::new(0, 0),
            ],
            &[0, 0],
            Some("lunghezza logica"),
        );
        verifica(
            &tipo,
            &[
                FieldNode::new(1, 0),
                FieldNode::new(1, 0),
                FieldNode::new(1, 1),
            ],
            &[0, larghezza],
            None,
        );
    }
}
