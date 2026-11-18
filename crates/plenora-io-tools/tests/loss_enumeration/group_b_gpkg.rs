//! Minimal native `GeoPackage` construction, independent of the IO writer.
use super::*;
use std::fmt::Write;

pub(super) const fn supports(element: E) -> bool {
    matches!(
        element,
        E::GeometryType
            | E::RingOrientation
            | E::CoordinatePrecision
            | E::SignedZero
            | E::Z
            | E::M
            | E::Dimensions
            | E::NullGeometry
            | E::EmptyGeometry
            | E::EmptyPoint
            | E::Utf8
            | E::EmptyString
            | E::Null
            | E::NotNull
            | E::FieldNames
            | E::FieldOrder
            | E::Floating
            | E::SignedInteger
            | E::Binary
            | E::NumberPrecision
    )
}

#[allow(clippy::too_many_lines)] // Keep native serialization next to its independent schema/value oracle.
pub(super) fn write(path: &Path, source: &RecordBatch, element: E) -> Option<RecordBatch> {
    use rusqlite::types::Value as Sql;
    let geometry_index = source
        .schema()
        .fields()
        .iter()
        .position(|f| plenora_io_model::geometry::is_geometry_field(f))
        .unwrap();
    let geometry = source
        .column(geometry_index)
        .as_any()
        .downcast_ref::<BinaryArray>()
        .unwrap();
    let decoded = (!geometry.is_null(0))
        .then(|| decode_wkb(geometry.value(0), &WkbLimits::default()).unwrap());
    let (kind, z, m) = decoded.as_ref().map_or_else(
        || ("POINT".to_owned(), 0, 0),
        |g| {
            let kind = format!("{:?}", g.geometry_type()).to_uppercase();
            let dims = format!("{:?}", g.dimensions).to_lowercase();
            (
                kind,
                i32::from(dims.contains('z')),
                i32::from(dims.contains('m')),
            )
        },
    );
    let mut fields = vec![Field::new("geometry", DataType::Binary, true)
        .with_metadata([("ARROW:extension:name".to_owned(), "geoarrow.wkb".to_owned())])];
    let mut arrays = vec![source.column(geometry_index).clone()];
    let mut ddl = String::new();
    let mut values = Vec::new();
    for (index, field) in source.schema().fields().iter().enumerate() {
        if index == geometry_index {
            continue;
        }
        let array = source.column(index);
        let (sql_type, native_array, value): (&str, ArrayRef, Sql) = match field.data_type() {
            DataType::Utf8 => {
                let a = array.as_any().downcast_ref::<StringArray>().unwrap();
                (
                    "TEXT",
                    array.clone(),
                    if a.is_null(0) {
                        Sql::Null
                    } else {
                        Sql::Text(a.value(0).to_owned())
                    },
                )
            }
            DataType::Int64 => {
                let a = array.as_any().downcast_ref::<Int64Array>().unwrap();
                ("INTEGER", array.clone(), Sql::Integer(a.value(0)))
            }
            DataType::Float64 => {
                let a = array.as_any().downcast_ref::<Float64Array>().unwrap();
                ("REAL", array.clone(), Sql::Real(a.value(0)))
            }
            DataType::Binary => {
                let a = array.as_any().downcast_ref::<BinaryArray>().unwrap();
                ("BLOB", array.clone(), Sql::Blob(a.value(0).to_vec()))
            }
            // SQL INTEGER/REAL have no separate Int8/Int32/Float32 domain.
            _ => return None,
        };
        write!(
            ddl,
            ",\"{}\" {sql_type}{}",
            field.name(),
            if field.is_nullable() { "" } else { " NOT NULL" }
        )
        .unwrap();
        fields.push(Field::new(
            field.name(),
            native_array.data_type().clone(),
            field.is_nullable(),
        ));
        arrays.push(native_array);
        values.push(value);
    }
    fields.push(Field::new("fid", DataType::Int64, false));
    arrays.push(Arc::new(Int64Array::from(vec![7])));
    let conn = rusqlite::Connection::open(path).unwrap();
    conn.execute_batch("PRAGMA application_id=1196444487; PRAGMA user_version=10300;
        CREATE TABLE gpkg_spatial_ref_sys(srs_name TEXT NOT NULL,srs_id INTEGER PRIMARY KEY,organization TEXT NOT NULL,organization_coordsys_id INTEGER NOT NULL,definition TEXT NOT NULL,description TEXT);
        INSERT INTO gpkg_spatial_ref_sys VALUES('undefined',-1,'NONE',-1,'undefined',NULL),('undefined',0,'NONE',0,'undefined',NULL),('WGS84',4326,'EPSG',4326,'undefined',NULL);
        CREATE TABLE gpkg_contents(table_name TEXT PRIMARY KEY,data_type TEXT NOT NULL,identifier TEXT,description TEXT,last_change DATETIME,min_x DOUBLE,min_y DOUBLE,max_x DOUBLE,max_y DOUBLE,srs_id INTEGER);
        INSERT INTO gpkg_contents VALUES('features','features','features','', '2026-01-01T00:00:00Z',NULL,NULL,NULL,NULL,4326);
        CREATE TABLE gpkg_geometry_columns(table_name TEXT,column_name TEXT,geometry_type_name TEXT,srs_id INTEGER,z TINYINT,m TINYINT);").unwrap();
    conn.execute(
        "INSERT INTO gpkg_geometry_columns VALUES('features','geometry',?1,4326,?2,?3)",
        rusqlite::params![kind, z, m],
    )
    .unwrap();
    let sql = if element == E::FieldOrder {
        format!("CREATE TABLE features(fid INTEGER PRIMARY KEY,geometry BLOB{ddl})")
    } else {
        format!("CREATE TABLE features(geometry BLOB{ddl},fid INTEGER PRIMARY KEY)")
    };
    conn.execute_batch(&sql).unwrap();
    let blob = decoded.as_ref().map_or(Sql::Null, |_| {
        let mut blob = b"GP\0\x01".to_vec();
        blob.extend_from_slice(&4326_i32.to_le_bytes());
        blob.extend_from_slice(geometry.value(0));
        Sql::Blob(blob)
    });
    values.insert(0, blob);
    values.push(Sql::Integer(7));
    let names = fields
        .iter()
        .map(|f| format!("\"{}\"", f.name()))
        .collect::<Vec<_>>()
        .join(",");
    let placeholders = vec!["?"; fields.len()].join(",");
    conn.execute(
        &format!("INSERT INTO features({names}) VALUES({placeholders})"),
        rusqlite::params_from_iter(values),
    )
    .unwrap();
    let expected = RecordBatch::try_new(Arc::new(Schema::new(fields)), arrays).unwrap();
    Some(if element == E::FieldOrder {
        expected.project(&[2, 0, 1]).unwrap()
    } else {
        expected
    })
}
