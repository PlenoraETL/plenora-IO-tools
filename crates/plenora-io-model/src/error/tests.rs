//! Le prove di [`super`], in un file loro.
//!
//! Restano un **modulo figlio**: vedono i privati del genitore
//! esattamente come quando stavano dentro di lui, e non allargano di
//! una riga la superficie pubblica del crate.

use super::*;

/// `CuratedPair` e' costruibile in contesto costante.
///
/// Non e' una curiosita': se lo fosse solo a runtime, un chiamante potrebbe
/// costruirla da un valore calcolato, e la garanzia varrebbe per
/// convenzione invece che per tipo.
const COPPIA_COSTANTE: PublicMessage =
    PublicMessage::CuratedPair("dimensioni non supportate:", "xyzm");

#[test]
fn la_coppia_curata_e_costante_e_si_rende_con_uno_spazio() {
    assert_eq!(
        COPPIA_COSTANTE.to_string(),
        "dimensioni non supportate: xyzm"
    );
}

/// Il rendering della coppia resta sotto il tetto globale.
///
/// I due argomenti sono `&'static str` e nessuno li limita a monte: due
/// costanti lunghe potrebbero superare `MAX_MESSAGE_BYTES` insieme, ed e'
/// il costruttore dell'errore a doverle tagliare — non la buona volonta'
/// di chi le scrive.
#[test]
fn la_coppia_curata_passa_dal_tetto_globale() {
    // Statico vero, non `Box::leak`: vedi la nota su `otto_volte!`. La
    // prima stesura di questo test si costruiva lo statico a runtime —
    // funzionava, e diceva la cosa sbagliata.
    let lungo: &'static str = LUNGO_ASCII;
    assert!(lungo.len() > MAX_MESSAGE_BYTES);

    let errore = PlenoraIoError::contratto_redatto(&PublicMessage::CuratedPair(lungo, lungo));
    assert!(
        errore.message.len() <= MAX_MESSAGE_BYTES,
        "messaggio da {} byte oltre il tetto",
        errore.message.len()
    );
}

/// I vocabolari statici sono completi, e la prova non e' il `match`.
///
/// Il `match` esaustivo garantisce che ogni variante abbia un nome; non
/// garantisce che i nomi siano distinti, ne' che qualcuno non abbia
/// scritto due volte lo stesso. Enumerarli qui rende l'una e l'altra cosa
/// dimostrate invece che vere per fortuna.
#[test]
fn i_nomi_strutturali_sono_statici_distinti_e_non_vuoti() {
    use crate::contract::{CoordinateDimensions, GeometryEncoding, SpatialSemantics};

    let encoding = [GeometryEncoding::Wkb, GeometryEncoding::Ewkb];
    let dimensioni = [
        CoordinateDimensions::Xy,
        CoordinateDimensions::Xyz,
        CoordinateDimensions::Xym,
        CoordinateDimensions::Xyzm,
        CoordinateDimensions::Unknown,
    ];
    let semantica = [SpatialSemantics::Geometry, SpatialSemantics::Geography];

    let mut visti = std::collections::BTreeSet::new();
    for nome in encoding
        .iter()
        .map(|v| v.nome())
        .chain(dimensioni.iter().map(|v| v.nome()))
        .chain(semantica.iter().map(|v| v.nome()))
    {
        assert!(
            !nome.is_empty(),
            "un nome strutturale vuoto non identifica niente"
        );
        assert!(visti.insert(nome), "nome strutturale duplicato: {nome}");
    }
    assert_eq!(
        visti.len(),
        encoding.len() + dimensioni.len() + semantica.len()
    );
}

/// L'identificatore di una colonna geometrica rifiuta invece di troncare.
///
/// Un nome tagliato a 256 caratteri e' un nome che non identifica piu'
/// nessuno, e passarlo comunque sarebbe peggio di non passarlo: chi legge
/// l'errore lo userebbe per cercare un campo che non esiste.
#[test]
fn l_identificatore_geometrico_rifiuta_i_nomi_non_attestabili() {
    use crate::contract::{
        CoordinateDimensions, CoordinatePrecision, FieldId, GeometryColumnContract,
        GeometryEncoding, SpatialSemantics, TypesDeclaration,
    };
    use crate::crs::CrsResolution;

    let colonna = |nome: &str| GeometryColumnContract {
        field_id: FieldId(0),
        scansione_completa: false,
        identita_dichiarata: None,
        name: nome.to_owned(),
        crs: CrsResolution::Missing,
        nullable: false,
        encoding: GeometryEncoding::Wkb,
        dimensions: CoordinateDimensions::Xy,
        spatial_semantics: SpatialSemantics::Geometry,
        srid: None,
        precision: CoordinatePrecision::Float64,
        geometry_types: Vec::new(),
        types_declaration: TypesDeclaration::Unresolved,
        native_metadata: BTreeMap::new(),
    };

    assert!(ContractIdentifier::from_geometry_column(&colonna("")).is_none());
    assert!(
        ContractIdentifier::from_geometry_column(&colonna(&"n".repeat(MAX_IDENTIFICATORE + 1)))
            .is_none(),
        "un nome oltre il tetto va rifiutato, non troncato"
    );

    let ammesso = ContractIdentifier::from_geometry_column(&colonna("geometry"))
        .expect("un nome nominabile deve produrre un identificatore");
    assert_eq!(ammesso.to_string(), "geometry");
}

use std::collections::BTreeMap;

use crate::{
    RowDiagnosticExample, RowDiagnosticKey, RowDiagnosticKeyState, RowDiagnosticKeyValue,
    RowDiagnosticScope, RowDiagnosticsCompleteness, ROW_DIAGNOSTICS_CONTRACT,
    ROW_DIAGNOSTICS_INDEX_BASIS,
};

/// L'identificatore nasce dallo schema, e solo da lì.
///
/// I casi negativi contano quanto quello positivo: un indice fuori
/// intervallo o un nome non nominabile producono **nessun identificatore**,
/// non uno inventato o troncato. Un nome troncato somiglia a un nome vero,
/// ed è il modo in cui un errore indica il campo sbagliato.
#[test]
fn l_identificatore_viene_dal_contratto_validato() {
    use arrow_schema::{DataType, Field, Schema};

    let schema = Schema::new(vec![
        Field::new("geometry", DataType::Binary, true),
        Field::new("nome", DataType::Utf8, true),
    ]);

    let primo = ContractIdentifier::from_schema_field(&schema, crate::contract::FieldId(0))
        .expect("il campo 0 esiste");
    assert_eq!(primo.to_string(), "geometry");
    let secondo = ContractIdentifier::from_schema_field(&schema, crate::contract::FieldId(1))
        .expect("il campo 1 esiste");
    assert_eq!(secondo.to_string(), "nome");

    // Indice fuori intervallo: nessun identificatore.
    assert!(ContractIdentifier::from_schema_field(&schema, crate::contract::FieldId(2)).is_none());

    // Nome vuoto: non nominabile.
    let vuoto = Schema::new(vec![Field::new("", DataType::Utf8, true)]);
    assert!(ContractIdentifier::from_schema_field(&vuoto, crate::contract::FieldId(0)).is_none());

    // Nome oltre il tetto: nessun identificatore, non uno troncato.
    let lunghissimo = Schema::new(vec![Field::new(
        "n".repeat(MAX_IDENTIFICATORE + 1),
        DataType::Utf8,
        true,
    )]);
    assert!(
        ContractIdentifier::from_schema_field(&lunghissimo, crate::contract::FieldId(0)).is_none(),
        "meglio non nominare che nominare a meta'"
    );

    // Esattamente al tetto: nominabile.
    let al_limite = Schema::new(vec![Field::new(
        "n".repeat(MAX_IDENTIFICATORE),
        DataType::Utf8,
        true,
    )]);
    assert!(
        ContractIdentifier::from_schema_field(&al_limite, crate::contract::FieldId(0)).is_some()
    );
}

/// Tripwire sull'indipendenza dal wire — **non** la sua prova principale.
///
/// Le garanzie vere sono tre, e nessuna è questo test: i campi sono privati
/// con costruttori controllati, il tipo **non implementa `Serialize`**, e la
/// traduzione verso il wire vive in un DTO separato. Sono proprietà del
/// tipo e del grafo dei moduli, verificate dal compilatore.
///
/// Questo test guarda il `Debug` e cerca i nomi del contratto di
/// destinazione. È un allarme a buon mercato per un caso specifico — un
/// campo rinominato `provider` per comodità del DTO — e vale quanto un
/// allarme: non dimostra l'indipendenza, segnala un modo particolare di
/// perderla. Attribuirgli più forza sarebbe descrivere male ciò che
/// protegge il tipo.
#[test]
fn il_contesto_non_conosce_i_nomi_del_wire() {
    let contesto = ErrorContext::nuovo()
        .con_driver("geoparquet")
        .con_layer(crate::contract::LayerId(0))
        .con_campo(crate::contract::FieldId(2))
        .con_capability(CapabilityReason::TypeNotRepresentable);

    let forma = format!("{contesto:?}");
    for del_wire in ["provider", "details", "row_diagnostics", "message"] {
        assert!(
            !forma.contains(del_wire),
            "il contesto semantico non deve nominare '{del_wire}': {forma}"
        );
    }

    assert_eq!(contesto.driver(), Some("geoparquet"));
    assert_eq!(contesto.layer(), Some(crate::contract::LayerId(0)));
    assert_eq!(contesto.campo(), Some(crate::contract::FieldId(2)));
    assert_eq!(
        contesto.capability_reason(),
        Some(CapabilityReason::TypeNotRepresentable)
    );
    assert!(contesto.identificatore().is_none());

    // Vuoto è vuoto: nessun campo si popola da solo.
    let vuoto = ErrorContext::nuovo();
    assert!(vuoto.driver().is_none());
    assert!(vuoto.layer().is_none());
    assert!(vuoto.campo().is_none());
    assert!(vuoto.capability_reason().is_none());
}

/// Il tetto di 2048 byte vale su **ogni** errore, comunque costruito.
///
/// Non è una raccomandazione che ogni sito deve ricordare: è applicato
/// nell'unico punto da cui passano tutti i costruttori, e il test lo
/// verifica su tutte le forme pubbliche invece che su una.
/// Ripete un letterale otto volte, **a compile time**.
///
/// La prima stesura di questi test otteneva gli statici lunghi con
/// `Box::leak`. Funzionava, e diceva la cosa sbagliata: `&'static str`
/// garantisce la **durata**, non la **provenienza**, e un test che si
/// costruisce lo statico a runtime dimostra proprio cio' che S9 non
/// promette. `concat!` opera su letterali e produce un letterale, quindi
/// qui la provenienza e' letterale per costruzione.
macro_rules! otto_volte {
    ($pezzo:expr) => {
        concat!($pezzo, $pezzo, $pezzo, $pezzo, $pezzo, $pezzo, $pezzo, $pezzo)
    };
}

/// 8^4 = 4096 caratteri, cioe' oltre il doppio di `MAX_MESSAGE_BYTES`.
macro_rules! lungo {
    ($pezzo:literal) => {
        otto_volte!(otto_volte!(otto_volte!(otto_volte!($pezzo))))
    };
}

const LUNGO_ASCII: &str = lungo!("x");
const LUNGO_VIRGOLETTE: &str = lungo!("\"");
const LUNGO_CONTROLLI: &str = lungo!("\u{1}");
const LUNGO_MULTIBYTE: &str = lungo!("à");

#[test]
fn nessun_errore_supera_il_tetto_del_messaggio() {
    let enorme = PublicMessage::Curated(LUNGO_ASCII);
    let costruiti = [
        PlenoraIoError::redatto(
            IoErrorCode::Generic,
            ErrorCategory::Internal,
            ErrorPhase::Validate,
            RemoteEffect::None,
            RetryDisposition::Never,
            &enorme,
        ),
        PlenoraIoError::contratto_redatto(&enorme),
        PlenoraIoError::non_supportato_redatto(&enorme),
        PlenoraIoError::schema_redatto(&enorme),
        PlenoraIoError::crs_redatto(&enorme),
        PlenoraIoError::wkb_redatto(&enorme),
        PlenoraIoError::limite_redatto(&enorme),
        PlenoraIoError::formato_redatto("prova", &enorme),
        PlenoraIoError::capability_redatta(
            "prova",
            None,
            CapabilityReason::TypeNotRepresentable,
            &enorme,
        ),
    ];
    for errore in &costruiti {
        assert!(
            errore.message.len() <= MAX_MESSAGE_BYTES,
            "messaggio da {} byte, tetto {MAX_MESSAGE_BYTES}",
            errore.message.len()
        );
        assert!(
            errore.message.ends_with('…'),
            "un messaggio troncato deve dirlo: {}",
            &errore.message[errore.message.len().saturating_sub(16)..]
        );
    }
}

/// Il tetto vale sul valore **decodificato**, non sul JSON serializzato.
///
/// È una controprova, non una verifica: misura la differenza fra ciò che il
/// tetto garantisce e ciò che finisce sul wire, così nessuno la deduca dal
/// nome della costante.
///
/// L'escaping JSON espande — `"` diventa due byte, un carattere di
/// controllo ne diventa sei — quindi un messaggio al limite si serializza
/// più lungo del limite. Il test lo **dimostra** e lo pinna: se un giorno
/// servisse anche un tetto sul wire, va dichiarato a parte e misurato qui,
/// dopo la serializzazione. Oggi non è promesso.
#[test]
fn il_tetto_e_sul_valore_decodificato_non_sul_json() {
    // Tre input al limite, di espansione crescente.
    let casi = [
        ("ascii", LUNGO_ASCII),
        ("virgolette", LUNGO_VIRGOLETTE),
        ("controlli", LUNGO_CONTROLLI),
    ];
    for (nome, grezzo) in casi {
        let errore = PlenoraIoError::contratto_redatto(&PublicMessage::Curated(grezzo));

        // Cio' che il tetto garantisce: il valore decodificato.
        assert!(
            errore.message.len() <= MAX_MESSAGE_BYTES,
            "{nome}: valore decodificato da {} byte",
            errore.message.len()
        );

        // Cio' che finisce sul wire, misurato dopo la serializzazione.
        let serializzato =
            serde_json::to_string(&errore.message).expect("il messaggio si serializza");
        // `to_string` di una String include le virgolette: le tolgo per
        // misurare il solo valore, che e' cio' di cui si sta parlando.
        let sul_wire = serializzato.len() - 2;

        match nome {
            // L'ASCII non si espande: qui i due numeri coincidono, ed e'
            // il caso che rende invisibile la differenza se lo si guarda
            // da solo.
            "ascii" => assert_eq!(sul_wire, errore.message.len()),
            // Le virgolette raddoppiano.
            "virgolette" => assert!(
                sul_wire > errore.message.len(),
                "le virgolette devono espandersi: {sul_wire} vs {}",
                errore.message.len()
            ),
            // I controlli sestuplicano: e' il caso peggiore, ed e' quello
            // che smentisce la promessa sbagliata.
            "controlli" => assert!(
                sul_wire > MAX_MESSAGE_BYTES * 5,
                "i controlli devono espandersi molto: {sul_wire} byte sul wire \
                     contro {} decodificati",
                errore.message.len()
            ),
            _ => unreachable!(),
        }
    }
}

/// Il troncamento è deterministico e non spezza un carattere.
///
/// Multibyte perché è lì che un taglio a byte fisso romperebbe: 2045 non
/// cade su un confine di `à`, e la ricerca del confine deve tornare
/// indietro invece di panicare.
#[test]
fn il_troncamento_e_deterministico_e_rispetta_i_caratteri() {
    let multibyte = PublicMessage::Curated(LUNGO_MULTIBYTE);
    let primo = PlenoraIoError::contratto_redatto(&multibyte).message;
    let secondo = PlenoraIoError::contratto_redatto(&multibyte).message;
    assert_eq!(primo, secondo, "il troncamento deve essere deterministico");
    assert!(primo.len() <= MAX_MESSAGE_BYTES);
    // Se il taglio avesse spezzato un carattere, la stringa non esisterebbe
    // nemmeno: qui si verifica che il contenuto sia quello atteso.
    assert!(primo.starts_with("àà"));
    assert!(primo.ends_with('…'));

    // Un messaggio sotto il tetto non viene toccato né marcato.
    let corto =
        PlenoraIoError::contratto_redatto(&PublicMessage::Curated("piano non valido")).message;
    assert_eq!(corto, "piano non valido");
}

#[test]
fn error_serializes_complete_bounded_read_diagnostics() {
    let diagnostics = RowDiagnostics {
        contract: ROW_DIAGNOSTICS_CONTRACT.to_owned(),
        scope: RowDiagnosticScope::Read,
        index_basis: ROW_DIAGNOSTICS_INDEX_BASIS.to_owned(),
        completeness: RowDiagnosticsCompleteness::Complete,
        observed_total: 3,
        total: Some(3),
        counts: BTreeMap::from([
            ("shapefile.inner_ring_without_outer".to_owned(), 2),
            ("shapefile.unclosed_ring".to_owned(), 1),
        ]),
        examples_limit: 2,
        examples_truncated: true,
        examples: vec![
            RowDiagnosticExample {
                source_index: 17,
                cause: "shapefile.inner_ring_without_outer".to_owned(),
                column: None,
                key: Some(RowDiagnosticKey {
                    field: "ID_PART".to_owned(),
                    state: RowDiagnosticKeyState::Value,
                    value: Some(RowDiagnosticKeyValue::String("9007199254741009".to_owned())),
                }),
                write_state: None,
            },
            RowDiagnosticExample {
                source_index: 89,
                cause: "shapefile.unclosed_ring".to_owned(),
                column: None,
                key: None,
                write_state: None,
            },
        ],
        knowledge_limits: None,
        input_total: None,
        diagnostic_state_counts: None,
        write_outcome: None,
    };
    let error = PlenoraIoError::formato_redatto(
        "shp",
        &PublicMessage::Curated("3 righe Shapefile non valide"),
    )
    .with_row_diagnostics(diagnostics);

    let value = serde_json::to_value(error).unwrap();
    assert_eq!(
        value["row_diagnostics"]["contract"],
        ROW_DIAGNOSTICS_CONTRACT
    );
    assert_eq!(value["row_diagnostics"]["observed_total"], 3);
    assert_eq!(
        value["row_diagnostics"]["examples"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        value["row_diagnostics"]["examples"][0]["key"]["value"],
        "9007199254741009"
    );
    assert!(value["row_diagnostics"]["input_total"].is_null());
}

#[test]
fn unresolved_authority_error_does_not_require_or_expose_a_definition() {
    let raw = RawCrs::from_authority_hint("EPSG:99999".to_owned());
    let error = PlenoraIoError::crs_non_risolto_redatto("ipc", &raw);

    // Il costruttore redatto rende i due conteggi con le parole del
    // messaggio curato: la proprieta' verificata resta «escono i byte,
    // non il valore».
    assert_eq!(
        error.message,
        "CRS dichiarato ma non risolto: authority_hint di 10 byte, definizione di 0",
    );
    assert!(!error.message.contains("EPSG:99999"));
}

#[test]
fn timeout_after_commit_keeps_cause_effect_and_recovery_separate() {
    let error = PlenoraIoError::redatto(
        IoErrorCode::Generic,
        ErrorCategory::Timeout,
        ErrorPhase::Commit,
        RemoteEffect::Unknown,
        RetryDisposition::RequiresRecovery,
        &PublicMessage::Curated("esito commit non verificabile"),
    );
    assert_eq!(error.category, ErrorCategory::Timeout);
    assert_eq!(error.remote_effect, RemoteEffect::Unknown);
    assert_eq!(error.retry, RetryDisposition::RequiresRecovery);
    assert!(error.is_retryable());
}

#[test]
fn filesystem_messages_do_not_expose_paths() {
    let source = std::io::Error::new(
        std::io::ErrorKind::PermissionDenied,
        "C:\\secret\\customer.geojson",
    );
    let error = PlenoraIoError::from(source);
    assert!(!error.to_string().contains("customer.geojson"));
    assert_eq!(error.category, ErrorCategory::Io);
}

#[test]
fn four_axis_error_roundtrips_and_rejects_unknown_fields() {
    let error = PlenoraIoError::redatto(
        IoErrorCode::Generic,
        ErrorCategory::Timeout,
        ErrorPhase::Commit,
        RemoteEffect::Unknown,
        RetryDisposition::RequiresRecovery,
        &PublicMessage::Curated("esito commit non verificabile"),
    );
    let value = serde_json::to_value(&error).unwrap();
    let decoded: PlenoraIoError = serde_json::from_value(value.clone()).unwrap();
    assert_eq!(decoded, error);

    let mut object = value.as_object().unwrap().clone();
    object.insert("future_axis".to_owned(), serde_json::Value::Null);
    assert!(serde_json::from_value::<PlenoraIoError>(serde_json::Value::Object(object)).is_err());
}

#[test]
fn retry_disposition_uses_the_shared_tagged_object_shape() {
    let cases = [
        (
            RetryDisposition::Never,
            serde_json::json!({"kind": "never"}),
        ),
        (RetryDisposition::Safe, serde_json::json!({"kind": "safe"})),
        (
            RetryDisposition::RequiresIdempotencyKey,
            serde_json::json!({"kind": "requires_idempotency_key"}),
        ),
        (
            RetryDisposition::RequiresRecovery,
            serde_json::json!({"kind": "requires_recovery"}),
        ),
        (
            RetryDisposition::After(2_750),
            serde_json::json!({"kind": "after", "delay_ms": 2_750}),
        ),
    ];

    for (retry, expected) in cases {
        assert_eq!(serde_json::to_value(retry).unwrap(), expected);
    }
}
