# Segnalazione a monte — `calamine`: overflow di `u32` sul riferimento di cella

Stato: **pronta, non pubblicata**. Richiede autorizzazione esplicita prima di
essere aperta su `tafia/calamine` o inviata altrove. Il testo sotto la linea è
già in inglese e pronto da incollare; sopra la linea c'è quello che serve a noi.

## Perché è preparata e non aperta

Il difetto è stato trovato dal nostro fuzzing (smoke del 2026-08-17, target
`xlsx_reader`) e riguarda una libreria di terze parti. Aprire una issue è una
comunicazione pubblica a nome del progetto: la decide chi ne ha titolo, non chi
la scrive. Il contenuto è quindi pronto e verificato, e resta qui finché non
viene autorizzato.

Il reproducer allegato **non** contiene nostri artefatti: è generato da uno
script di venti righe che costruisce un `.xlsx` minimo. L'input che ha prodotto
il finding — `fuzz/seeds/xlsx_reader/riferimento-cella-oltre-u32.xlsx` — è una
mutazione del nostro corpus e non serve a monte.

## Rapporto con la mitigazione locale

`driver-xls` ha una barriera propria (`leggendo_calamine`, XLSX-HARDENING) che
converte il panico in `PlenoraIoError` di fase `Read`. **Un aggiornamento di
`calamine` non la sostituisce**: chiuderebbe questo difetto, non la classe di
difetti. La barriera si toglie solo se e quando la libreria dichiara fallibile
quella conversione, e comunque per decisione separata — la stessa regola che
vale per la barriera arrow.

Verificato che il difetto è ancora presente in `calamine 0.36.1`, che è il pin
esatto del workspace.

---

## Unchecked `u32` arithmetic in `get_row_and_optional_column` panics on crafted cell references

**Version:** 0.36.1
**File:** `src/xlsx/mod.rs`, `get_row_and_optional_column` (lines 2837, 2838, 2853)

### Summary

`get_row_and_optional_column` accumulates the column and row components of a
cell reference with unchecked arithmetic:

```rust
c @ b'A'..=b'Z' => col = col * 26 + (c - b'A') as u32 + 1,   // line 2837
c @ b'a'..=b'z' => col = col * 26 + (c - b'a') as u32 + 1,   // line 2838
...
c @ b'0'..=b'9' => row = row * 10 + (c - b'0') as u32,       // line 2853
```

The number of letters (and digits) is not bounded. A worksheet whose `<c r="…">`
attribute carries seven or more letters overflows `u32`:

* in debug builds, or in any release build with `overflow-checks = true`, this
  is a **panic** — `attempt to multiply with overflow`;
* in a default release build the multiplication **wraps silently**, and the
  cell is reported at a position that does not correspond to the file.

The second case is arguably the worse one: no error is raised and the caller
gets plausible, wrong coordinates.

The largest column Excel can represent is `XFD` (three letters), so no valid
document reaches this. The input, however, comes from a file, and the parser
accepts an arbitrary run of letters before checking anything.

### Reproducer

No crafted binary needed — this script writes a minimal, structurally valid
`.xlsx` whose only unusual property is a nine-letter cell reference:

```python
import zipfile

CONTENT_TYPES = """<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
<Default Extension="xml" ContentType="application/xml"/>
<Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/>
<Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>
</Types>"""

RELS = """<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/>
</Relationships>"""

WORKBOOK = """<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
<sheets><sheet name="Sheet1" sheetId="1" r:id="rId1"/></sheets>
</workbook>"""

WORKBOOK_RELS = """<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/>
</Relationships>"""

SHEET = """<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
<dimension ref="A1:A2"/>
<sheetData>
<row r="1"><c r="A1" t="str"><v>header</v></c></row>
<row r="2"><c r="AAAAAAAAA1" t="str"><v>value</v></c></row>
</sheetData>
</worksheet>"""

with zipfile.ZipFile("overflow.xlsx", "w", zipfile.ZIP_DEFLATED) as z:
    z.writestr("[Content_Types].xml", CONTENT_TYPES)
    z.writestr("_rels/.rels", RELS)
    z.writestr("xl/workbook.xml", WORKBOOK)
    z.writestr("xl/_rels/workbook.xml.rels", WORKBOOK_RELS)
    z.writestr("xl/worksheets/sheet1.xml", SHEET)
```

Then, with `overflow-checks = true` (or a debug build):

```rust
use calamine::{open_workbook, Reader, Xlsx};

fn main() {
    let mut wb: Xlsx<_> = open_workbook("overflow.xlsx").unwrap();
    let mut cells = wb.worksheet_cells_reader("Sheet1").unwrap();
    while cells.next_cell().unwrap().is_some() {}
}
```

Observed:

```
thread 'main' panicked at src/xlsx/mod.rs:2837:38:
attempt to multiply with overflow
   core::panicking::panic_const::panic_const_mul_overflow
   get_row_and_optional_column
   get_row_column
   next_cell
```

Lowercase references (`r="bncasufw1"`) hit line 2838 instead; a long run of
digits hits line 2853 the same way.

### Suggested fix

Make the accumulation fallible instead of wrapping — `checked_mul` /
`checked_add`, returning the existing `XlsxError` variant for a malformed
range, or a new one. Bounding the number of letters to three and the digits to
seven (the actual XLSX limits: `XFD`, 1_048_576 rows) would also work and would
reject earlier.

Either way the important part is that a file cannot make the conversion produce
a position it did not contain: a silent wrap in release builds is harder to
notice than the panic.

### How it was found

Coverage-guided fuzzing of an XLSX reader built on `calamine`, with
`overflow-checks = true` in the fuzzing profile. The finding reproduces in
about one second and is deterministic.
