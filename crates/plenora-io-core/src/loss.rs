//! `LossReport` — un driver `Approximating` deve popolarlo, mai perdere in
//! silenzio.
//!
//! Vedi `PRODUCT.md § LossReport`. Aggregato per categoria e **bounded**:
//! conteggi piu' un numero limitato di esempi, mai una voce per feature.

use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;

use plenora_io_model::contract::LayerContract;
use plenora_io_model::crs::{definition_authority_srid, CrsResolution};

use crate::capabilities::known_crs_values_disagree;
use crate::descriptor::{ArrowTypeClass, Fidelity};

/// Tetto agli esempi diagnostici conservati (nessun accumulo illimitato).
pub const MAX_LOSS_EXAMPLES: usize = 64;
/// Anche le motivazioni della valutazione restano bounded.
pub const MAX_FIDELITY_REASONS: usize = 64;
/// Quante ragioni distinte si trattengono, oltre le `MAX_FIDELITY_REASONS`
/// che il v2 pubblica.
///
/// Trattenerne esattamente 64 renderebbe il taglio dipendente dall'inserimento:
/// la sessantacinquesima veniva scartata **prima** che l'adattatore ordinasse,
/// quindi l'insieme conservato dipendeva da quali adattatori fossero stati
/// composti e in che ordine. Trattenendone di piu' e ordinandole per chiave
/// canonica, cio' che si pubblica non dipende piu' da come e' arrivato.
///
/// Quattro volte il tetto sul filo, e non di piu', perche' e' la memoria a
/// governare la scelta: un file con mille colonne produce migliaia di ragioni
/// distinte e satura qualunque soglia ragionevole, quindi alzarla comprerebbe
/// esattezza solo per file gia' patologici pagandola su tutti gli altri.
pub const MAX_RAGIONI_TRATTENUTE: usize = 4 * MAX_FIDELITY_REASONS;
/// Quanti esempi distinti si trattengono, oltre i `MAX_LOSS_EXAMPLES` che il
/// v2 pubblica. Stessa ragione di `MAX_RAGIONI_TRATTENUTE`.
pub const MAX_ESEMPI_TRATTENUTI: usize = 4 * MAX_LOSS_EXAMPLES;
/// Quanti byte UTF-8 puo' misurare l'identificatore di una categoria.
///
/// Vive qui, e non nell'adattatore, perche' il filtro di ammissibilita' e'
/// **alla porta**: una voce fuori misura non viene trattenuta, quindi non puo'
/// occupare un posto ne' sfrattare una voce valida. Il tetto deve percio'
/// essere noto dove le voci entrano, e da qui l'adattatore lo prende invece di
/// possederne una copia. Il manifesto del protocollo e il registro delle
/// categorie sono confrontati con questa costante da `check_protocollo_v2.py`:
/// l'autorita' e' una sola, e le altre due sono copie verificate.
pub const MAX_BYTE_ID_CATEGORIA: usize = 128;
/// Quanti byte UTF-8 puo' misurare un dettaglio curato.
///
/// Stesso ragionamento di `MAX_BYTE_ID_CATEGORIA`, e vale su `reasons[].detail`
/// come su `esempi[].context`: sono la stessa specie di stringa, e limitarne
/// una sola lascerebbe l'altra a decidere i byte della sezione.
pub const MAX_BYTE_DETTAGLIO: usize = 512;
/// Categoria stabile per R4.3.1/R4.6.1, leggibile dagli harness di conformità.
pub const INCONSISTENT_CRS_REPRESENTATIONS: &str = "inconsistent_crs_representations";

/// Un esempio diagnostico, nella forma che il v2 pubblica.
///
/// L'ordine dei campi **e'** l'ordine canonico: `category`, poi `posizione`,
/// poi `context`. Il derive di `Ord` lo prende da qui, quindi spostarli
/// cambierebbe il punto in cui la sezione taglia.
///
/// Non c'e' un `context_v1` e non ci deve essere: il v1 pubblica solo
/// `lossless` e `counts`, quindi gli esempi non hanno un passato congelato da
/// conservare. Il vecchio `context` con `layer=` e `field=` non sopravvive in
/// nessuna forma -- non e' mai andato sul filo.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct LossExample {
    pub category: String,
    #[serde(flatten)]
    pub posizione: Posizione,
    /// Descrizione **curata**: strutturale, stabile, e mai un nome che viene
    /// dal file. Dove si e' persa una cosa lo dice `posizione`.
    pub context: String,
}

impl LossExample {
    /// Entra in cio' che il v2 puo' pubblicare?
    ///
    /// Il tetto sull'identificatore vale **ovunque compaia**, non solo in
    /// `counts`: limitarlo in meta' sezione e non nell'altra vorrebbe dire che
    /// i dodici KiB li decide la meta' senza tetto. E `context` prende lo
    /// stesso tetto di `detail`, perche' sono la stessa specie di stringa.
    const fn ammissibile(&self) -> bool {
        self.category.len() <= MAX_BYTE_ID_CATEGORIA && self.context.len() <= MAX_BYTE_DETTAGLIO
    }
}

/// I codici con cui una valutazione di fedelta' motiva il proprio livello.
///
/// `Ord` non e' decorativo: e' **l'ordine canonico** con cui le ragioni escono
/// nella busta, e coincide con l'ordine in cui i codici sono dichiarati qui.
/// Riordinare le varianti cambierebbe l'uscita di file che non sono cambiati,
/// quindi l'ordine di dichiarazione e' contratto quanto i nomi.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FidelityReasonCode {
    AssessmentPending,
    FormatConstraint,
    GeometryApproximation,
    StructureChanged,
    AttributeLoss,
    TypeCoercion,
    PrecisionChanged,
    NullabilityChanged,
    NativeMetadataLoss,
    LossReported,
}

/// Dove si e' persa una cosa, senza dire come si chiama.
///
/// Indici e non nomi, e nemmeno un hash dei nomi: un hash resta un
/// identificatore che chi fornisce il file controlla, e correlarlo e' banale.
///
/// `u64` e non `u32` perche' nessun tetto lo giustificherebbe: `max_columns` e'
/// `u64` nel modello ed e' configurabile, e sui layer non esiste alcun limite.
/// Entrambi gli indici sono **zero-based**; `field_index` indicizza
/// `layer.contract.schema.fields()` e conta **anche** la colonna geometrica,
/// perche' quella e' la sequenza che il ciclo attraversa e un indice che salta
/// un elemento non e' l'indice di quella sequenza.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct Posizione {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub layer_index: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub field_index: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub type_class: Option<ArrowTypeClass>,
}

/// Una motivazione della valutazione, nella forma che il v2 pubblica.
///
/// `detail` e' **curato**: stabile, descrittivo, e mai un nome che viene dal
/// file. La parte autorevole e' `posizione`; il testo la descrive e non la
/// sostituisce, e non e' un campo su cui costruire un parser.
///
/// `dettaglio_v1` porta il `detail` **esatto** che il v1 pubblicava, alla
/// lettera e non ricostruito dai pezzi: e' cio' che rende il congelamento del
/// v1 una tautologia invece di un invariante da difendere a ogni ritocco di un
/// `format!`. Vale `None` dove il sito non aveva nomi da togliere, e li' il v1
/// ricade sul testo curato, che e' gia' identico a quello di prima.
#[derive(Clone, Debug, Serialize)]
pub struct FidelityReason {
    pub code: FidelityReasonCode,
    pub detail: String,
    #[serde(flatten)]
    pub posizione: Posizione,
    /// `skip` non e' una precauzione: e' la ragione per cui questo campo puo'
    /// esistere. Il derive di `Serialize` non deve poter pubblicare i nomi che
    /// il v2 toglie, e a leggerlo e' il solo adattatore v1.
    #[serde(skip)]
    dettaglio_v1: Option<String>,
}

impl FidelityReason {
    /// Una ragione senza posizione ne' passato: il caso dei siti che non hanno
    /// mai portato nomi presi dal file.
    #[must_use]
    pub fn nuova(code: FidelityReasonCode, detail: impl Into<String>) -> Self {
        Self {
            code,
            detail: detail.into(),
            posizione: Posizione::default(),
            dettaglio_v1: None,
        }
    }

    /// Una ragione redatta: testo curato e posizione per il v2, la frase
    /// congelata per il v1.
    #[must_use]
    pub fn redatta(
        code: FidelityReasonCode,
        detail: impl Into<String>,
        posizione: Posizione,
        dettaglio_v1: impl Into<String>,
    ) -> Self {
        Self {
            code,
            detail: detail.into(),
            posizione,
            dettaglio_v1: Some(dettaglio_v1.into()),
        }
    }

    /// Il `detail` che il v1 pubblica.
    ///
    /// **Un solo chiamante**, l'adattatore v1, e a verificarlo e' un gate: la
    /// visibilita' di Rust non sa dire «questo modulo e nessun altro».
    #[must_use]
    pub fn detail_v1(&self) -> &str {
        self.dettaglio_v1.as_deref().unwrap_or(&self.detail)
    }

    /// La chiave canonica: cio' che il v2 vede, e nient'altro.
    fn chiave(&self) -> (FidelityReasonCode, Posizione, &str) {
        (self.code, self.posizione, &self.detail)
    }

    /// Entra in cio' che il v2 puo' pubblicare?
    ///
    /// Il filtro sta **alla porta** e non nell'adattatore: una voce fuori
    /// misura che venisse trattenuta occuperebbe un posto e potrebbe sfrattare
    /// una voce valida, e la sezione uscirebbe piu' povera di quanto il tetto
    /// imponga.
    const fn ammissibile(&self) -> bool {
        self.detail.len() <= MAX_BYTE_DETTAGLIO
    }

    /// La ragione senza il materiale riservato al v1.
    ///
    /// Cio' che si trattiene per il v2 non porta mai `dettaglio_v1`: quella
    /// stringa contiene nomi presi dal file, quindi la sua lunghezza la decide
    /// chi fornisce il file, e trattenerla su centinaia di copie sarebbe una
    /// quota di memoria non delimitata. Il beneficio non e' solo la memoria: la
    /// struttura che serve il v2 non contiene il materiale legacy, quindi non
    /// puo' perderlo per nessuna via.
    fn senza_legacy(mut self) -> Self {
        self.dettaglio_v1 = None;
        self
    }
}

/// `Eq` e `Ord` **non** guardano `dettaglio_v1`.
///
/// Se lo guardassero, il materiale riservato al v1 deciderebbe che cosa il v2
/// considera un duplicato e in che ordine taglia. Cio' che distinguevano i nomi
/// lo distinguono ora gli indici, che stanno in `posizione`.
impl PartialEq for FidelityReason {
    fn eq(&self, altra: &Self) -> bool {
        self.chiave() == altra.chiave()
    }
}

impl Eq for FidelityReason {}

impl PartialOrd for FidelityReason {
    fn partial_cmp(&self, altra: &Self) -> Option<Ordering> {
        Some(self.cmp(altra))
    }
}

impl Ord for FidelityReason {
    fn cmp(&self, altra: &Self) -> Ordering {
        self.chiave().cmp(&altra.chiave())
    }
}

/// Valutazione concreta restituita da `open`/`create`.
///
/// Vedi `PRODUCT.md § LossReport`. Il descrittore resta la capacità generale;
/// questa struttura porta l'esito osservato per il dataset o il contratto
/// corrente.
/// **Niente `Serialize`**, e non per dimenticanza.
///
/// Il derive pubblicherebbe `prime_v1`, `canoniche`, `respinte` e la meccanica
/// del trattenimento: il materiale riservato al v1 e la struttura interna,
/// entrambi fuori da qualunque forma sul filo. Le due forme le costruisce
/// l'adattatore, a mano, ed e' li' che i nomi sul filo sono contratto. Niente
/// lo serializzava, quindi non c'e' nulla da sostituire; a impedire che il
/// derive ritorni e' un gate, perche' toglierlo non impedisce a nessuno di
/// rimetterlo.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FidelityAssessment {
    pub level: Fidelity,
    /// Le prime `MAX_FIDELITY_REASONS` per **inserimento**, deduplicate sulla
    /// chiave vecchia -- `(code, detail_v1())` -- e senza filtro in byte: e'
    /// esattamente cio' che il v1 pubblicava, congelato.
    prime_v1: Vec<FidelityReason>,
    /// Le minori `MAX_RAGIONI_TRATTENUTE` per chiave **canonica**, tutte
    /// ammissibili sul filo e senza materiale legacy: e' da qui che il v2
    /// pubblica le sue sessantaquattro.
    canoniche: BTreeSet<FidelityReason>,
    /// Le chiavi delle voci respinte perche' fuori misura.
    ///
    /// Solo `(code, posizione)`: la stringa che le ha fatte respingere non si
    /// trattiene, se no il rifiuto non servirebbe a niente. Ne segue che il
    /// conteggio e' un **limite inferiore** -- due ragioni con lo stesso codice
    /// e la stessa posizione ma dettagli diversi contano per una -- ed e' la
    /// ragione per cui qualunque voce respinta rende i contatori non esatti.
    respinte: BTreeSet<(FidelityReasonCode, Posizione)>,
    /// Qualunque perdita di esattezza interna: un trattenimento saturo, una
    /// voce respinta per misura. Non e' una quinta causa di omissione, e' un
    /// qualificatore sull'esattezza delle quattro.
    esattezza_persa: bool,
}

impl FidelityAssessment {
    #[must_use]
    pub const fn lossless() -> Self {
        Self::con_livello(Fidelity::Lossless)
    }

    /// Una valutazione vuota al livello dato.
    #[must_use]
    pub const fn con_livello(level: Fidelity) -> Self {
        Self {
            level,
            prime_v1: Vec::new(),
            canoniche: BTreeSet::new(),
            respinte: BTreeSet::new(),
            esattezza_persa: false,
        }
    }

    #[must_use]
    pub fn for_format(format: &str, class: Fidelity) -> Self {
        let (level, reason) = match class {
            Fidelity::Lossless => return Self::lossless(),
            Fidelity::Conditional => (
                Fidelity::Conditional,
                FidelityReason::nuova(
                    FidelityReasonCode::FormatConstraint,
                    format!(
                        "{format}: fedeltà condizionata ai tipi e alle semantiche del contratto"
                    ),
                ),
            ),
            Fidelity::Approximating => (
                Fidelity::Approximating,
                FidelityReason::nuova(
                    FidelityReasonCode::GeometryApproximation,
                    format!("{format}: il formato può richiedere approssimazioni"),
                ),
            ),
        };
        let mut valutazione = Self::con_livello(level);
        valutazione.offri(reason);
        valutazione
    }

    pub fn unassessed(context: impl Into<String>) -> Self {
        let mut valutazione = Self::con_livello(Fidelity::Conditional);
        valutazione.offri(FidelityReason::nuova(
            FidelityReasonCode::AssessmentPending,
            context,
        ));
        valutazione
    }

    pub fn add_reason(&mut self, code: FidelityReasonCode, detail: impl Into<String>) {
        self.offri(FidelityReason::nuova(code, detail));
    }

    /// Una ragione con posizione strutturata e frase v1 congelata.
    pub fn add_reason_redatta(
        &mut self,
        code: FidelityReasonCode,
        detail: impl Into<String>,
        posizione: Posizione,
        dettaglio_v1: impl Into<String>,
    ) {
        self.offri(FidelityReason::redatta(
            code,
            detail,
            posizione,
            dettaglio_v1,
        ));
    }

    /// La porta: una ragione entra nelle due collezioni, ciascuna con la
    /// propria regola. Le due semantiche non si conciliano perche' non sono
    /// una sola.
    fn offri(&mut self, reason: FidelityReason) {
        self.accetta_v1(&reason);
        self.accetta_v2(reason);
    }

    /// Primi 64 per inserimento, dedup sulla chiave **vecchia**, nessun filtro.
    ///
    /// Deduplicare sull'`Eq` del v2 legherebbe il v1 congelato all'identita'
    /// nuova, che e' esattamente cio' che non deve poter accadere.
    fn accetta_v1(&mut self, reason: &FidelityReason) {
        if self.prime_v1.len() >= MAX_FIDELITY_REASONS {
            return;
        }
        let gia_presente = self
            .prime_v1
            .iter()
            .any(|r| r.code == reason.code && r.detail_v1() == reason.detail_v1());
        if !gia_presente {
            self.prime_v1.push(reason.clone());
        }
    }

    /// Filtro alla porta, poi trattenimento canonico con sfratto della maggiore.
    fn accetta_v2(&mut self, reason: FidelityReason) {
        if !reason.ammissibile() {
            self.esattezza_persa = true;
            self.respinte.insert((reason.code, reason.posizione));
            if self.respinte.len() > MAX_RAGIONI_TRATTENUTE {
                self.respinte.pop_last();
            }
            return;
        }
        if !self.canoniche.insert(reason.senza_legacy()) {
            return;
        }
        if self.canoniche.len() > MAX_RAGIONI_TRATTENUTE {
            // La **maggiore**, non l'ultima arrivata: rifiutare le successive
            // rimetterebbe l'insieme in mano all'ordine d'inserimento, che e'
            // cio' da cui il trattenimento canonico lo toglie.
            self.canoniche.pop_last();
            self.esattezza_persa = true;
        }
    }

    /// Le ragioni che il v1 pubblica, nell'ordine in cui sono arrivate.
    #[must_use]
    pub fn ragioni_v1(&self) -> &[FidelityReason] {
        &self.prime_v1
    }

    /// Le ragioni trattenute per il v2, gia' in ordine canonico.
    pub fn ragioni_canoniche(&self) -> impl Iterator<Item = &FidelityReason> {
        self.canoniche.iter()
    }

    /// Quante ne sono trattenute: il v2 ne pubblica al piu'
    /// `MAX_FIDELITY_REASONS`, e la differenza e' `ragioni_omesse`.
    #[must_use]
    pub fn ragioni_trattenute(&self) -> usize {
        self.canoniche.len()
    }

    /// Quante voci sono state respinte per misura, come limite inferiore.
    #[must_use]
    pub fn respinte_per_misura(&self) -> u64 {
        self.respinte.len() as u64
    }

    /// I contatori di omissione sono esatti?
    #[must_use]
    pub const fn omesse_esatte(&self) -> bool {
        !self.esattezza_persa
    }

    /// Fonde un'altra valutazione **nelle due collezioni separatamente**.
    ///
    /// Ricostruire ogni ragione da `code` e `detail`, come faceva
    /// `combined_fidelity`, perderebbe sia la posizione sia la frase v1: la
    /// sezione di conversione del v1 cambierebbe byte, e quella del v2
    /// perderebbe gli indici. E le due collezioni vanno fuse ciascuna con la
    /// propria regola, se no il v1 finisce a dipendere dall'identita' del v2.
    pub fn merge(&mut self, altra: &Self) {
        for reason in &altra.prime_v1 {
            self.accetta_v1(reason);
        }
        for reason in &altra.canoniche {
            self.accetta_v2(reason.clone());
        }
        for chiave in &altra.respinte {
            self.respinte.insert(*chiave);
        }
        while self.respinte.len() > MAX_RAGIONI_TRATTENUTE {
            self.respinte.pop_last();
        }
        self.esattezza_persa |= altra.esattezza_persa;
    }

    /// Le perdite osservate prevalgono su una valutazione teorica e rendono
    /// l'esito concretamente `Approximating`.
    #[must_use]
    pub fn with_loss_report(mut self, loss: &LossReport) -> Self {
        if loss.is_empty() {
            return self;
        }
        self.level = Fidelity::Approximating;
        for (category, count) in &loss.counts {
            self.add_reason(
                reason_code_for_loss(category),
                format!("{category}: {count} occorrenze"),
            );
        }
        self
    }
}

fn reason_code_for_loss(category: &str) -> FidelityReasonCode {
    let category = category.to_lowercase();
    if category.contains("tassell")
        || category.contains("approssim")
        || category.contains("curv")
        || category.contains("bulge")
    {
        FidelityReasonCode::GeometryApproximation
    } else if category.contains("esplos")
        || category.contains("multipart")
        || category.contains("collection")
    {
        FidelityReasonCode::StructureChanged
    } else if category.contains("coerc") {
        FidelityReasonCode::TypeCoercion
    } else if category.contains("attribut")
        || category.contains("colonn")
        || category.contains("propriet")
    {
        FidelityReasonCode::AttributeLoss
    } else if category.contains("precision") {
        FidelityReasonCode::PrecisionChanged
    } else if category.contains("null") {
        FidelityReasonCode::NullabilityChanged
    } else if category.contains("metadata")
        || category.contains("metadati")
        || category.starts_with("crs_")
        || category.starts_with("srid_")
    {
        FidelityReasonCode::NativeMetadataLoss
    } else {
        FidelityReasonCode::LossReported
    }
}

#[derive(Clone, Debug, Default)]
pub struct LossReport {
    /// Aggregati per categoria: (categoria -> conteggio).
    pub counts: BTreeMap<String, u64>,
    /// Gli esempi trattenuti: i **minori** `MAX_ESEMPI_TRATTENUTI` per chiave
    /// canonica, tutti ammissibili sul filo.
    ///
    /// Erano i primi 64 per inserimento, e la selezione dipendeva percio' da
    /// quali adattatori fossero stati composti: `merge` concatenava e troncava,
    /// quindi fondere A con B dava un risultato diverso da fondere B con A.
    esempi: BTreeSet<LossExample>,
    /// Le posizioni degli esempi respinti perche' fuori misura.
    ///
    /// La sola `posizione`, e non la categoria: quando e' proprio `category` a
    /// essere fuori misura non c'e' niente da conservare. Il conteggio e'
    /// percio' un **limite inferiore**, ed e' la ragione per cui una qualunque
    /// respinta rende i contatori non esatti.
    respinti: BTreeSet<Posizione>,
    /// Qualunque perdita di esattezza interna: trattenimento saturo, o una
    /// voce respinta per misura.
    esattezza_persa: bool,
}

impl LossReport {
    pub fn record(&mut self, category: &str, n: u64) {
        let count = self.counts.entry(category.to_owned()).or_default();
        *count = count.saturating_add(n);
    }

    /// La porta degli esempi: filtro di ammissibilita', poi trattenimento
    /// canonico con sfratto della maggiore.
    pub fn add_example(&mut self, example: LossExample) {
        if !example.ammissibile() {
            self.esattezza_persa = true;
            self.respinti.insert(example.posizione);
            if self.respinti.len() > MAX_ESEMPI_TRATTENUTI {
                self.respinti.pop_last();
            }
            return;
        }
        if self.esempi.insert(example) && self.esempi.len() > MAX_ESEMPI_TRATTENUTI {
            self.esempi.pop_last();
            self.esattezza_persa = true;
        }
    }

    /// Gli esempi trattenuti, gia' in ordine canonico.
    pub fn esempi_canonici(&self) -> impl Iterator<Item = &LossExample> {
        self.esempi.iter()
    }

    /// Quanti ne sono trattenuti: il v2 ne pubblica al piu'
    /// `MAX_LOSS_EXAMPLES`, e la differenza e' `esempi_omessi`.
    #[must_use]
    pub fn esempi_trattenuti(&self) -> usize {
        self.esempi.len()
    }

    /// Quanti esempi sono stati respinti per misura, come limite inferiore.
    #[must_use]
    pub fn respinti_per_misura(&self) -> u64 {
        self.respinti.len() as u64
    }

    /// I contatori degli esempi sono esatti?
    #[must_use]
    pub const fn omesse_esatte(&self) -> bool {
        !self.esattezza_persa
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.counts.is_empty()
    }

    /// Unisce report prodotti da livelli diversi (piano statico, driver,
    /// publish) senza perdere il bound diagnostico.
    ///
    /// **Componibile**: e' l'unione di due insiemi canonici, non una
    /// concatenazione troncata, quindi fondere A con B da' lo stesso risultato
    /// di fondere B con A. La forma precedente non lo garantiva, e la
    /// diagnostica cambiava a seconda dell'ordine in cui gli adattatori erano
    /// stati composti -- cioe' per ragioni che non la riguardano.
    pub fn merge(&mut self, other: &Self) {
        for (category, count) in &other.counts {
            self.record(category, *count);
        }
        for example in &other.esempi {
            self.add_example(example.clone());
        }
        for posizione in &other.respinti {
            self.respinti.insert(*posizione);
        }
        while self.respinti.len() > MAX_ESEMPI_TRATTENUTI {
            self.respinti.pop_last();
        }
        self.esattezza_persa |= other.esattezza_persa;
    }
}

/// Dichiara, senza respingerla né conciliarla, un'incoerenza rilevabile fra
/// `crs_definition`, `crs_id` EPSG e `plenora.geometry.srid` osservata da un
/// bordo di lettura.
///
/// La funzione è idempotente sul report così che più adattatori reader possano
/// essere composti senza moltiplicare la stessa osservazione strutturale.
pub(crate) fn declare_crs_inconsistency(contract: &LayerContract, report: &mut LossReport) {
    if report.counts.contains_key(INCONSISTENT_CRS_REPRESENTATIONS) {
        return;
    }
    let Some(geometry) = &contract.contract.geometry else {
        return;
    };
    let (crs_id, definition, definition_format) = match &geometry.crs {
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

    let id_srid = crs_id
        .and_then(plenora_io_model::crs::authority_srid)
        .map(i64::from);
    let definition_srid = definition
        .zip(definition_format)
        .and_then(|(value, format)| definition_authority_srid(value, format))
        .map(i64::from);
    let native_srid = geometry.srid.map(i64::from);
    if !known_crs_values_disagree([definition_srid, id_srid, native_srid]) {
        return;
    }

    report.record(INCONSISTENT_CRS_REPRESENTATIONS, 1);
    // Ne' `crs_id` ne' il nome del layer o della colonna entrano nel testo: il
    // primo e' una stringa libera che viene dal file, gli altri due pure.
    // Restano i tre codici di autorita', che sono numeri di un registro
    // standard e sono **la cosa** che l'esempio deve dire: senza i tre numeri
    // l'incoerenza non e' leggibile. Il contratto li autorizza per nome.
    let _ = crs_id;
    report.add_example(LossExample {
        category: INCONSISTENT_CRS_REPRESENTATIONS.to_owned(),
        posizione: Posizione {
            layer_index: None,
            field_index: None,
            type_class: None,
        },
        context: format!(
            "definition_epsg={} id_epsg={} srid={}",
            codice_di_autorita(definition_srid),
            codice_di_autorita(id_srid),
            codice_di_autorita(native_srid),
        ),
    });
}

/// Un codice di autorita' nella sua resa **curata**: il numero, oppure
/// `absent`.
///
/// `{:?}` su un `Option` produce `None` e `Some(4326)`, che e' la forma `Debug`
/// di un tipo della libreria standard: la stessa specie di cosa che la
/// redazione vieta quando esclude la forma `Debug` dei tipi di dipendenza. Un
/// consumatore che ne facesse `match` dipenderebbe da come `Option` si stampa,
/// che non e' un nostro contratto. La resa la scriviamo noi, e resta questa
/// anche se un giorno il tipo cambiasse.
fn codice_di_autorita(valore: Option<i64>) -> String {
    valore.map_or_else(|| "absent".to_owned(), |codice| codice.to_string())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use arrow_schema::{DataType, Field, Schema};
    use plenora_io_model::contract::{DataContract, FieldId, GeometryColumnContract, LayerId};
    use plenora_io_model::crs::{CrsKind, RawCrs, ResolvedCrs};

    use super::*;

    fn layer_with_geometry(geometry: GeometryColumnContract) -> LayerContract {
        LayerContract {
            id: LayerId(0),
            name: "parcels".to_owned(),
            contract: DataContract::new(
                Arc::new(Schema::new(vec![Field::new(
                    "geom",
                    DataType::Binary,
                    true,
                )])),
                Some(geometry),
            ),
        }
    }

    #[test]
    fn observed_loss_promotes_assessment_and_stays_bounded() {
        let mut report = LossReport::default();
        for index in 0..(MAX_FIDELITY_REASONS + 10) {
            report.record(&format!("category-{index}"), 1);
        }
        let assessment =
            FidelityAssessment::for_format("test", Fidelity::Conditional).with_loss_report(&report);
        assert_eq!(assessment.level, Fidelity::Approximating);
        assert_eq!(assessment.ragioni_v1().len(), MAX_FIDELITY_REASONS);
        assert!(assessment
            .ragioni_v1()
            .iter()
            .any(|reason| reason.code == FidelityReasonCode::LossReported));
    }

    #[test]
    fn lossless_assessment_has_no_reasons() {
        assert_eq!(
            FidelityAssessment::for_format("ipc", Fidelity::Lossless),
            FidelityAssessment::lossless()
        );
    }

    #[test]
    fn definition_and_srid_disagreement_is_declared_once() {
        let definition = concat!(
            "PROJCS[\"Monte Mario / Italy zone 1\",",
            "GEOGCS[\"Monte Mario\",AUTHORITY[\"EPSG\",\"4265\"]],",
            "AUTHORITY[\"EPSG\",\"3003\"]]"
        );
        let mut geometry = GeometryColumnContract::wkb_xy(
            FieldId(0),
            "geom",
            ResolvedCrs::new(None, CrsKind::Projected, Some(definition.to_owned())),
            true,
        );
        geometry.srid = Some(4326);
        let layer = layer_with_geometry(geometry);
        let mut report = LossReport::default();

        declare_crs_inconsistency(&layer, &mut report);
        declare_crs_inconsistency(&layer, &mut report);

        assert_eq!(report.counts[INCONSISTENT_CRS_REPRESENTATIONS], 1);
        assert!(report
            .esempi_canonici()
            .next()
            .expect("un esempio")
            .context
            .contains("definition_epsg=3003"));
    }

    #[test]
    fn unresolved_definition_and_authority_disagreement_is_declared() {
        let raw = RawCrs::new(
            "GEOGCS[\"WGS 84\",AUTHORITY[\"EPSG\",\"4326\"]]".to_owned(),
            Some("EPSG:3003".to_owned()),
        );
        let geometry = GeometryColumnContract::wkb_xy(
            FieldId(0),
            "geom",
            CrsResolution::DeclaredButUnresolved(raw),
            true,
        );
        let layer = layer_with_geometry(geometry);
        let mut report = LossReport::default();

        declare_crs_inconsistency(&layer, &mut report);

        assert_eq!(report.counts[INCONSISTENT_CRS_REPRESENTATIONS], 1);
    }
}
