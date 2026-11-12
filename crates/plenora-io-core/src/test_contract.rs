//! Produttore di schemi per le fixture native dei test del core.
use arrow_schema::SchemaRef;
use plenora_io_model::contract::{DataContract, GeometryColumnContract};

pub fn declared(schema: SchemaRef, geometry: Option<GeometryColumnContract>) -> DataContract {
    let schema = match &geometry {
        Some(g) => plenora_io_model::geometry::schema_from_native_fields(&schema, g),
        None => schema,
    };
    DataContract::new(schema, geometry)
}
