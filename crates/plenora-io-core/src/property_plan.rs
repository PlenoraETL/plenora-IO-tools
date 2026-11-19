//! Inventario dichiarativo: ogni chiave dell'ingresso ha una regola esplicita.
use arrow_schema::{Field, Schema};
use plenora_io_model::geometry::{PLENORA_ENCODING_KEY, PLENORA_SRID_KEY};
use serde::{Deserialize, Serialize};

pub use crate::loss_table::PROPERTIES;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Rule {
    Preserved,
    Rejected,
    Reconstructed,
    Removed,
    ArrowMetadataRemoved,
    EwkbToWkb,
    SridRemoved,
    SridToCatalog,
}

const METADATA_REMOVED: &str = "gpkg_metadata_not_preserved";
const ARROW_METADATA_REMOVED: &str = "arrow_metadata_not_preserved";
const GEOMETRY_NORMALIZED: &str = "geometry_representation_normalized";

impl Rule {
    const fn loss_category(self) -> Option<&'static str> {
        match self {
            Self::Removed => Some(METADATA_REMOVED),
            Self::ArrowMetadataRemoved => Some(ARROW_METADATA_REMOVED),
            Self::EwkbToWkb | Self::SridRemoved | Self::SridToCatalog => Some(GEOMETRY_NORMALIZED),
            _ => None,
        }
    }
}

/// Localizzazione strutturale completa, indipendente dagli esempi bounded.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PropertyEffect {
    pub field_index: Option<usize>,
    pub field_id: Option<u32>,
    /// Indice nella sequenza lessicografica delle chiavi originali, mai il payload.
    pub key_index: usize,
    /// Solo chiavi del vocabolario chiuso; le chiavi opache restano strutturali.
    pub property: Option<String>,
    pub rule: Rule,
}

#[must_use]
pub fn metadata_rule(driver: &str, key: &str, value: &str, convert_ewkb: bool) -> Rule {
    if convert_ewkb && key == PLENORA_ENCODING_KEY && value == "ewkb" {
        return Rule::EwkbToWkb;
    }
    if convert_ewkb && key == PLENORA_SRID_KEY {
        return if driver == "gpkg" {
            Rule::SridToCatalog
        } else {
            Rule::SridRemoved
        };
    }
    crate::loss_table::metadata_rule(driver, crate::loss_table::TransportDirection::Write, key)
}

#[must_use]
pub fn plan_metadata(
    schema: &Schema,
    driver: &str,
    converted_field: Option<usize>,
) -> Vec<PropertyEffect> {
    let mut effects = Vec::new();
    for (field_index, metadata) in std::iter::once((None, schema.metadata())).chain(
        schema
            .fields()
            .iter()
            .enumerate()
            .map(|(i, f)| (Some(i), f.metadata())),
    ) {
        let mut keys: Vec<_> = metadata.keys().collect();
        keys.sort_unstable();
        for (key_index, key) in keys.into_iter().enumerate() {
            effects.push(PropertyEffect {
                field_index,
                field_id: field_index.and_then(|i| {
                    schema
                        .field(i)
                        .metadata()
                        .get("plenora.field_id")
                        .and_then(|id| id.parse().ok())
                }),
                key_index,
                property: PROPERTIES
                    .iter()
                    .find(|(_, pattern, _, _)| matches_key(pattern, key))
                    .filter(|(name, _, _, _)| !name.is_empty())
                    .map(|(name, _, _, _)| (*name).to_owned()),
                rule: metadata_rule(
                    driver,
                    key,
                    &metadata[key],
                    converted_field.is_some() && converted_field == field_index,
                ),
            });
        }
    }
    effects
}

/// La stessa regola governa la modifica dello schema e l'inventario del piano.
#[must_use]
pub fn field_in_wkb(field: &Field) -> Field {
    let metadata: std::collections::HashMap<String, String> = field
        .metadata()
        .iter()
        .filter_map(|(key, value)| match metadata_rule("", key, value, true) {
            Rule::SridRemoved | Rule::SridToCatalog => None,
            Rule::EwkbToWkb => Some((key.clone(), "wkb".to_owned())),
            _ => Some((key.clone(), value.clone())),
        })
        .collect();
    field.clone().with_metadata(metadata)
}

fn matches_key(pattern: &str, key: &str) -> bool {
    pattern.is_empty() || pattern == key || (pattern.ends_with('.') && key.starts_with(pattern))
}

/// Complete structural plan. No input name, metadata value or digest is carried.
/// Covers format metadata and the EWKB representation adapter.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PropertyPlan {
    driver: String,
    effects: Vec<PropertyEffect>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Write,
    SourceWrite,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PropertyChange {
    pub layer_index: u64,
    pub phase: Phase,
    pub effect: PropertyEffect,
}

/// Reserved within the existing 12 KiB loss section. Never truncated.
pub const MAX_PROPERTY_REPORT_BYTES: usize = 8192;

impl PropertyPlan {
    #[must_use]
    pub fn new(schema: &Schema, driver: &str, converted_field: Option<usize>) -> Self {
        Self {
            driver: driver.to_owned(),
            effects: plan_metadata(schema, driver, converted_field),
        }
    }

    #[must_use]
    pub fn effects(&self) -> &[PropertyEffect] {
        &self.effects
    }

    /// Validate persisted plans before exposing any diagnostic text.
    /// # Errors
    /// Rejects a driver mismatch, invalid locations or non-vocabulary properties.
    pub fn validate(&self, driver: &str, fields: usize) -> plenora_io_model::Result<()> {
        let mut next = std::collections::BTreeMap::new();
        let mut identities = std::collections::BTreeMap::new();
        if self.driver != driver
            || self.effects.iter().any(|effect| {
                effect.field_index.is_some_and(|i| i >= fields)
                    || effect
                        .property
                        .as_ref()
                        .is_some_and(|p| !PROPERTIES.iter().any(|(name, _, _, _)| name == p))
                    || {
                        let index = next.entry(effect.field_index).or_insert(0);
                        let invalid = effect.key_index != *index;
                        *index += 1;
                        invalid
                    }
                    || (effect.field_index.is_none() && effect.field_id.is_some())
                    || identities
                        .insert(effect.field_index, effect.field_id)
                        .is_some_and(|id| id != effect.field_id)
                    || (effect.field_index.is_some()
                        && effect.property.as_deref() == Some("field_id")
                        && effect.field_id.is_none())
                    || !valid_rule(driver, effect)
                    || effect.rule == Rule::Rejected
                    || (driver == "gpkg" && effect.rule == Rule::SridRemoved)
            })
        {
            return Err(plenora_io_model::PlenoraIoError::contratto_redatto(
                &plenora_io_model::PublicMessage::Curated("piano delle proprieta non valido"),
            ));
        }
        Ok(())
    }

    /// Bind a supplied plan to the actual schema before any writer effect.
    /// Only the known EWKB adapter may have changed encoding and removed SRID.
    /// # Errors
    /// Rejects missing, invented or mislocalized effects.
    pub fn validate_schema(&self, driver: &str, schema: &Schema) -> plenora_io_model::Result<()> {
        self.validate(driver, schema.fields().len())?;
        let converted: Vec<_> = self
            .effects
            .iter()
            .filter(|e| e.rule == Rule::EwkbToWkb)
            .collect();
        let converted_field = converted.first().and_then(|e| e.field_index);
        let mut fields: Vec<_> = schema.fields().iter().map(|f| f.as_ref().clone()).collect();
        if let Some(index) = converted_field {
            let mut metadata = fields[index].metadata().clone();
            if converted.len() != 1
                || metadata.get(PLENORA_ENCODING_KEY).map(String::as_str) != Some("wkb")
            {
                return Err(invalid_plan());
            }
            metadata.insert(PLENORA_ENCODING_KEY.to_owned(), "ewkb".to_owned());
            if self.effects.iter().any(|e| {
                e.field_index == Some(index)
                    && matches!(e.rule, Rule::SridRemoved | Rule::SridToCatalog)
            }) {
                metadata.insert(PLENORA_SRID_KEY.to_owned(), "0".to_owned());
            }
            fields[index] = fields[index].clone().with_metadata(metadata);
        }
        let original = Schema::new_with_metadata(fields, schema.metadata().clone());
        if self.effects != plan_metadata(&original, driver, converted_field) {
            return Err(invalid_plan());
        }
        Ok(())
    }

    /// Representation effects and metadata counts with bounded indexed examples.
    /// # Errors
    /// Rejects reports exceeding the complete-inventory byte budget.
    pub fn report(
        &self,
        layer_index: u64,
        phase: Phase,
    ) -> plenora_io_model::Result<crate::LossReport> {
        let mut report = crate::LossReport::default();
        for effect in &self.effects {
            if effect.rule == Rule::Preserved {
                continue;
            }
            if effect.rule != Rule::ArrowMetadataRemoved {
                report.property_changes.push(PropertyChange {
                    layer_index,
                    phase,
                    effect: effect.clone(),
                });
            }
            let rule = effect.rule;
            // The complete transport inventory remains available; loss counts need observation.
            if matches!(rule, Rule::Removed | Rule::ArrowMetadataRemoved) {
                continue;
            }
            let Some(category) = rule.loss_category() else {
                continue;
            };
            // Persisted write provenance is visible on read, but is not counted
            // a second time as a new loss incurred by that read.
            if phase == Phase::Write {
                report.record(category, 1);
                report.add_example(crate::loss::LossExample {
                    category: category.to_owned(),
                    posizione: crate::loss::Posizione {
                        layer_index: Some(layer_index),
                        field_index: effect.field_index.map(|i| i as u64),
                        type_class: None,
                    },
                    context: format!(
                        "scope={} key_index={} field_id={} {} phase=write",
                        if effect.field_index.is_some() {
                            "field"
                        } else {
                            "schema"
                        },
                        effect.key_index,
                        effect.field_id.map_or_else(|| "absent".to_owned(), |id| id.to_string()),
                        match effect.rule {
                            Rule::ArrowMetadataRemoved => "rule=native_schema_reconstruction",
                            Rule::EwkbToWkb => "encoding=ewkb_to_iso_wkb byte_order=little_endian coordinates=bitwise",
                            Rule::SridToCatalog => "srid=gpkg_catalog payload_srid=removed",
                            Rule::SridRemoved => "srid=removed crs=validated",
                            _ => "rule=removed",
                        }
                    ),
                });
            }
        }
        report.validate_property_budget()?;
        Ok(report)
    }
}

fn invalid_plan() -> plenora_io_model::PlenoraIoError {
    plenora_io_model::PlenoraIoError::contratto_redatto(&plenora_io_model::PublicMessage::Curated(
        "piano delle proprieta non valido",
    ))
}

fn valid_rule(driver: &str, effect: &PropertyEffect) -> bool {
    match effect.rule {
        Rule::EwkbToWkb => {
            effect.field_index.is_some() && effect.property.as_deref() == Some("encoding")
        }
        Rule::SridRemoved | Rule::SridToCatalog => {
            effect.field_index.is_some()
                && effect.property.as_deref() == Some("srid")
                && (driver == "gpkg") == (effect.rule == Rule::SridToCatalog)
        }
        _ if driver != "gpkg" => {
            effect.rule
                == crate::loss_table::metadata_rule(
                    driver,
                    crate::loss_table::TransportDirection::Write,
                    "",
                )
        }
        _ => PROPERTIES.iter().any(|(name, _, rule, _)| {
            effect.property.as_deref() == (!name.is_empty()).then_some(*name)
                && effect.rule == *rule
        }),
    }
}

/// The writer takes SQL/contract metadata from this projection of the same rules
/// that produced the original-input report. Opaque keys never reach the codec.
#[must_use]
pub fn gpkg_write_schema(schema: &Schema) -> Schema {
    let filter = |metadata: &arrow_schema::Metadata| -> arrow_schema::Metadata {
        metadata
            .iter()
            .filter(|(key, value)| metadata_rule("gpkg", key, value, false) != Rule::Removed)
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect()
    };
    Schema::new_with_metadata(
        schema
            .fields()
            .iter()
            .map(|f| f.as_ref().clone().with_metadata(filter(f.metadata())))
            .collect::<Vec<_>>(),
        filter(schema.metadata()),
    )
}

/// New native metadata emitted by the reader must be classified in the table.
/// External files do not manufacture a history of missing Arrow source metadata.
/// # Errors
/// Rejects metadata emitted outside the declared reader vocabulary.
pub fn validate_gpkg_read_schema(schema: &Schema) -> plenora_io_model::Result<()> {
    for metadata in
        std::iter::once(schema.metadata()).chain(schema.fields().iter().map(|f| f.metadata()))
    {
        for key in metadata.keys() {
            let rule = crate::loss_table::metadata_rule(
                "gpkg",
                crate::loss_table::TransportDirection::Read,
                key,
            );
            if rule == Rule::Rejected {
                return Err(plenora_io_model::PlenoraIoError::contratto_redatto(
                    &plenora_io_model::PublicMessage::Curated(
                        "metadato di lettura non censito nel piano GeoPackage",
                    ),
                ));
            }
        }
    }
    Ok(())
}

/// Metadata evidence collected from the completed native file before publication.
/// A plan alone cannot establish whether reconstruction changed a source key.
pub struct NativeMetadataCheck {
    contracts: Vec<plenora_io_model::contract::DataContract>,
    names: Vec<String>,
    context: plenora_io_model::budget::PipelineContext,
    options: std::collections::BTreeMap<String, String>,
    attribute_source: std::sync::Mutex<Option<crate::driver::spool::StagedSpool>>,
    budget: plenora_io_model::budget::OperationBudget,
}

impl NativeMetadataCheck {
    #[must_use]
    pub fn new(plan: &crate::WritePlan, opts: &crate::WriteOptions) -> Self {
        Self {
            contracts: plan.layers.iter().map(|l| l.contract.clone()).collect(),
            names: plan.layers.iter().map(|l| l.name.clone()).collect(),
            context: opts.budget().context().clone(),
            options: opts.format_options.clone(),
            attribute_source: std::sync::Mutex::new(None),
            budget: opts.budget().clone(),
        }
    }

    /// Retain the original attributes under the existing memory/spill quotas.
    /// # Errors
    /// Propagates budget, cancellation and spool errors before publication.
    pub fn observe_attributes(
        &mut self,
        batch: &arrow_array::RecordBatch,
    ) -> plenora_io_model::Result<()> {
        crate::driver::validate_external_batch(batch, plenora_io_model::ErrorPhase::Write)?;
        let indices: Vec<_> = batch
            .schema()
            .fields()
            .iter()
            .enumerate()
            .filter(|(_, field)| !plenora_io_model::geometry::is_geometry_field(field))
            .map(|(i, _)| i)
            .collect();
        if indices.is_empty() || batch.num_rows() == 0 {
            return Ok(());
        }
        let attributes = batch.project(&indices).map_err(|_| invalid_plan())?;
        let lease = self
            .context
            .lease_memory_internal((attributes.get_array_memory_size() as u64).max(1))?;
        let spool = self
            .attribute_source
            .get_mut()
            .map_err(|_| invalid_plan())?;
        let spool = spool.get_or_insert_with(|| {
            crate::driver::spool::StagedSpool::new(
                attributes.schema(),
                self.budget.clone(),
                self.context.cancellation().clone(),
            )
        });
        spool.push(attributes, lease)
    }

    fn compare_attributes(
        &self,
        dataset: &dyn crate::OpenDatasetHandle,
        report: &mut crate::LossReport,
        native: crate::loss_table::NativeDriver,
    ) -> plenora_io_model::Result<()> {
        let Some(mut spool) = self
            .attribute_source
            .lock()
            .map_err(|_| invalid_plan())?
            .take()
        else {
            return Ok(());
        };
        spool.seal()?;
        let mut reader = dataset.open_layer_reader(&crate::ReadRequest {
            layer: dataset.layers().first().ok_or_else(invalid_plan)?.id,
            projected_fields: None,
            projection_mode: crate::ProjectionMode::BestEffort,
            pruning_predicate: None,
            spatial_pruning_hint: None,
            scope: crate::ReadScope::default(),
            batch_target: crate::BatchTarget::default(),
            cancellation: self.context.cancellation().clone(),
        })?;
        let mut output = reader.next_batch()?;
        let mut row = 0;
        while let Some(source) = spool.next_batch()? {
            for input_row in 0..source.num_rows() {
                self.context.ensure_active()?;
                while output.as_ref().is_some_and(|b| row == b.num_rows()) {
                    output = reader.next_batch()?;
                    row = 0;
                }
                for (column, field) in source.schema().fields().iter().enumerate() {
                    let equal = output.as_ref().is_some_and(|batch| {
                        batch.schema().index_of(field.name()).is_ok_and(|i| {
                            source.column(column).slice(input_row, 1).to_data()
                                == batch.column(i).slice(row, 1).to_data()
                        })
                    });
                    let field_index = self.contracts[0]
                        .schema
                        .index_of(field.name())
                        .ok()
                        .map(|i| i as u64);
                    crate::loss_table::observe_native(
                        report,
                        (native, crate::loss_table::TransportDirection::Write),
                        crate::loss_table::NativeElement::Utf8,
                        u64::from(!equal),
                        0,
                        field_index,
                    );
                }
                if output.is_some() {
                    row += 1;
                }
            }
        }
        // The reverse direction matters too: exploded geometries can create
        // additional native attribute rows after the source has been consumed.
        while let Some(batch) = output {
            self.context.ensure_active()?;
            let extra = batch.num_rows().saturating_sub(row) as u64;
            for (index, field) in self.contracts[0].schema.fields().iter().enumerate() {
                if !plenora_io_model::geometry::is_geometry_field(field) {
                    crate::loss_table::observe_native(
                        report,
                        (native, crate::loss_table::TransportDirection::Write),
                        crate::loss_table::NativeElement::Utf8,
                        extra,
                        0,
                        Some(index as u64),
                    );
                }
            }
            output = reader.next_batch()?;
            row = 0;
        }
        Ok(())
    }

    /// # Errors
    /// Propagates failure to inspect the completed output; no success is published.
    #[allow(clippy::too_many_lines)] // Decode options and source-layer matching are part of the comparison boundary.
    pub fn compare(
        &self,
        driver: &dyn crate::FormatDriver,
        native: crate::loss_table::NativeDriver,
        path: &std::path::Path,
    ) -> plenora_io_model::Result<crate::LossReport> {
        use crate::loss_table::{metadata_changes, TransportDirection};
        self.context.ensure_active()?;
        let mut builder = plenora_io_model::budget::PipelineBudget::builder()
            .limits((*self.context.limits()).with_duration_ms(
                self.context.remaining_duration().map_or(1, |d| {
                    u64::try_from(d.as_millis()).unwrap_or(u64::MAX).max(1)
                }),
            ))
            .cancellation(self.context.cancellation().clone());
        if let Some(pool) = self.context.resource_pool() {
            builder = builder.resource_pool(pool.clone());
        }
        let mut options = crate::ReadOptions::from_read_parts(builder.build()?.into_read_parts());
        if matches!(driver.descriptor().id(), "csv" | "xls") {
            if self
                .options
                .get("geometry_encoding")
                .is_some_and(|s| s == "xy")
            {
                options.assume_crs = Some("EPSG:4326".to_owned());
                options.format_options.insert("x_column".into(), "x".into());
                options.format_options.insert("y_column".into(), "y".into());
            } else if self
                .contracts
                .iter()
                .any(|contract| contract.geometry.is_some())
            {
                options.assume_crs = Some("EPSG:4326".to_owned());
                options
                    .format_options
                    .insert("wkt_column".into(), "geometry".into());
            }
            if driver.descriptor().id() == "csv" {
                if let Some(delimiter) = self.options.get("delimiter") {
                    options
                        .format_options
                        .insert("delimiter".into(), delimiter.clone());
                }
            }
        }
        let assumed_for_decoding = (matches!(driver.descriptor().id(), "csv" | "xls")
            && options.assume_crs.is_some())
            || (driver.descriptor().id() == "shp" && !path.with_extension("prj").exists());
        if assumed_for_decoding {
            options.assume_crs = Some("EPSG:4326".to_owned());
        }
        let dataset = driver.open(crate::Source::Path(path.to_owned()), options)?;
        let mut report = crate::LossReport::default();
        self.compare_attributes(dataset.as_ref(), &mut report, native)?;
        for (index, source) in self.contracts.iter().enumerate() {
            let before = &source.schema;
            let after = (if native == crate::loss_table::NativeDriver::FileGdb {
                dataset.layers().get(index)
            } else {
                dataset
                    .layers()
                    .iter()
                    .find(|layer| layer.name == self.names[index])
                    .or_else(|| {
                        (self.contracts.len() == 1)
                            .then(|| dataset.layers().first())
                            .flatten()
                    })
            })
            .ok_or_else(|| {
                plenora_io_model::PlenoraIoError::contratto_redatto(
                    &plenora_io_model::PublicMessage::Curated(
                        "layer scritto assente alla rilettura",
                    ),
                )
            })?;
            if matches!(
                native,
                crate::loss_table::NativeDriver::FileGdb | crate::loss_table::NativeDriver::Gpkg
            ) && after.name != self.names[index]
            {
                report.record("native_layer_metadata_not_preserved", 1);
                report.add_example(crate::loss::LossExample {
                    category: "native_layer_metadata_not_preserved".to_owned(),
                    posizione: crate::loss::Posizione {
                        layer_index: Some(index as u64),
                        field_index: None,
                        type_class: None,
                    },
                    context: "nome del layer diverso alla rilettura; fase write".to_owned(),
                });
            }
            let mut actual = after.contract.clone();
            if assumed_for_decoding {
                if let Some(geometry) = &mut actual.geometry {
                    geometry.crs = plenora_io_model::crs::CrsResolution::Missing;
                    geometry.srid = None;
                    let fields = actual
                        .schema
                        .fields()
                        .iter()
                        .map(|field| {
                            if field.name() == &geometry.name {
                                let mut field =
                                    plenora_io_model::geometry::with_geometry_contract_metadata(
                                        field, geometry,
                                    );
                                let mut metadata = field.metadata().clone();
                                metadata.remove("crs");
                                field = field.with_metadata(metadata);
                                field
                            } else {
                                field.as_ref().clone()
                            }
                        })
                        .collect::<Vec<_>>();
                    actual.schema = std::sync::Arc::new(arrow_schema::Schema::new_with_metadata(
                        fields,
                        actual.schema.metadata().clone(),
                    ));
                }
            }
            report.merge(&crate::driver::observed_geometry_metadata_loss(
                source,
                &actual,
                driver.descriptor(),
                index as u64,
            )?);
            let delivered = plenora_io_model::geometry::interchange_schema(&actual)?;
            let changes = metadata_changes(native, TransportDirection::Write, before, &delivered);
            for (category, count) in &changes.counts {
                report.record(category, *count);
            }
            for example in changes.esempi_canonici() {
                let mut example = example.clone();
                example.posizione.layer_index = Some(index as u64);
                report.add_example(example);
            }
        }
        self.context.ensure_active()?;
        Ok(report)
    }
}
