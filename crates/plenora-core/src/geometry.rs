//! Convenzione della colonna geometria nel bordo Arrow (GeoArrow-WKB).

/// Nome dell'estensione GeoArrow per una colonna geometria WKB.
pub const GEOARROW_WKB_EXTENSION: &str = "geoarrow.wkb";
/// Chiave dei metadati di estensione a livello di campo Arrow.
pub const ARROW_EXTENSION_NAME_KEY: &str = "ARROW:extension:name";
/// Chiave del metadato `geo` (a livello di campo/schema) che porta il CRS.
pub const GEO_METADATA_KEY: &str = "geo";
/// Chiave `crs` (PROJJSON) dentro il metadato `geo`.
pub const GEO_CRS_KEY: &str = "crs";

/// True se il campo Arrow è marcato come colonna geometria `geoarrow.wkb`.
pub fn is_geometry_field(field: &arrow_schema::Field) -> bool {
    field
        .metadata()
        .get(ARROW_EXTENSION_NAME_KEY)
        .map(|v| v == GEOARROW_WKB_EXTENSION)
        .unwrap_or(false)
}
