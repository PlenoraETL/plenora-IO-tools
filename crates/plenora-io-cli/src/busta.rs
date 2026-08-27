//! I tetti della diagnostica nella busta, e il troncamento che li dichiara.
//!
//! # Perche' esistono
//!
//! `read_loss`, `write_loss` e le tre valutazioni di fedelta' finiscono nella
//! busta JSON della CLI, che e' un'interfaccia esterna. Prima di questo modulo
//! nessuna di quelle sezioni aveva un tetto in byte, e le stringhe che le
//! riempiono portano identificatori che vengono dal file: la dimensione della
//! busta la decideva chi forniva il file.
//!
//! # La ripartizione e' fissa
//!
//! Dodici KiB per ciascuna delle **cinque** sezioni, quattro KiB per la
//! struttura aggregata e per le dichiarazioni di troncamento, sessantaquattro
//! in tutto. Il budget non speso **non** si redistribuisce, ed e' una scelta,
//! non un'omissione: con un consumo sequenziale una sezione grande affamerebbe
//! quelle che la seguono, e la stessa sezione produrrebbe un output diverso a
//! seconda di quanto ha occupato un'altra. Una diagnostica che cambia per
//! ragioni che non la riguardano non e' una diagnostica.
//!
//! # Il conteggio e' sui byte serializzati
//!
//! Non sulla lunghezza delle stringhe di partenza: un dettaglio che contiene
//! virgolette o accenti occupa nel JSON piu' byte di quanti caratteri abbia, e
//! misurare prima dell'escaping vorrebbe dire misurare un'altra cosa. Qui si
//! serializza e si contano i byte che escono davvero.
//!
//! # Le quattro cause restano separate
//!
//! Categorie, ragioni, esempi e budget in byte sono quattro modi diversi di
//! restare fuori, e sommarli in un numero solo direbbe a chi legge che qualcosa
//! e' stato tolto senza dirgli che cosa. Nessun troncamento e' silenzioso: se
//! nemmeno la struttura minima entra nel budget si fallisce chiusi, perche' la
//! sola cosa peggiore di una diagnostica troncata e' una troncata che tace.

use serde_json::{json, Map, Value};

use plenora_io_core::descriptor::Fidelity;
use plenora_io_core::loss::{
    FidelityAssessment, FidelityReason, FidelityReasonCode, LossReport, MAX_FIDELITY_REASONS,
};

/// Quante categorie distinte una sezione `counts` puo' pubblicare.
pub const MAX_CATEGORIE: usize = 64;
/// Quanti byte UTF-8 puo' misurare l'identificatore di una categoria.
pub const MAX_BYTE_ID_CATEGORIA: usize = 128;
/// Quanti byte UTF-8 puo' misurare un dettaglio curato.
pub const MAX_BYTE_DETTAGLIO: usize = 512;
/// Le sezioni con un budget proprio: tre fedelta' e due rapporti di perdita.
pub const SEZIONI: usize = 5;
/// Il budget di ciascuna sezione, in byte della sua serializzazione JSON.
pub const BYTE_PER_SEZIONE: usize = 12 * 1024;
/// Il budget riservato alla struttura aggregata e alle dichiarazioni.
pub const BYTE_DELLA_STRUTTURA: usize = 4 * 1024;
/// Il tetto complessivo della diagnostica in una busta: 64 KiB.
pub const MAX_BYTE_BUSTA: usize = SEZIONI * BYTE_PER_SEZIONE + BYTE_DELLA_STRUTTURA;

/// Quale protocollo la busta JSON su stdout deve rispettare.
///
/// Il valore predefinito e' **v2**: e' quello con i tetti, la redazione e le
/// dichiarazioni di troncamento, ed e' l'unico che i gate di release e la
/// qualifica cross-component usano.
///
/// `V1Legacy` esiste per compatibilita' esplicita e **rischiosa**, e si sceglie
/// con un flag che lo dice. Non e' un v2 con qualche campo in meno: e' il
/// protocollo congelato, byte per byte, difetti compresi -- fra cui una
/// cardinalita' e una dimensione che le decide chi fornisce il file. Correggerlo
/// a meta' produrrebbe un ibrido che si presenta come v1 e ne cambia il
/// significato, che e' proprio cio' che l'ICD vieta.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Protocollo {
    #[default]
    V2,
    V1Legacy,
}

impl Protocollo {
    /// Il numero che la busta dichiara.
    #[must_use]
    pub const fn versione(self) -> u64 {
        match self {
            Self::V2 => 2,
            Self::V1Legacy => 1,
        }
    }

    /// Il suffisso dei nomi di contratto.
    ///
    /// Una versione sola per tutte le buste: un protocollo in cui
    /// `protocol_version` dice 2 e i contratti dicono `-v1` non e' un
    /// protocollo, sono due affermazioni che si contraddicono nello stesso
    /// documento.
    #[must_use]
    pub const fn suffisso(self) -> &'static str {
        match self {
            Self::V2 => "v2",
            Self::V1Legacy => "v1",
        }
    }

    /// L'avviso che accompagna la scelta legacy, su stderr e mai su stdout.
    ///
    /// Su stdout non ci va perche' il v1 e' congelato **byte per byte**:
    /// aggiungere un avviso al documento sarebbe cambiarlo.
    #[must_use]
    pub const fn avviso(self) -> Option<&'static str> {
        match self {
            Self::V2 => None,
            Self::V1Legacy => Some(
                "attenzione: protocollo v1 legacy selezionato esplicitamente. \
                 La diagnostica di questa busta non e' limitata: `counts` puo' \
                 riportare identificatori controllati dal file e produrre fino a \
                 4096 chiavi di lunghezza libera. Il v2 e' il protocollo \
                 predefinito e l'unico usato dai gate di release.",
            ),
        }
    }
}

/// Il nome di contratto di una busta, nel protocollo scelto.
#[must_use]
pub fn contratto(nome: &str, protocollo: Protocollo) -> String {
    format!("plenora-io-{nome}-{}", protocollo.suffisso())
}

/// Che cosa e' rimasto fuori, e **per quale delle quattro ragioni**.
///
/// Quattro contatori e non uno: chi legge deve poter distinguere «ci sono piu'
/// di sessantaquattro categorie» da «lo spazio e' finito», perche' le due
/// portano a decisioni diverse.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Troncamento {
    /// Categorie oltre `MAX_CATEGORIE`.
    pub categorie_omesse: u64,
    /// Ragioni oltre `MAX_FIDELITY_REASONS`.
    pub ragioni_omesse: u64,
    /// Esempi oltre il proprio tetto di cardinalita'.
    ///
    /// Vale sempre zero finche' gli esempi non vanno sul filo: il v2 li
    /// pubblichera' quando la redazione ci sara', perche' oggi il loro
    /// `context` porta nomi presi dal file. Il campo c'e' dall'inizio perche'
    /// e' contratto, e un campo che compare piu' tardi e' un cambiamento di
    /// protocollo.
    pub esempi_omessi: u64,
    /// Voci lasciate fuori per un **limite in byte**: quello della singola
    /// voce -- un identificatore oltre 128 byte, un dettaglio oltre 512 -- o
    /// quello della sezione. Contate a parte dalle tre soglie di cardinalita',
    /// perche' «sono troppe» e «non ci stanno» sono due cose diverse.
    pub omesse_per_byte: u64,
}

impl Troncamento {
    /// La forma sul filo, scritta a mano e non derivata.
    ///
    /// I nomi dei quattro campi sono contratto: derivarli dai nomi Rust
    /// legherebbe l'interfaccia esterna a come si chiamano qui dentro, e
    /// rinominare un campo diventerebbe una rottura di protocollo per
    /// distrazione.
    fn documento(self) -> Value {
        json!({
            "categorie_omesse": self.categorie_omesse,
            "ragioni_omesse": self.ragioni_omesse,
            "esempi_omessi": self.esempi_omessi,
            "omesse_per_byte": self.omesse_per_byte,
        })
    }

    #[must_use]
    pub const fn niente_di_omesso(self) -> bool {
        self.categorie_omesse == 0
            && self.ragioni_omesse == 0
            && self.esempi_omessi == 0
            && self.omesse_per_byte == 0
    }
}

/// La sezione non entra nel proprio budget nemmeno da vuota.
///
/// E' l'unico esito che non e' un documento: si fallisce chiusi invece di
/// pubblicare una sezione senza la sua dichiarazione di troncamento, perche'
/// una diagnostica che tace su cio' che ha tolto e' peggio di nessuna.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BudgetInsufficiente;

/// I byte UTF-8 che quel documento occupa una volta serializzato.
///
/// La serializzazione e' la stessa che finisce sullo stdout, escaping compreso,
/// e non una stima. `Display` e non `serde_json::to_string`: il primo e'
/// infallibile su un `Value`, il secondo restituisce un `Result` che non puo'
/// fallire e che qualcuno dovrebbe ripiegare -- e un ripiego su una misura di
/// byte darebbe un numero inventato proprio a chi deve decidere se tagliare.
fn byte_serializzati(documento: &Value) -> usize {
    documento.to_string().len()
}

/// Il primo prefisso di `voci` che, aggiunto a `base` sotto `chiave`, resta nel
/// budget; e quante ne sono rimaste fuori.
///
/// Si aggiunge una voce alla volta e si **riserializza**: e' quadratico, e con
/// sessantaquattro voci non ha importanza. Una stima incrementale sarebbe piu'
/// veloce e direbbe un numero diverso da quello che esce, che e' esattamente
/// cio' che questo modulo esiste per non fare.
fn entro_il_budget<T, F>(
    base: &Map<String, Value>,
    chiave: &str,
    voci: &[T],
    budget: usize,
    aggiungi: F,
) -> (Value, u64)
where
    F: Fn(&mut Value, &T),
{
    let mut accettate = Value::Array(Vec::new());
    let mut ultima_buona = accettate.clone();
    let mut dentro = 0_usize;
    for voce in voci {
        aggiungi(&mut accettate, voce);
        let mut candidato = base.clone();
        candidato.insert(chiave.to_owned(), accettate.clone());
        if byte_serializzati(&Value::Object(candidato)) > budget {
            break;
        }
        ultima_buona = accettate.clone();
        dentro += 1;
    }
    let fuori = voci.len().saturating_sub(dentro);
    (ultima_buona, fuori as u64)
}

/// La sezione `counts` di un rapporto di perdita, dentro il proprio budget.
///
/// # Errors
///
/// `BudgetInsufficiente` se nemmeno la struttura minima -- l'oggetto vuoto con
/// la sua dichiarazione di troncamento -- entra in `BYTE_PER_SEZIONE`.
///
/// # L'ordine e' canonico
///
/// Le categorie arrivano da una `BTreeMap`, quindi ordinate per identificatore:
/// il troncamento non dipende dall'ordine in cui i driver hanno registrato le
/// perdite. Due corse sullo stesso file tagliano nello stesso punto.
pub fn sezione_di_perdita(
    rapporto: &LossReport,
    budget: usize,
) -> Result<(Value, Troncamento), BudgetInsufficiente> {
    let mut troncamento = Troncamento::default();

    // 1. I metadati di troncamento entrano per primi: sono la dichiarazione, e
    //    una dichiarazione che entra solo se avanza spazio non e' una garanzia.
    let mut base = Map::new();
    base.insert("troncato".to_owned(), json!(false));
    base.insert("omesse".to_owned(), Troncamento::default().documento());
    if byte_serializzati(&Value::Object(base.clone())) > budget {
        return Err(BudgetInsufficiente);
    }

    // 2. Le categorie, esatte nei valori: si omette una voce intera, mai si
    //    riscrive un conteggio. Un `12` al posto di `300` sarebbe un valore che
    //    significa due cose.
    // Il limite **per voce** viene prima di quello della sezione: un
    // identificatore fuori misura non entra nemmeno se lo spazio ci sarebbe,
    // perche' il tetto sull'identificatore e' una promessa a chi legge, non
    // una conseguenza dello spazio disponibile.
    let (ammesse, fuori_misura): (Vec<_>, Vec<_>) = rapporto
        .counts
        .iter()
        .partition(|(categoria, _)| categoria.len() <= MAX_BYTE_ID_CATEGORIA);
    troncamento.omesse_per_byte = fuori_misura.len() as u64;
    let tutte: Vec<(&String, &u64)> = ammesse;
    let oltre_la_soglia = tutte.len().saturating_sub(MAX_CATEGORIE);
    troncamento.categorie_omesse = oltre_la_soglia as u64;
    let candidate = &tutte[..tutte.len().min(MAX_CATEGORIE)];

    let (counts, per_byte) = entro_il_budget(
        &base,
        "counts",
        candidate,
        budget,
        |accumulatore: &mut Value, (categoria, conteggio): &(&String, &u64)| {
            if let Value::Array(voci) = accumulatore {
                voci.push(json!({"categoria": categoria, "conteggio": conteggio}));
            }
        },
    );
    troncamento.omesse_per_byte = troncamento.omesse_per_byte.saturating_add(per_byte);

    let mut documento = base;
    documento.insert("counts".to_owned(), counts);
    documento.insert(
        "troncato".to_owned(),
        json!(!troncamento.niente_di_omesso()),
    );
    documento.insert("omesse".to_owned(), troncamento.documento());
    Ok((Value::Object(documento), troncamento))
}

/// Una valutazione di fedelta' dentro il proprio budget.
///
/// # Errors
///
/// `BudgetInsufficiente` se nemmeno il livello con la sua dichiarazione entra.
///
/// # L'ordine e' canonico
///
/// Le ragioni entrano ordinate per `(codice, dettaglio)`, non nell'ordine in
/// cui i livelli le hanno aggiunte: quello dipende da quali adattatori sono
/// stati composti, cioe' da qualcosa che chi legge il file non controlla.
pub fn sezione_di_fedelta(
    valutazione: &FidelityAssessment,
    budget: usize,
) -> Result<(Value, Troncamento), BudgetInsufficiente> {
    let mut troncamento = Troncamento::default();

    let mut base = Map::new();
    base.insert("level".to_owned(), documento_del_livello(valutazione.level));
    base.insert("troncato".to_owned(), json!(false));
    base.insert("omesse".to_owned(), Troncamento::default().documento());
    if byte_serializzati(&Value::Object(base.clone())) > budget {
        return Err(BudgetInsufficiente);
    }

    // Come per gli identificatori: un dettaglio oltre il tetto non entra,
    // e il taglio non spezza un carattere perche' la voce esce **intera**.
    let (ammesse, fuori_misura): (Vec<_>, Vec<_>) = valutazione
        .reasons
        .iter()
        .partition(|ragione| ragione.detail.len() <= MAX_BYTE_DETTAGLIO);
    troncamento.omesse_per_byte = fuori_misura.len() as u64;
    let mut ordinate: Vec<_> = ammesse;
    ordinate.sort_by(|a, b| (a.code, &a.detail).cmp(&(b.code, &b.detail)));
    let oltre_la_soglia = ordinate.len().saturating_sub(MAX_FIDELITY_REASONS);
    troncamento.ragioni_omesse = oltre_la_soglia as u64;
    let candidate = &ordinate[..ordinate.len().min(MAX_FIDELITY_REASONS)];

    let (reasons, per_byte) =
        entro_il_budget(&base, "reasons", candidate, budget, |acc, ragione| {
            if let Value::Array(voci) = acc {
                voci.push(documento_della_ragione(ragione));
            }
        });
    troncamento.omesse_per_byte = troncamento.omesse_per_byte.saturating_add(per_byte);

    let mut documento = base;
    documento.insert("reasons".to_owned(), reasons);
    documento.insert(
        "troncato".to_owned(),
        json!(!troncamento.niente_di_omesso()),
    );
    documento.insert("omesse".to_owned(), troncamento.documento());
    Ok((Value::Object(documento), troncamento))
}

/// Il livello di fedelta' nella sua forma sul filo.
///
/// Scritto a mano come `Troncamento::documento`, e per la stessa ragione: i
/// nomi sono contratto. `serde_json::to_value` sarebbe una riga sola e
/// restituirebbe un `Result` che su un enum di varianti semplici non puo'
/// fallire: ripiegarlo su `Value::Null` metterebbe «fedelta' sconosciuta» dove
/// il codice sa benissimo quale sia. La sonda
/// `la_forma_scritta_a_mano_coincide_col_derive` impedisce che le due
/// scritture divergano.
fn documento_del_livello(livello: Fidelity) -> Value {
    Value::String(
        match livello {
            Fidelity::Lossless => "lossless",
            Fidelity::Conditional => "conditional",
            Fidelity::Approximating => "approximating",
        }
        .to_owned(),
    )
}

/// Il codice di una ragione nella sua forma sul filo.
fn documento_del_codice(codice: FidelityReasonCode) -> Value {
    Value::String(
        match codice {
            FidelityReasonCode::AssessmentPending => "assessment_pending",
            FidelityReasonCode::FormatConstraint => "format_constraint",
            FidelityReasonCode::GeometryApproximation => "geometry_approximation",
            FidelityReasonCode::StructureChanged => "structure_changed",
            FidelityReasonCode::AttributeLoss => "attribute_loss",
            FidelityReasonCode::TypeCoercion => "type_coercion",
            FidelityReasonCode::PrecisionChanged => "precision_changed",
            FidelityReasonCode::NullabilityChanged => "nullability_changed",
            FidelityReasonCode::NativeMetadataLoss => "native_metadata_loss",
            FidelityReasonCode::LossReported => "loss_reported",
        }
        .to_owned(),
    )
}

/// Una ragione di fedelta' nella sua forma sul filo.
fn documento_della_ragione(ragione: &FidelityReason) -> Value {
    json!({
        "code": documento_del_codice(ragione.code),
        "detail": ragione.detail,
    })
}

/// La diagnostica di una busta sta nel tetto complessivo?
///
/// I tetti per sezione non delimitano l'aggregato: cinque sezioni ciascuna
/// dentro i propri dodici KiB fanno sessanta KiB, e a quel punto la struttura
/// che le contiene non ha piu' un limite proprio. Questo controllo lo mette, e
/// **fallisce chiuso**: una busta oltre il tetto non si pubblica troncandola in
/// silenzio, perche' a quel punto nessuna delle dichiarazioni per sezione
/// direbbe la verita' sull'insieme.
///
/// # Errors
///
/// `BudgetInsufficiente` se le sezioni piu' la struttura superano
/// `MAX_BYTE_BUSTA`.
pub fn diagnostica_entro_il_totale(sezioni: &[&Value]) -> Result<usize, BudgetInsufficiente> {
    let byte: usize = sezioni.iter().map(|s| byte_serializzati(s)).sum();
    let totale = byte.saturating_add(BYTE_DELLA_STRUTTURA);
    if totale > MAX_BYTE_BUSTA {
        return Err(BudgetInsufficiente);
    }
    Ok(totale)
}

/// Il rapporto di perdita nella forma del protocollo scelto.
///
/// Le due forme non si assomigliano, e non devono: il v1 e' congelato -- due
/// campi, nessun tetto, nessuna dichiarazione -- e il v2 e' la forma che quel
/// congelamento impedisce di correggere. Scriverne una sola con qualche campo
/// condizionale le farebbe divergere alla prima modifica distratta.
///
/// # Errors
///
/// `BudgetInsufficiente` quando, nel v2, nemmeno la dichiarazione di
/// troncamento entra nel budget della sezione.
pub fn documento_di_perdita(
    valutazione: &FidelityAssessment,
    rapporto: &LossReport,
    protocollo: Protocollo,
) -> Result<Value, BudgetInsufficiente> {
    match protocollo {
        // La forma del 2026-08, riprodotta qui e non richiamata altrove:
        // `lossless` piu' `counts` come mappa. Nessun tetto, nessuna
        // dichiarazione, e le chiavi che il file decide.
        Protocollo::V1Legacy => Ok(json!({
            "lossless": valutazione.level == Fidelity::Lossless && rapporto.is_empty(),
            "counts": mappa_dei_conteggi(rapporto),
        })),
        Protocollo::V2 => {
            let (sezione, _) = sezione_di_perdita(rapporto, BYTE_PER_SEZIONE)?;
            let mut documento = sezione;
            if let Value::Object(campi) = &mut documento {
                campi.insert(
                    "lossless".to_owned(),
                    json!(valutazione.level == Fidelity::Lossless && rapporto.is_empty()),
                );
            }
            Ok(documento)
        }
    }
}

/// I conteggi nella mappa che il v1 pubblica.
///
/// Scritta a mano invece di serializzare la `BTreeMap`: e' la forma congelata,
/// e legarla al tipo Rust vorrebbe dire che cambiare il tipo cambia il
/// protocollo.
fn mappa_dei_conteggi(rapporto: &LossReport) -> Value {
    let mut mappa = Map::new();
    for (categoria, conteggio) in &rapporto.counts {
        mappa.insert(categoria.clone(), json!(conteggio));
    }
    Value::Object(mappa)
}

/// La valutazione di fedelta' nella forma del protocollo scelto.
///
/// # Errors
///
/// `BudgetInsufficiente` quando, nel v2, nemmeno il livello con la sua
/// dichiarazione entra nel budget della sezione.
pub fn documento_di_fedelta(
    valutazione: &FidelityAssessment,
    protocollo: Protocollo,
) -> Result<Value, BudgetInsufficiente> {
    match protocollo {
        Protocollo::V1Legacy => Ok(json!({
            "level": documento_del_livello(valutazione.level),
            "reasons": valutazione
                .reasons
                .iter()
                .map(documento_della_ragione)
                .collect::<Vec<_>>(),
        })),
        Protocollo::V2 => sezione_di_fedelta(valutazione, BYTE_PER_SEZIONE).map(|(v, _)| v),
    }
}

#[cfg(test)]
mod sonde {
    use super::*;

    fn rapporto_con(categorie: usize, byte_per_id: usize) -> LossReport {
        let mut rapporto = LossReport::default();
        for i in 0..categorie {
            // Identificatori distinti e tutti della lunghezza voluta: il
            // prefisso numerico li rende diversi senza cambiarne i byte.
            let prefisso = format!("{i:04}_");
            let riempimento = "a".repeat(byte_per_id.saturating_sub(prefisso.len()));
            rapporto.record(&format!("{prefisso}{riempimento}"), u64::MAX);
        }
        rapporto
    }

    #[test]
    fn il_caso_peggiore_dichiarato_entra_nei_dodici_kib() {
        // La promessa del contratto: sessantaquattro categorie da centoventotto
        // byte stanno in una sezione. Non si verifica sulla somma delle
        // lunghezze -- quella e' 8 KiB e direbbe di si' sbagliando -- ma sui
        // byte JSON che escono davvero, virgolette, chiavi e conteggi compresi.
        let rapporto = rapporto_con(MAX_CATEGORIE, MAX_BYTE_ID_CATEGORIA);
        let (documento, troncamento) =
            sezione_di_perdita(&rapporto, BYTE_PER_SEZIONE).expect("il budget basta");
        assert_eq!(
            troncamento,
            Troncamento::default(),
            "il caso peggiore dichiarato non deve troncare niente"
        );
        let byte = serde_json::to_string(&documento)
            .expect("serializzabile")
            .len();
        assert!(
            byte <= BYTE_PER_SEZIONE,
            "il caso peggiore occupa {byte} byte su {BYTE_PER_SEZIONE}"
        );
        assert_eq!(
            documento["counts"].as_array().map(Vec::len),
            Some(MAX_CATEGORIE)
        );
    }

    #[test]
    fn la_sessantacinquesima_categoria_resta_fuori_ed_e_dichiarata() {
        let rapporto = rapporto_con(MAX_CATEGORIE + 1, 16);
        let (documento, troncamento) =
            sezione_di_perdita(&rapporto, BYTE_PER_SEZIONE).expect("il budget basta");
        assert_eq!(troncamento.categorie_omesse, 1);
        assert_eq!(
            troncamento.omesse_per_byte, 0,
            "non e' il budget ad averla tolta"
        );
        assert_eq!(documento["troncato"], json!(true));
        assert_eq!(
            documento["counts"].as_array().map(Vec::len),
            Some(MAX_CATEGORIE)
        );
    }

    #[test]
    fn i_conteggi_pubblicati_restano_esatti() {
        // Si omette una voce intera, mai si riscrive un conteggio: un `12` al
        // posto di `300` sarebbe un valore che significa due cose.
        let mut rapporto = LossReport::default();
        rapporto.record("una", 300);
        let (documento, _) = sezione_di_perdita(&rapporto, BYTE_PER_SEZIONE).expect("budget");
        assert_eq!(documento["counts"][0]["conteggio"], json!(300_u64));
    }

    #[test]
    fn il_taglio_non_dipende_dall_ordine_di_inserimento() {
        // Stesso insieme, ordini d'inserimento opposti, stesso documento: il
        // troncamento segue l'ordine canonico degli identificatori, non quello
        // in cui i driver hanno registrato le perdite.
        let mut avanti = LossReport::default();
        let mut indietro = LossReport::default();
        let identificatori: Vec<String> = (0..MAX_CATEGORIE + 10)
            .map(|i| format!("{i:04}_categoria"))
            .collect();
        for id in &identificatori {
            avanti.record(id, 1);
        }
        for id in identificatori.iter().rev() {
            indietro.record(id, 1);
        }
        let (uno, primo) = sezione_di_perdita(&avanti, BYTE_PER_SEZIONE).expect("budget");
        let (due, secondo) = sezione_di_perdita(&indietro, BYTE_PER_SEZIONE).expect("budget");
        assert_eq!(
            uno, due,
            "l'ordine d'inserimento non deve cambiare l'uscita"
        );
        assert_eq!(primo, secondo);
    }

    #[test]
    fn il_budget_esaurito_e_una_causa_a_parte() {
        // Poche categorie, budget stretto: nessuna soglia di cardinalita' e'
        // superata, eppure qualcosa resta fuori. Le due cause non si mescolano.
        let rapporto = rapporto_con(8, 64);
        let (documento, troncamento) =
            sezione_di_perdita(&rapporto, 512).expect("la struttura minima entra");
        assert_eq!(troncamento.categorie_omesse, 0);
        assert!(troncamento.omesse_per_byte > 0);
        assert_eq!(documento["troncato"], json!(true));
        let byte = serde_json::to_string(&documento)
            .expect("serializzabile")
            .len();
        assert!(byte <= 512, "{byte} byte oltre il budget di 512");
    }

    #[test]
    fn senza_spazio_per_la_dichiarazione_si_fallisce_chiusi() {
        // La sola cosa peggiore di una diagnostica troncata e' una troncata che
        // tace: se la dichiarazione non entra, non esce un documento.
        let rapporto = rapporto_con(1, 16);
        assert_eq!(
            sezione_di_perdita(&rapporto, 8),
            Err(BudgetInsufficiente),
            "otto byte non bastano nemmeno alla struttura minima"
        );
    }

    #[test]
    fn le_ragioni_entrano_in_ordine_canonico_e_il_resto_e_dichiarato() {
        let mut valutazione = FidelityAssessment {
            level: Fidelity::Approximating,
            reasons: Vec::new(),
        };
        for i in 0..MAX_FIDELITY_REASONS {
            valutazione.add_reason(FidelityReasonCode::AttributeLoss, format!("{i:04}"));
        }
        let (documento, troncamento) =
            sezione_di_fedelta(&valutazione, BYTE_PER_SEZIONE).expect("budget");
        assert_eq!(troncamento, Troncamento::default());
        let ragioni = documento["reasons"].as_array().expect("elenco");
        assert_eq!(ragioni.len(), MAX_FIDELITY_REASONS);
        let dettagli: Vec<&str> = ragioni
            .iter()
            .filter_map(|r| r["detail"].as_str())
            .collect();
        let mut ordinati = dettagli.clone();
        ordinati.sort_unstable();
        assert_eq!(dettagli, ordinati, "le ragioni escono in ordine canonico");
    }

    #[test]
    fn una_sezione_di_fedelta_senza_spazio_fallisce_chiusa() {
        let valutazione = FidelityAssessment {
            level: Fidelity::Lossless,
            reasons: Vec::new(),
        };
        assert_eq!(
            sezione_di_fedelta(&valutazione, 8),
            Err(BudgetInsufficiente)
        );
    }

    #[test]
    fn un_identificatore_di_centoventinove_byte_resta_fuori() {
        // 128 byte entrano, 129 no. Il tetto e' sull'identificatore e non sullo
        // spazio: quella categoria resterebbe fuori anche in una sezione vuota.
        for (byte, atteso_fuori) in [(MAX_BYTE_ID_CATEGORIA, 0), (MAX_BYTE_ID_CATEGORIA + 1, 1)] {
            let mut rapporto = LossReport::default();
            rapporto.record(&"a".repeat(byte), 1);
            let (documento, troncamento) =
                sezione_di_perdita(&rapporto, BYTE_PER_SEZIONE).expect("budget");
            assert_eq!(
                troncamento.omesse_per_byte, atteso_fuori,
                "{byte} byte: {documento}"
            );
            assert_eq!(troncamento.categorie_omesse, 0, "non e' la cardinalita'");
        }
    }

    #[test]
    fn un_identificatore_unicode_si_misura_in_byte() {
        // 64 «à» sono 128 byte e passano; 65 sono 130 e no, pur restando 65
        // caratteri. La voce esce **intera**: il taglio non spezza un carattere.
        for (caratteri, atteso_fuori) in [(64_usize, 0_u64), (65, 1)] {
            let identificatore = "à".repeat(caratteri);
            assert_eq!(identificatore.len(), caratteri * 2);
            let mut rapporto = LossReport::default();
            rapporto.record(&identificatore, 1);
            let (_, troncamento) = sezione_di_perdita(&rapporto, BYTE_PER_SEZIONE).expect("budget");
            assert_eq!(
                troncamento.omesse_per_byte,
                atteso_fuori,
                "{caratteri} caratteri = {} byte",
                identificatore.len()
            );
        }
    }

    #[test]
    fn un_dettaglio_di_cinquecentotredici_byte_resta_fuori() {
        for (byte, dentro) in [(MAX_BYTE_DETTAGLIO, 1_usize), (MAX_BYTE_DETTAGLIO + 1, 0)] {
            let mut valutazione = FidelityAssessment {
                level: Fidelity::Approximating,
                reasons: Vec::new(),
            };
            valutazione.add_reason(FidelityReasonCode::AttributeLoss, "x".repeat(byte));
            let (documento, troncamento) =
                sezione_di_fedelta(&valutazione, BYTE_PER_SEZIONE).expect("budget");
            assert_eq!(
                documento["reasons"].as_array().map(Vec::len),
                Some(dentro),
                "{byte} byte di dettaglio"
            );
            assert_eq!(troncamento.omesse_per_byte, 1 - dentro as u64);
        }
    }

    #[test]
    fn il_tetto_complessivo_e_verificato_sull_insieme() {
        // Cinque sezioni ciascuna dentro i propri dodici KiB non dicono niente
        // sull'aggregato: e' la ragione per cui questo controllo esiste. Con le
        // cinque piene si sta dentro; con una sesta no -- e la sesta non esiste,
        // ma il controllo deve accorgersene se un giorno esistesse.
        let rapporto = rapporto_con(MAX_CATEGORIE, MAX_BYTE_ID_CATEGORIA);
        let (sezione, _) = sezione_di_perdita(&rapporto, BYTE_PER_SEZIONE).expect("budget");
        let cinque: Vec<&Value> = (0..SEZIONI).map(|_| &sezione).collect();
        let totale = diagnostica_entro_il_totale(&cinque).expect("cinque sezioni piene stanno");
        assert!(
            totale <= MAX_BYTE_BUSTA,
            "{totale} byte oltre il tetto di {MAX_BYTE_BUSTA}"
        );

        let sei: Vec<&Value> = (0..=SEZIONI).map(|_| &sezione).collect();
        assert_eq!(
            diagnostica_entro_il_totale(&sei),
            Err(BudgetInsufficiente),
            "una sezione in piu' deve far fallire il totale, non passare in silenzio"
        );
    }

    #[test]
    fn la_forma_scritta_a_mano_coincide_col_derive() {
        // La forma sul filo e' scritta a mano perche' i nomi sono contratto e
        // non devono seguire i nomi Rust per distrazione. Ma due scritture
        // divergono, e divergono in silenzio: questa sonda le lega.
        for livello in [
            Fidelity::Lossless,
            Fidelity::Conditional,
            Fidelity::Approximating,
        ] {
            assert_eq!(
                documento_del_livello(livello),
                serde_json::to_value(livello).expect("un enum semplice si serializza"),
                "{livello:?}"
            );
        }
        for codice in [
            FidelityReasonCode::AssessmentPending,
            FidelityReasonCode::FormatConstraint,
            FidelityReasonCode::GeometryApproximation,
            FidelityReasonCode::StructureChanged,
            FidelityReasonCode::AttributeLoss,
            FidelityReasonCode::TypeCoercion,
            FidelityReasonCode::PrecisionChanged,
            FidelityReasonCode::NullabilityChanged,
            FidelityReasonCode::NativeMetadataLoss,
            FidelityReasonCode::LossReported,
        ] {
            assert_eq!(
                documento_del_codice(codice),
                serde_json::to_value(codice).expect("un enum semplice si serializza"),
                "{codice:?}"
            );
        }
        let ragione = FidelityReason {
            code: FidelityReasonCode::AttributeLoss,
            detail: "con \"virgolette\" e accènti".to_owned(),
        };
        assert_eq!(
            documento_della_ragione(&ragione),
            serde_json::to_value(&ragione).expect("serializzabile")
        );
    }

    #[test]
    fn i_tetti_dichiarati_sommano_al_tetto_della_busta() {
        // I numeri del contratto, verificati fra loro invece che ripetuti a
        // mano: 12 KiB per cinque sezioni piu' 4 KiB di struttura fanno 64 KiB.
        assert_eq!(
            SEZIONI * BYTE_PER_SEZIONE + BYTE_DELLA_STRUTTURA,
            MAX_BYTE_BUSTA
        );
        assert_eq!(MAX_BYTE_BUSTA, 64 * 1024);
    }
}
