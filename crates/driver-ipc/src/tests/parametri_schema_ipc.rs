//! Schemi pericolosi anche quando non esiste un `DictionaryBatch` da validare.
use std::sync::Arc;

use arrow_ipc::FieldNode;
use arrow_schema::{DataType, Field, UnionFields, UnionMode};
use driver_common::prevalida_arrow::{valida_file_ipc, valida_flusso_ipc, valida_schema};
use plenora_io_model::{ErrorPhase, IoErrorCode};

use super::layout_ipc::fixture;

fn campo(tipo: DataType) -> Arc<Field> {
    Arc::new(Field::new("figlio", tipo, false))
}

fn verifica(tipo: &DataType, motivo: &str) {
    // Anche sotto un contenitore: i valori sintetizzati visitano tutto l'albero.
    for tipo in [tipo.clone(), DataType::List(campo(tipo.clone()))] {
        for stream in [false, true] {
            // Batch ordinario, dizionario, delta e indici senza DictionaryBatch.
            for (dizionario, ometti) in [
                (None, false),
                (Some(false), false),
                (Some(true), false),
                (Some(false), true),
            ] {
                let byte = fixture(
                    tipo.clone(),
                    &[FieldNode::new(0, 0)],
                    &[0, 0],
                    stream,
                    dizionario,
                    ometti,
                );
                let file = tempfile::NamedTempFile::new().unwrap();
                std::fs::write(file.path(), byte).unwrap();
                let valida = if stream {
                    valida_flusso_ipc
                } else {
                    valida_file_ipc
                };
                // Nessun decoder, nessun catch_unwind: il motivo prova che e'
                // lo schema a fermare la lettura, prima dei descrittori del batch.
                let e = valida("arrow", file.path()).unwrap_err();
                assert_eq!(e.code, IoErrorCode::Format);
                assert_eq!(e.phase, ErrorPhase::Read);
                assert!(e.message.contains(motivo), "{tipo:?}: {e}");
            }
        }
    }
}

#[test]
fn larghezza_binaria_negativa_anche_senza_dictionary_batch() {
    verifica(
        &DataType::FixedSizeBinary(-1),
        "larghezza binaria fissa Arrow negativa",
    );
}

#[test]
fn lista_fissa_negativa_anche_senza_dictionary_batch() {
    verifica(
        &DataType::FixedSizeList(campo(DataType::Null), -1),
        "dimensione della lista fissa Arrow negativa",
    );
}

#[test]
fn unione_vuota_anche_senza_dictionary_batch() {
    for mode in [UnionMode::Dense, UnionMode::Sparse] {
        verifica(
            &DataType::Union(UnionFields::empty(), mode),
            "unione Arrow senza figli",
        );
    }
}

#[test]
fn mappa_entries_non_struct_di_due_campi() {
    for tipo in [
        DataType::Null,
        DataType::Struct(arrow_schema::Fields::empty()),
        DataType::Struct(vec![campo(DataType::Null)].into()),
        DataType::Struct(vec![campo(DataType::Null); 3].into()),
        DataType::Dictionary(
            Box::new(DataType::Int8),
            Box::new(DataType::Struct(vec![campo(DataType::Null); 2].into())),
        ),
    ] {
        verifica(
            &DataType::Map(campo(tipo), false),
            "entries della mappa Arrow non struct di due campi",
        );
    }
}

#[test]
fn run_ends_tipo_effettivo_anche_senza_dictionary_batch() {
    for tipo in [
        DataType::Null,
        DataType::Int8,
        DataType::UInt16,
        DataType::Float32,
        DataType::Dictionary(Box::new(DataType::Int8), Box::new(DataType::Int32)),
    ] {
        verifica(
            &DataType::RunEndEncoded(campo(tipo), campo(DataType::Null)),
            "run ends Arrow non Int16, Int32 o Int64",
        );
    }
}

#[test]
fn schemi_limite_ammessi_producono_valori_vuoti() {
    for tipo in [
        DataType::FixedSizeBinary(0),
        DataType::FixedSizeList(campo(DataType::Null), 0),
        DataType::Struct(arrow_schema::Fields::empty()),
        DataType::ListView(campo(DataType::Null)),
        DataType::LargeListView(campo(DataType::Null)),
        DataType::Map(
            campo(DataType::Struct(vec![campo(DataType::Null); 2].into())),
            false,
        ),
        DataType::RunEndEncoded(campo(DataType::Int16), campo(DataType::Null)),
        DataType::Union(
            UnionFields::try_new([127], [campo(DataType::Null)]).unwrap(),
            UnionMode::Dense,
        ),
        DataType::Decimal32(9, -128),
        DataType::Decimal64(18, 18),
        DataType::Decimal128(38, 38),
        DataType::Decimal256(76, 76),
    ] {
        let schema = arrow_schema::Schema::new(vec![Field::new("valori", tipo.clone(), true)]);
        let mut b = flatbuffers::FlatBufferBuilder::new();
        let s = arrow_ipc::convert::IpcSchemaEncoder::new().schema_to_fb_offset(&mut b, &schema);
        b.finish(s, None);
        let s = flatbuffers::root::<arrow_ipc::Schema<'_>>(b.finished_data()).unwrap();
        valida_schema("arrow", s).unwrap();
        let convertito = arrow_ipc::convert::try_fb_to_schema(s).unwrap();
        assert_eq!(convertito.field(0).data_type(), &tipo);
        assert_eq!(arrow_array::new_empty_array(&tipo).len(), 0);
        // Percorso reale che sintetizza valori vuoti per il dizionario omesso.
        for stream in [false, true] {
            let byte = fixture(
                tipo.clone(),
                &[FieldNode::new(0, 0)],
                &[0, 0],
                stream,
                Some(false),
                true,
            );
            let file = tempfile::NamedTempFile::new().unwrap();
            std::fs::write(file.path(), &byte).unwrap();
            if stream {
                valida_flusso_ipc("arrow", file.path()).unwrap();
                arrow_ipc::reader::StreamReader::try_new(std::io::Cursor::new(byte), None)
                    .unwrap()
                    .next()
                    .unwrap()
                    .unwrap();
            } else {
                valida_file_ipc("arrow", file.path()).unwrap();
                arrow_ipc::reader::FileReader::try_new(std::io::Cursor::new(byte), None)
                    .unwrap()
                    .next()
                    .unwrap()
                    .unwrap();
            }
        }
    }
}
