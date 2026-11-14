//! Identità logiche persistite nell'estensione metadata OGC, separate dagli indici.
use std::collections::{BTreeMap, BTreeSet};

use arrow_schema::{Field, Schema};
use plenora_io_model::geometry::PLENORA_FIELD_ID_KEY;
use serde::{Deserialize, Serialize};

use super::{err, sql_err, Connection, PublicMessage, Result, PRIMARY_KEY};

const URI: &str = "urn:plenora:gpkg:field-identity:1";
const MAX_DOCUMENT: usize = 1_048_576;

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ColumnIdentity {
    column: String,
    field_id: u32,
    generated: bool,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContractPlan {
    version: u32,
    columns: Vec<ColumnIdentity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    properties: Option<plenora_io_core::property_plan::PropertyPlan>,
}

fn invalid() -> plenora_io_model::PlenoraIoError {
    err(&PublicMessage::Curated(
        "contratto di identita' GeoPackage non valido",
    ))
}

pub fn generated_primary_name(schema: &Schema) -> Option<String> {
    if schema
        .fields()
        .iter()
        .any(|f| f.metadata().contains_key(PRIMARY_KEY))
    {
        return None;
    }
    let mut name = "fid".to_owned();
    while schema
        .fields()
        .iter()
        .any(|f| f.name().eq_ignore_ascii_case(&name))
    {
        name.push('_');
    }
    Some(name)
}

impl ContractPlan {
    pub fn from_schema(schema: &Schema) -> Result<Self> {
        let mut occupied = BTreeSet::new();
        let mut names = BTreeSet::new();
        let mut columns = Vec::new();
        for field in schema.fields() {
            if !names.insert(field.name().to_ascii_lowercase()) {
                return Err(invalid());
            }
            if let Some(id) = field.metadata().get(PLENORA_FIELD_ID_KEY) {
                let number = id.parse::<u32>().map_err(|_| invalid())?;
                if number.to_string() != *id || !occupied.insert(number) {
                    return Err(invalid());
                }
            }
        }
        let mut next = 0_u32;
        for field in schema.fields() {
            let field_id = match field.metadata().get(PLENORA_FIELD_ID_KEY) {
                Some(id) => id.parse().map_err(|_| invalid())?,
                None => allocate(&mut occupied, &mut next)?,
            };
            columns.push(ColumnIdentity {
                column: field.name().clone(),
                field_id,
                generated: false,
            });
        }
        if let Some(column) = generated_primary_name(schema) {
            columns.push(ColumnIdentity {
                column,
                field_id: allocate(&mut occupied, &mut next)?,
                generated: true,
            });
        }
        let plan = Self {
            version: 1,
            columns,
            properties: None,
        };
        plan.encode()?;
        Ok(plan)
    }

    pub fn with_properties(
        mut self,
        properties: plenora_io_core::property_plan::PropertyPlan,
    ) -> Result<Self> {
        self.validate_properties(&properties)?;
        properties.report(0, plenora_io_core::property_plan::Phase::Write)?;
        self.properties = Some(properties);
        self.encode()?;
        Ok(self)
    }

    fn validate_properties(
        &self,
        properties: &plenora_io_core::property_plan::PropertyPlan,
    ) -> Result<()> {
        let original: Vec<_> = self.columns.iter().filter(|c| !c.generated).collect();
        properties.validate("gpkg", original.len())?;
        if properties.effects().iter().any(|e| {
            e.field_index
                .is_some_and(|i| e.field_id.is_some_and(|id| id != original[i].field_id))
        }) {
            return Err(invalid());
        }
        Ok(())
    }

    fn encode(&self) -> Result<String> {
        // Bound before serializing; JSON escaping expands at most sixfold.
        let bound = self
            .columns
            .iter()
            .try_fold(32_usize, |size, c| {
                size.checked_add(c.column.len().checked_mul(6)?.checked_add(80)?)
            })
            .ok_or_else(invalid)?;
        if bound > MAX_DOCUMENT {
            return Err(invalid());
        }
        let document = serde_json::to_string(self).map_err(|_| invalid())?;
        if document.len() > MAX_DOCUMENT {
            return Err(invalid());
        }
        Ok(document)
    }

    pub fn persist(&self, conn: &Connection, table: &str) -> Result<()> {
        let document = self.encode()?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS gpkg_extensions (
                table_name TEXT, column_name TEXT, extension_name TEXT NOT NULL,
                definition TEXT NOT NULL, scope TEXT NOT NULL,
                CONSTRAINT ge_tce UNIQUE (table_name, column_name, extension_name));
             CREATE TABLE IF NOT EXISTS gpkg_metadata (
                id INTEGER CONSTRAINT m_pk PRIMARY KEY ASC NOT NULL,
                md_scope TEXT NOT NULL DEFAULT 'dataset', md_standard_uri TEXT NOT NULL,
                mime_type TEXT NOT NULL DEFAULT 'text/xml', metadata TEXT NOT NULL DEFAULT '');
             CREATE TABLE IF NOT EXISTS gpkg_metadata_reference (
                reference_scope TEXT NOT NULL, table_name TEXT, column_name TEXT,
                row_id_value INTEGER, timestamp DATETIME NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
                md_file_id INTEGER NOT NULL, md_parent_id INTEGER,
                CONSTRAINT crmr_mfi_fk FOREIGN KEY (md_file_id) REFERENCES gpkg_metadata(id),
                CONSTRAINT crmr_mpi_fk FOREIGN KEY (md_parent_id) REFERENCES gpkg_metadata(id));"
        ).map_err(sql_err)?;
        for metadata_table in ["gpkg_metadata", "gpkg_metadata_reference"] {
            conn.execute("INSERT INTO gpkg_extensions SELECT ?1, NULL, 'gpkg_metadata',
                'http://www.geopackage.org/spec140/#extension_metadata', 'read-write'
                WHERE NOT EXISTS (SELECT 1 FROM gpkg_extensions WHERE table_name=?1 AND extension_name='gpkg_metadata')",
                [metadata_table]).map_err(sql_err)?;
        }
        conn.execute("INSERT INTO gpkg_metadata (md_scope, md_standard_uri, mime_type, metadata) VALUES ('featureType', ?1, 'application/json', ?2)",
            rusqlite::params![URI, document]).map_err(sql_err)?;
        let id = conn.last_insert_rowid();
        conn.execute("INSERT INTO gpkg_metadata_reference (reference_scope, table_name, md_file_id) VALUES ('table', ?1, ?2)",
            rusqlite::params![table, id]).map_err(sql_err)?;
        Ok(())
    }
}

fn allocate(occupied: &mut BTreeSet<u32>, next: &mut u32) -> Result<u32> {
    while occupied.contains(next) {
        *next = next.checked_add(1).ok_or_else(invalid)?;
    }
    occupied.insert(*next);
    Ok(*next)
}

pub fn restore_with_properties(
    conn: &Connection,
    table: &str,
    fields: Vec<Field>,
) -> Result<(
    Vec<Field>,
    Option<plenora_io_core::property_plan::PropertyPlan>,
)> {
    if !super::sqlite_table_exists(conn, "gpkg_metadata")? {
        return Ok((
            plenora_io_model::geometry::with_field_identity(fields),
            None,
        ));
    }
    let mut stmt = conn
        .prepare(
            "SELECT length(CAST(m.metadata AS BLOB)), m.id, (r.reference_scope='table' AND r.column_name IS NULL AND r.row_id_value IS NULL AND r.md_parent_id IS NULL) FROM gpkg_metadata m
        JOIN gpkg_metadata_reference r ON r.md_file_id=m.id
        WHERE m.md_standard_uri=?1 AND r.table_name=?2 COLLATE NOCASE LIMIT 2",
        )
        .map_err(sql_err)?;
    let ids = stmt
        .query_map(rusqlite::params![URI, table], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, bool>(2)?,
            ))
        })
        .map_err(sql_err)?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(sql_err)?;
    let Some(&(size, id, valid_scope)) = ids.first() else {
        return Ok((
            plenora_io_model::geometry::with_field_identity(fields),
            None,
        ));
    };
    if !valid_scope
        || ids.len() != 1
        || size < 0
        || size > i64::try_from(MAX_DOCUMENT).map_err(|_| invalid())?
    {
        return Err(invalid());
    }
    let registered: i64 = conn.query_row(
        "SELECT count(DISTINCT table_name) FROM gpkg_extensions WHERE table_name IN ('gpkg_metadata', 'gpkg_metadata_reference') AND column_name IS NULL AND extension_name='gpkg_metadata' AND scope='read-write'",
        [], |r| r.get(0)).map_err(sql_err)?;
    if registered != 2 {
        return Err(invalid());
    }
    let document: String = conn.query_row("SELECT metadata FROM gpkg_metadata WHERE id=?1 AND mime_type='application/json' AND md_scope='featureType'", [id], |r| r.get(0)).map_err(sql_err)?;
    let plan: ContractPlan = plenora_io_model::json::from_str(&document).map_err(|_| invalid())?;
    if plan.version != 1 || plan.columns.len() != fields.len() {
        return Err(invalid());
    }
    if let Some(properties) = &plan.properties {
        plan.validate_properties(properties)?;
        properties.report(0, plenora_io_core::property_plan::Phase::SourceWrite)?;
    }
    let mut ids = BTreeSet::new();
    let mut columns = BTreeMap::new();
    for column in plan.columns {
        if !ids.insert(column.field_id)
            || columns
                .insert(column.column.to_ascii_lowercase(), column.field_id)
                .is_some()
        {
            return Err(invalid());
        }
    }
    let fields = fields
        .into_iter()
        .map(|field| {
            let id = columns
                .remove(&field.name().to_ascii_lowercase())
                .ok_or_else(invalid)?;
            let mut metadata = field.metadata().clone();
            metadata.insert(PLENORA_FIELD_ID_KEY.to_owned(), id.to_string());
            Ok(field.with_metadata(metadata))
        })
        .collect::<Result<Vec<_>>>()?;
    Ok((fields, plan.properties))
}

/// The same immutable plans are validated before staging, persisted by the
/// writer and consumed by the common planned-loss wrapper.
pub fn prepare_write(
    plan: &plenora_io_core::WritePlan,
    opts: &plenora_io_core::WriteOptions,
) -> Result<plenora_io_core::WriteOptions> {
    if !opts.property_plans().is_empty() && opts.property_plans().len() != plan.layers.len() {
        return Err(invalid());
    }
    let mut properties = Vec::new();
    let mut source_report = plenora_io_core::LossReport::default();
    for (index, layer) in plan.layers.iter().enumerate() {
        super::schema::validate_sqlite_metadata(&layer.contract.schema)?;
        let property = opts.property_plan(index).cloned().unwrap_or_else(|| {
            plenora_io_core::property_plan::PropertyPlan::new(&layer.contract.schema, "gpkg", None)
        });
        ContractPlan::from_schema(&layer.contract.schema)?.with_properties(property.clone())?;
        source_report.merge(&property.report(
            index as u64,
            plenora_io_core::property_plan::Phase::SourceWrite,
        )?);
        properties.push(property);
    }
    source_report.validate_property_budget()?;
    let options = opts.clone().with_property_plans(properties);
    plenora_io_core::driver::planned_write_loss(&super::DESCRIPTOR, plan, &options)?;
    Ok(options)
}
