//! Il rifiuto dei sink monogeometria attraversa la CLI e non pubblica output.
use std::path::PathBuf;
use std::process::Command;

#[test]
fn sink_monogeometria_rifiutano_con_categoria_exit_e_nessun_output() {
    let sorgente = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../driver-geoparquet/tests/fixtures/multigeometria-pyarrow.parquet");
    let dir = tempfile::tempdir().unwrap();
    for (formato, suffisso) in [
        ("gpkg", "gpkg"),
        ("geojson", "geojson"),
        ("shp", "shp.d"),
        ("dxf", "dxf"),
        ("csv", "csv"),
        ("xls", "xlsx"),
        ("kml", "kml"),
    ] {
        let uscita = dir.path().join(format!("uscita.{suffisso}"));
        let output = Command::new(env!("CARGO_BIN_EXE_plenora-io"))
            .arg("convert")
            .arg(&sorgente)
            .arg(&uscita)
            .args(["--from", "geoparquet", "--to", formato, "--format", "json"])
            .output()
            .unwrap();
        assert_eq!(
            output.status.code(),
            Some(3),
            "{suffisso}: {}",
            String::from_utf8_lossy(&output.stdout)
        );
        assert!(output.stderr.is_empty());
        let busta: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(busta["error"]["category"], "unsupported", "{busta}");
        assert_eq!(busta["error"]["code"], "UNSUPPORTED", "{busta}");
        assert!(!uscita.exists());
    }
}

#[test]
fn cardinalita_enumera_tutti_i_driver_e_il_profilo_filegdb() {
    use plenora_io_core::geometry_cardinality::{GeometryCardinality, DRIVERS};
    let catalog = plenora_io_tools::cmd_catalog();
    let drivers = catalog["drivers"].as_array().unwrap();
    let actual: std::collections::BTreeSet<_> =
        drivers.iter().map(|d| d["id"].as_str().unwrap()).collect();
    let declared: std::collections::BTreeSet<_> = DRIVERS.iter().map(|(id, _)| *id).collect();
    assert_eq!(actual, declared);
    let expected = [
        ("ipc", GeometryCardinality::Multiple),
        ("geoparquet", GeometryCardinality::Multiple),
        ("gpkg", GeometryCardinality::Single),
        ("shp", GeometryCardinality::Single),
        ("csv", GeometryCardinality::Single),
        ("xls", GeometryCardinality::Single),
        ("filegdb", GeometryCardinality::Single),
        ("geojson", GeometryCardinality::Single),
        ("dxf", GeometryCardinality::Single),
        ("kml", GeometryCardinality::Single),
    ];
    assert_eq!(actual, expected.iter().map(|(id, _)| *id).collect());
    for (id, cardinality) in &expected {
        let driver = drivers.iter().find(|d| d["id"] == *id).unwrap();
        assert_eq!(
            driver["geometry_cardinality"],
            if *cardinality == GeometryCardinality::Multiple {
                "multiple"
            } else {
                "single"
            }
        );
    }
    assert!(!plenora_io_core::geometry_cardinality::supports_multiple(
        "filegdb"
    ));
}

// Visita espressioni di chiamata nel corpo della funzione attesa: commenti,
// import e chiamate in altre funzioni non soddisfano la guardia.
fn calls(source: &str, function: &str, callee: &str) -> bool {
    use syn::visit::Visit;
    struct Calls<'a> {
        callee: &'a str,
        found: bool,
    }
    impl<'ast> Visit<'ast> for Calls<'_> {
        fn visit_expr_call(&mut self, node: &'ast syn::ExprCall) {
            if let syn::Expr::Path(path) = node.func.as_ref() {
                let name = path
                    .path
                    .segments
                    .iter()
                    .map(|s| s.ident.to_string())
                    .collect::<Vec<_>>()
                    .join("::");
                self.found |= name == self.callee || name.ends_with(&format!("::{}", self.callee));
            }
            syn::visit::visit_expr_call(self, node);
        }
    }
    struct Functions<'a> {
        function: &'a str,
        calls: Calls<'a>,
    }
    impl<'ast> Visit<'ast> for Functions<'_> {
        fn visit_item_fn(&mut self, node: &'ast syn::ItemFn) {
            if node.sig.ident == self.function {
                self.calls.visit_block(&node.block);
            }
        }
        fn visit_impl_item_fn(&mut self, node: &'ast syn::ImplItemFn) {
            if node.sig.ident == self.function {
                self.calls.visit_block(&node.block);
            }
        }
    }
    let mut visitor = Functions {
        function,
        calls: Calls {
            callee,
            found: false,
        },
    };
    visitor.visit_file(&syn::parse_file(source).unwrap());
    visitor.calls.found
}

#[test]
fn enumeration_guards_driver_and_surface_routes() {
    use plenora_io_core::geometry_cardinality::DRIVERS;
    let adoption: serde_json::Value =
        serde_json::from_str(include_str!("../../../contracts/adoption-source.json")).unwrap();
    let expected_surfaces: std::collections::BTreeSet<_> = ["rust", "cli", "python_sdk", "runtime"]
        .into_iter()
        .collect();
    assert_eq!(
        adoption["superfici"]
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect::<std::collections::BTreeSet<_>>(),
        expected_surfaces
    );
    assert_eq!(
        plenora_io_core::geometry_cardinality::ARROW_SURFACES
            .iter()
            .map(|(id, _, _)| *id)
            .collect::<std::collections::BTreeSet<_>>(),
        expected_surfaces
    );

    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let registered: std::collections::BTreeSet<_> = DRIVERS
        .iter()
        .map(|(id, _)| format!("driver-{id}"))
        .collect();
    let on_disk: std::collections::BTreeSet<_> = std::fs::read_dir(root.join("crates"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with("driver-") && n != "driver-common")
        .collect();
    assert_eq!(registered, on_disk);
    for name in &registered {
        let source =
            std::fs::read_to_string(root.join("crates").join(name).join("src/lib.rs")).unwrap();
        assert!(calls(&source, "create", "validate_write"), "{name}: writer");
        assert!(calls(&source, "open", "with_read_budget"), "{name}: reader");
    }
    for (path, function, callee) in [
        (
            "plenora-io-model/src/primary.rs",
            "validate_contract",
            "reconcile_schema_contract",
        ),
        (
            "plenora-io-model/src/contract.rs",
            "geometries",
            "primary::reconcile_schema_contract",
        ),
        (
            "plenora-io-core/src/capabilities.rs",
            "validate_write",
            "primary::validate_contract",
        ),
        (
            "plenora-io-core/src/request.rs",
            "project_layer_contract",
            "primary::validate_contract",
        ),
        ("plenora-io-tools/src/operazioni.rs", "read", "cmd_read"),
        ("plenora-io-tools/src/operazioni.rs", "write", "cmd_write"),
        (
            "plenora-io-tools/src/operazioni.rs",
            "convert",
            "cmd_convert",
        ),
        (
            "plenora-io-tools/src/lib.rs",
            "cmd_write",
            "piano_di_scrittura",
        ),
        (
            "plenora-io-tools/src/lib.rs",
            "cmd_convert",
            "piano_di_scrittura",
        ),
        (
            "plenora-io-tools/src/runtime.rs",
            "consegna_in_flusso",
            "operazioni::read",
        ),
    ] {
        let source = std::fs::read_to_string(root.join("crates").join(path)).unwrap();
        assert!(
            calls(&source, function, callee),
            "{path}:{function} -> {callee}"
        );
    }
    assert!(!calls(
        "fn create() { /* validate_write() */ } fn other() { validate_write(); }",
        "create",
        "validate_write"
    ));
}

#[test]
fn filegdb_profile_rejects_multi_before_backend_or_publication() {
    use plenora_io_core::{
        FormatDriver, ReadOptions, Sink, Source, WriteLayer, WriteOptions, WritePlan,
    };
    use plenora_io_model::budget::PipelineBudget;
    let source = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../driver-geoparquet/tests/fixtures/multigeometria-pyarrow.parquet");
    let read =
        ReadOptions::from_read_parts(PipelineBudget::builder().build().unwrap().into_read_parts());
    let dataset = driver_geoparquet::GeoParquetDriver
        .open(Source::Path(source), read)
        .unwrap();
    let plan = WritePlan {
        layers: vec![WriteLayer {
            name: "l".into(),
            contract: dataset.layers()[0].contract.clone(),
        }],
    };
    let opts = WriteOptions::from_write_parts(
        PipelineBudget::builder()
            .build()
            .unwrap()
            .into_write_parts(),
    );
    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("rejected.gdb");
    let error = driver_filegdb::FileGdbDriver
        .create(Sink::Path(dest.clone()), &plan, &opts)
        .err()
        .unwrap();
    assert_eq!(error.category, plenora_io_model::ErrorCategory::Unsupported);
    assert_eq!(error.phase, plenora_io_model::ErrorPhase::Validate);
    assert!(error.message.contains("colonne geometriche"));
    assert!(!dest.exists());
}

#[test]
fn reconciliation_callers_are_enumerated_structurally() {
    use syn::visit::Visit;
    struct Caller {
        function: String,
        found: std::collections::BTreeSet<String>,
    }
    impl<'ast> Visit<'ast> for Caller {
        fn visit_item_fn(&mut self, node: &'ast syn::ItemFn) {
            self.function = node.sig.ident.to_string();
            syn::visit::visit_item_fn(self, node);
        }
        fn visit_impl_item_fn(&mut self, node: &'ast syn::ImplItemFn) {
            self.function = node.sig.ident.to_string();
            syn::visit::visit_impl_item_fn(self, node);
        }
        fn visit_expr_call(&mut self, node: &'ast syn::ExprCall) {
            if let syn::Expr::Path(path) = node.func.as_ref() {
                if path.path.segments.last().is_some_and(|s| {
                    s.ident == "validate_contract" || s.ident == "reconcile_schema_contract"
                }) {
                    self.found.insert(self.function.clone());
                }
            }
            syn::visit::visit_expr_call(self, node);
        }
    }
    fn scan(
        path: &std::path::Path,
        root: &std::path::Path,
        actual: &mut std::collections::BTreeSet<(String, String)>,
    ) {
        for entry in std::fs::read_dir(path).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                scan(&path, root, actual);
            } else if path.extension().is_some_and(|e| e == "rs") {
                let source = std::fs::read_to_string(&path).unwrap();
                let mut visitor = Caller {
                    function: String::new(),
                    found: std::collections::BTreeSet::new(),
                };
                visitor.visit_file(&syn::parse_file(&source).unwrap());
                for function in visitor.found {
                    actual.insert((
                        path.strip_prefix(root)
                            .unwrap()
                            .to_string_lossy()
                            .replace('\\', "/"),
                        function,
                    ));
                }
            }
        }
    }
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..");
    let mut actual = std::collections::BTreeSet::new();
    for entry in std::fs::read_dir(&root).unwrap() {
        let source = entry.unwrap().path().join("src");
        if source.is_dir() {
            scan(&source, &root, &mut actual);
        }
    }
    let expected = [
        ("plenora-io-model/src/primary.rs", "validate_contract"),
        ("plenora-io-model/src/contract.rs", "geometries"),
        ("plenora-io-model/src/contract.rs", "from_schema"),
        ("plenora-io-core/src/capabilities.rs", "validate_write"),
        ("plenora-io-core/src/request.rs", "project_layer_contract"),
        (
            "plenora-io-core/src/driver/reader_adapters.rs",
            "with_read_budget",
        ),
        ("plenora-io-core/src/ewkb.rs", "pianifica"),
        ("driver-gpkg/src/lib.rs", "open"),
        ("driver-gpkg/src/lib.rs", "project_gpkg_layer"),
        ("driver-geoparquet/src/lib.rs", "contract_from_footer"),
    ]
    .into_iter()
    .map(|(p, f)| (p.to_owned(), f.to_owned()))
    .collect();
    assert_eq!(actual, expected);
}
