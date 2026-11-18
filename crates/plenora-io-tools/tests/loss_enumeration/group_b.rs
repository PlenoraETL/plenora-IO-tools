//! Independent native writers for the second qualification group.
//! No fixture is built by a plenora writer; expectations precede the read.
use super::*;
#[path = "group_b_dxf.rs"]
mod dxf;
#[path = "group_b_gpkg.rs"]
mod gpkg;
#[path = "group_b_read.rs"]
mod read;
#[path = "group_b_shp.rs"]
mod shp;

pub(super) fn observe_additional(
    driver: &str,
    direction: D,
    before: &RecordBatch,
    after: &RecordBatch,
    observed: &mut BTreeMap<String, u64>,
) {
    if direction != D::Write || !matches!(driver, "shp" | "dxf" | "filegdb" | "gpkg" | "geoparquet")
    {
        return;
    }
    for field in before
        .schema()
        .fields()
        .iter()
        .filter(|f| plenora_io_model::geometry::is_geometry_field(f))
    {
        if let Some(srid) = field.metadata().get("plenora.geometry.srid") {
            let target = after.schema();
            let received = target
                .fields()
                .iter()
                .find(|f| plenora_io_model::geometry::is_geometry_field(f))
                .and_then(|f| f.metadata().get("plenora.geometry.srid"));
            if received != Some(srid) {
                *observed
                    .entry(
                        if received.is_some() {
                            "srid_not_preserved_derived"
                        } else {
                            "srid_not_preserved_absent"
                        }
                        .to_owned(),
                    )
                    .or_default() += 1;
            }
        }
    }
}

pub(super) fn generate_additional(driver: &str, element: E) -> Option<Vec<RecordBatch>> {
    if !matches!(driver, "shp" | "dxf" | "filegdb" | "gpkg" | "geoparquet") {
        return None;
    }
    let base = geometry_batch(WkbValue::Point(coordinate(1.0, 2.0)), GeometryType::Point);
    Some(match element {
        E::Srid => vec![metadata(&base, "plenora.geometry.srid", "4326", false)],
        E::CrsDefinition | E::DefinitionFormat => {
            let definition = r#"GEOGCS["WGS 84",DATUM["WGS_1984",SPHEROID["WGS 84",6378137,298.257223563]],PRIMEM["Greenwich",0],UNIT["degree",0.0174532925199433],AUTHORITY["EPSG","4326"]]"#;
            let b = metadata(&base, "plenora.geometry.crs_id", "EPSG:4326", false);
            let b = metadata(&b, "plenora.geometry.crs_definition", definition, false);
            vec![metadata(
                &b,
                "plenora.geometry.crs_definition_format",
                "wkt",
                false,
            )]
        }
        _ => return None,
    })
}

#[allow(clippy::too_many_lines)] // One adapter dispatch; all failures are collected before the matrix guard.
pub(super) fn exercise(
    driver: &str,
    native: NativeDriver,
    element: E,
    root: &Path,
    row: &mut Value,
) -> bool {
    if read::exercise(driver, native, element, root, row) {
        return true;
    }
    let supported = match driver {
        "geoparquet" => true,
        "gpkg" => gpkg::supports(element),
        "shp" => shp::supports(element),
        "dxf" => dxf::supports(element),
        "filegdb" => matches!(element, E::Utf8 | E::Null | E::Floating | E::Z),
        _ => false,
    };
    if !supported {
        return false;
    }
    if driver == "filegdb" && std::env::var_os("PLENORA_ENUM_PYTHON").is_none() {
        row["status"] = json!("missing-native-witness");
        row["reason"] =
            json!("native FileGDB generator requires PLENORA_ENUM_PYTHON and GDAL runtime");
        return true;
    }
    let Ok(fixtures) =
        generate(element).or_else(|reason| generate_additional(driver, element).ok_or(reason))
    else {
        return false;
    };
    let mut failures = Vec::new();
    let mut exercised = 0;
    for (variant, source) in fixtures.iter().enumerate() {
        let dir = tempfile::tempdir_in(root).unwrap();
        let mut input = dir.path().join(format!("native.{}", extension(driver)));
        let output = dir.path().join("result.arrow");
        // The low-level Apache Parquet writer preserves the independently
        // constructed Arrow schema. It does not call the IO format driver.
        let expected;
        if driver == "filegdb" {
            let seed = dir.path().join("seed.gpkg");
            let Some(batch) = gpkg::write(&seed, source, element) else {
                row["native_domain"][variant.to_string()] =
                    json!("SQL seed uses native INTEGER/REAL, not Arrow narrow numeric types");
                continue;
            };
            let python = std::env::var("PLENORA_ENUM_PYTHON")
                .expect("set PLENORA_ENUM_PYTHON to the qualification venv Python");
            let result = Command::new(python)
                .arg(
                    Path::new(env!("CARGO_MANIFEST_DIR"))
                        .join("tests/loss_enumeration/group_b_filegdb.py"),
                )
                .arg(&seed)
                .arg(&input)
                .output()
                .unwrap();
            assert!(
                result.status.success(),
                "{}",
                String::from_utf8_lossy(&result.stderr)
            );
            expected = batch
                .project(&(0..batch.num_columns() - 1).collect::<Vec<_>>())
                .unwrap();
        } else if driver == "shp" {
            expected = shp::write(&input, source);
            input = input.join("features.shp");
        } else if driver == "dxf" {
            expected = dxf::write(&input, source);
        } else if driver == "gpkg" {
            let Some(batch) = gpkg::write(&input, source, element) else {
                row["native_domain"][variant.to_string()] =
                    json!("SQL uses native INTEGER/REAL, not Arrow narrow numeric types");
                continue;
            };
            expected = batch;
        } else {
            let mut writer = parquet::arrow::ArrowWriter::try_new(
                std::fs::File::create(&input).unwrap(),
                source.schema(),
                None,
            )
            .unwrap();
            writer.write(source).unwrap();
            writer.close().unwrap();
            expected = source.clone();
        }
        exercised += 1;
        let (ok, report) = run(&input, &output, driver, "ipc");
        if !ok {
            failures.push(json!({"variant":variant,"native_read_failed":report}));
            continue;
        }
        let after = native_batch(&input, &output, driver);
        let differences = compare(&expected, &after);
        let mut observed = BTreeMap::<String, u64>::new();
        for difference in &differences {
            let count = observed
                .entry(category(native, D::Read, difference))
                .or_default();
            if driver == "gpkg" && difference.element == E::FieldOrder {
                *count = 1;
            } else {
                *count += 1;
            }
        }
        let declared = report_categories(&report, "read_loss");
        if observed != declared {
            failures.push(json!({"variant":variant,"observed":observed,"declared":declared,"differences":format!("{differences:?}"),"fixture":dir.keep()}));
        }
    }
    assert!(
        exercised > 0,
        "native adapter produced no witnesses for {driver}/{element:?}"
    );
    row["variants_exercised"] = json!(exercised);
    row["status"] = json!(if failures.is_empty() {
        "passed"
    } else {
        "failed"
    });
    row["witness"] = json!(format!(
        "direct native {driver} construction; independent source schema and values; no IO writer"
    ));
    row["failures"] = json!(failures);
    true
}
