use std::collections::BTreeMap;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

pub const ROW_DIAGNOSTICS_CONTRACT: &str = "plenora-row-diagnostics-v1";
pub const ROW_DIAGNOSTICS_INDEX_BASIS: &str = "source_row_zero_based";
pub const ROW_DIAGNOSTIC_COLUMN_UNATTESTABLE: &str = "diagnostic_column_name_unattestable";

/// Closed vocabulary for diagnostic text. Schema identifiers cannot inhabit this type.
/// Construct causes with enum variants; external text must pass through [`Self::parse`].
///
/// ```compile_fail
/// use plenora_io_model::diagnostics::DiagnosticCause;
/// let cause: DiagnosticCause = "conversion.invalid_date".into();
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DiagnosticCause {
    #[serde(rename = "contract.coordinate_dimensions")]
    ContractCoordinateDimensions,
    #[serde(rename = "contract.geometry_encoding")]
    ContractGeometryEncoding,
    #[serde(rename = "contract.geometry_type")]
    ContractGeometryType,
    #[serde(rename = "contract.nullability")]
    ContractNullability,
    #[serde(rename = "contract.srid")]
    ContractSrid,
    #[serde(rename = "conversion.invalid_date")]
    ConversionInvalidDate,
    #[serde(rename = "conversion.invalid_geometry")]
    ConversionInvalidGeometry,
    #[serde(rename = "driver.invalid_attribute")]
    DriverInvalidAttribute,
    #[serde(rename = "dxf.coordinate_dimensions_unsupported")]
    DxfCoordinateDimensionsUnsupported,
    #[serde(rename = "dxf.degenerate_geometry")]
    DxfDegenerateGeometry,
    #[serde(rename = "dxf.embedded_srid_unsupported")]
    DxfEmbeddedSridUnsupported,
    #[serde(rename = "dxf.empty_geometry_unsupported")]
    DxfEmptyGeometryUnsupported,
    #[serde(rename = "dxf.entity_decode_failed")]
    DxfEntityDecodeFailed,
    #[serde(rename = "dxf.entity_not_representable")]
    DxfEntityNotRepresentable,
    #[serde(rename = "dxf.geometry_type_unsupported")]
    DxfGeometryTypeUnsupported,
    #[serde(rename = "dxf.interior_rings_unsupported")]
    DxfInteriorRingsUnsupported,
    #[serde(rename = "dxf.layer_not_representable")]
    DxfLayerNotRepresentable,
    #[serde(rename = "dxf.measure_ordinate_unsupported")]
    DxfMeasureOrdinateUnsupported,
    #[serde(rename = "dxf.non_finite_coordinate")]
    DxfNonFiniteCoordinate,
    #[serde(rename = "dxf.null_geometry_unsupported")]
    DxfNullGeometryUnsupported,
    #[serde(rename = "dxf.unclosed_polygon_ring")]
    DxfUnclosedPolygonRing,
    #[serde(rename = "geojson.invalid_feature")]
    GeojsonInvalidFeature,
    #[serde(rename = "kml.cell_not_representable")]
    KmlCellNotRepresentable,
    #[serde(rename = "kml.geometry_not_representable")]
    KmlGeometryNotRepresentable,
    #[serde(rename = "kml.invalid_geometry")]
    KmlInvalidGeometry,
    #[serde(rename = "kml.invalid_placemark")]
    KmlInvalidPlacemark,
    #[serde(rename = "kml.unsupported_geometry")]
    KmlUnsupportedGeometry,
    #[serde(rename = "shapefile.attribute_conversion_failed")]
    ShapefileAttributeConversionFailed,
    #[serde(rename = "shapefile.attribute_decode_failed")]
    ShapefileAttributeDecodeFailed,
    #[serde(rename = "shapefile.attribute_numeric_invalid")]
    ShapefileAttributeNumericInvalid,
    #[serde(rename = "shapefile.attribute_scan_interrupted")]
    ShapefileAttributeScanInterrupted,
    #[serde(rename = "shapefile.cell_not_representable")]
    ShapefileCellNotRepresentable,
    #[serde(rename = "shapefile.dbf_exact_scan_interrupted")]
    ShapefileDbfExactScanInterrupted,
    #[serde(rename = "shapefile.degenerate_ring")]
    ShapefileDegenerateRing,
    #[serde(rename = "shapefile.geometry_conversion_failed")]
    ShapefileGeometryConversionFailed,
    #[serde(rename = "shapefile.geometry_encoding_failed")]
    ShapefileGeometryEncodingFailed,
    #[serde(rename = "shapefile.geometry_not_representable")]
    ShapefileGeometryNotRepresentable,
    #[serde(rename = "shapefile.geometry_type_unsupported")]
    ShapefileGeometryTypeUnsupported,
    #[serde(rename = "shapefile.inner_ring_without_outer")]
    ShapefileInnerRingWithoutOuter,
    #[serde(rename = "shapefile.invalid_geometry")]
    ShapefileInvalidGeometry,
    #[serde(rename = "shapefile.mixed_geometry_type")]
    ShapefileMixedGeometryType,
    #[serde(rename = "shapefile.polygon_without_outer")]
    ShapefilePolygonWithoutOuter,
    #[serde(rename = "shapefile.scan_cancelled")]
    ShapefileScanCancelled,
    #[serde(rename = "shapefile.scan_interrupted")]
    ShapefileScanInterrupted,
    #[serde(rename = "shapefile.shape_type_mismatch")]
    ShapefileShapeTypeMismatch,
    #[serde(rename = "shapefile.unclosed_ring")]
    ShapefileUnclosedRing,
}
impl DiagnosticCause {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ContractCoordinateDimensions => "contract.coordinate_dimensions",
            Self::ContractGeometryEncoding => "contract.geometry_encoding",
            Self::ContractGeometryType => "contract.geometry_type",
            Self::ContractNullability => "contract.nullability",
            Self::ContractSrid => "contract.srid",
            Self::ConversionInvalidDate => "conversion.invalid_date",
            Self::ConversionInvalidGeometry => "conversion.invalid_geometry",
            Self::DriverInvalidAttribute => "driver.invalid_attribute",
            Self::DxfCoordinateDimensionsUnsupported => "dxf.coordinate_dimensions_unsupported",
            Self::DxfDegenerateGeometry => "dxf.degenerate_geometry",
            Self::DxfEmbeddedSridUnsupported => "dxf.embedded_srid_unsupported",
            Self::DxfEmptyGeometryUnsupported => "dxf.empty_geometry_unsupported",
            Self::DxfEntityDecodeFailed => "dxf.entity_decode_failed",
            Self::DxfEntityNotRepresentable => "dxf.entity_not_representable",
            Self::DxfGeometryTypeUnsupported => "dxf.geometry_type_unsupported",
            Self::DxfInteriorRingsUnsupported => "dxf.interior_rings_unsupported",
            Self::DxfLayerNotRepresentable => "dxf.layer_not_representable",
            Self::DxfMeasureOrdinateUnsupported => "dxf.measure_ordinate_unsupported",
            Self::DxfNonFiniteCoordinate => "dxf.non_finite_coordinate",
            Self::DxfNullGeometryUnsupported => "dxf.null_geometry_unsupported",
            Self::DxfUnclosedPolygonRing => "dxf.unclosed_polygon_ring",
            Self::GeojsonInvalidFeature => "geojson.invalid_feature",
            Self::KmlCellNotRepresentable => "kml.cell_not_representable",
            Self::KmlGeometryNotRepresentable => "kml.geometry_not_representable",
            Self::KmlInvalidGeometry => "kml.invalid_geometry",
            Self::KmlInvalidPlacemark => "kml.invalid_placemark",
            Self::KmlUnsupportedGeometry => "kml.unsupported_geometry",
            Self::ShapefileAttributeConversionFailed => "shapefile.attribute_conversion_failed",
            Self::ShapefileAttributeDecodeFailed => "shapefile.attribute_decode_failed",
            Self::ShapefileAttributeNumericInvalid => "shapefile.attribute_numeric_invalid",
            Self::ShapefileAttributeScanInterrupted => "shapefile.attribute_scan_interrupted",
            Self::ShapefileCellNotRepresentable => "shapefile.cell_not_representable",
            Self::ShapefileDbfExactScanInterrupted => "shapefile.dbf_exact_scan_interrupted",
            Self::ShapefileDegenerateRing => "shapefile.degenerate_ring",
            Self::ShapefileGeometryConversionFailed => "shapefile.geometry_conversion_failed",
            Self::ShapefileGeometryEncodingFailed => "shapefile.geometry_encoding_failed",
            Self::ShapefileGeometryNotRepresentable => "shapefile.geometry_not_representable",
            Self::ShapefileGeometryTypeUnsupported => "shapefile.geometry_type_unsupported",
            Self::ShapefileInnerRingWithoutOuter => "shapefile.inner_ring_without_outer",
            Self::ShapefileInvalidGeometry => "shapefile.invalid_geometry",
            Self::ShapefileMixedGeometryType => "shapefile.mixed_geometry_type",
            Self::ShapefilePolygonWithoutOuter => "shapefile.polygon_without_outer",
            Self::ShapefileScanCancelled => "shapefile.scan_cancelled",
            Self::ShapefileScanInterrupted => "shapefile.scan_interrupted",
            Self::ShapefileShapeTypeMismatch => "shapefile.shape_type_mismatch",
            Self::ShapefileUnclosedRing => "shapefile.unclosed_ring",
        }
    }
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "contract.coordinate_dimensions" => Some(Self::ContractCoordinateDimensions),
            "contract.geometry_encoding" => Some(Self::ContractGeometryEncoding),
            "contract.geometry_type" => Some(Self::ContractGeometryType),
            "contract.nullability" => Some(Self::ContractNullability),
            "contract.srid" => Some(Self::ContractSrid),
            "conversion.invalid_date" => Some(Self::ConversionInvalidDate),
            "conversion.invalid_geometry" => Some(Self::ConversionInvalidGeometry),
            "driver.invalid_attribute" => Some(Self::DriverInvalidAttribute),
            "dxf.coordinate_dimensions_unsupported" => {
                Some(Self::DxfCoordinateDimensionsUnsupported)
            }
            "dxf.degenerate_geometry" => Some(Self::DxfDegenerateGeometry),
            "dxf.embedded_srid_unsupported" => Some(Self::DxfEmbeddedSridUnsupported),
            "dxf.empty_geometry_unsupported" => Some(Self::DxfEmptyGeometryUnsupported),
            "dxf.entity_decode_failed" => Some(Self::DxfEntityDecodeFailed),
            "dxf.entity_not_representable" => Some(Self::DxfEntityNotRepresentable),
            "dxf.geometry_type_unsupported" => Some(Self::DxfGeometryTypeUnsupported),
            "dxf.interior_rings_unsupported" => Some(Self::DxfInteriorRingsUnsupported),
            "dxf.layer_not_representable" => Some(Self::DxfLayerNotRepresentable),
            "dxf.measure_ordinate_unsupported" => Some(Self::DxfMeasureOrdinateUnsupported),
            "dxf.non_finite_coordinate" => Some(Self::DxfNonFiniteCoordinate),
            "dxf.null_geometry_unsupported" => Some(Self::DxfNullGeometryUnsupported),
            "dxf.unclosed_polygon_ring" => Some(Self::DxfUnclosedPolygonRing),
            "geojson.invalid_feature" => Some(Self::GeojsonInvalidFeature),
            "kml.cell_not_representable" => Some(Self::KmlCellNotRepresentable),
            "kml.geometry_not_representable" => Some(Self::KmlGeometryNotRepresentable),
            "kml.invalid_geometry" => Some(Self::KmlInvalidGeometry),
            "kml.invalid_placemark" => Some(Self::KmlInvalidPlacemark),
            "kml.unsupported_geometry" => Some(Self::KmlUnsupportedGeometry),
            "shapefile.attribute_conversion_failed" => {
                Some(Self::ShapefileAttributeConversionFailed)
            }
            "shapefile.attribute_decode_failed" => Some(Self::ShapefileAttributeDecodeFailed),
            "shapefile.attribute_numeric_invalid" => Some(Self::ShapefileAttributeNumericInvalid),
            "shapefile.attribute_scan_interrupted" => Some(Self::ShapefileAttributeScanInterrupted),
            "shapefile.cell_not_representable" => Some(Self::ShapefileCellNotRepresentable),
            "shapefile.dbf_exact_scan_interrupted" => Some(Self::ShapefileDbfExactScanInterrupted),
            "shapefile.degenerate_ring" => Some(Self::ShapefileDegenerateRing),
            "shapefile.geometry_conversion_failed" => Some(Self::ShapefileGeometryConversionFailed),
            "shapefile.geometry_encoding_failed" => Some(Self::ShapefileGeometryEncodingFailed),
            "shapefile.geometry_not_representable" => Some(Self::ShapefileGeometryNotRepresentable),
            "shapefile.geometry_type_unsupported" => Some(Self::ShapefileGeometryTypeUnsupported),
            "shapefile.inner_ring_without_outer" => Some(Self::ShapefileInnerRingWithoutOuter),
            "shapefile.invalid_geometry" => Some(Self::ShapefileInvalidGeometry),
            "shapefile.mixed_geometry_type" => Some(Self::ShapefileMixedGeometryType),
            "shapefile.polygon_without_outer" => Some(Self::ShapefilePolygonWithoutOuter),
            "shapefile.scan_cancelled" => Some(Self::ShapefileScanCancelled),
            "shapefile.scan_interrupted" => Some(Self::ShapefileScanInterrupted),
            "shapefile.shape_type_mismatch" => Some(Self::ShapefileShapeTypeMismatch),
            "shapefile.unclosed_ring" => Some(Self::ShapefileUnclosedRing),
            _ => None,
        }
    }
}
impl PartialEq<&str> for DiagnosticCause {
    fn eq(&self, other: &&str) -> bool {
        self.as_str() == *other
    }
}
/// A schema identifier, serialized only in a dedicated structured field.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct DiagnosticFieldName(String);
impl From<String> for DiagnosticFieldName {
    fn from(value: String) -> Self {
        Self(value)
    }
}
impl std::ops::Deref for DiagnosticFieldName {
    type Target = str;
    fn deref(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RowDiagnosticColumn {
    Attested(DiagnosticFieldName),
    Unattestable,
}

impl RowDiagnosticColumn {
    pub fn attest(value: impl Into<String>) -> Self {
        let value = value.into();
        if !value.is_empty() && value.chars().count() <= 256 {
            Self::Attested(value.into())
        } else {
            Self::Unattestable
        }
    }

    #[must_use]
    pub fn into_option(self) -> Option<DiagnosticFieldName> {
        match self {
            Self::Attested(value) => Some(value),
            Self::Unattestable => None,
        }
    }

    #[must_use]
    pub const fn is_attested(&self) -> bool {
        matches!(self, Self::Attested(_))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RowDiagnosticScope {
    Read,
    Write,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RowDiagnosticsCompleteness {
    Complete,
    Partial,
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RowDiagnosticKeyState {
    Value,
    Redacted,
    Unavailable,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum RowDiagnosticKeyValue {}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RowDiagnosticKey {
    pub field: DiagnosticFieldName,
    pub state: RowDiagnosticKeyState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<RowDiagnosticKeyValue>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RowDiagnosticWriteState {
    CertainlyRejected,
    CertainlyNotAttempted,
    CertainlyRolledBack,
    EffectUnknown,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RowDiagnosticExample {
    pub source_index: u64,
    pub cause: DiagnosticCause,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub column: Option<DiagnosticFieldName>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<RowDiagnosticKey>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub write_state: Option<RowDiagnosticWriteState>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum KnownOrUnknownCount {
    Known { value: u64 },
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WriteDiagnosticStateCounts {
    pub certainly_rejected: u64,
    pub certainly_not_attempted: u64,
    pub certainly_rolled_back: u64,
    pub effect_unknown: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RowDiagnosticWriteOutcome {
    pub certainly_rejected: KnownOrUnknownCount,
    pub certainly_not_attempted: KnownOrUnknownCount,
    pub certainly_rolled_back: KnownOrUnknownCount,
    pub effect_unknown: KnownOrUnknownCount,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RowDiagnostics {
    pub contract: String,
    pub scope: RowDiagnosticScope,
    pub index_basis: String,
    pub completeness: RowDiagnosticsCompleteness,
    pub knowledge_limits: Option<Vec<String>>,
    pub observed_total: u64,
    pub total: Option<u64>,
    pub input_total: Option<u64>,
    pub counts: BTreeMap<String, u64>,
    pub examples_limit: u64,
    pub examples_truncated: bool,
    pub examples: Vec<RowDiagnosticExample>,
    pub diagnostic_state_counts: Option<WriteDiagnosticStateCounts>,
    pub write_outcome: Option<RowDiagnosticWriteOutcome>,
}

impl RowDiagnostics {
    /// Verifica sia la forma sia gli invarianti aritmetici di
    /// `plenora-row-diagnostics-v1`. La serializzazione richiama sempre questo
    /// metodo, così un documento costruito manualmente non può oltrepassare il
    /// bordo pubblico in uno stato non conforme.
    ///
    /// # Errors
    ///
    /// Restituisce la descrizione del primo invariante violato: contratto o
    /// `index_basis` non riconosciuto, `examples_limit` nullo, conteggi non
    /// coerenti con `observed_total`, completezza incompatibile con i
    /// conteggi, esempi o chiavi non conformi.
    // La sequenza di controlli resta lineare e monolitica per essere
    // confrontabile, nell'ordine, con il contratto normativo
    // `plenora-row-diagnostics-v1`.
    #[allow(clippy::too_many_lines)]
    pub fn validate(&self) -> std::result::Result<(), String> {
        if self.contract != ROW_DIAGNOSTICS_CONTRACT {
            return Err("contratto row diagnostics non riconosciuto".to_owned());
        }
        if self.index_basis != ROW_DIAGNOSTICS_INDEX_BASIS {
            return Err("index_basis row diagnostics non riconosciuto".to_owned());
        }
        if self.examples_limit == 0 {
            return Err("examples_limit deve essere almeno 1".to_owned());
        }
        let counts_sum = self.counts.iter().try_fold(0_u64, |sum, (cause, count)| {
            if DiagnosticCause::parse(cause).is_none() || *count == 0 {
                return Err("causa o conteggio row diagnostics non valido".to_owned());
            }
            sum.checked_add(*count)
                .ok_or_else(|| "overflow nei conteggi row diagnostics".to_owned())
        })?;
        if counts_sum != self.observed_total {
            return Err("counts non somma a observed_total".to_owned());
        }
        match self.completeness {
            RowDiagnosticsCompleteness::Complete => {
                if self.total != Some(self.observed_total) || self.observed_total == 0 {
                    return Err(
                        "report complete richiede total positivo uguale a observed_total"
                            .to_owned(),
                    );
                }
                if self.knowledge_limits.is_some() {
                    return Err("report complete non ammette knowledge_limits".to_owned());
                }
            }
            RowDiagnosticsCompleteness::Partial => {
                validate_knowledge_limits(self.knowledge_limits.as_deref())?;
                if self
                    .total
                    .is_some_and(|total| total < self.observed_total || total == 0)
                {
                    return Err("total parziale non valido".to_owned());
                }
            }
            RowDiagnosticsCompleteness::Unknown => {
                validate_knowledge_limits(self.knowledge_limits.as_deref())?;
                if self.total.is_some() {
                    return Err("report unknown non ammette total".to_owned());
                }
            }
        }
        let examples_len = u64::try_from(self.examples.len())
            .map_err(|_| "troppi esempi row diagnostics".to_owned())?;
        if examples_len > self.examples_limit || examples_len > self.observed_total {
            return Err("examples supera il limite o observed_total".to_owned());
        }
        if self.completeness == RowDiagnosticsCompleteness::Complete
            && examples_len != self.observed_total.min(self.examples_limit)
        {
            return Err("report complete con esempi incompleti".to_owned());
        }
        let expected_truncated =
            self.observed_total > examples_len && examples_len == self.examples_limit;
        if self.examples_truncated != expected_truncated {
            return Err("examples_truncated incoerente".to_owned());
        }
        let mut indices = std::collections::BTreeSet::new();
        let mut example_causes = BTreeMap::<&str, u64>::new();
        for example in &self.examples {
            if !indices.insert(example.source_index) {
                return Err("source_index duplicato negli esempi".to_owned());
            }
            if !self.counts.contains_key(example.cause.as_str()) {
                return Err("causa esempio assente dai conteggi".to_owned());
            }
            if example
                .column
                .as_ref()
                .is_some_and(|value| value.is_empty() || value.chars().count() > 256)
            {
                return Err("column esempio non valida".to_owned());
            }
            validate_key(example.key.as_ref())?;
            *example_causes.entry(example.cause.as_str()).or_default() += 1;
        }
        for (cause, count) in example_causes {
            if count > self.counts[cause] {
                return Err("esempi eccedono il conteggio della causa".to_owned());
            }
        }
        match self.scope {
            RowDiagnosticScope::Read => {
                if self.input_total.is_some()
                    || self.diagnostic_state_counts.is_some()
                    || self.write_outcome.is_some()
                    || self
                        .examples
                        .iter()
                        .any(|example| example.write_state.is_some())
                {
                    return Err("report read contiene campi riservati alla write".to_owned());
                }
            }
            RowDiagnosticScope::Write => self.validate_write(examples_len)?,
        }
        Ok(())
    }

    fn validate_write(&self, _examples_len: u64) -> std::result::Result<(), String> {
        let input_total = self
            .input_total
            .filter(|value| *value > 0)
            .ok_or_else(|| "report write richiede input_total positivo".to_owned())?;
        if self.observed_total > input_total
            || self.examples.iter().any(|e| e.write_state.is_none())
        {
            return Err("report write privo di input totale o stato esempio valido".to_owned());
        }
        let states = self
            .diagnostic_state_counts
            .as_ref()
            .ok_or_else(|| "report write richiede diagnostic_state_counts".to_owned())?;
        let state_sum = states
            .certainly_rejected
            .checked_add(states.certainly_not_attempted)
            .and_then(|sum| sum.checked_add(states.certainly_rolled_back))
            .and_then(|sum| sum.checked_add(states.effect_unknown))
            .ok_or_else(|| "overflow nei diagnostic_state_counts".to_owned())?;
        if state_sum != self.observed_total {
            return Err("diagnostic_state_counts non somma a observed_total".to_owned());
        }
        let mut example_states = WriteDiagnosticStateCounts {
            certainly_rejected: 0,
            certainly_not_attempted: 0,
            certainly_rolled_back: 0,
            effect_unknown: 0,
        };
        for example in &self.examples {
            let count = match example.write_state {
                Some(RowDiagnosticWriteState::CertainlyRejected) => {
                    &mut example_states.certainly_rejected
                }
                Some(RowDiagnosticWriteState::CertainlyNotAttempted) => {
                    &mut example_states.certainly_not_attempted
                }
                Some(RowDiagnosticWriteState::CertainlyRolledBack) => {
                    &mut example_states.certainly_rolled_back
                }
                Some(RowDiagnosticWriteState::EffectUnknown) => &mut example_states.effect_unknown,
                None => return Err("report write privo di stato esempio".to_owned()),
            };
            *count = count
                .checked_add(1)
                .ok_or_else(|| "overflow negli stati degli esempi write".to_owned())?;
        }
        if example_states.certainly_rejected > states.certainly_rejected
            || example_states.certainly_not_attempted > states.certainly_not_attempted
            || example_states.certainly_rolled_back > states.certainly_rolled_back
            || example_states.effect_unknown > states.effect_unknown
        {
            return Err("esempi eccedono diagnostic_state_counts".to_owned());
        }
        let outcome = self
            .write_outcome
            .as_ref()
            .ok_or_else(|| "report write richiede write_outcome".to_owned())?;
        let buckets = [
            (&outcome.certainly_rejected, states.certainly_rejected),
            (
                &outcome.certainly_not_attempted,
                states.certainly_not_attempted,
            ),
            (&outcome.certainly_rolled_back, states.certainly_rolled_back),
            (&outcome.effect_unknown, states.effect_unknown),
        ];
        let mut known_sum = 0_u64;
        let mut diagnosed_unknown = 0_u64;
        let mut all_known = true;
        for (bucket, diagnosed) in buckets {
            match bucket {
                KnownOrUnknownCount::Known { value } => {
                    if diagnosed > *value {
                        return Err("diagnostica eccede un bucket write noto".to_owned());
                    }
                    known_sum = known_sum
                        .checked_add(*value)
                        .ok_or_else(|| "overflow nei bucket write".to_owned())?;
                }
                KnownOrUnknownCount::Unknown => {
                    all_known = false;
                    diagnosed_unknown = diagnosed_unknown
                        .checked_add(diagnosed)
                        .ok_or_else(|| "overflow nei bucket write unknown".to_owned())?;
                }
            }
        }
        if known_sum > input_total
            || known_sum
                .checked_add(diagnosed_unknown)
                .is_none_or(|sum| sum > input_total)
            || (all_known && known_sum != input_total)
        {
            return Err("partizione write_outcome incoerente con input_total".to_owned());
        }
        Ok(())
    }
}

fn valid_knowledge_limit(value: &str) -> bool {
    matches!(
        value,
        "diagnostic_column_name_unattestable"
            | "source_row_identity_unattestable"
            | "scan_terminated_before_eof"
            | "scan_cancelled_before_eof"
            | "scan_deadline_exceeded_before_eof"
            | "driver_row_diagnostics_invalid"
            | "read_scope_row_limit_reached"
            | "driver_scan_interrupted"
            | "write_validation_stopped_at_first_rejected_batch"
            | "shapefile.scan_cancelled"
            | "shapefile.dbf_exact_scan_interrupted"
            | "shapefile.scan_interrupted"
            | "shapefile.attribute_scan_interrupted"
    )
}

fn validate_knowledge_limits(values: Option<&[String]>) -> std::result::Result<(), String> {
    let values = values
        .filter(|values| !values.is_empty())
        .ok_or_else(|| "report non complete richiede knowledge_limits".to_owned())?;
    let mut unique = std::collections::BTreeSet::new();
    if values
        .iter()
        .any(|value| !valid_knowledge_limit(value) || !unique.insert(value))
    {
        return Err("knowledge_limits non validi o duplicati".to_owned());
    }
    Ok(())
}

fn validate_key(key: Option<&RowDiagnosticKey>) -> std::result::Result<(), String> {
    let Some(key) = key else {
        return Ok(());
    };
    if key.field.is_empty() || key.field.chars().count() > 256 {
        return Err("campo chiave diagnostica non valido".to_owned());
    }
    match (&key.state, &key.value) {
        (RowDiagnosticKeyState::Redacted | RowDiagnosticKeyState::Unavailable, None) => Ok(()),
        _ => Err("diagnostic key values are forbidden".to_owned()),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DeserializableRowDiagnostics {
    contract: String,
    scope: RowDiagnosticScope,
    index_basis: String,
    completeness: RowDiagnosticsCompleteness,
    #[serde(default)]
    knowledge_limits: Option<Vec<String>>,
    observed_total: u64,
    #[serde(default)]
    total: Option<u64>,
    #[serde(default)]
    input_total: Option<u64>,
    counts: BTreeMap<String, u64>,
    examples_limit: u64,
    examples_truncated: bool,
    examples: Vec<RowDiagnosticExample>,
    #[serde(default)]
    diagnostic_state_counts: Option<WriteDiagnosticStateCounts>,
    #[serde(default)]
    write_outcome: Option<RowDiagnosticWriteOutcome>,
}

impl<'de> Deserialize<'de> for RowDiagnostics {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        let raw = DeserializableRowDiagnostics::deserialize(deserializer)?;
        let diagnostics = Self {
            contract: raw.contract,
            scope: raw.scope,
            index_basis: raw.index_basis,
            completeness: raw.completeness,
            knowledge_limits: raw.knowledge_limits,
            observed_total: raw.observed_total,
            total: raw.total,
            input_total: raw.input_total,
            counts: raw.counts,
            examples_limit: raw.examples_limit,
            examples_truncated: raw.examples_truncated,
            examples: raw.examples,
            diagnostic_state_counts: raw.diagnostic_state_counts,
            write_outcome: raw.write_outcome,
        };
        diagnostics.validate().map_err(serde::de::Error::custom)?;
        Ok(diagnostics)
    }
}

#[derive(Serialize)]
struct SerializableRowDiagnostics<'a> {
    contract: &'a str,
    scope: RowDiagnosticScope,
    index_basis: &'a str,
    completeness: RowDiagnosticsCompleteness,
    #[serde(skip_serializing_if = "Option::is_none")]
    knowledge_limits: &'a Option<Vec<String>>,
    observed_total: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    total: &'a Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    input_total: &'a Option<u64>,
    counts: &'a BTreeMap<String, u64>,
    examples_limit: u64,
    examples_truncated: bool,
    examples: &'a [RowDiagnosticExample],
    #[serde(skip_serializing_if = "Option::is_none")]
    diagnostic_state_counts: &'a Option<WriteDiagnosticStateCounts>,
    #[serde(skip_serializing_if = "Option::is_none")]
    write_outcome: &'a Option<RowDiagnosticWriteOutcome>,
}

impl Serialize for RowDiagnostics {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        self.validate().map_err(serde::ser::Error::custom)?;
        SerializableRowDiagnostics {
            contract: &self.contract,
            scope: self.scope,
            index_basis: &self.index_basis,
            completeness: self.completeness,
            knowledge_limits: &self.knowledge_limits,
            observed_total: self.observed_total,
            total: &self.total,
            input_total: &self.input_total,
            counts: &self.counts,
            examples_limit: self.examples_limit,
            examples_truncated: self.examples_truncated,
            examples: &self.examples,
            diagnostic_state_counts: &self.diagnostic_state_counts,
            write_outcome: &self.write_outcome,
        }
        .serialize(serializer)
    }
}

#[cfg(test)]
mod tests;
