use super::*;

#[test]
fn parquet_senza_footer_geo_conserva_il_crs_arrow_dichiarato() {
    for key in ["crs", "ARROW:extension:metadata"] {
        let value = if key == "crs" {
            "EPSG:3857"
        } else {
            r#"{"crs":"EPSG:3857","crs_type":"authority_code"}"#
        };
        let field = Field::new("geometry", DataType::Binary, true).with_metadata(
            std::iter::once((key.to_owned(), value.to_owned())).collect::<arrow_schema::Metadata>(),
        );
        let (_, crs) = resolve_geometry_and_crs(&Schema::new(vec![field]), None, None).unwrap();
        assert_eq!(crs.id(), Some("EPSG:3857"));
    }
}

#[test]
fn crs_legacy_secondario_senza_projjson_non_diventa_null() {
    let (mut contratto, _) = leggi(fixture());
    let mut fields: Vec<Field> = contratto
        .schema
        .fields()
        .iter()
        .map(|f| f.as_ref().clone())
        .collect();
    let mut md = fields[0].metadata().clone();
    md.insert("plenora.geometry.crs_resolution", "resolved");
    md.insert("plenora.geometry.crs_id", "EPSG:3857");
    md.remove("plenora.geometry.crs_definition");
    md.remove("plenora.geometry.crs_definition_format");
    md.insert("plenora.geometry.axis_order", "easting_northing");
    md.insert("crs", "EPSG:3857");
    fields[0] = driver_common::geometry_field("traccia", "EPSG:3857").with_metadata(md);
    contratto.schema = Arc::new(Schema::new_with_metadata(
        fields,
        contratto.schema.metadata().clone(),
    ));
    assert_eq!(
        contratto.geometries().unwrap()[0].crs.id(),
        Some("EPSG:3857")
    );
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("rifiutato.parquet");
    let plan = WritePlan {
        layers: vec![WriteLayer {
            name: "l".into(),
            contract: contratto,
        }],
    };
    let error = GeoParquetDriver
        .create(Sink::Path(path.clone()), &plan, &opzioni_scrittura())
        .err()
        .unwrap();
    assert_eq!(error.category, plenora_io_model::ErrorCategory::Unsupported);
    let crs_error = crs_da_scrivere(
        Some(&plan.layers[0].contract.geometries().unwrap()[0]),
        None,
    )
    .unwrap_err();
    assert!(crs_error.message.contains("PROJJSON"));
    assert!(!path.exists());
}

#[test]
fn crs_geoarrow_per_colonna_round_trip_con_coordinate_asimmetriche() {
    let (mut contratto, batch) = leggi(fixture());
    let geometries = contratto.geometries().unwrap();
    let fields = contratto
        .schema
        .fields()
        .iter()
        .map(|field| {
            let Some(g) = geometries.iter().find(|g| &g.name == field.name()) else {
                return field.as_ref().clone();
            };
            let extension = g.crs.definition().map_or_else(
                || {
                    format!(
                        r#"{{"crs":"{}","crs_type":"authority_code"}}"#,
                        g.crs.id().unwrap()
                    )
                },
                |definition| format!(r#"{{"crs":{definition},"crs_type":"projjson"}}"#),
            );
            let mut md = field.metadata().clone();
            md.remove("crs");
            md.insert("ARROW:extension:metadata", extension);
            field.as_ref().clone().with_metadata(md)
        })
        .collect::<Vec<_>>();
    contratto.schema = Arc::new(Schema::new_with_metadata(
        fields,
        contratto.schema.metadata().clone(),
    ));
    let batch = RecordBatch::try_new(contratto.schema.clone(), batch.columns().to_vec()).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("estensioni.parquet");
    let plan = WritePlan {
        layers: vec![WriteLayer {
            name: "l".into(),
            contract: contratto,
        }],
    };
    let mut writer = GeoParquetDriver
        .create(Sink::Path(path.clone()), &plan, &opzioni_scrittura())
        .unwrap();
    writer.write(&batch).unwrap();
    writer.finish().unwrap();
    let (read, values) = leggi(path);
    assert_eq!(batch.columns(), values.columns());
    for (before, after) in geometries.iter().zip(read.geometries().unwrap()) {
        assert_eq!(before.crs, after.crs);
    }
}

#[test]
fn footer_e_dichiarazioni_arrow_in_conflitto_su_ogni_colonna_sono_rifiutati() {
    let builder = ParquetRecordBatchReaderBuilder::try_new(File::open(fixture()).unwrap()).unwrap();
    let geo = read_geo_meta(&builder, false).unwrap().unwrap();
    for index in [0, 2] {
        for key in ["crs", "ARROW:extension:metadata"] {
            let mut fields: Vec<Field> = builder
                .schema()
                .fields()
                .iter()
                .map(|f| f.as_ref().clone())
                .collect();
            let value = if key == "crs" {
                "EPSG:3857"
            } else {
                r#"{"crs":"EPSG:3857","crs_type":"authority_code"}"#
            };
            fields[index] = fields[index].clone().with_metadata(
                std::iter::once((key.to_owned(), value.to_owned()))
                    .collect::<arrow_schema::Metadata>(),
            );
            let error =
                crate::multigeometria::verifica_crs(&Schema::new(fields), &geo).unwrap_err();
            assert_eq!(error.category, plenora_io_model::ErrorCategory::Schema);
        }
    }
}

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/multigeometria-pyarrow.parquet")
}

fn leggi(percorso: PathBuf) -> (DataContract, RecordBatch) {
    let dataset = GeoParquetDriver
        .open(Source::Path(percorso), opzioni_lettura())
        .unwrap();
    let contratto = dataset.layers()[0].contract.clone();
    let mut reader = dataset.open_layer_reader(&richiesta_semplice()).unwrap();
    let batch = reader.next_batch().unwrap().unwrap();
    assert!(reader.next_batch().unwrap().is_none());
    (contratto, batch)
}

#[test]
fn un_nome_sconosciuto_non_eredita_i_metadati_della_primaria() {
    let (contratto, _) = leggi(fixture());
    let builder = ParquetRecordBatchReaderBuilder::try_new(File::open(fixture()).unwrap()).unwrap();
    let geo = read_geo_meta(&builder, false).unwrap().unwrap();
    for nome in ["posizione-segreta", "traccia-segreta", ""] {
        let mut geometria = contratto.geometry.clone().unwrap();
        geometria.name = nome.to_owned();
        geometria.native_metadata.clear();
        let tipi = geometria.geometry_types.clone();
        let errore = apply_geo_column_metadata(&mut geometria, Some(&geo)).unwrap_err();
        assert!(errore
            .to_string()
            .contains("colonna geometrica assente dai metadati GeoParquet"));
        assert!(!errore.to_string().contains("segreta"));
        assert!(geometria.native_metadata.is_empty());
        assert_eq!(geometria.geometry_types, tipi);
    }
}

#[test]
fn pyarrow_multigeometria_round_trip_esatto_con_primaria_non_iniziale() {
    let (contratto, batch) = leggi(fixture());
    assert_eq!(contratto.geometry.as_ref().unwrap().name, "posizione");
    let geometrie = contratto.geometries().unwrap();
    assert_eq!(geometrie.len(), 2);
    assert_eq!(geometrie[0].name, "traccia");
    assert_eq!(geometrie[0].crs.id(), Some("EPSG:32632"));
    assert_eq!(geometrie[1].crs.id(), Some("OGC:CRS84"));
    assert_eq!(geometrie[0].geometry_types, [GeometryType::LineString]);
    assert_eq!(geometrie[1].geometry_types, [GeometryType::Point]);
    assert_eq!(
        batch.schema().field(0).metadata()["plenora.geometry.axis_order"],
        "easting_northing"
    );
    assert_eq!(
        batch.schema().field(2).metadata()["plenora.geometry.axis_order"],
        "lon_lat"
    );
    plenora_io_model::vocabolario::verifica_schema(&contratto.schema).unwrap();

    let dir = tempfile::tempdir().unwrap();
    let uscita = dir.path().join("roundtrip.parquet");
    let piano = WritePlan {
        layers: vec![WriteLayer {
            name: "l".into(),
            contract: contratto,
        }],
    };
    let mut writer = GeoParquetDriver
        .create(Sink::Path(uscita.clone()), &piano, &opzioni_scrittura())
        .unwrap();
    writer.write(&batch).unwrap();
    writer.finish().unwrap();
    let (riletto, dati) = leggi(uscita.clone());
    assert_eq!(riletto.geometry.as_ref().unwrap().name, "posizione");
    assert_eq!(batch.columns(), dati.columns());
    for (prima, dopo) in geometrie.iter().zip(riletto.geometries().unwrap()) {
        assert_eq!(prima.name, dopo.name);
        assert_eq!(prima.crs, dopo.crs);
        assert_eq!(prima.encoding, dopo.encoding);
        assert_eq!(prima.dimensions, dopo.dimensions);
        assert_eq!(prima.geometry_types, dopo.geometry_types);
        assert_eq!(prima.nullable, dopo.nullable);
    }
    let builder = ParquetRecordBatchReaderBuilder::try_new(File::open(uscita).unwrap()).unwrap();
    let geo = read_geo_meta(&builder, false).unwrap().unwrap();
    assert_eq!(geo.nome_primaria, "posizione");
    assert!(geo.primaria.covering.is_some());
    assert!(geo.secondarie["traccia"].covering.is_none());
}

#[test]
fn secondaria_invalida_non_si_scrive_ne_si_pubblica() {
    let (contratto, batch) = leggi(fixture());
    let dir = tempfile::tempdir().unwrap();
    let uscita = dir.path().join("non-pubblicato.parquet");
    let piano = WritePlan {
        layers: vec![WriteLayer {
            name: "l".into(),
            contract: contratto,
        }],
    };
    let mut writer = GeoParquetDriver
        .create(Sink::Path(uscita.clone()), &piano, &opzioni_scrittura())
        .unwrap();
    let mut colonne = batch.columns().to_vec();
    colonne[0] = Arc::new(BinaryArray::from(vec![
        Some(b"segreto-non-wkb".as_slice()),
        None,
        None,
    ]));
    let invalido = RecordBatch::try_new(batch.schema(), colonne).unwrap();
    let errore = writer.write(&invalido).unwrap_err();
    assert!(!errore.message.contains("segreto"));
    assert!(writer.finish().is_err());
    assert!(!uscita.exists());
}

#[test]
fn assume_crs_si_verifica_anche_sulla_secondaria() {
    let mut opzioni = opzioni_lettura();
    opzioni.assume_crs = Some("OGC:CRS84".into());
    let errore = GeoParquetDriver
        .open(Source::Path(fixture()), opzioni)
        .err()
        .unwrap();
    assert_eq!(errore.category, plenora_io_model::ErrorCategory::Crs);
}

#[test]
fn scrittura_multigeometria_senza_primaria_non_sceglie() {
    let (mut contratto, _) = leggi(fixture());
    contratto.geometry = None;
    contratto.schema = plenora_io_model::primary::emit(&contratto.schema, None).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let uscita = dir.path().join("senza-primaria.parquet");
    let piano = WritePlan {
        layers: vec![WriteLayer {
            name: "l".into(),
            contract: contratto,
        }],
    };
    let errore = GeoParquetDriver
        .create(Sink::Path(uscita.clone()), &piano, &opzioni_scrittura())
        .err()
        .unwrap();
    assert_eq!(
        errore.category,
        plenora_io_model::ErrorCategory::Unsupported
    );
    assert!(!uscita.exists());
}

#[test]
fn tipi_della_secondaria_si_verificano_in_lettura() {
    let dir = tempfile::tempdir().unwrap();
    let geo = GEO_DUE_GEOMETRIE.replace(
        "\"geometry_types\":[]",
        "\"geometry_types\":[\"LineString\"]",
    );
    let percorso = parquet_con_due_binarie(&dir, &geo);
    let dataset = GeoParquetDriver
        .open(Source::Path(percorso), opzioni_lettura())
        .unwrap();
    let mut reader = dataset.open_layer_reader(&richiesta_semplice()).unwrap();
    assert!(reader.next_batch().is_err());
}

#[test]
fn quota_componenti_somma_primaria_e_secondaria() {
    let (contratto, batch) = leggi(fixture());
    let wkb_limits = opzioni_scrittura().wkb_limits();
    let totale: usize = batch
        .columns()
        .iter()
        .filter_map(|a| a.as_any().downcast_ref::<BinaryArray>())
        .flat_map(BinaryArray::iter)
        .flatten()
        .map(|bytes| {
            plenora_io_model::wkb::inspect_wkb(bytes, &wkb_limits)
                .unwrap()
                .components
        })
        .sum();
    let limiti = plenora_io_model::budget::PipelineLimits::default()
        .with_max_geometry_components(u64::try_from(totale - 1).unwrap());
    let bundle = plenora_io_model::budget::PipelineBudget::builder()
        .limits(limiti)
        .build()
        .unwrap();
    let dataset = GeoParquetDriver
        .open(
            Source::Path(fixture()),
            ReadOptions::from_read_parts(bundle.into_read_parts()),
        )
        .unwrap();
    let mut reader = dataset.open_layer_reader(&richiesta_semplice()).unwrap();
    assert_eq!(
        reader.next_batch().unwrap_err().category,
        plenora_io_model::ErrorCategory::ResourceLimit
    );

    let bundle = plenora_io_model::budget::PipelineBudget::builder()
        .limits(limiti)
        .build()
        .unwrap();
    let dir = tempfile::tempdir().unwrap();
    let uscita = dir.path().join("quota.parquet");
    let piano = WritePlan {
        layers: vec![WriteLayer {
            name: "l".into(),
            contract: contratto,
        }],
    };
    let mut writer = GeoParquetDriver
        .create(
            Sink::Path(uscita.clone()),
            &piano,
            &WriteOptions::from_write_parts(bundle.into_write_parts()),
        )
        .unwrap();
    assert_eq!(
        writer.write(&batch).unwrap_err().category,
        plenora_io_model::ErrorCategory::ResourceLimit
    );
    drop(writer);
    assert!(!uscita.exists());
}

#[test]
fn projection_secondaria_non_diventa_primaria_in_scrittura() {
    let dataset = GeoParquetDriver
        .open(Source::Path(fixture()), opzioni_lettura())
        .unwrap();
    let reader = dataset
        .open_layer_reader(&richiesta_proiettata(&[0]))
        .unwrap();
    assert!(reader.contract().contract.geometry.is_none());
    let piano = WritePlan {
        layers: vec![WriteLayer {
            name: "l".into(),
            contract: reader.contract().contract.clone(),
        }],
    };
    let dir = tempfile::tempdir().unwrap();
    let uscita = dir.path().join("senza-primaria.parquet");
    assert_eq!(
        GeoParquetDriver
            .create(Sink::Path(uscita.clone()), &piano, &opzioni_scrittura())
            .err()
            .unwrap()
            .category,
        plenora_io_model::ErrorCategory::Unsupported
    );
    assert!(!uscita.exists());
}

#[test]
fn pyarrow_identita_e_perdite_metadati_per_ogni_colonna() {
    let percorso = fixture().with_file_name("multigeometria-metadati-pyarrow.parquet");
    let (contratto, batch) = leggi(percorso);
    let geometrie = contratto.geometries().unwrap();
    for (g, id) in geometrie.iter().zip([42, 73]) {
        assert_eq!(g.identita_dichiarata, Some(id));
        assert!(g.native_metadata.contains_key("geoparquet.bbox"));
        assert_eq!(
            g.native_metadata["geoparquet.orientation"],
            "counterclockwise"
        );
    }
    assert_eq!(geometrie[0].field_id, FieldId(0));
    assert_eq!(geometrie[1].field_id, FieldId(2));
    assert_eq!(geometrie[0].geometry_types, [GeometryType::Polygon]);
    assert_eq!(
        geometrie[0].native_metadata["geoparquet.bbox"],
        "500000,4982950,500010,4982960"
    );
    let dir = tempfile::tempdir().unwrap();
    let uscita = dir.path().join("metadati.parquet");
    let piano = WritePlan {
        layers: vec![WriteLayer {
            name: "l".into(),
            contract: contratto,
        }],
    };
    let mut writer = GeoParquetDriver
        .create(Sink::Path(uscita.clone()), &piano, &opzioni_scrittura())
        .unwrap();
    writer.write(&batch).unwrap();
    let esito = writer.finish().unwrap();
    for categoria in [
        "geometry_bbox_not_preserved",
        "geometry_orientation_not_preserved",
    ] {
        assert_eq!(esito.loss.counts.get(categoria), Some(&2));
        let posizioni: Vec<_> = esito
            .loss
            .esempi_canonici()
            .filter(|e| e.category == categoria)
            .map(|e| (e.posizione.layer_index, e.posizione.field_index))
            .collect();
        assert_eq!(posizioni, [(Some(0), Some(0)), (Some(0), Some(2))]);
    }
    let (riletto, dati) = leggi(uscita);
    assert_eq!(batch.columns(), dati.columns());
    for (g, id) in riletto.geometries().unwrap().iter().zip([42, 73]) {
        assert_eq!(g.identita_dichiarata, Some(id));
        assert!(!g.native_metadata.contains_key("geoparquet.bbox"));
        assert!(!g.native_metadata.contains_key("geoparquet.orientation"));
    }
}

#[test]
fn covering_second_primary_nonpositional_ids_and_secondary_projection() {
    let (contract, batch) =
        leggi(fixture().with_file_name("multigeometria-metadati-pyarrow.parquet"));
    assert_eq!(
        contract.geometry.as_ref().unwrap().identita_dichiarata,
        Some(73)
    );
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("covering.parquet");
    let plan = WritePlan {
        layers: vec![WriteLayer {
            name: "l".into(),
            contract,
        }],
    };
    let mut writer = GeoParquetDriver
        .create(Sink::Path(path.clone()), &plan, &opzioni_scrittura())
        .unwrap();
    writer.write(&batch).unwrap();
    writer.finish().unwrap();
    let dataset = GeoParquetDriver
        .open(Source::Path(path), opzioni_lettura())
        .unwrap();
    for fields in [vec![0], vec![2], vec![0, 1, 2]] {
        for outside in [false, true] {
            let mut request = richiesta_proiettata(&fields);
            request.spatial_pruning_hint = Some(plenora_io_core::request::Bbox {
                minx: if outside { 100. } else { 8. },
                miny: 44.,
                maxx: if outside { 101. } else { 10. },
                maxy: 46.,
            });
            let mut reader = dataset.open_layer_reader(&request).unwrap();
            let expected_primary = fields.contains(&2).then_some(73);
            assert_eq!(
                reader
                    .contract()
                    .contract
                    .geometry
                    .as_ref()
                    .and_then(|g| g.identita_dichiarata),
                expected_primary
            );
            let mut rows = 0;
            while let Some(observed) = reader.next_batch().unwrap() {
                for (i, index) in fields.iter().enumerate() {
                    assert_eq!(observed.column(i), batch.column(*index as usize));
                    assert_eq!(
                        observed
                            .schema()
                            .field(i)
                            .metadata()
                            .get("plenora.field_id"),
                        dataset.layers()[0]
                            .contract
                            .schema
                            .field(*index as usize)
                            .metadata()
                            .get("plenora.field_id")
                    );
                }
                rows += observed.num_rows();
            }
            assert_eq!(rows, if outside { 0 } else { 3 });
        }
    }
}

#[test]
fn ogni_proprieta_footer_campo_discorde_rifiutata_per_primaria_e_secondaria() {
    let builder = ParquetRecordBatchReaderBuilder::try_new(File::open(fixture()).unwrap()).unwrap();
    let geo = read_geo_meta(&builder, false).unwrap().unwrap();
    for index in [0, 2] {
        for (key, value) in [
            ("plenora.geometry.types", "polygon"),
            ("plenora.geometry.types_declaration", "mixed"),
            ("plenora.geometry.encoding", "ewkb"),
            ("plenora.geometry.dimensions", "xyzm"),
            ("plenora.geometry.precision", "float32"),
            ("plenora.geometry.spatial_semantics", "geography"),
            ("plenora.geometry.srid", "1234"),
            ("crs", "EPSG:3857"),
            ("plenora.geometry.native.geoparquet.edges", "spherical"),
        ] {
            let mut fields: Vec<Field> = builder
                .schema()
                .fields()
                .iter()
                .map(|f| f.as_ref().clone())
                .collect();
            let mut md = fields[index].metadata().clone();
            md.insert(key, value);
            fields[index] = fields[index].clone().with_metadata(md);
            if key == "plenora.geometry.srid" {
                // The EPSG ID derives a fallback SRID, not a competing declaration.
                crate::multigeometria::verifica_crs(&Schema::new(fields), &geo).unwrap();
                continue;
            }
            let error =
                crate::multigeometria::verifica_crs(&Schema::new(fields), &geo).expect_err(key);
            assert_eq!(
                error.category,
                plenora_io_model::ErrorCategory::Schema,
                "{index}: {key}"
            );
        }
    }
}

#[test]
fn dimensioni_miste_del_footer_non_annullano_il_vincolo_del_campo() {
    let builder = ParquetRecordBatchReaderBuilder::try_new(File::open(fixture()).unwrap()).unwrap();
    for index in [0, 2] {
        let mut geo = read_geo_meta(&builder, false).unwrap().unwrap();
        let name = builder.schema().field(index).name();
        let column = if name == &geo.nome_primaria {
            &mut geo.primaria
        } else {
            geo.secondarie.get_mut(name).unwrap()
        };
        let kind = column.tipi[0].0;
        column.tipi.push((kind, CoordinateDimensions::Xyz));
        let mut fields: Vec<Field> = builder
            .schema()
            .fields()
            .iter()
            .map(|f| f.as_ref().clone())
            .collect();
        let mut md = fields[index].metadata().clone();
        md.insert("plenora.geometry.dimensions", "xy");
        fields[index] = fields[index].clone().with_metadata(md);
        let error = crate::multigeometria::verifica_crs(&Schema::new(fields), &geo).unwrap_err();
        assert_eq!(error.category, plenora_io_model::ErrorCategory::Schema);
    }
}

#[test]
fn unresolved_and_declared_srid_survive_full_reader_completion() {
    let builder = ParquetRecordBatchReaderBuilder::try_new(File::open(fixture()).unwrap()).unwrap();
    let geo = read_geo_meta(&builder, false).unwrap().unwrap();
    for index in [0, 2] {
        for unresolved in [false, true] {
            let mut fields: Vec<Field> = builder
                .schema()
                .fields()
                .iter()
                .map(|f| f.as_ref().clone())
                .collect();
            let mut md = fields[index].metadata().clone();
            md.insert("plenora.geometry.dimensions", "unknown");
            md.insert("plenora.geometry.srid", "1234");
            md.insert("plenora.geometry.native.geoparquet.epoch", "2020");
            if unresolved {
                md.insert("plenora.geometry.types_declaration", "unresolved");
                md.remove("plenora.geometry.types");
            }
            fields[index] = fields[index].clone().with_metadata(md);
            let schema = Schema::new_with_metadata(fields, builder.schema().metadata().clone());
            crate::multigeometria::verifica_crs(&schema, &geo).unwrap();
            let (name, crs) = resolve_geometry_and_crs(&schema, Some(&geo), None).unwrap();
            let schema = retag_schema(&schema, &name, &crs, None);
            let schema = crate::multigeometria::completa_schema(schema, Some(&geo)).unwrap();
            let (contract, _) = contract_from_footer(&schema, name, crs, Some(&geo)).unwrap();
            let geometries = contract.geometries().unwrap();
            let observed = geometries
                .iter()
                .find(|g| g.field_id.0 as usize == index)
                .unwrap();
            assert_eq!(observed.srid, Some(1234));
            assert_eq!(
                observed
                    .native_metadata
                    .get("geoparquet.epoch")
                    .map(String::as_str),
                Some("2020")
            );
            assert_eq!(observed.dimensions, CoordinateDimensions::Xy);
            assert_eq!(
                observed.types_declaration,
                plenora_io_model::contract::TypesDeclaration::Exact
            );
            assert_eq!(
                observed.geometry_types,
                if index == 0 {
                    vec![GeometryType::LineString]
                } else {
                    vec![GeometryType::Point]
                }
            );
            assert_eq!(
                contract
                    .schema
                    .field(index)
                    .metadata()
                    .get("plenora.geometry.srid")
                    .unwrap(),
                "1234"
            );
        }
    }
}

#[test]
fn empty_footer_types_preserve_resolved_field_and_crs() {
    let builder = ParquetRecordBatchReaderBuilder::try_new(File::open(fixture()).unwrap()).unwrap();
    let mut geo = read_geo_meta(&builder, false).unwrap().unwrap();
    geo.primaria.tipi.clear();
    geo.primaria.crs = metadati::Crs::Nullo;
    for column in geo.secondarie.values_mut() {
        column.tipi.clear();
        column.crs = metadati::Crs::Nullo;
    }
    let (declared, _) = leggi(fixture());
    let fields = declared
        .schema
        .fields()
        .iter()
        .map(|f| {
            let mut md = f.metadata().clone();
            md.retain(|k, _| !k.starts_with("plenora.geometry.native.geoparquet."));
            f.as_ref().clone().with_metadata(md)
        })
        .collect::<Vec<_>>();
    let schema = &Schema::new_with_metadata(fields, declared.schema.metadata().clone());
    crate::multigeometria::verifica_crs(schema, &geo).unwrap();
    let (name, crs) = resolve_geometry_and_crs(schema, Some(&geo), None).unwrap();
    let schema = retag_schema(schema, &name, &crs, None);
    let schema = crate::multigeometria::completa_schema(schema, Some(&geo)).unwrap();
    let (contract, _) = contract_from_footer(&schema, name, crs, Some(&geo)).unwrap();
    for g in contract.geometries().unwrap() {
        assert_ne!(g.crs, CrsResolution::Missing);
        assert_eq!(g.dimensions, CoordinateDimensions::Xy);
        assert!(!g.geometry_types.is_empty());
    }
}

#[test]
fn historical_703_geojson_point_unknown_dimensions_is_readable() {
    // Exact 7.0.3 matrix output, SHA256 ba4f871acc239bbe946a205ada66f53737ae65d5c4645fa20d3238e2210b7265.
    let path = fixture().with_file_name("historical-7.0.3-geojson-point.parquet");
    let builder = ParquetRecordBatchReaderBuilder::try_new(File::open(&path).unwrap()).unwrap();
    assert_eq!(
        builder
            .schema()
            .field_with_name("geometry")
            .unwrap()
            .metadata()
            .get("plenora.geometry.dimensions")
            .unwrap(),
        "unknown"
    );
    let (contract, batch) = leggi(path);
    let g = contract.geometry.unwrap();
    assert_eq!(g.dimensions, CoordinateDimensions::Xy);
    assert_eq!(g.geometry_types, [GeometryType::Point]);
    assert_eq!(batch.num_rows(), 1);
}

#[test]
fn unresolved_field_crs_is_completed_by_resolved_footer() {
    let (contract, batch) = leggi(fixture());
    let builder = ParquetRecordBatchReaderBuilder::try_new(File::open(fixture()).unwrap()).unwrap();
    let geo = read_geo_meta(&builder, false).unwrap().unwrap();
    for index in [0, 2] {
        for missing in [false, true] {
            let mut g = contract
                .geometries()
                .unwrap()
                .into_iter()
                .find(|g| g.field_id.0 as usize == index)
                .unwrap();
            let expected = g.crs.clone();
            g.crs = if missing {
                CrsResolution::Missing
            } else {
                CrsResolution::DeclaredButUnresolved(
                    plenora_io_model::crs::RawCrs::from_authority_hint("LOCAL:unresolved".into()),
                )
            };
            g.srid = None;
            g.native_metadata.clear();
            let mut fields: Vec<Field> = contract
                .schema
                .fields()
                .iter()
                .map(|f| f.as_ref().clone())
                .collect();
            let mut md = fields[index].metadata().clone();
            md.remove("crs");
            md.remove("ARROW:extension:metadata");
            md.retain(|key, _| !key.starts_with("plenora.geometry.native."));
            fields[index] = plenora_io_model::geometry::with_geometry_contract_metadata(
                &fields[index].clone().with_metadata(md),
                &g,
            );
            let schema = Arc::new(Schema::new_with_metadata(
                fields,
                contract.schema.metadata().clone(),
            ));
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("unresolved.parquet");
            let mut writer =
                ArrowWriter::try_new(File::create(&path).unwrap(), schema.clone(), None).unwrap();
            writer
                .write(&RecordBatch::try_new(schema, batch.columns().to_vec()).unwrap())
                .unwrap();
            let raw = builder
                .metadata()
                .file_metadata()
                .key_value_metadata()
                .unwrap()
                .iter()
                .find(|kv| kv.key == "geo")
                .unwrap()
                .clone();
            writer.append_key_value_metadata(raw);
            writer.close().unwrap();
            let (observed, values) = leggi(path);
            assert_eq!(values.columns(), batch.columns());
            assert_eq!(
                observed
                    .geometries()
                    .unwrap()
                    .iter()
                    .find(|g| g.field_id.0 as usize == index)
                    .unwrap()
                    .crs,
                expected
            );
            crate::multigeometria::verifica_crs(&observed.schema, &geo).unwrap();
        }
    }
}
