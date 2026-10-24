//! Regressione #66: un elenco non enumerabile si omette (decisione 0022).

use super::*;
use plenora_io_model::geometry::{PLENORA_GEOMETRY_TYPES_KEY, PLENORA_TYPES_DECLARATION_KEY};

#[test]
fn round_trip_ipc_preserves_mixed_and_unresolved_type_metadata() {
    for (declaration, types) in [
        ("mixed", None),
        ("mixed", Some("point,linestring")),
        ("unresolved", None),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source.arrow");
        let output = dir.path().join("output.arrow");
        // Il vocabolario completo: la prova e' sulla dichiarazione dei tipi,
        // non sulle altre chiavi obbligatorie.
        let mut metadata = super::metadati_geometria_conformi();
        metadata.insert(
            PLENORA_TYPES_DECLARATION_KEY.to_owned(),
            declaration.to_owned(),
        );
        if let Some(types) = types {
            metadata.insert(PLENORA_GEOMETRY_TYPES_KEY.to_owned(), types.to_owned());
        }
        let schema = with_contract_version(Arc::new(Schema::new(vec![Field::new(
            "geometry",
            DataType::Binary,
            true,
        )
        .with_metadata(metadata)])));
        let point = to_wkb(&geo_types::Geometry::Point(geo_types::Point::new(1.0, 2.0))).unwrap();
        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![Arc::new(BinaryArray::from(vec![Some(point.as_slice())]))],
        )
        .unwrap();
        let mut physical = FileWriter::try_new(File::create(&source).unwrap(), &schema).unwrap();
        physical.write(&batch).unwrap();
        physical.finish().unwrap();
        drop(physical);

        let dataset = IpcDriver
            .open(Source::Path(source), opzioni_lettura())
            .unwrap();
        let layer = dataset.layers()[0].clone();
        let mut reader = dataset
            .open_layer_reader(&ReadRequest {
                layer: LayerId(0),
                projected_fields: None,
                projection_mode: ProjectionMode::BestEffort,
                pruning_predicate: None,
                spatial_pruning_hint: None,
                scope: ReadScope::default(),
                batch_target: BatchTarget::default(),
                cancellation: CancellationToken::default(),
            })
            .unwrap();
        let plan = WritePlan {
            layers: vec![WriteLayer {
                name: layer.name,
                contract: layer.contract,
            }],
        };
        let writer = IpcDriver.create(Sink::Path(output.clone()), &plan, &opzioni_scrittura());
        // Unresolved senza scansione completa resta rifiutato dal writer:
        // la correzione non allarga le capability del formato.
        if declaration == "unresolved" {
            let Err(error) = writer else {
                panic!("unresolved senza scansione completa deve restare rifiutato");
            };
            assert_eq!(
                error.capability_reason,
                Some(plenora_io_model::CapabilityReason::GeometryNotSupported)
            );
            assert!(!output.exists());
            continue;
        }
        let mut writer = writer.unwrap();
        while let Some(batch) = reader.next_batch().unwrap() {
            writer.write(&batch).unwrap();
        }
        writer.finish().unwrap();

        let output_rilettura = output.clone();
        let mut published = FileReader::try_new(File::open(output).unwrap(), None).unwrap();
        let schema = published.schema();
        let metadata = schema.field(0).metadata();
        assert_eq!(
            metadata
                .get(PLENORA_TYPES_DECLARATION_KEY)
                .map(String::as_str),
            Some(declaration)
        );
        assert_eq!(
            metadata.get(PLENORA_GEOMETRY_TYPES_KEY).map(String::as_str),
            types
        );
        assert_eq!(published.next().unwrap().unwrap().num_rows(), 1);
        // Cio' che IO pubblica, IO lo rilegge.
        assert_eq!(super::categoria_in_apertura(output_rilettura), None);
    }
}
