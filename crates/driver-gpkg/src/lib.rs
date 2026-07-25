//! driver-gpkg — GeoPackage ⇄ RecordBatch (Fase 1 + ottimizzazione mirata).
//! Geometria WKB nativa: blob `GeoPackageBinaryHeader` + payload WKB, passato
//! senza decodifica (V4). Multi-layer.
//!
//! Prestazioni (misurate contro la baseline): la scrittura usa una singola
//! transazione + `synchronous=OFF`/`journal_mode=MEMORY` (sicuro perché il
//! tempfile è pubblicato atomicamente solo a `finish`) + statement preparato +
//! streaming per batch. La lettura è a pagine (keyset su `rowid`) con builder
//! Arrow tipizzati: memoria O(batch), non O(tabella).
#![forbid(unsafe_code)]

use std::path::PathBuf;
use std::sync::Arc;

use arrow_array::builder::{BinaryBuilder, Float64Builder, Int64Builder, StringBuilder};
use arrow_array::{Array, ArrayRef, BinaryArray, BooleanArray, Float64Array, Int64Array,
    RecordBatch, StringArray};
use arrow_schema::{DataType, Field, Schema, SchemaRef};
use rusqlite::types::{Value, ValueRef};
use rusqlite::Connection;

use driver_common::geometry_field;
use plenora_core::contract::{DataContract, FieldId, GeometryColumnContract, LayerContract, LayerId};
use plenora_core::crs::{CrsKind, ResolvedCrs};
use plenora_core::geometry::is_geometry_field;
use plenora_core::{PlenoraError, Result};
use plenora_io_core::descriptor::{
    CrsHandling, Direction, Fidelity, FormatDescriptor, ReadMode, ReaderConcurrency, Runtime,
    WriteMode,
};
use plenora_io_core::driver::{
    FormatDriver, FormatWriter, LayerReader, OpenDatasetHandle, Published, ReadOptions, Sink,
    Source, WriteOptions,
};
use plenora_io_core::loss::LossReport;
use plenora_io_core::publish::publish_file_atomic;
use plenora_io_core::request::ReadRequest;
use plenora_io_core::WritePlan;

fn err(reason: impl Into<String>) -> PlenoraError {
    PlenoraError::Format {
        driver: "gpkg",
        reason: reason.into(),
    }
}

fn sql_err(e: rusqlite::Error) -> PlenoraError {
    err(format!("sqlite: {e}"))
}

static DESCRIPTOR: FormatDescriptor = FormatDescriptor {
    id: "gpkg",
    direction: Direction::Bidirectional,
    read_mode: ReadMode::StreamingSequential, // pagine keyset, O(batch)
    write_mode: Some(WriteMode::Streaming),    // per batch, in transazione
    multi_layer: true,
    multi_file: false,
    reader_concurrency: ReaderConcurrency::MultipleIndependentReaders,
    crs_handling: CrsHandling::Embedded,
    fidelity_class: Fidelity::Conditional,
    runtime: Runtime::PureRust,
    semantic_version: 1,
    driver_version: 2,
    descriptor_version: 1,
};

pub struct GpkgDriver;

impl FormatDriver for GpkgDriver {
    fn descriptor(&self) -> &FormatDescriptor {
        &DESCRIPTOR
    }

    fn open(&self, source: Source, _opts: &ReadOptions) -> Result<Box<dyn OpenDatasetHandle>> {
        let Source::Path(path) = source;
        let conn = Connection::open(&path).map_err(sql_err)?;
        let tables = feature_tables(&conn)?;
        if tables.is_empty() {
            return Err(err(
                "nessuna feature table (gpkg_contents data_type='features')",
            ));
        }
        let mut layers = Vec::new();
        let mut metas = Vec::new();
        for (i, (table, geom_col, srs_id)) in tables.into_iter().enumerate() {
            let crs = crs_for(&conn, srs_id)?;
            let (schema, attrs) = build_schema(&conn, &table, &geom_col, &crs)?;
            let contract = DataContract {
                schema: schema.clone(),
                geometry: Some(GeometryColumnContract {
                    field_id: FieldId(0),
                    name: geom_col.clone(),
                    crs,
                    nullable: true,
                }),
            };
            layers.push(LayerContract {
                id: LayerId(i as u32),
                name: table.clone(),
                contract,
            });
            metas.push(LayerRead {
                table,
                geom_col,
                schema,
                attrs,
            });
        }
        Ok(Box::new(GpkgDataset {
            path,
            layers,
            metas,
        }))
    }

    fn create(
        &self,
        sink: Sink,
        plan: &WritePlan,
        opts: &WriteOptions,
    ) -> Result<Box<dyn FormatWriter>> {
        let Sink::Path(path) = sink;
        if path.exists() {
            return Err(PlenoraError::OutputExists(path.display().to_string()));
        }
        if !path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("gpkg"))
        {
            return Err(PlenoraError::Unsupported(
                "l'output deve avere estensione .gpkg".to_owned(),
            ));
        }
        let mut names = std::collections::BTreeSet::new();
        for l in &plan.layers {
            if !names.insert(l.name.clone()) {
                return Err(err(format!("nome layer duplicato: {}", l.name)));
            }
            if geometry_index(&l.contract.schema).is_none() {
                return Err(err(format!("layer '{}' senza colonna geometria", l.name)));
            }
        }
        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."));
        let temp = tempfile::Builder::new()
            .suffix(".gpkg")
            .tempfile_in(&parent)?;
        let conn = Connection::open(temp.path()).map_err(sql_err)?;
        // Bulk-load veloce: la durabilità è garantita dal publish atomico, non
        // dal file temporaneo (un crash a metà non pubblica nulla).
        conn.execute_batch("PRAGMA synchronous = OFF; PRAGMA journal_mode = MEMORY;")
            .map_err(sql_err)?;
        init_gpkg(&conn)?;
        let mut layers: Vec<ActiveLayer> = Vec::with_capacity(plan.layers.len());
        for l in &plan.layers {
            let geom_idx = geometry_index(&l.contract.schema)
                .ok_or_else(|| err(format!("layer '{}' senza colonna geometria", l.name)))?;
            let (crs_id, crs_def) = layer_crs(l, geom_idx);
            let srs_id = register_srs(&conn, crs_id.as_deref(), crs_def.as_deref())?;
            layers.push(create_feature_table(&conn, &l.name, &l.contract.schema, srs_id)?);
        }
        if layers.is_empty() {
            return Err(err("WritePlan senza layer"));
        }
        conn.execute_batch("BEGIN").map_err(sql_err)?;
        Ok(Box::new(GpkgWriter {
            temp: Some(temp),
            conn: Some(conn),
            path,
            durable: opts.durable,
            layers,
        }))
    }
}

// --- lettura (streaming a pagine) -----------------------------------------

struct LayerRead {
    table: String,
    geom_col: String,
    schema: SchemaRef,
    attrs: Vec<(String, DataType)>,
}

struct GpkgDataset {
    path: PathBuf,
    layers: Vec<LayerContract>,
    metas: Vec<LayerRead>,
}

impl OpenDatasetHandle for GpkgDataset {
    fn layers(&self) -> &[LayerContract] {
        &self.layers
    }

    fn open_layer_reader(&self, request: &ReadRequest) -> Result<Box<dyn LayerReader>> {
        let idx = self
            .layers
            .iter()
            .position(|l| l.id.0 == request.layer.0)
            .ok_or_else(|| err(format!("layer {} inesistente", request.layer.0)))?;
        let m = &self.metas[idx];
        let conn = Connection::open(&self.path).map_err(sql_err)?;
        let attr_cols: Vec<String> = m
            .attrs
            .iter()
            .map(|(n, _)| format!("\"{}\"", n.replace('"', "\"\"")))
            .collect();
        let select = if attr_cols.is_empty() {
            format!("\"{}\"", m.geom_col.replace('"', "\"\""))
        } else {
            format!(
                "\"{}\", {}",
                m.geom_col.replace('"', "\"\""),
                attr_cols.join(", ")
            )
        };
        let sql = format!(
            "SELECT rowid, {} FROM \"{}\" WHERE rowid > ?1 ORDER BY rowid LIMIT ?2",
            select,
            m.table.replace('"', "\"\"")
        );
        Ok(Box::new(GpkgReader {
            conn,
            sql,
            schema: m.schema.clone(),
            attrs: m.attrs.clone(),
            batch_size: request.batch_target.max_rows.max(1) as i64,
            last_rowid: 0,
            layer: self.layers[idx].clone(),
        }))
    }
}

struct GpkgReader {
    conn: Connection,
    sql: String,
    schema: SchemaRef,
    attrs: Vec<(String, DataType)>,
    batch_size: i64,
    last_rowid: i64,
    layer: LayerContract,
}

impl LayerReader for GpkgReader {
    fn contract(&self) -> &LayerContract {
        &self.layer
    }

    fn next_batch(&mut self) -> Result<Option<RecordBatch>> {
        let mut stmt = self.conn.prepare_cached(&self.sql).map_err(sql_err)?;
        let mut geom = BinaryBuilder::new();
        let mut attr_builders: Vec<ColBuilder> =
            self.attrs.iter().map(|(_, dt)| ColBuilder::new(dt)).collect();
        let mut count = 0usize;
        let mut max_rowid = self.last_rowid;
        let mut rows = stmt
            .query(rusqlite::params![self.last_rowid, self.batch_size])
            .map_err(sql_err)?;
        while let Some(row) = rows.next().map_err(sql_err)? {
            let rowid: i64 = row.get(0).map_err(sql_err)?;
            max_rowid = max_rowid.max(rowid);
            match row.get_ref(1).map_err(sql_err)? {
                ValueRef::Null => geom.append_null(),
                ValueRef::Blob(b) => geom.append_value(strip_gpkg_header(b)?),
                _ => return Err(err("colonna geometria non è un BLOB")),
            }
            for (i, b) in attr_builders.iter_mut().enumerate() {
                b.append(row.get_ref(2 + i).map_err(sql_err)?);
            }
            count += 1;
        }
        if count == 0 {
            return Ok(None);
        }
        self.last_rowid = max_rowid;
        let mut arrays: Vec<ArrayRef> = Vec::with_capacity(1 + attr_builders.len());
        arrays.push(Arc::new(geom.finish()));
        for b in attr_builders {
            arrays.push(b.finish());
        }
        let batch = RecordBatch::try_new(self.schema.clone(), arrays)
            .map_err(|e| err(format!("batch: {e}")))?;
        Ok(Some(batch))
    }
}

enum ColBuilder {
    I64(Int64Builder),
    F64(Float64Builder),
    Str(StringBuilder),
    Bin(BinaryBuilder),
}

impl ColBuilder {
    fn new(dt: &DataType) -> Self {
        match dt {
            DataType::Int64 => ColBuilder::I64(Int64Builder::new()),
            DataType::Float64 => ColBuilder::F64(Float64Builder::new()),
            DataType::Binary => ColBuilder::Bin(BinaryBuilder::new()),
            _ => ColBuilder::Str(StringBuilder::new()),
        }
    }
    fn append(&mut self, v: ValueRef) {
        match self {
            ColBuilder::I64(b) => match v {
                ValueRef::Integer(i) => b.append_value(i),
                ValueRef::Real(r) => b.append_value(r as i64),
                _ => b.append_null(),
            },
            ColBuilder::F64(b) => match v {
                ValueRef::Real(r) => b.append_value(r),
                ValueRef::Integer(i) => b.append_value(i as f64),
                _ => b.append_null(),
            },
            ColBuilder::Str(b) => match v {
                ValueRef::Text(t) => b.append_value(String::from_utf8_lossy(t)),
                ValueRef::Null => b.append_null(),
                ValueRef::Integer(i) => b.append_value(i.to_string()),
                ValueRef::Real(r) => b.append_value(r.to_string()),
                ValueRef::Blob(_) => b.append_null(),
            },
            ColBuilder::Bin(b) => match v {
                ValueRef::Blob(x) => b.append_value(x),
                _ => b.append_null(),
            },
        }
    }
    fn finish(mut self) -> ArrayRef {
        match &mut self {
            ColBuilder::I64(b) => Arc::new(b.finish()),
            ColBuilder::F64(b) => Arc::new(b.finish()),
            ColBuilder::Str(b) => Arc::new(b.finish()),
            ColBuilder::Bin(b) => Arc::new(b.finish()),
        }
    }
}

// --- scrittura (transazione + streaming) -----------------------------------

struct ActiveLayer {
    geom_idx: usize,
    insert_sql: String,
    srs_id: i32,
}

struct GpkgWriter {
    temp: Option<tempfile::NamedTempFile>,
    conn: Option<Connection>,
    path: PathBuf,
    durable: bool,
    layers: Vec<ActiveLayer>,
}

impl FormatWriter for GpkgWriter {
    fn write(&mut self, batch: &RecordBatch) -> Result<()> {
        self.write_to_layer(LayerId(0), batch)
    }

    fn write_to_layer(&mut self, layer: LayerId, batch: &RecordBatch) -> Result<()> {
        let conn = self.conn.as_ref().ok_or_else(|| err("writer chiuso"))?;
        let a = self.layers.get(layer.0 as usize).ok_or_else(|| {
            err(format!("layer {} inesistente nel piano di scrittura", layer.0))
        })?;
        insert_batch(conn, a, batch)
    }

    fn finish(mut self: Box<Self>) -> Result<Published> {
        let conn = self.conn.take().ok_or_else(|| err("writer già chiuso"))?;
        conn.execute_batch("COMMIT").map_err(sql_err)?;
        drop(conn);
        let temp = self.temp.take().ok_or_else(|| err("temp mancante"))?;
        let (bytes, outcome) = publish_file_atomic(temp, &self.path, self.durable)?;
        Ok(Published {
            bytes,
            loss: LossReport::default(),
            outcome,
        })
    }
}

fn insert_batch(conn: &Connection, a: &ActiveLayer, batch: &RecordBatch) -> Result<()> {
    let geom_col = batch
        .column(a.geom_idx)
        .as_any()
        .downcast_ref::<BinaryArray>()
        .ok_or_else(|| err("colonna geometria non binaria"))?;
    let attr_idx: Vec<usize> = (0..batch.num_columns()).filter(|i| *i != a.geom_idx).collect();
    let mut stmt = conn.prepare_cached(&a.insert_sql).map_err(sql_err)?;
    for row in 0..batch.num_rows() {
        let mut params: Vec<Value> = Vec::with_capacity(1 + attr_idx.len());
        if geom_col.is_null(row) {
            params.push(Value::Null);
        } else {
            let mut blob = gpkg_header(a.srs_id).to_vec();
            blob.extend_from_slice(geom_col.value(row));
            params.push(Value::Blob(blob));
        }
        for &i in &attr_idx {
            params.push(arrow_cell_to_sql(batch.column(i), row));
        }
        stmt.execute(rusqlite::params_from_iter(params.iter()))
            .map_err(sql_err)?;
    }
    Ok(())
}

fn arrow_cell_to_sql(array: &ArrayRef, row: usize) -> Value {
    if array.is_null(row) {
        return Value::Null;
    }
    let a = array.as_any();
    if let Some(x) = a.downcast_ref::<Int64Array>() {
        return Value::Integer(x.value(row));
    }
    if let Some(x) = a.downcast_ref::<Float64Array>() {
        return Value::Real(x.value(row));
    }
    if let Some(x) = a.downcast_ref::<BooleanArray>() {
        return Value::Integer(x.value(row) as i64);
    }
    if let Some(x) = a.downcast_ref::<StringArray>() {
        return Value::Text(x.value(row).to_owned());
    }
    Value::Null
}

// --- helpers comuni --------------------------------------------------------

fn feature_tables(conn: &Connection) -> Result<Vec<(String, String, i64)>> {
    let mut out = Vec::new();
    let mut stmt = conn
        .prepare(
            "SELECT c.table_name, g.column_name, g.srs_id
             FROM gpkg_contents c JOIN gpkg_geometry_columns g ON c.table_name = g.table_name
             WHERE c.data_type = 'features' ORDER BY c.table_name",
        )
        .map_err(sql_err)?;
    let rows = stmt
        .query_map([], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)?))
        })
        .map_err(sql_err)?;
    for r in rows {
        out.push(r.map_err(sql_err)?);
    }
    Ok(out)
}

fn crs_for(conn: &Connection, srs_id: i64) -> Result<ResolvedCrs> {
    if srs_id == 4326 {
        return Ok(ResolvedCrs {
            id: Some("EPSG:4326".to_owned()),
            kind: CrsKind::Geographic,
            definition: None,
        });
    }
    let row: rusqlite::Result<(String, i64, String)> = conn.query_row(
        "SELECT organization, organization_coordsys_id, definition FROM gpkg_spatial_ref_sys WHERE srs_id = ?1",
        [srs_id],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    );
    Ok(match row {
        Ok((org, code, def)) => ResolvedCrs {
            id: Some(format!("{}:{}", org.to_uppercase(), code)),
            kind: CrsKind::Unknown,
            definition: if def == "undefined" || def.is_empty() {
                None
            } else {
                Some(def)
            },
        },
        Err(_) => ResolvedCrs {
            id: Some(format!("srs_id:{srs_id}")),
            kind: CrsKind::Unknown,
            definition: None,
        },
    })
}

fn sqlite_declared_to_arrow(t: &str) -> DataType {
    let t = t.to_ascii_uppercase();
    if t.contains("INT") {
        DataType::Int64
    } else if t.contains("REAL") || t.contains("FLOA") || t.contains("DOUB") {
        DataType::Float64
    } else if t.contains("BLOB") {
        DataType::Binary
    } else {
        DataType::Utf8
    }
}

/// Schema (senza leggere dati) dai tipi dichiarati; attributi = colonne non
/// geometria e non chiave primaria.
fn build_schema(
    conn: &Connection,
    table: &str,
    geom_col: &str,
    crs: &ResolvedCrs,
) -> Result<(SchemaRef, Vec<(String, DataType)>)> {
    let mut stmt = conn
        .prepare(&format!("PRAGMA table_info(\"{}\")", table.replace('"', "\"\"")))
        .map_err(sql_err)?;
    let rows = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(1)?, // name
                r.get::<_, String>(2)?, // declared type
                r.get::<_, i64>(5)?,     // pk
            ))
        })
        .map_err(sql_err)?;
    let mut attrs: Vec<(String, DataType)> = Vec::new();
    for r in rows {
        let (name, decl, pk) = r.map_err(sql_err)?;
        if name == geom_col || pk > 0 {
            continue;
        }
        attrs.push((name, sqlite_declared_to_arrow(&decl)));
    }
    let mut fields = vec![geometry_field(geom_col, crs.id.as_deref().unwrap_or("OGC:CRS84"))];
    for (n, dt) in &attrs {
        fields.push(Field::new(n, dt.clone(), true));
    }
    Ok((Arc::new(Schema::new(fields)), attrs))
}

fn strip_gpkg_header(blob: &[u8]) -> Result<&[u8]> {
    if blob.len() < 8 || &blob[0..2] != b"GP" {
        return Err(err("blob geometria GeoPackage non valido (magic)"));
    }
    let envelope = match (blob[3] >> 1) & 0x07 {
        0 => 0,
        1 => 32,
        2 | 3 => 48,
        4 => 64,
        _ => return Err(err("envelope GeoPackage non valido")),
    };
    let start = 8 + envelope;
    if blob.len() < start {
        return Err(err("blob geometria GeoPackage troncato"));
    }
    Ok(&blob[start..])
}

fn gpkg_header(srs_id: i32) -> [u8; 8] {
    let s = srs_id.to_le_bytes();
    [b'G', b'P', 0, 0x01, s[0], s[1], s[2], s[3]]
}

fn geometry_index(schema: &Schema) -> Option<usize> {
    schema.fields().iter().position(|f| is_geometry_field(f))
}

fn init_gpkg(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "PRAGMA application_id = 1196444487;
         PRAGMA user_version = 10300;
         CREATE TABLE gpkg_spatial_ref_sys (srs_name TEXT NOT NULL, srs_id INTEGER PRIMARY KEY,
            organization TEXT NOT NULL, organization_coordsys_id INTEGER NOT NULL,
            definition TEXT NOT NULL, description TEXT);
         CREATE TABLE gpkg_contents (table_name TEXT PRIMARY KEY, data_type TEXT NOT NULL,
            identifier TEXT, description TEXT DEFAULT '', last_change TEXT,
            min_x DOUBLE, min_y DOUBLE, max_x DOUBLE, max_y DOUBLE, srs_id INTEGER);
         CREATE TABLE gpkg_geometry_columns (table_name TEXT NOT NULL, column_name TEXT NOT NULL,
            geometry_type_name TEXT NOT NULL, srs_id INTEGER NOT NULL, z TINYINT NOT NULL,
            m TINYINT NOT NULL, PRIMARY KEY (table_name, column_name));
         INSERT INTO gpkg_spatial_ref_sys VALUES
            ('WGS 84 geodetic', 4326, 'EPSG', 4326, 'GEOGCS[\"WGS 84\"]', 'longitude/latitude'),
            ('undefined cartesian', -1, 'NONE', -1, 'undefined', 'undefined'),
            ('undefined geographic', 0, 'NONE', 0, 'undefined', 'undefined');",
    )
    .map_err(sql_err)?;
    Ok(())
}

fn sqlite_type(dt: &DataType) -> &'static str {
    match dt {
        DataType::Int8 | DataType::Int16 | DataType::Int32 | DataType::Int64 | DataType::Boolean => {
            "INTEGER"
        }
        DataType::Float32 | DataType::Float64 => "REAL",
        _ => "TEXT",
    }
}

/// CRS del layer in scrittura: dal contratto (id + WKT) se presente, altrimenti
/// dall'id nei metadati del campo geometria.
fn layer_crs(
    layer: &plenora_io_core::WriteLayer,
    geom_idx: usize,
) -> (Option<String>, Option<String>) {
    if let Some(g) = &layer.contract.geometry {
        return (g.crs.id.clone(), g.crs.definition.clone());
    }
    let id = layer
        .contract
        .schema
        .field(geom_idx)
        .metadata()
        .get(plenora_core::geometry::GEO_CRS_KEY)
        .cloned();
    (id, None)
}

/// Risolve il `srs_id` GeoPackage per il CRS dato, registrandolo in
/// `gpkg_spatial_ref_sys` se non è il WGS84 built-in. Senza WKT reale usa
/// `definition='undefined'`: GDAL risolve comunque il CRS da organization+code.
fn register_srs(conn: &Connection, id: Option<&str>, def: Option<&str>) -> Result<i32> {
    let id = match id {
        Some(s) => s,
        None => return Ok(4326),
    };
    if id.eq_ignore_ascii_case("EPSG:4326") || id.eq_ignore_ascii_case("OGC:CRS84") {
        return Ok(4326);
    }
    if let Some((auth, code)) = id.split_once(':') {
        if let Ok(code_i) = code.parse::<i32>() {
            conn.execute(
                "INSERT OR IGNORE INTO gpkg_spatial_ref_sys \
                 (srs_name, srs_id, organization, organization_coordsys_id, definition, description) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                rusqlite::params![
                    id,
                    code_i,
                    auth.to_uppercase(),
                    code_i,
                    def.unwrap_or("undefined"),
                    "importato da plenora-io"
                ],
            )
            .map_err(sql_err)?;
            return Ok(code_i);
        }
    }
    Ok(4326) // id non parseable: fallback WGS84
}

fn create_feature_table(
    conn: &Connection,
    name: &str,
    schema: &Schema,
    srs_id: i32,
) -> Result<ActiveLayer> {
    let geom_idx = geometry_index(schema).ok_or_else(|| err("layer senza geometria"))?;
    let geom_name = schema.field(geom_idx).name().clone();

    let mut cols_ddl = vec!["fid INTEGER PRIMARY KEY AUTOINCREMENT".to_owned()];
    cols_ddl.push(format!("\"{}\" BLOB", geom_name.replace('"', "\"\"")));
    for (i, f) in schema.fields().iter().enumerate() {
        if i == geom_idx {
            continue;
        }
        cols_ddl.push(format!(
            "\"{}\" {}",
            f.name().replace('"', "\"\""),
            sqlite_type(f.data_type())
        ));
    }
    conn.execute(
        &format!(
            "CREATE TABLE \"{}\" ({})",
            name.replace('"', "\"\""),
            cols_ddl.join(", ")
        ),
        [],
    )
    .map_err(sql_err)?;
    conn.execute(
        "INSERT INTO gpkg_contents (table_name, data_type, identifier, srs_id) VALUES (?1,'features',?1,?2)",
        rusqlite::params![name, srs_id],
    )
    .map_err(sql_err)?;
    conn.execute(
        "INSERT INTO gpkg_geometry_columns VALUES (?1, ?2, 'GEOMETRY', ?3, 0, 0)",
        rusqlite::params![name, geom_name, srs_id],
    )
    .map_err(sql_err)?;

    let mut col_list = vec![format!("\"{}\"", geom_name.replace('"', "\"\""))];
    for (i, f) in schema.fields().iter().enumerate() {
        if i != geom_idx {
            col_list.push(format!("\"{}\"", f.name().replace('"', "\"\"")));
        }
    }
    let placeholders: Vec<String> = (1..=col_list.len()).map(|i| format!("?{i}")).collect();
    let insert_sql = format!(
        "INSERT INTO \"{}\" ({}) VALUES ({})",
        name.replace('"', "\"\""),
        col_list.join(", "),
        placeholders.join(", ")
    );
    Ok(ActiveLayer {
        geom_idx,
        insert_sql,
        srs_id,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use plenora_core::wkb::to_wkb;
    use plenora_io_core::request::{BatchTarget, ProjectionMode};
    use plenora_io_core::WriteLayer;

    #[test]
    fn round_trip_gpkg() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("out.gpkg");
        let wkb = to_wkb(&geo_types::Geometry::Point(geo_types::Point::new(12.5, 45.9))).unwrap();
        let geom = BinaryArray::from(vec![Some(wkb.as_slice()), Some(wkb.as_slice())]);
        let ids = Int64Array::from(vec![1i64, 2]);
        let schema: SchemaRef = Arc::new(Schema::new(vec![
            geometry_field("geom", "EPSG:4326"),
            Field::new("id", DataType::Int64, false),
        ]));
        let batch =
            RecordBatch::try_new(schema.clone(), vec![Arc::new(geom), Arc::new(ids)]).unwrap();

        let driver = GpkgDriver;
        let plan = WritePlan {
            layers: vec![WriteLayer {
                name: "vani".to_owned(),
                contract: DataContract {
                    schema: schema.clone(),
                    geometry: None,
                },
            }],
        };
        let mut w = driver
            .create(Sink::Path(path.clone()), &plan, &WriteOptions::default())
            .unwrap();
        w.write(&batch).unwrap();
        w.finish().unwrap();

        let ds = driver
            .open(Source::Path(path), &ReadOptions::default())
            .unwrap();
        assert_eq!(ds.layers().len(), 1);
        assert_eq!(ds.layers()[0].name, "vani");
        let mut reader = ds
            .open_layer_reader(&ReadRequest {
                layer: LayerId(0),
                projected_fields: None,
                projection_mode: ProjectionMode::BestEffort,
                pruning_predicate: None,
                spatial_pruning_hint: None,
                batch_target: BatchTarget::default(),
            })
            .unwrap();
        let out = reader.next_batch().unwrap().unwrap();
        assert_eq!(out.num_rows(), 2);
        let gcol = out
            .column_by_name("geom")
            .unwrap()
            .as_any()
            .downcast_ref::<BinaryArray>()
            .unwrap();
        assert_eq!(gcol.value(0), wkb.as_slice());
        // id preservato come Int64
        let idcol = out
            .column_by_name("id")
            .unwrap()
            .as_any()
            .downcast_ref::<Int64Array>()
            .unwrap();
        assert_eq!(idcol.value(1), 2);
        assert!(reader.next_batch().unwrap().is_none());
    }

    #[test]
    fn write_two_layers_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("multi.gpkg");
        let wkb = to_wkb(&geo_types::Geometry::Point(geo_types::Point::new(1.0, 2.0))).unwrap();

        let s0: SchemaRef = Arc::new(Schema::new(vec![
            geometry_field("geom", "EPSG:4326"),
            Field::new("id", DataType::Int64, false),
        ]));
        let b0 = RecordBatch::try_new(
            s0.clone(),
            vec![
                Arc::new(BinaryArray::from(vec![Some(wkb.as_slice())])),
                Arc::new(Int64Array::from(vec![10i64])),
            ],
        )
        .unwrap();

        let s1: SchemaRef = Arc::new(Schema::new(vec![
            geometry_field("geom", "EPSG:4326"),
            Field::new("nome", DataType::Utf8, true),
        ]));
        let b1 = RecordBatch::try_new(
            s1.clone(),
            vec![
                Arc::new(BinaryArray::from(vec![Some(wkb.as_slice()), Some(wkb.as_slice())])),
                Arc::new(StringArray::from(vec!["A", "B"])),
            ],
        )
        .unwrap();

        let driver = GpkgDriver;
        let plan = WritePlan {
            layers: vec![
                WriteLayer {
                    name: "vani".to_owned(),
                    contract: DataContract {
                        schema: s0.clone(),
                        geometry: None,
                    },
                },
                WriteLayer {
                    name: "strade".to_owned(),
                    contract: DataContract {
                        schema: s1.clone(),
                        geometry: None,
                    },
                },
            ],
        };
        let mut w = driver
            .create(Sink::Path(path.clone()), &plan, &WriteOptions::default())
            .unwrap();
        w.write_to_layer(LayerId(0), &b0).unwrap();
        w.write_to_layer(LayerId(1), &b1).unwrap();
        w.finish().unwrap();

        let ds = driver.open(Source::Path(path), &ReadOptions::default()).unwrap();
        assert_eq!(ds.layers().len(), 2);
        let names: Vec<&str> = ds.layers().iter().map(|l| l.name.as_str()).collect();
        assert!(names.contains(&"vani") && names.contains(&"strade"), "layer: {names:?}");

        // Ogni layer ha il suo conteggio righe (instradamento corretto).
        for l in ds.layers() {
            let expected = if l.name == "vani" { 1 } else { 2 };
            let mut r = ds
                .open_layer_reader(&ReadRequest {
                    layer: l.id,
                    projected_fields: None,
                    projection_mode: ProjectionMode::BestEffort,
                    pruning_predicate: None,
                    spatial_pruning_hint: None,
                    batch_target: BatchTarget::default(),
                })
                .unwrap();
            let rb = r.next_batch().unwrap().unwrap();
            assert_eq!(rb.num_rows(), expected, "layer '{}'", l.name);
        }
    }

    #[test]
    fn round_trip_non_wgs84_crs() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("m3857.gpkg");
        // Un punto in EPSG:3857 (Web Mercator).
        let wkb =
            to_wkb(&geo_types::Geometry::Point(geo_types::Point::new(1113194.0, 5621521.0))).unwrap();
        let schema: SchemaRef = Arc::new(Schema::new(vec![
            geometry_field("geom", "EPSG:3857"),
            Field::new("id", DataType::Int64, false),
        ]));
        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![
                Arc::new(BinaryArray::from(vec![Some(wkb.as_slice())])),
                Arc::new(Int64Array::from(vec![1i64])),
            ],
        )
        .unwrap();
        let driver = GpkgDriver;
        let plan = WritePlan {
            layers: vec![WriteLayer {
                name: "l".to_owned(),
                contract: DataContract {
                    schema: schema.clone(),
                    geometry: None,
                },
            }],
        };
        let mut w = driver
            .create(Sink::Path(path.clone()), &plan, &WriteOptions::default())
            .unwrap();
        w.write(&batch).unwrap();
        w.finish().unwrap();

        // Rilettura: il CRS NON è più 4326 fisso, è EPSG:3857.
        let ds = driver.open(Source::Path(path), &ReadOptions::default()).unwrap();
        let crs = ds.layers()[0].contract.geometry.as_ref().unwrap().crs.id.clone();
        assert_eq!(crs.as_deref(), Some("EPSG:3857"));
    }
}
