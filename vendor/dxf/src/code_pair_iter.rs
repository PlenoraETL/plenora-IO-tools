use crate::{CodePair, CodePairValue, DxfError, DxfResult, ExpectedType};

use crate::code_pair_value::decodifica_testo;
use crate::helper_functions::*;
use encoding_rs::Encoding;
use std::io::{BufRead, BufReader, Cursor, Read};

pub(crate) trait CodePairIter: Iterator<Item = DxfResult<CodePair>> {
    fn read_as_utf8(&mut self);
    /// Dichiara che il valore appena letto -- un gruppo `3` di MTEXT -- continua
    /// nel prossimo, e restituisce quanti caratteri della coda sospesa sono gia'
    /// nel valore consegnato (vedi `CodaSospesa`), che il chiamante deve
    /// togliere. Zero se non c'e' coda.
    fn continua_frammento(&mut self) -> usize {
        0
    }
}

/// La sequenza di escape rimasta aperta alla fine di un gruppo `3`.
///
/// MTEXT spezza il testo in frammenti da 250 caratteri, e il taglio puo'
/// cadere dentro una `\U+XXXX` o una `^x`. Decodificare i frammenti uno per
/// uno rifiutava la prima meta' e leggeva la seconda come testo. Qui la coda
/// grezza resta da parte: se il lettore di MTEXT la reclama, il valore
/// seguente si decodifica con la coda davanti; se nessuno la reclama, una coda
/// che non e' un carattere letterale (`\U+` con meno di quattro cifre) rende
/// il documento illeggibile.
#[derive(Default)]
pub(crate) struct CodaSospesa {
    /// La coda grezza, se c'e'.
    grezza: Option<String>,
    /// Se la coda e' anche, letteralmente, alla fine del valore consegnato:
    /// `^`, `\`, `\U` lo sono; una `\U+` incompleta no.
    nel_valore: bool,
    /// Il lettore di MTEXT l'ha reclamata: il prossimo valore la continua.
    reclamata: bool,
}

impl CodaSospesa {
    /// Da chiamare prima di leggere una coppia nuova. Errore se c'e' una coda
    /// non letterale che nessuno ha reclamato.
    fn verifica_prima_di_leggere(&mut self, offset: usize) -> DxfResult<()> {
        if self.grezza.is_some() && !self.reclamata {
            let letterale = self.nel_valore;
            *self = CodaSospesa::default();
            if !letterale {
                return Err(DxfError::ParseError(offset));
            }
        }
        Ok(())
    }
    fn reclama(&mut self) -> usize {
        match self.grezza {
            Some(ref coda) => {
                self.reclamata = true;
                if self.nel_valore {
                    coda.chars().count()
                } else {
                    0
                }
            }
            None => 0,
        }
    }
    /// Decodifica un valore di testo: prima la coda reclamata, se c'e'.
    fn decodifica(
        &mut self,
        codice: i32,
        valore: &str,
        unicode: bool,
        offset: usize,
    ) -> DxfResult<String> {
        let grezzo = if self.reclamata {
            if codice != 1 && codice != 3 {
                return Err(DxfError::ParseError(offset));
            }
            let mut unito = self.grezza.take().unwrap_or_default();
            unito.push_str(valore);
            unito
        } else {
            String::from(valore)
        };
        *self = CodaSospesa::default();
        let consenti_coda = codice == 3;
        let (mut testo, coda) = decodifica_testo(&grezzo, unicode, consenti_coda)
            .ok_or(DxfError::ParseError(offset))?;
        if !coda.is_empty() {
            // La coda e' letterale se, decodificata senza coda, resta se stessa.
            let letterale = decodifica_testo(&coda, unicode, false)
                .is_some_and(|(come_testo, _)| come_testo == coda);
            if letterale {
                testo.push_str(&coda);
            }
            self.grezza = Some(coda);
            self.nel_valore = letterale;
        }
        Ok(testo)
    }
    /// Alla fine dell'ingresso una coda reclamata -- letterale o no: il
    /// gruppo che doveva continuarla non c'e' -- e una coda non letterale sono
    /// errori. Va chiamata su **ogni** via verso la fine, spazio bianco finale
    /// compreso: altrimenti il lettore di MTEXT consegnava un testo mutilato.
    fn verifica_alla_fine(&mut self, offset: usize) -> Option<DxfResult<CodePair>> {
        if self.grezza.is_some() && (self.reclamata || !self.nel_valore) {
            *self = CodaSospesa::default();
            return Some(Err(DxfError::ParseError(offset)));
        }
        None
    }
    /// Una coda reclamata si continua solo con un gruppo di testo `1` o `3`:
    /// qualunque altro gruppo, di qualunque tipo, e' un errore. Va chiamata
    /// appena letto il codice, in entrambi i formati.
    fn verifica_codice(&self, codice: i32, offset: usize) -> DxfResult<()> {
        if self.reclamata && codice != 1 && codice != 3 {
            return Err(DxfError::ParseError(offset));
        }
        Ok(())
    }
}

/// Directly returns code pairs; primarily used in tests.
#[cfg(test)]
pub(crate) struct DirectCodePairIter {
    pairs: Vec<CodePair>,
    offset: usize,
}

#[cfg(test)]
impl CodePairIter for DirectCodePairIter {
    fn read_as_utf8(&mut self) {
        // noop
    }
}

#[cfg(test)]
impl Iterator for DirectCodePairIter {
    type Item = DxfResult<CodePair>;
    fn next(&mut self) -> Option<DxfResult<CodePair>> {
        if self.offset < self.pairs.len() {
            let pair = self.pairs[self.offset].clone();
            self.offset += 1;
            return Some(Ok(pair));
        }

        None
    }
}

#[cfg(test)]
impl DirectCodePairIter {
    pub(crate) fn new(pairs: Vec<CodePair>) -> Self {
        DirectCodePairIter { pairs, offset: 0 }
    }
}

/// Returns code pairs as read from text.  Handles the most common DXF files and when parsed from strings.
pub(crate) struct TextCodePairIter<T: Read> {
    reader: BufReader<T>,
    string_encoding: &'static Encoding,
    first_line: String,
    read_first_line: bool,
    offset: usize,
    coda: CodaSospesa,
}

impl<T: Read> CodePairIter for TextCodePairIter<T> {
    fn read_as_utf8(&mut self) {
        self.string_encoding = encoding_rs::UTF_8;
    }
    fn continua_frammento(&mut self) -> usize {
        self.coda.reclama()
    }
}

impl<T: Read> Iterator for TextCodePairIter<T> {
    type Item = DxfResult<CodePair>;
    fn next(&mut self) -> Option<DxfResult<CodePair>> {
        self.read_code_pair()
    }
}

impl<T: Read> TextCodePairIter<T> {
    pub fn new(
        reader: T,
        string_encoding: &'static Encoding,
        first_line: String,
        offset: usize,
    ) -> Self {
        TextCodePairIter {
            reader: BufReader::with_capacity(1024, reader),
            string_encoding,
            first_line,
            read_first_line: false,
            offset,
            coda: CodaSospesa::default(),
        }
    }
    fn read_code_pair(&mut self) -> Option<DxfResult<CodePair>> {
        if let Err(e) = self.coda.verifica_prima_di_leggere(self.offset) {
            return Some(Err(e));
        }
        // Read code.  If no line is available, fail gracefully.
        let code_line = if self.read_first_line {
            self.offset += 1;
            match read_buffered_line(&mut self.reader, true, encoding_rs::WINDOWS_1252) {
                Ok(Some(v)) => v,
                // la fine vera dell'ingresso
                Ok(None) => return self.coda.verifica_alla_fine(self.offset),
                Err(e) => return Some(Err(e)),
            }
        } else {
            self.read_first_line = true;

            // .clone() is fine because it'll only ever be called once and the only valid
            // values that might be cloned are: "0" and "999"; all others are errors.
            self.first_line.clone()
        };
        let code_line = code_line.trim();
        if code_line.is_empty() {
            // Upstream trattava una riga di codice vuota come la fine
            // dell'ingresso, e tutto cio' che seguiva spariva: un DXF con una
            // riga vuota in testa si leggeva come un documento vuoto. Ora e' la
            // fine solo se dopo non resta altro che spazio bianco -- un file
            // fatto di soli a capo resta un file vuoto --, altrimenti e' un
            // errore.
            return match resto_solo_spazio(&mut self.reader) {
                Ok(true) => self.coda.verifica_alla_fine(self.offset),
                Ok(false) => Some(Err(DxfError::ParseError(self.offset))),
                Err(e) => Some(Err(e)),
            };
        }

        let code_offset = self.offset;
        let code = try_into_option!(parse_i32(String::from(code_line), code_offset));
        if let Err(e) = self.coda.verifica_codice(code, code_offset) {
            return Some(Err(e));
        }

        // Read value.  If no line is available die horribly.
        self.offset += 1;
        let value_line = match read_buffered_line(&mut self.reader, false, self.string_encoding) {
            Ok(Some(v)) => v,
            // Un codice senza la riga del valore: upstream lo leggeva come
            // valore vuoto.
            Ok(None) => return Some(Err(DxfError::UnexpectedEndOfInput)),
            Err(e) => return Some(Err(e)),
        };

        // construct the value pair
        let expected_type = match ExpectedType::new(code) {
            Some(t) => t,
            None => return Some(Err(DxfError::UnexpectedEnumValue(self.offset))),
        };
        let value = match expected_type {
            ExpectedType::Boolean => {
                CodePairValue::Boolean(try_into_option!(parse_i16(value_line, self.offset)))
            }
            ExpectedType::Integer => {
                CodePairValue::Integer(try_into_option!(parse_i32(value_line, self.offset)))
            }
            ExpectedType::Long => {
                CodePairValue::Long(try_into_option!(parse_i64(value_line, self.offset)))
            }
            ExpectedType::Short => {
                CodePairValue::Short(try_into_option!(parse_i16(value_line, self.offset)))
            }
            ExpectedType::Double => {
                CodePairValue::Double(try_into_option!(parse_f64(value_line, self.offset)))
            }
            ExpectedType::Str => {
                // `\U+` solo nel DXF ASCII codificato Windows-1252, come upstream.
                let unicode = self.string_encoding == encoding_rs::WINDOWS_1252;
                CodePairValue::Str(try_into_option!(self.coda.decodifica(
                    code,
                    &value_line,
                    unicode,
                    self.offset
                )))
            }
            ExpectedType::Binary => {
                let mut data = vec![];
                match parse_hex_string(&value_line, &mut data, self.offset) {
                    Ok(()) => CodePairValue::Binary(data),
                    Err(e) => return Some(Err(e)),
                }
            }
        };

        Some(Ok(CodePair::new(code, value, code_offset)))
    }
}

/// Una riga, oppure `None` alla fine dell'ingresso: le due cose upstream si
/// confondevano, perche' entrambe arrivavano come stringa vuota.
fn read_buffered_line<T: BufRead + ?Sized>(
    reader: &mut T,
    allow_bom: bool,
    encoding: &'static Encoding,
) -> DxfResult<Option<String>> {
    let mut bytes = Vec::new();
    if reader.read_until(b'\n', &mut bytes)? == 0 {
        return Ok(None);
    }
    if bytes.last() == Some(&b'\n') {
        bytes.pop();
    }
    if bytes.last() == Some(&b'\r') {
        bytes.pop();
    }
    if allow_bom && bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        bytes.drain(..3);
    }
    match encoding.decode(&bytes) {
        (value, _, false) => Ok(Some(value.into_owned())),
        (_, _, true) => Err(DxfError::MalformedString),
    }
}

/// `true` se da qui alla fine dell'ingresso c'e' solo spazio bianco ASCII.
fn resto_solo_spazio<T: BufRead + ?Sized>(reader: &mut T) -> DxfResult<bool> {
    loop {
        let blocco = reader.fill_buf()?;
        if blocco.is_empty() {
            return Ok(true);
        }
        if !blocco.iter().all(u8::is_ascii_whitespace) {
            return Ok(false);
        }
        let quanti = blocco.len();
        reader.consume(quanti);
    }
}

/// Returns code pairs as read from a binary file.  Usually created _after_ the first line of a file has been read.
pub(crate) struct BinaryCodePairIter<T: Read> {
    reader: T,
    code_size_detection_complete: bool,
    codes_are_two_bytes: bool,
    offset: usize,
    coda: CodaSospesa,
}

impl<T: Read> CodePairIter for BinaryCodePairIter<T> {
    fn read_as_utf8(&mut self) {
        // noop
    }
    fn continua_frammento(&mut self) -> usize {
        self.coda.reclama()
    }
}

impl<T: Read> Iterator for BinaryCodePairIter<T> {
    type Item = DxfResult<CodePair>;
    fn next(&mut self) -> Option<DxfResult<CodePair>> {
        self.read_code_pair()
    }
}

impl<T: Read> BinaryCodePairIter<T> {
    pub fn new(reader: T, offset: usize) -> Self {
        BinaryCodePairIter {
            reader,
            code_size_detection_complete: false,
            codes_are_two_bytes: false,
            offset,
            coda: CodaSospesa::default(),
        }
    }
    fn read_code_pair(&mut self) -> Option<DxfResult<CodePair>> {
        if let Err(e) = self.coda.verifica_prima_di_leggere(self.offset) {
            return Some(Err(e));
        }
        // Read code.  If no data is available, fail gracefully.
        let mut code = match read_u8(&mut self.reader) {
            Some(Ok(c)) => i32::from(c),
            Some(Err(e)) => return Some(Err(DxfError::IoError(e))),
            None => return self.coda.verifica_alla_fine(self.offset),
        };
        self.offset += 1;

        // If reading a larger code and no data is available, die horribly.
        if self.codes_are_two_bytes {
            // post R13 codes are 2 bytes, read the second byte of the code
            let high_byte = i32::from(try_from_dxf_result!(read_u8_strict(&mut self.reader)));
            code += high_byte << 8;
            self.offset += 1;
        } else if code == 255 {
            // pre R13 codes are either 1 or 3 bytes
            code = i32::from(try_from_dxf_result!(read_i16(&mut self.reader)));
            self.offset += 2;
        }
        // Come nel DXF ASCII: una coda reclamata si continua solo con un
        // gruppo `1` o `3`. Prima il controllo stava nel solo ramo del testo, e
        // un gruppo numerico in mezzo passava.
        if let Err(e) = self.coda.verifica_codice(code, self.offset) {
            return Some(Err(e));
        }

        // Read value.  If no data is available die horribly.
        let expected_type = match ExpectedType::new(code) {
            Some(t) => t,
            None => return Some(Err(DxfError::UnexpectedEnumValue(self.offset))),
        };
        let (value, read_bytes) = match expected_type {
            ExpectedType::Boolean => {
                // after R13 bools are encoded as a single byte
                let (b_value, read_bytes) = if self.codes_are_two_bytes {
                    (
                        i16::from(try_from_dxf_result!(read_u8_strict(&mut self.reader))),
                        1,
                    )
                } else {
                    (try_from_dxf_result!(read_i16(&mut self.reader)), 2)
                };
                (CodePairValue::Boolean(b_value), read_bytes)
            }
            ExpectedType::Integer => (
                CodePairValue::Integer(try_from_dxf_result!(read_i32(&mut self.reader))),
                4,
            ),
            ExpectedType::Long => (
                CodePairValue::Long(try_from_dxf_result!(read_i64(&mut self.reader))),
                8,
            ),
            ExpectedType::Short => (
                CodePairValue::Short(try_from_dxf_result!(read_i16(&mut self.reader))),
                2,
            ),
            ExpectedType::Double => (
                CodePairValue::Double(try_from_dxf_result!(read_f64(&mut self.reader))),
                8,
            ),
            ExpectedType::Str => {
                let mut value = try_from_dxf_result!(self.read_string_binary());
                if !self.code_size_detection_complete && code == 0 && value.is_empty() {
                    // If this is the first pair being read and the code is 0, the only valid string value is "SECTION".
                    // If the read value is instead empty, that means the string reader found a single 0x00 byte which
                    // indicates that this is a post R13 binary file where codes are always read as 2 bytes.  The 0x00
                    // byte was really the second byte of {0x00, 0x00}, so we need to do one more string read to catch
                    // the reader up.
                    self.codes_are_two_bytes = true;
                    self.offset += 1; // account for the NULL byte that was interpreted as an empty string
                    value = try_from_dxf_result!(self.read_string_binary()); // now read the actual value
                }
                let lunghezza = value.len() + 1; // +1 to account for the NULL terminator
                (
                    CodePairValue::Str(try_from_dxf_result!(self.coda.decodifica(
                        code,
                        &value,
                        false,
                        self.offset
                    ))),
                    lunghezza,
                )
            }
            ExpectedType::Binary => {
                let length = try_from_dxf_result!(read_u8_strict(&mut self.reader)) as usize;
                let mut data = vec![];
                for _ in 0..length {
                    data.push(try_from_dxf_result!(read_u8_strict(&mut self.reader)));
                }

                (CodePairValue::Binary(data), length + 1) // +1 to account for initial length byte
            }
        };
        self.offset += read_bytes;
        self.code_size_detection_complete = true;

        Some(Ok(CodePair::new(code, value, self.offset)))
    }
    fn read_string_binary(&mut self) -> DxfResult<String> {
        let mut s = String::new();
        loop {
            match read_u8(&mut self.reader) {
                Some(Ok(0)) => break,
                Some(Ok(c)) => s.push(c as char),
                Some(Err(e)) => return Err(DxfError::IoError(e)),
                None => return Err(DxfError::UnexpectedEndOfInput),
            }
        }

        Ok(s)
    }
}

//---------------------------

pub(crate) fn new_code_pair_iter_from_reader<T>(
    mut reader: T,
    string_encoding: &'static Encoding,
    first_line: String,
) -> DxfResult<Box<dyn CodePairIter>>
where
    T: Read,
{
    let mut bytes = vec![];
    reader.read_to_end(&mut bytes)?;
    let mut cursor = Cursor::new(bytes);
    let iter: Box<dyn CodePairIter> = match &*first_line {
        "AutoCAD Binary DXF" => {
            // swallow 0x1A,0x00
            assert_or_err!(
                try_option_io_result_into_err!(read_u8(&mut cursor)),
                0x1A,
                18
            );
            assert_or_err!(
                try_option_io_result_into_err!(read_u8(&mut cursor)),
                0x00,
                19
            );
            Box::new(BinaryCodePairIter::new(cursor, 20))
        }
        _ => Box::new(TextCodePairIter::new(
            cursor,
            string_encoding,
            first_line,
            1,
        )),
    };
    Ok(iter)
}

/// Builds a code-pair iterator without first copying the complete input into
/// memory. This is used by the progressive drawing reader, which owns its
/// input and can therefore keep it behind the type-erased iterator safely.
pub(crate) fn new_code_pair_iter_from_owned_reader<T>(
    mut reader: T,
    string_encoding: &'static Encoding,
) -> DxfResult<Box<dyn CodePairIter>>
where
    T: Read + 'static,
{
    let first_line = read_line(&mut reader, true, string_encoding)?;
    let iter: Box<dyn CodePairIter> = match &*first_line {
        "AutoCAD Binary DXF" => {
            assert_or_err!(
                try_option_io_result_into_err!(read_u8(&mut reader)),
                0x1A,
                18
            );
            assert_or_err!(
                try_option_io_result_into_err!(read_u8(&mut reader)),
                0x00,
                19
            );
            Box::new(BinaryCodePairIter::new(reader, 20))
        }
        "AutoCAD DXB 1.0" => return Err(DxfError::InvalidBinaryFile),
        _ => Box::new(TextCodePairIter::new(
            reader,
            string_encoding,
            first_line,
            1,
        )),
    };
    Ok(iter)
}

#[cfg(test)]
mod tests {
    use crate::code_pair_iter::{BinaryCodePairIter, TextCodePairIter};
    use crate::CodePair;
    use std::io::BufReader;

    use super::{CodaSospesa, DirectCodePairIter};

    fn read_in_binary(codes_are_two_bytes: bool, data: Vec<u8>) -> CodePair {
        let mut reader = BinaryCodePairIter {
            reader: data.as_slice(),
            code_size_detection_complete: true,
            codes_are_two_bytes,
            offset: 0,
            coda: CodaSospesa::default(),
        };
        reader.read_code_pair().unwrap().unwrap()
    }

    #[test]
    fn read_string_in_binary() {
        // code 0x0001, value 0x41 = "A", NUL
        let pair = read_in_binary(true, vec![0x01, 0x00, 0x41, 0x00]);
        assert_eq!(1, pair.code);
        assert_eq!("A", pair.assert_string().expect("should be a string"));
    }

    #[test]
    fn read_binary_chunk_in_binary() {
        // code 0x136, length 2, data [0x01, 0x02]
        let pair = read_in_binary(true, vec![0x36, 0x01, 0x02, 0x01, 0x02]);
        assert_eq!(310, pair.code);
        assert_eq!(
            vec![0x01, 0x02],
            pair.assert_binary().expect("should be binary")
        );
    }

    fn read_in_text(data: &str) -> CodePair {
        let mut reader = TextCodePairIter::<&[u8]> {
            reader: BufReader::new(data.as_bytes()),
            string_encoding: encoding_rs::WINDOWS_1252,
            first_line: String::from("not-important"),
            read_first_line: true,
            offset: 0,
            coda: CodaSospesa::default(),
        };
        reader.read_code_pair().unwrap().unwrap()
    }

    #[test]
    fn read_binary_chunk_in_ascii() {
        let pair = read_in_text("310\r\n0102");
        assert_eq!(310, pair.code);
        assert_eq!(
            vec![0x01, 0x02],
            pair.assert_binary().expect("should be binary")
        );
    }

    #[test]
    fn read_code_450_in_binary() {
        // code 450 = 0x1C2, value = 37 (0x25)
        let pair = read_in_binary(true, vec![0xC2, 0x01, 0x25, 0x00, 0x00, 0x00]);
        assert_eq!(450, pair.code);
        assert_eq!(37, pair.assert_i32().expect("should be int"));
    }

    #[test]
    fn read_code_pairs_directly() {
        // really just a smoke test to verify the direct code pair reader
        let mut reader = DirectCodePairIter::new(vec![
            CodePair::new_f64(10, 1.0),
            CodePair::new_str(1, "abc"),
        ]);
        assert_eq!(
            Some(CodePair::new_f64(10, 1.0)),
            reader.next().unwrap().ok()
        );
        assert_eq!(
            Some(CodePair::new_str(1, "abc")),
            reader.next().unwrap().ok()
        );
        assert!(reader.next().is_none());
    }
}
