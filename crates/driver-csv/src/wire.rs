//! Presenza e quoting sono parte del contratto CSV, prima dell'inferenza.
use std::io::{BufRead, BufReader, BufWriter, Read, Write};

pub struct Reader<R> {
    input: BufReader<R>,
    delimiter: u8,
    pub quoted: Vec<bool>,
    width: Option<usize>,
    cell: Vec<u8>,
}

fn invalid() -> std::io::Error {
    std::io::Error::from(std::io::ErrorKind::InvalidData)
}

impl<R: Read> Reader<R> {
    pub fn new(input: R, delimiter: u8) -> Self {
        Self {
            input: BufReader::new(input),
            delimiter,
            quoted: Vec::new(),
            width: None,
            cell: Vec::new(),
        }
    }

    fn byte(&mut self) -> std::io::Result<Option<u8>> {
        let byte = self.input.fill_buf()?.first().copied();
        if byte.is_some() {
            self.input.consume(1);
        }
        Ok(byte)
    }

    // Strict quoting: malformed input cannot silently acquire a different presence.
    pub fn read_record(&mut self, record: &mut csv::StringRecord) -> std::io::Result<bool> {
        if self.width.is_none() && self.input.fill_buf()?.starts_with(b"\xef\xbb\xbf") {
            self.input.consume(3);
        }
        self.quoted.clear();
        let mut fields = std::mem::take(record).into_byte_record();
        fields.clear();
        let mut cell = std::mem::take(&mut self.cell);
        cell.clear();
        let mut quoted = false;
        let mut in_quotes = false;
        let mut closed = false;
        let mut started = false;
        loop {
            // Copia in blocco soltanto i byte che non cambiano lo stato del
            // quoting. Delimitatori, virgolette, CR/LF ed EOF passano ancora
            // tutti dalla macchina rigorosa qui sotto, anche a cavallo del
            // buffer. Evita fill_buf/consume e push per ogni byte di testo.
            if !closed {
                let buffer = self.input.fill_buf()?;
                let length = buffer
                    .iter()
                    .position(|&b| {
                        b == b'"'
                            || (!in_quotes && (b == self.delimiter || b == b'\r' || b == b'\n'))
                    })
                    .unwrap_or(buffer.len());
                if length != 0 {
                    cell.extend_from_slice(&buffer[..length]);
                    self.input.consume(length);
                    started = true;
                    continue;
                }
            }
            let byte = self.byte()?;
            if byte.is_none() && !started {
                return Ok(false);
            }
            started = true;
            if in_quotes {
                match byte {
                    None => return Err(invalid()),
                    Some(b'"') => {
                        in_quotes = false;
                        closed = true;
                    }
                    Some(b) => cell.push(b),
                }
                continue;
            }
            if closed && byte == Some(b'"') {
                cell.push(b'"');
                in_quotes = true;
                closed = false;
                continue;
            }
            let end = matches!(byte, None | Some(b'\r' | b'\n'));
            if end || byte == Some(self.delimiter) {
                fields.push_field(&cell);
                self.quoted.push(quoted);
                cell.clear();
                quoted = false;
                closed = false;
                if end {
                    if byte == Some(b'\r') && self.input.fill_buf()?.first() == Some(&b'\n') {
                        self.input.consume(1);
                    }
                    break;
                }
            } else if byte == Some(b'"') && cell.is_empty() && !closed {
                quoted = true;
                in_quotes = true;
            } else {
                if closed || byte == Some(b'"') {
                    return Err(invalid());
                }
                if let Some(b) = byte {
                    cell.push(b);
                }
            }
        }
        if self.width.is_some_and(|width| width != fields.len()) {
            return Err(invalid());
        }
        self.width = Some(fields.len());
        *record = csv::StringRecord::from_byte_record(fields).map_err(|_| invalid())?;
        self.cell = cell;
        Ok(true)
    }
}

pub struct Writer<W: Write> {
    output: BufWriter<W>,
    delimiter: u8,
    first: bool,
}

impl<W: Write> Writer<W> {
    pub fn new(output: W, delimiter: u8) -> Self {
        Self {
            output: BufWriter::new(output),
            delimiter,
            first: true,
        }
    }

    pub fn write_field(&mut self, value: impl AsRef<[u8]>) -> std::io::Result<()> {
        self.field(value.as_ref(), false)
    }

    pub fn write_text(&mut self, value: impl AsRef<[u8]>) -> std::io::Result<()> {
        self.field(value.as_ref(), true)
    }

    fn field(&mut self, value: &[u8], text: bool) -> std::io::Result<()> {
        if !self.first {
            self.output.write_all(&[self.delimiter])?;
        }
        self.first = false;
        let quote = text
            || value
                .iter()
                .any(|b| [self.delimiter, b'"', b'\r', b'\n'].contains(b));
        if quote {
            self.output.write_all(b"\"")?;
        }
        for &byte in value {
            self.output.write_all(&[byte])?;
            if byte == b'"' {
                self.output.write_all(b"\"")?;
            }
        }
        if quote {
            self.output.write_all(b"\"")?;
        }
        Ok(())
    }

    pub fn write_record<I, T>(&mut self, fields: I) -> std::io::Result<()>
    where
        I: IntoIterator<Item = T>,
        T: AsRef<[u8]>,
    {
        for field in fields {
            if self.first && field.as_ref().starts_with(b"\xef\xbb\xbf") {
                self.write_text(field)?;
            } else {
                self.write_field(field)?;
            }
        }
        self.output.write_all(b"\r\n")?;
        self.first = true;
        Ok(())
    }

    pub fn flush(&mut self) -> std::io::Result<()> {
        self.output.flush()
    }
}
