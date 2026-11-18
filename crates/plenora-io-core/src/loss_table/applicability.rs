//! Read grammar applicability, independent of preservation/rejection policy.
use super::{NativeDriver as F, NativeElement as E};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReadApplicability {
    Expressible,
    NotApplicable {
        reason: &'static str,
        reference: &'static str,
    },
}

/// Classify the source grammar, never the capabilities of the current reader.
/// IPC and Parquet can carry foreign semantics in Arrow metadata.
#[must_use]
#[allow(clippy::too_many_lines, clippy::match_same_arms)]
pub fn read_applicability(driver: F, element: E) -> ReadApplicability {
    if matches!(driver, F::Ipc | F::GeoParquet) {
        return ReadApplicability::Expressible;
    }
    // Distinguish a native scalar/schema domain from values derivable from
    // another construct (e.g. positive signed integers or a CRS authority).
    let absent = match (driver, element) {
        (F::Shp | F::Dxf | F::FileGdb | F::Gpkg, E::UnsignedInteger) => Some("Native integer domains are signed; positive values do not declare an unsigned type"),
        (F::FileGdb, E::SignedZero) => Some("Coordinates are stored on an integer precision grid, which has no negative-zero encoding"),
        (F::FileGdb, E::EmptyPoint | E::EmptyGeometry) => Some("Empty shapes use the native null-shape representation; no distinct typed EMPTY value survives native storage (NullGeometry is exercised)"),
        (F::FileGdb, E::Boolean) => Some("FileGDB field types have no Boolean domain; Short integers are not Boolean declarations"),
        (F::Shp | F::FileGdb, E::MissingValue) => Some("Fixed record schemas have null slots, not absent members distinct from NULL"),
        (F::Shp | F::Dxf | F::FileGdb, E::Srid) => Some("No independent per-geometry SRID slot; spatial-reference authorities are covered by CrsId/CrsDefinition"),
        (F::Dxf | F::FileGdb, E::UnknownDimensions) => Some("Native shape/entity coordinates determine dimensions; no explicit unknown-dimensions declaration"),
        (F::Dxf, E::NullGeometry | E::EmptyPoint | E::EmptyGeometry) => Some("DXF has no NULL or EMPTY geometry value; omitted entities and malformed coordinates are not empty geometries"),
        (F::Shp, E::LayerMetadata | E::Extensions) => Some("SHP/SHX/DBF grammar has no layer metadata or extension registry; vendor sidecars are separate formats"),
        _ => None,
    };
    if let Some(reason) = absent {
        return ReadApplicability::NotApplicable {
            reason,
            reference: match driver {
                F::Shp => "ESRI Shapefile Technical Description, record and field layouts",
                F::Dxf => "Autodesk DXF Reference, group-code value types and entity definitions",
                F::FileGdb => "ESRI File Geodatabase API, field and geometry types",
                F::Gpkg => "OGC GeoPackage 1.3.1 Table 1, integer data types",
                _ => unreachable!(),
            },
        };
    }
    let expressible = match element {
        E::Subtypes | E::Aliases | E::Domains => driver == F::FileGdb,
        E::TextStyle
        | E::PaperSpace
        | E::Xdata
        | E::Width
        | E::Thickness
        | E::Lineweight
        | E::Linetype
        | E::Color
        | E::Handle => driver == F::Dxf,
        E::DbfDate => driver == F::Shp,
        E::Formula | E::FormulaCache | E::NumericFormat => driver == F::Xls,
        E::Covering => false,
        E::DataColumns | E::SqlConstraints | E::Identifier => driver == F::Gpkg,
        E::Description => matches!(driver, F::Gpkg | F::Kml),
        E::ExtendedData | E::AltitudeMode => driver == F::Kml,
        E::Hierarchy => matches!(driver, F::Kml | F::Dxf | F::FileGdb),
        E::ForeignMembers => matches!(driver, F::GeoJson | F::Kml),
        E::Styles => matches!(driver, F::GeoJson | F::Kml | F::Dxf | F::Gpkg),
        E::FeatureMetadata => matches!(driver, F::GeoJson | F::Kml | F::Dxf),
        E::LayerMetadata | E::Extensions => !matches!(driver, F::Csv | F::Xls),
        E::ExternalReference => matches!(driver, F::GeoJson | F::Kml | F::Dxf),
        E::Bbox => matches!(driver, F::GeoJson | F::Shp | F::FileGdb),
        E::FeatureId => !matches!(driver, F::Csv | F::Xls),
        E::CoordinateColumns => matches!(driver, F::Csv | F::Xls),
        E::Inference => matches!(driver, F::GeoJson | F::Csv | F::Xls | F::Shp),
        E::StructuredValue | E::NestedDictionary => matches!(driver, F::GeoJson | F::Kml),
        E::MissingValue => !matches!(driver, F::Csv | F::Xls | F::Gpkg),
        E::Binary => matches!(driver, F::FileGdb | F::Gpkg),
        E::Decimal => false,
        E::Temporal => matches!(driver, F::Xls | F::Shp | F::FileGdb | F::Gpkg | F::Kml),
        E::NotNull => matches!(driver, F::FileGdb | F::Gpkg),
        E::SchemaMetadata => matches!(driver, F::Gpkg | F::Xls),
        E::FieldMetadata
        | E::ExtensionMetadata
        | E::FieldIdentity
        | E::ContractVersion
        | E::ExtensionName => driver == F::Gpkg,
        E::CrsResolution => false,
        E::CrsId => !matches!(driver, F::Csv | F::Xls),
        E::M => !matches!(driver, F::GeoJson | F::Kml | F::Dxf),
        E::NativeMetadata => true,
        E::CrsDefinition | E::DefinitionFormat | E::Srid => {
            matches!(driver, F::Shp | F::Dxf | F::FileGdb | F::Gpkg)
        }
        E::SignedInteger | E::UnsignedInteger | E::Floating | E::Boolean => true,
        E::NullGeometry
        | E::EmptyPoint
        | E::SignedZero
        | E::Utf8
        | E::Null
        | E::EmptyString
        | E::FieldNames
        | E::FieldOrder
        | E::Z
        | E::Dimensions
        | E::GeometryType
        | E::RingOrientation
        | E::EmptyGeometry
        | E::CoordinatePrecision
        | E::NumberPrecision
        | E::TypesDeclaration
        | E::GeometryEncoding
        | E::SpatialSemantics
        | E::Precision
        | E::AxisOrder
        | E::PrimaryGeometry
        | E::UnknownDimensions => true,
    };
    if expressible {
        return ReadApplicability::Expressible;
    }
    let reference = match driver {
        F::GeoJson => "RFC 7946 sections 3, 6 (GeoJSON objects and foreign members)",
        F::Kml => "OGC 07-147r2 (KML 2.2), Feature, Geometry and ExtendedData",
        F::Csv => "RFC 4180 section 2 (records and text fields; no schema metadata)",
        F::Xls => "ECMA-376 Part 1, 18 SpreadsheetML (cells, formulas and styles)",
        F::Shp => "ESRI Shapefile Technical Description, July 1998 (SHP/SHX/DBF)",
        F::Dxf => "Autodesk DXF Reference (HEADER, TABLES, BLOCKS, ENTITIES, OBJECTS)",
        F::FileGdb => "ESRI File Geodatabase API, geodatabase schema XML",
        F::Gpkg => "OGC GeoPackage 1.3.1, core tables and registered extensions",
        F::GeoParquet | F::Ipc => unreachable!(),
    };
    ReadApplicability::NotApplicable {
        reason:
            "No native grammar construct for this element; foreign Arrow transport is a Write case",
        reference,
    }
}
