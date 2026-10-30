use super::*;

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
