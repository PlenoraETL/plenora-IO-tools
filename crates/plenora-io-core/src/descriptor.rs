//! `FormatDescriptor` — catalogo machine-readable dei driver (Architetture §2.3).

use serde::Serialize;

use plenora_io_model::contract::{
    CoordinateDimensions, GeometryEncoding, GeometryType, SpatialSemantics,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    Read,
    Write,
    Bidirectional,
}

/// Modalità di lettura, per-driver e per-versione (D9): non permanente.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReadMode {
    StreamingSequential,
    StreamingColumnar,
    Materializing,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WriteMode {
    Streaming,
    Buffered,
}

/// Cosa fa il **parser grezzo** del driver, prima di qualunque adapter (INV-7).
///
/// E' il primo dei tre assi che `read_mode` conflava in un valore solo. Un
/// consumatore che leggeva `StreamingSequential` non poteva sapere se il
/// consumo effettivo fosse streaming, spooled o in memoria: la tripla
/// `(native_read_mode, effective_delivery, buffering)` lo esplicita.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum NativeReadMode {
    /// Una passata sola, batch emessi in ordine mentre la sorgente scorre.
    StreamingSequential,
    /// Il parser sa posizionarsi: row group Parquet, blocchi IPC, cursore su
    /// chiave. Non implica che il driver lo usi per saltare, implica che
    /// potrebbe.
    StreamingRandom,
    /// Il parser **consuma o materializza l'intero input** prima di emettere
    /// il primo batch.
    ///
    /// La definizione e' quella, non «carica tutto in RAM»: il supporto fisico
    /// lo descrive [`BufferingStrategy`] e nessun altro campo. Un parser che
    /// riversa tutta la sorgente in uno spool RAM-poi-disco e' `Materialized`
    /// con `AdaptiveMemoryThenDisk`, e la coppia dice esattamente cosa
    /// succede: serve tutto l'input, non serve tutta la RAM. Confondere i due
    /// assi era il difetto che INV-7 chiude, e ripeterlo dentro una singola
    /// variante lo reintrodurrebbe.
    Materialized,
}

/// Cosa il consumatore **osserva** a livello di contratto pubblico (INV-7).
///
/// Descrive *quando* il primo batch e' visibile e *cosa* succede se un errore
/// emerge dopo la consegna.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum DeliverySemantics {
    /// Nessun prefisso «accettato» viene consegnato se una violazione emerge
    /// in un punto qualsiasi della sorgente: l'operazione e' rigettata come un
    /// blocco unico.
    ///
    /// E' il comportamento di `BudgetedReader`, che esegue `drain_operation`
    /// durante la **prima** chiamata di `next_batch` — dichiarato da
    /// `ENGINEERING.md § Pipeline di lettura`.
    OperationAtomic,
    /// Batch consegnati appena disponibili, con errore terminale possibile
    /// dopo batch gia' consegnati.
    ///
    /// **Dichiarabile, non implementata nel Lotto 0**: richiede una categoria
    /// d'errore nuova e un bump del protocollo, non ratificati. La variante
    /// esiste perche' l'asse la prevede, non perche' qualcuno la selezioni.
    Streaming,
}

/// Come l'implementazione **bounda la memoria interna** (INV-7).
///
/// Ortogonale alla semantica di consegna: due driver con la stessa
/// `DeliverySemantics` possono avere impronte di risorse molto diverse, ed e'
/// questo campo — non `native_read_mode` — a dirlo.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum BufferingStrategy {
    /// Nessun buffer interno oltre il batch corrente.
    Passthrough,
    /// Buffer in RAM, bounded da `memory_bytes` del `PipelineContext`.
    InMemoryBounded,
    /// Resta in RAM sotto una soglia adattiva derivata da `memory_bytes`, poi
    /// migra su file temporaneo in Arrow IPC e **non torna indietro**.
    ///
    /// Il picco e' `soglia + batch corrente`, indipendente dalla dimensione
    /// totale dell'input. E' la strategia dello `StagedSpool` che
    /// `BudgetedReader` usa, descritta da `ENGINEERING.md § Spool e memoria`.
    AdaptiveMemoryThenDisk,
}

/// Livello di determinismo garantito a parità di input, opzioni e versione
/// dell'implementazione (ICD §12).
///
/// `Semantic` è la garanzia minima: stessi valori e stesso insieme di righe,
/// senza assumere un ordine o una rappresentazione fisica identici.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DeterminismLevel {
    Semantic,
    Ordered,
    ByteForByte,
    Unordered,
}

/// Fedeltà a tre livelli: dipende dal contratto, non solo dal formato (`PRODUCT.md § LossReport`).
/// Il descrittore porta la capacità generale; `open`/`create` la valutazione
/// specifica.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Fidelity {
    Lossless,
    Conditional,
    Approximating,
}

/// Concorrenza dei reader (`ENGINEERING.md § Interfaccia dei driver`): più espressiva di un bool.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReaderConcurrency {
    SingleActiveReader,
    MultipleIndependentReaders,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Runtime {
    PureRust,
    Gdal,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CrsHandling {
    Embedded,
    FixedWgs84,
    None,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TextEncoding {
    Utf8,
    Ascii,
    FormatDefined,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NameNormalization {
    None,
    Nfc,
    FormatDefined,
}

#[derive(Clone, Copy, Debug, Serialize)]
pub struct FieldNamePolicy {
    pub max_bytes: Option<usize>,
    pub max_chars: Option<usize>,
    pub encoding: TextEncoding,
    pub case_sensitive: bool,
    pub normalization: NameNormalization,
    pub reserved_names: &'static [&'static str],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ArrowTypeClass {
    Boolean,
    SignedInteger,
    UnsignedInteger,
    Floating,
    Utf8,
    Binary,
    Temporal,
    Decimal,
    Nested,
    Other,
}

impl ArrowTypeClass {
    /// Nome statico della classe Arrow, per i messaggi pubblici.
    #[must_use]
    pub const fn nome(self) -> &'static str {
        match self {
            Self::Boolean => "boolean",
            Self::SignedInteger => "signed_integer",
            Self::UnsignedInteger => "unsigned_integer",
            Self::Floating => "floating",
            Self::Utf8 => "utf8",
            Self::Binary => "binary",
            Self::Temporal => "temporal",
            Self::Decimal => "decimal",
            Self::Nested => "nested",
            Self::Other => "other",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TypeCoercionPolicy {
    Reject,
    ExplicitText,
    LossReported,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AttributeWriteSupport {
    All,
    NamedSubset(&'static [&'static str]),
    None,
    LossReported,
}

#[derive(Clone, Copy, Debug, Serialize)]
pub struct GeometryWriteSupport {
    pub supported: bool,
    pub encodings: &'static [GeometryEncoding],
    pub dimensions: &'static [CoordinateDimensions],
    pub spatial_semantics: &'static [SpatialSemantics],
    /// Tipi geometrici accettati dal formato nel profilo corrente.
    pub geometry_types: &'static [GeometryType],
    pub mixed_types: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CrsWriteSupport {
    /// Il formato richiede un CRS risolto e lo incorpora.
    Embedded,
    /// Il formato incorpora senza perdita anche gli stati dichiarato ma non
    /// risolto e assente.
    EmbeddedOptional,
    Fixed(&'static str),
    None,
}

/// Esito di una rappresentazione CRS presente nel contratto quando attraversa
/// un writer.
///
/// Solo `Preserved` conserva il valore sorgente in modo indipendente;
/// `Derived` indica che il formato o il driver ricostruisce il valore da
/// un'altra rappresentazione.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CrsRepresentationState {
    Preserved,
    Absent,
    Derived,
}

/// Capability generale del writer per le tre rappresentazioni CRS del
/// contratto.
///
/// È distinta da [`CrsWriteSupport`]: quest'ultima stabilisce se il formato
/// richiede/incorpora un CRS, questa struttura stabilisce cosa sopravvive
/// davvero al bordo di scrittura.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct CrsRepresentationCapabilities {
    pub crs_id: CrsRepresentationState,
    pub srid: CrsRepresentationState,
    pub crs_definition: CrsRepresentationState,
}

impl CrsRepresentationCapabilities {
    #[must_use]
    pub const fn new(
        crs_id: CrsRepresentationState,
        srid: CrsRepresentationState,
        crs_definition: CrsRepresentationState,
    ) -> Self {
        Self {
            crs_id,
            srid,
            crs_definition,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NullabilitySupport {
    Preserve,
    FormatDefined,
    NoNulls,
}

#[derive(Clone, Copy, Debug, Serialize)]
pub struct FormatWriteCapabilities {
    pub field_names: FieldNamePolicy,
    pub allowed_types: &'static [ArrowTypeClass],
    pub type_coercion: TypeCoercionPolicy,
    pub attributes: AttributeWriteSupport,
    pub geometry: GeometryWriteSupport,
    pub crs: CrsWriteSupport,
    pub crs_representations: CrsRepresentationCapabilities,
    pub nullability: NullabilitySupport,
    pub multi_layer: bool,
}

pub const UTF8_FIELD_NAMES: FieldNamePolicy = FieldNamePolicy {
    max_bytes: None,
    max_chars: None,
    encoding: TextEncoding::Utf8,
    case_sensitive: true,
    normalization: NameNormalization::None,
    reserved_names: &[],
};

pub const DBF_FIELD_NAMES: FieldNamePolicy = FieldNamePolicy {
    max_bytes: Some(10),
    max_chars: Some(10),
    encoding: TextEncoding::Ascii,
    case_sensitive: false,
    normalization: NameNormalization::FormatDefined,
    reserved_names: &[],
};

pub const SCALAR_TYPES: &[ArrowTypeClass] = &[
    ArrowTypeClass::Boolean,
    ArrowTypeClass::SignedInteger,
    ArrowTypeClass::UnsignedInteger,
    ArrowTypeClass::Floating,
    ArrowTypeClass::Utf8,
    ArrowTypeClass::Binary,
    ArrowTypeClass::Temporal,
    ArrowTypeClass::Decimal,
];

pub const ALL_ARROW_TYPES: &[ArrowTypeClass] = &[
    ArrowTypeClass::Boolean,
    ArrowTypeClass::SignedInteger,
    ArrowTypeClass::UnsignedInteger,
    ArrowTypeClass::Floating,
    ArrowTypeClass::Utf8,
    ArrowTypeClass::Binary,
    ArrowTypeClass::Temporal,
    ArrowTypeClass::Decimal,
    ArrowTypeClass::Nested,
    ArrowTypeClass::Other,
];

pub const ALL_GEOMETRY_TYPES: &[GeometryType] = &[
    GeometryType::Point,
    GeometryType::LineString,
    GeometryType::Polygon,
    GeometryType::MultiPoint,
    GeometryType::MultiLineString,
    GeometryType::MultiPolygon,
    GeometryType::GeometryCollection,
    GeometryType::CircularString,
    GeometryType::CompoundCurve,
    GeometryType::CurvePolygon,
    GeometryType::MultiCurve,
    GeometryType::MultiSurface,
    GeometryType::PolyhedralSurface,
    GeometryType::Tin,
    GeometryType::Triangle,
    GeometryType::Unknown,
];

/// Sette tipi semplici decodificati dal codec WKB locale. È distinto
/// dall'universo normativo R3.1: conoscere un tipo nel contratto non implica
/// che un driver sappia materializzarlo.
pub const SIMPLE_WKB_GEOMETRY_TYPES: &[GeometryType] = &[
    GeometryType::Point,
    GeometryType::LineString,
    GeometryType::Polygon,
    GeometryType::MultiPoint,
    GeometryType::MultiLineString,
    GeometryType::MultiPolygon,
    GeometryType::GeometryCollection,
];

pub const SHAPEFILE_GEOMETRY_TYPES: &[GeometryType] = &[
    GeometryType::Point,
    GeometryType::LineString,
    GeometryType::Polygon,
    GeometryType::MultiPoint,
    GeometryType::MultiLineString,
    GeometryType::MultiPolygon,
];

pub const WKB_XY_GEOMETRY: GeometryWriteSupport = GeometryWriteSupport {
    supported: true,
    encodings: &[GeometryEncoding::Wkb],
    dimensions: &[CoordinateDimensions::Xy],
    spatial_semantics: &[SpatialSemantics::Geometry],
    geometry_types: SIMPLE_WKB_GEOMETRY_TYPES,
    mixed_types: true,
};

/// GeoJSON/KML-like geometry support: XY plus an interoperable altitude Z.
/// M is deliberately excluded because these formats do not assign it a stable
/// round-trip semantic.
pub const WKB_XY_XYZ_GEOMETRY: GeometryWriteSupport = GeometryWriteSupport {
    supported: true,
    encodings: &[GeometryEncoding::Wkb],
    dimensions: &[
        CoordinateDimensions::Xy,
        CoordinateDimensions::Xyz,
        CoordinateDimensions::Unknown,
    ],
    spatial_semantics: &[SpatialSemantics::Geometry],
    geometry_types: SIMPLE_WKB_GEOMETRY_TYPES,
    mixed_types: true,
};

pub const WKB_SINGLE_TYPE_XY_GEOMETRY: GeometryWriteSupport = GeometryWriteSupport {
    mixed_types: false,
    ..WKB_XY_GEOMETRY
};

/// Shapefile-like support: one native shape family per dataset, with the
/// dimensional variants represented by the format itself.
pub const WKB_SINGLE_TYPE_ALL_DIMENSIONS_GEOMETRY: GeometryWriteSupport = GeometryWriteSupport {
    geometry_types: SHAPEFILE_GEOMETRY_TYPES,
    mixed_types: false,
    ..WKB_PASSTHROUGH_GEOMETRY
};

pub const WKB_PASSTHROUGH_GEOMETRY: GeometryWriteSupport = GeometryWriteSupport {
    supported: true,
    encodings: &[GeometryEncoding::Wkb],
    dimensions: &[
        CoordinateDimensions::Xy,
        CoordinateDimensions::Xyz,
        CoordinateDimensions::Xym,
        CoordinateDimensions::Xyzm,
        CoordinateDimensions::Unknown,
    ],
    spatial_semantics: &[SpatialSemantics::Geometry],
    geometry_types: SIMPLE_WKB_GEOMETRY_TYPES,
    mixed_types: true,
};

pub const WKB_EWKB_PASSTHROUGH_GEOMETRY: GeometryWriteSupport = GeometryWriteSupport {
    encodings: &[GeometryEncoding::Wkb, GeometryEncoding::Ewkb],
    spatial_semantics: &[SpatialSemantics::Geometry, SpatialSemantics::Geography],
    ..WKB_PASSTHROUGH_GEOMETRY
};

pub const NO_GEOMETRY: GeometryWriteSupport = GeometryWriteSupport {
    supported: false,
    encodings: &[],
    dimensions: &[],
    spatial_semantics: &[],
    geometry_types: &[],
    mixed_types: false,
};

/// Garanzia offerta dal reader per `ReadRequest::projected_fields` (`ENGINEERING.md § Projection e pruning`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectionSupport {
    /// La projection può essere ignorata in modalità `BestEffort`.
    None,
    /// Il reader può produrre esattamente e soltanto i campi richiesti.
    Exact,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PredicatePruningSupport {
    None,
    NumericMinMaxStatistics,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SpatialPruningSupport {
    None,
    BoundingBoxStatistics,
    OptionalRtreeIndex,
}

/// Il descrittore di un driver: le sue capability, dichiarate.
///
/// # INV-14 — costruzione solo via [`FormatDescriptor::const_new`]
///
/// La struct e' `#[non_exhaustive]` e tutti i campi sono privati, quindi il
/// literal non compila da fuori:
///
/// ```compile_fail
/// use plenora_io_core::{Direction, FormatDescriptor, ReadMode};
/// let _ = FormatDescriptor {
///     id: "finto",
///     direction: Direction::Read,
///     read_mode: ReadMode::StreamingSequential,
/// };
/// ```
///
/// Nemmeno con l'aggiornamento funzionale, che `#[non_exhaustive]` vieta
/// altrettanto:
///
/// ```compile_fail
/// use plenora_io_core::FormatDescriptor;
/// fn variante(base: &FormatDescriptor) -> FormatDescriptor {
///     FormatDescriptor { id: "finto", ..base.clone() }
/// }
/// ```
///
/// E i campi non si leggono direttamente, solo con i getter:
///
/// ```compile_fail
/// fn identita(descriptor: &plenora_io_core::FormatDescriptor) -> &'static str {
///     descriptor.id
/// }
/// ```
///
/// Che invece con il getter compila:
///
/// ```
/// fn identita(descriptor: &plenora_io_core::FormatDescriptor) -> &'static str {
///     descriptor.id()
/// }
/// ```
///
/// Non e' una formalita'. Un campo aggiunto a `const_new` diventa un errore di
/// compilazione in tutti e dieci i driver: chi lo aggiunge deve decidere il
/// valore per ognuno, invece di lasciarne dieci al default di qualcun altro.
/// I tre assi di INV-7 esistono perche' un descrittore che tace **e'** un
/// descrittore che mente.
#[derive(Clone, Debug, Serialize)]
#[non_exhaustive]
pub struct FormatDescriptor {
    id: &'static str,
    direction: Direction,
    /// **Legacy**, e preservato driver per driver, byte per byte.
    ///
    /// Conflava i tre assi di INV-7 in un valore solo, ed e' la ragione per
    /// cui la tripla esiste. Non viene derivato dai nuovi campi ne'
    /// riallineato a essi: `plenora-io-catalog-v1` lo emette da sempre, e
    /// cambiarlo per farlo «tornare» con `native_read_mode` romperebbe i
    /// consumatori senza aggiungere verita' — la divergenza fra i due **e'**
    /// l'informazione. `FileGDB` e' l'esempio: qui `Materializing`, nativamente
    /// una passata sola.
    read_mode: ReadMode,
    /// Cosa fa il parser grezzo (INV-7).
    native_read_mode: NativeReadMode,
    /// Cosa osserva il consumatore (INV-7).
    effective_delivery: DeliverySemantics,
    /// Come e' bounded la memoria interna (INV-7).
    buffering: BufferingStrategy,
    /// Garanzia dell'operazione di lettura sul medesimo snapshot locale.
    read_determinism: DeterminismLevel,
    write_mode: Option<WriteMode>,
    /// Garanzia dell'operazione di scrittura; `None` per i driver read-only.
    write_determinism: Option<DeterminismLevel>,
    multi_layer: bool,
    multi_file: bool,
    /// Concorrenza dei reader ammessa dal formato (`ENGINEERING.md § Interfaccia dei driver`).
    reader_concurrency: ReaderConcurrency,
    /// Garanzia di projection applicabile al `ReadRequest` (`ENGINEERING.md § Projection e pruning`).
    projection_support: ProjectionSupport,
    /// Pruning attributivo disponibile senza filtering riga-per-riga.
    predicate_pruning_support: PredicatePruningSupport,
    /// Pruning spaziale nativo; può dipendere dal dataset aperto.
    spatial_pruning_support: SpatialPruningSupport,
    crs_handling: CrsHandling,
    /// Capacità generale di fedeltà; la valutazione per-contratto è in open/create.
    fidelity_class: Fidelity,
    runtime: Runtime,
    /// Il parsing degli input non fidati e' **bounded durante il parse**.
    ///
    /// `true` dice una cosa precisa e verificabile: ogni testo che questo
    /// driver interpreta come geometria passa da un'analisi che applica i
    /// tetti del bordo -- byte, componenti, profondita' -- **mentre** consuma,
    /// non dopo aver costruito l'albero. Cio' che non e' stato letto non e'
    /// stato allocato.
    ///
    /// `false` non dice «insicuro»: dice **non dichiarato**. Un driver che
    /// legge un formato binario ha altre difese -- prevalidazione, decoder
    /// bounded -- e questa capability non le riassume. Riassumerle in un
    /// booleano solo sarebbe il modo di renderlo inutile.
    ///
    /// Chi la dichiara `true` deve avere una misura di profondita' che lo
    /// dimostri: `scripts/check_capability_input_ostile.py` confronta questa
    /// riga con i moduli che il driver attraversa davvero.
    hostile_input_hardened: bool,
    /// La versione massima della specifica del formato che il driver legge
    /// **per intero**, o `None` se il formato non si versiona cosi'.
    ///
    /// Serve a dire dove il supporto si ferma. Senza, un consumatore che vede
    /// `geoparquet` nel catalogo non ha modo di sapere se una 2.0 sarebbe
    /// letta: dedurrebbe di si', e si sbaglierebbe. Il driver che la dichiara
    /// rifiuta le versioni oltre con un errore di funzionalita' non
    /// supportata, distinto da quello di metadati non conformi.
    spec_version_supported: Option<&'static str>,
    write_capabilities: Option<FormatWriteCapabilities>,
    /// Le `format_options` che il driver interpreta (L0.7, S6).
    ///
    /// Il campo e' **obbligatorio**, e non `Option`: un driver che non
    /// interpreta alcuna opzione dichiara `SchemaOpzioniFormato::VUOTO`, che e'
    /// un'affermazione — qualunque chiave e' sconosciuta — mentre l'assenza
    /// sarebbe un'omissione indistinguibile da una dimenticanza.
    ///
    /// Sta qui e non in una tabella indicizzata per `id` perche' il legame
    /// dev'essere **strutturale**: un driver senza schema non compila, invece
    /// di lasciare un buco che solo un test troverebbe. Il registry per il
    /// comando `options` si compone dall'elenco dei driver, che il core ha
    /// gia'.
    format_options: plenora_io_model::format_options::SchemaOpzioniFormato,
    // Versioni esplicite: il fingerprint del catalogo deriva da queste (D17).
    semantic_version: u32,
    driver_version: u32,
    descriptor_version: u32,
}

impl FormatDescriptor {
    /// Costruttore `const` per i driver del workspace (INV-14).
    ///
    /// E' l'**unico** modo di costruire un descrittore da fuori: la struct e'
    /// `#[non_exhaustive]` e i campi sono privati, quindi il literal non
    /// compila piu'. Non e' una formalita': un campo aggiunto qui diventa un
    /// errore di compilazione in tutti e dieci i driver, che e' esattamente
    /// cio' che serve — un descrittore incompleto e' un descrittore che mente,
    /// e i tre assi di INV-7 esistono perche' era gia' successo.
    ///
    /// Tutti i parametri sono obbligatori, `read_mode` compreso. **Non c'e'
    /// nessun mapping automatico** da `native_read_mode`: i due divergono in
    /// sette driver su dieci, e derivare l'uno dall'altro cancellerebbe proprio
    /// l'informazione per cui lo split e' stato fatto.
    #[must_use]
    #[allow(clippy::too_many_arguments, clippy::fn_params_excessive_bools)]
    pub const fn const_new(
        id: &'static str,
        direction: Direction,
        read_mode: ReadMode,
        native_read_mode: NativeReadMode,
        effective_delivery: DeliverySemantics,
        buffering: BufferingStrategy,
        read_determinism: DeterminismLevel,
        write_mode: Option<WriteMode>,
        write_determinism: Option<DeterminismLevel>,
        multi_layer: bool,
        multi_file: bool,
        reader_concurrency: ReaderConcurrency,
        projection_support: ProjectionSupport,
        predicate_pruning_support: PredicatePruningSupport,
        spatial_pruning_support: SpatialPruningSupport,
        crs_handling: CrsHandling,
        fidelity_class: Fidelity,
        runtime: Runtime,
        hostile_input_hardened: bool,
        spec_version_supported: Option<&'static str>,
        write_capabilities: Option<FormatWriteCapabilities>,
        format_options: plenora_io_model::format_options::SchemaOpzioniFormato,
        semantic_version: u32,
        driver_version: u32,
        descriptor_version: u32,
    ) -> Self {
        Self {
            id,
            direction,
            read_mode,
            native_read_mode,
            effective_delivery,
            buffering,
            read_determinism,
            write_mode,
            write_determinism,
            multi_layer,
            multi_file,
            reader_concurrency,
            projection_support,
            predicate_pruning_support,
            spatial_pruning_support,
            crs_handling,
            fidelity_class,
            runtime,
            hostile_input_hardened,
            spec_version_supported,
            write_capabilities,
            format_options,
            semantic_version,
            driver_version,
            descriptor_version,
        }
    }

    /// Un descrittore uguale a questo, con altre capability di scrittura.
    ///
    /// **Solo per i test del crate.** I campi sono privati per INV-14, e i test
    /// delle capability costruiscono varianti di uno stesso descrittore
    /// cambiando un campo solo. L'alternativa sarebbe riesporre il campo —
    /// cioe' togliere l'invariante per comodita' di un test, che e' il modo in
    /// cui un'invariante smette di valere.
    #[cfg(test)]
    #[must_use]
    pub(crate) const fn con_write_capabilities(
        mut self,
        write_capabilities: Option<FormatWriteCapabilities>,
    ) -> Self {
        self.write_capabilities = write_capabilities;
        self
    }

    #[must_use]
    pub const fn id(&self) -> &'static str {
        self.id
    }

    #[must_use]
    pub const fn direction(&self) -> Direction {
        self.direction
    }

    #[must_use]
    pub const fn read_mode(&self) -> ReadMode {
        self.read_mode
    }

    #[must_use]
    pub const fn native_read_mode(&self) -> NativeReadMode {
        self.native_read_mode
    }

    #[must_use]
    pub const fn effective_delivery(&self) -> DeliverySemantics {
        self.effective_delivery
    }

    #[must_use]
    pub const fn buffering(&self) -> BufferingStrategy {
        self.buffering
    }

    #[must_use]
    pub const fn read_determinism(&self) -> DeterminismLevel {
        self.read_determinism
    }

    #[must_use]
    pub const fn write_mode(&self) -> Option<WriteMode> {
        self.write_mode
    }

    #[must_use]
    pub const fn write_determinism(&self) -> Option<DeterminismLevel> {
        self.write_determinism
    }

    #[must_use]
    pub const fn multi_layer(&self) -> bool {
        self.multi_layer
    }

    #[must_use]
    pub const fn multi_file(&self) -> bool {
        self.multi_file
    }

    #[must_use]
    pub const fn reader_concurrency(&self) -> ReaderConcurrency {
        self.reader_concurrency
    }

    #[must_use]
    pub const fn projection_support(&self) -> ProjectionSupport {
        self.projection_support
    }

    #[must_use]
    pub const fn predicate_pruning_support(&self) -> PredicatePruningSupport {
        self.predicate_pruning_support
    }

    #[must_use]
    pub const fn spatial_pruning_support(&self) -> SpatialPruningSupport {
        self.spatial_pruning_support
    }

    #[must_use]
    pub const fn crs_handling(&self) -> CrsHandling {
        self.crs_handling
    }

    #[must_use]
    pub const fn fidelity_class(&self) -> Fidelity {
        self.fidelity_class
    }

    #[must_use]
    pub const fn runtime(&self) -> Runtime {
        self.runtime
    }

    /// Il parsing degli input non fidati e' bounded durante il parse.
    #[must_use]
    pub const fn hostile_input_hardened(&self) -> bool {
        self.hostile_input_hardened
    }

    /// La versione massima della specifica del formato letta per intero.
    #[must_use]
    pub const fn spec_version_supported(&self) -> Option<&'static str> {
        self.spec_version_supported
    }

    #[must_use]
    pub const fn write_capabilities(&self) -> Option<FormatWriteCapabilities> {
        self.write_capabilities
    }

    #[must_use]
    pub const fn format_options(&self) -> plenora_io_model::format_options::SchemaOpzioniFormato {
        self.format_options
    }

    #[must_use]
    pub const fn semantic_version(&self) -> u32 {
        self.semantic_version
    }

    #[must_use]
    pub const fn driver_version(&self) -> u32 {
        self.driver_version
    }

    #[must_use]
    pub const fn descriptor_version(&self) -> u32 {
        self.descriptor_version
    }
}
