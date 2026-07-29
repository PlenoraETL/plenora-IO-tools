use arrow_array::{
    Array, ArrayRef, BinaryArray, Int64Array, LargeBinaryArray, RecordBatch, StringArray,
};
use arrow_ipc::reader::FileReader;
use arrow_schema::{DataType, Field, Schema, SchemaRef};
use driver_ipc::IpcDriver;
use plenora_database_core::ewkb::inspect_ewkb_detailed;
use plenora_database_core::geometry::{
    AxisOrder, CoordinatePrecision, CrsDefinitionFormat, CrsResolution, DeclaredCrs, FieldId,
    GeometryContract, GeometryType, ResolvedCrs, SpatialSemantics, TypesDeclaration,
};
use plenora_database_core::protocol;
use plenora_io_core::{FormatDriver, Sink, WriteLayer, WriteOptions, WritePlan};
use plenora_io_model::contract::{
    CoordinateDimensions as IoDimensions, DataContract as IoDataContract, FieldId as IoFieldId,
    GeometryColumnContract as IoGeometryContract, GeometryType as IoGeometryType,
};
use plenora_io_model::crs::{CrsKind as IoCrsKind, ResolvedCrs as IoResolvedCrs};
use plenora_io_model::wkb::{encode_wkb, WkbCoordinate, WkbFlavor, WkbGeometry, WkbValue};
use serde::de::DeserializeOwned;
use serde_json::json;
use std::error::Error;
use std::fs::File;
use std::path::PathBuf;
use std::sync::Arc;

fn required<'a>(field: &'a Field, key: &str) -> Result<&'a str, Box<dyn Error>> {
    field
        .metadata()
        .get(key)
        .map(String::as_str)
        .ok_or_else(|| format!("metadato obbligatorio assente: {key}").into())
}

fn parse_wire<T: DeserializeOwned>(value: &str, key: &str) -> Result<T, Box<dyn Error>> {
    serde_json::from_value(serde_json::Value::String(value.to_owned()))
        .map_err(|_| format!("valore non canonico per {key}: {value}").into())
}

fn optional_wire<T: DeserializeOwned>(
    field: &Field,
    key: &str,
) -> Result<Option<T>, Box<dyn Error>> {
    field
        .metadata()
        .get(key)
        .map(|value| parse_wire(value, key))
        .transpose()
}

fn parse_types(field: &Field) -> Result<Vec<GeometryType>, Box<dyn Error>> {
    field
        .metadata()
        .get(protocol::GEOMETRY_TYPES)
        .map_or(Ok(Vec::new()), |values| {
            values
                .split(',')
                .map(|value| parse_wire(value, protocol::GEOMETRY_TYPES))
                .collect()
        })
}

fn parse_crs(field: &Field) -> Result<CrsResolution, Box<dyn Error>> {
    let metadata = field.metadata();
    let resolution = required(field, protocol::GEOMETRY_CRS_RESOLUTION)?;
    let id = metadata.get(protocol::GEOMETRY_CRS_ID).cloned();
    let srid = metadata
        .get(protocol::GEOMETRY_SRID)
        .map(|value| value.parse::<u32>())
        .transpose()?;
    let definition = metadata.get(protocol::GEOMETRY_CRS_DEFINITION).cloned();
    let definition_format =
        optional_wire::<CrsDefinitionFormat>(field, protocol::GEOMETRY_CRS_DEFINITION_FORMAT)?;
    let axis_order = optional_wire::<AxisOrder>(field, protocol::GEOMETRY_AXIS_ORDER)?;

    if definition.is_some() != definition_format.is_some() {
        return Err("definizione CRS e formato non sono una coppia".into());
    }
    match resolution {
        "resolved" => Ok(CrsResolution::Resolved(ResolvedCrs {
            id,
            srid,
            definition,
            definition_format,
            axis_order: axis_order.ok_or("axis_order assente per CRS resolved")?,
        })),
        "declared_unresolved" => Ok(CrsResolution::DeclaredUnresolved(DeclaredCrs {
            id,
            srid,
            definition,
            definition_format,
            axis_order: axis_order.ok_or("axis_order assente per CRS dichiarato")?,
        })),
        "missing" => {
            if id.is_some()
                || srid.is_some()
                || definition.is_some()
                || definition_format.is_some()
                || axis_order.is_some()
            {
                return Err("CRS missing accompagnato da metadati CRS".into());
            }
            Ok(CrsResolution::Missing)
        }
        _ => Err(format!("crs_resolution non canonica: {resolution}").into()),
    }
}

fn contract_from_field(field: &Field) -> Result<GeometryContract, Box<dyn Error>> {
    Ok(GeometryContract {
        field_id: FieldId(required(field, protocol::FIELD_ID)?.parse()?),
        encoding: parse_wire(
            required(field, protocol::GEOMETRY_ENCODING)?,
            protocol::GEOMETRY_ENCODING,
        )?,
        dimensions: parse_wire(
            required(field, protocol::GEOMETRY_DIMENSIONS)?,
            protocol::GEOMETRY_DIMENSIONS,
        )?,
        nullable: field.is_nullable(),
        types_declaration: parse_wire(
            required(field, protocol::GEOMETRY_TYPES_DECLARATION)?,
            protocol::GEOMETRY_TYPES_DECLARATION,
        )?,
        geometry_types: parse_types(field)?,
        crs: parse_crs(field)?,
        spatial_semantics: optional_wire::<SpatialSemantics>(
            field,
            protocol::GEOMETRY_SPATIAL_SEMANTICS,
        )?,
        precision: optional_wire::<CoordinatePrecision>(field, protocol::GEOMETRY_PRECISION)?,
    })
}

fn inspect_cell(bytes: &[u8], contract: &GeometryContract) -> Result<(), Box<dyn Error>> {
    let inspected = inspect_ewkb_detailed(bytes, 10_000_000, 64)?;
    if inspected.root.dimensions_label()
        != serde_json::to_value(contract.dimensions)?
            .as_str()
            .ok_or("dimensioni non serializzabili")?
    {
        return Err("dimensioni WKB divergenti dal contratto".into());
    }
    let observed_type = inspected
        .root
        .geometry_type_name()
        .ok_or("tipo WKB non riconosciuto dal bordo database")?
        .to_ascii_lowercase();
    let declared = contract
        .geometry_types
        .iter()
        .map(|value| {
            serde_json::to_value(value)
                .ok()
                .and_then(|value| value.as_str().map(str::to_owned))
        })
        .collect::<Option<Vec<_>>>()
        .ok_or("tipi dichiarati non serializzabili")?;
    if contract.types_declaration == TypesDeclaration::Exact && !declared.contains(&observed_type) {
        return Err(format!("tipo WKB {observed_type} fuori dalla dichiarazione exact").into());
    }
    Ok(())
}

fn generate_io_fixture(path: PathBuf) -> Result<(), Box<dyn Error>> {
    let geometry_field = Field::new("geometry", DataType::Binary, false).with_metadata(
        std::collections::HashMap::from([
            (
                plenora_io_model::geometry::ARROW_EXTENSION_NAME_KEY.to_owned(),
                plenora_io_model::geometry::GEOARROW_WKB_EXTENSION.to_owned(),
            ),
            (
                plenora_io_model::geometry::GEO_METADATA_KEY.to_owned(),
                r#"{"crs":"EPSG:4326","dimensions":"xyz"}"#.to_owned(),
            ),
        ]),
    );
    let schema: SchemaRef = Arc::new(Schema::new(vec![
        geometry_field,
        Field::new("id", DataType::Int64, false),
        Field::new("name", DataType::Utf8, false),
    ]));
    let mut geometry = IoGeometryContract::wkb_passthrough(
        IoFieldId(0),
        "geometry",
        IoResolvedCrs::new(Some("EPSG:4326".to_owned()), IoCrsKind::Geographic, None),
        false,
    );
    geometry.dimensions = IoDimensions::Xyz;
    geometry.srid = Some(4326);
    geometry.set_exact_geometry_types(vec![IoGeometryType::Point]);
    let plan = WritePlan {
        layers: vec![WriteLayer {
            name: "chain".to_owned(),
            contract: IoDataContract {
                schema: schema.clone(),
                geometry: Some(geometry),
            },
        }],
    };
    let payloads = [(9.0, 45.0, 100.0), (9.1, 45.1, 110.0), (9.2, 45.2, 120.0)]
        .into_iter()
        .map(|(x, y, z)| {
            encode_wkb(
                &WkbGeometry {
                    value: WkbValue::Point(WkbCoordinate {
                        x,
                        y,
                        z: Some(z),
                        m: None,
                    }),
                    dimensions: IoDimensions::Xyz,
                    srid: None,
                },
                WkbFlavor::Iso,
            )
        })
        .collect::<Result<Vec<_>, _>>()?;
    let batch = RecordBatch::try_new(
        schema,
        vec![
            Arc::new(BinaryArray::from_iter_values(
                payloads.iter().map(Vec::as_slice),
            )) as ArrayRef,
            Arc::new(Int64Array::from(vec![0, 1, 2])) as ArrayRef,
            Arc::new(StringArray::from(vec!["discard", "alpha", "bravo"])) as ArrayRef,
        ],
    )?;
    let mut writer = IpcDriver.create(Sink::Path(path), &plan, &WriteOptions::default())?;
    writer.write(&batch)?;
    writer.finish()?;
    Ok(())
}

fn main() -> Result<(), Box<dyn Error>> {
    let mut arguments = std::env::args_os().skip(1);
    let first = arguments
        .next()
        .ok_or("uso: oracle [generate] OUTPUT.arrow")?;
    if first == "generate" {
        let path = arguments
            .next()
            .map(PathBuf::from)
            .ok_or("uso: oracle generate INPUT.arrow")?;
        if arguments.next().is_some() {
            return Err("troppi argomenti per generate".into());
        }
        generate_io_fixture(path)?;
        return Ok(());
    }
    let path = PathBuf::from(first);
    if arguments.next().is_some() {
        return Err("troppi argomenti per oracle".into());
    }
    let mut reader = FileReader::try_new(File::open(&path)?, None)?;
    let schema = reader.schema();
    if schema
        .metadata()
        .get(protocol::CONTRACT_VERSION_KEY)
        .map(String::as_str)
        != Some(protocol::CONTRACT_VERSION)
    {
        return Err("contract.version assente o incompatibile".into());
    }
    let (geometry_index, geometry_field) = schema
        .fields()
        .iter()
        .enumerate()
        .find(|(_, field)| {
            field
                .metadata()
                .get(protocol::GEOARROW_EXTENSION_NAME)
                .is_some_and(|value| value == "geoarrow.wkb")
        })
        .ok_or("colonna GeoArrow-WKB assente")?;
    let contract = contract_from_field(geometry_field)?;
    contract.validate()?;

    let mut rows = 0_usize;
    let mut geometries = 0_usize;
    for batch in &mut reader {
        let batch = batch?;
        rows += batch.num_rows();
        let column = batch.column(geometry_index);
        if let Some(values) = column.as_any().downcast_ref::<BinaryArray>() {
            for index in 0..values.len() {
                if !values.is_null(index) {
                    inspect_cell(values.value(index), &contract)?;
                    geometries += 1;
                }
            }
        } else if let Some(values) = column.as_any().downcast_ref::<LargeBinaryArray>() {
            for index in 0..values.len() {
                if !values.is_null(index) {
                    inspect_cell(values.value(index), &contract)?;
                    geometries += 1;
                }
            }
        } else {
            return Err("colonna geometria non Binary/LargeBinary".into());
        }
    }
    if rows != 2 || geometries != 2 {
        return Err(format!("cardinalità inattesa: rows={rows}, geometries={geometries}").into());
    }
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "status": "pass",
            "chain": ["plenora-IO-tools", "plenora-data-tools", "plenora-database-tools"],
            "rows": rows,
            "geometries_checked_by_database_ewkb": geometries,
            "contract": contract
        }))?
    );
    Ok(())
}
