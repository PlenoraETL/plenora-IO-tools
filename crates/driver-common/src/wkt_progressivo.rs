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
//! # Che cosa **non** cambia
//!
//! L'insieme accettato. Restano i sette tipi classici con le loro regole di
//! coerenza dimensionale, `POINT EMPTY` resta non rappresentabile nel core
//! WKB, e la verifica di esprimibilita' resta a valle: questo modulo cambia
//! *quando* si rifiuta, non *che cosa* si accetta. Un lotto che allargasse o
//! stringesse l'insieme accettato mentre sposta il confine renderebbe
//! impossibile dire quale delle due cose ha rotto un file.

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
    let tag = analizzatore.parola();
    if tag.vuota() {
        return Err(errore("tipo di geometria atteso"));
    }
    let dimensioni = dimensioni_dichiarate(analizzatore, attese)?;

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
        Err(errore("tipo di geometria non riconosciuto"))
    }
}

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
    if analizzatore.se_prossimo(b')') {
        return Ok((Vec::new(), del_vuoto(dichiarate, attese)?));
    }
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
    if analizzatore.se_prossimo(b')') {
        return Ok((Vec::new(), del_vuoto(dichiarate, attese)?));
    }
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
    if analizzatore.se_prossimo(b')') {
        return Ok(costruita(
            WkbValue::MultiPoint(Vec::new()),
            del_vuoto(dichiarate, attese)?,
        ));
    }
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
    if analizzatore.se_prossimo(b')') {
        return Ok(costruita(
            WkbValue::MultiLineString(Vec::new()),
            del_vuoto(dichiarate, attese)?,
        ));
    }
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
    if analizzatore.se_prossimo(b')') {
        return Ok(costruita(
            WkbValue::MultiPolygon(Vec::new()),
            del_vuoto(dichiarate, attese)?,
        ));
    }
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
    if analizzatore.se_prossimo(b')') {
        return Ok(costruita(
            WkbValue::GeometryCollection(Vec::new()),
            del_vuoto(dichiarate, attese)?,
        ));
    }
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
