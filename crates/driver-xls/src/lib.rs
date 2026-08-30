//! driver-xls — XLSX ↔ `RecordBatch`. Foglio tabellare: la
//! geometria è dichiarata via `format_options` (`x_column`+`y_column` XY o
//! `wkt_column` XY/XYZ/XYM/XYZM), il CRS via `assume_crs` (`PRODUCT.md § CRS`). Foglio scelto con
//! `format_options["sheet"]` o il primo. Multi-foglio: incremento futuro.
#![forbid(unsafe_code)]

use std::collections::{BTreeMap, BTreeSet};
use std::io::{BufReader, BufWriter, Read, Seek, Write as _};
use std::path::PathBuf;
use std::sync::Arc;

use arrow_array::builder::BinaryBuilder;
use arrow_array::{Array, ArrayRef, BinaryArray, RecordBatch, RecordBatchOptions};
use arrow_schema::{Field, Schema, SchemaRef};
use calamine::{open_workbook, Data, Reader, Xlsx, XlsxCellReader};
use rust_xlsxwriter::Workbook;
use serde_json::Value as JsonValue;

use driver_common::wkt_lossless::{format_wkt, parse_wkt_bounded};
use driver_common::{
    classify_i64, geometry_field, geometry_index, json_from_array, ColType, InferredColumnBuilder,
    ObservedValueClass, TypeAccumulator,
};
use plenora_io_core::descriptor::{
    CrsHandling, Direction, Fidelity, FormatDescriptor, ReadMode, ReaderConcurrency, Runtime,
    WriteMode,
};
use plenora_io_core::driver::{
    spawn_batch_reader, BatchEmitter, FormatDriver, FormatWriter, LayerReader, OpenDatasetHandle,
    Published, ReadOptions, Sink, Source, WriteOptions,
};
use plenora_io_core::loss::LossReport;
use plenora_io_core::publish::{create_staged_file, publish_file_atomic_limited};
use plenora_io_core::request::ReadRequest;
use plenora_io_core::{
    check_cancelled, check_cancelled_periodically, validate_write, with_write_validation,
    AttributeWriteSupport, CrsRepresentationCapabilities, CrsRepresentationState, CrsWriteSupport,
    FormatWriteCapabilities, NullabilitySupport, SingleReaderGate, TypeCoercionPolicy, WritePlan,
    SCALAR_TYPES, UTF8_FIELD_NAMES, WKB_PASSTHROUGH_GEOMETRY,
};
use plenora_io_model::budget::{OperationBudget, SpillLease};
use plenora_io_model::contract::{
    CoordinateDimensions, DataContract, FieldId, GeometryColumnContract, GeometryType,
    LayerContract, LayerId,
};
use plenora_io_model::crs::{CrsKind, ResolvedCrs};
use plenora_io_model::geometry::with_geometry_contract_metadata;
use plenora_io_model::limits::WkbLimits;
use plenora_io_model::wkb::{decode_wkb, WkbCoordinate, WkbFlavor, WkbGeometry, WkbValue};
use plenora_io_model::{
    CancellationToken, ErrorPhase, NumeroStrutturale, PlenoraIoError, PublicMessage, Result,
};

#[cfg(test)]
use plenora_io_model::wkb::encode_wkb;

const GEOMETRY: &str = "geometry";

fn err(reason: &PublicMessage) -> PlenoraIoError {
    PlenoraIoError::formato_redatto("xls", reason)
}

/// Esegue una chiamata a `calamine` convertendo un suo panico in errore
/// tipizzato (XLSX-HARDENING).
///
/// `calamine` converte il riferimento testuale di una cella (`A1`) in
/// coordinate accumulando senza controlli: `col = col * 26 + …` e
/// `row = row * 10 + …` su `u32` (0.36.1, `src/xlsx/mod.rs:2837-2853`). Un
/// riferimento con abbastanza lettere trabocca — sette bastano — e il
/// workspace tiene `overflow-checks = true` **anche in release**, per scelta
/// dichiarata in `Cargo.toml`: l'overflow e' quindi un panico anche nel
/// binario spedito, non solo sotto il profilo di fuzzing. Senza quella riga
/// sarebbe peggio, non meglio: la moltiplicazione avvolgerebbe in silenzio e
/// il foglio verrebbe letto a coordinate sbagliate.
///
/// Il driver legge file esterni non fidati per mestiere e promette una busta
/// d'errore a quattro assi: la barriera ripristina il contratto, non lo aggira.
/// Un aggiornamento di `calamine` che renda fallibile quella conversione
/// **non** la sostituisce — chiude questo difetto, non la classe.
///
/// # Perimetro
///
/// Avvolge le sole chiamate che toccano l'input non fidato — apertura del
/// workbook, nomi dei fogli, creazione del lettore di celle, dimensioni,
/// `next_cell` e l'estrazione di posizione e valore — e non la logica del
/// driver che ci sta attorno. Avvolgerla tutta trasformerebbe in "panico di
/// calamine" anche un difetto nostro, che invece deve restare visibile.
///
/// # Correttezza dell'unwind safety
///
/// `AssertUnwindSafe` dichiara che lo stato attraversato dal panico non viene
/// piu' osservato, e qui e' vero per costruzione, non per promessa: il
/// chiamante riceve `Err`, e ogni struttura `calamine` toccata dal panico
/// viene scartata prima che l'errore risalga — il lettore di celle da
/// [`LettoreCelleSorvegliato`], che si invalida da solo, e il workbook da
/// `open`, che lo lascia cadere prima di propagare. Nessuno stato parziale
/// resta raggiungibile, quindi non c'e' invariante rotta da osservare.
///
/// # Nota per chi legge un fuzz target rosso
///
/// `xlsx_reader` resta rosso **anche a barriera funzionante**: `libfuzzer-sys`
/// installa un hook che chiama `std::process::abort()` prima che l'unwinding
/// cominci (0.4.10, `src/lib.rs:92-95`), apposta perche' un `catch_unwind` nel
/// codice sotto test non possa nascondere difetti al fuzzer. La copertura di
/// questa barriera e' il test del driver sul seme versionato, non il fuzzing.
///
/// # Errors
///
/// Propaga l'errore dell'operazione, oppure `PlenoraIoError::format` — fase
/// `Read` — con un messaggio **statico**: mai il testo del panico, il percorso
/// del file o un valore di cella.
fn leggendo_calamine<T>(operazione: impl FnOnce() -> Result<T>) -> Result<T> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(operazione))
        .unwrap_or_else(|_| Err(err(&PublicMessage::Curated(MESSAGGIO_PANICO_CALAMINE))))
}

/// Messaggio pubblico del panico di `calamine`, statico e curato.
///
/// Non porta impronta del panico. Un'impronta derivata dal messaggio e' un
/// valore che nasce dall'input e finisce in un errore serializzato e
/// registrato: per un componente che promette di non far uscire nulla che
/// derivi dal payload e' una promessa in meno, in cambio di una correlazione
/// che i log del processo — dove l'hook di panico scrive comunque il testo
/// completo — gia' permettono.
const MESSAGGIO_PANICO_CALAMINE: &str =
    "la libreria XLSX e' andata in panico su un input non conforme";

const LETTORE_INVALIDATO: &str = "lettore XLSX invalidato da un errore precedente";

/// Lettore di celle `calamine` che non sopravvive a un proprio fallimento.
///
/// Il contratto «dopo un panico il lettore viene scartato» qui non e' una
/// convenzione da rispettare a ogni chiamata: e' il tipo a imporlo. Al primo
/// fallimento il lettore viene lasciato cadere e ogni chiamata successiva
/// trova `None`, quindi non esiste un modo di continuare a leggere celle da
/// uno stato che il panico ha attraversato — nemmeno per distrazione, in un
/// ciclo che oggi non c'e' e domani potrebbe esserci.
///
/// Vale per qualunque fallimento, non solo per i panici: dopo un errore di
/// `calamine` il flusso XML e' comunque a meta', e proseguire darebbe celle
/// non attribuibili a una posizione. Nessun percorso del driver ci prova —
/// tutti propagano — ma qui il "nessuno ci prova" e' verificato dal
/// compilatore invece che riletto.
struct LettoreCelleSorvegliato<'a, RS: Read + Seek> {
    lettore: Option<XlsxCellReader<'a, RS>>,
}

impl<'a, RS: Read + Seek> LettoreCelleSorvegliato<'a, RS> {
    /// Apre il lettore di celle del foglio, sorvegliando `calamine`.
    fn nuovo(workbook: &'a mut Xlsx<RS>, foglio: &str) -> Result<Self> {
        let lettore = leggendo_calamine(|| {
            workbook
                .worksheet_cells_reader(foglio)
                .map_err(|_| err(&PublicMessage::Curated("foglio XLSX non leggibile")))
        })?;
        Ok(Self {
            lettore: Some(lettore),
        })
    }

    /// Dimensioni dichiarate dal foglio.
    fn dimensioni(&mut self) -> Result<SheetBounds> {
        let Some(lettore) = self.lettore.as_mut() else {
            return Err(err(&PublicMessage::Curated(LETTORE_INVALIDATO)));
        };
        let esito = leggendo_calamine(|| {
            let dimensioni = lettore.dimensions();
            Ok(SheetBounds {
                start: dimensioni.start,
                end: dimensioni.end,
            })
        });
        if esito.is_err() {
            self.lettore = None;
        }
        esito
    }

    /// La cella successiva come dato **nostro**: posizione e valore lasciano
    /// la barriera gia' copiati, cosi' nessun tipo di `calamine` sopravvive
    /// alla chiamata e non c'e' un accessore che possa panicare piu' tardi,
    /// fuori dal `catch_unwind`.
    fn prossima_cella(&mut self) -> Result<Option<(u32, u32, Data)>> {
        let Some(lettore) = self.lettore.as_mut() else {
            return Err(err(&PublicMessage::Curated(LETTORE_INVALIDATO)));
        };
        let esito = leggendo_calamine(|| {
            let cella = lettore
                .next_cell()
                .map_err(|_| err(&PublicMessage::Curated("lettura delle celle XLSX fallita")))?;
            Ok(cella.map(|cella| {
                let (riga, colonna) = cella.get_position();
                let valore: Data = cella.get_value().clone().into();
                (riga, colonna, valore)
            }))
        });
        if esito.is_err() {
            self.lettore = None;
        }
        esito
    }
}

use plenora_io_model::format_options::{
    FaseOpzione, OpzioneFormato, SchemaOpzioniFormato, ValoreAmmesso,
};

/// Le `format_options` interpretate dal driver XLSX (L0.7, S6).
const SCHEMA_OPZIONI: SchemaOpzioniFormato = SchemaOpzioniFormato::nuovo(&[
    OpzioneFormato {
        chiave: "geometry_encoding",
        fase: FaseOpzione::Scrittura,
        valore: ValoreAmmesso::Enumerato(&["wkt", "xy"]),
        predefinito: Some("wkt"),
        descrizione: "come scrivere la geometria: colonna WKT o colonne x/y",
    },
    OpzioneFormato {
        chiave: "sheet",
        fase: FaseOpzione::Lettura,
        valore: ValoreAmmesso::Testo,
        predefinito: None,
        descrizione: "nome del foglio da leggere; in assenza, il primo",
    },
    OpzioneFormato {
        chiave: "wkt_column",
        fase: FaseOpzione::Lettura,
        valore: ValoreAmmesso::Testo,
        predefinito: None,
        descrizione: "colonna che contiene la geometria in WKT",
    },
    OpzioneFormato {
        chiave: "x_column",
        fase: FaseOpzione::Lettura,
        valore: ValoreAmmesso::Testo,
        predefinito: None,
        descrizione: "colonna dell'ascissa, da usare insieme a y_column",
    },
    OpzioneFormato {
        chiave: "y_column",
        fase: FaseOpzione::Lettura,
        valore: ValoreAmmesso::Testo,
        predefinito: None,
        descrizione: "colonna dell'ordinata, da usare insieme a x_column",
    },
]);

static DESCRIPTOR: FormatDescriptor = FormatDescriptor::const_new(
    "xls",
    Direction::Bidirectional,
    ReadMode::StreamingSequential,
    // INV-7: `infer_layout` restituisce lo spool completo prima che `open` costruisca il dataset.
    plenora_io_core::NativeReadMode::Materialized,
    // Il drenaggio e lo spool sono dell'adapter comune, non di
    // questo driver: `BudgetedReader` li impone a tutti.
    plenora_io_core::DeliverySemantics::OperationAtomic,
    plenora_io_core::BufferingStrategy::AdaptiveMemoryThenDisk,
    plenora_io_core::DeterminismLevel::Semantic,
    Some(WriteMode::Buffered),
    Some(plenora_io_core::DeterminismLevel::Semantic),
    false, // primo foglio nella v1; multi-foglio futuro
    false,
    ReaderConcurrency::SingleActiveReader,
    plenora_io_core::ProjectionSupport::None,
    plenora_io_core::PredicatePruningSupport::None,
    plenora_io_core::SpatialPruningSupport::None,
    CrsHandling::None,
    Fidelity::Conditional,
    Runtime::PureRust,
    // `hostile_input_hardened`: come CSV: le celle WKT passano dall'analisi progressiva (S12).
    true,
    // `spec_version_supported`: il formato non si versiona in un modo che
    // il driver possa dichiarare per intero.
    None,
    Some(FormatWriteCapabilities {
        field_names: UTF8_FIELD_NAMES,
        allowed_types: SCALAR_TYPES,
        type_coercion: TypeCoercionPolicy::ExplicitText,
        attributes: AttributeWriteSupport::All,
        geometry: WKB_PASSTHROUGH_GEOMETRY,
        crs: CrsWriteSupport::None,
        crs_representations: CrsRepresentationCapabilities::new(
            CrsRepresentationState::Absent,
            CrsRepresentationState::Absent,
            CrsRepresentationState::Absent,
        ),
        nullability: NullabilitySupport::FormatDefined,
        multi_layer: false,
    }),
    SCHEMA_OPZIONI,
    1,
    5,
    9,
);

pub struct XlsDriver;

impl FormatDriver for XlsDriver {
    fn descriptor(&self) -> &FormatDescriptor {
        &DESCRIPTOR
    }

    fn open(&self, source: Source, mut opts: ReadOptions) -> Result<Box<dyn OpenDatasetHandle>> {
        let path = plenora_io_core::preflight_source(self.descriptor(), source, &mut opts)?;
        if !path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("xlsx"))
        {
            return Err(PlenoraIoError::non_supportato_redatto(
                &PublicMessage::Curated(
                    "il driver supporta in lettura soltanto .xlsx; .xls non e instradato",
                ),
            ));
        }
        validate_archive_ratio(&path, opts.budget())?;
        // FZ-0: il panico di calamine sui riferimenti di cella e' impedito
        // qui, prima che la libreria veda il foglio. La barriera resta sotto,
        // ma un panico catturato e' pur sempre un panico avvenuto.
        valida_riferimenti_cella(&path, opts.budget())?;
        let mut wb: Xlsx<_> = leggendo_calamine(|| {
            open_workbook(&path).map_err(|_| err(&PublicMessage::Curated("apertura XLSX fallita")))
        })?;
        check_cancelled(opts.cancellation(), ErrorPhase::Read)?;
        let sheet = match opts.format_options.get("sheet").cloned() {
            Some(dichiarato) => dichiarato,
            None => leggendo_calamine(|| Ok(wb.sheet_names().first().cloned()))?
                .ok_or_else(|| err(&PublicMessage::Curated("nessun foglio nel workbook")))?,
        };
        let crs = opts.assume_crs.clone().ok_or_else(|| {
            PlenoraIoError::crs_redatto(&PublicMessage::Curated(
                "XLSX con geometria richiede --assume-crs",
            ))
        })?;
        // Il workbook viene lasciato cadere **prima** di propagare l'esito, non
        // dopo: se `infer_layout` e' rientrato per un panico di calamine, lo
        // stato attraversato dal panico smette di esistere qui, e non c'e' un
        // ramo d'errore che possa ancora toccarlo. E' la meta' che riguarda il
        // workbook della promessa di `leggendo_calamine`; l'altra meta', il
        // lettore di celle, se la impone da solo.
        let inferenza = infer_layout(
            &mut wb,
            &sheet,
            &opts.format_options,
            &crs,
            opts.cancellation(),
            XlsxQuote::from_read_options(&opts),
            opts.budget(),
        );
        drop(wb);
        let (layout, contract, spool) = inferenza?;
        Ok(plenora_io_core::with_read_budget(
            Box::new(XlsDataset {
                layers: vec![LayerContract {
                    id: LayerId(0),
                    name: sheet.clone(),
                    contract,
                }],
                layout,
                spool,
                reader_gate: SingleReaderGate::new(DESCRIPTOR.id()),
            }),
            &opts,
            true,
        ))
    }

    fn create(
        &self,
        sink: Sink,
        plan: &WritePlan,
        opts: &WriteOptions,
    ) -> Result<Box<dyn FormatWriter>> {
        validate_write(
            self.descriptor(),
            plan,
            opts.max_columns(),
            &opts.format_options,
        )?;
        let Sink::Path(path) = sink;
        if path.exists() {
            return Err(PlenoraIoError::destinazione_esistente());
        }
        if !path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("xlsx"))
        {
            return Err(PlenoraIoError::non_supportato_redatto(
                &PublicMessage::Curated("l'output deve avere estensione .xlsx"),
            ));
        }
        if plan.layers.len() != 1 {
            return Err(PlenoraIoError::non_supportato_redatto(
                &PublicMessage::Curated("XLSX: un solo foglio per file nella v1"),
            ));
        }
        // Prima: `matches!(..., Some("xy"))`, cioe' qualunque valore diverso
        // da "xy" — compreso "XY", "WKB" o un refuso — significava WKT senza
        // dirlo. Ora le due grafie ammesse sono trattate come due casi, e
        // l'assenza vale il default dichiarato nello schema.
        let xy = match opts
            .format_options
            .get("geometry_encoding")
            .map(String::as_str)
        {
            Some("xy") => true,
            None | Some("wkt") => false,
            // Il valore non esce: lo schema dichiara `geometry_encoding`
            // come `Enumerato`, quindi un valore diverso e' gia' stato respinto
            // da `valida_opzioni` con il suo token. Questo ramo e' difensivo.
            Some(_) => {
                return Err(PlenoraIoError::redatto(
                    plenora_io_model::IoErrorCode::Generic,
                    plenora_io_model::ErrorCategory::InvalidConfiguration,
                    plenora_io_model::ErrorPhase::Validate,
                    plenora_io_model::RemoteEffect::None,
                    plenora_io_model::RetryDisposition::Never,
                    &PublicMessage::Curated("xls: geometry_encoding non riconosciuto"),
                ))
            }
        };
        with_write_validation(
            Box::new(XlsWriterState {
                path,
                durable: opts.durable,
                xy,
                batches: Vec::new(),
                wkb_limits: opts.wkb_limits(),
                max_output_bytes: opts.max_output_bytes(),
            }),
            self.descriptor(),
            plan,
            opts,
        )
    }
}

fn validate_archive_ratio(path: &PathBuf, budget: &OperationBudget) -> Result<()> {
    let maximum_ratio = budget.context().limits().decompression_ratio();
    let file = std::fs::File::open(path)?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|_| err(&PublicMessage::Curated("contenitore XLSX non valido")))?;
    let mut compressed = 0_u64;
    let mut expanded = 0_u64;
    for index in 0..archive.len() {
        budget.context().ensure_active()?;
        let entry = archive
            .by_index(index)
            .map_err(|_| err(&PublicMessage::Curated("voce XLSX non valida")))?;
        compressed = compressed
            .checked_add(entry.compressed_size())
            .ok_or_else(|| {
                PlenoraIoError::limite_redatto(&PublicMessage::Curated(
                    "overflow nel conteggio dei byte compressi XLSX",
                ))
            })?;
        expanded = expanded.checked_add(entry.size()).ok_or_else(|| {
            PlenoraIoError::limite_redatto(&PublicMessage::Curated(
                "overflow nel conteggio dei byte decompressi XLSX",
            ))
        })?;
    }
    let allowed = compressed.checked_mul(maximum_ratio).ok_or_else(|| {
        PlenoraIoError::limite_redatto(&PublicMessage::Curated(
            "overflow nel calcolo del rapporto di decompressione XLSX",
        ))
    })?;
    if expanded > 0 && (compressed == 0 || expanded > allowed) {
        return Err(PlenoraIoError::limite_redatto(
            &PublicMessage::CuratedBetween(
                "XLSX:",
                NumeroStrutturale::Conteggio(expanded),
                "byte decompressi superano il rapporto massimo, moltiplicatore",
                NumeroStrutturale::Limite(maximum_ratio),
            ),
        ));
    }
    Ok(())
}

/// Tetto sulla **lunghezza** di un token, ricavato dai massimi del formato
/// XLSX e non da noi: ECMA-376 fissa l'ultima colonna a `XFD` e l'ultima riga a
/// 1.048.576, cioe' tre lettere e sette cifre. Un token piu' lungo non puo'
/// essere un riferimento di cella conforme, ma questa sola osservazione non
/// rende il controllo sottostante un validatore completo del formato.
///
/// E' un tetto sulla lunghezza, **non** sul valore: `XFE1` ha tre lettere e
/// passa pur essendo oltre l'ultima colonna. Per l'overflow che questo controllo
/// esiste per impedire la lunghezza basta e avanza, e pretendere il valore
/// esatto vorrebbe il contesto dell'elemento -- vedi `valida_valore_riferimento`.
const MAX_LETTERE_RIFERIMENTO: usize = 3;
const MAX_CIFRE_RIFERIMENTO: usize = 7;

/// Tetto sui byte di XML ispezionati per singola parte. La prevalidazione deve
/// essere bounded quanto la lettura che protegge: un `.xlsx` ostile non deve
/// poter spendere memoria o tempo illimitati *nel controllo*.
const MAX_BYTE_PARTE_XML: u64 = 64 * 1024 * 1024;

/// Numero massimo di **membri dell'archivio**, non delle sole parti XML.
///
/// Il controllo guarda `archive.len()`, cioe' ogni voce del central directory:
/// un contenitore con migliaia di immagini viene fermato quanto uno con
/// migliaia di fogli. E' voluto -- il tetto difende dall'abuso del contenitore,
/// non dal numero di fogli -- e la costante si chiamava `MAX_PARTI_XML`, che
/// suggeriva l'altra cosa. Un workbook conforme ha una parte per foglio piu' il
/// manifesto e qualche risorsa; migliaia sono un abuso.
///
/// Il conteggio avviene **dopo** `ZipArchive::new`: limita cio' che si
/// ispeziona, non la memoria gia' spesa per caricare il central directory.
const MAX_MEMBRI_ARCHIVIO: usize = 4096;

/// Impedisce il panico di `calamine` **prima** che avvenga (FZ-0).
///
/// `calamine` 0.36.1 converte il riferimento testuale di una cella in
/// coordinate accumulando in `u32` senza controlli
/// (`src/xlsx/mod.rs:2837-2853`): `col = col * 26 + …` e `row = row * 10 + …`.
/// Sette lettere bastano a superare `u32::MAX`. Con `overflow-checks = true`,
/// che il workspace tiene anche in release, e' un panico; senza, sarebbe un
/// avvolgimento silenzioso e coordinate false.
///
/// La barriera `leggendo_calamine` resta come difesa in profondita', ma non
/// chiude il finding: un panico catturato e' pur sempre un panico avvenuto, e
/// sotto `libfuzzer-sys` diventa `abort()` prima dell'unwinding. Qui il panico
/// non avviene.
///
/// # Perche' non e' una reimplementazione di `calamine`
///
/// Il controllo non indovina la soglia alla quale la libreria trabocca: applica
/// le lunghezze massime che **il formato stesso** dichiara. `XFD` e 1.048.576
/// sono nell'ECMA-376, non nel codice di `calamine`, quindi il criterio non
/// cambia quando cambia la libreria. Un riferimento conforme non viene
/// rifiutato per questi due tetti di lunghezza; non e' una promessa sulla
/// conformita' completa di ogni attributo chiamato `r` o `ref`.
///
/// # Fail-closed
///
/// Un contenitore che non si apre, una parte che non si decomprime, un CRC che
/// non torna, un XML malformato o piu' parti/byte dei tetti sopra fermano la
/// lettura con un errore tipizzato. Non c'e' un ramo che, non riuscendo a
/// controllare, prosegua lo stesso.
fn valida_riferimenti_cella(path: &PathBuf, budget: &OperationBudget) -> Result<()> {
    let file = std::fs::File::open(path)?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|_| err(&PublicMessage::Curated("contenitore XLSX non valido")))?;

    if archive.len() > MAX_MEMBRI_ARCHIVIO {
        return Err(PlenoraIoError::limite_redatto(
            &PublicMessage::CuratedBetween(
                "XLSX:",
                NumeroStrutturale::Conteggio(driver_common::saturating_u64(archive.len())),
                "parti nel contenitore superano il tetto di",
                NumeroStrutturale::Limite(driver_common::saturating_u64(MAX_MEMBRI_ARCHIVIO)),
            ),
        ));
    }

    // Perimetro del pre-filtro: fogli e workbook. Lo scanner sotto ispeziona
    // soltanto attributi non qualificati chiamati `r` o `ref`; non legge nodi
    // di testo e quindi non pretende di validare, per esempio, il contenuto
    // dei nomi definiti in `xl/workbook.xml`.
    //
    // **La selezione non apre niente.** `name_for_index` legge il nome dal
    // central directory, che e' gia' in memoria; l'apertura avviene dopo, solo
    // per le voci scelte, e il suo errore si propaga.
    //
    // Fino a questa revisione la selezione passava da
    // `archive.by_index(indice).ok()?` dentro un `filter_map`, e quel `?`
    // trasformava **qualunque** errore di apertura in «parte ignorata»: una
    // parte con central directory leggibile e header locale irraggiungibile
    // spariva dal perimetro, e la funzione restituiva `Ok`. Era fail-open in
    // una funzione il cui contratto dice che «non c'e' un ramo che, non
    // riuscendo a controllare, prosegua lo stesso», e il `by_name` che seguiva
    // non lo riparava, perche' cercava fra i nomi gia' filtrati.
    let indici_nel_perimetro: Vec<usize> = (0..archive.len())
        .filter(|indice| {
            archive.name_for_index(*indice).is_some_and(|nome| {
                let minuscolo = nome.to_ascii_lowercase();
                let e_foglio = minuscolo.starts_with("xl/worksheets/")
                    && std::path::Path::new(&minuscolo)
                        .extension()
                        .is_some_and(|estensione| estensione.eq_ignore_ascii_case("xml"));
                e_foglio || minuscolo == "xl/workbook.xml"
            })
        })
        .collect();

    for indice in indici_nel_perimetro {
        budget.context().ensure_active()?;
        let membro = archive
            .by_index(indice)
            .map_err(|_| err(&PublicMessage::Curated("parte XLSX non leggibile")))?;
        if membro.size() > MAX_BYTE_PARTE_XML {
            return Err(PlenoraIoError::limite_redatto(&PublicMessage::CuratedWith(
                "XLSX: una parte XML supera il tetto, byte",
                NumeroStrutturale::Limite(MAX_BYTE_PARTE_XML),
            )));
        }
        ispeziona_parte_xml(BufReader::new(membro), budget)?;
    }
    Ok(())
}

/// Scorre una parte XML e verifica ogni attributo che porta un riferimento.
fn ispeziona_parte_xml<R: std::io::BufRead>(sorgente: R, budget: &OperationBudget) -> Result<()> {
    let mut lettore = quick_xml::Reader::from_reader(sorgente);
    let mut buffer = Vec::new();
    let mut eventi = 0usize;
    loop {
        buffer.clear();
        // Un CRC che non torna o un flusso troncato arrivano qui come errore di
        // lettura, e fermano la lettura invece di proseguire su dati parziali.
        let prossimo = lettore
            .read_event_into(&mut buffer)
            .map_err(|_| err(&PublicMessage::Curated("XML XLSX non valido")))?;
        let elemento = match prossimo {
            quick_xml::events::Event::Start(elemento)
            | quick_xml::events::Event::Empty(elemento) => elemento,
            quick_xml::events::Event::Eof => return Ok(()),
            _ => continue,
        };
        eventi = eventi.saturating_add(1);
        check_cancelled_periodically(budget.context().cancellation(), ErrorPhase::Read, eventi)?;

        for attributo in elemento.attributes().with_checks(true) {
            let attributo =
                attributo.map_err(|_| err(&PublicMessage::Curated("attributo XLSX non valido")))?;
            // Ogni attributo **non qualificato** chiamato `r` o `ref` nelle
            // parti ispezionate entra nello stesso pre-filtro lessicale. Il
            // nome dell'elemento non viene portato a valle, quindi qui non si
            // assegna all'attributo la grammatica di `row`, `c`, `dimension` o
            // di un altro elemento. Un `r:id` di relazione ha il prefisso e
            // non entra.
            if !matches!(attributo.key.as_ref(), b"r" | b"ref") {
                continue;
            }
            valida_valore_riferimento(attributo.value.as_ref())?;
        }
    }
}

/// **Pre-filtro contro l'overflow di `calamine`, non un validatore di
/// formato.**
///
/// La distinzione non e' pedanteria: decide che cosa questa funzione promette e
/// che cosa no, e la prima stesura della sua sonda l'ha sbagliata in entrambi i
/// versi -- prima pretendendo il rifiuto di `AA`, poi chiamandolo «riferimento
/// valido». Non e' ne' l'uno ne' l'altro: e' un token che **non puo' far
/// traboccare** l'accumulatore, e tanto basta a lasciarlo passare.
///
/// # Che cosa garantisce
///
/// Che nessun token oltre le lunghezze massime di un riferimento conforme
/// arrivi al parser. Il tetto e' preso dal formato -- `XFD` e 1.048.576, cioe'
/// tre lettere e sette cifre -- e non dalle lunghezze alle quali l'overflow di
/// `u32` diventa possibile in `calamine`, sette lettere o dieci cifre. I tetti
/// sono quindi sufficienti a impedire il finding noto e non rifiutano un
/// riferimento conforme **per la sola lunghezza**.
///
/// # Che cosa **non** garantisce, deliberatamente
///
/// Che il valore sia un riferimento conforme. Restano ammessi:
///
/// * i token di sole lettere o di sole cifre, che presi da soli non sono uno
///   `ST_CellRef` -- un riferimento di cella vuole colonna **e** riga;
/// * `XFE1` e `A1048577`, che stanno nei conteggi (tre lettere, sette cifre) ma
///   fuori dagli estremi reali del formato;
/// * la differenza fra gli elementi che portano il riferimento. `row@r` e' un
///   indice di riga, `c@r` un riferimento singolo, `dimension@ref` un singolo o
///   un intervallo, e questa funzione li riceve **tutti dallo stesso punto**,
///   senza il nome dell'elemento: non puo' quindi pretendere la grammatica
///   giusta per ciascuno.
///
/// Rendere il controllo conforme richiede di portare qui il contesto
/// dell'elemento e di verificare gli estremi numerici, non le lunghezze. Un
/// valore che oggi il pre-filtro inoltra potrebbe allora essere fermato prima
/// del parser: e' una modifica del confine del pre-filtro e va decisa, non
/// fatta passare per una correzione di questa sonda. Da cio' non segue che
/// l'intero driver accetti oggi quel valore: `calamine` puo' ancora rifiutarlo.
fn valida_valore_riferimento(valore: &[u8]) -> Result<()> {
    for token in valore
        .split(|byte| matches!(byte, b':' | b' ' | b',' | b'$'))
        .filter(|token| !token.is_empty())
    {
        let lettere = token
            .iter()
            .take_while(|byte| byte.is_ascii_alphabetic())
            .count();
        let resto = &token[lettere..];
        let cifre = resto
            .iter()
            .take_while(|byte| byte.is_ascii_digit())
            .count();
        if lettere + cifre != token.len() {
            return Err(err(&PublicMessage::Curated(
                "riferimento di cella XLSX non conforme: atteso stile A1",
            )));
        }
        if lettere > MAX_LETTERE_RIFERIMENTO || cifre > MAX_CIFRE_RIFERIMENTO {
            return Err(err(&PublicMessage::Curated(
                "riferimento di cella XLSX oltre i limiti del formato \
                 (ultima colonna XFD, ultima riga 1048576)",
            )));
        }
    }
    Ok(())
}

struct XlsDataset {
    layers: Vec<LayerContract>,
    layout: XlsxLayout,
    spool: Arc<tempfile::NamedTempFile>,
    reader_gate: SingleReaderGate,
}

impl OpenDatasetHandle for XlsDataset {
    fn layers(&self) -> &[LayerContract] {
        &self.layers
    }
    fn fidelity_assessment(&self) -> plenora_io_core::FidelityAssessment {
        plenora_io_core::FidelityAssessment::for_format(
            DESCRIPTOR.id(),
            DESCRIPTOR.fidelity_class(),
        )
    }
    fn open_layer_reader(&self, request: &ReadRequest) -> Result<Box<dyn LayerReader>> {
        plenora_io_core::validate_read_projection(&DESCRIPTOR, request)?;
        let layout = self.layout.clone();
        let spool = Arc::clone(&self.spool);
        let layer = self.layers[0].clone();
        let cancellation = request.cancellation.clone();
        let batch_sizer = plenora_io_core::AdaptiveBatchSizer::new(
            layer.contract.schema.as_ref(),
            request.batch_target,
        );
        let reader = self.reader_gate.open(request.layer, || {
            spawn_xlsx_reader(spool, layout, batch_sizer, layer, cancellation.clone())
        })?;
        Ok(plenora_io_core::with_cancellation(
            reader,
            request.cancellation.clone(),
        ))
    }
}

#[derive(Clone, Copy)]
enum XlsxGeomSpec {
    Wkt(u32),
    Xy(u32, u32),
}

#[derive(Clone)]
struct XlsxLayout {
    attrs: Vec<(u32, ColType)>,
    schema: SchemaRef,
    data_rows: usize,
}

#[derive(Clone, Copy)]
struct SheetBounds {
    start: (u32, u32),
    end: (u32, u32),
}

// --- scrittura -------------------------------------------------------------

struct XlsWriterState {
    path: PathBuf,
    durable: bool,
    xy: bool,
    batches: Vec<RecordBatch>,
    wkb_limits: WkbLimits,
    max_output_bytes: u64,
}

// Usata come funzione in `map_err`: la firma per valore è imposta dal punto di
// chiamata, prenderla per riferimento costringerebbe a chiusure inutili.
#[allow(clippy::needless_pass_by_value)]
fn xls_err(e: rust_xlsxwriter::XlsxError) -> PlenoraIoError {
    err(&PublicMessage::CuratedPair("XLSX:", classe_xlsx(&e)))
}

/// Classe statica di un errore `rust_xlsxwriter`, per i messaggi pubblici.
///
/// Dodici percorsi di scrittura passavano da `xls_err`, e tutti riportavano il
/// `Display` della dipendenza. Non e' un rischio teorico: sette varianti di
/// `XlsxError` **portano il nome del foglio come dato** — `SheetnameReused`,
/// `SheetnameLengthExceeded`, `UnknownWorksheetNameOrIndex` e le altre — e il
/// nome del foglio viene dal file letto o dal piano di scrittura.
///
/// Come `classe_sqlite` in `driver-gpkg`: un vocabolario nostro, chiuso, che
/// tiene distinte le cause senza far uscire nulla e che non cambia se la
/// dipendenza riscrive i propri testi.
const fn classe_xlsx(errore: &rust_xlsxwriter::XlsxError) -> &'static str {
    use rust_xlsxwriter::XlsxError as E;
    match errore {
        E::RowColumnLimitError => "riga o colonna oltre il limite del formato",
        E::SheetnameCannotBeBlank(_) => "nome del foglio vuoto",
        E::SheetnameLengthExceeded(_) => "nome del foglio troppo lungo",
        E::SheetnameReused(_) => "nome del foglio gia' usato",
        E::SheetnameContainsInvalidCharacter(_) => "nome del foglio con caratteri non ammessi",
        E::SheetnameStartsOrEndsWithApostrophe(_) => "nome del foglio delimitato da apostrofi",
        E::MaxStringLengthExceeded => "stringa oltre il limite del formato",
        E::UnknownWorksheetNameOrIndex(_) => "foglio inesistente",
        E::ParameterError(_) => "parametro non valido",
        E::IoError(_) => "errore di I/O",
        E::ZipError(_) => "errore del contenitore ZIP",
        _ => "altro",
    }
}

fn write_cell(
    sheet: &mut rust_xlsxwriter::Worksheet,
    r: u32,
    c: u16,
    array: &ArrayRef,
    row: usize,
) -> Result<()> {
    match json_from_array(array, row)? {
        JsonValue::Null => {}
        JsonValue::Bool(b) => {
            sheet.write_boolean(r, c, b).map_err(xls_err)?;
        }
        JsonValue::Number(n) => {
            let value = n
                .as_f64()
                // Il valore non esce: e' una cella del dataset in ingresso.
                .ok_or_else(|| {
                    err(&PublicMessage::Curated(
                        "numero non rappresentabile come f64 in XLSX",
                    ))
                })?;
            sheet.write_number(r, c, value).map_err(xls_err)?;
        }
        JsonValue::String(s) => {
            sheet.write_string(r, c, &s).map_err(xls_err)?;
        }
        other => {
            sheet
                .write_string(r, c, other.to_string())
                .map_err(xls_err)?;
        }
    }
    Ok(())
}

impl FormatWriter for XlsWriterState {
    fn write(&mut self, batch: &RecordBatch) -> Result<()> {
        self.batches.push(batch.clone());
        Ok(())
    }

    fn finish(self: Box<Self>) -> Result<Published> {
        let mut wb = Workbook::new();
        let sheet = wb.add_worksheet();
        let limits = self.wkb_limits;
        let mut wrote_header = false;
        let mut r: u32 = 0;

        for batch in &self.batches {
            let schema = batch.schema();
            let geom_idx = geometry_index(&schema)
                .ok_or_else(|| err(&PublicMessage::Curated("nessuna colonna geometria")))?;
            let geom_col = batch
                .column(geom_idx)
                .as_any()
                .downcast_ref::<BinaryArray>()
                .ok_or_else(|| err(&PublicMessage::Curated("colonna geometria non binaria")))?;

            if !wrote_header {
                let mut col: u16 = 0;
                for (i, f) in schema.fields().iter().enumerate() {
                    if i != geom_idx {
                        sheet.write_string(0, col, f.name()).map_err(xls_err)?;
                        col += 1;
                    }
                }
                if self.xy {
                    sheet.write_string(0, col, "x").map_err(xls_err)?;
                    sheet.write_string(0, col + 1, "y").map_err(xls_err)?;
                } else {
                    sheet.write_string(0, col, "geometry").map_err(xls_err)?;
                }
                wrote_header = true;
                r = 1;
            }

            for row in 0..batch.num_rows() {
                let mut col: u16 = 0;
                for (i, _) in schema.fields().iter().enumerate() {
                    if i != geom_idx {
                        write_cell(sheet, r, col, batch.column(i), row)?;
                        col += 1;
                    }
                }
                if !geom_col.is_null(row) {
                    let g = decode_wkb(geom_col.value(row), &limits)?;
                    if self.xy {
                        match &g.value {
                            WkbValue::Point(point) if g.dimensions == CoordinateDimensions::Xy => {
                                sheet.write_number(r, col, point.x).map_err(xls_err)?;
                                sheet.write_number(r, col + 1, point.y).map_err(xls_err)?;
                            }
                            _ => {
                                return Err(err(&PublicMessage::Curated(
                                    "encoding xy richiede geometrie Point strettamente XY",
                                )))
                            }
                        }
                    } else {
                        sheet
                            .write_string(r, col, format_wkt(&g)?)
                            .map_err(xls_err)?;
                    }
                }
                r += 1;
            }
        }

        let buf = wb.save_to_buffer().map_err(xls_err)?;
        let mut temp = create_staged_file(&self.path)?;
        temp.as_file_mut().write_all(&buf)?;
        temp.as_file_mut().flush()?;
        let (bytes, outcome) =
            publish_file_atomic_limited(temp, &self.path, self.durable, self.max_output_bytes)?;
        Ok(Published {
            bytes,
            loss: LossReport::default(),
            fidelity: plenora_io_core::FidelityAssessment::lossless(),
            outcome,
        })
    }
}

fn data_to_string(d: &Data) -> String {
    match d {
        Data::String(s) => s.clone(),
        Data::Empty => String::new(),
        other => other.to_string(),
    }
}

const fn classify_data(data: &Data) -> ObservedValueClass {
    match data {
        Data::Int(value) => classify_i64(*value),
        Data::Float(value) if value.is_finite() => ObservedValueClass::Number,
        Data::String(_) | Data::DateTimeIso(_) | Data::DurationIso(_) => ObservedValueClass::Text,
        Data::Bool(_) => ObservedValueClass::Boolean,
        _ => ObservedValueClass::Null,
    }
}

fn data_row_width(bounds: SheetBounds) -> Result<usize> {
    bounds
        .end
        .1
        .checked_sub(bounds.start.1)
        .and_then(|width| width.checked_add(1))
        .and_then(|width| usize::try_from(width).ok())
        .ok_or_else(|| err(&PublicMessage::Curated("dimensioni XLSX non valide")))
}

fn data_row_count(bounds: SheetBounds) -> Result<usize> {
    bounds
        .end
        .0
        .checked_sub(bounds.start.0)
        .and_then(|rows| usize::try_from(rows).ok())
        .ok_or_else(|| err(&PublicMessage::Curated("dimensioni XLSX non valide")))
}

fn for_each_dense_row<RS, F>(
    reader: &mut LettoreCelleSorvegliato<'_, RS>,
    bounds: SheetBounds,
    cancellation: &CancellationToken,
    mut visit: F,
) -> Result<usize>
where
    RS: Read + Seek,
    F: FnMut(u32, &[Data]) -> Result<bool>,
{
    let width = data_row_width(bounds)?;
    let mut pending: Option<(u32, u32, Data)> = None;
    let mut observed_cells = 0usize;

    for (row_index, row) in (bounds.start.0..=bounds.end.0).enumerate() {
        check_cancelled_periodically(cancellation, ErrorPhase::Read, row_index)?;
        let mut values = vec![Data::Empty; width];
        loop {
            let next = if let Some(cell) = pending.take() {
                Some(cell)
            } else {
                let cell = reader.prossima_cella()?;
                if cell.is_some() {
                    observed_cells += 1;
                }
                cell
            };
            let Some((cell_row, cell_column, value)) = next else {
                break;
            };
            if cell_row > row {
                pending = Some((cell_row, cell_column, value));
                break;
            }
            if cell_row < row {
                return Err(err(&PublicMessage::Curated(
                    "ordine delle celle XLSX non monotono",
                )));
            }
            if cell_column < bounds.start.1 || cell_column > bounds.end.1 {
                return Err(err(&PublicMessage::Curated(
                    "cella XLSX fuori dalle dimensioni dichiarate",
                )));
            }
            let offset = usize::try_from(cell_column - bounds.start.1).map_err(|_| {
                err(&PublicMessage::Curated(
                    "indice colonna XLSX non rappresentabile",
                ))
            })?;
            values[offset] = value;
        }
        if !visit(row, &values)? {
            break;
        }
    }
    Ok(observed_cells)
}

fn resolve_geometry(
    headers: &[String],
    start_column: u32,
    opts: &BTreeMap<String, String>,
) -> Result<(XlsxGeomSpec, BTreeSet<u32>)> {
    let index = |name: &str| {
        headers
            .iter()
            .position(|header| header == name)
            .and_then(|offset| u32::try_from(offset).ok())
            .and_then(|offset| start_column.checked_add(offset))
    };
    if let Some(wkt_name) = opts.get("wkt_column") {
        // I nomi delle colonne non escono: sono valori d'opzione, e l'unico
        // testo runtime ammesso e' il token del validatore centrale, che qui
        // non c'e' — `wkt_column` e' `Testo`, quindi lo schema lo accetta e il
        // rifiuto nasce dal confronto con l'intestazione di questo foglio.
        let column = index(wkt_name).ok_or_else(|| {
            err(&PublicMessage::Curated(
                "colonna WKT assente dall'intestazione",
            ))
        })?;
        return Ok((XlsxGeomSpec::Wkt(column), BTreeSet::from([column])));
    }
    if let (Some(x_name), Some(y_name)) = (opts.get("x_column"), opts.get("y_column")) {
        let x_column = index(x_name).ok_or_else(|| {
            err(&PublicMessage::Curated(
                "colonna X assente dall'intestazione",
            ))
        })?;
        let y_column = index(y_name).ok_or_else(|| {
            err(&PublicMessage::Curated(
                "colonna Y assente dall'intestazione",
            ))
        })?;
        return Ok((
            XlsxGeomSpec::Xy(x_column, y_column),
            BTreeSet::from([x_column, y_column]),
        ));
    }
    Err(err(&PublicMessage::Curated(
        "specificare wkt_column, oppure x_column con y_column, in format_options",
    )))
}

fn cell_at(row: &[Data], bounds: SheetBounds, column: u32) -> Result<&Data> {
    let offset = column
        .checked_sub(bounds.start.1)
        .and_then(|offset| usize::try_from(offset).ok())
        .ok_or_else(|| err(&PublicMessage::Curated("indice colonna XLSX non valido")))?;
    row.get(offset).ok_or_else(|| {
        err(&PublicMessage::Curated(
            "riga XLSX fuori dalle dimensioni dichiarate",
        ))
    })
}

fn encode_geometry_cell(
    row: &[Data],
    bounds: SheetBounds,
    geom: XlsxGeomSpec,
    cella_wkt: WkbLimits,
    detected_dimensions: &mut BTreeSet<CoordinateDimensions>,
    detected_types: &mut BTreeSet<GeometryType>,
    wkb_buffer: &mut Vec<u8>,
) -> Result<bool> {
    match geom {
        XlsxGeomSpec::Wkt(column) => {
            let text = data_to_string(cell_at(row, bounds, column)?);
            if text.trim().is_empty() {
                return Ok(false);
            }
            // Cap sulla lunghezza della cella WKT prima di costruire l'AST.
            // Da S5 e' la quota **configurata** dal chiamante: chi stringe
            // `--max-wkb-cell-bytes` vede il rifiuto qui, dove l'AST verrebbe
            // allocato, invece che dopo.
            let geometry = parse_wkt_bounded(text.trim(), &cella_wkt)?;
            detected_dimensions.insert(geometry.dimensions);
            detected_types.insert(geometry.geometry_type());
            wkb_buffer.clear();
            plenora_io_model::wkb::encode_wkb_into_bounded(
                &geometry,
                WkbFlavor::Iso,
                wkb_buffer,
                cella_wkt.max_cell_bytes,
            )?;
        }
        XlsxGeomSpec::Xy(x_column, y_column) => {
            let x = coordinate_cell(Some(cell_at(row, bounds, x_column)?), "X")?;
            let y = coordinate_cell(Some(cell_at(row, bounds, y_column)?), "Y")?;
            match (x, y) {
                (Some(x), Some(y)) => {
                    let geometry = WkbGeometry {
                        value: WkbValue::Point(WkbCoordinate {
                            x,
                            y,
                            z: None,
                            m: None,
                        }),
                        dimensions: CoordinateDimensions::Xy,
                        srid: None,
                    };
                    wkb_buffer.clear();
                    plenora_io_model::wkb::encode_wkb_into_bounded(
                        &geometry,
                        WkbFlavor::Iso,
                        wkb_buffer,
                        cella_wkt.max_cell_bytes,
                    )?;
                }
                (None, None) => return Ok(false),
                _ => {
                    return Err(err(&PublicMessage::Curated(
                        "geometria XY incompleta: X e Y devono essere entrambi presenti",
                    )))
                }
            }
        }
    }
    Ok(true)
}

const SPOOL_NULL_GEOMETRY: u32 = u32::MAX;
const SPOOL_NULL: u8 = 0;
const SPOOL_INTEGER: u8 = 1;
const SPOOL_NUMBER: u8 = 2;
const SPOOL_BOOLEAN: u8 = 3;
const SPOOL_TEXT: u8 = 4;

struct BoundedSpoolWriter<'a> {
    writer: BufWriter<&'a std::fs::File>,
    bytes: u64,
    limit: u64,
    budget: OperationBudget,
    /// Le prenotazioni di spill restano vive quanto il file temporaneo.
    ///
    /// Nel modello legacy si faceva `commit`, cioe' consumo definitivo: la
    /// quota non tornava mai, nemmeno dopo che il file era stato rimosso. Nel
    /// modello unificato lo spill e' occupazione trattenuta e la `SpillLease`
    /// la restituisce al drop, insieme allo spool che l'ha creata.
    leases: Vec<SpillLease>,
}

impl<'a> BoundedSpoolWriter<'a> {
    fn new(file: &'a std::fs::File, limit: u64, budget: OperationBudget) -> Self {
        Self {
            writer: BufWriter::new(file),
            bytes: 0,
            limit,
            budget,
            leases: Vec::new(),
        }
    }

    fn write(&mut self, bytes: &[u8]) -> Result<()> {
        let length = u64::try_from(bytes.len())
            .map_err(|_| err(&PublicMessage::Curated("spool XLSX non rappresentabile")))?;
        let next = self.bytes.checked_add(length).ok_or_else(|| {
            err(&PublicMessage::Curated(
                "dimensione spool XLSX fuori intervallo",
            ))
        })?;
        if next > self.limit {
            return Err(PlenoraIoError::limite_redatto(
                &PublicMessage::CuratedBetween(
                    "spool XLSX di",
                    NumeroStrutturale::Conteggio(next),
                    "byte oltre il limite di",
                    NumeroStrutturale::Limite(self.limit),
                ),
            ));
        }
        let lease = self.budget.context().lease_spill(length)?;
        self.writer.write_all(bytes)?;
        self.leases.push(lease);
        self.bytes = next;
        Ok(())
    }

    fn finish(mut self) -> Result<()> {
        self.writer.flush()?;
        Ok(())
    }

    fn geometry(&mut self, value: Option<&[u8]>) -> Result<()> {
        let length = match value {
            None => SPOOL_NULL_GEOMETRY,
            Some(bytes) => u32::try_from(bytes.len()).map_err(|_| {
                PlenoraIoError::limite_redatto(&PublicMessage::Curated(
                    "geometria XLSX troppo grande per lo spool",
                ))
            })?,
        };
        self.write(&length.to_le_bytes())?;
        if let Some(bytes) = value {
            self.write(bytes)?;
        }
        Ok(())
    }

    fn data(&mut self, value: &Data) -> Result<()> {
        match value {
            Data::Int(value) => {
                self.write(&[SPOOL_INTEGER])?;
                self.write(&value.to_le_bytes())
            }
            Data::Float(value) if value.is_finite() => {
                self.write(&[SPOOL_NUMBER])?;
                self.write(&value.to_le_bytes())
            }
            Data::Bool(value) => self.write(&[SPOOL_BOOLEAN, u8::from(*value)]),
            Data::String(value) | Data::DateTimeIso(value) | Data::DurationIso(value) => {
                let bytes = value.as_bytes();
                let length = u32::try_from(bytes.len()).map_err(|_| {
                    PlenoraIoError::limite_redatto(&PublicMessage::Curated(
                        "testo XLSX troppo grande per lo spool",
                    ))
                })?;
                self.write(&[SPOOL_TEXT])?;
                self.write(&length.to_le_bytes())?;
                self.write(bytes)
            }
            _ => self.write(&[SPOOL_NULL]),
        }
    }
}

// Inferenza di layout e contratto in una sola passata sul foglio: le fasi
// (intestazioni, accumulatori di tipo, spool) condividono lo stato riga per riga
// e separarle non ridurrebbe la complessità, solo la leggibilità.
/// Le tre quote che l'inferenza del layout consulta.
///
/// Un config privato tipizzato invece di un `Limits` intero: sono i soli
/// valori usati, e nel modello unificato quel tipo non esiste. Tenerli
/// insieme evita anche di allungare la lista dei parametri oltre il tetto.
#[derive(Clone, Copy)]
struct XlsxQuote {
    colonne: usize,
    righe: usize,
    byte_ingresso: u64,
    /// Tetto sui byte di una cella WKT, applicato **prima** di costruire
    /// l'AST. Fino a S5 il percorso di produzione usava qui il default del
    /// contratto, quindi `--max-wkb-cell-bytes` non arrivava all'inferenza.
    cella_wkt: WkbLimits,
}

impl XlsxQuote {
    fn from_read_options(opts: &ReadOptions) -> Self {
        Self {
            colonne: opts.max_columns(),
            righe: opts.max_rows(),
            byte_ingresso: opts.max_input_bytes(),
            cella_wkt: opts.wkb_limits(),
        }
    }
}

#[allow(clippy::too_many_lines)]
fn infer_layout<RS>(
    workbook: &mut Xlsx<RS>,
    sheet: &str,
    opts: &BTreeMap<String, String>,
    crs: &str,
    cancellation: &CancellationToken,
    quote: XlsxQuote,
    budget: &OperationBudget,
) -> Result<(XlsxLayout, DataContract, Arc<tempfile::NamedTempFile>)>
where
    RS: Read + Seek,
{
    check_cancelled(cancellation, ErrorPhase::Read)?;
    let mut reader = LettoreCelleSorvegliato::nuovo(workbook, sheet)?;
    let bounds = reader.dimensioni()?;
    let width = data_row_width(bounds)?;
    let row_count = data_row_count(bounds)?;
    if width > quote.colonne {
        return Err(PlenoraIoError::limite_redatto(
            &PublicMessage::CuratedBetween(
                "XLSX:",
                NumeroStrutturale::Conteggio(driver_common::saturating_u64(width)),
                "colonne oltre il limite di",
                NumeroStrutturale::Limite(driver_common::saturating_u64(quote.colonne)),
            ),
        ));
    }
    if row_count > quote.righe {
        return Err(PlenoraIoError::limite_redatto(
            &PublicMessage::CuratedBetween(
                "XLSX:",
                NumeroStrutturale::Conteggio(driver_common::saturating_u64(row_count)),
                "righe oltre il limite di",
                NumeroStrutturale::Limite(driver_common::saturating_u64(quote.righe)),
            ),
        ));
    }

    let mut headers: Option<Vec<String>> = None;
    let mut geom = None;
    let mut geom_columns = BTreeSet::new();
    let mut accumulators: Vec<TypeAccumulator> = Vec::new();
    let mut detected_dimensions = BTreeSet::new();
    let mut detected_types = BTreeSet::new();
    let spool = Arc::new(tempfile::NamedTempFile::new()?);
    let mut spool_writer =
        BoundedSpoolWriter::new(spool.as_file(), quote.byte_ingresso, budget.clone());
    let mut wkb_buffer = Vec::new();
    let observed_cells =
        for_each_dense_row(&mut reader, bounds, cancellation, |row_index, row| {
            budget.context().ensure_active()?;
            if row_index == bounds.start.0 {
                let row_headers: Vec<String> = row.iter().map(data_to_string).collect();
                let (resolved_geom, resolved_columns) =
                    resolve_geometry(&row_headers, bounds.start.1, opts)?;
                accumulators = vec![TypeAccumulator::default(); width - resolved_columns.len()];
                geom = Some(resolved_geom);
                geom_columns = resolved_columns;
                headers = Some(row_headers);
                return Ok(true);
            }
            let resolved_geom =
                geom.ok_or_else(|| err(&PublicMessage::Curated("intestazione XLSX assente")))?;
            let has_geometry = encode_geometry_cell(
                row,
                bounds,
                resolved_geom,
                quote.cella_wkt,
                &mut detected_dimensions,
                &mut detected_types,
                &mut wkb_buffer,
            )?;
            spool_writer.geometry(has_geometry.then_some(wkb_buffer.as_slice()))?;
            let mut attribute_index = 0usize;
            for (offset, data) in row.iter().enumerate() {
                let column = bounds
                    .start
                    .1
                    .checked_add(
                        u32::try_from(offset)
                            .map_err(|_| err(&PublicMessage::Curated("troppe colonne XLSX")))?,
                    )
                    .ok_or_else(|| {
                        err(&PublicMessage::Curated(
                            "indice colonna XLSX fuori intervallo",
                        ))
                    })?;
                if geom_columns.contains(&column) {
                    continue;
                }
                accumulators[attribute_index].observe(classify_data(data));
                spool_writer.data(data)?;
                attribute_index += 1;
            }
            Ok(true)
        })?;
    spool_writer.finish()?;
    if observed_cells == 0 {
        return Err(err(&PublicMessage::Curated("foglio vuoto")));
    }
    let headers =
        headers.ok_or_else(|| err(&PublicMessage::Curated("intestazione XLSX assente")))?;
    let geom =
        geom.ok_or_else(|| err(&PublicMessage::Curated("geometria XLSX non configurata")))?;

    if matches!(geom, XlsxGeomSpec::Xy(_, _)) {
        detected_dimensions.insert(CoordinateDimensions::Xy);
        detected_types.insert(GeometryType::Point);
    }
    let dimensions = if detected_dimensions.len() == 1 {
        detected_dimensions
            .iter()
            .next()
            .copied()
            .unwrap_or(CoordinateDimensions::Unknown)
    } else {
        CoordinateDimensions::Unknown
    };
    let kind = if crs == "OGC:CRS84" || crs == "EPSG:4326" {
        CrsKind::Geographic
    } else {
        CrsKind::Unknown
    };
    let mut geometry_contract = GeometryColumnContract::wkb_xy(
        FieldId(0),
        GEOMETRY,
        ResolvedCrs::new(Some(crs.to_owned()), kind, None),
        true,
    );
    geometry_contract.dimensions = dimensions;
    geometry_contract.set_exact_geometry_types(detected_types.into_iter().collect());
    geometry_contract.native_metadata.insert(
        "xlsx.geometry_encoding".to_owned(),
        if matches!(geom, XlsxGeomSpec::Wkt(_)) {
            "wkt"
        } else {
            "xy_columns"
        }
        .to_owned(),
    );
    let mut fields = vec![with_geometry_contract_metadata(
        &geometry_field(GEOMETRY, crs),
        &geometry_contract,
    )];
    let mut attrs = Vec::with_capacity(accumulators.len());
    let mut attribute_index = 0usize;
    for (offset, name) in headers.iter().enumerate() {
        let column = bounds
            .start
            .1
            .checked_add(
                u32::try_from(offset)
                    .map_err(|_| err(&PublicMessage::Curated("troppe colonne XLSX")))?,
            )
            .ok_or_else(|| {
                err(&PublicMessage::Curated(
                    "indice colonna XLSX fuori intervallo",
                ))
            })?;
        if geom_columns.contains(&column) {
            continue;
        }
        let column_type = accumulators[attribute_index].column_type();
        fields.push(Field::new(name, column_type.arrow_data_type(), true));
        attrs.push((column, column_type));
        attribute_index += 1;
    }

    let schema: SchemaRef = Arc::new(Schema::new(fields));
    let contract = DataContract::new(schema, Some(geometry_contract));
    let schema = contract.schema.clone();
    Ok((
        XlsxLayout {
            attrs,
            schema,
            data_rows: row_count,
        },
        contract,
        spool,
    ))
}

fn finish_read_batch(
    schema: &SchemaRef,
    geometry: &mut BinaryBuilder,
    attributes: &mut [InferredColumnBuilder],
    row_count: usize,
) -> Result<RecordBatch> {
    let mut arrays: Vec<ArrayRef> = Vec::with_capacity(1 + attributes.len());
    arrays.push(Arc::new(geometry.finish()));
    for builder in attributes {
        arrays.push(builder.finish());
    }
    let options = RecordBatchOptions::new().with_row_count(Some(row_count));
    RecordBatch::try_new_with_options(schema.clone(), arrays, &options)
        .map_err(|_| err(&PublicMessage::Curated("batch XLSX non costruibile")))
}

fn read_spool_exact(reader: &mut impl Read, bytes: &mut [u8]) -> Result<()> {
    reader
        .read_exact(bytes)
        .map_err(|_| err(&PublicMessage::Curated("spool XLSX troncato o illeggibile")))
}

fn read_spool_geometry(
    reader: &mut impl Read,
    builder: &mut BinaryBuilder,
    buffer: &mut Vec<u8>,
) -> Result<()> {
    let mut length_bytes = [0u8; 4];
    read_spool_exact(reader, &mut length_bytes)?;
    let length = u32::from_le_bytes(length_bytes);
    if length == SPOOL_NULL_GEOMETRY {
        builder.append_null();
        return Ok(());
    }
    let length = usize::try_from(length).map_err(|_| {
        err(&PublicMessage::Curated(
            "lunghezza geometria spool non valida",
        ))
    })?;
    buffer.resize(length, 0);
    read_spool_exact(reader, buffer)?;
    builder.append_value(buffer.as_slice());
    Ok(())
}

fn read_spool_data(
    reader: &mut impl Read,
    builder: &mut InferredColumnBuilder,
    buffer: &mut Vec<u8>,
) -> Result<()> {
    let mut tag = [0u8; 1];
    read_spool_exact(reader, &mut tag)?;
    match tag[0] {
        SPOOL_NULL => {
            builder.append_null();
            Ok(())
        }
        SPOOL_INTEGER => {
            let mut bytes = [0u8; 8];
            read_spool_exact(reader, &mut bytes)?;
            builder.append_i64(i64::from_le_bytes(bytes))
        }
        SPOOL_NUMBER => {
            let mut bytes = [0u8; 8];
            read_spool_exact(reader, &mut bytes)?;
            builder.append_f64(f64::from_le_bytes(bytes))
        }
        SPOOL_BOOLEAN => {
            let mut value = [0u8; 1];
            read_spool_exact(reader, &mut value)?;
            match value[0] {
                0 => builder.append_bool(false),
                1 => builder.append_bool(true),
                _ => Err(err(&PublicMessage::Curated(
                    "booleano spool XLSX non valido",
                ))),
            }
        }
        SPOOL_TEXT => {
            let mut length = [0u8; 4];
            read_spool_exact(reader, &mut length)?;
            let length = usize::try_from(u32::from_le_bytes(length))
                .map_err(|_| err(&PublicMessage::Curated("lunghezza testo spool non valida")))?;
            buffer.resize(length, 0);
            read_spool_exact(reader, buffer)?;
            let text = std::str::from_utf8(buffer)
                .map_err(|_| err(&PublicMessage::Curated("testo spool XLSX non UTF-8")))?;
            builder.append_str(text)
        }
        _ => Err(err(&PublicMessage::Curated("tag spool XLSX non valido"))),
    }
}

fn spawn_xlsx_reader(
    spool: Arc<tempfile::NamedTempFile>,
    layout: XlsxLayout,
    mut batch_sizer: plenora_io_core::AdaptiveBatchSizer,
    layer: LayerContract,
    cancellation: CancellationToken,
) -> Result<Box<dyn LayerReader>> {
    spawn_batch_reader(DESCRIPTOR.id(), layer, 2, move |emitter: BatchEmitter| {
        let file = spool.reopen()?;
        let mut reader = BufReader::new(file);
        let mut geometry = BinaryBuilder::new();
        let mut attributes: Vec<InferredColumnBuilder> = layout
            .attrs
            .iter()
            .map(|(_, column_type)| InferredColumnBuilder::new(*column_type))
            .collect();
        let mut geometry_buffer = Vec::new();
        let mut text_buffer = Vec::new();
        let mut rows_in_batch = 0usize;
        for row_index in 0..layout.data_rows {
            check_cancelled_periodically(&cancellation, ErrorPhase::Read, row_index)?;
            read_spool_geometry(&mut reader, &mut geometry, &mut geometry_buffer)?;
            for builder in &mut attributes {
                read_spool_data(&mut reader, builder, &mut text_buffer)?;
            }
            rows_in_batch += 1;
            if rows_in_batch >= batch_sizer.rows() {
                let batch = finish_read_batch(
                    &layout.schema,
                    &mut geometry,
                    &mut attributes,
                    rows_in_batch,
                )?;
                batch_sizer.observe(&batch);
                rows_in_batch = 0;
                if !emitter.send(batch) {
                    return Ok(());
                }
            }
        }
        if rows_in_batch > 0 {
            let batch = finish_read_batch(
                &layout.schema,
                &mut geometry,
                &mut attributes,
                rows_in_batch,
            )?;
            if !emitter.send(batch) {
                return Ok(());
            }
        }
        Ok(())
    })
}

fn coordinate_cell(cell: Option<&Data>, axis: &'static str) -> Result<Option<f64>> {
    const MAX_EXACT_F64_INTEGER: i64 = 1_i64 << 53;

    let value = match cell {
        None | Some(Data::Empty) => return Ok(None),
        Some(Data::Float(value)) if value.is_finite() => *value,
        Some(Data::Int(value))
            if *value >= -MAX_EXACT_F64_INTEGER && *value <= MAX_EXACT_F64_INTEGER =>
        {
            // La guardia limita |value| a 2^53: la conversione a f64 è esatta,
            // nessuna perdita di precisione possibile.
            #[allow(clippy::cast_precision_loss)]
            {
                *value as f64
            }
        }
        Some(Data::String(value)) if value.trim().is_empty() => return Ok(None),
        Some(Data::String(value)) => value
            .trim()
            .parse::<f64>()
            .ok()
            .filter(|value| value.is_finite())
            .ok_or_else(|| {
                err(&PublicMessage::CuratedPair(
                    "coordinata non numerica o non finita sull'asse",
                    axis,
                ))
            })?,
        Some(_) => {
            return Err(err(&PublicMessage::CuratedPair(
                "coordinata non numerica, non finita o non rappresentabile senza perdita \
                 sull'asse",
                axis,
            )))
        }
    };
    Ok(Some(value))
}

#[cfg(test)]
mod tests {
    /// WKT con i tetti predefiniti: qui si provano le fixture, non le quote.
    fn wkt(testo: &str) -> plenora_io_model::Result<plenora_io_model::wkb::WkbGeometry> {
        super::parse_wkt_bounded(testo, &plenora_io_model::limits::WkbLimits::default())
    }
    use super::*;

    // --- ASSURANCE-N1: i rami negativi del pre-filtro sui riferimenti ---

    /// `valida_valore_riferimento`: gli input che farebbero traboccare
    /// `calamine` non arrivano al parser.
    ///
    /// La sonda prova **il contratto del pre-filtro**, che non e' la conformita'
    /// del riferimento. Vale la pena dire perche', perche' la prima stesura di
    /// questa sonda ha sbagliato in tutti e due i versi: prima pretendeva il
    /// rifiuto di `AA` -- descrivendo una grammatica che la funzione non
    /// promette -- e poi, corretta, lo chiamava «riferimento valido», che e'
    /// altrettanto falso. `AA` non e' un riferimento di cella: e' un token che
    /// non puo' far traboccare l'accumulatore, e il pre-filtro lascia passare
    /// esattamente quello.
    ///
    /// Le tre affermazioni, separate:
    ///
    /// 1. **fermato** cio' che eccede i conteggi del formato, con i due rifiuti
    ///    tenuti distinti -- forma non A1 e lunghezza oltre i limiti;
    /// 2. **passano** riferimenti conformi rappresentativi, estremi inclusi;
    ///    il percorso end-to-end su un workbook conforme resta la prova che il
    ///    pre-filtro non rifiuti il caso reale;
    /// 3. **passano anche** valori lessicalmente innocui la cui conformita' e'
    ///    falsa o dipende dal tipo dell'attributo: il prefiltro li delega al
    ///    parser successivo, senza affermare che il driver li accetti.
    #[test]
    fn n1_valida_valore_riferimento_applica_i_tetti_senza_validare_il_formato() {
        // 1. Fermato: piu' di tre lettere o piu' di sette cifre. L'overflow di
        //    `calamine` diventa possibile solo a lunghezze maggiori -- sette
        //    lettere o dieci cifre. Questi casi sono fermati perche' eccedono i
        //    massimi lessicali del formato, non perche' ciascuno di essi
        //    traboccherebbe davvero.
        let oltre_i_conteggi: Vec<&str> = vec![
            "ABCD1",
            "XFDA1",
            "AAAAAAA1",
            "A12345678",
            "A1234567890",
            "A1:ZZZZ1",
        ];
        for valore in oltre_i_conteggi {
            let Err(errore) = valida_valore_riferimento(valore.as_bytes()) else {
                panic!("«{valore}» eccede i conteggi del formato e doveva essere fermato");
            };
            assert!(
                errore.message.contains("oltre i limiti del formato"),
                "«{valore}»: atteso il rifiuto sui limiti, arrivato «{}»",
                errore.message
            );
        }

        // Il secondo rifiuto, distinto dal primo: cifre prima delle lettere, o
        // caratteri che non sono ne' l'une ne' l'altre.
        let forma_non_a1: Vec<&str> = vec!["1A", "A1B", "A-1", "A1:2B"];
        for valore in forma_non_a1 {
            let Err(errore) = valida_valore_riferimento(valore.as_bytes()) else {
                panic!("«{valore}» non ha la forma lettere-poi-cifre e doveva essere fermato");
            };
            assert!(
                errore.message.contains("atteso stile A1"),
                "«{valore}»: atteso il rifiuto di forma, arrivato «{}»",
                errore.message
            );
        }

        // 2. Passano riferimenti conformi rappresentativi, estremi compresi.
        //    L'esaustivita' non viene inventata qui: il test sul workbook
        //    conforme esercita il pre-filtro nel suo percorso reale.
        let conformi: Vec<&str> = vec!["A1", "A1:C4", "XFD1048576", "XFD1", "A1048576"];
        for valore in conformi {
            assert!(
                valida_valore_riferimento(valore.as_bytes()).is_ok(),
                "«{valore}» e' conforme e non deve essere fermato"
            );
        }

        // 3. Passano anche, ed e' **voluto**, valori che il pre-filtro puo'
        //    inoltrare senza rischio di overflow. Alcuni non sono conformi,
        //    per altri la conformita' dipende dal tipo dell'attributo: questa
        //    funzione non ha quel contesto e non emette un verdetto. Il test
        //    fissa la delega al parser, non l'accettazione da parte del driver.
        let tollerati_dal_prefiltro: Vec<(&str, &str)> = vec![
            ("", "nessun token da accumulare"),
            (
                "AA",
                "sole lettere: non e' uno ST_CellRef, gli manca la riga",
            ),
            ("12", "sole cifre: valido per row@r, non come ST_CellRef"),
            (
                "$A$1",
                "il dollaro viene separato senza decidere il tipo dell'attributo",
            ),
            (
                "A1 B2 C3",
                "la lista viene separata senza decidere il tipo dell'attributo",
            ),
            (
                "A1,B2",
                "l'unione viene separata senza decidere il tipo dell'attributo",
            ),
            ("A:A", "colonna intera: non ammessa da dimension@ref"),
            ("1:1", "riga intera: non ammessa da dimension@ref"),
            ("XFE1", "tre lettere, ma oltre l'ultima colonna XFD"),
            ("A1048577", "sette cifre, ma oltre l'ultima riga"),
        ];
        for (valore, perche) in tollerati_dal_prefiltro {
            assert!(
                valida_valore_riferimento(valore.as_bytes()).is_ok(),
                "«{valore}» ({perche}) viene oggi delegato al parser: fermarlo qui \
                 cambia il confine del pre-filtro e richiede una decisione esplicita"
            );
        }
    }

    /// `cell_at`: i due modi in cui una colonna puo' non esistere.
    ///
    /// Una colonna **prima** dell'inizio dichiarato non produce un offset --
    /// `checked_sub` fallisce -- e una colonna oltre la fine produce un offset
    /// che la riga non contiene. Sono due rifiuti diversi, e la sonda li
    /// distingue: confonderli manderebbe chi legge a cercare il difetto nel
    /// posto sbagliato.
    #[test]
    fn n1_cell_at_distingue_la_colonna_prima_dell_inizio_da_quella_oltre_la_fine() {
        let riga = vec![Data::Int(1), Data::Int(2), Data::Int(3)];
        let bounds = SheetBounds {
            start: (0, 5),
            end: (10, 7),
        };

        for (colonna, atteso) in [(5_u32, 1_i64), (6, 2), (7, 3)] {
            let Ok(Data::Int(valore)) = cell_at(&riga, bounds, colonna) else {
                panic!("la colonna {colonna} e' dentro le dimensioni dichiarate");
            };
            assert_eq!(*valore, atteso, "colonna {colonna}");
        }

        for colonna in [0_u32, 1, 4] {
            let Err(errore) = cell_at(&riga, bounds, colonna) else {
                panic!("la colonna {colonna} precede l'inizio dichiarato");
            };
            assert!(
                errore.message.contains("indice colonna XLSX non valido"),
                "colonna {colonna}: arrivato «{}»",
                errore.message
            );
        }

        for colonna in [8_u32, 9, u32::MAX] {
            let Err(errore) = cell_at(&riga, bounds, colonna) else {
                panic!("la colonna {colonna} eccede la riga");
            };
            assert!(
                errore.message.contains("fuori dalle dimensioni dichiarate"),
                "colonna {colonna}: arrivato «{}»",
                errore.message
            );
        }
    }

    /// `LettoreCelleSorvegliato`: dopo un fallimento non si legge piu', e le due
    /// vie di lettura lo dicono con lo stesso errore.
    ///
    /// Il tipo promette che «dopo un panico il lettore viene scartato» sia una
    /// proprieta' del **tipo** e non una convenzione da ricordare: al primo
    /// fallimento il lettore cade e ogni chiamata successiva trova `None`.
    /// Nessun percorso del driver ci prova -- tutti propagano -- ed e' proprio
    /// per questo che le due guardie non erano mai state eseguite.
    ///
    /// La sonda costruisce lo stato invalidato direttamente, che e' l'unico modo
    /// di raggiungerle senza far panicare `calamine` davvero: il tipo e' privato
    /// del modulo e il campo pure, quindi la prova vive accanto a cio' che prova
    /// e non finge di passare da un'API che non lo permette.
    ///
    /// # Che cosa **non** prova
    ///
    /// Che la guardia non sia a consumo. Partendo da `None` un `take()` e un
    /// `as_mut()` si comportano identicamente -- entrambi lasciano il campo a
    /// `None` e ogni chiamata successiva fallisce -- quindi la ripetizione qui
    /// sotto osserva che il rifiuto e' **stabile**, non che sia `as_mut()` a
    /// produrlo. Distinguere i due vorrebbe partire da un lettore vivo e
    /// invalidarlo, cioe' far fallire `calamine` davvero.
    #[test]
    fn n1_un_lettore_invalidato_rifiuta_entrambe_le_letture() {
        // `std::io::Cursor` soddisfa `Read + Seek` e non viene mai toccato: il
        // lettore e' gia' `None`, quindi nessuna chiamata a `calamine` parte.
        let mut invalidato: LettoreCelleSorvegliato<'_, std::io::Cursor<Vec<u8>>> =
            LettoreCelleSorvegliato { lettore: None };

        let Err(su_dimensioni) = invalidato.dimensioni() else {
            panic!("un lettore invalidato non puo' dichiarare dimensioni");
        };
        assert!(
            su_dimensioni.message.contains(LETTORE_INVALIDATO),
            "dimensioni: arrivato «{}»",
            su_dimensioni.message
        );

        let Err(su_cella) = invalidato.prossima_cella() else {
            panic!("un lettore invalidato non puo' consegnare celle");
        };
        assert!(
            su_cella.message.contains(LETTORE_INVALIDATO),
            "prossima_cella: arrivato «{}»",
            su_cella.message
        );

        // Il rifiuto e' stabile: interrogarlo di nuovo non lo fa cambiare idea.
        // Non dice quale forma abbia la guardia -- vedi «Che cosa non prova».
        assert!(invalidato.dimensioni().is_err());
        assert!(invalidato.prossima_cella().is_err());
    }

    /// Un central directory ZIP64 costruito a mano, che **dichiara** dimensioni
    /// senza contenerle.
    ///
    /// Non e' uno ZIP64 pienamente conforme: le voci non hanno payload, e un
    /// lettore che provasse a **estrarle** fallirebbe. Cio' che la fixture
    /// costruisce, e l'unica cosa che serve qui, e' un central directory che
    /// `ZipArchive::new` accetta e da cui `compressed_size()` e `size()`
    /// restituiscono i valori dichiarati -- che e' esattamente la superficie su
    /// cui `validate_archive_ratio` lavora, perche' somma cio' che le voci
    /// dichiarano senza aprirle.
    ///
    /// Serve a misurare una cosa sola: se il crate `zip` restituisca i valori
    /// dichiarati nel central directory senza confrontarli con i byte che
    /// l'archivio contiene davvero. Da quella risposta dipende se i due
    /// `checked_add` di `validate_archive_ratio` siano raggiungibili.
    ///
    /// Due voci, entrambe `stored` e vuote, ciascuna con la propria coppia
    /// `(compressa, decompressa)`.
    ///
    /// Le due dimensioni sono **separate** e non e' un dettaglio di comodo:
    /// `validate_archive_ratio` le somma in due accumulatori distinti, e la
    /// prima stesura di questa fixture passava lo stesso valore a entrambi.
    /// Con `u64::MAX` e `1` su tutti e due, alla seconda voce traboccava per
    /// prima la somma dei compressi e il ramo dei decompressi non veniva mai
    /// raggiunto: una sonda verde che copriva un `checked_add` su due.
    fn zip64_con_dimensioni_dichiarate(voci: [(u64, u64); 2]) -> Vec<u8> {
        let mut archivio = Vec::new();
        let nomi = ["xl/worksheets/sheet1.xml", "xl/workbook.xml"];
        let mut offset_locali = Vec::new();

        for (nome, (compressa, decompressa)) in nomi.iter().zip(voci.iter()) {
            offset_locali.push(archivio.len() as u64);
            archivio.extend_from_slice(&0x0403_4b50_u32.to_le_bytes()); // firma locale
            archivio.extend_from_slice(&45_u16.to_le_bytes()); // versione: ZIP64
            archivio.extend_from_slice(&0_u16.to_le_bytes()); // flag
            archivio.extend_from_slice(&0_u16.to_le_bytes()); // metodo: stored
            archivio.extend_from_slice(&0_u16.to_le_bytes()); // ora
            archivio.extend_from_slice(&0_u16.to_le_bytes()); // data
            archivio.extend_from_slice(&0_u32.to_le_bytes()); // crc32
            archivio.extend_from_slice(&u32::MAX.to_le_bytes()); // compresso: vedi ZIP64
            archivio.extend_from_slice(&u32::MAX.to_le_bytes()); // decompresso: vedi ZIP64
            let lunghezza_nome =
                u16::try_from(nome.len()).expect("i nomi della sonda stanno in un u16");
            archivio.extend_from_slice(&lunghezza_nome.to_le_bytes());
            archivio.extend_from_slice(&20_u16.to_le_bytes()); // extra: 4 + 16
            archivio.extend_from_slice(nome.as_bytes());
            archivio.extend_from_slice(&0x0001_u16.to_le_bytes()); // tag ZIP64
            archivio.extend_from_slice(&16_u16.to_le_bytes());
            // Nessun byte di dati segue: la dichiarazione e' tutto cio' che conta.
            archivio.extend_from_slice(&decompressa.to_le_bytes());
            archivio.extend_from_slice(&compressa.to_le_bytes());
        }

        let inizio_central = archivio.len() as u64;
        for ((nome, (compressa, decompressa)), offset) in
            nomi.iter().zip(voci.iter()).zip(offset_locali.iter())
        {
            archivio.extend_from_slice(&0x0201_4b50_u32.to_le_bytes()); // firma central
            archivio.extend_from_slice(&45_u16.to_le_bytes()); // creato da
            archivio.extend_from_slice(&45_u16.to_le_bytes()); // richiede
            archivio.extend_from_slice(&0_u16.to_le_bytes()); // flag
            archivio.extend_from_slice(&0_u16.to_le_bytes()); // metodo
            archivio.extend_from_slice(&0_u16.to_le_bytes()); // ora
            archivio.extend_from_slice(&0_u16.to_le_bytes()); // data
            archivio.extend_from_slice(&0_u32.to_le_bytes()); // crc32
            archivio.extend_from_slice(&u32::MAX.to_le_bytes()); // compresso
            archivio.extend_from_slice(&u32::MAX.to_le_bytes()); // decompresso
            let lunghezza_nome =
                u16::try_from(nome.len()).expect("i nomi della sonda stanno in un u16");
            archivio.extend_from_slice(&lunghezza_nome.to_le_bytes());
            archivio.extend_from_slice(&28_u16.to_le_bytes()); // extra: 4 + 24
            archivio.extend_from_slice(&0_u16.to_le_bytes()); // commento
            archivio.extend_from_slice(&0_u16.to_le_bytes()); // disco
            archivio.extend_from_slice(&0_u16.to_le_bytes()); // attributi interni
            archivio.extend_from_slice(&0_u32.to_le_bytes()); // attributi esterni
            archivio.extend_from_slice(&u32::MAX.to_le_bytes()); // offset: vedi ZIP64
            archivio.extend_from_slice(nome.as_bytes());
            archivio.extend_from_slice(&0x0001_u16.to_le_bytes());
            archivio.extend_from_slice(&24_u16.to_le_bytes());
            archivio.extend_from_slice(&decompressa.to_le_bytes());
            archivio.extend_from_slice(&compressa.to_le_bytes());
            archivio.extend_from_slice(&offset.to_le_bytes()); // offset locale
        }
        let dimensione_central = archivio.len() as u64 - inizio_central;
        let offset_zip64_eocd = archivio.len() as u64;

        archivio.extend_from_slice(&0x0606_4b50_u32.to_le_bytes()); // ZIP64 EOCD
        archivio.extend_from_slice(&44_u64.to_le_bytes()); // dimensione residua
        archivio.extend_from_slice(&45_u16.to_le_bytes());
        archivio.extend_from_slice(&45_u16.to_le_bytes());
        archivio.extend_from_slice(&0_u32.to_le_bytes()); // disco
        archivio.extend_from_slice(&0_u32.to_le_bytes()); // disco del central
        archivio.extend_from_slice(&(nomi.len() as u64).to_le_bytes()); // voci sul disco
        archivio.extend_from_slice(&(nomi.len() as u64).to_le_bytes()); // voci totali
        archivio.extend_from_slice(&dimensione_central.to_le_bytes());
        archivio.extend_from_slice(&inizio_central.to_le_bytes());

        archivio.extend_from_slice(&0x0706_4b50_u32.to_le_bytes()); // localizzatore
        archivio.extend_from_slice(&0_u32.to_le_bytes());
        archivio.extend_from_slice(&offset_zip64_eocd.to_le_bytes());
        archivio.extend_from_slice(&1_u32.to_le_bytes());

        archivio.extend_from_slice(&0x0605_4b50_u32.to_le_bytes()); // EOCD
        archivio.extend_from_slice(&0_u16.to_le_bytes());
        archivio.extend_from_slice(&0_u16.to_le_bytes());
        archivio.extend_from_slice(&u16::MAX.to_le_bytes()); // vedi ZIP64
        archivio.extend_from_slice(&u16::MAX.to_le_bytes());
        archivio.extend_from_slice(&u32::MAX.to_le_bytes());
        archivio.extend_from_slice(&u32::MAX.to_le_bytes());
        archivio.extend_from_slice(&0_u16.to_le_bytes()); // commento
        archivio
    }

    /// Un caso della tabella di overflow: il nome, le due voci dell'archivio
    /// come `(compressa, decompressa)`, e il messaggio che deve arrivare.
    type CasoDiOverflow = (&'static str, [(u64, u64); 2], &'static str);

    /// I due `checked_add` di `validate_archive_ratio` sono raggiungibili, **uno
    /// per volta**, e la somma delle dimensioni dichiarate fallisce chiusa.
    ///
    /// # Perche' serviva un archivio costruito a mano
    ///
    /// Con ZIP32 l'overflow non e' raggiungibile per aritmetica: le dimensioni
    /// stanno in campi `u32` -- al piu' 4 294 967 295 ciascuna -- e il conteggio
    /// delle voci nell'EOCD e' un `u16`, al piu' 65 535. La somma massima e'
    /// circa 2,8 x 10^14, cinque ordini di grandezza sotto `u64::MAX`. Nessuna
    /// libreria che rispetti il formato puo' portarci.
    ///
    /// ZIP64 cambia i due campi in `u64`, e la domanda diventa una sola: il
    /// crate `zip` restituisce cio' che il central directory **dichiara**, o lo
    /// confronta con i byte presenti? Questa sonda lo misura invece di
    /// supporlo, ed e' la ragione per cui il gruppo non era dichiarabile
    /// difensivo.
    ///
    /// # Perche' due casi e non uno
    ///
    /// Gli accumulatori sono due e il primo che trabocca ferma la funzione. Con
    /// la stessa coppia di valori su compresso e decompresso -- come faceva la
    /// prima stesura -- fallisce sempre la somma dei **compressi**, e il ramo
    /// dei decompressi resta scoperto mentre la sonda e' verde. Ogni caso
    /// carica quindi l'overflow su un accumulatore e tiene innocuo l'altro, e
    /// l'asserzione nomina il messaggio specifico invece del prefisso comune:
    /// «overflow nel conteggio dei byte» non distingue i due.
    #[test]
    fn n1_le_dimensioni_dichiarate_in_zip64_non_sommano_in_silenzio() {
        let dir = tempfile::tempdir().unwrap();
        let opzioni = opzioni_lettura();

        // Il controllo che rende interpretabile il resto: lo stesso archivio con
        // dimensioni piccole **passa**. Senza, un rifiuto non distinguerebbe
        // «la somma trabocca» da «il central directory costruito dalla sonda non
        // e' leggibile», e la sonda proverebbe l'incapacita' di chi l'ha scritta
        // invece del comportamento del driver.
        let innocuo = dir.path().join("innocuo.xlsx");
        std::fs::write(&innocuo, zip64_con_dimensioni_dichiarate([(2, 2), (3, 3)])).unwrap();
        validate_archive_ratio(&innocuo, opzioni.budget()).expect(
            "il central directory della sonda deve essere accettato da `ZipArchive::new`              con dimensioni piccole: se fallisce qui, a essere sbagliato e' l'archivio              costruito dalla sonda e non il driver",
        );

        // Un accumulatore per volta: `(compressa, decompressa)` per ciascuna
        // delle due voci, e il messaggio che deve arrivare.
        let casi: [CasoDiOverflow; 2] = [
            (
                "compressi",
                [(u64::MAX, 2), (1, 3)],
                "overflow nel conteggio dei byte compressi",
            ),
            (
                "decompressi",
                [(2, u64::MAX), (3, 1)],
                "overflow nel conteggio dei byte decompressi",
            ),
        ];

        for (nome, voci, atteso) in casi {
            let percorso = dir.path().join(format!("{nome}.xlsx"));
            std::fs::write(&percorso, zip64_con_dimensioni_dichiarate(voci)).unwrap();
            let Err(errore) = validate_archive_ratio(&percorso, opzioni.budget()) else {
                panic!(
                    "«{nome}»: la somma di u64::MAX e 1 deve traboccare; se passa, il crate                      `zip` sta normalizzando le dimensioni dichiarate e il ramo va                      riclassificato"
                );
            };
            assert_eq!(
                errore.category,
                plenora_io_model::ErrorCategory::ResourceLimit,
                "«{nome}»: un overflow di conteggio e' un limite, non un formato non valido"
            );
            assert!(
                errore.message.contains(atteso),
                "«{nome}»: atteso «{atteso}», arrivato «{}». Un messaggio generico non                  distinguerebbe i due accumulatori, ed e' il difetto che questo caso esiste                  per escludere",
                errore.message
            );
        }
    }

    /// Una parte enumerabile ma **non apribile** ferma la lettura.
    ///
    /// # Il difetto che questa sonda ha trovato
    ///
    /// La selezione del perimetro passava da `archive.by_index(indice).ok()?`
    /// dentro un `filter_map`: qualunque errore di apertura diventava «parte
    /// ignorata». Una parte con central directory leggibile e header locale
    /// irraggiungibile spariva quindi dal perimetro, e
    /// `valida_riferimenti_cella` restituiva `Ok` -- cioe' **fail-open**, in una
    /// funzione il cui doc-comment promette che «non c'e' un ramo che, non
    /// riuscendo a controllare, prosegua lo stesso».
    ///
    /// Non era una svista di stile: `?` dentro `filter_map` scarta l'errore per
    /// costruzione, e il `by_name` successivo non lo ripara, perche' cerca fra i
    /// nomi **gia' filtrati**.
    /// Un archivio con due parti XML vere, e l'offset dell'header locale di una
    /// di esse **corrotto nel central directory**.
    ///
    /// Costruito con il crate `zip` e poi ritoccato in un campo solo: cosi' il
    /// contenuto e' valido, il central directory resta leggibile, e l'unica
    /// cosa che cambia e' che quella voce non si apre. Costruirlo a mano
    /// avrebbe messo in gioco anche la correttezza della fixture, e un rifiuto
    /// non avrebbe piu' distinto le due cause.
    fn xlsx_con_una_parte_non_apribile(corrompi: bool) -> Vec<u8> {
        let mut buffer = std::io::Cursor::new(Vec::new());
        {
            let mut scrittore = zip::ZipWriter::new(&mut buffer);
            let opzioni: zip::write::FileOptions<'_, ()> = zip::write::FileOptions::default()
                .compression_method(zip::CompressionMethod::Stored);
            for nome in ["xl/worksheets/sheet1.xml", "xl/workbook.xml"] {
                scrittore.start_file(nome, opzioni).unwrap();
                std::io::Write::write_all(&mut scrittore, b"<x r=\"A1\"/>").unwrap();
            }
            scrittore.finish().unwrap();
        }
        let mut byte = buffer.into_inner();
        if !corrompi {
            return byte;
        }

        // La prima voce del central directory: firma `PK\x01\x02`, e il
        // relative offset dell'header locale nei quattro byte a 42.
        let posizione = byte
            .windows(4)
            .position(|finestra| finestra == [0x50, 0x4b, 0x01, 0x02])
            .expect("il central directory esiste");
        byte[posizione + 42..posizione + 46].copy_from_slice(&3_u32.to_le_bytes());
        byte
    }

    #[test]
    fn n1_una_parte_enumerabile_ma_non_apribile_ferma_la_lettura() {
        let dir = tempfile::tempdir().unwrap();
        let opzioni = opzioni_lettura();

        // Il controllo che rende interpretabile il rifiuto: lo stesso archivio
        // **non** corrotto passa.
        let integro = dir.path().join("integro.xlsx");
        std::fs::write(&integro, xlsx_con_una_parte_non_apribile(false)).unwrap();
        valida_riferimenti_cella(&integro, opzioni.budget())
            .expect("con l'offset corretto entrambe le parti si aprono e si ispezionano");

        // La voce ritoccata e' `xl/worksheets/sheet1.xml`, cioe' dentro il
        // perimetro: e' quella che non deve poter sparire in silenzio.
        let rotto = dir.path().join("rotto.xlsx");
        std::fs::write(&rotto, xlsx_con_una_parte_non_apribile(true)).unwrap();

        let Err(errore) = valida_riferimenti_cella(&rotto, opzioni.budget()) else {
            panic!(
                "una parte del perimetro che non si apre deve fermare la lettura; \
                 restituire Ok e' il fail-open che il contratto esclude"
            );
        };
        assert!(
            errore.message.contains("parte XLSX non leggibile"),
            "atteso il rifiuto sull'apertura della parte, arrivato «{}»",
            errore.message
        );
    }

    /// Un archivio con `quante` voci vuote e nomi **realmente distinti**.
    ///
    /// I nomi devono essere unici: `zip` 8.6 conserva le voci in una mappa
    /// indicizzata per nome, e due voci omonime si sovrascrivono invece di
    /// contarsi due volte. Una fixture con nomi ripetuti proverebbe un tetto
    /// piu' basso di quello dichiarato.
    fn xlsx_con_parti(quante: usize) -> Vec<u8> {
        let mut buffer = std::io::Cursor::new(Vec::new());
        {
            let mut scrittore = zip::ZipWriter::new(&mut buffer);
            let opzioni: zip::write::FileOptions<'_, ()> = zip::write::FileOptions::default()
                .compression_method(zip::CompressionMethod::Stored);
            for indice in 0..quante {
                scrittore
                    .start_file(format!("parte{indice}.bin"), opzioni)
                    .unwrap();
            }
            scrittore.finish().unwrap();
        }
        buffer.into_inner()
    }

    /// Il tetto sulle parti conta **i membri dell'archivio**, non le sole parti
    /// XML, e i due confini stanno uno accanto all'altro.
    ///
    /// Il **vecchio** nome della costante diceva «parti XML» mentre il controllo
    /// guarda `archive.len()`, cioe' ogni membro: un contenitore con migliaia di
    /// immagini viene fermato quanto uno con migliaia di fogli. E' il
    /// comportamento voluto -- il tetto difende dall'abuso del contenitore, non
    /// dal numero di fogli -- e la sonda lo fissa perche' un nome non e' una
    /// prova: `MAX_MEMBRI_ARCHIVIO` oggi lo dice, ma potrebbe tornare a mentire.
    #[test]
    fn n1_il_tetto_sulle_parti_conta_i_membri_e_ha_i_due_confini() {
        let dir = tempfile::tempdir().unwrap();
        let opzioni = opzioni_lettura();

        // Nessuna delle voci e' una parte XML del perimetro: se il tetto
        // contasse le sole parti XML, questo archivio passerebbe.
        let al_limite = dir.path().join("al-limite.xlsx");
        std::fs::write(&al_limite, xlsx_con_parti(MAX_MEMBRI_ARCHIVIO)).unwrap();
        valida_riferimenti_cella(&al_limite, opzioni.budget())
            .expect("il confine e' inclusivo: esattamente MAX_MEMBRI_ARCHIVIO membri passano");

        let oltre = dir.path().join("oltre.xlsx");
        std::fs::write(&oltre, xlsx_con_parti(MAX_MEMBRI_ARCHIVIO + 1)).unwrap();
        let Err(errore) = valida_riferimenti_cella(&oltre, opzioni.budget()) else {
            panic!("un membro oltre il tetto deve fermare la lettura");
        };
        assert_eq!(
            errore.category,
            plenora_io_model::ErrorCategory::ResourceLimit,
            "un tetto superato e' un limite, non un formato non valido"
        );
    }

    /// Un `.xlsx` con `righe` x `colonne` celle, piu' l'intestazione.
    ///
    /// Passa dal writer vero invece di costruire byte a mano: qui non si prova
    /// la tolleranza al formato malformato -- quello e' il mestiere delle
    /// fixture di `valida_riferimenti_cella` -- ma il comportamento su un foglio
    /// **valido** e semplicemente troppo grande per la quota.
    fn scrivi_xlsx(percorso: &std::path::Path, righe: u32, colonne: u16) {
        let mut cartella = Workbook::new();
        let foglio = cartella.add_worksheet();
        if colonne > 0 {
            foglio.write_string(0, 0, "geometry").unwrap();
            for colonna in 1..colonne {
                foglio
                    .write_string(0, colonna, format!("c{colonna}"))
                    .unwrap();
            }
            for riga in 1..=righe {
                foglio.write_string(riga, 0, "POINT (1 2)").unwrap();
                for colonna in 1..colonne {
                    foglio.write_string(riga, colonna, "v").unwrap();
                }
            }
        }
        cartella.save(percorso).unwrap();
    }

    /// Opzioni di lettura complete per un foglio con una colonna WKT.
    ///
    /// Il driver esige sia `wkt_column` sia `assume_crs`: senza, il rifiuto
    /// arriverebbe da `resolve_geometry` o dalla fase CRS, e le sonde qui sotto
    /// misurerebbero una guardia diversa da quella che dichiarano.
    fn opzioni_lettura_wkt() -> ReadOptions {
        opzioni_lettura_wkt_con(plenora_io_model::budget::PipelineLimits::default())
    }

    fn opzioni_lettura_wkt_con(limits: plenora_io_model::budget::PipelineLimits) -> ReadOptions {
        opzioni_lettura_con(limits)
            .with_assume_crs("EPSG:4326")
            .with_format_option("wkt_column", "geometry")
    }

    /// I rifiuti che `infer_layout` decide **da se'** e puo' pronunciare.
    ///
    /// Il censimento del gruppo conta venticinque righe, ma solo tre sono
    /// decisioni della funzione: la larghezza oltre la quota, l'altezza oltre la
    /// quota, e il foglio che non produce nemmeno una cella. Tutto il resto e'
    /// propagazione da un helper -- che ha il proprio gruppo, e va provato li',
    /// altrimenti la stessa riga risulterebbe coperta due volte senza che
    /// nessuna delle due prove dica dove -- oppure e' irraggiungibile, e le
    /// quattro irraggiungibilita' hanno la loro sonda subito sotto.
    ///
    /// I due limiti si provano dal `open` del driver e non chiamando
    /// `infer_layout` a mano: la quota arriva da `ReadOptions`, e passare per
    /// l'entry point verifica anche che ci arrivi davvero.
    #[test]
    fn n1_infer_layout_rifiuta_le_due_quote() {
        let dir = tempfile::tempdir().unwrap();

        // Il controllo positivo: lo stesso foglio con quote sufficienti passa.
        // Senza, un `open` che fallisse per qualunque altra ragione farebbe
        // verde la tabella dei rifiuti.
        let normale = dir.path().join("normale.xlsx");
        scrivi_xlsx(&normale, 3, 3);
        XlsDriver
            .open(Source::Path(normale), opzioni_lettura_wkt())
            .expect("tre righe per tre colonne stanno in qualunque quota di default");

        // Larghezza oltre la quota: `--max-columns` a 2 su un foglio da 3.
        let largo = dir.path().join("largo.xlsx");
        scrivi_xlsx(&largo, 2, 3);
        let Err(errore) = XlsDriver.open(
            Source::Path(largo),
            opzioni_lettura_wkt_con(
                plenora_io_model::budget::PipelineLimits::default().with_max_columns(2),
            ),
        ) else {
            panic!("tre colonne oltre una quota di due devono essere rifiutate");
        };
        assert!(
            errore.message.contains("colonne oltre il limite"),
            "atteso il rifiuto sulla larghezza, arrivato «{}»",
            errore.message
        );

        // Altezza oltre la quota: `--max-rows` a 1 su un foglio da 3 righe dati.
        let alto = dir.path().join("alto.xlsx");
        scrivi_xlsx(&alto, 3, 2);
        let Err(errore) = XlsDriver.open(
            Source::Path(alto),
            opzioni_lettura_wkt_con(
                plenora_io_model::budget::PipelineLimits::default().with_max_rows(1),
            ),
        ) else {
            panic!("tre righe oltre una quota di una devono essere rifiutate");
        };
        assert!(
            errore.message.contains("righe oltre il limite"),
            "atteso il rifiuto sull'altezza, arrivato «{}»",
            errore.message
        );
    }

    /// Il foglio senza celle: la guardia esiste ed e' corretta, ma `open` non
    /// la puo' raggiungere.
    ///
    /// # Perche' non e' un rifiuto raggiungibile dall'esterno
    ///
    /// `observed_cells` conta **celle**, non righe: `for_each_dense_row`
    /// attraversa comunque l'intervallo dichiarato dalle dimensioni, quindi la
    /// riga d'intestazione viene sempre visitata, e su quella riga
    /// `resolve_geometry` deve riuscire prima che il flusso arrivi al
    /// controllo. Se il flusso di celle e' vuoto, ogni intestazione e' la
    /// stringa vuota -- `data_to_string(Data::Empty)` la produce -- quindi
    /// l'unico nome di colonna che vi si troverebbe e' `""`. E `""` non
    /// arriva: `wkt_column` e' `ValoreAmmesso::Testo`, che esige testo non
    /// vuoto, e il validatore centrale delle `format_options` lo ferma prima
    /// che il driver apra il file.
    ///
    /// Le due meta' si provano separatamente perche' affermano cose diverse:
    /// la prima che il controllo funziona, la seconda che la strada per
    /// arrivarci e' chiusa altrove. Provare solo la seconda lascerebbe il ramo
    /// non eseguito; provare solo la prima direbbe che e' raggiungibile.
    #[test]
    fn n1_il_foglio_senza_celle_e_fermato_dallo_schema_prima_di_infer_layout() {
        let dir = tempfile::tempdir().unwrap();
        let vuoto = dir.path().join("vuoto.xlsx");
        scrivi_xlsx(&vuoto, 0, 0);

        // Meta' uno: chiamata diretta, con l'unica configurazione che porta
        // fino al controllo. Il tipo e' privato del crate, quindi la prova puo'
        // costruire cio' che l'entry point rifiuta.
        let opzioni = opzioni_lettura();
        let mut cartella: calamine::Xlsx<_> = calamine::open_workbook(&vuoto).unwrap();
        let foglio = cartella.sheet_names().first().cloned().unwrap();
        let mut nomi_colonne = BTreeMap::new();
        nomi_colonne.insert("wkt_column".to_owned(), String::new());
        let esito = infer_layout(
            &mut cartella,
            &foglio,
            &nomi_colonne,
            "EPSG:4326",
            opzioni.cancellation(),
            XlsxQuote::from_read_options(&opzioni),
            opzioni.budget(),
        );
        let Err(errore) = esito else {
            panic!("un foglio che non consegna nemmeno una cella non ha un layout da inferire");
        };
        assert!(
            errore.message.contains("foglio vuoto"),
            "atteso il rifiuto sul foglio vuoto, arrivato «{}»",
            errore.message
        );

        // Meta' due: la stessa configurazione passata da `open` non arriva al
        // driver. Il rifiuto e' dello schema delle opzioni, in fase di
        // validazione, non dell'inferenza.
        let Err(fermato) = XlsDriver.open(
            Source::Path(vuoto),
            opzioni_lettura()
                .with_assume_crs("EPSG:4326")
                .with_format_option("wkt_column", ""),
        ) else {
            panic!("un nome di colonna vuoto non e' un valore ammesso");
        };
        assert_eq!(
            fermato.phase,
            ErrorPhase::Validate,
            "il rifiuto deve venire dalla validazione delle opzioni, non dalla lettura: {}",
            fermato.message
        );
        assert!(
            fermato.message.contains("testo non vuoto"),
            "atteso il rifiuto dello schema sul valore vuoto, arrivato «{}»",
            fermato.message
        );
    }

    /// «intestazione XLSX assente» e «geometria XLSX non configurata» sono
    /// irraggiungibili: `data_row_count` rifiuta prima le dimensioni che
    /// renderebbero vuoto il ciclo.
    ///
    /// Le tre occorrenze -- il `geom.ok_or_else` dentro il ciclo, e i due
    /// `ok_or_else` su `headers` e `geom` dopo -- esistono perche' il tipo e'
    /// `Option`, non perche' un input le produca. `for_each_dense_row` itera su
    /// `bounds.start.0..=bounds.end.0`, e la prima riga visitata assegna
    /// entrambe le variabili oppure propaga l'errore di `resolve_geometry`.
    /// Perche' quell'intervallo sia vuoto servirebbe `start.0 > end.0`, e
    /// `data_row_count` lo rifiuta con «dimensioni XLSX non valide».
    ///
    /// La sonda non copre quelle righe: esegue la **guardia** che le rende
    /// inarrivabili. Se la precedenza cambia -- se le dimensioni invertite
    /// smettessero di essere rifiutate -- diventa rossa, e la
    /// classificazione va rifatta.
    #[test]
    fn n1_le_dimensioni_invertite_precedono_l_intestazione_assente() {
        for invertite in [
            SheetBounds {
                start: (5, 0),
                end: (4, 3),
            },
            SheetBounds {
                start: (0, 5),
                end: (3, 4),
            },
        ] {
            let esito = data_row_count(invertite).and_then(|_| data_row_width(invertite));
            let Err(errore) = esito else {
                panic!(
                    "dimensioni con inizio oltre la fine devono essere rifiutate: start {:?}, end {:?}",
                    invertite.start, invertite.end
                );
            };
            assert!(
                errore.message.contains("dimensioni XLSX non valide"),
                "atteso il rifiuto sulle dimensioni, arrivato «{}»",
                errore.message
            );
        }

        // Il complemento, senza il quale la sonda direbbe solo che qualcosa
        // viene rifiutato: con dimensioni accettate l'intervallo delle righe
        // contiene almeno la riga d'intestazione, che e' l'affermazione da cui
        // dipende l'irraggiungibilita'.
        let accettate = SheetBounds {
            start: (7, 2),
            end: (7, 2),
        };
        assert!(
            data_row_count(accettate).is_ok() && accettate.start.0 <= accettate.end.0,
            "un foglio di una sola cella e' comunque un foglio con una riga da visitare"
        );
    }

    /// «troppe colonne XLSX» e «indice colonna XLSX fuori intervallo» sono
    /// irraggiungibili: la larghezza che `data_row_width` accetta non lascia
    /// spazio ne' al `try_from` ne' all'overflow.
    ///
    /// Le quattro occorrenze -- due nel ciclo delle righe, due nel ciclo che
    /// costruisce lo schema dalle intestazioni -- indicizzano una riga lunga
    /// `data_row_width(bounds)`. Quella funzione calcola `end.1 - start.1 + 1`
    /// in `u32`, quindi rifiuta gia' la riga larga quanto l'intero spazio delle
    /// colonne: la larghezza massima accettata e' `u32::MAX`, l'offset massimo
    /// `u32::MAX - 1`, e `start.1 + offset` non supera mai `end.1`.
    ///
    /// Gli estremi del formato sono verificati invece che argomentati: sono il
    /// caso peggiore rappresentabile, e se passano passa ogni foglio.
    #[test]
    fn n1_la_larghezza_accettata_precede_i_due_rifiuti_sull_indice_di_colonna() {
        assert!(
            data_row_width(SheetBounds {
                start: (0, 0),
                end: (0, u32::MAX),
            })
            .is_err(),
            "la riga larga quanto l'intero spazio delle colonne non e' rappresentabile"
        );

        for estreme in [
            SheetBounds {
                start: (0, 0),
                end: (0, u32::MAX - 1),
            },
            SheetBounds {
                start: (0, u32::MAX - 3),
                end: (0, u32::MAX),
            },
        ] {
            let larghezza = data_row_width(estreme).expect("le dimensioni sono valide");
            let offset_massimo = larghezza - 1;
            let Ok(offset) = u32::try_from(offset_massimo) else {
                panic!("l'offset massimo di una riga larga {larghezza} deve stare in u32");
            };
            assert!(
                estreme.start.1.checked_add(offset).is_some(),
                "start.1 + offset non puo' traboccare: e' al piu' end.1, che e' un u32"
            );
        }
    }

    /// `classe_xlsx` traduce ogni variante che sappiamo costruire.
    ///
    /// Sette varianti di `XlsxError` **portano il nome del foglio come dato**,
    /// e il `Display` della dipendenza lo stampava: dodici percorsi di
    /// scrittura lo facevano uscire nel messaggio pubblico. La classe li tiene
    /// distinti senza far uscire nulla.
    ///
    /// Il test esiste per la lezione di `classe_sqlite`: la copertura
    /// differenziale del checkpoint su `effc4ab` ha trovato quel `match` mai
    /// attraversato. Un vocabolario senza test e' una tabella di traduzione di
    /// cui nessuno ha mai letto una riga.
    #[test]
    fn la_classe_xlsx_traduce_ogni_variante_costruibile() {
        use rust_xlsxwriter::XlsxError as E;

        let campioni: Vec<(E, &str)> = vec![
            (
                E::RowColumnLimitError,
                "riga o colonna oltre il limite del formato",
            ),
            (
                E::SheetnameCannotBeBlank("Foglio segreto".to_owned()),
                "nome del foglio vuoto",
            ),
            (
                E::SheetnameLengthExceeded("Foglio segreto".to_owned()),
                "nome del foglio troppo lungo",
            ),
            (
                E::SheetnameReused("Foglio segreto".to_owned()),
                "nome del foglio gia' usato",
            ),
            (
                E::SheetnameContainsInvalidCharacter("Foglio segreto".to_owned()),
                "nome del foglio con caratteri non ammessi",
            ),
            (
                E::SheetnameStartsOrEndsWithApostrophe("Foglio segreto".to_owned()),
                "nome del foglio delimitato da apostrofi",
            ),
            (
                E::MaxStringLengthExceeded,
                "stringa oltre il limite del formato",
            ),
            (
                E::UnknownWorksheetNameOrIndex("Foglio segreto".to_owned()),
                "foglio inesistente",
            ),
            (
                E::ParameterError("dettaglio".to_owned()),
                "parametro non valido",
            ),
        ];

        let mut visti = std::collections::BTreeSet::new();
        for (errore, atteso) in &campioni {
            assert_eq!(classe_xlsx(errore), *atteso);
            visti.insert(*atteso);
        }
        assert_eq!(
            visti.len(),
            campioni.len(),
            "due varianti distinte non devono avere la stessa classe"
        );

        // Il nome del foglio non esce: e' il punto dell'intero cambiamento.
        let errore = xls_err(E::SheetnameReused("Foglio segreto".to_owned()));
        assert_eq!(errore.driver.as_deref(), Some("xls"));
        assert!(errore.message.contains("nome del foglio gia' usato"));
        assert!(
            !errore.message.contains("Foglio segreto"),
            "il nome del foglio non deve comparire nel messaggio: {}",
            errore.message
        );
    }

    /// Opzioni di lettura sul modello unificato.
    ///
    /// Da S4.d il percorso di lettura vive interamente li': la memoria dei
    /// batch e' una `InternalMemoryLease`, che esiste solo dentro un
    /// `PipelineContext`. `opzioni_lettura()` costruisce ancora il ramo
    /// legacy — sparira' in S4.e — e con quello `open` fallisce chiuso.
    /// La capability `hostile_input_hardened`, provata dove S12 la sposta.
    ///
    /// Non e' il cap in byte: quello esisteva prima del parser progressivo e
    /// scatta **prima** di deserializzare, quindi un test che lo esercita
    /// resterebbe verde anche rimettendo il parser vecchio. Prova nulla di
    /// questo lotto.
    ///
    /// Qui l'input sta comodamente sotto il cap in byte, e a fermarlo e' il
    /// tetto sui **componenti** -- l'unita' che solo un'analisi che addebita
    /// mentre consuma puo' applicare. Le tre condizioni stanno insieme
    /// apposta:
    ///
    ///   * con il tetto stretto il rifiuto e' esattamente `LimitExceeded`;
    ///   * con il default lo stesso identico input passa, quindi il rifiuto
    ///     viene dal tetto e non dall'input;
    ///   * l'input e' molto piu' corto del cap in byte, che percio' non
    ///     c'entra.
    ///
    /// E' la prova che `check_capability_input_ostile.py` esegue per questo
    /// driver: cancellarla, rinominarla o indebolirla rende rossa la
    /// capability nel catalogo.
    #[test]
    fn la_cella_wkt_e_rifiutata_per_componenti_sotto_il_cap_in_byte() {
        const COMPONENTI: usize = 5;
        let wkt = "LINESTRING (0 0,1 1,2 2,3 3,4 4)";

        let dir = tempfile::tempdir().expect("tempdir");
        let output = dir.path().join("linea.xlsx");
        let mut workbook = Workbook::new();
        let sheet = workbook.add_worksheet();
        sheet.write_string(0, 0, "geometry").expect("intestazione");
        sheet.write_string(1, 0, wkt).expect("cella");
        workbook.save(&output).expect("salvataggio");

        assert!(
            wkt.len() < plenora_io_model::limits::WkbLimits::default().max_cell_bytes / 1_000,
            "l'input deve stare comodamente sotto il cap in byte"
        );

        let opzioni = |componenti: usize| {
            opzioni_lettura_con(
                plenora_io_model::budget::PipelineLimits::default()
                    .with_max_wkb_components(componenti),
            )
            .with_assume_crs("EPSG:4326")
            .with_format_option("wkt_column", "geometry")
        };

        assert!(
            XlsDriver
                .open(Source::Path(output.clone()), opzioni(COMPONENTI))
                .is_ok(),
            "con {COMPONENTI} componenti di tetto la stessa cella deve passare"
        );

        match XlsDriver.open(Source::Path(output), opzioni(COMPONENTI - 1)) {
            Err(errore) => assert_eq!(
                errore.code,
                plenora_io_model::IoErrorCode::LimitExceeded,
                "il rifiuto deve venire dal tetto sui componenti: {}",
                errore.message
            ),
            Ok(_) => panic!("una cella oltre il tetto sui componenti deve fallire"),
        }
    }

    /// L'inferenza XLSX usa il tetto per cella **configurato**.
    ///
    /// Fino a S5 `encode_geometry_cell` passava
    /// `WkbLimits::default().max_cell_bytes` — 64 MiB — quindi
    /// `--max-wkb-cell-bytes` non arrivava al parsing WKT, e una cella oltre
    /// la soglia richiesta veniva parsata comunque.
    #[test]
    fn inference_uses_configured_wkt_cell_bytes_not_default() {
        let dir = tempfile::tempdir().expect("tempdir");
        let output = dir.path().join("punto.xlsx");
        let mut workbook = Workbook::new();
        let sheet = workbook.add_worksheet();
        sheet.write_string(0, 0, "geometry").expect("intestazione");
        sheet.write_string(1, 0, "POINT (1 2)").expect("cella");
        workbook.save(&output).expect("salvataggio");

        let opzioni = |byte: usize| {
            opzioni_lettura_con(
                plenora_io_model::budget::PipelineLimits::default().with_max_wkb_cell_bytes(byte),
            )
            .with_assume_crs("EPSG:4326")
            .with_format_option("wkt_column", "geometry")
        };

        assert!(
            XlsDriver
                .open(Source::Path(output.clone()), opzioni(64))
                .is_ok(),
            "una cella dentro il tetto configurato deve passare"
        );

        let esito = XlsDriver.open(Source::Path(output), opzioni(4));
        assert!(
            matches!(
                esito,
                Err(ref errore) if errore.code == plenora_io_model::IoErrorCode::LimitExceeded
                    || errore.message.contains("limite")
            ),
            "una cella oltre il tetto configurato deve fallire"
        );
        assert!(
            64 < plenora_io_model::limits::WkbLimits::default().max_cell_bytes,
            "la soglia del test deve stare sotto il default"
        );
    }

    /// Il testo della cella sta nel tetto, il WKB codificato no.
    ///
    /// `POINT (1 2)` occupa undici caratteri e ventuno byte in WKB: due `f64`
    /// costano sedici byte da soli. Il controllo sul testo, che XLSX fa prima
    /// di costruire l'AST, non e' quindi una maggiorazione della dimensione
    /// codificata, e fino a S5.1 il buffer cresceva oltre il tetto prima che
    /// qualcuno se ne accorgesse.
    #[test]
    fn il_wkb_codificato_non_supera_il_tetto_anche_se_il_testo_ci_sta() {
        // Fra la lunghezza del testo (11) e quella del WKB (21).
        const SOGLIA: usize = 15;

        let dir = tempfile::tempdir().expect("tempdir");
        let output = dir.path().join("punto-stretto.xlsx");
        let mut workbook = Workbook::new();
        let sheet = workbook.add_worksheet();
        sheet.write_string(0, 0, "geometry").expect("intestazione");
        sheet.write_string(1, 0, "POINT (1 2)").expect("cella");
        workbook.save(&output).expect("salvataggio");
        assert!(
            "POINT (1 2)".len() <= SOGLIA,
            "la premessa: il testo deve stare nel tetto"
        );

        let opzioni = opzioni_lettura_con(
            plenora_io_model::budget::PipelineLimits::default().with_max_wkb_cell_bytes(SOGLIA),
        )
        .with_assume_crs("EPSG:4326")
        .with_format_option("wkt_column", "geometry");

        let messaggio = XlsDriver
            .open(Source::Path(output), opzioni)
            .err()
            .map(|errore| errore.message);
        assert!(
            matches!(messaggio, Some(ref testo) if testo.contains("oltre il limite")),
            "la codifica WKB deve fermarsi al tetto, non il parsing del testo: {messaggio:?}"
        );
    }

    fn opzioni_lettura_con(limits: plenora_io_model::budget::PipelineLimits) -> ReadOptions {
        match plenora_io_model::budget::PipelineBudget::builder()
            .limits(limits)
            .build()
        {
            Ok(bundle) => ReadOptions::from_read_parts(bundle.into_read_parts()),
            Err(error) => unreachable!("bundle di test non costruibile: {error:?}"),
        }
    }

    /// Opzioni di scrittura sul modello unificato.
    ///
    /// `opzioni_scrittura()` non esiste piu' (S4.e): le opzioni portano un
    /// `OperationBudget`, che nasce da una costruzione che puo' fallire.
    fn opzioni_scrittura() -> WriteOptions {
        match plenora_io_model::budget::PipelineBudget::builder().build() {
            Ok(bundle) => WriteOptions::from_write_parts(bundle.into_write_parts()),
            Err(error) => unreachable!("bundle di test non costruibile: {error:?}"),
        }
    }

    fn opzioni_lettura() -> ReadOptions {
        match plenora_io_model::budget::PipelineBudget::builder().build() {
            Ok(bundle) => ReadOptions::from_read_parts(bundle.into_read_parts()),
            Err(error) => unreachable!("bundle di test non costruibile: {error:?}"),
        }
    }

    use plenora_io_core::request::{BatchTarget, ProjectionMode, ReadScope};
    use plenora_io_core::WriteLayer;

    /// Un `.xlsx` con riferimenti oltre i limiti del formato viene rifiutato
    /// **prima** che `calamine` lo veda, quindi il panico non avviene (FZ-0).
    ///
    /// I due input sono complementari:
    ///
    /// * `riferimento-cella-oltre-u32.xlsx` e' l'input che ha prodotto il
    ///   finding nello smoke del 2026-08-17, conservato intatto;
    /// * `riferimento-cella-nove-lettere.xlsx` e' costruito da zero con CRC
    ///   corretti e un riferimento `AAAAAAAAA1`. Serve perche' il primo ha il
    ///   CRC rotto dalla mutazione e verrebbe fermato gia' da quello: senza il
    ///   secondo, il controllo sui limiti del formato non sarebbe osservato da
    ///   nessun test.
    ///
    /// La verifica e' che l'errore **non** venga dalla barriera: se il
    /// messaggio parlasse di panico, vorrebbe dire che `calamine` e' stato
    /// raggiunto lo stesso e che la prevalidazione non serve a niente.
    #[test]
    fn un_riferimento_oltre_i_limiti_del_formato_e_rifiutato_prima_di_calamine() {
        let semi = [
            "riferimento-cella-oltre-u32.xlsx",
            "riferimento-cella-nove-lettere.xlsx",
        ];
        // Le stesse due dichiarazioni di geometria che usa il fuzz target: il
        // rifiuto precede la geometria, quindi vale per entrambe.
        let dichiarazioni: [Vec<(&str, &str)>; 2] = [
            vec![("wkt_column", "geometry")],
            vec![("x_column", "x"), ("y_column", "y")],
        ];

        for nome in semi {
            let percorso_seme = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../fuzz/seeds/xlsx_reader")
                .join(nome);
            assert!(
                percorso_seme.is_file(),
                "seme assente: {}",
                percorso_seme.display()
            );

            for dichiarazione in &dichiarazioni {
                let mut opzioni = opzioni_lettura().with_assume_crs("EPSG:4326");
                for (chiave, valore) in dichiarazione {
                    opzioni = opzioni.with_format_option(*chiave, *valore);
                }

                // Nessun panico: senza la prevalidazione questa riga abbatte
                // il processo di test invece di restituire.
                let esito = XlsDriver.open(Source::Path(percorso_seme.clone()), opzioni);

                // Nessun dataset parziale: `open` non consegna un handle a
                // meta'. La prova e' che non ne consegna affatto uno.
                let Err(errore) = esito else {
                    panic!("{nome} {dichiarazione:?}: l'input doveva essere rifiutato")
                };

                // Errore tipizzato, fase Read.
                assert_eq!(errore.code, plenora_io_model::IoErrorCode::Format);
                assert_eq!(
                    errore.category,
                    plenora_io_model::ErrorCategory::DataMapping
                );
                assert_eq!(errore.phase, ErrorPhase::Read);
                assert_eq!(errore.driver.as_deref(), Some("xls"));

                // Il panico e' *impedito*, non catturato: se il messaggio
                // venisse dalla barriera, `calamine` sarebbe stato raggiunto.
                assert!(
                    !errore.message.contains("in panico"),
                    "{nome}: il rifiuto deve precedere calamine: {errore}"
                );

                // Messaggio pubblico redatto: nessun percorso, nessun valore
                // dell'input.
                let messaggio = errore.message.as_str();
                for vietato in ["riferimento-cella", "fuzz/seeds", "Bncasufw", "AAAAAAAAA"] {
                    assert!(
                        !messaggio.contains(vietato),
                        "il messaggio pubblico non deve contenere {vietato:?}: {messaggio}"
                    );
                }
            }
        }
    }

    /// Il seme conforme continua a essere letto: la prevalidazione non rifiuta
    /// cio' che il formato ammette.
    ///
    /// Senza questo, un controllo troppo severo — per esempio uno che
    /// rifiutasse ogni riferimento con lettere — passerebbe il test sopra e
    /// romperebbe ogni XLSX reale, senza che nessuno se ne accorgesse qui.
    #[test]
    fn un_xlsx_conforme_supera_la_prevalidazione() {
        let seme = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fuzz/seeds/xlsx_reader/minimal.xlsx");
        assert!(seme.is_file(), "seme assente: {}", seme.display());
        let opzioni = opzioni_lettura();
        valida_riferimenti_cella(&seme, opzioni.budget())
            .expect("un workbook conforme non deve essere rifiutato");
    }

    /// La barriera resta come difesa in profondita' ed e' verificata **da
    /// sola**, senza dipendere da un input che faccia ancora panicare la
    /// libreria.
    ///
    /// E' la forma giusta dopo FZ-0: la prevalidazione impedisce il panico
    /// noto, ma non puo' dimostrare che nessun altro percorso di `calamine`
    /// ne produca uno. Se domani ne comparisse un altro, la barriera lo
    /// converte comunque in errore tipizzato — e questo test lo dimostra
    /// invece di dedurlo.
    #[test]
    fn la_barriera_converte_un_panico_di_calamine_in_errore_tipizzato() {
        let errore = leggendo_calamine::<()>(|| panic!("panico simulato della libreria"))
            .expect_err("un panico deve diventare un errore");
        assert_eq!(errore.code, plenora_io_model::IoErrorCode::Format);
        assert_eq!(
            errore.category,
            plenora_io_model::ErrorCategory::DataMapping
        );
        assert_eq!(errore.phase, ErrorPhase::Read);
        assert_eq!(errore.driver.as_deref(), Some("xls"));
        // Messaggio statico curato: niente testo del panico, niente impronta.
        assert!(
            !errore.message.contains("panico simulato"),
            "il testo del panico non deve raggiungere il messaggio pubblico: {errore}"
        );
        assert_eq!(errore.message, MESSAGGIO_PANICO_CALAMINE);

        // Percorso normale: la barriera non altera nulla.
        assert_eq!(leggendo_calamine(|| Ok(7_u8)).unwrap(), 7);
    }

    #[test]
    fn coordinate_cells_fail_closed_on_invalid_or_lossy_values() {
        assert!(coordinate_cell(Some(&Data::String("not-a-number".to_owned())), "X").is_err());
        assert!(coordinate_cell(Some(&Data::Float(f64::INFINITY)), "X").is_err());
        assert!(coordinate_cell(Some(&Data::Int((1_i64 << 53) + 1)), "X").is_err());
        assert_eq!(coordinate_cell(Some(&Data::Empty), "X").unwrap(), None);
        assert_eq!(
            coordinate_cell(Some(&Data::String(" 12.5 ".to_owned())), "X").unwrap(),
            Some(12.5)
        );
    }

    // --- ASSURANCE-N1, tranche 1 -------------------------------------------
    //
    // I rami negativi di `open`, `create` e `validate_archive_ratio`: mai
    // eseguiti da nulla — ne' dai test, ne' dal replay — e preesistenti a S9.
    //
    // La forma e' quella indicata dal registro: **la classe di equivalenza
    // della precondizione**. Non serve un file enorme per superare un tetto,
    // serve un tetto stretto e un file normale; non serve un `.xls` vero per
    // provare che non e' instradato, serve un nome con quell'estensione.

    /// `.xls` non e' instradato in lettura, e il rifiuto e' una capability.
    ///
    /// La distinzione conta: un `Format` direbbe «il file e' rotto», e chi
    /// legge cercherebbe un errore nel proprio dato invece di convertirlo.
    #[test]
    fn n1_open_rifiuta_xls_come_capability_non_come_formato() {
        let dir = tempfile::tempdir().unwrap();
        let percorso = dir.path().join("storico.xls");
        std::fs::write(&percorso, b"non importa: l'estensione basta").unwrap();

        let esito = XlsDriver.open(Source::Path(percorso), opzioni_lettura());
        let Err(errore) = esito else {
            panic!(".xls non deve essere aperto da questo driver");
        };
        assert_eq!(
            errore.category,
            plenora_io_model::ErrorCategory::Unsupported,
            "e' una capability mancante, non un file malformato"
        );
        assert!(
            errore.message.contains(".xls"),
            "il messaggio deve dire quale estensione: {}",
            errore.message
        );
    }

    /// Un XLSX con geometria senza `assume_crs` e' rifiutato in fase CRS.
    #[test]
    fn n1_open_esige_assume_crs_quando_c_e_geometria() {
        let dir = tempfile::tempdir().unwrap();
        let percorso = dir.path().join("senza-crs.xlsx");
        let mut cartella = Workbook::new();
        let foglio = cartella.add_worksheet();
        foglio.write_string(0, 0, "geometry").unwrap();
        foglio.write_string(1, 0, "POINT (1 2)").unwrap();
        cartella.save(&percorso).unwrap();

        // Le stesse opzioni **senza** `with_assume_crs`: e' l'unica differenza
        // rispetto al caso che passa, ed e' cio' che rende il test una prova
        // del ramo e non di un guasto qualunque.
        let esito = XlsDriver.open(
            Source::Path(percorso.clone()),
            opzioni_lettura().with_format_option("wkt_column", "geometry"),
        );
        let Err(errore) = esito else {
            panic!("senza CRS dichiarato la geometria non e' interpretabile");
        };
        assert_eq!(errore.category, plenora_io_model::ErrorCategory::Crs);

        // Controprova: con il CRS lo stesso file si apre. Senza, il test
        // proverebbe soltanto che *qualcosa* fallisce.
        let esito = XlsDriver.open(
            Source::Path(percorso.clone()),
            opzioni_lettura()
                .with_format_option("wkt_column", "geometry")
                .with_assume_crs("EPSG:4326"),
        );
        assert!(
            esito.is_ok(),
            "con `assume_crs` lo stesso file deve aprirsi"
        );

        // E il foglio **dichiarato** invece che dedotto: e' l'altro ramo del
        // `match` su `format_options["sheet"]`, che nessun test toccava.
        // `rust_xlsxwriter` nomina il primo foglio «Sheet1».
        let esito = XlsDriver.open(
            Source::Path(percorso),
            opzioni_lettura()
                .with_format_option("wkt_column", "geometry")
                .with_format_option("sheet", "Sheet1")
                .with_assume_crs("EPSG:4326"),
        );
        assert!(
            esito.is_ok(),
            "il foglio dichiarato per nome deve essere accettato"
        );
    }

    /// La destinazione senza estensione `.xlsx` e' rifiutata.
    ///
    /// Va provato **dopo** aver escluso il conflitto di destinazione: nel
    /// codice quel controllo viene prima, e un file preesistente
    /// maschererebbe il ramo sotto esame.
    #[test]
    fn n1_create_rifiuta_una_destinazione_senza_estensione_xlsx() {
        let dir = tempfile::tempdir().unwrap();
        let uscita = dir.path().join("uscita.ods");
        assert!(
            !uscita.exists(),
            "la premessa: nessun conflitto di destinazione"
        );

        let schema = Arc::new(Schema::new(vec![Field::new(
            "valore",
            arrow_schema::DataType::Utf8,
            true,
        )]));
        let piano = WritePlan {
            layers: vec![WriteLayer {
                name: "foglio".to_owned(),
                contract: DataContract::new(schema, None),
            }],
        };

        let errore = XlsDriver
            .create(Sink::Path(uscita.clone()), &piano, &opzioni_scrittura())
            .err()
            .expect("un'estensione diversa da .xlsx va rifiutata");
        assert_eq!(
            errore.category,
            plenora_io_model::ErrorCategory::Unsupported
        );
        assert!(!uscita.exists(), "un rifiuto non lascia destinazione");
    }

    /// Un piano senza esattamente un layer e' fermato **prima** di `create`.
    ///
    /// La prima stesura di questo test si chiamava «`create` rifiuta un piano
    /// che non ha esattamente un layer» e passava — ma la misura di copertura
    /// ha mostrato che il ramo di `create` **restava scoperto**: `validate_write`
    /// nel core ferma entrambe le classi prima, e l'asserzione sulla sola
    /// categoria `Unsupported` era soddisfatta da un errore diverso.
    ///
    /// Era un test verde che provava un'altra cosa. Ora prova quella giusta, ed
    /// e' un contratto piu' forte: fissa la **precedenza**. Il ramo di `create`
    /// resta difensivo, e questo test e' la ragione per cui possiamo dirlo
    /// invece di supporlo.
    #[test]
    fn n1_un_piano_senza_un_solo_layer_e_fermato_prima_di_create() {
        let dir = tempfile::tempdir().unwrap();
        let schema = Arc::new(Schema::new(vec![Field::new(
            "valore",
            arrow_schema::DataType::Utf8,
            true,
        )]));
        let strato = |nome: &str| WriteLayer {
            name: nome.to_owned(),
            contract: DataContract::new(Arc::clone(&schema), None),
        };

        // Le due classi di equivalenza della precondizione `len() != 1`:
        // sotto e sopra. Provarne una sola lascerebbe l'altra a nessuno.
        for (nome, strati) in [
            ("nessun-layer", vec![]),
            ("due-layer", vec![strato("primo"), strato("secondo")]),
        ] {
            let uscita = dir.path().join(format!("{nome}.xlsx"));
            let piano = WritePlan { layers: strati };
            let Err(errore) =
                XlsDriver.create(Sink::Path(uscita.clone()), &piano, &opzioni_scrittura())
            else {
                panic!("{nome}: il piano va rifiutato");
            };
            assert_eq!(
                errore.category,
                plenora_io_model::ErrorCategory::Unsupported,
                "{nome}"
            );
            // `Capability` e non `Unsupported` come codice: e' la firma di
            // `validate_write`, ed e' cio' che dimostra **chi** ha rifiutato.
            // Senza questa riga il test tornerebbe a essere soddisfatto da
            // qualunque rifiuto.
            assert_eq!(
                errore.code,
                plenora_io_model::IoErrorCode::Capability,
                "{nome}: il rifiuto deve venire da validate_write, non da create"
            );
            assert!(
                !uscita.exists(),
                "{nome}: un rifiuto non lascia destinazione"
            );
        }
    }

    /// Un `geometry_encoding` non ammesso e' fermato dalla validazione delle
    /// opzioni, non da `create`.
    ///
    /// Il commento nel codice lo dichiarava «difensivo». Questo test lo
    /// **misura**: l'errore che esce e' quello del validatore delle opzioni,
    /// con il token dell'opzione rifiutata e l'elenco degli ammessi. Il ramo
    /// dentro `create` resta percio' irraggiungibile dall'API pubblica.
    #[test]
    fn n1_un_geometry_encoding_non_ammesso_e_fermato_prima_di_create() {
        let dir = tempfile::tempdir().unwrap();
        let uscita = dir.path().join("encoding.xlsx");
        let schema = Arc::new(Schema::new(vec![Field::new(
            "valore",
            arrow_schema::DataType::Utf8,
            true,
        )]));
        let piano = WritePlan {
            layers: vec![WriteLayer {
                name: "foglio".to_owned(),
                contract: DataContract::new(schema, None),
            }],
        };
        let mut opzioni = opzioni_scrittura();
        opzioni
            .format_options
            .insert("geometry_encoding".to_owned(), "WKB".to_owned());

        let errore = XlsDriver
            .create(Sink::Path(uscita.clone()), &piano, &opzioni)
            .err()
            .expect("un encoding non ammesso va rifiutato");

        assert_eq!(
            errore.category,
            plenora_io_model::ErrorCategory::InvalidConfiguration,
            "e' una configurazione non valida, non una capability mancante"
        );
        // Il messaggio del validatore enumera gli ammessi: e' la firma che
        // distingue **chi** ha rifiutato, e senza di essa il test sarebbe
        // soddisfatto anche dal ramo difensivo di `create`.
        assert!(
            errore.message.contains("wkt") && errore.message.contains("xy"),
            "il rifiuto deve venire dal validatore delle opzioni: {}",
            errore.message
        );
        assert!(!uscita.exists(), "un rifiuto non lascia destinazione");
    }

    /// Il calcolo del rapporto di decompressione non puo' andare in overflow.
    ///
    /// Non serve un archivio enorme: serve un **moltiplicatore** enorme.
    /// `compressed.checked_mul(maximum_ratio)` con `ratio` vicino a `u64::MAX`
    /// trabocca su qualunque file non vuoto, ed e' la classe di equivalenza
    /// della precondizione — non un caso patologico costruito ad arte.
    #[test]
    fn n1_il_rapporto_di_decompressione_non_trabocca_in_silenzio() {
        let dir = tempfile::tempdir().unwrap();
        let percorso = dir.path().join("overflow.xlsx");
        let mut cartella = Workbook::new();
        let foglio = cartella.add_worksheet();
        foglio.write_string(0, 0, "valore").unwrap();
        foglio.write_string(1, 0, "uno").unwrap();
        cartella.save(&percorso).unwrap();

        let esito = XlsDriver.open(
            Source::Path(percorso),
            opzioni_lettura_con(
                plenora_io_model::budget::PipelineLimits::default()
                    .with_decompression_ratio(u64::MAX),
            ),
        );
        let Err(errore) = esito else {
            panic!("il prodotto deve traboccare e fallire chiuso");
        };
        assert_eq!(
            errore.category,
            plenora_io_model::ErrorCategory::ResourceLimit,
            "un overflow di calcolo e' un limite, non un formato non valido"
        );
        assert!(
            errore.message.contains("overflow"),
            "il messaggio deve dire che si tratta di un overflow: {}",
            errore.message
        );
    }

    #[test]
    fn existing_destination_precedes_unsupported_xlsx_extension() {
        let dir = tempfile::tempdir().unwrap();
        let output = dir.path().join("existing.unsupported");
        std::fs::write(&output, b"sentinel").unwrap();
        let schema = Arc::new(Schema::new(vec![Field::new(
            "value",
            arrow_schema::DataType::Utf8,
            true,
        )]));
        let plan = WritePlan {
            layers: vec![WriteLayer {
                name: "sheet".to_owned(),
                contract: DataContract::new(schema, None),
            }],
        };

        let error = XlsDriver
            .create(Sink::Path(output.clone()), &plan, &opzioni_scrittura())
            .err()
            .expect("la destinazione esistente deve essere rifiutata");
        assert_eq!(error.code, plenora_io_model::IoErrorCode::OutputExists);
        assert_eq!(error.category, plenora_io_model::ErrorCategory::Conflict);
        assert_eq!(std::fs::read(output).unwrap(), b"sentinel");
    }

    #[test]
    fn write_then_read_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("out.xlsx");
        let wkb = encode_wkb(&wkt("POINT (12.5 45.9)").unwrap(), WkbFlavor::Iso).unwrap();
        let schema: SchemaRef = Arc::new(Schema::new(vec![
            geometry_field(GEOMETRY, "EPSG:4326"),
            Field::new("nome", arrow_schema::DataType::Utf8, true),
            Field::new("pop", arrow_schema::DataType::Int64, true),
        ]));
        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![
                Arc::new(BinaryArray::from(vec![Some(wkb.as_slice())])),
                Arc::new(arrow_array::StringArray::from(vec!["Roma"])),
                Arc::new(arrow_array::Int64Array::from(vec![2_800_000i64])),
            ],
        )
        .unwrap();

        let driver = XlsDriver;
        let plan = WritePlan {
            layers: vec![WriteLayer {
                name: "l".to_owned(),
                contract: DataContract {
                    schema,
                    geometry: None,
                },
            }],
        };
        let mut w = driver
            .create(Sink::Path(out.clone()), &plan, &opzioni_scrittura())
            .unwrap();
        w.write(&batch).unwrap();
        w.finish().unwrap();

        let ropts = opzioni_lettura()
            .with_assume_crs("EPSG:4326")
            .with_format_option("wkt_column", "geometry");
        let ds = driver.open(Source::Path(out), ropts).unwrap();
        let mut r = ds
            .open_layer_reader(&ReadRequest {
                layer: LayerId(0),
                projected_fields: None,
                projection_mode: ProjectionMode::BestEffort,
                pruning_predicate: None,
                spatial_pruning_hint: None,
                scope: ReadScope::default(),
                batch_target: BatchTarget::default(),
                cancellation: CancellationToken::default(),
            })
            .unwrap();
        let rb = r.next_batch().unwrap().unwrap();
        assert_eq!(rb.num_rows(), 1);
        let nome = rb
            .column_by_name("nome")
            .unwrap()
            .as_any()
            .downcast_ref::<arrow_array::StringArray>()
            .unwrap();
        assert_eq!(nome.value(0), "Roma");
    }

    #[test]
    fn xlsx_reader_emits_bounded_batches_and_preserves_sparse_rows() {
        let dir = tempfile::tempdir().unwrap();
        let output = dir.path().join("sparse.xlsx");
        let mut workbook = Workbook::new();
        let sheet = workbook.add_worksheet();
        sheet.write_string(0, 0, "name").unwrap();
        sheet.write_string(0, 1, "geometry").unwrap();
        sheet.write_string(1, 0, "first").unwrap();
        sheet.write_string(1, 1, "POINT (1 2)").unwrap();
        sheet.write_string(3, 0, "third").unwrap();
        sheet.write_string(3, 1, "POINT (3 4)").unwrap();
        workbook.save(&output).unwrap();

        let driver = XlsDriver;
        let dataset = driver
            .open(
                Source::Path(output),
                opzioni_lettura()
                    .with_assume_crs("EPSG:4326")
                    .with_format_option("wkt_column", "geometry"),
            )
            .unwrap();
        let mut reader = dataset
            .open_layer_reader(&ReadRequest {
                layer: LayerId(0),
                projected_fields: None,
                projection_mode: ProjectionMode::BestEffort,
                pruning_predicate: None,
                spatial_pruning_hint: None,
                scope: ReadScope::default(),
                batch_target: BatchTarget {
                    target_bytes: 1024,
                    max_rows: 1,
                },
                cancellation: CancellationToken::default(),
            })
            .unwrap();

        let first = reader.next_batch().unwrap().unwrap();
        let empty = reader.next_batch().unwrap().unwrap();
        let third = reader.next_batch().unwrap().unwrap();
        assert_eq!(
            [first.num_rows(), empty.num_rows(), third.num_rows()],
            [1, 1, 1]
        );
        assert!(empty
            .column(0)
            .as_any()
            .downcast_ref::<BinaryArray>()
            .unwrap()
            .is_null(0));
        assert!(empty
            .column_by_name("name")
            .unwrap()
            .as_any()
            .downcast_ref::<arrow_array::StringArray>()
            .unwrap()
            .is_null(0));
        assert!(reader.next_batch().unwrap().is_none());
    }

    #[test]
    fn xlsx_reader_stops_after_cancellation_between_batches() {
        let dir = tempfile::tempdir().unwrap();
        let output = dir.path().join("cancel.xlsx");
        let mut workbook = Workbook::new();
        let sheet = workbook.add_worksheet();
        sheet.write_string(0, 0, "geometry").unwrap();
        for row in 1..=4 {
            sheet
                .write_string(row, 0, format!("POINT ({row} {row})"))
                .unwrap();
        }
        workbook.save(&output).unwrap();

        let driver = XlsDriver;
        let dataset = driver
            .open(
                Source::Path(output),
                opzioni_lettura()
                    .with_assume_crs("EPSG:4326")
                    .with_format_option("wkt_column", "geometry"),
            )
            .unwrap();
        let cancellation = CancellationToken::new();
        let mut reader = dataset
            .open_layer_reader(&ReadRequest {
                layer: LayerId(0),
                projected_fields: None,
                projection_mode: ProjectionMode::BestEffort,
                pruning_predicate: None,
                spatial_pruning_hint: None,
                scope: ReadScope::default(),
                batch_target: BatchTarget {
                    target_bytes: 1024,
                    max_rows: 1,
                },
                cancellation: cancellation.clone(),
            })
            .unwrap();
        assert_eq!(reader.next_batch().unwrap().unwrap().num_rows(), 1);
        cancellation.cancel();
        assert!(reader.next_batch().is_err());
    }

    #[test]
    fn xlsx_spool_is_bounded_by_the_input_byte_limit() {
        let dir = tempfile::tempdir().unwrap();
        let output = dir.path().join("bounded.xlsx");
        let mut workbook = Workbook::new();
        let sheet = workbook.add_worksheet();
        sheet.write_string(0, 0, "name").unwrap();
        sheet.write_string(0, 1, "geometry").unwrap();
        let repeated = "x".repeat(1_000);
        for row in 1..=100 {
            sheet.write_string(row, 0, &repeated).unwrap();
            sheet.write_string(row, 1, "POINT (1 2)").unwrap();
        }
        workbook.save(&output).unwrap();
        let input_bytes = std::fs::metadata(&output).unwrap().len();

        let result = XlsDriver.open(
            Source::Path(output),
            opzioni_lettura_con(
                plenora_io_model::budget::PipelineLimits::default()
                    .with_max_input_bytes(input_bytes),
            )
            .with_assume_crs("EPSG:4326")
            .with_format_option("wkt_column", "geometry"),
        );
        let error = result.err().expect("lo spool deve rispettare il limite");
        assert_eq!(error.code, plenora_io_model::IoErrorCode::LimitExceeded);
    }

    #[test]
    fn xlsx_decompression_ratio_is_checked_before_cell_materialization() {
        let dir = tempfile::tempdir().unwrap();
        let output = dir.path().join("compressed.xlsx");
        let mut workbook = Workbook::new();
        let sheet = workbook.add_worksheet();
        sheet.write_string(0, 0, "geometry").unwrap();
        sheet.write_string(1, 0, "POINT (1 2)").unwrap();
        workbook.save(&output).unwrap();
        let result = XlsDriver.open(
            Source::Path(output),
            opzioni_lettura_con(
                plenora_io_model::budget::PipelineLimits::default().with_decompression_ratio(1),
            )
            .with_assume_crs("EPSG:4326")
            .with_format_option("wkt_column", "geometry"),
        );
        let error = result.err().expect("il rapporto deve fallire chiuso");
        assert_eq!(
            error.category,
            plenora_io_model::ErrorCategory::ResourceLimit
        );
    }

    #[test]
    fn xlsx_wkt_xym_round_trip_preserves_payload_and_contract() {
        let dir = tempfile::tempdir().unwrap();
        let output = dir.path().join("xym.xlsx");
        let expected = wkt("MULTILINESTRING M ((0 0 5,1 1 6))").unwrap();
        let bytes = encode_wkb(&expected, WkbFlavor::Iso).unwrap();
        let mut geometry_contract =
            GeometryColumnContract::wkb_xy(FieldId(0), GEOMETRY, ResolvedCrs::wgs84(), false);
        geometry_contract.dimensions = CoordinateDimensions::Xym;
        geometry_contract.set_exact_geometry_types(vec![GeometryType::MultiLineString]);
        let schema: SchemaRef = Arc::new(Schema::new(vec![
            with_geometry_contract_metadata(
                &geometry_field(GEOMETRY, "EPSG:4326"),
                &geometry_contract,
            ),
            Field::new("id", arrow_schema::DataType::Int64, false),
        ]));
        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![
                Arc::new(BinaryArray::from(vec![Some(bytes.as_slice())])),
                Arc::new(arrow_array::Int64Array::from(vec![1_i64])),
            ],
        )
        .unwrap();
        let plan = WritePlan {
            layers: vec![WriteLayer {
                name: "layer".to_owned(),
                contract: DataContract {
                    schema,
                    geometry: Some(geometry_contract),
                },
            }],
        };

        let driver = XlsDriver;
        let mut writer = driver
            .create(Sink::Path(output.clone()), &plan, &opzioni_scrittura())
            .unwrap();
        writer.write(&batch).unwrap();
        writer.finish().unwrap();

        let read_options = opzioni_lettura()
            .with_assume_crs("EPSG:4326")
            .with_format_option("wkt_column", "geometry");
        let dataset = driver.open(Source::Path(output), read_options).unwrap();
        let output_contract = dataset.layers()[0].contract.geometry.as_ref().unwrap();
        assert_eq!(output_contract.dimensions, CoordinateDimensions::Xym);
        assert_eq!(
            output_contract.geometry_types,
            vec![GeometryType::MultiLineString]
        );
        assert_eq!(
            output_contract
                .native_metadata
                .get("xlsx.geometry_encoding")
                .map(String::as_str),
            Some("wkt")
        );
        let mut reader = dataset
            .open_layer_reader(&ReadRequest {
                layer: LayerId(0),
                projected_fields: None,
                projection_mode: ProjectionMode::BestEffort,
                pruning_predicate: None,
                spatial_pruning_hint: None,
                scope: ReadScope::default(),
                batch_target: BatchTarget::default(),
                cancellation: CancellationToken::default(),
            })
            .unwrap();
        let batch = reader.next_batch().unwrap().unwrap();
        let geometry = batch
            .column(0)
            .as_any()
            .downcast_ref::<BinaryArray>()
            .unwrap();
        let actual = decode_wkb(
            geometry.value(0),
            &plenora_io_model::limits::WkbLimits::default(),
        )
        .unwrap();
        assert_eq!(actual, expected);
    }
}
