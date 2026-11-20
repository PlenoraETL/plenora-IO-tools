//! Shapefile/DBF bytes authored directly from semantic fixtures.
use super::*;

pub(super) const fn supports(element: E) -> bool {
    matches!(
        element,
        E::CoordinatePrecision
            | E::SignedZero
            | E::Z
            | E::M
            | E::Dimensions
            | E::NullGeometry
            | E::Utf8
            | E::Boolean
            | E::Floating
            | E::NumberPrecision
            | E::EmptyString
            | E::Null
    )
}

#[allow(clippy::too_many_lines)] // Keep native serialization next to its independent schema/value oracle.
pub(super) fn write(path: &Path, source: &RecordBatch) -> RecordBatch {
    let geometry = source
        .column(0)
        .as_any()
        .downcast_ref::<BinaryArray>()
        .unwrap();
    let point = if geometry.is_null(0) {
        None
    } else {
        let g = decode_wkb(geometry.value(0), &WkbLimits::default()).unwrap();
        let WkbValue::Point(c) = g.value else {
            panic!("point fixture required");
        };
        Some(c)
    };
    let shape_type: i32 = point.as_ref().map_or(0, |c| {
        if c.z.is_some() {
            11
        } else if c.m.is_some() {
            21
        } else {
            1
        }
    });
    let mut record = shape_type.to_le_bytes().to_vec();
    let shape_type = if shape_type == 0 { 1 } else { shape_type };
    if let Some(c) = &point {
        for v in [Some(c.x), Some(c.y), c.z, c.m].into_iter().flatten() {
            record.extend(v.to_le_bytes());
        }
        if c.z.is_some() && c.m.is_none() {
            record.extend((-1e39_f64).to_le_bytes());
        }
    }
    let words = i32::try_from(record.len() / 2).unwrap();
    let header = |length: i32| {
        let mut bytes = Vec::new();
        for v in [9994_i32, 0, 0, 0, 0, 0, length] {
            bytes.extend(v.to_be_bytes());
        }
        bytes.extend(1000_i32.to_le_bytes());
        bytes.extend(shape_type.to_le_bytes());
        let c = point.unwrap_or(coordinate(0.0, 0.0));
        for v in [c.x, c.y, c.x, c.y, 0.0, 0.0, 0.0, 0.0] {
            bytes.extend(v.to_le_bytes());
        }
        bytes
    };
    let mut shp = header(54 + words);
    shp.extend(1_i32.to_be_bytes());
    shp.extend(words.to_be_bytes());
    shp.extend(record);
    let mut shx = header(54);
    shx.extend(50_i32.to_be_bytes());
    shx.extend(words.to_be_bytes());
    let mut descriptors = Vec::new();
    let mut row = vec![b' '];
    let mut fields = vec![Field::new("geometry", DataType::Binary, true)
        .with_metadata([("ARROW:extension:name".to_owned(), "geoarrow.wkb".to_owned())])];
    let mut columns = vec![source.column(0).clone()];
    for (i, field) in source.schema().fields().iter().enumerate().skip(1) {
        let array = source.column(i);
        let (kind, bytes, expected): (u8, Vec<u8>, ArrayRef) = match field.data_type() {
            DataType::Utf8 => {
                let a = array.as_any().downcast_ref::<StringArray>().unwrap();
                let mut bytes = vec![b' '; 32];
                if !a.is_null(0) {
                    bytes[..a.value(0).len()].copy_from_slice(a.value(0).as_bytes());
                }
                // DBF character blanks represent NULL in the native domain.
                let v = if a.is_null(0) || a.value(0).is_empty() {
                    None
                } else {
                    Some(a.value(0))
                };
                (b'C', bytes, Arc::new(StringArray::from(vec![v])))
            }
            DataType::Boolean => {
                let a = array.as_any().downcast_ref::<BooleanArray>().unwrap();
                (
                    b'L',
                    vec![if a.value(0) { b'T' } else { b'F' }],
                    array.clone(),
                )
            }
            DataType::Float32 => {
                let a = array.as_any().downcast_ref::<Float32Array>().unwrap();
                let v = f64::from(a.value(0));
                (
                    b'B',
                    v.to_le_bytes().to_vec(),
                    Arc::new(Float64Array::from(vec![v])),
                )
            }
            DataType::Float64 => {
                let a = array.as_any().downcast_ref::<Float64Array>().unwrap();
                (b'B', a.value(0).to_le_bytes().to_vec(), array.clone())
            }
            _ => panic!("unsupported DBF fixture"),
        };
        let mut descriptor = [0_u8; 32];
        descriptor[..field.name().len()].copy_from_slice(field.name().as_bytes());
        descriptor[11] = kind;
        descriptor[16] = u8::try_from(bytes.len()).unwrap();
        descriptors.extend(descriptor);
        row.extend(bytes);
        fields.push(Field::new(field.name(), expected.data_type().clone(), true));
        columns.push(expected);
    }
    let mut dbf = vec![0xfb_u8, 126, 1, 1];
    dbf.extend(1_u32.to_le_bytes());
    dbf.extend(u16::try_from(33 + descriptors.len()).unwrap().to_le_bytes());
    dbf.extend(u16::try_from(row.len()).unwrap().to_le_bytes());
    dbf.extend([0; 20]);
    dbf.extend(descriptors);
    dbf.push(13);
    dbf.extend(row);
    dbf.push(26);
    std::fs::create_dir(path).unwrap();
    let base = path.join("features");
    for (extension, bytes) in [("shp",shp),("shx",shx),("dbf",dbf),("cpg",b"UTF-8".to_vec()),("prj",br#"GEOGCS["WGS 84",DATUM["WGS_1984",SPHEROID["WGS 84",6378137,298.257223563]],PRIMEM["Greenwich",0],UNIT["degree",0.0174532925199433],AUTHORITY["EPSG","4326"]]"#.to_vec())] {
        std::fs::write(base.with_extension(extension),bytes).unwrap();
    }
    RecordBatch::try_new(Arc::new(Schema::new(fields)), columns).unwrap()
}
