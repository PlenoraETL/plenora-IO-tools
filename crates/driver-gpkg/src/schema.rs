//! Metadati SQL conservabili e normalizzazioni dello schema.
use super::{
    err, geometry_field, sql_err, sqlite_type, Arc, Connection, DataType, Field, LossReport,
    PlenoraIoError, PublicMessage, ResolvedCrs, Result, Schema, SchemaRef, DECLARED_TYPE,
    PRIMARY_KEY, SCHEMA_NORMALIZED,
};

/// Vocabolario chiuso: nessun frammento SQL proveniente dal file entra nel DDL.
pub fn recognized_declared_type(declared: &str) -> Option<&'static str> {
    [
        "BOOLEAN",
        "TINYINT",
        "SMALLINT",
        "MEDIUMINT",
        "INT",
        "INTEGER",
        "FLOAT",
        "DOUBLE",
        "REAL",
        "TEXT",
        "BLOB",
        "DATE",
        "DATETIME",
    ]
    .into_iter()
    .find(|candidate| declared.eq_ignore_ascii_case(candidate))
}

pub fn validate_sqlite_metadata(schema: &Schema) -> Result<()> {
    let mut keys = 0;
    for field in schema.fields() {
        if let Some(declared) = field.metadata().get(DECLARED_TYPE) {
            if recognized_declared_type(declared).is_none()
                || sqlite_declared_to_arrow(declared) != *field.data_type()
            {
                return Err(err(&PublicMessage::Curated(
                    "metadato del tipo SQLite non riconosciuto o incompatibile con il tipo Arrow",
                )));
            }
        }
        if let Some(key) = field.metadata().get(PRIMARY_KEY) {
            keys += 1;
            if key != "true"
                || field.data_type() != &DataType::Int64
                || field.is_nullable()
                || field
                    .metadata()
                    .get(DECLARED_TYPE)
                    .is_none_or(|t| t != "INTEGER")
            {
                return Err(err(&PublicMessage::Curated(
                    "metadato della chiave primaria SQLite incompatibile con il contratto",
                )));
            }
        }
    }
    if keys > 1 {
        return Err(err(&PublicMessage::Curated(
            "GeoPackage: chiave primaria composta non rappresentabile",
        )));
    }
    Ok(())
}

pub fn schema_read_loss(schema: &Schema, layer: u32) -> LossReport {
    let mut loss = LossReport::default();
    if let Some(reasons) = schema.metadata().get("gpkg.schema_normalized") {
        loss.record(SCHEMA_NORMALIZED, 1);
        loss.add_example(plenora_io_core::loss::LossExample {
            category: SCHEMA_NORMALIZED.to_owned(),
            posizione: plenora_io_core::loss::Posizione {
                layer_index: Some(u64::from(layer)),
                field_index: None,
                type_class: None,
            },
            context: [
                ("type", "tipo dichiarato non riconosciuto"),
                (
                    "constraints",
                    "default, vincoli SQL, indici o trigger non ricostruiti",
                ),
                (
                    "identity",
                    "identita' della riga senza chiave primaria nel contratto",
                ),
                (
                    "order",
                    "ordine delle colonne normalizzato: geometria prima, chiave primaria ultima",
                ),
            ]
            .iter()
            .filter(|(key, _)| reasons.split(',').any(|reason| reason == *key))
            .map(|(_, reason)| *reason)
            .collect::<Vec<_>>()
            .join("; "),
        });
    }
    loss
}

pub fn sqlite_declared_to_arrow(t: &str) -> DataType {
    let t = t.to_ascii_uppercase();
    if t == "BOOLEAN" || t.contains("INT") {
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
/// geometria, inclusa la chiave primaria.
pub fn build_schema(
    conn: &Connection,
    table: &str,
    geom_col: &str,
    crs: &ResolvedCrs,
) -> Result<(SchemaRef, Vec<(String, DataType)>)> {
    let mut stmt = conn
        .prepare(&format!(
            "PRAGMA table_xinfo(\"{}\")",
            table.replace('"', "\"\"")
        ))
        .map_err(sql_err)?;
    let rows = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(1)?,         // name
                r.get::<_, String>(2)?,         // declared type
                r.get::<_, i64>(5)?,            // pk
                r.get::<_, i64>(3)?,            // not null
                r.get::<_, Option<String>>(4)?, // default
                r.get::<_, i64>(6)?,            // hidden/generated
            ))
        })
        .map_err(sql_err)?;
    let mut attrs: Vec<(String, DataType)> = Vec::new();
    let mut attribute_fields = Vec::new();
    let mut primary = None;
    let mut normalized = std::collections::BTreeSet::new();
    let mut original_order = Vec::new();
    let mut geometry_nullable = true;
    for r in rows {
        let (name, decl, pk, not_null, default, hidden) = r.map_err(sql_err)?;
        original_order.push(name.clone());
        if default.is_some() || hidden != 0 {
            normalized.insert("constraints");
        }
        // La lettura pagina con `WHERE t.rowid > ?`, dove `rowid` e' l'alias
        // implicito della chiave di riga. Se la tabella dichiara una colonna
        // con quel nome, SQLite risolve l'alias sulla colonna *utente* e la
        // paginazione avviene su valori arbitrari: righe saltate, ripetute, o
        // un cursore che non avanza mai.
        //
        // SQLite ha tre alias per la chiave di riga e sono tutti oscurabili;
        // non esiste una forma non oscurabile a cui ripiegare. L'unica difesa
        // e' rifiutare la tabella, ed e' coerente con la semantica fail-closed
        // del componente: meglio non leggere che leggere righe sbagliate senza
        // dirlo.
        //
        // Il confronto e' senza distinzione di maiuscole perche' SQLite
        // risolve i nomi di colonna in modo case-insensitive.
        if matches!(
            name.to_ascii_lowercase().as_str(),
            "rowid" | "_rowid_" | "oid"
        ) {
            // I nomi di tabella e colonna vengono dal file: sono la forma
            // piu' diretta di testo di payload, e non escono.
            return Err(err(&PublicMessage::Curated(
                "una colonna dichiarata dalla tabella oscura l'alias della chiave di riga usato per la paginazione; il GeoPackage non e' leggibile in modo deterministico",
            )));
        }
        // Il nome di colonna dichiarato in `gpkg_geometry_columns` si
        // confronta come lo risolve `SQLite`, senza distinzione di maiuscole
        // ASCII: un confronto esatto esporrebbe la colonna geometrica una
        // seconda volta, come attributo binario.
        if name.eq_ignore_ascii_case(geom_col) {
            geometry_nullable = not_null == 0;
            continue;
        }
        let dt = sqlite_declared_to_arrow(&decl);
        let mut metadata = std::collections::HashMap::new();
        if let Some(declared) = recognized_declared_type(&decl) {
            metadata.insert(DECLARED_TYPE.to_owned(), declared.to_owned());
        } else {
            normalized.insert("type");
        }
        if pk > 0 {
            if pk != 1 || primary.is_some() || !decl.eq_ignore_ascii_case("INTEGER") {
                return Err(err(&PublicMessage::Curated(
                    "GeoPackage: chiave primaria diversa da un singolo INTEGER non rappresentabile",
                )));
            }
            metadata.insert(PRIMARY_KEY.to_owned(), "true".to_owned());
        }
        let field = Field::new(&name, dt.clone(), not_null == 0 && pk == 0).with_metadata(metadata);
        if pk > 0 {
            primary = Some((name, dt, field));
        } else {
            attrs.push((name, dt));
            attribute_fields.push(field);
        }
    }
    let crs_id = crs.id.as_deref().ok_or_else(|| {
        PlenoraIoError::crs_redatto(&PublicMessage::Curated(
            "GeoPackage: CRS risolto senza identificatore; vietato assumere OGC:CRS84",
        ))
    })?;
    let mut fields = vec![geometry_field(geom_col, crs_id).with_nullable(geometry_nullable)];
    if let Some((name, dt, field)) = primary {
        attrs.push((name, dt));
        attribute_fields.push(field);
    } else {
        normalized.insert("identity");
    }
    fields.extend(attribute_fields);
    if has_unrepresented_sql(conn, table)? {
        normalized.insert("constraints");
    }
    if original_order
        .iter()
        .map(String::as_str)
        .ne(fields.iter().map(|f| f.name().as_str()))
    {
        normalized.insert("order");
    }
    let mut metadata = std::collections::HashMap::new();
    if !normalized.is_empty() {
        metadata.insert(
            "gpkg.schema_normalized".to_owned(),
            normalized.into_iter().collect::<Vec<_>>().join(","),
        );
    }
    let fields = super::contract_plan::restore(conn, table, fields)?;
    Ok((Arc::new(Schema::new_with_metadata(fields, metadata)), attrs))
}

fn has_unrepresented_sql(conn: &Connection, table: &str) -> Result<bool> {
    let ddl: String = conn
        .query_row(
            "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = ?1 COLLATE NOCASE",
            [table],
            |r| r.get(0),
        )
        .map_err(sql_err)?;
    // SQL non ricostruito: segnalarlo anche se la parola compare in un
    // identificatore quotato e' conservativo, mai una promessa di conservazione.
    let constraints = ddl
        .split(|c: char| !c.is_ascii_alphanumeric() && c != '_')
        .any(|word| {
            [
                "CHECK",
                "UNIQUE",
                "REFERENCES",
                "COLLATE",
                "GENERATED",
                "DEFAULT",
                "AUTOINCREMENT",
                "CONFLICT",
                "STRICT",
                "WITHOUT",
                "DESC",
            ]
            .iter()
            .any(|keyword| word.eq_ignore_ascii_case(keyword))
        });
    let indices = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE tbl_name = ?1 COLLATE NOCASE AND type IN ('index', 'trigger'))",
        [table], |r| r.get::<_, bool>(0),
    ).map_err(sql_err)?;
    Ok(constraints || indices)
}

pub fn columns_ddl(schema: &Schema, geom_idx: usize) -> Vec<String> {
    let geom_name = schema.field(geom_idx).name();
    let mut cols_ddl = Vec::new();
    if let Some(generated) = super::contract_plan::generated_primary_name(schema) {
        cols_ddl.push(format!("\"{generated}\" INTEGER PRIMARY KEY AUTOINCREMENT"));
    }
    cols_ddl.push(format!(
        "\"{}\" BLOB{}",
        geom_name.replace('"', "\"\""),
        if schema.field(geom_idx).is_nullable() {
            ""
        } else {
            " NOT NULL"
        }
    ));
    for (i, f) in schema.fields().iter().enumerate() {
        if i == geom_idx {
            continue;
        }
        cols_ddl.push(format!(
            "\"{}\" {}{}{}",
            f.name().replace('"', "\"\""),
            f.metadata()
                .get(DECLARED_TYPE)
                .map_or_else(|| sqlite_type(f.data_type()), String::as_str),
            if f.is_nullable() { "" } else { " NOT NULL" },
            if f.metadata().contains_key(PRIMARY_KEY) {
                " PRIMARY KEY"
            } else {
                ""
            },
        ));
    }
    cols_ddl
}
