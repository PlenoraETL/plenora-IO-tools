//! `LossReport` — un driver `Approximating` deve popolarlo, mai perdere in
//! silenzio (ADR-IO 5). Aggregato per categoria e **bounded**: conteggi +
//! un numero limitato di esempi, mai una voce per feature.

use std::collections::BTreeMap;

use serde::Serialize;

use plenora_io_model::contract::LayerContract;
use plenora_io_model::crs::{definition_authority_srid, CrsResolution};

use crate::descriptor::Fidelity;

/// Tetto agli esempi diagnostici conservati (nessun accumulo illimitato).
pub const MAX_LOSS_EXAMPLES: usize = 64;
/// Anche le motivazioni della valutazione restano bounded.
pub const MAX_FIDELITY_REASONS: usize = 64;
/// Categoria stabile per R4.3.1/R4.6.1, leggibile dagli harness di conformità.
pub const INCONSISTENT_CRS_REPRESENTATIONS: &str = "inconsistent_crs_representations";

#[derive(Clone, Debug, Serialize)]
pub struct LossExample {
    pub category: String,
    /// Descrizione strutturale, mai il valore sensibile (es. "layer=X row=12").
    pub context: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FidelityReasonCode {
    AssessmentPending,
    FormatConstraint,
    GeometryApproximation,
    StructureChanged,
    AttributeLoss,
    TypeCoercion,
    PrecisionChanged,
    NullabilityChanged,
    NativeMetadataLoss,
    LossReported,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct FidelityReason {
    pub code: FidelityReasonCode,
    pub detail: String,
}

/// Valutazione concreta restituita da `open`/`create` (ADR-IO 5). Il
/// descrittore resta la capacità generale; questa struttura porta l'esito
/// osservato per il dataset o il contratto corrente.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct FidelityAssessment {
    pub level: Fidelity,
    pub reasons: Vec<FidelityReason>,
}

impl FidelityAssessment {
    pub fn lossless() -> Self {
        Self {
            level: Fidelity::Lossless,
            reasons: Vec::new(),
        }
    }

    pub fn for_format(format: &str, class: Fidelity) -> Self {
        match class {
            Fidelity::Lossless => Self::lossless(),
            Fidelity::Conditional => Self {
                level: Fidelity::Conditional,
                reasons: vec![FidelityReason {
                    code: FidelityReasonCode::FormatConstraint,
                    detail: format!(
                        "{format}: fedeltà condizionata ai tipi e alle semantiche del contratto"
                    ),
                }],
            },
            Fidelity::Approximating => Self {
                level: Fidelity::Approximating,
                reasons: vec![FidelityReason {
                    code: FidelityReasonCode::GeometryApproximation,
                    detail: format!("{format}: il formato può richiedere approssimazioni"),
                }],
            },
        }
    }

    pub fn unassessed(context: impl Into<String>) -> Self {
        Self {
            level: Fidelity::Conditional,
            reasons: vec![FidelityReason {
                code: FidelityReasonCode::AssessmentPending,
                detail: context.into(),
            }],
        }
    }

    pub fn add_reason(&mut self, code: FidelityReasonCode, detail: impl Into<String>) {
        if self.reasons.len() >= MAX_FIDELITY_REASONS {
            return;
        }
        let reason = FidelityReason {
            code,
            detail: detail.into(),
        };
        if !self.reasons.contains(&reason) {
            self.reasons.push(reason);
        }
    }

    /// Le perdite osservate prevalgono su una valutazione teorica e rendono
    /// l'esito concretamente `Approximating`.
    pub fn with_loss_report(mut self, loss: &LossReport) -> Self {
        if loss.is_empty() {
            return self;
        }
        self.level = Fidelity::Approximating;
        for (category, count) in &loss.counts {
            self.add_reason(
                reason_code_for_loss(category),
                format!("{category}: {count} occorrenze"),
            );
        }
        self
    }
}

fn reason_code_for_loss(category: &str) -> FidelityReasonCode {
    let category = category.to_lowercase();
    if category.contains("tassell")
        || category.contains("approssim")
        || category.contains("curv")
        || category.contains("bulge")
    {
        FidelityReasonCode::GeometryApproximation
    } else if category.contains("esplos")
        || category.contains("multipart")
        || category.contains("collection")
    {
        FidelityReasonCode::StructureChanged
    } else if category.contains("coerc") {
        FidelityReasonCode::TypeCoercion
    } else if category.contains("attribut")
        || category.contains("colonn")
        || category.contains("propriet")
    {
        FidelityReasonCode::AttributeLoss
    } else if category.contains("precision") {
        FidelityReasonCode::PrecisionChanged
    } else if category.contains("null") {
        FidelityReasonCode::NullabilityChanged
    } else if category.contains("metadata")
        || category.contains("metadati")
        || category.starts_with("crs_")
        || category.starts_with("srid_")
    {
        FidelityReasonCode::NativeMetadataLoss
    } else {
        FidelityReasonCode::LossReported
    }
}

#[derive(Clone, Debug, Default)]
pub struct LossReport {
    /// Aggregati per categoria: (categoria -> conteggio).
    pub counts: BTreeMap<String, u64>,
    /// Esempi diagnostici, limitati a `MAX_LOSS_EXAMPLES`.
    examples: Vec<LossExample>,
}

impl LossReport {
    pub fn record(&mut self, category: &str, n: u64) {
        let count = self.counts.entry(category.to_owned()).or_default();
        *count = count.saturating_add(n);
    }

    /// Aggiunge un esempio solo finché sotto il tetto (bounded).
    pub fn add_example(&mut self, example: LossExample) {
        if self.examples.len() < MAX_LOSS_EXAMPLES {
            self.examples.push(example);
        }
    }

    pub fn examples(&self) -> &[LossExample] {
        &self.examples
    }

    pub fn is_empty(&self) -> bool {
        self.counts.is_empty()
    }

    /// Unisce report prodotti da livelli diversi (piano statico, driver,
    /// publish) senza perdere il bound diagnostico.
    pub fn merge(&mut self, other: &Self) {
        for (category, count) in &other.counts {
            self.record(category, *count);
        }
        for example in &other.examples {
            if self.examples.len() >= MAX_LOSS_EXAMPLES {
                break;
            }
            self.examples.push(example.clone());
        }
    }
}

/// Dichiara, senza respingerla né conciliarla, un'incoerenza rilevabile fra
/// `crs_definition`, `crs_id` EPSG e `plenora.geometry.srid` osservata da un
/// bordo di lettura.
///
/// La funzione è idempotente sul report così che più adattatori reader possano
/// essere composti senza moltiplicare la stessa osservazione strutturale.
pub(crate) fn declare_crs_inconsistency(contract: &LayerContract, report: &mut LossReport) {
    if report.counts.contains_key(INCONSISTENT_CRS_REPRESENTATIONS) {
        return;
    }
    let Some(geometry) = &contract.contract.geometry else {
        return;
    };
    let (crs_id, definition, definition_format) = match &geometry.crs {
        CrsResolution::Resolved(crs) => (
            crs.id.as_deref(),
            crs.definition.as_deref(),
            crs.definition_format,
        ),
        CrsResolution::DeclaredButUnresolved(raw) => (
            raw.authority_hint.as_deref(),
            raw.definition.as_deref(),
            raw.definition_format,
        ),
        CrsResolution::Missing => (None, None, None),
    };

    let id_srid = crs_id
        .and_then(plenora_io_model::crs::authority_srid)
        .map(i64::from);
    let definition_srid = definition
        .zip(definition_format)
        .and_then(|(value, format)| definition_authority_srid(value, format))
        .map(i64::from);
    let native_srid = geometry.srid.map(i64::from);
    let known = [definition_srid, id_srid, native_srid]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
    if known.len() < 2 || known.windows(2).all(|pair| pair[0] == pair[1]) {
        return;
    }

    report.record(INCONSISTENT_CRS_REPRESENTATIONS, 1);
    let srid = geometry
        .srid
        .map_or_else(|| "<none>".to_owned(), |value| value.to_string());
    let crs_id = crs_id.map_or("<none>", |value| value);
    report.add_example(LossExample {
        category: INCONSISTENT_CRS_REPRESENTATIONS.to_owned(),
        context: format!(
            "layer={} field={} definition_epsg={definition_srid:?} crs_id={} srid={srid}",
            contract.name, geometry.name, crs_id
        ),
    });
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use arrow_schema::{DataType, Field, Schema};
    use plenora_io_model::contract::{DataContract, FieldId, GeometryColumnContract, LayerId};
    use plenora_io_model::crs::{CrsKind, RawCrs, ResolvedCrs};

    use super::*;

    fn layer_with_geometry(geometry: GeometryColumnContract) -> LayerContract {
        LayerContract {
            id: LayerId(0),
            name: "parcels".to_owned(),
            contract: DataContract::new(
                Arc::new(Schema::new(vec![Field::new(
                    "geom",
                    DataType::Binary,
                    true,
                )])),
                Some(geometry),
            ),
        }
    }

    #[test]
    fn observed_loss_promotes_assessment_and_stays_bounded() {
        let mut report = LossReport::default();
        for index in 0..(MAX_FIDELITY_REASONS + 10) {
            report.record(&format!("category-{index}"), 1);
        }
        let assessment =
            FidelityAssessment::for_format("test", Fidelity::Conditional).with_loss_report(&report);
        assert_eq!(assessment.level, Fidelity::Approximating);
        assert_eq!(assessment.reasons.len(), MAX_FIDELITY_REASONS);
        assert!(assessment
            .reasons
            .iter()
            .any(|reason| reason.code == FidelityReasonCode::LossReported));
    }

    #[test]
    fn lossless_assessment_has_no_reasons() {
        assert_eq!(
            FidelityAssessment::for_format("ipc", Fidelity::Lossless),
            FidelityAssessment::lossless()
        );
    }

    #[test]
    fn definition_and_srid_disagreement_is_declared_once() {
        let definition = concat!(
            "PROJCS[\"Monte Mario / Italy zone 1\",",
            "GEOGCS[\"Monte Mario\",AUTHORITY[\"EPSG\",\"4265\"]],",
            "AUTHORITY[\"EPSG\",\"3003\"]]"
        );
        let mut geometry = GeometryColumnContract::wkb_xy(
            FieldId(0),
            "geom",
            ResolvedCrs::new(None, CrsKind::Projected, Some(definition.to_owned())),
            true,
        );
        geometry.srid = Some(4326);
        let layer = layer_with_geometry(geometry);
        let mut report = LossReport::default();

        declare_crs_inconsistency(&layer, &mut report);
        declare_crs_inconsistency(&layer, &mut report);

        assert_eq!(report.counts[INCONSISTENT_CRS_REPRESENTATIONS], 1);
        assert!(report.examples()[0]
            .context
            .contains("definition_epsg=Some(3003)"));
    }

    #[test]
    fn unresolved_definition_and_authority_disagreement_is_declared() {
        let raw = RawCrs::new(
            "GEOGCS[\"WGS 84\",AUTHORITY[\"EPSG\",\"4326\"]]".to_owned(),
            Some("EPSG:3003".to_owned()),
        );
        let geometry = GeometryColumnContract::wkb_xy(
            FieldId(0),
            "geom",
            CrsResolution::DeclaredButUnresolved(raw),
            true,
        );
        let layer = layer_with_geometry(geometry);
        let mut report = LossReport::default();

        declare_crs_inconsistency(&layer, &mut report);

        assert_eq!(report.counts[INCONSISTENT_CRS_REPRESENTATIONS], 1);
    }
}
