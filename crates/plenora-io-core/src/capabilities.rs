//! Validazione comune del `WritePlan` contro capability machine-readable.
//!
//! Vedi `ENGINEERING.md § Pipeline di scrittura`. I driver possono aggiungere
//! vincoli specifici, ma non saltare questi controlli di base.

use std::collections::BTreeSet;

use arrow_schema::DataType;
use plenora_io_model::crs::{definition_authority_srid, CrsResolution};
use plenora_io_model::{
    CapabilityReason, ContractIdentifier, NumeroStrutturale, PlenoraIoError, PublicMessage, Result,
};

use crate::descriptor::{
    ArrowTypeClass, AttributeWriteSupport, CrsRepresentationState, CrsWriteSupport,
    FormatDescriptor, NullabilitySupport, TextEncoding, TypeCoercionPolicy, ALL_GEOMETRY_TYPES,
};
use crate::request::WritePlan;

fn violation(
    driver: &'static str,
    field: Option<&ContractIdentifier>,
    reason: CapabilityReason,
    detail: &PublicMessage,
) -> PlenoraIoError {
    PlenoraIoError::capability_redatta(driver, field, reason, detail)
}

/// L'ordine memorizzato che il campo dichiara; `unknown` senza CRS, perche'
/// un campo `missing` non porta la chiave (ARROW-VOCABULARY-1.0 §4).
const fn declared_axis_order(
    geometry: &plenora_io_model::contract::GeometryColumnContract,
) -> plenora_io_model::crs::AxisOrder {
    match &geometry.crs {
        CrsResolution::Resolved(resolved) => resolved.axis_order,
        CrsResolution::DeclaredButUnresolved(raw) => raw.axis_order,
        CrsResolution::Missing => plenora_io_model::crs::AxisOrder::Unknown,
    }
}

fn declared_crs_id(crs: &CrsResolution) -> Option<&str> {
    match crs {
        CrsResolution::Resolved(resolved) => resolved.id.as_deref(),
        CrsResolution::DeclaredButUnresolved(raw) => raw.authority_hint.as_deref(),
        CrsResolution::Missing => None,
    }
}

/// La definizione dichiarata e la sua grammatica.
fn declared_definition(
    crs: &CrsResolution,
) -> (
    Option<&str>,
    Option<plenora_io_model::crs::CrsDefinitionFormat>,
) {
    match crs {
        CrsResolution::Resolved(resolved) => {
            (resolved.definition.as_deref(), resolved.definition_format)
        }
        CrsResolution::DeclaredButUnresolved(raw) => {
            (raw.definition.as_deref(), raw.definition_format)
        }
        CrsResolution::Missing => (None, None),
    }
}

/// Un bersaglio che conserva l'identificatore e non la definizione non sceglie
/// l'identificatore quando i due si contraddicono.
///
/// La regola e' quella di [`plenora_io_model::crs::crs_kind_for`], la stessa
/// con cui Shapefile, DXF, `GeoPackage` e `GeoParquet` rifiutano in lettura la
/// coppia: un identificatore che il bordo sa orizzontale su una radice che non
/// lo e', o di genere opposto. Un Arrow conforme la puo' portare (GEO-005
/// confronta solo gli identificatori), e IPC la conserva intera. Un bersaglio
/// che tiene solo l'identificatore -- `GeoParquet` con `EPSG:4326` e un WKT,
/// `GeoPackage`, `GeoJSON`, KML -- sceglierebbe uno dei due al posto di chi ha
/// scritto il file: fino alla rilettura 10 `GeoParquet` scriveva `EPSG:4326`
/// per un `LOCAL_CS` e si dichiarava `lossless`.
///
/// E' un limite del bersaglio su un ingresso conforme, quindi `unsupported`
/// (GEO-000 punto 2, REJ-001 «support»), con la ragione delle rappresentazioni
/// discordanti. Restano fuori solo i bersagli che non scelgono: quello che non
/// conserva l'identificatore (CSV, XLSX), quello che conserva la coppia intera
/// e la rilegge com'e' (IPC), e quello che la consegna a un runtime che la
/// riscrive (`FileGDB`).
///
/// Fino alla rilettura 11 restava fuori anche ogni bersaglio che conserva la
/// definizione, con l'idea che li' non si scegliesse niente. Non e' vero: il
/// lettore applica la stessa regola alla coppia che rilegge. `GeoParquet`
/// scriveva `EPSG:4326` dentro un PROJJSON `EngineeringCRS`, la conversione era
/// `lossless` e il file non si rileggeva (`crs`); lo Shapefile e il DXF
/// scrivevano un `LOCAL_CS` per `EPSG:4326`, dichiaravano l'identificatore
/// derivato, e il file si rileggeva non risolto.
fn verifica_identificatore_conservato(
    driver: &'static str,
    caps: &crate::descriptor::FormatWriteCapabilities,
    geometry: &plenora_io_model::contract::GeometryColumnContract,
) -> Result<()> {
    let (Some(id), (Some(definizione), grammatica)) = (
        declared_crs_id(&geometry.crs),
        declared_definition(&geometry.crs),
    ) else {
        return Ok(());
    };
    let rappresentazioni = &caps.crs_representations;
    if rappresentazioni.crs_id == CrsRepresentationState::Absent
        || rappresentazioni.conserva_la_coppia()
        || rappresentazioni.crs_definition_per(grammatica)
            == CrsRepresentationState::Derived(crate::descriptor::CrsDerivation::RuntimeResolved)
    {
        return Ok(());
    }
    match plenora_io_model::crs::crs_kind_for(Some(id), Some(definizione), driver) {
        Ok(_) => Ok(()),
        Err(errore) if errore.category == plenora_io_model::ErrorCategory::Crs => Err(violation(
            driver,
            ContractIdentifier::from_geometry_column(geometry).as_ref(),
            CapabilityReason::CrsRepresentationsInconsistent,
            &PublicMessage::Curated(
                "l'identificatore e la definizione del CRS si contraddicono nel genere, e il \
                 formato non li conserva com'e' la coppia",
            ),
        )),
        Err(errore) => Err(errore),
    }
}

/// Un CRS risolto non si scrive dove i lettori lo rileggerebbero non
/// risolto, se non c'e' un identificatore con cui dichiararne la perdita.
///
/// E' lo Shapefile e il DXF, che dell'identificatore scrivono solo il WKT: la
/// regola dei lettori (`identificatore_wkt`) ne ricava uno soltanto da
/// un'autorita' EPSG di radice o dalla forma ESRI di WGS 84. Un `PROJCS`
/// anonimo senza identificatore nel contratto si rileggeva «dichiarato ma non
/// risolto», e la conversione si dichiarava `conditional` senza nessuna
/// ragione CRS (rilettura 11). Con un identificatore la perdita si dichiara
/// (`crs_id_not_preserved_absent`, il verdetto per piano) e `--assume-crs` lo
/// riporta; senza, non c'e' niente da riportare. Un WKT che i lettori non
/// rileggono affatto (ambiguo, malformato) si rifiuta in entrambi i casi.
fn verifica_rilettura_risolta(
    driver: &'static str,
    caps: &crate::descriptor::FormatWriteCapabilities,
    geometry: &plenora_io_model::contract::GeometryColumnContract,
) -> Result<()> {
    let CrsResolution::Resolved(crs) = &geometry.crs else {
        return Ok(());
    };
    let riletto = caps.crs_representations.identificatore_riletto(
        crs.id.as_deref(),
        crs.definition.as_deref(),
        crs.definition_format,
    );
    match (riletto, crs.id.as_deref()) {
        (None | Some(Ok(Some(_))), _) | (Some(Ok(None)), Some(_)) => Ok(()),
        (Some(Ok(None)), None) | (Some(Err(_)), _) => Err(violation(
            driver,
            ContractIdentifier::from_geometry_column(geometry).as_ref(),
            CapabilityReason::CrsUnresolved,
            &PublicMessage::Curated(
                "il formato porta il CRS nel solo WKT, e da questo WKT i lettori non \
                 ricavano un identificatore: il CRS si rileggerebbe non risolto",
            ),
        )),
    }
}

pub(crate) fn comparable_crs_representations(
    geometry: &plenora_io_model::contract::GeometryColumnContract,
) -> [Option<i64>; 3] {
    let (definition, definition_format) = declared_definition(&geometry.crs);
    [
        declared_crs_id(&geometry.crs)
            .and_then(plenora_io_model::crs::authority_srid)
            .map(i64::from),
        geometry.srid.map(i64::from),
        definition
            .zip(definition_format)
            .and_then(|(value, format)| definition_authority_srid(value, format))
            .map(i64::from),
    ]
}

fn crs_representations_are_inconsistent(
    geometry: &plenora_io_model::contract::GeometryColumnContract,
) -> bool {
    known_crs_values_disagree(comparable_crs_representations(geometry))
}

pub(crate) fn known_crs_values_disagree(values: [Option<i64>; 3]) -> bool {
    let mut known = values.into_iter().flatten();
    let Some(first) = known.next() else {
        return false;
    };
    known.any(|value| value != first)
}

#[must_use]
pub const fn arrow_type_class(data_type: &DataType) -> ArrowTypeClass {
    use DataType::{
        Binary, BinaryView, Boolean, Date32, Date64, Decimal128, Decimal256, Decimal32, Decimal64,
        Duration, FixedSizeBinary, FixedSizeList, Float16, Float32, Float64, Int16, Int32, Int64,
        Int8, Interval, LargeBinary, LargeList, LargeListView, LargeUtf8, List, ListView, Map,
        Struct, Time32, Time64, Timestamp, UInt16, UInt32, UInt64, UInt8, Union, Utf8, Utf8View,
    };
    match data_type {
        Boolean => ArrowTypeClass::Boolean,
        Int8 | Int16 | Int32 | Int64 => ArrowTypeClass::SignedInteger,
        UInt8 | UInt16 | UInt32 | UInt64 => ArrowTypeClass::UnsignedInteger,
        Float16 | Float32 | Float64 => ArrowTypeClass::Floating,
        Utf8 | LargeUtf8 | Utf8View => ArrowTypeClass::Utf8,
        Binary | LargeBinary | BinaryView | FixedSizeBinary(_) => ArrowTypeClass::Binary,
        Date32 | Date64 | Time32(_) | Time64(_) | Timestamp(_, _) | Duration(_) | Interval(_) => {
            ArrowTypeClass::Temporal
        }
        Decimal32(_, _) | Decimal64(_, _) | Decimal128(_, _) | Decimal256(_, _) => {
            ArrowTypeClass::Decimal
        }
        List(_)
        | ListView(_)
        | FixedSizeList(_, _)
        | LargeList(_)
        | LargeListView(_)
        | Struct(_)
        | Union(_, _)
        | Map(_, _) => ArrowTypeClass::Nested,
        _ => ArrowTypeClass::Other,
    }
}

/// Valida tutto ciò che è determinabile dal contratto statico. Vincoli che
/// richiedono valori (per esempio tipo geometrico ignoto fino al primo record)
/// restano una seconda guardia nel writer.
///
/// # Errors
///
/// Restituisce [`plenora_io_model::PlenoraIoError`] con categoria
/// `Unsupported` al primo vincolo di capability violato dal piano: piano
/// vuoto, layer multipli o nomi duplicati, nomi di campo non rappresentabili,
/// tipo Arrow non supportato, geometria/encoding/dimensioni/semantica non
/// ammesse, CRS non risolvibile o rappresentazioni CRS incoerenti,
/// nullability non esprimibile.
// Sequenza lineare di guardie, una per vincolo di capability: la lunghezza e'
// nel numero di vincoli del contratto, non in complessita' logica. Restano
// nell'ordine per essere confrontabili con la matrice delle capability.
#[allow(clippy::too_many_lines)]
/// Verifica statica del piano di scrittura contro le capability del driver.
///
/// Prende `max_columns` invece dell'intero `Limits` perche' e' l'unica quota
/// che consulta; le `format_options` sono un parametro a parte per la stessa
/// ragione, e non l'intero `WriteOptions`. La differenza non e' cosmetica: legare questa firma al tipo
/// legacy costringerebbe ogni chiamante a possedere un `Limits` anche dopo la
/// migrazione al modello unificato, cioe' terrebbe in vita il tipo vecchio
/// per un campo solo.
pub fn validate_write(
    descriptor: &FormatDescriptor,
    plan: &WritePlan,
    max_columns: usize,
    format_options: &std::collections::BTreeMap<String, String>,
) -> Result<()> {
    let driver = descriptor.id();
    // Stesso motivo della lettura: le opzioni arrivano per parametro, cosi'
    // nessun driver puo' scrivere senza averle sottoposte allo schema.
    plenora_io_model::format_options::valida_opzioni(
        driver,
        descriptor.format_options(),
        format_options,
        plenora_io_model::format_options::FaseOpzione::Scrittura,
    )?;
    let caps = descriptor.write_capabilities().ok_or_else(|| {
        violation(
            driver,
            None,
            CapabilityReason::TypeNotRepresentable,
            &PublicMessage::Curated("driver scrivibile senza capability dichiarate"),
        )
    })?;

    if plan.layers.is_empty() {
        return Err(violation(
            driver,
            None,
            CapabilityReason::EmptyWritePlan,
            &PublicMessage::Curated("WritePlan senza layer"),
        ));
    }
    if !caps.multi_layer && plan.layers.len() != 1 {
        return Err(violation(
            driver,
            None,
            CapabilityReason::MultipleLayers,
            &PublicMessage::CuratedWith(
                "atteso un solo layer, ricevuti",
                NumeroStrutturale::Conteggio(crate::driver::saturating_u64(plan.layers.len())),
            ),
        ));
    }

    let mut layer_names = BTreeSet::new();
    for (indice_layer, layer) in plan.layers.iter().enumerate() {
        if layer.name.is_empty() || !layer_names.insert(layer.name.clone()) {
            // Il nome non entra nel messaggio: il layer si nomina per
            // indice nel piano, che il chiamante ha scritto e sa rileggere.
            return Err(violation(
                driver,
                None,
                CapabilityReason::DuplicateLayerName,
                &PublicMessage::CuratedWith(
                    "nome layer vuoto o duplicato al layer",
                    NumeroStrutturale::Indice(crate::driver::saturating_u64(indice_layer)),
                ),
            ));
        }
        if layer.contract.schema.fields().len() > max_columns {
            return Err(PlenoraIoError::limite_redatto(
                &PublicMessage::CuratedBetween(
                    "layer con",
                    NumeroStrutturale::Conteggio(crate::driver::saturating_u64(
                        layer.contract.schema.fields().len(),
                    )),
                    "colonne oltre il limite di",
                    NumeroStrutturale::Limite(crate::driver::saturating_u64(max_columns)),
                ),
            ));
        }

        let geometries = layer.contract.geometries()?;
        if geometries.len() > 1 && driver != "geoparquet" {
            return Err(PlenoraIoError::non_supportato_redatto(
                &PublicMessage::Curated(
                    "il driver di destinazione non rappresenta più colonne geometriche",
                ),
            ));
        }
        if layer.contract.geometry.is_none()
            && (geometries.len() > 1
                || (driver == "geoparquet"
                    && geometries
                        .iter()
                        .any(|g| g.native_metadata.contains_key("geoparquet.primary_column"))))
        {
            return Err(PlenoraIoError::non_supportato_redatto(
                &PublicMessage::Curated(
                    "la scrittura multigeometria richiede una primaria esplicita",
                ),
            ));
        }
        let mut normalized_names = BTreeSet::new();
        for (indice, field) in layer.contract.schema.fields().iter().enumerate() {
            let name = field.name();
            // L'identificatore nasce dallo schema che dichiara il campo:
            // `None` quando il nome non e' nominabile, e in quel caso
            // l'errore resta senza campo invece di portarne uno inventato.
            let campo = u32::try_from(indice).ok().and_then(|posizione| {
                ContractIdentifier::from_schema_field(
                    &layer.contract.schema,
                    plenora_io_model::contract::FieldId(posizione),
                )
            });
            if caps
                .field_names
                .max_bytes
                .is_some_and(|limit| name.len() > limit)
                || caps
                    .field_names
                    .max_chars
                    .is_some_and(|limit| name.chars().count() > limit)
            {
                return Err(violation(
                    driver,
                    campo.as_ref(),
                    CapabilityReason::FieldNameTooLong,
                    &PublicMessage::Curated("nome oltre il limite del formato"),
                ));
            }
            if caps.field_names.encoding == TextEncoding::Ascii && !name.is_ascii() {
                return Err(violation(
                    driver,
                    campo.as_ref(),
                    CapabilityReason::FieldNameEncoding,
                    &PublicMessage::Curated("il formato richiede nomi ASCII"),
                ));
            }
            if caps
                .field_names
                .reserved_names
                .iter()
                .any(|reserved| reserved.eq_ignore_ascii_case(name))
            {
                return Err(violation(
                    driver,
                    campo.as_ref(),
                    CapabilityReason::FieldNameCollision,
                    &PublicMessage::Curated("nome riservato dal formato"),
                ));
            }
            let normalized = if caps.field_names.case_sensitive {
                name.clone()
            } else {
                name.to_lowercase()
            };
            if !normalized_names.insert(normalized) {
                return Err(violation(
                    driver,
                    campo.as_ref(),
                    CapabilityReason::FieldNameCollision,
                    &PublicMessage::Curated("collisione dopo la normalizzazione del formato"),
                ));
            }

            if geometries.iter().any(|g| g.name == *name) {
                continue;
            }
            match caps.attributes {
                AttributeWriteSupport::None => {
                    return Err(violation(
                        driver,
                        campo.as_ref(),
                        CapabilityReason::TypeNotRepresentable,
                        &PublicMessage::Curated("il formato non rappresenta attributi"),
                    ))
                }
                AttributeWriteSupport::NamedSubset(names)
                    if !names.iter().any(|allowed| *allowed == name) =>
                {
                    return Err(violation(
                        driver,
                        campo.as_ref(),
                        CapabilityReason::TypeNotRepresentable,
                        &PublicMessage::Curated("attributo fuori dal sottoinsieme rappresentabile"),
                    ))
                }
                AttributeWriteSupport::All
                | AttributeWriteSupport::NamedSubset(_)
                | AttributeWriteSupport::LossReported => {}
            }

            let class = arrow_type_class(field.data_type());
            if !caps.allowed_types.contains(&class)
                && caps.type_coercion == TypeCoercionPolicy::Reject
            {
                return Err(violation(
                    driver,
                    campo.as_ref(),
                    CapabilityReason::TypeNotRepresentable,
                    &PublicMessage::Curated(class.nome()),
                ));
            }
            if caps.nullability == NullabilitySupport::NoNulls && field.is_nullable() {
                return Err(violation(
                    driver,
                    campo.as_ref(),
                    CapabilityReason::Nullability,
                    &PublicMessage::Curated("campo nullable non supportato"),
                ));
            }
        }

        // Il piano legacy monogeometria senza contratto esplicito conserva
        // la validazione runtime storica; i nuovi piani multigeometria sono espliciti.
        for geometry in geometries
            .iter()
            .filter(|_| layer.contract.geometry.is_some() || geometries.len() > 1)
        {
            if !caps.geometry.supported {
                return Err(violation(
                    driver,
                    ContractIdentifier::from_geometry_column(geometry).as_ref(),
                    CapabilityReason::GeometryNotSupported,
                    &PublicMessage::Curated("geometria non supportata"),
                ));
            }
            if !caps.geometry.encodings.contains(&geometry.encoding) {
                return Err(violation(
                    driver,
                    ContractIdentifier::from_geometry_column(geometry).as_ref(),
                    CapabilityReason::GeometryEncoding,
                    &PublicMessage::Curated(geometry.encoding.nome()),
                ));
            }
            if !caps.geometry.dimensions.contains(&geometry.dimensions) {
                return Err(violation(
                    driver,
                    ContractIdentifier::from_geometry_column(geometry).as_ref(),
                    CapabilityReason::CoordinateDimensions,
                    &PublicMessage::Curated(geometry.dimensions.nome()),
                ));
            }
            if !caps
                .geometry
                .spatial_semantics
                .contains(&geometry.spatial_semantics)
            {
                return Err(violation(
                    driver,
                    ContractIdentifier::from_geometry_column(geometry).as_ref(),
                    CapabilityReason::SpatialSemantics,
                    &PublicMessage::Curated(geometry.spatial_semantics.nome()),
                ));
            }
            let declared_mixed =
                geometry.types_declaration == plenora_io_model::contract::TypesDeclaration::Mixed;
            if !caps.geometry.mixed_types && (declared_mixed || geometry.geometry_types.len() > 1) {
                return Err(violation(
                    driver,
                    ContractIdentifier::from_geometry_column(geometry).as_ref(),
                    CapabilityReason::MixedGeometry,
                    &PublicMessage::Curated("il formato richiede un solo tipo geometrico"),
                ));
            }
            let unique_geometry_types = geometry
                .geometry_types
                .iter()
                .copied()
                .collect::<BTreeSet<_>>();
            if unique_geometry_types.len() != geometry.geometry_types.len() {
                return Err(violation(
                    driver,
                    ContractIdentifier::from_geometry_column(geometry).as_ref(),
                    CapabilityReason::MixedGeometry,
                    &PublicMessage::Curated("il contratto geometrico contiene tipi duplicati"),
                ));
            }
            let restricts_geometry_types = caps.geometry.geometry_types.len()
                != ALL_GEOMETRY_TYPES.len()
                || !ALL_GEOMETRY_TYPES
                    .iter()
                    .all(|geometry_type| caps.geometry.geometry_types.contains(geometry_type));
            // `scansione_completa` separa i due stati che `Unresolved`
            // conflava. Una sorgente percorsa fino in fondo che non ha tipi non
            // ha niente da dichiarare, e pretendere da lei una dichiarazione
            // preventiva rifiutava una conversione che il file puo' fare. Chi
            // non l'ha percorsa resta rifiutato: li' l'ignoranza e' reale, e un
            // sink che restringe i tipi la scoprirebbe a scrittura iniziata.
            let assenza_accertata =
                geometry.scansione_completa && geometry.geometry_types.is_empty();
            if restricts_geometry_types
                && !assenza_accertata
                && matches!(
                    geometry.types_declaration,
                    plenora_io_model::contract::TypesDeclaration::Unresolved
                        | plenora_io_model::contract::TypesDeclaration::LegacyUndeclared
                )
            {
                return Err(violation(
                    driver,
                    ContractIdentifier::from_geometry_column(geometry).as_ref(),
                    CapabilityReason::GeometryNotSupported,
                    &PublicMessage::Curated(
                        "il formato richiede una dichiarazione preventiva dei tipi geometrici",
                    ),
                ));
            }
            if let Some(unsupported) = geometry
                .geometry_types
                .iter()
                .find(|geometry_type| !caps.geometry.geometry_types.contains(geometry_type))
            {
                return Err(violation(
                    driver,
                    ContractIdentifier::from_geometry_column(geometry).as_ref(),
                    CapabilityReason::GeometryNotSupported,
                    &PublicMessage::Curated(unsupported.canonical_name()),
                ));
            }
            // GEO-001: un formato che fissa easting o longitudine prima
            // scriverebbe invertito un campo con latitudine o northing prima.
            // Il bersaglio non rappresenta l'ingresso: `unsupported` prima di
            // ogni effetto (GEO-000 punto 2), qui per tutti i driver.
            if !caps
                .geometry
                .axis_orders
                .contains(&declared_axis_order(geometry))
            {
                return Err(violation(
                    driver,
                    ContractIdentifier::from_geometry_column(geometry).as_ref(),
                    CapabilityReason::AxisOrderNotRepresentable,
                    &PublicMessage::Curated(
                        "il formato memorizza easting o longitudine prima, il campo dichiara un altro ordine",
                    ),
                ));
            }
            match caps.crs {
                CrsWriteSupport::Embedded if geometry.resolved_crs().is_none() => {
                    return Err(violation(
                        driver,
                        ContractIdentifier::from_geometry_column(geometry).as_ref(),
                        CapabilityReason::CrsUnresolved,
                        &PublicMessage::Curated("il formato richiede un CRS risolto"),
                    ))
                }
                // `missing` si scrive come «CRS ignoto»; il dichiarato ma non
                // risolto no: il formato non ha un modo di dirlo.
                CrsWriteSupport::EmbeddedOrUnknown
                    if matches!(geometry.crs, CrsResolution::DeclaredButUnresolved(_)) =>
                {
                    return Err(violation(
                        driver,
                        ContractIdentifier::from_geometry_column(geometry).as_ref(),
                        CapabilityReason::CrsUnresolved,
                        &PublicMessage::Curated(
                            "il formato scrive un CRS risolto o un CRS ignoto, non un CRS \
                             dichiarato e non risolto",
                        ),
                    ))
                }
                CrsWriteSupport::Fixed(expected) if geometry.crs.id() != Some(expected) => {
                    return Err(violation(
                        driver,
                        ContractIdentifier::from_geometry_column(geometry).as_ref(),
                        CapabilityReason::ReprojectionRequired,
                        // Il CRS atteso e quello dichiarato non entrano nel
                        // testo: il primo e' leggibile dalle capability del
                        // driver, il secondo dal contratto. Metterli qui
                        // significherebbe far uscire dal bordo una stringa
                        // che il chiamante ha gia'.
                        &PublicMessage::Curated(
                            "il formato impone un CRS fisso diverso da quello del contratto",
                        ),
                    ));
                }
                CrsWriteSupport::Embedded
                | CrsWriteSupport::EmbeddedOptional
                | CrsWriteSupport::EmbeddedOrUnknown
                | CrsWriteSupport::Fixed(_)
                | CrsWriteSupport::None => {}
            }
            verifica_identificatore_conservato(driver, &caps, geometry)?;
            verifica_rilettura_risolta(driver, &caps, geometry)?;
            let [comparable_id, comparable_srid, comparable_definition] =
                comparable_crs_representations(geometry);
            let (definizione, grammatica) = declared_definition(&geometry.crs);
            let representations_are_preserved = comparable_id.is_none()
                || caps.crs_representations.crs_id_per(
                    declared_crs_id(&geometry.crs),
                    definizione,
                    grammatica,
                ) == CrsRepresentationState::Preserved;
            let representations_are_preserved = representations_are_preserved
                && (comparable_srid.is_none()
                    || caps.crs_representations.srid == CrsRepresentationState::Preserved);
            let representations_are_preserved = representations_are_preserved
                && (comparable_definition.is_none()
                    || caps.crs_representations.crs_definition_per(grammatica)
                        == CrsRepresentationState::Preserved);
            if crs_representations_are_inconsistent(geometry) && !representations_are_preserved {
                return Err(violation(
                    driver,
                    ContractIdentifier::from_geometry_column(geometry).as_ref(),
                    CapabilityReason::CrsRepresentationsInconsistent,
                    &PublicMessage::Curated(
                        "rappresentazioni CRS discordanti non preservabili indipendentemente",
                    ),
                ));
            }
        }
    }
    // La geometria richiesta dal bersaglio si guarda in una passata a se',
    // **dopo** i controlli strutturali del ciclo principale: un piano con due
    // layer omonimi deve sentirsi dire che i nomi collidono, non che al primo
    // manca la geometria. La precedenza fra i rifiuti e' parte di cio' che il
    // chiamante legge.
    for layer in &plan.layers {
        // Un bersaglio che **incorpora o fissa** un CRS e' un formato spaziale:
        // il CRS sta su una geometria, e senza geometria non c'e' niente a cui
        // attaccarlo. GeoParquet vuole una `primary_column` nel proprio
        // metadato `geo`, lo Shapefile una geometria per record, KML un
        // `Placemark`: un layer di soli attributi non e' un dataset di quei
        // formati.
        //
        // Il rifiuto sta **qui** e non nel writer perche' e' una capability del
        // bersaglio, e va pronunciata in validazione con la propria ragione:
        // scoprirlo mentre si scrive lo faceva arrivare come errore di formato
        // in fase di lettura, cioe' nominando la fase sbagliata e senza dire
        // che era il bersaglio a non poterlo rappresentare.
        //
        // `EmbeddedOptional` e `None` restano fuori, ed e' voluto: Arrow IPC,
        // CSV e XLSX portano tabelle senza geometria e non hanno un CRS da
        // attaccare a nulla.
        // «Senza geometria» si decide come lo decide il writer: contratto
        // geometrico assente **e** nessuna colonna `geoarrow.wkb` nello schema.
        // Guardare il solo contratto non basterebbe -- un piano puo' portare la
        // geometria nel solo schema Arrow, ed e' li' che il writer la cerca --
        // e rifiutarlo sarebbe rifiutare un dataset spaziale scambiandolo per
        // una tabella.
        let senza_geometria = layer.contract.geometry.is_none()
            && !layer
                .contract
                .schema
                .fields()
                .iter()
                .any(|field| plenora_io_model::geometry::is_geometry_field(field));
        if senza_geometria
            && matches!(
                caps.crs,
                CrsWriteSupport::Embedded
                    | CrsWriteSupport::EmbeddedOrUnknown
                    | CrsWriteSupport::Fixed(_)
            )
        {
            return Err(violation(
                driver,
                None,
                CapabilityReason::GeometryNotSupported,
                &PublicMessage::Curated(
                    "il formato di destinazione richiede una colonna geometrica, e il layer non ne ha",
                ),
            ));
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests;
