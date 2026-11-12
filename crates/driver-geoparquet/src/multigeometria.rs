//! Contratti e metadati delle colonne geometriche secondarie.

use super::{
    apply_geo_column_metadata, build_geo_metadata, crs_da_scrivere, crs_della_colonna,
    epoca_da_scrivere, fmt_err, retag_schema, BTreeSet, CoordinateDimensions, CrsDaScrivere,
    DataContract, FieldId, GeometryColumnContract, GeometryType, MetadatiGeo, PublicMessage,
    Result, SchemaRef,
};
use std::collections::BTreeMap;

/// Confronto prima del retag, su ogni colonna dichiarata nel footer.
pub fn verifica_crs(schema: &arrow_schema::Schema, geo: &MetadatiGeo) -> Result<()> {
    for (nome, colonna) in
        std::iter::once((&geo.nome_primaria, &geo.primaria)).chain(geo.secondarie.iter())
    {
        let field = schema.field_with_name(nome).map_err(|_| {
            fmt_err(&PublicMessage::Curated(
                "colonna geometrica assente dallo schema",
            ))
        })?;
        let mut footer = GeometryColumnContract::wkb_passthrough(
            FieldId(0),
            nome,
            crs_della_colonna(colonna)?,
            field.is_nullable(),
        );
        apply_geo_column_metadata(&mut footer, Some(geo))?;
        let mut shared = field.metadata().clone();
        if colonna.tipi.is_empty() {
            // No footer type means no footer dimensionality declaration.
            // Mixed XY/XYZ is different: it must conflict with a field restricted to XY.
            shared.remove(plenora_io_model::geometry::PLENORA_DIMENSIONS_KEY);
        }
        let field = field.clone().with_metadata(shared);
        plenora_io_model::primary::reconcile_geometry_properties(&field, &footer)?;
    }
    Ok(())
}

pub fn completa_schema(mut schema: SchemaRef, geo: Option<&MetadatiGeo>) -> Result<SchemaRef> {
    let Some(geo) = geo else { return Ok(schema) };
    for (nome, colonna) in &geo.secondarie {
        let crs = crs_della_colonna(colonna)?;
        schema = retag_schema(&schema, nome, &crs, None);
        let indice = schema.index_of(nome).map_err(|_| {
            fmt_err(&PublicMessage::Curated(
                "colonna geometrica secondaria assente dallo schema",
            ))
        })?;
        let id = u32::try_from(indice).map_err(|_| {
            fmt_err(&PublicMessage::Curated(
                "indice geometrico non rappresentabile",
            ))
        })?;
        let mut contratto = GeometryColumnContract::wkb_passthrough(
            FieldId(id),
            nome,
            crs,
            schema.field(indice).is_nullable(),
        );
        // CRS e tipi vengono dal geo del footer, identita' dal campo Arrow.
        let mut dichiarato = contratto.clone();
        plenora_io_model::geometry::read_geometry_contract_metadata(
            schema.field(indice),
            &mut dichiarato,
        )?;
        let footer_crs = contratto.crs.clone();
        contratto = dichiarato;
        contratto.crs = footer_crs;
        if contratto.types_declaration
            == plenora_io_model::contract::TypesDeclaration::LegacyUndeclared
        {
            contratto.types_declaration = plenora_io_model::contract::TypesDeclaration::Unresolved;
        }
        apply_geo_column_metadata(&mut contratto, Some(geo))?;
        // Completa il campo senza eleggerlo temporaneamente a primaria.
        // Il footer sceglie la primaria una sola volta nel reader chiamante.
        let fields = schema
            .fields()
            .iter()
            .enumerate()
            .map(|(i, field)| {
                if i == indice {
                    plenora_io_model::geometry::with_geometry_contract_metadata(field, &contratto)
                } else {
                    field.as_ref().clone()
                }
            })
            .collect::<Vec<_>>();
        schema = plenora_io_model::geometry::with_contract_version(std::sync::Arc::new(
            arrow_schema::Schema::new_with_metadata(fields, schema.metadata().clone()),
        ));
    }
    Ok(schema)
}

pub struct ColonnaScritta {
    pub indice: usize,
    nome: String,
    pub crs: CrsDaScrivere,
    epoca: Option<String>,
    pub tipi: BTreeSet<(GeometryType, CoordinateDimensions)>,
}

pub fn colonne_da_scrivere(
    contratto: &DataContract,
    primaria: &str,
) -> Result<Vec<ColonnaScritta>> {
    contratto
        .geometries()?
        .into_iter()
        .filter(|g| g.name != primaria)
        .map(|g| {
            let indice = contratto.schema.index_of(&g.name).map_err(|_| {
                fmt_err(&PublicMessage::Curated(
                    "colonna geometrica secondaria assente dallo schema",
                ))
            })?;
            Ok(ColonnaScritta {
                indice,
                crs: crs_da_scrivere(Some(&g), None)?,
                epoca: epoca_da_scrivere(Some(&g))?,
                nome: g.name,
                tipi: BTreeSet::new(),
            })
        })
        .collect()
}

/// I valori grezzi evitano di arrotondare i numeri di PROJJSON ed epoch.
pub fn aggiungi_colonne(geo: String, secondarie: &[ColonnaScritta]) -> Result<String> {
    use serde_json::value::RawValue;
    type Oggetto = BTreeMap<String, Box<RawValue>>;
    let errore = || {
        fmt_err(&PublicMessage::Curated(
            "metadato multigeometria non serializzabile",
        ))
    };
    if secondarie.is_empty() {
        return Ok(geo);
    }
    let mut radice: Oggetto = serde_json::from_str(&geo).map_err(|_| errore())?;
    let mut colonne: Oggetto =
        serde_json::from_str(radice.get("columns").ok_or_else(errore)?.get())
            .map_err(|_| errore())?;
    for colonna in secondarie {
        let testo = build_geo_metadata(
            &colonna.nome,
            &colonna.tipi,
            &colonna.crs,
            colonna.epoca.as_deref(),
        )?;
        let documento: Oggetto = serde_json::from_str(&testo).map_err(|_| errore())?;
        let mut altre: Oggetto =
            serde_json::from_str(documento.get("columns").ok_or_else(errore)?.get())
                .map_err(|_| errore())?;
        let valore = altre.remove(&colonna.nome).ok_or_else(errore)?;
        let mut campi: Oggetto = serde_json::from_str(valore.get()).map_err(|_| errore())?;
        // Il covering prodotto misura soltanto la primaria.
        campi.remove("covering");
        colonne.insert(
            colonna.nome.clone(),
            serde_json::value::to_raw_value(&campi).map_err(|_| errore())?,
        );
    }
    radice.insert(
        "columns".to_owned(),
        serde_json::value::to_raw_value(&colonne).map_err(|_| errore())?,
    );
    serde_json::to_string(&radice).map_err(|_| errore())
}
