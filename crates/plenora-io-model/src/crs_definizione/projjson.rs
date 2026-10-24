//! Gli identificatori di primo livello di un PROJJSON, letti come li legge
//! l'oracolo dei contratti (`arrow_data.py`, `load_projjson` e
//! `projjson_identifiers`).
//!
//! # Perche' non `serde_json`
//!
//! L'oracolo legge il documento con `json.loads` di Python, interi a precisione
//! arbitraria e decimali esatti (`parse_float=Decimal`). `serde_json` senza le
//! feature `arbitrary_precision` e `unbounded_depth` -- che il workspace non
//! abilita, e una dipendenza non cambia feature per questo -- rifiuta come
//! malformati documenti che per l'oracolo sono ben formati: un numero oltre
//! `f64` (`1e400`), una stringa con un surrogato isolato (`"\ud800"`), piu' di
//! 128 livelli di annidamento; e legge `-0` o un intero oltre `u64` come
//! decimali, che come codice di un identificatore sono malformati. Il bordo non
//! deve rifiutare cio' che il contratto accetta (GEO-005, REJ-001), quindi qui
//! c'e' un lettore piccolo e dedicato: verifica la sintassi del documento intero
//! e materializza soltanto `id` e `ids` della radice.
//!
//! # Che cosa replica
//!
//! La grammatica di `json.loads` con i ganci dell'oracolo: spazi solo
//! `[ \t\n\r]`; nessun carattere di controllo crudo nelle stringhe; gli escape
//! di JSON, con le coppie di surrogati unite e i surrogati isolati conservati;
//! numeri `-?(0|[1-9][0-9]*)(\.[0-9]+)?([eE][+-]?[0-9]+)?` di qualunque
//! grandezza; `NaN` e `Infinity` rifiutati (`parse_constant`); una chiave
//! ripetuta in qualunque oggetto rifiutata (`object_pairs_hook`); la radice
//! deve essere un oggetto.
//!
//! # Dove resta una differenza
//!
//! L'oracolo, in `CPython`, non da' un verdetto oltre circa mille livelli
//! (`RecursionError`) ne' su un intero di piu' di 4300 cifre (`ValueError`):
//! sono limiti dell'interprete, non della grammatica. Questo lettore e'
//! iterativo, non ha un tetto di annidamento proprio e legge gli interi come
//! testo: li' accetta. Il documento e' gia' limitato dai byte del metadato.

use std::collections::HashSet;

use super::{Malformata, TestoDefinizione};

/// Un valore di `id` o `ids`, materializzato fino a tre livelli: oltre, a un
/// identificatore non serve.
enum Valore {
    Oggetto(Vec<(Vec<u32>, Self)>),
    Lista(Vec<Self>),
    Testo(Vec<u32>),
    Intero(String),
    /// Un decimale, un booleano, `null`, o un contenitore oltre il terzo
    /// livello: nessuno dei due membri di un identificatore lo ammette.
    Altro,
}

/// Le coppie (autorita', codice) di `id` o `ids` della radice.
pub(super) fn identificatori(
    testo: &str,
) -> Result<Vec<(TestoDefinizione, TestoDefinizione)>, Malformata> {
    valida(testo.as_bytes())?;
    let mut cursore = Cursore {
        byte: testo.as_bytes(),
        pos: 0,
    };
    let (id, ids) = radice(&mut cursore)?;
    // `id` e `ids` insieme, o `ids` che non e' una lista, sono malformati.
    let trovati = match (id, ids) {
        (Some(id), None) => vec![id],
        (None, Some(Valore::Lista(ids))) => ids,
        (None, None) => Vec::new(),
        (_, Some(_)) => return Err(Malformata),
    };
    trovati.into_iter().map(identificatore).collect()
}

fn identificatore(voce: Valore) -> Result<(TestoDefinizione, TestoDefinizione), Malformata> {
    let Valore::Oggetto(membri) = voce else {
        return Err(Malformata);
    };
    let mut autorita = None;
    let mut codice = None;
    for (chiave, valore) in membri {
        if chiave == unita("authority") {
            autorita = Some(valore);
        } else if chiave == unita("code") {
            codice = Some(valore);
        }
    }
    let Some(Valore::Testo(autorita)) = autorita else {
        return Err(Malformata);
    };
    let codice = match codice {
        Some(Valore::Testo(codice)) => testo_definizione(&codice),
        Some(Valore::Intero(codice)) => TestoDefinizione::Unicode(codice),
        _ => return Err(Malformata),
    };
    Ok((testo_definizione(&autorita), codice))
}

fn unita(testo: &str) -> Vec<u32> {
    testo.chars().map(u32::from).collect()
}

/// Un testo JSON decodificato in punti di codice; un surrogato isolato non ha
/// una stringa Rust che lo rappresenti.
fn testo_definizione(punti: &[u32]) -> TestoDefinizione {
    punti
        .iter()
        .map(|p| char::from_u32(*p))
        .collect::<Option<String>>()
        .map_or(
            TestoDefinizione::SurrogatiIsolati,
            TestoDefinizione::Unicode,
        )
}

struct Cursore<'a> {
    byte: &'a [u8],
    pos: usize,
}

impl Cursore<'_> {
    fn spazi(&mut self) {
        while matches!(self.byte.get(self.pos), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.pos += 1;
        }
    }

    fn guarda(&self) -> Option<u8> {
        self.byte.get(self.pos).copied()
    }

    fn atteso(&mut self, byte: u8) -> Result<(), Malformata> {
        if self.guarda() == Some(byte) {
            self.pos += 1;
            Ok(())
        } else {
            Err(Malformata)
        }
    }

    /// Una stringa JSON, dalle virgolette d'apertura, in punti di codice.
    fn stringa(&mut self) -> Result<Vec<u32>, Malformata> {
        self.atteso(b'"')?;
        let mut punti = Vec::new();
        loop {
            let byte = self.guarda().ok_or(Malformata)?;
            match byte {
                b'"' => {
                    self.pos += 1;
                    return Ok(punti);
                }
                b'\\' => {
                    self.pos += 1;
                    punti.push(self.escape()?);
                }
                // `strict=True`: nessun carattere di controllo crudo.
                0x00..=0x1f => return Err(Malformata),
                _ => {
                    // Il documento e' una `&str`: dal byte iniziale si sa
                    // quanti byte ha il carattere, e si decodifica solo lui
                    // (decodificare tutto il resto a ogni carattere sarebbe
                    // quadratico).
                    let lunghezza = match byte {
                        0x00..=0x7f => 1,
                        0xc0..=0xdf => 2,
                        0xe0..=0xef => 3,
                        _ => 4,
                    };
                    let pezzo = self
                        .byte
                        .get(self.pos..self.pos + lunghezza)
                        .ok_or(Malformata)?;
                    let carattere = std::str::from_utf8(pezzo)
                        .map_err(|_| Malformata)?
                        .chars()
                        .next()
                        .ok_or(Malformata)?;
                    punti.push(u32::from(carattere));
                    self.pos += carattere.len_utf8();
                }
            }
        }
    }

    fn escape(&mut self) -> Result<u32, Malformata> {
        let byte = self.guarda().ok_or(Malformata)?;
        self.pos += 1;
        let semplice = match byte {
            b'"' => 0x22,
            b'\\' => 0x5c,
            b'/' => 0x2f,
            b'b' => 0x08,
            b'f' => 0x0c,
            b'n' => 0x0a,
            b'r' => 0x0d,
            b't' => 0x09,
            b'u' => {
                let alto = self.esadecimale()?;
                // Una coppia di surrogati si unisce; un alto non seguito da un
                // basso resta isolato, e cio' che segue si rilegge da capo.
                if (0xd800..=0xdbff).contains(&alto)
                    && self.byte.get(self.pos..self.pos + 2) == Some(b"\\u")
                {
                    let ritorno = self.pos;
                    self.pos += 2;
                    let basso = self.esadecimale()?;
                    if (0xdc00..=0xdfff).contains(&basso) {
                        return Ok(0x10000 + ((alto - 0xd800) << 10) + (basso - 0xdc00));
                    }
                    self.pos = ritorno;
                }
                return Ok(alto);
            }
            _ => return Err(Malformata),
        };
        Ok(semplice)
    }

    fn esadecimale(&mut self) -> Result<u32, Malformata> {
        let quattro = self.byte.get(self.pos..self.pos + 4).ok_or(Malformata)?;
        let mut valore = 0_u32;
        for byte in quattro {
            let cifra = char::from(*byte).to_digit(16).ok_or(Malformata)?;
            valore = valore * 16 + cifra;
        }
        self.pos += 4;
        Ok(valore)
    }

    /// Un numero JSON: `Some(testo)` se intero, nella forma di `str(int(...))`
    /// di Python (`-0` vale `0`), `None` se decimale. Come la regex di
    /// `json.scanner`, una frazione o un esponente incompleti non si consumano:
    /// il byte che resta lo rifiuta chi viene dopo.
    fn numero(&mut self) -> Result<Option<String>, Malformata> {
        let inizio = self.pos;
        if self.guarda() == Some(b'-') {
            self.pos += 1;
        }
        match self.guarda() {
            Some(b'0') => self.pos += 1,
            Some(b'1'..=b'9') => self.cifre(),
            _ => return Err(Malformata),
        }
        let fine_intero = self.pos;
        if self.guarda() == Some(b'.')
            && self.byte.get(self.pos + 1).is_some_and(u8::is_ascii_digit)
        {
            self.pos += 1;
            self.cifre();
        }
        if matches!(self.guarda(), Some(b'e' | b'E')) {
            let mut dopo = self.pos + 1;
            if matches!(self.byte.get(dopo), Some(b'+' | b'-')) {
                dopo += 1;
            }
            if self.byte.get(dopo).is_some_and(u8::is_ascii_digit) {
                self.pos = dopo;
                self.cifre();
            }
        }
        // Una frazione o un esponente consumati fanno un decimale.
        if self.pos != fine_intero {
            return Ok(None);
        }
        let testo = std::str::from_utf8(&self.byte[inizio..fine_intero]).map_err(|_| Malformata)?;
        Ok(Some(if testo == "-0" {
            "0".to_owned()
        } else {
            testo.to_owned()
        }))
    }

    fn cifre(&mut self) {
        while self.guarda().is_some_and(|b| b.is_ascii_digit()) {
            self.pos += 1;
        }
    }

    fn letterale(&mut self, parola: &[u8]) -> Result<(), Malformata> {
        if self.byte.get(self.pos..self.pos + parola.len()) == Some(parola) {
            self.pos += parola.len();
            Ok(())
        } else {
            Err(Malformata)
        }
    }

    /// Un valore scalare, o l'apertura di un contenitore (`{` o `[`, non
    /// consumata).
    fn scalare(&mut self) -> Result<Option<Valore>, Malformata> {
        Ok(Some(match self.guarda().ok_or(Malformata)? {
            b'{' | b'[' => return Ok(None),
            b'"' => Valore::Testo(self.stringa()?),
            b'-' | b'0'..=b'9' => self.numero()?.map_or(Valore::Altro, Valore::Intero),
            b't' => {
                self.letterale(b"true")?;
                Valore::Altro
            }
            b'f' => {
                self.letterale(b"false")?;
                Valore::Altro
            }
            b'n' => {
                self.letterale(b"null")?;
                Valore::Altro
            }
            // `NaN`, `Infinity`, `-Infinity` (gia' rifiutato da `numero`) e
            // ogni altro byte.
            _ => return Err(Malformata),
        }))
    }
}

enum Contenitore {
    Oggetto(HashSet<Vec<u32>>),
    Lista,
}

/// La sintassi del documento intero, senza ricorsione: una pila di contenitori,
/// e per ogni oggetto l'insieme delle sue chiavi.
fn valida(byte: &[u8]) -> Result<(), Malformata> {
    let mut cursore = Cursore { byte, pos: 0 };
    let mut pila: Vec<Contenitore> = Vec::new();
    'valore: loop {
        cursore.spazi();
        match cursore.scalare()? {
            Some(_) => {}
            None if cursore.guarda() == Some(b'{') => {
                cursore.pos += 1;
                cursore.spazi();
                if cursore.guarda() == Some(b'}') {
                    cursore.pos += 1;
                } else {
                    let mut chiavi = HashSet::new();
                    chiave(&mut cursore, &mut chiavi)?;
                    pila.push(Contenitore::Oggetto(chiavi));
                    continue 'valore;
                }
            }
            None => {
                cursore.pos += 1;
                cursore.spazi();
                if cursore.guarda() == Some(b']') {
                    cursore.pos += 1;
                } else {
                    pila.push(Contenitore::Lista);
                    continue 'valore;
                }
            }
        }
        // Un valore e' finito: si chiudono i contenitori che finiscono con lui.
        loop {
            cursore.spazi();
            match pila.last_mut() {
                None => break 'valore,
                Some(Contenitore::Oggetto(chiavi)) => match cursore.guarda() {
                    Some(b',') => {
                        cursore.pos += 1;
                        cursore.spazi();
                        chiave(&mut cursore, chiavi)?;
                        continue 'valore;
                    }
                    Some(b'}') => {
                        cursore.pos += 1;
                        pila.pop();
                    }
                    _ => return Err(Malformata),
                },
                Some(Contenitore::Lista) => match cursore.guarda() {
                    Some(b',') => {
                        cursore.pos += 1;
                        continue 'valore;
                    }
                    Some(b']') => {
                        cursore.pos += 1;
                        pila.pop();
                    }
                    _ => return Err(Malformata),
                },
            }
        }
    }
    cursore.spazi();
    if cursore.pos == byte.len() {
        Ok(())
    } else {
        Err(Malformata)
    }
}

/// Una chiave e i due punti; una chiave gia' vista nello stesso oggetto e'
/// malformata.
fn chiave(cursore: &mut Cursore<'_>, chiavi: &mut HashSet<Vec<u32>>) -> Result<(), Malformata> {
    let nome = cursore.stringa()?;
    if !chiavi.insert(nome) {
        return Err(Malformata);
    }
    cursore.spazi();
    cursore.atteso(b':')
}

/// `id` e `ids` della radice, che deve essere un oggetto. Il documento e' gia'
/// stato validato: qui si percorre soltanto.
fn radice(cursore: &mut Cursore<'_>) -> Result<(Option<Valore>, Option<Valore>), Malformata> {
    cursore.spazi();
    if cursore.guarda() != Some(b'{') {
        return Err(Malformata);
    }
    let (mut id, mut ids) = (None, None);
    for (nome, valore) in membri(cursore, 1)? {
        if nome == unita("id") {
            id = Some(valore);
        } else if nome == unita("ids") {
            ids = Some(valore);
        }
    }
    Ok((id, ids))
}

/// I membri dell'oggetto che comincia al cursore, con i valori materializzati
/// fino al livello `livello + 2`.
fn membri(
    cursore: &mut Cursore<'_>,
    livello: usize,
) -> Result<Vec<(Vec<u32>, Valore)>, Malformata> {
    cursore.atteso(b'{')?;
    let mut trovati = Vec::new();
    cursore.spazi();
    if cursore.guarda() == Some(b'}') {
        cursore.pos += 1;
        return Ok(trovati);
    }
    loop {
        cursore.spazi();
        let nome = cursore.stringa()?;
        cursore.spazi();
        cursore.atteso(b':')?;
        cursore.spazi();
        trovati.push((nome, valore(cursore, livello + 1)?));
        cursore.spazi();
        match cursore.guarda() {
            Some(b',') => cursore.pos += 1,
            Some(b'}') => {
                cursore.pos += 1;
                return Ok(trovati);
            }
            _ => return Err(Malformata),
        }
    }
}

/// Il livello oltre il quale un valore non si materializza: `ids` (1), un suo
/// identificatore (2), un membro dell'identificatore (3).
const LIVELLO_MASSIMO: usize = 3;

fn valore(cursore: &mut Cursore<'_>, livello: usize) -> Result<Valore, Malformata> {
    if let Some(scalare) = cursore.scalare()? {
        return Ok(scalare);
    }
    if livello > LIVELLO_MASSIMO {
        salta_contenitore(cursore)?;
        return Ok(Valore::Altro);
    }
    if cursore.guarda() == Some(b'{') {
        return Ok(Valore::Oggetto(membri(cursore, livello)?));
    }
    cursore.atteso(b'[')?;
    let mut elementi = Vec::new();
    cursore.spazi();
    if cursore.guarda() == Some(b']') {
        cursore.pos += 1;
        return Ok(Valore::Lista(elementi));
    }
    loop {
        cursore.spazi();
        elementi.push(valore(cursore, livello + 1)?);
        cursore.spazi();
        match cursore.guarda() {
            Some(b',') => cursore.pos += 1,
            Some(b']') => {
                cursore.pos += 1;
                return Ok(Valore::Lista(elementi));
            }
            _ => return Err(Malformata),
        }
    }
}

/// Salta un contenitore gia' validato contando le parentesi fuori dalle
/// stringhe, senza ricorsione.
fn salta_contenitore(cursore: &mut Cursore<'_>) -> Result<(), Malformata> {
    let mut aperti = 0_usize;
    loop {
        match cursore.guarda().ok_or(Malformata)? {
            b'"' => {
                cursore.stringa()?;
                continue;
            }
            b'{' | b'[' => aperti += 1,
            b'}' | b']' => {
                aperti = aperti.checked_sub(1).ok_or(Malformata)?;
                if aperti == 0 {
                    cursore.pos += 1;
                    return Ok(());
                }
            }
            _ => {}
        }
        cursore.pos += 1;
    }
}
