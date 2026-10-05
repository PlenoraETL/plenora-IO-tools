use crate::code_pair_iter::CodePairIter;
use crate::dxf_error::DxfError;
use crate::dxf_result::DxfResult;
use crate::CodePair;

/// Quante letture di fila possono essere servite dalla pila delle coppie
/// rimesse indietro prima che l'iteratore si fermi.
///
/// Un lettore annidato guarda la coppia successiva, la rimette indietro e
/// lascia decidere al chiamante: la stessa coppia passa cosi' per tutti i
/// livelli che la guardano, e ciascuno la rilegge una volta o due. Nel fork la
/// catena piu' profonda arriva a una manciata di riletture; 1024 sta due ordini
/// di grandezza sopra, e molto sotto cio' che serve a un ciclo senza progresso
/// per diventare un problema di memoria.
///
/// E' il **fondo** della classe dei lettori che restituiscono qualcosa senza
/// consumare: le guardie esplicite stanno nei cicli che il censimento ha
/// trovato, questa ferma quelli che nessuno ha ancora trovato. Un ciclo che
/// gira senza consumare e continua a leggere rilegge per forza dalla pila --
/// non c'e' altro modo di non avanzare -- ed e' li' che viene contato.
const MASSIME_RILETTURE_SENZA_PROGRESSO: u32 = 1024;

pub(crate) struct CodePairPutBack {
    top: Vec<DxfResult<CodePair>>,
    iter: Box<dyn CodePairIter>,
    /// Coppie estratte dalla sorgente, commenti `999` compresi.
    estratte: u64,
    /// Letture servite dalla pila dall'ultima coppia nuova estratta.
    riletture: u32,
    /// Una volta fermo, l'iteratore resta fermo: ogni lettura successiva e' un
    /// errore. Serve contro chi inghiotte gli errori -- `EntityIter` e
    /// `ObjectIter` li trasformano in fine sequenza -- e potrebbe altrimenti
    /// proseguire come se la sezione fosse finita li'. Porta l'offset della
    /// coppia su cui la lettura si e' fermata.
    fermo: Option<usize>,
    /// Entita' scartate da `Entity::read` perche' di un tipo che il fork non
    /// conosce, o DIMENSION senza un sottotipo riconosciuto. Upstream le
    /// scartava in silenzio; qui si contano, e chi legge decide.
    entita_ignorate: u64,
}

impl CodePairPutBack {
    pub fn from_code_pair_iter(iter: Box<dyn CodePairIter>) -> Self {
        CodePairPutBack {
            top: vec![],
            iter,
            estratte: 0,
            riletture: 0,
            fermo: None,
            entita_ignorate: 0,
        }
    }
    pub fn put_back(&mut self, item: DxfResult<CodePair>) {
        self.top.push(item);
    }
    pub fn read_as_utf8(&mut self) {
        self.iter.read_as_utf8()
    }

    /// Le coppie consumate finora: estratte dalla sorgente, meno quelle
    /// rimesse indietro e non ancora rilette.
    ///
    /// Serve a una sola domanda -- «questo giro ha consumato qualcosa?» -- e
    /// per quella basta confrontare due valori presi dallo stesso iteratore.
    /// La sottrazione non scende sotto zero finche' si rimette indietro solo
    /// cio' che si e' letto; se accadesse satura a zero, e una guardia che
    /// confronta due zeri rifiuta invece di accettare.
    pub fn posizione(&self) -> u64 {
        let in_attesa = u64::try_from(self.top.len()).unwrap_or(u64::MAX);
        self.estratte.saturating_sub(in_attesa)
    }

    /// Conta un'entita' che `Entity::read` scarta.
    pub fn segnala_entita_ignorata(&mut self) {
        self.entita_ignorate = self.entita_ignorate.saturating_add(1);
    }

    /// Le entita' scartate finora.
    pub fn entita_ignorate(&self) -> u64 {
        self.entita_ignorate
    }

    /// Rifiuta un giro che non ha consumato niente.
    ///
    /// `prima` e' la `posizione()` presa all'inizio del giro. Se non e'
    /// cresciuta, chi ha letto ha restituito un valore senza consumare input,
    /// e il ciclo che lo richiama ricomincerebbe identico: l'iteratore si
    /// ferma, e l'errore porta l'offset della coppia su cui la lettura e'
    /// rimasta -- nessun valore del documento.
    pub fn esigi_progresso(&mut self, prima: u64) -> DxfResult<()> {
        if self.posizione() > prima {
            return Ok(());
        }
        let offset = match self.top.last() {
            Some(Ok(pair)) => pair.offset,
            _ => 0,
        };
        Err(self.ferma(offset))
    }

    /// Ferma l'iteratore per sempre e restituisce l'errore da propagare.
    ///
    /// Da quando e' fermo, ogni lettura e' `ParseError(offset)`. Serve a ogni
    /// errore che rischia di essere inghiottito: `EntityIter` e `ObjectIter`
    /// trasformano un errore in fine sequenza, e dentro un BLOCK o nella
    /// sezione OBJECTS il chiamante proseguiva come se la sequenza fosse
    /// finita li', accettando il documento senza cio' che non aveva saputo
    /// leggere. Con l'iteratore fermo, la lettura successiva -- qualunque sia
    /// -- porta l'errore fino al lettore esterno.
    pub fn ferma(&mut self, offset: usize) -> DxfError {
        if self.fermo.is_none() {
            self.fermo = Some(offset);
        }
        DxfError::ParseError(offset)
    }
}

/// L'offset che un errore porta con se', o zero. Solo l'offset: mai il valore.
pub(crate) fn offset_dell_errore(errore: &DxfError) -> usize {
    match errore {
        DxfError::ParseFloatError(_, o)
        | DxfError::ParseIntError(_, o)
        | DxfError::ParseError(o)
        | DxfError::UnexpectedCode(_, o)
        | DxfError::UnexpectedByte(_, o)
        | DxfError::UnexpectedEnumValue(o)
        | DxfError::ExpectedTableType(o)
        | DxfError::WrongValueType(o) => *o,
        DxfError::UnexpectedCodePair(pair, _) => pair.offset,
        _ => 0,
    }
}

impl Iterator for CodePairPutBack {
    type Item = DxfResult<CodePair>;

    fn next(&mut self) -> Option<DxfResult<CodePair>> {
        if let Some(offset) = self.fermo {
            return Some(Err(DxfError::ParseError(offset)));
        }
        match self.top.pop() {
            Some(item) => {
                self.riletture = self.riletture.saturating_add(1);
                if self.riletture > MASSIME_RILETTURE_SENZA_PROGRESSO {
                    let offset = match item {
                        Ok(ref pair) => pair.offset,
                        Err(_) => 0,
                    };
                    self.fermo = Some(offset);
                    return Some(Err(DxfError::ParseError(offset)));
                }
                Some(item)
            }
            None => loop {
                let pair = self.iter.next();
                if pair.is_some() {
                    self.estratte = self.estratte.saturating_add(1);
                    self.riletture = 0;
                }
                match pair {
                    Some(Ok(CodePair { code: 999, .. })) => (), // a 999 comment code, try again
                    _ => return pair,
                }
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::code_pair_iter::DirectCodePairIter;

    fn iteratore(pairs: Vec<CodePair>) -> CodePairPutBack {
        CodePairPutBack::from_code_pair_iter(Box::new(DirectCodePairIter::new(pairs)))
    }

    #[test]
    fn la_posizione_conta_le_coppie_consumate() {
        let mut iter = iteratore(vec![
            CodePair::new_str(0, "SECTION"),
            CodePair::new_str(999, "commento"),
            CodePair::new_str(2, "OBJECTS"),
        ]);
        assert_eq!(iter.posizione(), 0);
        let prima = iter.next().unwrap().unwrap();
        assert_eq!(iter.posizione(), 1);
        iter.put_back(Ok(prima));
        assert_eq!(iter.posizione(), 0);
        let _ = iter.next();
        let _ = iter.next(); // salta il commento: due coppie estratte
        assert_eq!(iter.posizione(), 3);
    }

    #[test]
    fn un_giro_senza_progresso_ferma_l_iteratore() {
        let mut iter = iteratore(vec![CodePair::new_str(72, "x")]);
        let pair = iter.next().unwrap().unwrap();
        iter.put_back(Ok(pair));
        let prima = iter.posizione();
        // chi legge guarda la coppia e la rimette: nessun consumo
        let pair = iter.next().unwrap().unwrap();
        iter.put_back(Ok(pair));
        assert!(iter.esigi_progresso(prima).is_err());
        // fermo resta fermo, anche per chi inghiotte il primo errore
        assert!(matches!(iter.next(), Some(Err(DxfError::ParseError(_)))));
        assert!(matches!(iter.next(), Some(Err(DxfError::ParseError(_)))));
    }

    #[test]
    fn un_giro_che_consuma_passa() {
        let mut iter = iteratore(vec![CodePair::new_str(72, "x"), CodePair::new_str(0, "EOF")]);
        let prima = iter.posizione();
        let _ = iter.next();
        assert!(iter.esigi_progresso(prima).is_ok());
        assert!(matches!(iter.next(), Some(Ok(_))));
    }

    /// Il fondo: un ciclo che rilegge la stessa coppia senza mai consumarla
    /// si ferma con un errore, senza che il ciclo abbia una guardia propria.
    #[test]
    fn il_fondo_ferma_un_ciclo_che_rilegge_senza_fine() {
        let mut iter = iteratore(vec![CodePair::new_str(72, "x")]);
        let mut giri = 0_u32;
        let esito = loop {
            match iter.next() {
                Some(Ok(pair)) => iter.put_back(Ok(pair)),
                altro => break altro,
            }
            giri += 1;
            assert!(giri < 10_000, "il fondo non e' intervenuto");
        };
        assert!(matches!(esito, Some(Err(DxfError::ParseError(_)))));
        // la prima lettura estrae dalla sorgente, le successive rileggono
        assert_eq!(giri, MASSIME_RILETTURE_SENZA_PROGRESSO + 1);
    }

    /// Le riletture legittime -- una coppia guardata da piu' livelli e poi
    /// consumata -- non si accumulano: ogni coppia nuova azzera il conto.
    #[test]
    fn le_riletture_legittime_non_si_accumulano() {
        let coppie: Vec<CodePair> = (0..5000).map(|i| CodePair::new_i32(90, i)).collect();
        let mut iter = iteratore(coppie);
        let mut lette = 0;
        while let Some(esito) = iter.next() {
            let pair = esito.unwrap();
            for _ in 0..8 {
                iter.put_back(Ok(pair.clone()));
                let _ = iter.next();
            }
            lette += 1;
        }
        assert_eq!(lette, 5000);
    }
}
