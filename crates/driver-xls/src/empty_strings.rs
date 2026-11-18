//! Celle stringa vuota esplicite nel documento generato dal writer.

use std::collections::BTreeSet;
use std::io::{Cursor, Read, Write};

use plenora_io_model::{PublicMessage, Result};

pub fn encode(buffer: Vec<u8>, cells: &BTreeSet<String>) -> Result<Vec<u8>> {
    if cells.is_empty() {
        return Ok(buffer);
    }
    let failure = || {
        super::err(&PublicMessage::Curated(
            "serializzazione delle stringhe vuote XLSX fallita",
        ))
    };
    let mut source = zip::ZipArchive::new(Cursor::new(buffer)).map_err(|_| failure())?;
    let mut output = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for index in 0..source.len() {
        let mut member = source.by_index(index).map_err(|_| failure())?;
        if member.name().map_err(|_| failure())? != "xl/worksheets/sheet1.xml" {
            output.raw_copy_file(member).map_err(|_| failure())?;
            continue;
        }
        let mut xml = String::new();
        member.read_to_string(&mut xml).map_err(|_| failure())?;
        output
            .start_file(
                "xl/worksheets/sheet1.xml",
                zip::write::SimpleFileOptions::default(),
            )
            .map_err(|_| failure())?;
        let mut replaced = 0;
        for segment in xml.split_inclusive("</c>") {
            let replacement = segment.rfind("<c r=\"").and_then(|start| {
                let reference = segment[start + 6..].split('"').next()?;
                cells.contains(reference).then_some((start, reference))
            });
            if let Some((start, reference)) = replacement {
                output.write_all(&segment.as_bytes()[..start])?;
                write!(
                    output,
                    "<c r=\"{reference}\" t=\"inlineStr\"><is><t></t></is></c>"
                )?;
                replaced += 1;
            } else {
                output.write_all(segment.as_bytes())?;
            }
        }
        if replaced != cells.len() {
            return Err(failure());
        }
    }
    Ok(output.finish().map_err(|_| failure())?.into_inner())
}
