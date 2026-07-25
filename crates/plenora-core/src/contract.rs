//! `DataContract` e affini (scheletro Fase 0). La v1 ammette al massimo una
//! colonna geometria (Architetture §2.2, D16 data-tools).

use arrow_schema::SchemaRef;

use crate::crs::ResolvedCrs;

/// Identità logica stabile di un campo nel grafo (namespace globale).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FieldId(pub u32);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct LayerId(pub u32);

/// Contratto di una colonna geometrica.
#[derive(Clone, Debug)]
pub struct GeometryColumnContract {
    pub field_id: FieldId,
    pub name: String,
    pub crs: ResolvedCrs,
    pub nullable: bool,
}

/// Contratto dei dati che attraversano un arco / un layer.
#[derive(Clone, Debug)]
pub struct DataContract {
    pub schema: SchemaRef,
    /// v1: al massimo una colonna geometria.
    pub geometry: Option<GeometryColumnContract>,
}

/// Contratto di un layer di un dataset aperto.
#[derive(Clone, Debug)]
pub struct LayerContract {
    pub id: LayerId,
    pub name: String,
    pub contract: DataContract,
}
