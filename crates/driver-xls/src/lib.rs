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
    FormatWriteCapabilities, NullabilitySupport, SingleReaderGate, SinkPathConstraint,
    TypeCoercionPolicy, WritePlan, SCALAR_TYPES, UTF8_FIELD_NAMES, WKB_PASSTHROUGH_GEOMETRY,
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
        sink_path: SinkPathConstraint::Free,
    }),
    SCHEMA_OPZIONI,
    &["xlsx"],
    1,
    5,
    10,
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
        // Nessun controllo sul suffisso della destinazione: era una
        // convenzione, non un requisito del formato. Vedi la nota identica
        // negli altri sei driver, e `recognised_suffixes` per la meta' che
        // resta vera -- quella di chi rilegge senza dichiarare il formato.
        //
        // Il controllo in `open` invece **resta**, e non e' incoerenza: li'
        // l'estensione e' l'unico segnale disponibile, perche' una sorgente si
        // riconosce e non si dichiara. Scrivere e leggere non hanno lo stesso
        // problema.
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
    // Come in `driver-kml`: la 0.42 pretende UTF-8 dal lettore, e la
    // transcodifica dalla codifica dichiarata passa da `DecodingReader`.
    let mut lettore =
        quick_xml::Reader::from_reader(quick_xml::encoding::DecodingReader::new(sorgente));
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
            if !matches!(attributo.key.as_ref(), "r" | "ref") {
                continue;
            }
            valida_valore_riferimento(attributo.value.as_ref().as_bytes())?;
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

/// Il foglio **non** dichiara le proprie dimensioni.
///
/// `<dimension>` e' opzionale in ECMA-376, e `calamine` non distingue fra
/// «assente» e «presente e pari a `A1:A1`»: in entrambi i casi restituisce la
/// cornice di una cella sola. La distinzione richiederebbe di portare il nome
/// dell'elemento attraverso il pre-filtro XML, il cui confine e' dichiarato
/// come una decisione e non come una correzione.
///
/// Non la si paga: nel caso ambiguo -- un foglio che dichiara `A1:A1` e scrive
/// altrove -- ricavare i limiti dalle celle e' cio' che la specifica prescrive
/// comunque, perche' l'elemento e' un suggerimento. La severita' resta dove
/// serve: una cornice dichiarata **non degenere** e contraddetta dalle celle
/// e' un file che si contraddice, e viene rifiutata come prima.
const fn cornice_non_dichiarata(bounds: SheetBounds) -> bool {
    bounds.start.0 == bounds.end.0
        && bounds.start.1 == bounds.end.1
        && bounds.start.0 == 0
        && bounds.start.1 == 0
}

/// I limiti **ricavati dalle celle**, sotto le quote di lettura.
///
/// La scansione e' O(celle) in tempo e O(1) in memoria, e le quote si applicano
/// mentre scorre e non dopo: un foglio che omette `<dimension>` non ha un
/// numero di colonne da leggere prima di aprirlo, e senza questo tetto
/// omettere l'elemento sarebbe il modo di farsi allocare una riga larga a
/// piacere.
fn limiti_osservati<RS>(
    reader: &mut LettoreCelleSorvegliato<'_, RS>,
    quote: XlsxQuote,
    cancellation: &CancellationToken,
) -> Result<SheetBounds>
where
    RS: Read + Seek,
{
    let mut estremi: Option<SheetBounds> = None;
    let mut celle = 0usize;
    while let Some((riga, colonna, _)) = reader.prossima_cella()? {
        celle = celle.saturating_add(1);
        check_cancelled_periodically(cancellation, ErrorPhase::Read, celle)?;
        let cornice = estremi.get_or_insert(SheetBounds {
            start: (riga, colonna),
            end: (riga, colonna),
        });
        cornice.start.0 = cornice.start.0.min(riga);
        cornice.start.1 = cornice.start.1.min(colonna);
        cornice.end.0 = cornice.end.0.max(riga);
        cornice.end.1 = cornice.end.1.max(colonna);
        let larghezza = data_row_width(*cornice)?;
        if larghezza > quote.colonne {
            return Err(PlenoraIoError::limite_redatto(
                &PublicMessage::CuratedBetween(
                    "XLSX:",
                    NumeroStrutturale::Conteggio(driver_common::saturating_u64(larghezza)),
                    "colonne oltre il limite di",
                    NumeroStrutturale::Limite(driver_common::saturating_u64(quote.colonne)),
                ),
            ));
        }
        let righe = data_row_count(*cornice)?;
        if righe > quote.righe {
            return Err(PlenoraIoError::limite_redatto(
                &PublicMessage::CuratedBetween(
                    "XLSX:",
                    NumeroStrutturale::Conteggio(driver_common::saturating_u64(righe)),
                    "righe oltre il limite di",
                    NumeroStrutturale::Limite(driver_common::saturating_u64(quote.righe)),
                ),
            ));
        }
    }
    // Nessuna cella: si restituisce la cornice degenere, e a dire che il foglio
    // e' vuoto resta il conteggio delle celle di `infer_layout`. Deciderlo qui
    // vorrebbe dire due punti che pronunciano lo stesso rifiuto.
    Ok(estremi.unwrap_or(SheetBounds {
        start: (0, 0),
        end: (0, 0),
    }))
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

/// La lunghezza di un payload dello spool, nei quattro byte che il formato le
/// riserva.
///
/// Estratta dai due chiamanti -- `geometry` e `data` -- perche' dove stava non
/// era provabile: la applicano alla lunghezza di una **fetta vera**, e una
/// fetta oltre i quattro gibibyte non e' costruibile in una sonda senza
/// allocarla. Qui la lunghezza e' un argomento, e il rifiuto si prova con un
/// numero.
///
/// Il messaggio arriva dal chiamante e non e' un dettaglio: chi legge deve
/// sapere se a non entrare sia una geometria o un testo, perche' le due cose si
/// riducono in modi diversi.
///
/// # Errors
///
/// [`PlenoraIoError`] con categoria `ResourceLimit` se la lunghezza non entra
/// in `u32`.
fn lunghezza_dello_spool(byte: usize, oltre_il_tetto: &'static str) -> Result<u32> {
    u32::try_from(byte)
        .map_err(|_| PlenoraIoError::limite_redatto(&PublicMessage::Curated(oltre_il_tetto)))
}

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
        // Una fetta vuota non occupa spool e non prenota niente.
        //
        // Senza questo ritorno la scrittura chiedeva `lease_spill(0)`, che nel
        // modello di budget e' un errore -- «una lease deve essere maggiore di
        // zero» -- e un `.xlsx` conforme con una cella di testo esplicitamente
        // vuota veniva rifiutato come se avesse superato una quota. Il ramo
        // non e' una scorciatoia: zero byte scritti sono zero byte contati, e
        // `write_all(&[])` era gia' un'operazione senza effetto.
        if bytes.is_empty() {
            return Ok(());
        }
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
            Some(bytes) => {
                lunghezza_dello_spool(bytes.len(), "geometria XLSX troppo grande per lo spool")?
            }
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
                let length =
                    lunghezza_dello_spool(bytes.len(), "testo XLSX troppo grande per lo spool")?;
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

/// Crea l'inode dello spool nella directory che l'operatore ha scelto.
///
/// # Il difetto che questa estrazione corregge
///
/// `infer_layout` creava il file con `NamedTempFile::new()`, che usa la
/// directory temporanea di sistema e **ignora** `PLENORA_SPILL_DIR`. La
/// documentazione dichiara che quella variabile sceglie dove vive lo spill; chi
/// la impostava per mettere lo spill su un volume capiente vedeva lo spool
/// XLSX finire altrove lo stesso, e se ne accorgeva a disco pieno.
///
/// # Perche' e' una funzione e non due righe
///
/// Il fallimento della creazione e' un errore d'ambiente, e dentro
/// `infer_layout` non era raggiungibile da nessuna prova: la directory la
/// sceglieva la libreria. Qui la directory e' un argomento, e una che non
/// esiste si passa senza toccare l'ambiente del processo -- che gli altri test
/// condividono.
///
/// # Errors
///
/// [`PlenoraIoError`] se la directory non ospita un file temporaneo nuovo.
fn crea_lo_spool(directory: &std::path::Path) -> Result<Arc<tempfile::NamedTempFile>> {
    tempfile::NamedTempFile::new_in(directory)
        .map(Arc::new)
        .map_err(|_| err(&PublicMessage::Curated("spool XLSX non creabile")))
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
    // Tre aperture del lettore di celle, e non una: la cornice dichiarata, la
    // scansione che la ricava quando non c'e', e la passata vera. Il costo e'
    // una rilettura delle celle **solo** nel foglio che omette `<dimension>`;
    // quello che la dichiara apre due volte e scorre una volta sola.
    let dichiarate = {
        let mut sonda = LettoreCelleSorvegliato::nuovo(workbook, sheet)?;
        sonda.dimensioni()?
    };
    let bounds = if cornice_non_dichiarata(dichiarate) {
        let mut scansione = LettoreCelleSorvegliato::nuovo(workbook, sheet)?;
        limiti_osservati(&mut scansione, quote, cancellation)?
    } else {
        dichiarate
    };
    let mut reader = LettoreCelleSorvegliato::nuovo(workbook, sheet)?;
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
    let spool = crea_lo_spool(&plenora_io_core::driver::spool::spill_directory()?)?;
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
mod tests;
