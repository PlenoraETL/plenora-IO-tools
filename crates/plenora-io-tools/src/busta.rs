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

use plenora_io_core::descriptor::{ArrowTypeClass, Fidelity};
use plenora_io_core::loss::{
    FidelityAssessment, FidelityReason, FidelityReasonCode, LossExample, LossReport,
    MAX_FIDELITY_REASONS, MAX_LOSS_EXAMPLES,
};
// Il tetto sull'identificatore viene da `plenora-io-core`. Riscriverlo qui ne
// farebbe una seconda definizione, e il gate del manifesto confronta il
// contratto con **una** costante, non con la copia che l'adattatore si tiene.
//
// `MAX_BYTE_DETTAGLIO` non compare piu' qui: il filtro che lo applica si e'
// spostato alla porta, in core, e un adattatore che lo nominasse ancora
// lascerebbe credere che sia lui a farlo rispettare.
pub use plenora_io_core::loss::MAX_BYTE_ID_CATEGORIA;

/// Quante categorie distinte una sezione `counts` puo' pubblicare.
pub const MAX_CATEGORIE: usize = 64;
/// Le sezioni con un budget proprio: tre fedelta' e due rapporti di perdita.
pub const SEZIONI: usize = 5;
/// Il budget di ciascuna sezione, in byte della sua serializzazione JSON.
pub const BYTE_PER_SEZIONE: usize = 12 * 1024;
/// Il budget riservato alla struttura aggregata e alle dichiarazioni.
pub const BYTE_DELLA_STRUTTURA: usize = 4 * 1024;
/// Il tetto complessivo della diagnostica in una busta: 64 KiB.
pub const MAX_BYTE_BUSTA: usize = SEZIONI * BYTE_PER_SEZIONE + BYTE_DELLA_STRUTTURA;

/// Il tetto che ERR-011 pone alla **busta d'errore intera**.
///
/// # Perche' e' distinto da `MAX_BYTE_BUSTA`
///
/// `MAX_BYTE_BUSTA` limita le cinque sezioni diagnostiche di un risultato
/// riuscito: tre fedelta' e due rapporti di perdita. E' nostro, ed e' otto
/// volte piu' stretto di questo.
///
/// ERR-011 parla d'altro: «The compact UTF-8 JSON encoding of a public error
/// MUST NOT exceed 524,288 bytes». L'oggetto misurato e' la busta **completa**
/// -- identita', messaggio, assi, e `row_diagnostics` quando c'e' -- e nessuna
/// costante la limitava. Il registro `contracts/limiti-della-diagnostica.json`
/// lo dichiarava come l'unico asse scoperto dei sette; questo modulo lo chiude.
pub const MAX_BYTE_ERRORE: usize = 524_288;

/// Lo spazio che i campi d'identita' aggiungeranno alla busta.
///
/// La libreria rende la busta **senza** identita': `component`,
/// `component_version` e `command` li mette il binding di processo. Se la
/// libreria misurasse soltanto cio' che ha in mano, una busta al limite
/// passerebbe da lei e sforerebbe dopo, nel punto in cui nessuno guarda piu'.
///
/// La riserva e' generosa per costruzione -- il nome del componente e' una
/// costante, la versione e il comando sono corti -- e generosa e' la direzione
/// giusta: sbagliarla in eccesso taglia un po' prima del necessario, sbagliarla
/// in difetto lascia passare una busta fuori contratto.
pub const RISERVA_IDENTITA: usize = 256;

/// Che cosa e' stato tolto per far stare la busta nel tetto di ERR-011.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RiduzioneDellErrore {
    /// La busta ci stava gia'.
    Nessuna,
    /// Gli esempi della diagnostica di riga sono stati tolti.
    EsempiDiRiga,
    /// L'intera diagnostica di riga e' stata tolta.
    DiagnosticaDiRiga,
    /// Non bastava: resta la sola busta minima con i quattro assi.
    SoloGliAssi,
}

impl RiduzioneDellErrore {
    /// Il codice che la busta ridotta dichiara al posto del suo.
    const fn codice(self) -> Option<&'static str> {
        match self {
            Self::Nessuna => None,
            Self::EsempiDiRiga | Self::DiagnosticaDiRiga => Some("DIAGNOSTICS_TRUNCATED"),
            Self::SoloGliAssi => Some("ERROR_TRUNCATED"),
        }
    }
}

/// I byte della codifica compatta UTF-8, che e' l'unita' in cui ERR-011 misura.
fn byte_compatti(documento: &Value) -> usize {
    serde_json::to_vec(documento).map_or(usize::MAX, |v| v.len())
}

/// Riduce una busta d'errore finche' non sta nel tetto di ERR-011.
///
/// # L'ordine in cui si toglie, e perche' e' quello
///
/// Si toglie prima cio' che costa meno a chi legge. Gli **esempi** della
/// diagnostica di riga sono illustrazioni: `counts` e `observed_total` restano,
/// e chi automatizza decide su quelli. Poi l'intera diagnostica, che e'
/// facoltativa per contratto. Da ultimo restano i quattro assi piu' il codice,
/// che sono cio' su cui una macchina **deve** poter decidere: toglierli
/// renderebbe l'errore illeggibile invece che piu' corto.
///
/// Ogni riduzione si **dichiara**, sostituendo il codice: una busta accorciata
/// in silenzio direbbe a chi la riceve che la diagnostica non c'era, mentre la
/// verita' e' che non ci stava. Sono due cose diverse, e la seconda si puo'
/// chiedere di nuovo con un'invocazione piu' stretta.
///
/// # Che cosa questa funzione non promette
///
/// Che il caso si presenti. Nessun percorso d'errore del prodotto arriva vicino
/// a mezzo megabyte: `row_diagnostics` ha un tetto di 64 esempi e il messaggio
/// e' curato. E' una guardia, e una guardia che non scatta mai va comunque
/// provata -- altrimenti si scopre che non funziona il giorno in cui serve.
#[must_use]
pub fn entro_il_tetto_dell_errore(
    mut documento: Value,
    riserva: usize,
) -> (Value, RiduzioneDellErrore) {
    let tetto = MAX_BYTE_ERRORE.saturating_sub(riserva);
    if byte_compatti(&documento) <= tetto {
        return (documento, RiduzioneDellErrore::Nessuna);
    }

    // 1. Gli esempi della diagnostica di riga.
    if let Some(diagnostica) = documento
        .get_mut("error")
        .and_then(|e| e.get_mut("row_diagnostics"))
        .and_then(Value::as_object_mut)
    {
        diagnostica.insert("examples".to_owned(), Value::Array(Vec::new()));
        diagnostica.insert("examples_truncated".to_owned(), Value::Bool(true));
    }
    if byte_compatti(&documento) <= tetto {
        return (
            dichiara(documento, RiduzioneDellErrore::EsempiDiRiga),
            RiduzioneDellErrore::EsempiDiRiga,
        );
    }

    // 2. L'intera diagnostica di riga.
    if let Some(errore) = documento.get_mut("error").and_then(Value::as_object_mut) {
        errore.remove("row_diagnostics");
        errore.remove("details");
    }
    if byte_compatti(&documento) <= tetto {
        return (
            dichiara(documento, RiduzioneDellErrore::DiagnosticaDiRiga),
            RiduzioneDellErrore::DiagnosticaDiRiga,
        );
    }

    // 3. I soli assi. Il messaggio sparisce con tutto il resto: e' curato e non
    //    porta dati della sorgente, quindi perderlo costa la leggibilita' e non
    //    l'informazione su cui si decide.
    let minima = json!({
        "status": "error",
        "protocol_version": PROTOCOLLO,
        "contract": "plenora-error-v1",
        "error": {
            "category": documento["error"]["category"].clone(),
            "phase": documento["error"]["phase"].clone(),
            "remote_effect": documento["error"]["remote_effect"].clone(),
            "retry": documento["error"]["retry"].clone(),
            "code": "ERROR_TRUNCATED",
            "message": "busta oltre il tetto del contratto: restano i soli assi",
        },
    });
    (minima, RiduzioneDellErrore::SoloGliAssi)
}

/// Sostituisce il codice con quello che dichiara la riduzione.
fn dichiara(mut documento: Value, riduzione: RiduzioneDellErrore) -> Value {
    if let (Some(codice), Some(errore)) = (
        riduzione.codice(),
        documento.get_mut("error").and_then(Value::as_object_mut),
    ) {
        errore.insert("code".to_owned(), Value::String(codice.to_owned()));
    }
    documento
}

/// La versione del protocollo che ogni busta dichiara.
///
/// Una sola, e per questo una costante invece di un tipo. Fino alla 3.0.0 era
/// un enum con due varianti, e il binario sapeva consegnare entrambe: il
/// profilo pubblico vieta a un artefatto di servire due versioni del protocollo
/// JSON, e un consumatore che ne trovava due nello stesso binario non poteva
/// sapere quale gli sarebbe arrivata senza leggere il comando.
pub const PROTOCOLLO: u64 = 2;

/// Il nome di contratto di una busta.
///
/// # Da dove viene il nome
///
/// CLI-2.0 §5 chiama questo campo «operation-specific, namespaced contract
/// identifier», e §10 esige che il contratto d'uscita resti **equivalente**
/// fra le superfici. Quindi dove un contratto fissato assegna gia' un
/// identificatore, quello e' il nome: non c'e' niente da scegliere.
///
/// Fino alla 3.0.0 tutte le buste portavano il suffisso `v2`, che e' quello
/// del **protocollo** — un'altra cosa dalla versione dell'operazione. La
/// coincidenza reggeva finche' nessuno confrontava; confrontando, cinque nomi
/// su nove erano diversi da quelli del catalogo comune.
///
/// # Perche' la tabella sta anche in `contracts/nomi-dei-contratti.json`
///
/// Perche' il confronto con la fonte dev'essere **eseguibile**. Due voci
/// smentiscono la somiglianza dei nomi, ed e' esattamente il caso in cui una
/// sostituzione meccanica avrebbe sbagliato:
///
/// * `read` rende `plenora-io-read-result-v1` e non `plenora-io-read-v1`: il
///   catalogo distingue l'ingresso `…-read-input-v1` dall'uscita, e il
///   segmento `result` serve a quello;
/// * l'errore e' `plenora-error-v1` **senza** `io`, perche' SURF-015 dice che
///   i fallimenti pubblici mappano sul contratto d'errore **comune**, che ha
///   un nome suo. Resta nostro `plenora-io-error-details-v1`, che e' il
///   contenuto facoltativo di `details`, non la busta.
///
/// `--version` e' l'unica busta senza un contratto fissato a cui allinearsi:
/// non e' un'operazione del catalogo ma una superficie di scoperta, quindi il
/// suffisso resta quello del protocollo ed e' l'unico caso in cui `v2`
/// significa ancora quello che dice.
#[must_use]
pub fn contratto(nome: &str) -> String {
    match nome {
        "catalog" => "plenora-io-catalog-v1".to_owned(),
        "inspect" => "plenora-io-inspect-v1".to_owned(),
        "layers" => "plenora-io-layers-v1".to_owned(),
        "read" => "plenora-io-read-result-v1".to_owned(),
        "write" => "plenora-io-write-result-v1".to_owned(),
        "convert" => "plenora-io-convert-v1".to_owned(),
        // Condiviso, non nostro: lo emettono anche gli altri componenti.
        "capabilities" => "plenora-capabilities-v2".to_owned(),
        // La sola busta il cui nome resta nostro, e il solo `v2` che sia
        // ancora quello del protocollo.
        _ => format!("plenora-io-{nome}-v2"),
    }
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
    /// Esempi oltre `MAX_LOSS_EXAMPLES`.
    ///
    /// Il campo esisteva prima degli esempi sul filo, e valeva sempre zero: era
    /// dichiarato dall'inizio perche' e' contratto, e un campo che compare piu'
    /// tardi e' un cambiamento di protocollo. Ora il v2 li pubblica -- la
    /// redazione c'e', e il `context` non porta piu' nomi presi dal file --
    /// quindi il contatore conta davvero.
    pub esempi_omessi: u64,
    /// Voci lasciate fuori per un **limite in byte**: quello della singola
    /// voce -- un identificatore oltre 128 byte, un dettaglio oltre 512 -- o
    /// quello della sezione. Contate a parte dalle tre soglie di cardinalita',
    /// perche' «sono troppe» e «non ci stanno» sono due cose diverse.
    pub omesse_per_byte: u64,
}

impl Troncamento {
    /// La dichiarazione **piu' grande** che questa sezione potra' emettere.
    ///
    /// Si misura con questa, non con quella vuota. La prima stesura riservava
    /// lo spazio della dichiarazione a zero e poi ci scriveva i contatori veri:
    /// una sezione che ne tronca ottantamila sostituisce `0` con cinque cifre,
    /// e il documento finale usciva **oltre** il budget su cui era stato deciso
    /// il taglio. I dodici KiB devono comprendere anche cio' che la sezione
    /// dichiara di aver tolto, se no il tetto vale per un documento diverso da
    /// quello che esce.
    ///
    /// Lo spazio si riserva a `u64::MAX` -- venti cifre -- e non al massimo che
    /// l'ingresso corrente potrebbe produrre: il contatore e' un `u64` e niente
    /// nel tipo lo limita piu' in basso. Una stima piu' stretta rimetterebbe il
    /// tetto in mano a quante voci porta il file, che e' esattamente cio' che i
    /// limiti esistono per togliergli.
    ///
    /// `troncato` sta a `false` e non a `true` perche' `false` e' la stringa
    /// **piu' lunga** delle due: il caso peggiore e' quello, per quanto suoni
    /// strano che il peggiore sia il caso in cui non si e' tolto niente.
    fn segnaposto_massimo() -> Value {
        Self {
            categorie_omesse: u64::MAX,
            ragioni_omesse: u64::MAX,
            esempi_omessi: u64::MAX,
            omesse_per_byte: u64::MAX,
        }
        .documento()
    }

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
    // `false` e non `true`: e' la stringa piu' lunga delle due, e lo spazio si
    // riserva al caso peggiore come per la dichiarazione di troncamento.
    base.insert("omesse_esatte".to_owned(), json!(false));
    base.insert("omesse".to_owned(), Troncamento::segnaposto_massimo());
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
    troncamento.omesse_per_byte = troncamento
        .omesse_per_byte
        .saturating_add(rapporto.respinti_per_misura());

    // 3. Gli esempi, **fino al budget residuo**: sono l'ultimo blocco
    //    dell'ordine canonico perche' sono la voce piu' sacrificabile. I
    //    conteggi devono uscire interi anche quando gli esempi non ci stanno,
    //    mai il contrario: un conteggio troncato mentirebbe su un numero, un
    //    esempio omesso e' un'illustrazione in meno e viene dichiarata.
    //
    //    Arrivano gia' tutti ammissibili -- categoria entro i 128 byte,
    //    contesto entro i 512 -- perche' il filtro sta alla porta di
    //    `LossReport`, e gia' in ordine canonico.
    let mut documento = base;
    documento.insert("counts".to_owned(), counts);
    troncamento.esempi_omessi = rapporto
        .esempi_trattenuti()
        .saturating_sub(MAX_LOSS_EXAMPLES) as u64;
    let esempi_candidati: Vec<_> = rapporto.esempi_canonici().take(MAX_LOSS_EXAMPLES).collect();
    let (esempi, esempi_per_byte) = entro_il_budget(
        &documento,
        "esempi",
        &esempi_candidati,
        budget,
        |accumulatore: &mut Value, esempio: &&LossExample| {
            if let Value::Array(voci) = accumulatore {
                voci.push(documento_dell_esempio(esempio));
            }
        },
    );
    troncamento.omesse_per_byte = troncamento.omesse_per_byte.saturating_add(esempi_per_byte);

    documento.insert("esempi".to_owned(), esempi);
    documento.insert(
        "troncato".to_owned(),
        json!(!troncamento.niente_di_omesso()),
    );
    documento.insert("omesse_esatte".to_owned(), json!(rapporto.omesse_esatte()));
    documento.insert("omesse".to_owned(), troncamento.documento());
    Ok((Value::Object(documento), troncamento))
}

/// Un esempio diagnostico nella sua forma sul filo.
///
/// Scritto a mano come tutti i nomi del protocollo, e una sonda pretende che
/// coincida col derive: cosi' il derive non puo' divergere da questo senza
/// diventare rosso.
fn documento_dell_esempio(esempio: &LossExample) -> Value {
    let mut documento = Map::new();
    documento.insert("category".to_owned(), json!(esempio.category));
    if let Some(indice) = esempio.posizione.layer_index {
        documento.insert("layer_index".to_owned(), json!(indice));
    }
    if let Some(indice) = esempio.posizione.field_index {
        documento.insert("field_index".to_owned(), json!(indice));
    }
    if let Some(classe) = esempio.posizione.type_class {
        documento.insert("type_class".to_owned(), documento_della_classe(classe));
    }
    documento.insert("context".to_owned(), json!(esempio.context));
    Value::Object(documento)
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
    // `false` e non `true`: e' la stringa piu' lunga delle due, e lo spazio si
    // riserva al caso peggiore come per la dichiarazione di troncamento.
    base.insert("omesse_esatte".to_owned(), json!(false));
    base.insert("omesse".to_owned(), Troncamento::segnaposto_massimo());
    if byte_serializzati(&Value::Object(base.clone())) > budget {
        return Err(BudgetInsufficiente);
    }

    // Il filtro sui byte **non e' qui**: sta alla porta di `FidelityAssessment`,
    // dove le ragioni entrano. Partizionare adesso lascerebbe le voci fuori
    // misura occupare un posto nel trattenimento e sfrattare voci valide, e la
    // sezione uscirebbe piu' povera di quanto il tetto imponga. Qui arrivano
    // gia' tutte ammissibili e gia' in ordine canonico.
    troncamento.omesse_per_byte = valutazione.respinte_per_misura();
    troncamento.ragioni_omesse = valutazione
        .ragioni_trattenute()
        .saturating_sub(MAX_FIDELITY_REASONS) as u64;
    let candidate: Vec<_> = valutazione
        .ragioni_canoniche()
        .take(MAX_FIDELITY_REASONS)
        .collect();

    let (reasons, per_byte) =
        entro_il_budget(&base, "reasons", &candidate, budget, |acc, ragione| {
            if let Value::Array(voci) = acc {
                voci.push(documento_della_ragione_v2(ragione));
            }
        });
    troncamento.omesse_per_byte = troncamento.omesse_per_byte.saturating_add(per_byte);

    let mut documento = base;
    documento.insert("reasons".to_owned(), reasons);
    documento.insert(
        "troncato".to_owned(),
        json!(!troncamento.niente_di_omesso()),
    );
    // I quattro contatori sono esatti? Non e' una quinta causa di omissione: e'
    // un qualificatore sull'esattezza delle quattro. Vale `false` per qualunque
    // perdita di esattezza interna -- un trattenimento saturo, una voce
    // respinta per misura -- e allora i quattro sono **limiti inferiori**. Una
    // diagnostica che tacesse la propria approssimazione sarebbe peggio di una
    // troncata, che almeno lo dichiara.
    documento.insert(
        "omesse_esatte".to_owned(),
        json!(valutazione.omesse_esatte()),
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
fn documento_della_ragione_v2(ragione: &FidelityReason) -> Value {
    let mut documento = Map::new();
    documento.insert("code".to_owned(), documento_del_codice(ragione.code));
    documento.insert("detail".to_owned(), json!(ragione.detail));
    if let Some(indice) = ragione.posizione.layer_index {
        documento.insert("layer_index".to_owned(), json!(indice));
    }
    if let Some(indice) = ragione.posizione.field_index {
        documento.insert("field_index".to_owned(), json!(indice));
    }
    if let Some(classe) = ragione.posizione.type_class {
        documento.insert("type_class".to_owned(), documento_della_classe(classe));
    }
    Value::Object(documento)
}

/// La classe di tipo nella sua forma sul filo.
///
/// Riusa `ArrowTypeClass::nome()`, che quella mappatura ce l'ha gia': scriverne
/// qui una seconda avrebbe aggiunto una terza rappresentazione delle stesse
/// dieci stringhe -- col `Serialize` derivato a fare da terza -- dentro il
/// lotto che le copie esiste per toglierle. La sonda
/// `la_forma_scritta_a_mano_coincide_col_derive` pretende che adattatore e
/// derive coincidano, quindi la catena resta inchiodata da un capo all'altro.
fn documento_della_classe(classe: ArrowTypeClass) -> Value {
    json!(classe.nome())
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
pub fn diagnostica_entro_il_totale(
    sezioni: &[(&str, &Value)],
) -> Result<usize, BudgetInsufficiente> {
    // La serializzazione **finale** dell'oggetto diagnostico, non la somma
    // nominale delle cinque quote: sommare i numeri del contratto direbbe
    // sempre 61 440 e non guarderebbe mai i nomi delle chiavi, le graffe, le
    // virgole -- cioe' i byte che escono. I quattro KiB sono riserva
    // strutturale **dentro** questo tetto, non un addendo da mettergli accanto,
    // e non sono un limite sul resto della risposta, che diagnostica non e'.
    let mut diagnostica = Map::new();
    for (nome, valore) in sezioni {
        diagnostica.insert((*nome).to_owned(), (*valore).clone());
    }
    let totale = byte_serializzati(&Value::Object(diagnostica));
    if totale > MAX_BYTE_BUSTA {
        return Err(BudgetInsufficiente);
    }
    Ok(totale)
}

/// Il rapporto di perdita, nella forma del protocollo corrente.
///
/// # Errors
///
/// `BudgetInsufficiente` quando nemmeno la dichiarazione di troncamento entra
/// nel budget della sezione.
pub fn documento_di_perdita(
    valutazione: &FidelityAssessment,
    rapporto: &LossReport,
) -> Result<Value, BudgetInsufficiente> {
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

/// La valutazione di fedelta', nella forma del protocollo corrente.
///
/// # Errors
///
/// `BudgetInsufficiente` quando nemmeno il livello con la sua dichiarazione
/// entra nel budget della sezione.
pub fn documento_di_fedelta(
    valutazione: &FidelityAssessment,
) -> Result<Value, BudgetInsufficiente> {
    sezione_di_fedelta(valutazione, BYTE_PER_SEZIONE).map(|(v, _)| v)
}

#[cfg(test)]
mod sonde;
