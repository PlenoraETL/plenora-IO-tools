//! Che cosa una scrittura perdera', calcolato prima di scriverla.
//!
//! Estratto da `driver.rs` per M1. Il gruppo non chiama nessuno degli altri due
//! della validazione -- la misura dei riferimenti che attraversano i confini lo
//! dava a zero in entrambe le direzioni -- e si raggiunge da
//! `with_write_validation`, che resta nel genitore.
//!
//! Lo spostamento e' meccanico: nessuna firma cambia, nessun ordine di
//! operazioni cambia, e cio' che il gruppo usa del genitore arriva per `use
//! super::*`.

use plenora_io_model::crs::CrsDefinitionFormat;

use crate::descriptor::CrsRepresentationCapabilities;

use super::{
    is_geometry_field, saturating_u64, ArrowTypeClass, AttributeWriteSupport, CrsDerivation,
    CrsRepresentationState, CrsResolution, DataType, FidelityAssessment, FidelityReasonCode,
    FormatDescriptor, GeometryType, LossExample, LossReport, NullabilitySupport, Posizione,
    TypeCoercionPolicy, WritePlan,
};

pub fn planned_write_loss(
    descriptor: &FormatDescriptor,
    plan: &WritePlan,
) -> plenora_io_model::Result<LossReport> {
    let mut loss = LossReport::default();
    let Some(capabilities) = descriptor.write_capabilities() else {
        return Ok(loss);
    };

    for (indice_layer, layer) in plan.layers.iter().enumerate() {
        let layer_index = Some(saturating_u64(indice_layer));
        let geometries = layer.contract.geometries()?;
        for geometry in geometries
            .iter()
            .filter(|_| layer.contract.geometry.is_some() || geometries.len() > 1)
        {
            let (crs_id, crs_definition, grammatica) = match &geometry.crs {
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
            // Le capability dicono da dove ogni rappresentazione si ricava;
            // il piano dice se quella fonte c'e'. Il verdetto e' l'incrocio
            // dei due, e si calcola una volta sola per layer.
            let fonti = FontiDelPiano {
                crs_id,
                crs_definition,
            };
            let dove = Posizione {
                layer_index,
                field_index: indice_della_geometria(layer, &geometry.name),
                type_class: None,
            };
            record_crs_representation_loss(
                &mut loss,
                dove,
                RappresentazioneDelCrs::CrsId,
                crs_id.map(str::len),
                stato_dell_identificatore(&capabilities.crs_representations, &fonti, grammatica),
            );
            record_crs_representation_loss(
                &mut loss,
                dove,
                RappresentazioneDelCrs::Srid,
                geometry.srid.map(|srid| srid.to_string().len()),
                stato_per_il_piano(capabilities.crs_representations.srid, &fonti),
            );
            record_crs_representation_loss(
                &mut loss,
                dove,
                RappresentazioneDelCrs::CrsDefinition,
                crs_definition.map(str::len),
                // La grammatica decide se `Preserved` regge: un writer che
                // conserva solo il PROJJSON non conserva un WKT.
                stato_per_il_piano(
                    capabilities
                        .crs_representations
                        .crs_definition_per(grammatica),
                    &fonti,
                ),
            );
            record_coordinate_epoch_loss(&mut loss, descriptor.id(), geometry, dove);
            record_geometry_metadata_loss(&mut loss, descriptor.id(), geometry, dove);
        }

        let geometry_name = layer
            .contract
            .geometry
            .as_ref()
            .map(|geometry| geometry.name.as_str());
        for (indice_campo, field) in layer.contract.schema.fields().iter().enumerate() {
            if geometry_name == Some(field.name().as_str()) || is_geometry_field(field) {
                continue;
            }
            let type_class = crate::capabilities::arrow_type_class(field.data_type());
            let unsupported_text_coercion = !capabilities.allowed_types.contains(&type_class)
                && matches!(
                    capabilities.type_coercion,
                    TypeCoercionPolicy::ExplicitText | TypeCoercionPolicy::LossReported
                );
            let kml_scalar_to_text = descriptor.id() == "kml" && type_class != ArrowTypeClass::Utf8;
            let gpkg_type_normalization = descriptor.id() == "gpkg"
                && !matches!(
                    field.data_type(),
                    DataType::Int64 | DataType::Float64 | DataType::Utf8 | DataType::Binary
                );
            if unsupported_text_coercion || kml_scalar_to_text || gpkg_type_normalization {
                loss.record("coercion tipo attributo", 1);
                loss.add_example(LossExample {
                    category: "coercion tipo attributo".to_owned(),
                    posizione: Posizione {
                        layer_index,
                        field_index: Some(saturating_u64(indice_campo)),
                        type_class: Some(type_class),
                    },
                    context: "il tipo dell'attributo richiede una coercizione".to_owned(),
                });
            }
        }
    }
    Ok(loss)
}

/// Le fonti che il **piano** mette a disposizione della derivazione.
///
/// Non e' il CRS: e' la risposta alle due domande che una provenienza pone --
/// «c'e' una definizione da emettere?» e «c'e' un identificatore da cui
/// ricavare?». L'SRID non entra perche' nessun driver deriva da lui.
pub struct FontiDelPiano<'a> {
    pub crs_id: Option<&'a str>,
    pub crs_definition: Option<&'a str>,
}

/// Il writer produce comunque una definizione per questo identificatore.
fn definizione_sintetizzabile(id: Option<&str>, ammessi: &[&str]) -> bool {
    id.is_some_and(|id| ammessi.contains(&id))
}

/// Lo stato di una rappresentazione **per questo piano**.
///
/// `Derived` e' una promessa condizionata, non un fatto: dice che la
/// rappresentazione si ricava da un'altra cosa, e regge solo se quella cosa
/// c'e'. Quando non c'e', la rappresentazione non e' ricavabile e lo stato
/// onesto e' `Absent` -- che produce una categoria di perdita **diversa**, e
/// piu' severa, di quella derivata: chi legge `..._derived` va a cercare nel
/// file un valore che nessuno ha scritto.
///
/// Il raffinamento e' per provenienza, mai per driver: `FixedByFormat` e
/// `RuntimeResolved` non dipendono dal piano e non decadono mai, ed e' per
/// questo che `geojson`, `kml` e `filegdb` non si muovono.
pub fn stato_per_il_piano(
    stato: CrsRepresentationState,
    fonti: &FontiDelPiano,
) -> CrsRepresentationState {
    let CrsRepresentationState::Derived(derivazione) = stato else {
        return stato;
    };
    let disponibile = match derivazione {
        CrsDerivation::FromDefinition {
            synthesized_for, ..
        } => {
            fonti.crs_definition.is_some()
                || definizione_sintetizzabile(fonti.crs_id, synthesized_for)
        }
        CrsDerivation::FromIdentifier => fonti.crs_id.is_some(),
        CrsDerivation::SynthesizedFor { ids } => definizione_sintetizzabile(fonti.crs_id, ids),
        CrsDerivation::FixedByFormat | CrsDerivation::RuntimeResolved => true,
    };
    if disponibile {
        stato
    } else {
        CrsRepresentationState::Absent
    }
}

/// Lo stato dell'identificatore **per questo piano**.
///
/// Due raffinamenti sopra [`stato_per_il_piano`]. Il primo e' della capability
/// ([`CrsRepresentationCapabilities::crs_id_per`]): senza la definizione che lo
/// porta, l'identificatore di `GeoParquet` si rilegge come il default del
/// formato. Il secondo e' della derivazione dalla definizione: `derived` vale
/// solo se il WKT che il writer emette si rilegge, con la regola dei lettori,
/// nello **stesso** identificatore (GEO-005). Un `.prj` sintetizzato per
/// `OGC:CRS84` si rilegge `EPSG:4326`, un `PROJCS` anonimo si rilegge non
/// risolto: in entrambi l'identificatore va riportato da fuori, e lo stato
/// onesto e' `absent`. Fino alla rilettura 11 erano `derived` (era la deviazione
/// dichiarata «Shapefile, DXF: identificatore dal WKT»).
pub fn stato_dell_identificatore(
    rappresentazioni: &CrsRepresentationCapabilities,
    fonti: &FontiDelPiano,
    grammatica: Option<CrsDefinitionFormat>,
) -> CrsRepresentationState {
    let stato = rappresentazioni.crs_id_per(fonti.crs_id, fonti.crs_definition, grammatica);
    match rappresentazioni.identificatore_riletto(fonti.crs_id, fonti.crs_definition, grammatica) {
        None => stato_per_il_piano(stato, fonti),
        Some(Ok(Some(riletto)))
            if fonti
                .crs_id
                .is_some_and(|id| plenora_io_model::crs::stesso_identificatore(&riletto, id)) =>
        {
            stato
        }
        Some(_) => CrsRepresentationState::Absent,
    }
}

/// Quale delle tre rappresentazioni del CRS non e' stata preservata.
///
/// Un tipo e non una stringa: la categoria di perdita che ne esce e' una
/// **chiave sul filo**, e una chiave costruita con `format!` e' una chiave che
/// nessuno puo' enumerare leggendo il codice. Le sei combinazioni sono qui,
/// scritte per esteso, ed e' cio' che permette al registro di dichiararle e al
/// gate di verificarle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RappresentazioneDelCrs {
    CrsId,
    Srid,
    CrsDefinition,
}

impl RappresentazioneDelCrs {
    /// Il nome della rappresentazione, per la diagnostica strutturale.
    const fn nome(self) -> &'static str {
        match self {
            Self::CrsId => "crs_id",
            Self::Srid => "srid",
            Self::CrsDefinition => "crs_definition",
        }
    }

    /// La categoria di perdita, una delle **sei** che questa coppia produce.
    const fn categoria(self, stato: CrsRepresentationState) -> Option<&'static str> {
        match (self, stato) {
            (_, CrsRepresentationState::Preserved) => None,
            (Self::CrsId, CrsRepresentationState::Absent) => Some(CRS_ID_NOT_PRESERVED_ABSENT),
            (Self::CrsId, CrsRepresentationState::Derived(_)) => Some(CRS_ID_NOT_PRESERVED_DERIVED),
            (Self::Srid, CrsRepresentationState::Absent) => Some(SRID_NOT_PRESERVED_ABSENT),
            (Self::Srid, CrsRepresentationState::Derived(_)) => Some(SRID_NOT_PRESERVED_DERIVED),
            (Self::CrsDefinition, CrsRepresentationState::Absent) => {
                Some(CRS_DEFINITION_NOT_PRESERVED_ABSENT)
            }
            (Self::CrsDefinition, CrsRepresentationState::Derived(_)) => {
                Some(CRS_DEFINITION_NOT_PRESERVED_DERIVED)
            }
        }
    }
}

const CRS_ID_NOT_PRESERVED_ABSENT: &str = "crs_id_not_preserved_absent";
const CRS_ID_NOT_PRESERVED_DERIVED: &str = "crs_id_not_preserved_derived";
const SRID_NOT_PRESERVED_ABSENT: &str = "srid_not_preserved_absent";
const SRID_NOT_PRESERVED_DERIVED: &str = "srid_not_preserved_derived";
const CRS_DEFINITION_NOT_PRESERVED_ABSENT: &str = "crs_definition_not_preserved_absent";
const CRS_DEFINITION_NOT_PRESERVED_DERIVED: &str = "crs_definition_not_preserved_derived";

/// L'epoca delle coordinate di un CRS dinamico (`GeoParquet` 1.1, `epoch`), come
/// il contratto la porta: un metadato nativo, perche' il vocabolario Arrow non
/// ha una chiave per lei (rilettura 13).
const EPOCA_DELLE_COORDINATE: &str = "geoparquet.epoch";

/// I formati che scrivono l'epoca: `GeoParquet` nel campo `epoch` della
/// colonna, Arrow IPC nella chiave nativa del campo. Gli altri non hanno dove
/// metterla, e senza l'epoca le coordinate di un CRS dinamico si leggono a
/// un'altra data: la perdita si dichiara (`crs_coordinate_epoch_not_preserved`).
/// Fino alla rilettura 13 spariva in una conversione `lossless`.
const PORTANO_L_EPOCA: [&str; 2] = ["geoparquet", "ipc"];

const CRS_COORDINATE_EPOCH_NOT_PRESERVED: &str = "crs_coordinate_epoch_not_preserved";

/// IPC conserva le chiavi native. Gli altri writer non verificano bbox e
/// orientamento della sorgente sui batch ricevuti: potrebbero essere cambiati
/// o filtrati. Non copiare dichiarazioni non verificate e non perderle mute.
pub(super) fn record_geometry_metadata_loss(
    loss: &mut LossReport,
    formato: &str,
    geometry: &plenora_io_model::contract::GeometryColumnContract,
    dove: Posizione,
) {
    if formato == "ipc" {
        return;
    }
    if geometry.native_metadata.contains_key("geoparquet.bbox") {
        loss.record("geometry_bbox_not_preserved", 1);
        loss.add_example(LossExample {
            category: "geometry_bbox_not_preserved".to_owned(),
            posizione: dove,
            context: "riquadro non verificato sui dati scritti".to_owned(),
        });
    }
    if geometry
        .native_metadata
        .contains_key("geoparquet.orientation")
    {
        loss.record("geometry_orientation_not_preserved", 1);
        loss.add_example(LossExample {
            category: "geometry_orientation_not_preserved".to_owned(),
            posizione: dove,
            context: "orientamento non verificato sui dati scritti".to_owned(),
        });
    }
}

pub fn record_coordinate_epoch_loss(
    loss: &mut LossReport,
    formato: &str,
    geometry: &plenora_io_model::contract::GeometryColumnContract,
    dove: Posizione,
) {
    let Some(epoca) = geometry.native_metadata.get(EPOCA_DELLE_COORDINATE) else {
        return;
    };
    if PORTANO_L_EPOCA.contains(&formato) {
        return;
    }
    loss.record(CRS_COORDINATE_EPOCH_NOT_PRESERVED, 1);
    loss.add_example(LossExample {
        category: CRS_COORDINATE_EPOCH_NOT_PRESERVED.to_owned(),
        posizione: dove,
        context: format!(
            "representation=coordinate_epoch value_bytes={}",
            epoca.len()
        ),
    });
}

/// L'indice della colonna geometrica in `schema.fields()`.
///
/// Il contratto nomina la geometria, la posizione la conta: e' la stessa
/// sequenza che gli altri `field_index` indicizzano, quindi il numero e'
/// confrontabile con i loro. `None` se il contratto nomina una colonna che lo
/// schema non ha -- che sarebbe un'incoerenza da dichiarare altrove, non da
/// nascondere qui con uno zero.
fn indice_della_geometria(layer: &crate::request::WriteLayer, nome: &str) -> Option<u64> {
    layer
        .contract
        .schema
        .fields()
        .iter()
        .position(|field| field.name() == nome)
        .map(saturating_u64)
}

pub fn record_crs_representation_loss(
    loss: &mut LossReport,
    dove: Posizione,
    representation: RappresentazioneDelCrs,
    value_bytes: Option<usize>,
    state: CrsRepresentationState,
) {
    // Due guardie e non una tupla: la categoria e' una **chiave sul filo**, e
    // legarla da sola la rende leggibile a chi la cerca -- il gate del
    // vocabolario compreso, che deve poter risalire dall'uso alla costante.
    let Some(category) = representation.categoria(state) else {
        return;
    };
    let Some(value_bytes) = value_bytes else {
        return;
    };
    let nome = representation.nome();
    loss.record(category, 1);
    // `nome` viene dal nostro vocabolario chiuso e `value_bytes` e' una
    // lunghezza: nessuno dei due e' un identificatore preso dal file. Dove si
    // sia persa la rappresentazione lo dice `posizione`.
    loss.add_example(LossExample {
        category: category.to_owned(),
        posizione: dove,
        context: format!("representation={nome} value_bytes={value_bytes}"),
    });
}

pub fn assess_write_contract(
    descriptor: &FormatDescriptor,
    plan: &WritePlan,
) -> FidelityAssessment {
    let mut assessment =
        FidelityAssessment::for_format(descriptor.id(), descriptor.fidelity_class());
    let Some(capabilities) = descriptor.write_capabilities() else {
        return assessment;
    };

    // I quattro siti che portavano nomi presi dal file. Ciascuno emette ora due
    // cose: un testo **curato** con la posizione strutturata, che e' cio' che il
    // v2 pubblica, e la frase congelata alla lettera, che e' cio' che il v1
    // continua a pubblicare. Non si ricostruisce: si conserva, cosi' il
    // congelamento del v1 e' una tautologia invece di un invariante da
    // difendere a ogni ritocco di un `format!`.
    //
    // `field_index` e' l'indice in `schema.fields()` e conta **anche** la
    // colonna geometrica: quella e' la sequenza che questo ciclo attraversa, e
    // un indice che ne saltasse un elemento non sarebbe l'indice di questa
    // sequenza. Il ramo la salta come *ragione*, non come *posizione*.
    for (indice_layer, layer) in plan.layers.iter().enumerate() {
        let layer_index = Some(saturating_u64(indice_layer));
        let geometry_name = layer
            .contract
            .geometry
            .as_ref()
            .map(|geometry| geometry.name.as_str());
        for (indice_campo, field) in layer.contract.schema.fields().iter().enumerate() {
            let field_index = Some(saturating_u64(indice_campo));
            let dove = Posizione {
                layer_index,
                field_index,
                type_class: None,
            };
            let is_geometry = geometry_name == Some(field.name().as_str());
            if !is_geometry && capabilities.attributes == AttributeWriteSupport::LossReported {
                assessment.add_reason_redatta(
                    FidelityReasonCode::AttributeLoss,
                    "l'attributo non e' nativo del formato, o e' dichiarato come perdita",
                    dove,
                    format!(
                        "{}: attributo '{}' non nativo o loss-reported",
                        layer.name,
                        field.name()
                    ),
                );
            }
            let classe = crate::capabilities::arrow_type_class(field.data_type());
            if !capabilities.allowed_types.contains(&classe)
                && capabilities.type_coercion == TypeCoercionPolicy::LossReported
            {
                assessment.add_reason_redatta(
                    FidelityReasonCode::TypeCoercion,
                    "il tipo dell'attributo richiede una coercizione",
                    Posizione {
                        // La **classe**, non la forma `Debug` del tipo di
                        // `arrow`: quella e' di una dipendenza, e un suo
                        // aggiornamento cambierebbe la busta senza che nessuno
                        // tocchi il protocollo.
                        type_class: Some(classe),
                        ..dove
                    },
                    format!(
                        "{}: tipo {:?} di '{}' richiede coercion",
                        layer.name,
                        field.data_type(),
                        field.name()
                    ),
                );
            }
            if field.is_nullable() && capabilities.nullability == NullabilitySupport::FormatDefined
            {
                assessment.add_reason_redatta(
                    FidelityReasonCode::NullabilityChanged,
                    "la nullability dell'attributo la definisce il formato",
                    dove,
                    format!(
                        "{}: nullability di '{}' definita dal formato",
                        layer.name,
                        field.name()
                    ),
                );
            }
        }

        if descriptor.id() == "dxf"
            && layer.contract.geometry.as_ref().is_some_and(|geometry| {
                geometry.geometry_types.iter().any(|geometry_type| {
                    matches!(
                        geometry_type,
                        GeometryType::MultiPoint
                            | GeometryType::MultiLineString
                            | GeometryType::MultiPolygon
                            | GeometryType::GeometryCollection
                    )
                })
            })
        {
            assessment.add_reason_redatta(
                FidelityReasonCode::StructureChanged,
                "le geometrie multipart sono esplose in entita' singole",
                Posizione {
                    layer_index,
                    field_index: None,
                    type_class: None,
                },
                format!("{}: geometrie multipart esplose in entità DXF", layer.name),
            );
        }
    }
    assessment
}
