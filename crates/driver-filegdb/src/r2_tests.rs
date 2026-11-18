use super::*;

#[test]
fn catalogo_con_oggetti_non_leggibili_non_scala_gli_indici() {
    use gdal::vector::{FieldDefn, LayerOptions, OGRFieldType};
    use plenora_io_model::CapabilityReason as R;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("objects.gdb");
    let driver = gdal::DriverManager::get_driver_by_name("OpenFileGDB").unwrap();
    let mut dataset = driver.create_vector_only(&path).unwrap();
    let cases = [
        ("Healthy", OGRFieldType::OFTString, None),
        (
            "NativeDate",
            OGRFieldType::OFTDate,
            Some(R::TypeNotRepresentable),
        ),
        (
            "DateTime",
            OGRFieldType::OFTDateTime,
            Some(R::TypeNotRepresentable),
        ),
        (
            "Binary",
            OGRFieldType::OFTBinary,
            Some(R::TypeNotRepresentable),
        ),
        ("Missing", OGRFieldType::OFTString, Some(R::ObjectMissing)),
        ("Empty", OGRFieldType::OFTString, Some(R::ObjectEmpty)),
    ];
    for (name, typ, _) in cases {
        let layer = dataset
            .create_layer(LayerOptions {
                name,
                ty: gdal::vector::OGRwkbGeometryType::wkbNone,
                ..Default::default()
            })
            .unwrap();
        FieldDefn::new("value", typ)
            .unwrap()
            .add_to_layer(&layer)
            .unwrap();
    }
    drop(dataset);
    let objects = catalog_objects(&path).unwrap();
    for (id, name) in objects {
        if name == "Missing" {
            std::fs::remove_file(path.join(format!("a{id:08x}.gdbtable"))).unwrap();
        }
        if name == "Empty" {
            std::fs::write(path.join(format!("a{id:08x}.gdbtablx")), []).unwrap();
        }
    }
    for _ in 0..2 {
        let dataset = open(&path, None).unwrap();
        assert_eq!(dataset.layers().len(), cases.len());
        for (index, (name, _, reason)) in cases.iter().enumerate() {
            let layer = &dataset.layers()[index];
            assert_eq!(&layer.name, name);
            assert_eq!(layer.id.0 as usize, index);
            let request = ReadRequest {
                layer: layer.id,
                projected_fields: None,
                projection_mode: plenora_io_core::ProjectionMode::BestEffort,
                pruning_predicate: None,
                spatial_pruning_hint: None,
                scope: plenora_io_core::ReadScope::Complete,
                batch_target: plenora_io_core::BatchTarget::default(),
                cancellation: plenora_io_model::CancellationToken::default(),
            };
            if let Some(reason) = reason {
                assert_eq!(
                    dataset
                        .rifiuto_del_layer(layer.id)
                        .unwrap()
                        .capability_reason,
                    Some(*reason)
                );
                assert_eq!(
                    dataset
                        .open_layer_reader(&request)
                        .err()
                        .unwrap()
                        .capability_reason,
                    Some(*reason)
                );
            } else {
                assert!(dataset.rifiuto_del_layer(layer.id).is_none());
                assert!(
                    dataset
                        .open_layer_reader(&request)
                        .unwrap()
                        .next_batch()
                        .unwrap()
                        .is_none()
                );
            }
        }
    }
}
