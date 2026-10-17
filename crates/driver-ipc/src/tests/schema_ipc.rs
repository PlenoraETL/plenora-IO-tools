//! Divergenze fra schema `FlatBuffer` e albero effettivo di Arrow 60.
use arrow_ipc::*;
use driver_common::prevalida_arrow::{valida_file_ipc, valida_flusso_ipc, valida_schema};
use flatbuffers::{FlatBufferBuilder, WIPOffset};
use plenora_io_model::{ErrorPhase, IoErrorCode};

fn nullo<'a>(b: &mut FlatBufferBuilder<'a>, dizionario: bool) -> WIPOffset<Field<'a>> {
    let tipo = Null::create(b, &NullArgs {}).as_union_value();
    let dictionary = dizionario.then(|| {
        let indice = Int::create(
            b,
            &IntArgs {
                bitWidth: 8,
                is_signed: true,
            },
        );
        DictionaryEncoding::create(
            b,
            &DictionaryEncodingArgs {
                id: 7,
                indexType: Some(indice),
                ..Default::default()
            },
        )
    });
    Field::create(
        b,
        &FieldArgs {
            type_type: Type::Null,
            type_: Some(tipo),
            dictionary,
            ..Default::default()
        },
    )
}

fn schema<'a>(b: &mut FlatBufferBuilder<'a>, contenitore: Type) -> WIPOffset<Schema<'a>> {
    let spurio = nullo(b, true);
    let valore = nullo(b, false);
    let figli = b.create_vector(&[spurio, valore]);
    let tipo = match contenitore {
        Type::RunEndEncoded => RunEndEncoded::create(b, &RunEndEncodedArgs {}).as_union_value(),
        Type::Struct_ => Struct_::create(b, &Struct_Args {}).as_union_value(),
        Type::Bool => Bool::create(b, &BoolArgs {}).as_union_value(),
        Type::Binary => Binary::create(b, &BinaryArgs {}).as_union_value(),
        Type::LargeBinary => LargeBinary::create(b, &LargeBinaryArgs {}).as_union_value(),
        Type::BinaryView => BinaryView::create(b, &BinaryViewArgs {}).as_union_value(),
        Type::Utf8 => Utf8::create(b, &Utf8Args {}).as_union_value(),
        Type::LargeUtf8 => LargeUtf8::create(b, &LargeUtf8Args {}).as_union_value(),
        Type::Utf8View => Utf8View::create(b, &Utf8ViewArgs {}).as_union_value(),
        _ => Null::create(b, &NullArgs {}).as_union_value(),
    };
    let primo = Field::create(
        b,
        &FieldArgs {
            type_type: contenitore,
            type_: Some(tipo),
            children: Some(figli),
            ..Default::default()
        },
    );
    let figlio = nullo(b, false);
    let figli = b.create_vector(&[figlio]);
    let ids = b.create_vector(&[0_i32]);
    let tipo = Union::create(
        b,
        &UnionArgs {
            mode: UnionMode::Sparse,
            typeIds: Some(ids),
        },
    )
    .as_union_value();
    let indice = Int::create(
        b,
        &IntArgs {
            bitWidth: 8,
            is_signed: true,
        },
    );
    let dictionary = DictionaryEncoding::create(
        b,
        &DictionaryEncodingArgs {
            id: 7,
            indexType: Some(indice),
            ..Default::default()
        },
    );
    let reale = Field::create(
        b,
        &FieldArgs {
            type_type: Type::Union,
            type_: Some(tipo),
            children: Some(figli),
            dictionary: Some(dictionary),
            ..Default::default()
        },
    );
    let campi = b.create_vector(&[primo, reale]);
    Schema::create(
        b,
        &SchemaArgs {
            fields: Some(campi),
            ..Default::default()
        },
    )
}

fn incapsula(metadati: &[u8], corpo: &[u8], byte: &mut Vec<u8>) -> i32 {
    let n = metadati.len().next_multiple_of(8);
    byte.extend_from_slice(&[255; 4]);
    byte.extend_from_slice(&i32::try_from(n).unwrap().to_le_bytes());
    byte.extend_from_slice(metadati);
    byte.resize(byte.len() + n - metadati.len(), 0);
    byte.extend_from_slice(corpo);
    i32::try_from(n + 8).unwrap()
}

fn fixture(contenitore: Type, stream: bool, delta: bool) -> Vec<u8> {
    let mut b = FlatBufferBuilder::new();
    let s = schema(&mut b, contenitore);
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
    let mut byte = if stream {
        vec![]
    } else {
        b"ARROW1\0\0".to_vec()
    };
    incapsula(b.finished_data(), &[], &mut byte);
    b.reset();
    let nodi = b.create_vector(&[FieldNode::new(2, 0), FieldNode::new(2, 2)]);
    // Dentro il corpo, ma insufficiente per i due identificatori della Union.
    let buffer = b.create_vector(&[Buffer::new(0, 1)]);
    let batch = RecordBatch::create(
        &mut b,
        &RecordBatchArgs {
            length: 2,
            nodes: Some(nodi),
            buffers: Some(buffer),
            ..Default::default()
        },
    );
    let d = DictionaryBatch::create(
        &mut b,
        &DictionaryBatchArgs {
            id: 7,
            data: Some(batch),
            isDelta: delta,
        },
    );
    let m = Message::create(
        &mut b,
        &MessageArgs {
            version: MetadataVersion::V5,
            header_type: MessageHeader::DictionaryBatch,
            header: Some(d.as_union_value()),
            bodyLength: 8,
            ..Default::default()
        },
    );
    b.finish(m, None);
    let offset = i64::try_from(byte.len()).unwrap();
    let lunghezza = incapsula(b.finished_data(), &[0; 8], &mut byte);
    if !stream {
        b.reset();
        let s = schema(&mut b, contenitore);
        let dizionari = b.create_vector(&[Block::new(offset, lunghezza, 8)]);
        let blocchi = b.create_vector::<Block>(&[]);
        let footer = Footer::create(
            &mut b,
            &FooterArgs {
                version: MetadataVersion::V5,
                schema: Some(s),
                dictionaries: Some(dizionari),
                recordBatches: Some(blocchi),
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
    byte
}

#[test]
fn dizionario_ombra_in_file_flussi_e_delta() {
    for contenitore in [
        Type::Null,
        Type::Bool,
        Type::Binary,
        Type::LargeBinary,
        Type::BinaryView,
        Type::Utf8,
        Type::LargeUtf8,
        Type::Utf8View,
        Type::RunEndEncoded,
        Type::Struct_,
    ] {
        for stream in [false, true] {
            for delta in [false, true] {
                let byte = fixture(contenitore, stream, delta);
                let file = tempfile::NamedTempFile::new().unwrap();
                std::fs::write(file.path(), byte).unwrap();
                let valida = if stream {
                    valida_flusso_ipc
                } else {
                    valida_file_ipc
                };
                let esito = valida("arrow", file.path());
                if contenitore == Type::Struct_ {
                    // ID condiviso visibile: Arrow usa il primo (Null), come noi.
                    esito.expect("il primo dizionario visibile resta quello selezionato");
                } else {
                    let e = esito.unwrap_err();
                    assert_eq!(e.code, IoErrorCode::Format);
                    assert_eq!(e.phase, ErrorPhase::Read);
                    let motivo = if contenitore == Type::RunEndEncoded {
                        "buffer degli identificatori di unione Arrow piu' corto"
                    } else {
                        "numero di figli non conforme"
                    };
                    assert!(e.message.contains(motivo), "{e}");
                }
            }
        }
    }
}

#[test]
fn unioni_con_identificatori_ambigui_o_impliciti_fuori_limite() {
    for (quanti, ids, valido) in [
        (128, None, true),
        (129, None, false),
        (2, Some(vec![0, 0]), false),
        (1, Some(vec![-1]), false),
        (1, Some(vec![128]), false),
        (1, Some(vec![0]), true),
    ] {
        let mut b = FlatBufferBuilder::new();
        let campi: Vec<_> = (0..quanti).map(|_| nullo(&mut b, false)).collect();
        let figli = b.create_vector(&campi);
        let ids = ids.map(|v| b.create_vector(&v));
        let tipo = Union::create(
            &mut b,
            &UnionArgs {
                mode: UnionMode::Sparse,
                typeIds: ids,
            },
        )
        .as_union_value();
        let unione = Field::create(
            &mut b,
            &FieldArgs {
                type_type: Type::Union,
                type_: Some(tipo),
                children: Some(figli),
                ..Default::default()
            },
        );
        let campi = b.create_vector(&[unione]);
        let s = Schema::create(
            &mut b,
            &SchemaArgs {
                fields: Some(campi),
                ..Default::default()
            },
        );
        b.finish(s, None);
        let s = flatbuffers::root::<Schema<'_>>(b.finished_data()).unwrap();
        let esito = valida_schema("arrow", s);
        assert_eq!(esito.is_ok(), valido);
        if let Err(e) = esito {
            assert_eq!(e.code, IoErrorCode::Format);
            assert_eq!(e.phase, ErrorPhase::Read);
        } else {
            arrow_ipc::convert::try_fb_to_schema(s).unwrap();
        }
    }
}
