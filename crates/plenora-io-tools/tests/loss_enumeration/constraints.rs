//! Representation domains of the native encodings, evaluated before executing IO.
//! A family-level P/T rule does not make every width, ordinate or value writable.
use super::{decode_wkb, Array, DataType, Float64Array, Int64Array, RecordBatch, UInt64Array};
use super::{GeometryType, WkbLimits};
use plenora_io_model::error_table::Situation;

#[allow(clippy::too_many_lines)] // Keep the independent native-domain predicates together.
pub(super) fn rejection(driver: &str, batch: &RecordBatch) -> Option<(Situation, &'static str)> {
    for (field, column) in batch.schema().fields().iter().zip(batch.columns()) {
        if plenora_io_model::geometry::is_geometry_field(field) {
            if matches!(driver, "geojson" | "kml")
                && field
                    .metadata()
                    .get("plenora.geometry.crs_id")
                    .is_some_and(|id| id != "OGC:CRS84")
            {
                return Some((
                    Situation::UnrepresentableWriteType,
                    "fixed CRS requires OGC:CRS84 without reprojection",
                ));
            }

            if driver == "gpkg"
                && field
                    .metadata()
                    .get("plenora.geometry.crs_id")
                    .is_some_and(|id| {
                        id != "OGC:CRS84"
                            && id
                                .split_once(':')
                                .is_none_or(|(_, code)| code.parse::<i32>().is_err())
                    })
            {
                return Some((
                    Situation::UnrepresentableWriteType,
                    "integer CRS authority code required",
                ));
            }
            let values = column
                .as_any()
                .downcast_ref::<arrow_array::BinaryArray>()
                .unwrap();
            for row in 0..values.len() {
                if values.is_null(row) {
                    continue;
                }
                let g = decode_wkb(values.value(row), &WkbLimits::default()).unwrap();
                if matches!(driver, "geojson" | "kml" | "dxf" | "geoparquet")
                    && matches!(
                        g.dimensions,
                        plenora_io_model::contract::CoordinateDimensions::Xym
                            | plenora_io_model::contract::CoordinateDimensions::Xyzm
                    )
                {
                    return Some((
                        Situation::UnrepresentableWriteType,
                        "M ordinate outside native domain",
                    ));
                }
                if driver == "filegdb"
                    && !matches!(
                        g.geometry_type(),
                        GeometryType::Point
                            | GeometryType::MultiPoint
                            | GeometryType::MultiLineString
                            | GeometryType::MultiPolygon
                    )
                {
                    return Some((
                        Situation::UnrepresentableWriteType,
                        "FileGDB requires supported multipart geometry",
                    ));
                }
            }
            continue;
        }
        if driver == "shp" && (field.name().len() > 10 || !field.name().is_ascii()) {
            return Some((
                Situation::UnrepresentableWriteType,
                "DBF field name exceeds native domain",
            ));
        }
        if driver == "filegdb"
            && !matches!(
                field.data_type(),
                DataType::Utf8 | DataType::Int32 | DataType::Float64
            )
        {
            return Some((
                Situation::UnrepresentableWriteType,
                "FileGDB scalar storage type unsupported",
            ));
        }
        for row in 0..column.len() {
            if column.is_null(row) {
                continue;
            }
            let integer = column
                .as_any()
                .downcast_ref::<Int64Array>()
                .map(|a| i128::from(a.value(row)))
                .or_else(|| {
                    column
                        .as_any()
                        .downcast_ref::<UInt64Array>()
                        .map(|a| i128::from(a.value(row)))
                });
            if let Some(integer) = integer {
                if (driver == "gpkg" && integer > i128::from(i64::MAX))
                    || (driver == "xls" && integer.unsigned_abs() > (1_u128 << 53))
                {
                    return Some((
                        Situation::UnrepresentableWriteValue,
                        "integer outside exact native domain",
                    ));
                }
                if driver == "shp" {
                    let text = integer.to_string();
                    let binary64 = text.parse::<f64>().unwrap();
                    if text.len() > 18 || format!("{binary64:.0}") != text {
                        return Some((
                            Situation::UnrepresentableWriteValue,
                            "DBF integer exceeds width or exact binary64 transport",
                        ));
                    }
                }
            }
            if driver == "shp" {
                if let Some(a) = column.as_any().downcast_ref::<Float64Array>() {
                    let value = a.value(row);
                    let encoded = format!("{value:.8}");
                    if encoded.len() > 20
                        || encoded.parse::<f64>().unwrap().to_bits() != value.to_bits()
                    {
                        return Some((
                            Situation::UnrepresentableWriteValue,
                            "DBF number exceeds width 20 / scale 8",
                        ));
                    }
                }
            }
        }
    }
    None
}
