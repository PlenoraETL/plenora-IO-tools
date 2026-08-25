//! Analisi WKT **limitata durante il parse** (lotto S12).
//!
//! # Perche' non basta un tetto a valle
//!
//! Fino a questo modulo il WKT veniva dato alla crate `wkt`, che costruisce
//! l'albero **intero** e poi lo restituisce: i tetti del bordo -- vertici,
//! profondita' -- si applicavano dopo, su una struttura gia' allocata. Un
//! tetto che si applica dopo non e' un tetto: e' un rendiconto. Una cella
//! ostile da qualche megabyte otteneva la memoria che chiedeva e veniva
//! rifiutata dopo averla ottenuta.
//!
//! L'unica difesa che c'era e' il cap in **byte** sul testo, esatto e
//! grossolano: dice quanto puo' essere lungo l'input, non quanto puo' costare.
//! Sessantaquattro megabyte di `LINESTRING` sono quattro milioni di vertici, e
//! quattro milioni di vertici stanno sotto il cap dei byte e sopra ogni altro
//! limite dichiarato.
//!
//! # Che cosa fa questo modulo
//!
//! Consuma il testo da sinistra a destra e costruisce la geometria mentre
//! consuma. Ogni coordinata e ogni geometria figlia sono **addebitate nel
//! momento in cui vengono lette**, e la profondita' e' un parametro della
//! discesa: il rifiuto arriva al token che supera il tetto, non alla fine.
//! Cio' che non e' stato letto non e' stato allocato.
//!
//! # L'unita' di conteggio, che e' quella del bordo
//!
//! Un componente e' una **coordinata** o una **geometria figlia**, come conta
//! `inspect_geometry` in `plenora-io-model`. E' la stessa lezione del lotto
//! S11: due tetti con lo stesso nome e due unita' di misura diverse sono
//! peggio di due tetti con nomi diversi, perche' nessuno va a guardare.
//!
//! # Che cosa cambia dell'insieme accettato: una cosa sola
//!
//! Restano i sette tipi classici con le loro regole di coerenza dimensionale,
//! le due sintassi di `MULTIPOINT`, il suffisso dimensionale attaccato o
//! staccato, `POINT EMPTY` non rappresentabile nel core WKB, e la verifica di
//! esprimibilita' a valle. Una sonda lo prova contro il parser precedente su
//! oltre trecento casi generati per combinazione.
//!
//! L'unica variazione e' deliberata: **il testo non-whitespace dopo la
//! geometria e' rifiutato**. La crate `wkt` lo ignorava, e per lei
//! `POINT (1 2))` e `POINT (1 2) POINT (3 4)` erano un punto e il resto non
//! c'era. Una cella WKT rappresenta una geometria completa: ignorare una
//! parentesi in piu' o una seconda geometria nasconde un input malformato, e
//! contraddirebbe la garanzia che questo modulo esiste per dare. E' un bug del
//! confine precedente, non una sintassi da conservare.
//!
//! Il rifiuto e' di **sintassi**, non di budget: il testo residuo non e' un
//! superamento di tetto, e classificarlo come tale direbbe a chi legge
//! l'errore di allargare una quota che non c'entra. Lo spazio finale --
//! spazi, tabulazioni, ritorni a capo -- resta accettato, perche' non e'
//! testo.

use plenora_io_model::contract::CoordinateDimensions;
use plenora_io_model::limits::WkbLimits;
use plenora_io_model::wkb::{WkbCoordinate, WkbGeometry, WkbValue};
use plenora_io_model::{NumeroStrutturale, PlenoraIoError, PublicMessage, Result};

/// Il prefisso del sottosistema, lo stesso dell'adattatore.
const PREFISSO: &str = "WKT:";

fn errore(messaggio: &'static str) -> PlenoraIoError {
    PlenoraIoError::wkb_redatto(&PublicMessage::CuratedPair(PREFISSO, messaggio))
}

/// Un rifiuto che porta **quanto** era il tetto, e nient'altro dell'input.
///
/// I numeri ammessi al bordo sono indici, conteggi, tetti e codici
/// strutturali: un limite e' un tetto, e dirlo aiuta chi deve allargarlo.
fn oltre_il_tetto(messaggio: &'static str, tetto: usize) -> PlenoraIoError {
    PlenoraIoError::limite_redatto(&PublicMessage::CuratedBetween(
        PREFISSO,
        NumeroStrutturale::Limite(crate::saturating_u64(tetto)),
        messaggio,
        NumeroStrutturale::Limite(crate::saturating_u64(tetto)),
    ))
}

/// Lo stato dell'analisi: dove siamo nel testo, e quanto budget resta.
struct Analizzatore<'a> {
    // Il WKT e' ASCII: scorrerlo a byte evita di ragionare sui confini dei
    // caratteri, e un byte non ASCII e' comunque un errore di sintassi.
    testo: &'a [u8],
    posizione: usize,
    componenti_residui: usize,
    profondita_massima: usize,
}

impl<'a> Analizzatore<'a> {
    const fn nuovo(testo: &'a str, limiti: &WkbLimits) -> Self {
        Self {
            testo: testo.as_bytes(),
            posizione: 0,
            componenti_residui: limiti.max_components,
            profondita_massima: limiti.max_depth,
        }
    }

    /// Addebita un componente letto.
    ///
    /// Qui sta la differenza fra questo modulo e cio' che c'era prima: il
    /// budget cala **mentre** si legge, quindi il rifiuto arriva sulla
    /// coordinata che supera il tetto e non sull'albero finito.
    fn addebita(&mut self) -> Result<()> {
        self.componenti_residui = self.componenti_residui.checked_sub(1).ok_or_else(|| {
            oltre_il_tetto("componenti oltre il limite di", self.componenti_residui)
        })?;
        Ok(())
    }

    fn salta_spazi(&mut self) {
        while matches!(
            self.testo.get(self.posizione),
            Some(b' ' | b'\t' | b'\r' | b'\n')
        ) {
            self.posizione += 1;
        }
    }

    fn guarda(&mut self) -> Option<u8> {
        self.salta_spazi();
        self.testo.get(self.posizione).copied()
    }

    fn se_prossimo(&mut self, atteso: u8) -> bool {
        if self.guarda() == Some(atteso) {
            self.posizione += 1;
            return true;
        }
        false
    }

    fn attende(&mut self, atteso: u8, messaggio: &'static str) -> Result<()> {
        if self.se_prossimo(atteso) {
            return Ok(());
        }
        Err(errore(messaggio))
    }

    /// Una parola di sole lettere ASCII, resa maiuscola in un buffer fisso.
    ///
    /// Il buffer e' fisso perche' le parole del vocabolario WKT sono note e
    /// corte: una parola piu' lunga non e' una parola del vocabolario, e
    /// allocarla per poi rifiutarla darebbe a un input ostile un modo di far
    /// allocare a piacere.
    fn parola(&mut self) -> Parola {
        self.salta_spazi();
        let mut buffer = [0_u8; PAROLA_MASSIMA];
        let mut quante = 0;
        while let Some(byte) = self.testo.get(self.posizione) {
            if !byte.is_ascii_alphabetic() {
                break;
            }
            if quante < PAROLA_MASSIMA {
                buffer[quante] = byte.to_ascii_uppercase();
            }
            quante += 1;
            self.posizione += 1;
        }
        Parola { buffer, quante }
    }

    /// Un numero in virgola mobile, delimitato dai caratteri che WKT ammette.
    fn numero(&mut self) -> Result<f64> {
        self.salta_spazi();
        let principio = self.posizione;
        while let Some(byte) = self.testo.get(self.posizione) {
            if byte.is_ascii_digit() || matches!(byte, b'+' | b'-' | b'.' | b'e' | b'E') {
                self.posizione += 1;
            } else {
                break;
            }
        }
        if principio == self.posizione {
            return Err(errore("coordinata attesa"));
        }
        // Il testo del numero non esce mai: se non e' un numero, il messaggio
        // dice che si aspettava un numero, non quale non-numero e' arrivato.
        let grezzo = self
            .testo
            .get(principio..self.posizione)
            .and_then(|byte| std::str::from_utf8(byte).ok())
            .ok_or_else(|| errore("coordinata non leggibile"))?;
        let valore: f64 = grezzo
            .parse()
            .map_err(|_| errore("coordinata non numerica"))?;
        if !valore.is_finite() {
            return Err(errore("coordinata non finita"));
        }
        Ok(valore)
    }

    /// Fine del testo, al netto degli spazi.
    fn alla_fine(&mut self) -> bool {
        self.salta_spazi();
        self.posizione >= self.testo.len()
    }
}

/// La lunghezza massima di una parola del vocabolario, `GEOMETRYCOLLECTION`.
const PAROLA_MASSIMA: usize = 18;

/// Una parola letta dall'input, senza allocazione.
struct Parola {
    buffer: [u8; PAROLA_MASSIMA],
    quante: usize,
}

impl Parola {
    fn e(&self, atteso: &str) -> bool {
        self.quante == atteso.len()
            && self
                .buffer
                .get(..self.quante)
                .is_some_and(|letto| letto == atteso.as_bytes())
    }

    const fn vuota(&self) -> bool {
        self.quante == 0
    }

    const fn copia(&self) -> Self {
        Self {
            buffer: self.buffer,
            quante: self.quante,
        }
    }

    /// La stessa parola senza il suffisso indicato, se ce l'ha.
    fn senza_coda(&self, coda: &str) -> Option<Self> {
        let nuda = self.quante.checked_sub(coda.len())?;
        let letto = self.buffer.get(..self.quante)?;
        if !letto.ends_with(coda.as_bytes()) {
            return None;
        }
        Some(Self {
            buffer: self.buffer,
            quante: nuda,
        })
    }
}

/// Quante ordinate porta una dimensionalita'.
const fn ordinate(dimensioni: CoordinateDimensions) -> usize {
    match dimensioni {
        CoordinateDimensions::Xy | CoordinateDimensions::Unknown => 2,
        CoordinateDimensions::Xyz | CoordinateDimensions::Xym => 3,
        CoordinateDimensions::Xyzm => 4,
    }
}

/// Analizza un testo WKT applicando i tetti **mentre** lo consuma.
///
/// # Errors
///
/// Sintassi non valida, dimensionalita' incoerente, coordinata non finita,
/// oppure il superamento di uno dei tetti dichiarati in `limiti`.
pub fn analizza(testo: &str, limiti: &WkbLimits) -> Result<WkbGeometry> {
    let mut analizzatore = Analizzatore::nuovo(testo, limiti);
    let geometria = geometria(&mut analizzatore, 0, None)?;
    if !analizzatore.alla_fine() {
        return Err(errore("testo residuo dopo la geometria"));
    }
    Ok(geometria)
}

/// Una geometria completa: tag, dimensionalita', corpo.
///
/// `attese` sono le dimensionalita' che il genitore impone: un figlio con
/// dimensionalita' diversa e' un rifiuto, ed e' la stessa regola che
/// l'adattatore applicava sull'albero gia' costruito.
fn geometria(
    analizzatore: &mut Analizzatore,
    profondita: usize,
    attese: Option<CoordinateDimensions>,
) -> Result<WkbGeometry> {
    if profondita > analizzatore.profondita_massima {
        return Err(oltre_il_tetto(
            "annidamento oltre il limite di",
            analizzatore.profondita_massima,
        ));
    }
    let parola = analizzatore.parola();
    if parola.vuota() {
        return Err(errore("tipo di geometria atteso"));
    }
    // `POINT Z` e `POINTZ` sono la stessa cosa, e la seconda forma esce da
    // writer veri. La sonda comparativa con il parser precedente l'ha trovata:
    // leggere la parola per intero e non riconoscerla sarebbe stata una
    // regressione grammaticale, cioe' esattamente cio' che quella sonda esiste
    // per impedire.
    let (tag, attaccate) = tipo_e_suffisso(&parola)?;
    let dimensioni = match attaccate {
        Some(attaccate) => {
            if attese.is_some_and(|attese| attese != attaccate) {
                return Err(errore("geometria annidata con dimensionalità incoerente"));
            }
            Some(attaccate)
        }
        None => dimensioni_dichiarate(analizzatore, attese)?,
    };

    if tag.e("POINT") {
        punto(analizzatore, dimensioni, attese)
    } else if tag.e("LINESTRING") {
        sequenza(analizzatore, dimensioni, attese)
    } else if tag.e("POLYGON") {
        poligono(analizzatore, dimensioni, attese)
    } else if tag.e("MULTIPOINT") {
        multipunto(analizzatore, dimensioni, attese)
    } else if tag.e("MULTILINESTRING") {
        multisequenza(analizzatore, dimensioni, attese)
    } else if tag.e("MULTIPOLYGON") {
        multipoligono(analizzatore, dimensioni, attese)
    } else if tag.e("GEOMETRYCOLLECTION") {
        collezione(analizzatore, profondita, dimensioni, attese)
    } else {
        // `tipo_e_suffisso` ha gia' verificato che il nome sia uno dei sette:
        // questo ramo resta perche' il `match` sia esaustivo, e vale il giorno
        // in cui l'elenco `TIPI` e questa catena smettessero di coincidere.
        Err(errore("tipo di geometria non riconosciuto"))
    }
}

/// Il nome del tipo e il suffisso dimensionale che gli sta attaccato.
///
/// `POINTZ` e' `POINT` piu' `Z`. Il taglio si prova solo quando la parola
/// intera non e' un tipo noto: `POINT` finisce per `T`, e nessun tipo finisce
/// per `Z` o `M`, quindi non c'e' ambiguita' da risolvere.
fn tipo_e_suffisso(parola: &Parola) -> Result<(Parola, Option<CoordinateDimensions>)> {
    if TIPI.iter().any(|tipo| parola.e(tipo)) {
        return Ok((parola.copia(), None));
    }
    for (suffisso, dimensioni) in [
        ("ZM", CoordinateDimensions::Xyzm),
        ("Z", CoordinateDimensions::Xyz),
        ("M", CoordinateDimensions::Xym),
    ] {
        if let Some(nudo) = parola.senza_coda(suffisso) {
            if TIPI.iter().any(|tipo| nudo.e(tipo)) {
                return Ok((nudo, Some(dimensioni)));
            }
        }
    }
    Err(errore("tipo di geometria non riconosciuto"))
}

/// I sette tipi che il core WKB rappresenta, e nessun altro.
const TIPI: [&str; 7] = [
    "POINT",
    "LINESTRING",
    "POLYGON",
    "MULTIPOINT",
    "MULTILINESTRING",
    "MULTIPOLYGON",
    "GEOMETRYCOLLECTION",
];

/// La dimensionalita' dichiarata dal tag `Z`/`M`/`ZM`, se c'e'.
///
/// `None` vuol dire «da dedurre dalla prima coordinata», che e' cio' che fa
/// la grammatica quando il tag manca: `POINT (1 2 3)` e' XYZ.
fn dimensioni_dichiarate(
    analizzatore: &mut Analizzatore,
    attese: Option<CoordinateDimensions>,
) -> Result<Option<CoordinateDimensions>> {
    let principio = analizzatore.posizione;
    let parola = analizzatore.parola();
    let dichiarate = if parola.e("Z") {
        Some(CoordinateDimensions::Xyz)
    } else if parola.e("M") {
        Some(CoordinateDimensions::Xym)
    } else if parola.e("ZM") {
        Some(CoordinateDimensions::Xyzm)
    } else {
        // Non era un suffisso dimensionale: puo' essere `EMPTY`, o niente.
        analizzatore.posizione = principio;
        None
    };
    if let (Some(dichiarate), Some(attese)) = (dichiarate, attese) {
        if dichiarate != attese {
            return Err(errore("geometria annidata con dimensionalità incoerente"));
        }
    }
    Ok(dichiarate)
}

/// `EMPTY` al posto del corpo?
fn e_vuota(analizzatore: &mut Analizzatore) -> bool {
    let principio = analizzatore.posizione;
    if analizzatore.parola().e("EMPTY") {
        return true;
    }
    analizzatore.posizione = principio;
    false
}

/// Una coordinata, addebitata nel momento in cui viene letta.
///
/// Restituisce anche la dimensionalita' che ha **osservato**: e' cosi' che si
/// decide quella della geometria quando il tag non la dichiara, e cosi' che si
/// scopre una coordinata incoerente con quella che la contiene.
fn coordinata(
    analizzatore: &mut Analizzatore,
    dichiarate: Option<CoordinateDimensions>,
) -> Result<(WkbCoordinate, CoordinateDimensions)> {
    analizzatore.addebita()?;
    let x = analizzatore.numero()?;
    let y = analizzatore.numero()?;
    let mut lette = 2;
    let mut terza = None;
    let mut quarta = None;
    while lette < 4 {
        let prossimo = analizzatore.guarda();
        if !matches!(prossimo, Some(byte) if byte.is_ascii_digit() || matches!(byte, b'+' | b'-' | b'.'))
        {
            break;
        }
        let valore = analizzatore.numero()?;
        if lette == 2 {
            terza = Some(valore);
        } else {
            quarta = Some(valore);
        }
        lette += 1;
    }
    let osservate = dedotte(lette);

    // I due rami fanno cose diverse, e non sono la stessa espressione scritta
    // in due modi: quando la dimensionalita' e' **dichiarata** va verificata
    // contro le ordinate lette -- e' li' che `POINT Z (1 2)` viene rifiutato --
    // mentre quando non lo e' la si legge dalla coordinata. Non c'e' un valore
    // di ripiego: c'e' una regola che dipende da chi ha parlato per primo.
    let scelte = match dichiarate {
        Some(dichiarate) => {
            if ordinate(dichiarate) != lette {
                return Err(PlenoraIoError::wkb_redatto(&PublicMessage::CuratedPair(
                    "WKT: coordinata con dimensionalità incoerente con la geometria:",
                    osservate.nome(),
                )));
            }
            dichiarate
        }
        None => osservate,
    };

    // Con tre ordinate e' il tag a dire se la terza e' Z o M; senza tag e' Z.
    let (z, m) = match scelte {
        CoordinateDimensions::Xyz => (terza, None),
        CoordinateDimensions::Xym => (None, terza),
        CoordinateDimensions::Xyzm => (terza, quarta),
        CoordinateDimensions::Xy | CoordinateDimensions::Unknown => (None, None),
    };
    Ok((WkbCoordinate { x, y, z, m }, scelte))
}

/// Una geometria costruita dai suoi pezzi, senza SRID.
const fn costruita(value: WkbValue, dimensions: CoordinateDimensions) -> WkbGeometry {
    WkbGeometry {
        value,
        dimensions,
        srid: None,
    }
}

/// La dimensionalita' di un aggregato, decisa dal **primo** elemento.
///
/// Non e' un dettaglio di implementazione: e' la regola della grammatica.
/// `MULTIPOINT (1 2, 3 4 5)` e' incoerente perche' il primo punto ha gia'
/// deciso, e scriverla come un valore di ripiego -- «se non si sa, XY» --
/// direbbe che esiste un caso in cui nessuno ha deciso. Non esiste: l'aggregato
/// vuoto non entra mai qui, e ogni altro ha almeno un elemento.
fn concordata(
    dichiarate: Option<CoordinateDimensions>,
    attese: Option<CoordinateDimensions>,
    del_primo: CoordinateDimensions,
) -> Result<CoordinateDimensions> {
    if dichiarate.is_some_and(|dichiarate| dichiarate != del_primo) {
        return Err(errore("geometria con dimensionalità incoerente"));
    }
    if attese.is_some_and(|attese| attese != del_primo) {
        return Err(errore("geometria annidata con dimensionalità incoerente"));
    }
    Ok(del_primo)
}

/// La dimensionalita' di un aggregato **vuoto**, che nessun elemento decide.
///
/// Senza elementi non c'e' niente da osservare: vale cio' che il tag dichiara,
/// o cio' che il genitore impone, o XY. Qui il valore di ripiego e' la
/// grammatica, non una degradazione -- `MULTIPOINT EMPTY` e' XY per
/// definizione, non perche' non si sia riusciti a leggerlo.
fn del_vuoto(
    dichiarate: Option<CoordinateDimensions>,
    attese: Option<CoordinateDimensions>,
) -> Result<CoordinateDimensions> {
    match (dichiarate, attese) {
        (Some(dichiarate), Some(attese)) if dichiarate != attese => {
            Err(errore("geometria annidata con dimensionalità incoerente"))
        }
        (Some(scelte), _) | (None, Some(scelte)) => Ok(scelte),
        (None, None) => Ok(CoordinateDimensions::Xy),
    }
}

const fn dedotte(ordinate_lette: usize) -> CoordinateDimensions {
    match ordinate_lette {
        2 => CoordinateDimensions::Xy,
        3 => CoordinateDimensions::Xyz,
        _ => CoordinateDimensions::Xyzm,
    }
}

fn punto(
    analizzatore: &mut Analizzatore,
    dichiarate: Option<CoordinateDimensions>,
    attese: Option<CoordinateDimensions>,
) -> Result<WkbGeometry> {
    if e_vuota(analizzatore) {
        // Invariato dall'adattatore: il core WKB non ha un punto vuoto, e
        // fingere che l'abbia produrrebbe un round-trip che perde.
        return Err(errore("POINT EMPTY non rappresentabile nel core WKB"));
    }
    analizzatore.attende(b'(', "parentesi aperta attesa dopo POINT")?;
    let (coordinata, osservate) = coordinata(analizzatore, dichiarate.or(attese))?;
    analizzatore.attende(b')', "parentesi chiusa attesa dopo la coordinata")?;
    let dimensioni = concordata(dichiarate, attese, osservate)?;
    Ok(costruita(WkbValue::Point(coordinata), dimensioni))
}

/// Una `LineString`, che e' una sequenza di coordinate fra parentesi.
fn sequenza(
    analizzatore: &mut Analizzatore,
    dichiarate: Option<CoordinateDimensions>,
    attese: Option<CoordinateDimensions>,
) -> Result<WkbGeometry> {
    let (coordinate, dimensioni) = coordinate(analizzatore, dichiarate, attese, true)?;
    Ok(costruita(WkbValue::LineString(coordinate), dimensioni))
}

/// Le coordinate fra parentesi, con la dimensionalita' che la prima decide.
///
/// `ammette_vuota` distingue una `LineString`, che puo' essere `EMPTY`, da un
/// anello di poligono, che dentro le parentesi del poligono non lo e' mai.
fn coordinate(
    analizzatore: &mut Analizzatore,
    dichiarate: Option<CoordinateDimensions>,
    attese: Option<CoordinateDimensions>,
    ammette_vuota: bool,
) -> Result<(Vec<WkbCoordinate>, CoordinateDimensions)> {
    if ammette_vuota && e_vuota(analizzatore) {
        return Ok((Vec::new(), del_vuoto(dichiarate, attese)?));
    }
    analizzatore.attende(b'(', "parentesi aperta attesa")?;
    let (prima, osservate) = coordinata(analizzatore, dichiarate.or(attese))?;
    let dimensioni = concordata(dichiarate, attese, osservate)?;
    let mut lette = vec![prima];
    while analizzatore.se_prossimo(b',') {
        let (altra, _) = coordinata(analizzatore, Some(dimensioni))?;
        lette.push(altra);
    }
    analizzatore.attende(b')', "parentesi chiusa attesa dopo le coordinate")?;
    Ok((lette, dimensioni))
}

fn poligono(
    analizzatore: &mut Analizzatore,
    dichiarate: Option<CoordinateDimensions>,
    attese: Option<CoordinateDimensions>,
) -> Result<WkbGeometry> {
    let (anelli, dimensioni) = anelli(analizzatore, dichiarate, attese)?;
    Ok(costruita(WkbValue::Polygon(anelli), dimensioni))
}

fn anelli(
    analizzatore: &mut Analizzatore,
    dichiarate: Option<CoordinateDimensions>,
    attese: Option<CoordinateDimensions>,
) -> Result<(Vec<Vec<WkbCoordinate>>, CoordinateDimensions)> {
    if e_vuota(analizzatore) {
        return Ok((Vec::new(), del_vuoto(dichiarate, attese)?));
    }
    analizzatore.attende(b'(', "parentesi aperta attesa dopo POLYGON")?;
    let (primo, dimensioni) = coordinate(analizzatore, dichiarate, attese, false)?;
    let mut fuori = vec![primo];
    while analizzatore.se_prossimo(b',') {
        let (altro, _) = coordinate(analizzatore, Some(dimensioni), Some(dimensioni), false)?;
        fuori.push(altro);
    }
    analizzatore.attende(b')', "parentesi chiusa attesa dopo gli anelli")?;
    Ok((fuori, dimensioni))
}

/// `MULTIPOINT` accetta due sintassi, e le accettava anche prima.
///
/// `MULTIPOINT (1 2, 3 4)` e `MULTIPOINT ((1 2), (3 4))` sono lo stesso
/// oggetto: la prima e' quella che scrivono quasi tutti, la seconda quella
/// dello standard.
fn multipunto(
    analizzatore: &mut Analizzatore,
    dichiarate: Option<CoordinateDimensions>,
    attese: Option<CoordinateDimensions>,
) -> Result<WkbGeometry> {
    if e_vuota(analizzatore) {
        return Ok(costruita(
            WkbValue::MultiPoint(Vec::new()),
            del_vuoto(dichiarate, attese)?,
        ));
    }
    analizzatore.attende(b'(', "parentesi aperta attesa dopo MULTIPOINT")?;
    let (prima, osservate) = punto_membro(analizzatore, dichiarate.or(attese))?;
    let dimensioni = concordata(dichiarate, attese, osservate)?;
    let mut figli = vec![costruita(WkbValue::Point(prima), dimensioni)];
    while analizzatore.se_prossimo(b',') {
        let (altra, _) = punto_membro(analizzatore, Some(dimensioni))?;
        figli.push(costruita(WkbValue::Point(altra), dimensioni));
    }
    analizzatore.attende(b')', "parentesi chiusa attesa dopo i punti")?;
    Ok(costruita(WkbValue::MultiPoint(figli), dimensioni))
}

/// Un membro di `MULTIPOINT`, con o senza le sue parentesi.
fn punto_membro(
    analizzatore: &mut Analizzatore,
    dimensioni: Option<CoordinateDimensions>,
) -> Result<(WkbCoordinate, CoordinateDimensions)> {
    analizzatore.addebita()?;
    if e_vuota(analizzatore) {
        return Err(errore("POINT EMPTY annidato non rappresentabile"));
    }
    let fra_parentesi = analizzatore.se_prossimo(b'(');
    let letta = coordinata(analizzatore, dimensioni)?;
    if fra_parentesi {
        analizzatore.attende(b')', "parentesi chiusa attesa dopo il punto")?;
    }
    Ok(letta)
}

fn multisequenza(
    analizzatore: &mut Analizzatore,
    dichiarate: Option<CoordinateDimensions>,
    attese: Option<CoordinateDimensions>,
) -> Result<WkbGeometry> {
    if e_vuota(analizzatore) {
        return Ok(costruita(
            WkbValue::MultiLineString(Vec::new()),
            del_vuoto(dichiarate, attese)?,
        ));
    }
    analizzatore.attende(b'(', "parentesi aperta attesa dopo MULTILINESTRING")?;
    analizzatore.addebita()?;
    let (prima, dimensioni) = coordinate(analizzatore, dichiarate, attese, true)?;
    let mut figli = vec![costruita(WkbValue::LineString(prima), dimensioni)];
    while analizzatore.se_prossimo(b',') {
        analizzatore.addebita()?;
        let (altra, _) = coordinate(analizzatore, Some(dimensioni), Some(dimensioni), true)?;
        figli.push(costruita(WkbValue::LineString(altra), dimensioni));
    }
    analizzatore.attende(b')', "parentesi chiusa attesa dopo le linee")?;
    Ok(costruita(WkbValue::MultiLineString(figli), dimensioni))
}

fn multipoligono(
    analizzatore: &mut Analizzatore,
    dichiarate: Option<CoordinateDimensions>,
    attese: Option<CoordinateDimensions>,
) -> Result<WkbGeometry> {
    if e_vuota(analizzatore) {
        return Ok(costruita(
            WkbValue::MultiPolygon(Vec::new()),
            del_vuoto(dichiarate, attese)?,
        ));
    }
    analizzatore.attende(b'(', "parentesi aperta attesa dopo MULTIPOLYGON")?;
    analizzatore.addebita()?;
    let (primo, dimensioni) = anelli(analizzatore, dichiarate, attese)?;
    let mut figli = vec![costruita(WkbValue::Polygon(primo), dimensioni)];
    while analizzatore.se_prossimo(b',') {
        analizzatore.addebita()?;
        let (altro, _) = anelli(analizzatore, Some(dimensioni), Some(dimensioni))?;
        figli.push(costruita(WkbValue::Polygon(altro), dimensioni));
    }
    analizzatore.attende(b')', "parentesi chiusa attesa dopo i poligoni")?;
    Ok(costruita(WkbValue::MultiPolygon(figli), dimensioni))
}

/// L'unico tipo che annida geometrie complete, e quindi l'unico che fa
/// crescere la profondita'.
fn collezione(
    analizzatore: &mut Analizzatore,
    profondita: usize,
    dichiarate: Option<CoordinateDimensions>,
    attese: Option<CoordinateDimensions>,
) -> Result<WkbGeometry> {
    if e_vuota(analizzatore) {
        return Ok(costruita(
            WkbValue::GeometryCollection(Vec::new()),
            del_vuoto(dichiarate, attese)?,
        ));
    }
    analizzatore.attende(b'(', "parentesi aperta attesa dopo GEOMETRYCOLLECTION")?;
    let profondita_figlio = profondita
        .checked_add(1)
        .ok_or_else(|| errore("annidamento non rappresentabile"))?;
    analizzatore.addebita()?;
    let primo = geometria(analizzatore, profondita_figlio, dichiarate.or(attese))?;
    let dimensioni = concordata(dichiarate, attese, primo.dimensions)?;
    let mut figli = vec![primo];
    while analizzatore.se_prossimo(b',') {
        analizzatore.addebita()?;
        let figlio = geometria(analizzatore, profondita_figlio, Some(dimensioni))?;
        if figlio.dimensions != dimensioni {
            return Err(errore(
                "GeometryCollection con dimensionalità annidate differenti",
            ));
        }
        figli.push(figlio);
    }
    analizzatore.attende(b')', "parentesi chiusa attesa dopo le geometrie")?;
    Ok(costruita(WkbValue::GeometryCollection(figli), dimensioni))
}

/// Quanto testo l'analisi ha consumato prima di fermarsi.
///
/// Serve alle sonde, ed e' l'unico modo di provare la differenza fra «rifiuta»
/// e «rifiuta **prima**»: un tetto applicato a valle avrebbe consumato tutto
/// l'input prima di dire di no.
#[cfg(test)]
fn consumato_prima_del_rifiuto(testo: &str, limiti: &WkbLimits) -> usize {
    let mut analizzatore = Analizzatore::nuovo(testo, limiti);
    let _ = geometria(&mut analizzatore, 0, None);
    analizzatore.posizione
}

/// Quanti componenti l'analisi ha addebitato per un testo accettato.
#[cfg(test)]
fn componenti_usati(testo: &str, limiti: &WkbLimits) -> usize {
    let mut analizzatore = Analizzatore::nuovo(testo, limiti);
    let esito = geometria(&mut analizzatore, 0, None);
    assert!(esito.is_ok(), "il testo doveva essere accettato");
    limiti.max_components - analizzatore.componenti_residui
}

/// L'adattatore **storico**, conservato come oracolo di confronto.
///
/// # Perche' esiste ancora
///
/// La sostituzione di un parser e' il tipo di cambiamento che rompe la
/// grammatica senza rompere un test: l'insieme accettato e' molto piu' grande
/// del corpus di sonde che lo descrive, e «il workspace e' verde» non dice che
/// i due parser accettino le stesse cose. Dice che accettano le cose che
/// qualcuno ha gia' scritto.
///
/// Qui il vecchio percorso -- `wkt 0.14.0` che costruisce l'albero, piu'
/// l'adattatore che lo converte -- resta disponibile ai soli test, e la sonda
/// del confronto lo interroga su un corpus generato: stessi ingressi, stessa
/// risposta, stessa geometria. Le uniche divergenze ammesse sono i rifiuti che
/// il nuovo confine aggiunge, cioe' i tetti.
///
/// # Quando si potra' togliere
///
/// Quando la crate `wkt` uscira' anche dalla scrittura. Finche' e' una
/// dipendenza comunque presente, tenerla come oracolo costa nulla e prova
/// qualcosa che nessun'altra sonda prova.
#[cfg(test)]
mod adattatore_storico {
    use plenora_io_model::contract::CoordinateDimensions;
    use plenora_io_model::wkb::{WkbCoordinate, WkbGeometry, WkbValue};
    use plenora_io_model::{PlenoraIoError, PublicMessage, Result};
    use wkt::types::{Coord, Dimension};
    use wkt::Wkt;

    fn error(message: &'static str) -> PlenoraIoError {
        PlenoraIoError::wkb_redatto(&PublicMessage::CuratedPair("WKT:", message))
    }

    fn validate_finite_coordinate(x: f64, y: f64, z: Option<f64>, m: Option<f64>) -> Result<()> {
        if !x.is_finite()
            || !y.is_finite()
            || z.is_some_and(|value| !value.is_finite())
            || m.is_some_and(|value| !value.is_finite())
        {
            return Err(error("coordinata non finita"));
        }
        Ok(())
    }

    const fn contract_dimensions(dimension: Dimension) -> CoordinateDimensions {
        match dimension {
            Dimension::XY => CoordinateDimensions::Xy,
            Dimension::XYZ => CoordinateDimensions::Xyz,
            Dimension::XYM => CoordinateDimensions::Xym,
            Dimension::XYZM => CoordinateDimensions::Xyzm,
        }
    }

    fn coordinate_from_wkt(
        coordinate: &Coord<f64>,
        expected: CoordinateDimensions,
    ) -> Result<WkbCoordinate> {
        let actual = contract_dimensions(coordinate.dimension());
        if actual != expected {
            // La dimensionalita' attesa e' quella della geometria, che il
            // chiamante ha in mano: nel messaggio resta quella osservata, che e'
            // l'informazione che lui non ha.
            return Err(PlenoraIoError::wkb_redatto(&PublicMessage::CuratedPair(
                "WKT: coordinata con dimensionalità incoerente con la geometria:",
                actual.nome(),
            )));
        }
        validate_finite_coordinate(coordinate.x, coordinate.y, coordinate.z, coordinate.m)?;
        Ok(WkbCoordinate {
            x: coordinate.x,
            y: coordinate.y,
            z: coordinate.z,
            m: coordinate.m,
        })
    }

    fn coordinates_from_wkt(
        coordinates: &[Coord<f64>],
        expected: CoordinateDimensions,
    ) -> Result<Vec<WkbCoordinate>> {
        coordinates
            .iter()
            .map(|coordinate| coordinate_from_wkt(coordinate, expected))
            .collect()
    }

    // Dispatch esaustivo sui rami del tipo WKT: la lunghezza e' nel numero di
    // varianti, non in complessita' logica.
    #[allow(clippy::too_many_lines)]
    fn geometry_from_wkt(value: &Wkt<f64>) -> Result<WkbGeometry> {
        let (value, dimensions) = match value {
            Wkt::Point(point) => {
                let dimensions = contract_dimensions(point.dimension());
                let coordinate = point
                    .coord()
                    .ok_or_else(|| error("POINT EMPTY non rappresentabile nel core WKB"))?;
                (
                    WkbValue::Point(coordinate_from_wkt(coordinate, dimensions)?),
                    dimensions,
                )
            }
            Wkt::LineString(line) => {
                let dimensions = contract_dimensions(line.dimension());
                (
                    WkbValue::LineString(coordinates_from_wkt(line.coords(), dimensions)?),
                    dimensions,
                )
            }
            Wkt::Polygon(polygon) => {
                let dimensions = contract_dimensions(polygon.dimension());
                let rings = polygon
                    .rings()
                    .iter()
                    .map(|ring| {
                        if contract_dimensions(ring.dimension()) != dimensions {
                            return Err(error("anello Polygon con dimensionalità incoerente"));
                        }
                        coordinates_from_wkt(ring.coords(), dimensions)
                    })
                    .collect::<Result<Vec<_>>>()?;
                (WkbValue::Polygon(rings), dimensions)
            }
            Wkt::MultiPoint(multipoint) => {
                let dimensions = contract_dimensions(multipoint.dimension());
                let children = multipoint
                    .points()
                    .iter()
                    .map(|point| {
                        if contract_dimensions(point.dimension()) != dimensions {
                            return Err(error("Point annidato con dimensionalità incoerente"));
                        }
                        let coordinate = point
                            .coord()
                            .ok_or_else(|| error("POINT EMPTY annidato non rappresentabile"))?;
                        Ok(WkbGeometry {
                            value: WkbValue::Point(coordinate_from_wkt(coordinate, dimensions)?),
                            dimensions,
                            srid: None,
                        })
                    })
                    .collect::<Result<Vec<_>>>()?;
                (WkbValue::MultiPoint(children), dimensions)
            }
            Wkt::MultiLineString(multiline) => {
                let dimensions = contract_dimensions(multiline.dimension());
                let children = multiline
                    .line_strings()
                    .iter()
                    .map(|line| {
                        if contract_dimensions(line.dimension()) != dimensions {
                            return Err(error("LineString annidata con dimensionalità incoerente"));
                        }
                        Ok(WkbGeometry {
                            value: WkbValue::LineString(coordinates_from_wkt(
                                line.coords(),
                                dimensions,
                            )?),
                            dimensions,
                            srid: None,
                        })
                    })
                    .collect::<Result<Vec<_>>>()?;
                (WkbValue::MultiLineString(children), dimensions)
            }
            Wkt::MultiPolygon(multipolygon) => {
                let dimensions = contract_dimensions(multipolygon.dimension());
                let children = multipolygon
                    .polygons()
                    .iter()
                    .map(|polygon| {
                        if contract_dimensions(polygon.dimension()) != dimensions {
                            return Err(error("Polygon annidato con dimensionalità incoerente"));
                        }
                        let rings = polygon
                            .rings()
                            .iter()
                            .map(|ring| {
                                if contract_dimensions(ring.dimension()) != dimensions {
                                    return Err(error(
                                        "anello MultiPolygon con dimensionalità incoerente",
                                    ));
                                }
                                coordinates_from_wkt(ring.coords(), dimensions)
                            })
                            .collect::<Result<Vec<_>>>()?;
                        Ok(WkbGeometry {
                            value: WkbValue::Polygon(rings),
                            dimensions,
                            srid: None,
                        })
                    })
                    .collect::<Result<Vec<_>>>()?;
                (WkbValue::MultiPolygon(children), dimensions)
            }
            Wkt::GeometryCollection(collection) => {
                let dimensions = contract_dimensions(collection.dimension());
                let children = collection
                    .geometries()
                    .iter()
                    .map(|child| {
                        let child = geometry_from_wkt(child)?;
                        if child.dimensions != dimensions {
                            return Err(error(
                                "GeometryCollection con dimensionalità annidate differenti",
                            ));
                        }
                        Ok(child)
                    })
                    .collect::<Result<Vec<_>>>()?;
                (WkbValue::GeometryCollection(children), dimensions)
            }
        };
        Ok(WkbGeometry {
            value,
            dimensions,
            srid: None,
        })
    }

    /// Il percorso di lettura come era prima del lotto S12.
    pub(super) fn analizza_come_prima(testo: &str) -> Result<WkbGeometry> {
        let albero: Wkt<f64> = testo
            .parse()
            .map_err(|_| error("sintassi WKT non valida"))?;
        let geometria = geometry_from_wkt(&albero)?;
        crate::wkt_lossless::verifica_esprimibile(&geometria)?;
        Ok(geometria)
    }
}

#[cfg(test)]
mod sonde {
    use super::{analizza, componenti_usati, consumato_prima_del_rifiuto};
    use plenora_io_model::contract::CoordinateDimensions;
    use plenora_io_model::limits::WkbLimits;
    use plenora_io_model::wkb::{encode_wkb, inspect_wkb, WkbFlavor, WkbValue};

    fn stretti(componenti: usize, profondita: usize) -> WkbLimits {
        WkbLimits {
            max_components: componenti,
            max_depth: profondita,
            ..WkbLimits::default()
        }
    }

    /// **Il fatto che il lotto S12 esiste per stabilire.**
    ///
    /// Un tetto applicato a valle avrebbe consumato tutto l'input prima di
    /// dire di no: l'albero si costruisce per intero e poi lo si misura. Qui
    /// il rifiuto arriva alla coordinata che supera il tetto, e la posizione
    /// raggiunta lo dimostra.
    #[test]
    fn il_rifiuto_arriva_prima_della_fine_del_testo() {
        let mut testo = String::from("LINESTRING (");
        for indice in 0..5_000 {
            if indice > 0 {
                testo.push(',');
            }
            testo.push_str("1 2");
        }
        testo.push(')');

        let limiti = stretti(10, 64);
        assert!(analizza(&testo, &limiti).is_err());

        let consumato = consumato_prima_del_rifiuto(&testo, &limiti);
        assert!(
            consumato < testo.len() / 100,
            "consumati {consumato} byte su {}: il rifiuto non e' arrivato presto",
            testo.len()
        );
    }

    /// Lo stesso fatto, visto da fuori e senza guardare la posizione.
    ///
    /// La coda e' sintatticamente impossibile. Se l'analisi arrivasse in fondo
    /// prima di misurare, il rifiuto sarebbe quello di sintassi; siccome
    /// misura mentre legge, la coda non viene nemmeno guardata.
    #[test]
    fn il_rifiuto_e_quello_del_tetto_non_quello_della_coda() {
        let mut testo = String::from("LINESTRING (1 2,3 4,5 6,7 8,9 10");
        testo.push_str(", questa coda non e' un WKT e non deve essere letta");
        let errore = analizza(&testo, &stretti(3, 64)).unwrap_err();
        let reso = format!("{errore}");
        assert!(
            reso.contains("limite") || reso.contains("componenti"),
            "atteso il rifiuto del tetto, ottenuto: {reso}"
        );
    }

    /// L'annidamento e' un parametro della discesa, non una misura finale.
    #[test]
    fn l_annidamento_oltre_il_tetto_e_rifiutato() {
        let profondo = |livelli: usize| {
            let mut testo = String::new();
            for _ in 0..livelli {
                testo.push_str("GEOMETRYCOLLECTION (");
            }
            testo.push_str("POINT (1 2)");
            for _ in 0..livelli {
                testo.push(')');
            }
            testo
        };
        let limiti = stretti(1_000, 4);
        assert!(
            analizza(&profondo(4), &limiti).is_ok(),
            "quattro livelli stanno nel tetto"
        );
        assert!(analizza(&profondo(6), &limiti).is_err());
    }

    /// Il tetto sui componenti, provato **esattamente al confine**.
    ///
    /// E' la sonda che deve reggere il peso: il target di fuzzing non arriva a
    /// centomila coordinate -- non stanno in un input da quattro kilobyte --
    /// quindi sotto fuzzing quel ramo non e' esercitato, e i registri delle
    /// misure lo dicono. Qui si prova al confine e non «da qualche parte
    /// sopra»: `n` passa, `n+1` no, per ogni forma che conta i componenti in
    /// un modo diverso.
    #[test]
    fn il_tetto_sui_componenti_e_esatto() {
        // (testo, componenti che costa). I costi sono quelli del bordo:
        // una coordinata ciascuna, piu' una per ogni geometria figlia.
        let casi: [(&str, usize); 6] = [
            ("POINT (1 2)", 1),
            ("LINESTRING (0 0,1 1,2 2)", 3),
            ("POLYGON ((0 0,1 0,1 1,0 0))", 4),
            ("MULTIPOINT (1 2,3 4)", 4),
            ("MULTILINESTRING ((0 0,1 1),(2 2,3 3))", 6),
            ("GEOMETRYCOLLECTION (POINT (1 2),LINESTRING (0 0,1 1))", 5),
        ];
        for (testo, costo) in casi {
            let esatto = WkbLimits {
                max_components: costo,
                ..WkbLimits::default()
            };
            let stretto = WkbLimits {
                max_components: costo - 1,
                ..WkbLimits::default()
            };
            assert!(
                analizza(testo, &esatto).is_ok(),
                "{testo} costa {costo} componenti e con {costo} deve passare"
            );
            let errore = analizza(testo, &stretto)
                .expect_err(&format!("{testo} con {} deve fallire", costo - 1));
            assert_eq!(
                errore.code,
                plenora_io_model::IoErrorCode::LimitExceeded,
                "{testo}: al confine il rifiuto e' del tetto"
            );
            // E il costo dichiarato e' quello che l'analisi addebita davvero.
            assert_eq!(componenti_usati(testo, &esatto), costo, "{testo}");
        }
    }

    /// L'unita' di conteggio e' quella del bordo, e non «una simile».
    ///
    /// E' la lezione del lotto S11: due tetti con lo stesso nome e due unita'
    /// di misura diverse sono peggio di due tetti con nomi diversi. La sonda
    /// non confronta il codice, confronta i **conteggi**: cio' che l'analisi
    /// addebita leggendo il testo deve essere cio' che `inspect_wkb` conta
    /// sulla stessa geometria in WKB.
    #[test]
    fn i_componenti_coincidono_con_quelli_del_parser_condiviso() {
        let campioni = [
            "POINT (1 2)",
            "LINESTRING (0 0,1 1,2 2)",
            "POLYGON ((0 0,1 0,1 1,0 0))",
            "POLYGON ((0 0,1 0,1 1,0 0),(0 0,1 0,1 1,0 0))",
            "MULTIPOINT (1 2,3 4)",
            "MULTIPOINT ((1 2),(3 4))",
            "MULTILINESTRING ((0 0,1 1),(2 2,3 3))",
            "MULTIPOLYGON (((0 0,1 0,1 1,0 0)))",
            "GEOMETRYCOLLECTION (POINT (1 2),LINESTRING (0 0,1 1))",
            "GEOMETRYCOLLECTION (GEOMETRYCOLLECTION (POINT (1 2)))",
            "POINT Z (1 2 3)",
            "LINESTRING ZM (0 0 0 0,1 1 1 1)",
        ];
        let limiti = WkbLimits::default();
        for testo in campioni {
            let nostri = componenti_usati(testo, &limiti);
            let geometria = analizza(testo, &limiti).expect("campione valido");
            let byte = encode_wkb(&geometria, WkbFlavor::Iso).expect("codificabile");
            let ispezione = inspect_wkb(&byte, &limiti).expect("ispezionabile");
            assert_eq!(
                nostri, ispezione.components,
                "{testo}: l'analisi addebita {nostri}, il parser condiviso conta {}",
                ispezione.components
            );
        }
    }

    /// La grammatica accettata e' quella di prima, e questa sonda la fissa.
    ///
    /// Il lotto sposta *quando* si rifiuta, non *che cosa* si accetta: se
    /// avesse allargato o stretto l'insieme, un file che smette di funzionare
    /// non si saprebbe imputare a quale delle due cose.
    #[test]
    fn la_grammatica_accettata_e_quella_dichiarata() {
        let limiti = WkbLimits::default();
        let casi: [(&str, CoordinateDimensions); 8] = [
            ("POINT (1 2)", CoordinateDimensions::Xy),
            ("POINT (1 2 3)", CoordinateDimensions::Xyz),
            ("POINT Z (1 2 3)", CoordinateDimensions::Xyz),
            ("POINT M (1 2 3)", CoordinateDimensions::Xym),
            ("POINT ZM (1 2 3 4)", CoordinateDimensions::Xyzm),
            ("point (1 2)", CoordinateDimensions::Xy),
            ("  POINT   (  1   2  )  ", CoordinateDimensions::Xy),
            ("LINESTRING EMPTY", CoordinateDimensions::Xy),
        ];
        for (testo, attese) in casi {
            let geometria = analizza(testo, &limiti)
                .unwrap_or_else(|errore| panic!("{testo} doveva essere accettato: {errore}"));
            assert_eq!(geometria.dimensions, attese, "{testo}");
        }

        // Le due sintassi di MULTIPOINT sono lo stesso oggetto.
        assert_eq!(
            analizza("MULTIPOINT (1 2,3 4)", &limiti).unwrap(),
            analizza("MULTIPOINT ((1 2),(3 4))", &limiti).unwrap()
        );

        // I vuoti che il core WKB rappresenta.
        for testo in [
            "MULTIPOINT EMPTY",
            "MULTILINESTRING EMPTY",
            "MULTIPOLYGON EMPTY",
            "GEOMETRYCOLLECTION EMPTY",
            "POLYGON EMPTY",
        ] {
            let geometria = analizza(testo, &limiti)
                .unwrap_or_else(|errore| panic!("{testo} doveva essere accettato: {errore}"));
            let vuota = match &geometria.value {
                WkbValue::MultiPoint(figli)
                | WkbValue::MultiLineString(figli)
                | WkbValue::MultiPolygon(figli)
                | WkbValue::GeometryCollection(figli) => figli.is_empty(),
                WkbValue::Polygon(anelli) => anelli.is_empty(),
                _ => false,
            };
            assert!(vuota, "{testo}");
        }
    }

    /// I rifiuti che c'erano prima restano, con lo stesso significato.
    #[test]
    fn i_rifiuti_strutturali_restano() {
        let limiti = WkbLimits::default();
        let rifiutati = [
            ("POINT EMPTY", "il core WKB non ha un punto vuoto"),
            ("MULTIPOINT (EMPTY)", "ne' un punto vuoto annidato"),
            (
                "POINT (1 2 3 4 5)",
                "cinque ordinate non sono una coordinata",
            ),
            ("POINT Z (1 2)", "il tag dichiara tre ordinate"),
            (
                "LINESTRING (0 0,1 1 1)",
                "coordinate di dimensionalita' diversa",
            ),
            (
                "GEOMETRYCOLLECTION (POINT (1 2),POINT Z (1 2 3))",
                "figli di dimensionalita' diversa",
            ),
            ("POINT (1 2)POINT (3 4)", "testo residuo dopo la geometria"),
            (
                "CIRCULARSTRING (0 0,1 1,2 2)",
                "tipo fuori dall'insieme accettato",
            ),
            ("POINT (1 nan)", "coordinata non numerica"),
            ("POINT", "corpo assente"),
            ("", "testo vuoto"),
        ];
        for (testo, perche) in rifiutati {
            assert!(analizza(testo, &limiti).is_err(), "{testo}: {perche}");
        }
    }

    /// **La prova che la grammatica non e' cambiata.**
    ///
    /// Sostituire un parser rompe l'insieme accettato senza rompere un test:
    /// l'insieme e' molto piu' grande del corpus che lo descrive, e «il
    /// workspace e' verde» dice soltanto che i due parser accettano le cose
    /// che qualcuno ha gia' scritto.
    ///
    /// Qui il confronto e' con il parser **precedente** -- la crate `wkt` piu'
    /// il suo adattatore, conservati per questo -- su un corpus generato per
    /// combinazione: sette tipi, quattro dimensionalita', le forme vuote, le
    /// due sintassi di `MULTIPOINT`, gli spazi, le maiuscole, e una quarantina
    /// di storpiature.
    ///
    /// La regola del confronto ha tre righe e la terza e' quella che conta:
    ///
    /// * accettano entrambi -> la geometria deve essere **la stessa**;
    /// * rifiutano entrambi -> niente da dire;
    /// * il vecchio accetta e il nuovo no -> ammesso **solo** se il nuovo
    ///   rifiuto e' un tetto, che e' cio' che il lotto aggiunge;
    /// * il vecchio rifiuta e il nuovo accetta -> mai. Accettare piu' di prima
    ///   e' una regressione silenziosa, ed e' la direzione che nessun test
    ///   esistente avrebbe visto.
    #[test]
    fn accetta_esattamente_cio_che_accettava_il_parser_precedente() {
        let limiti = WkbLimits::default();
        let corpus = corpus_di_confronto();
        assert!(
            corpus.len() > 200,
            "corpus troppo piccolo: {}",
            corpus.len()
        );

        let mut concordi = 0_usize;
        let mut solo_per_i_tetti = 0_usize;
        let mut piu_stretti = 0_usize;
        for testo in &corpus {
            let prima = super::adattatore_storico::analizza_come_prima(testo);
            let dopo = crate::wkt_lossless::parse_wkt_bounded(testo, &limiti);
            match (prima, dopo) {
                (Ok(prima), Ok(dopo)) => {
                    assert_eq!(prima, dopo, "geometrie diverse per «{testo}»");
                    concordi += 1;
                }
                (Err(_), Err(_)) => concordi += 1,
                (Ok(_), Err(errore)) => {
                    let per_un_tetto = errore.code == plenora_io_model::IoErrorCode::LimitExceeded;
                    assert!(
                        per_un_tetto || per_il_testo_residuo(testo),
                        "«{testo}» era accettato e ora e' rifiutato senza che sia un tetto: {errore}"
                    );
                    if per_un_tetto {
                        solo_per_i_tetti += 1;
                    } else {
                        piu_stretti += 1;
                    }
                }
                (Err(errore), Ok(_)) => {
                    panic!("«{testo}» era rifiutato ({errore}) e ora e' accettato");
                }
            }
        }
        assert_eq!(
            piu_stretti, 3,
            "le divergenze piu' strette sono tre e sono elencate: se cambiano, \
             la decisione sul testo residuo va ripresa"
        );
        assert_eq!(
            concordi + solo_per_i_tetti + piu_stretti,
            corpus.len(),
            "ogni caso deve ricadere in una delle righe della regola"
        );
    }

    /// I tre casi in cui l'analisi progressiva e' **piu' stretta**, elencati.
    ///
    /// # La decisione, presa
    ///
    /// La crate `wkt` ignorava cio' che segue la geometria: `POINT (1 2))` e
    /// `POINT (1 2) POINT (3 4)` per lei erano un punto, e il resto non c'era.
    /// L'analisi progressiva li rifiuta, e il rifiuto **non** viene da un
    /// tetto: e' l'unico irrigidimento del lotto, ed e' deliberato -- una
    /// cella WKT rappresenta una geometria completa, e ignorare il resto
    /// nasconde un input malformato.
    ///
    /// I tre casi restano qui **per nome**, e il loro numero e' asserito: non
    /// perche' la decisione sia sospesa, ma perche' l'eccezione resti chiusa.
    /// Se ne comparisse un quarto, questa sonda diventerebbe rossa invece di
    /// allargarla in silenzio.
    fn per_il_testo_residuo(testo: &str) -> bool {
        [
            "POINT (1 2))",
            "POINT (1 2) POINT (3 4)",
            include_str!("../../../fuzz/seeds/wkt_parse/multipolygon-con-membro-vuoto.wkt"),
        ]
        .contains(&testo)
    }

    /// Il corpus del confronto, generato per combinazione.
    ///
    /// Generato e non scritto a mano: un elenco scritto a mano contiene i casi
    /// a cui si e' pensato, che sono esattamente quelli che i test esistenti
    /// gia' coprono. La combinazione produce anche quelli a cui non si e'
    /// pensato -- ed e' li' che una riscrittura sbaglia.
    // Il corpus e' lungo perche' e' un elenco di casi, non una funzione con
    // logica: dividerlo in tre pezzi renderebbe piu' difficile leggere che cosa
    // copre, che e' la sola cosa che conta qui.
    #[allow(clippy::too_many_lines)]
    fn corpus_di_confronto() -> Vec<String> {
        let dimensioni = ["", " Z", " M", " ZM"];
        let coordinate = ["1 2", "1 2 3", "1 2 3", "1 2 3 4"];
        let mut corpus = Vec::new();

        for (indice, suffisso) in dimensioni.iter().enumerate() {
            let uno = coordinate[indice];
            let due = format!("{uno},{uno}");
            let quattro = format!("{uno},{uno},{uno},{uno}");
            let forme = [
                format!("POINT{suffisso} ({uno})"),
                format!("POINT{suffisso} EMPTY"),
                format!("LINESTRING{suffisso} ({due})"),
                format!("LINESTRING{suffisso} EMPTY"),
                format!("POLYGON{suffisso} (({quattro}))"),
                format!("POLYGON{suffisso} (({quattro}),({quattro}))"),
                format!("POLYGON{suffisso} EMPTY"),
                format!("POLYGON{suffisso} (EMPTY)"),
                format!("MULTIPOINT{suffisso} ({due})"),
                format!("MULTIPOINT{suffisso} (({uno}),({uno}))"),
                format!("MULTIPOINT{suffisso} EMPTY"),
                format!("MULTIPOINT{suffisso} (EMPTY)"),
                format!("MULTILINESTRING{suffisso} (({due}),({due}))"),
                format!("MULTILINESTRING{suffisso} EMPTY"),
                format!("MULTILINESTRING{suffisso} (EMPTY)"),
                format!("MULTIPOLYGON{suffisso} ((({quattro})))"),
                format!("MULTIPOLYGON{suffisso} EMPTY"),
                format!("MULTIPOLYGON{suffisso} (EMPTY)"),
                format!("GEOMETRYCOLLECTION{suffisso} (POINT{suffisso} ({uno}))"),
                format!(
                    "GEOMETRYCOLLECTION{suffisso} (POINT{suffisso} ({uno}),LINESTRING{suffisso} ({due}))"
                ),
                format!("GEOMETRYCOLLECTION{suffisso} EMPTY"),
                format!(
                    "GEOMETRYCOLLECTION{suffisso} (GEOMETRYCOLLECTION{suffisso} (POINT{suffisso} ({uno})))"
                ),
            ];
            for forma in forme {
                // Ogni forma in quattro vesti: com'e', minuscola, con spazi
                // dentro le parentesi, e senza lo spazio prima della parentesi.
                corpus.push(forma.clone());
                corpus.push(forma.to_lowercase());
                corpus.push(forma.replace('(', "(  ").replace(')', "  )"));
                corpus.push(forma.replace(" (", "("));
            }
        }

        // Le parentesi vuote, una per tipo. Mancavano, e la loro assenza ha
        // lasciato passare una regressione nella direzione vietata: il parser
        // nuovo accettava `MULTIPOINT ()` dove il precedente lo rifiutava. La
        // forma vuota e' `EMPTY`; `()` non e' WKT, e l'ha trovato la
        // diagnostica differenziale del livello 2, non questa sonda.
        for tipo in [
            "POINT",
            "LINESTRING",
            "POLYGON",
            "MULTIPOINT",
            "MULTILINESTRING",
            "MULTIPOLYGON",
            "GEOMETRYCOLLECTION",
        ] {
            corpus.push(format!("{tipo} ()"));
            corpus.push(format!("{tipo} (  )"));
            corpus.push(format!("{tipo}()"));
        }

        // Le storpiature: quelle che un file vero produce sbagliando, e quelle
        // che un input ostile produce apposta.
        for storpiatura in [
            "",
            " ",
            "POINT",
            "POINT (",
            "POINT ()",
            "POINT (1)",
            "POINT (1 2",
            "POINT 1 2)",
            "POINT (1 2))",
            "POINT (1 2) POINT (3 4)",
            "POINT (1 2 3 4 5)",
            "POINT (a b)",
            "POINT (1 due)",
            "POINT (1 2,3 4)",
            "POINT Z (1 2)",
            "POINT M (1 2)",
            "POINT ZM (1 2 3)",
            "LINESTRING (1 2)",
            "LINESTRING (,)",
            "LINESTRING (1 2,)",
            "LINESTRING (1 2,3 4 5)",
            "POLYGON (1 2,3 4)",
            "POLYGON ((1 2,3 4)",
            "MULTIPOINT (1 2,(3 4))",
            "MULTIPOINT ((1 2),3 4)",
            "MULTIPOLYGON (((1 2,3 4)),((5 6",
            "GEOMETRYCOLLECTION (POINT (1 2)",
            "GEOMETRYCOLLECTION (GEOMETRYCOLLECTION EMPTY)",
            "GEOMETRYCOLLECTION (POINT (1 2),POINT Z (1 2 3))",
            "CIRCULARSTRING (0 0,1 1,2 2)",
            "TRIANGLE ((0 0,1 0,1 1,0 0))",
            "TIN (((0 0,1 0,1 1,0 0)))",
            "POINT EMPTY EMPTY",
            "EMPTY",
            "POINT (1e2 -3E-1)",
            "POINT (+1 -2)",
            "POINT (1. .2)",
            "POINT (1..2 3)",
            "POINT (1 2)\n",
            "\tPOINT (1 2)",
            "POINT\n(1 2)",
            "PoInT (1 2)",
            "POINTZ (1 2 3)",
            "POINT Z(1 2 3)",
            "POINT  ZM  (1 2 3 4)",
            "POINT (1 2 nan)",
            "POINT (inf 2)",
            "MULTIPOINT (EMPTY,EMPTY)",
            "GEOMETRYCOLLECTION (EMPTY)",
        ] {
            corpus.push(storpiatura.to_owned());
        }

        // I semi versionati del target: sono il corpus che il fuzzer ha gia'
        // trovato interessante, ed e' il posto dove le divergenze si nascondono.
        for seme in [
            include_str!("../../../fuzz/seeds/wkt_parse/polygon-con-anello-vuoto.wkt"),
            include_str!("../../../fuzz/seeds/wkt_parse/multipolygon-con-membro-vuoto.wkt"),
        ] {
            corpus.push(seme.to_owned());
        }
        corpus
    }

    /// L'unico irrigidimento del lotto, provato nei due versi.
    ///
    /// Lo spazio finale non e' testo e resta accettato; tutto il resto no. E'
    /// un errore di **sintassi**, non di budget: dire «limite superato» a chi
    /// ha una parentesi di troppo lo manderebbe ad allargare una quota che non
    /// c'entra.
    #[test]
    fn la_coda_non_vuota_e_rifiutata_come_sintassi() {
        let limiti = WkbLimits::default();

        // Lo spazio finale, in tutte le sue forme, e' ammesso.
        for coda in ["", " ", "   ", "\t", "\n", "\r\n", " \t\r\n "] {
            let testo = format!("POINT (1 2){coda}");
            assert!(
                analizza(&testo, &limiti).is_ok(),
                "lo spazio finale non e' testo residuo: {testo:?}"
            );
        }

        for testo in [
            "POINT (1 2))",
            "POINT (1 2) POINT (3 4)",
            "POINT (1 2),",
            "POINT (1 2) 3",
            "LINESTRING (0 0,1 1) )",
            "GEOMETRYCOLLECTION (POINT (1 2)) EMPTY",
        ] {
            let errore =
                analizza(testo, &limiti).expect_err(&format!("{testo} deve essere rifiutato"));
            assert_eq!(
                errore.code,
                plenora_io_model::IoErrorCode::Wkb,
                "{testo}: il testo residuo e' un errore di sintassi, non di budget"
            );
        }
    }

    /// Cio' che i writer producono resta leggibile.
    ///
    /// E' la meta' che l'irrigidimento potrebbe rompere senza farsi vedere: se
    /// `format_wkt` emettesse uno spazio, una parentesi o un a capo di troppo,
    /// il round-trip fallirebbe -- e fallirebbe in produzione, non qui.
    #[test]
    fn cio_che_scriviamo_resta_rileggibile() {
        let limiti = WkbLimits::default();
        for testo in [
            "POINT (1 2)",
            "POINT Z (1 2 3)",
            "POINT ZM (1 2 3 4)",
            "LINESTRING (0 0,1 1,2 2)",
            "LINESTRING M (0 0 5,1 1 6)",
            "POLYGON ((0 0,1 0,1 1,0 0))",
            "POLYGON ((0 0,1 0,1 1,0 0),(0 0,1 0,1 1,0 0))",
            "MULTIPOINT (1 2,3 4)",
            "MULTILINESTRING ((0 0,1 1),(2 2,3 3))",
            "MULTIPOLYGON (((0 0,1 0,1 1,0 0)))",
            "GEOMETRYCOLLECTION (POINT (1 2),LINESTRING (0 0,1 1))",
            "MULTIPOINT EMPTY",
            "GEOMETRYCOLLECTION EMPTY",
        ] {
            let geometria =
                analizza(testo, &limiti).unwrap_or_else(|errore| panic!("{testo}: {errore}"));
            let scritto = crate::wkt_lossless::format_wkt(&geometria)
                .unwrap_or_else(|errore| panic!("{testo}: {errore}"));
            let riletto = analizza(&scritto, &limiti).unwrap_or_else(|errore| {
                panic!("{testo} scritto come {scritto} non e' rileggibile: {errore}")
            });
            assert_eq!(geometria, riletto, "round-trip di {testo}");
        }
    }

    /// Una parola piu' lunga del vocabolario non alloca.
    ///
    /// Il buffer delle parole e' fisso: un input che ne dichiarasse una da
    /// megabyte otterrebbe un rifiuto, non la memoria che chiede.
    #[test]
    fn una_parola_smisurata_non_diventa_memoria() {
        let testo = "A".repeat(4_000_000);
        assert!(analizza(&testo, &WkbLimits::default()).is_err());
    }
}
