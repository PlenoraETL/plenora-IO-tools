//! Cardinalita' geometrica dichiarata dai driver di formato.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GeometryCardinality {
    Single,
    Multiple,
}

pub const DRIVERS: &[(&str, GeometryCardinality)] = &[
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

#[must_use]
pub fn supports_multiple(driver: &str) -> bool {
    DRIVERS
        .iter()
        .any(|(id, cardinality)| *id == driver && *cardinality == GeometryCardinality::Multiple)
}

/// Superfici pubbliche e percorso verso il bordo Arrow condiviso.
pub const ARROW_SURFACES: &[(&str, &str, &str)] = &[
    (
        "rust",
        "crates/plenora-io-tools/src/operazioni.rs",
        "cmd_read",
    ),
    (
        "cli",
        "crates/plenora-io-tools/src/lib.rs",
        "piano_di_scrittura",
    ),
    (
        "python_sdk",
        "sdk/python/src/plenora_io/_arrow.py",
        "scrivi_ipc",
    ),
    (
        "runtime",
        "crates/plenora-io-tools/src/runtime.rs",
        "operazioni::",
    ),
];
