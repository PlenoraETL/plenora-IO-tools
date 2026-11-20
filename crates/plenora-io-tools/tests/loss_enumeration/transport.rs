//! One Arrow transport fixture per semantic element, shared by every writer.
//! Keys for omitted native objects define the foreign transport envelope; they
//! do not claim that today's lossy native readers emit the omitted payload.
use super::*;

pub(super) fn generate(element: E) -> Option<Vec<RecordBatch>> {
    if let Some(batch) = group_a::factory(element) {
        return Some(batch);
    }
    let base = geometry_batch(WkbValue::Point(coordinate(1.0, 2.0)), GeometryType::Point);
    let (key, value, schema) = match element {
        E::DbfDate => return Some(vec![attribute(Arc::new(StringArray::from(vec!["1970-01-02"])), true)]),
        E::FormulaCache => return Some(vec![attribute(Arc::new(Float64Array::from(vec![42.0])), true)]),
        E::Inference => return Some(vec![attribute(Arc::new(Int64Array::from(vec![7])), true)]),
        E::StructuredValue => return Some(vec![attribute(Arc::new(StringArray::from(vec![r#"{"items":[1,2]}"#])), true)]),
        E::MissingValue => return Some(super::generate(E::Null).unwrap()),
        E::Subtypes => ("filegdb.subtypes", "<Subtypes><Subtype><SubtypeName>Road</SubtypeName><SubtypeCode>1</SubtypeCode></Subtype></Subtypes>", false),
        E::Aliases => ("filegdb.aliases", r#"{"value":"Road name"}"#, false),
        E::Domains => ("filegdb.domains", "<Domain><DomainName>RoadType</DomainName><CodedValues><CodedValue><Name>Road</Name><Code>1</Code></CodedValue></CodedValues></Domain>", false),
        E::TextStyle => ("dxf.text_style", "Standard", false),
        E::PaperSpace => ("dxf.paper_space", "1", false),
        E::Xdata => ("dxf.xdata", r#"[[1001,"APP"],[1000,"payload"]]"#, false),
        E::Width => ("dxf.width", "0.5", false),
        E::Thickness => ("dxf.thickness", "2", false),
        E::Lineweight => ("dxf.lineweight", "25", false),
        E::Linetype => ("dxf.linetype", "DASHED", false),
        E::Color => ("dxf.color", "3", false),
        E::Handle => ("dxf.handle", "A7", false),
        E::FeatureId => ("geojson.id", r#""native-id-47""#, false),
        E::Bbox => ("geoparquet.bbox", "0,0,3,3", false),
        E::ForeignMembers => ("geojson.foreign_members", r#"{"native-key":{"payload":[3,7]}}"#, false),
        E::LayerMetadata => ("kml.document", "<name>native collection title</name>", false),
        E::Styles => ("kml.styles", "<Style id=\"native-style\"><LineStyle><color>ff123456</color></LineStyle></Style>", false),
        E::Hierarchy => ("kml.hierarchy", r#"["Document","Folder"]"#, false),
        E::Extensions => ("gpkg.extensions", r#"[{"extension_name":"example_extension","definition":"urn:example:extension","scope":"read-write"}]"#, false),
        E::ExternalReference => ("kml.network_link", "<NetworkLink><Link><href>relative/external.kml</href></Link></NetworkLink>", false),
        E::ExtendedData => ("kml.extended_data", "<ExtendedData><Data name=\"value\"><value>native payload</value></Data></ExtendedData>", false),
        E::FeatureMetadata => ("kml.feature", "<visibility>0</visibility>", false),
        E::AltitudeMode => ("kml.altitude_mode", "relativeToGround", false),
        E::CoordinateColumns => ("csv.geometry_encoding", "xy_columns", false),
        E::NumericFormat => ("xlsx.numeric_format", "0.00%", false),
        E::Formula => ("xlsx.formula", r#"{"B2":"6*7"}"#, false),
        E::Identifier => ("gpkg.identifier", "native identifier", true),
        E::Description => ("gpkg.description", "native description", true),
        E::DataColumns => ("gpkg.data_columns", r#"[{"column_name":"value","title":"Native title"}]"#, false),
        E::SqlConstraints => ("gpkg.sql_constraints", "CHECK (value > 0)", false),
        E::Covering => ("geoparquet.column", r#"{"encoding":"WKB","geometry_types":["Point"],"covering":{"bbox":{"xmin":["bbox","xmin"],"ymin":["bbox","ymin"],"xmax":["bbox","xmax"],"ymax":["bbox","ymax"]}}"#, false),
        _ => return None,
    };
    let full_key = if schema {
        key.to_owned()
    } else {
        format!("plenora.geometry.native.{key}")
    };
    Some(vec![metadata(&base, &full_key, value, schema)])
}
