//! Un foglio XLSX senza celle non lascia una destinazione.
//!
//! # La meta' che il driver non puo' mostrare
//!
//! La revisione H-01 ammette un ripiego in `driver-xls`: quando la scansione
//! dei limiti non osserva nemmeno una cella, la cornice restituita e' quella
//! degenere. La deroga vale a una condizione -- che da quella cornice sintetica
//! non nasca **niente** -- e il driver ne prova due terzi da se': `open`
//! fallisce, e senza handle non c'e' schema da leggere ne' batch da consegnare.
//!
//! La terza parte, che nessuna **uscita** venga scritta, si osserva soltanto
//! dove una destinazione esiste: qui, dal binario. Il file sta a se' perche' e'
//! una condizione di una revisione nominata, non un caso della matrice delle
//! conversioni: metterlo li' l'avrebbe fatto sembrare una conversione che il
//! prodotto promette.

use std::process::Command;

const fn binario() -> &'static str {
    env!("CARGO_BIN_EXE_plenora-io")
}

/// Un XLSX conforme, senza `<dimension>` e con le celle date.
///
/// Scritto a mano: `<dimension>` e' opzionale in ECMA-376, e una libreria che
/// non sa ometterlo non puo' produrre il caso. Le parti sono le cinque minime
/// che un pacchetto XLSX pretende.
fn xlsx_con(celle: &str) -> Vec<u8> {
    let parti = [
        (
            "[Content_Types].xml",
            "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
             <Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">\
             <Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/>\
             <Default Extension=\"xml\" ContentType=\"application/xml\"/>\
             <Override PartName=\"/xl/workbook.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml\"/>\
             <Override PartName=\"/xl/worksheets/sheet1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml\"/>\
             </Types>",
        ),
        (
            "_rels/.rels",
            "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
             <Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
             <Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"xl/workbook.xml\"/>\
             </Relationships>",
        ),
        (
            "xl/workbook.xml",
            "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
             <workbook xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\" xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\">\
             <sheets><sheet name=\"foglio\" sheetId=\"1\" r:id=\"rId1\"/></sheets></workbook>",
        ),
        (
            "xl/_rels/workbook.xml.rels",
            "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
             <Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
             <Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet\" Target=\"worksheets/sheet1.xml\"/>\
             </Relationships>",
        ),
    ];
    // Il foglio si costruisce a parte perche' e' l'unica parte che varia, ed e'
    // la variabile delle sonde: quante celle il file contiene.
    let foglio = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
         <worksheet xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\">\
         <sheetData>{celle}</sheetData></worksheet>"
    );

    let mut buffer = std::io::Cursor::new(Vec::new());
    {
        let mut scrittore = zip::ZipWriter::new(&mut buffer);
        let opzioni: zip::write::FileOptions<'_, ()> =
            zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Stored);
        for (nome, testo) in parti.into_iter().chain(std::iter::once((
            "xl/worksheets/sheet1.xml",
            foglio.as_str(),
        ))) {
            scrittore.start_file(nome, opzioni).unwrap();
            std::io::Write::write_all(&mut scrittore, testo.as_bytes()).unwrap();
        }
        scrittore.finish().unwrap();
    }
    buffer.into_inner()
}

/// La conversione si ferma in lettura, e la destinazione non nasce.
///
/// Il controllo positivo sta nella stessa sonda: lo **stesso** comando su un
/// foglio che una cella ce l'ha produce il file. Senza, «la destinazione non
/// esiste» sarebbe vero anche di un binario che non scrive mai niente.
#[test]
fn un_foglio_senza_celle_non_lascia_una_destinazione() {
    let dir = tempfile::tempdir().expect("directory temporanea");

    let vuoto = dir.path().join("vuoto.xlsx");
    std::fs::write(&vuoto, xlsx_con("")).expect("sorgente scritta");
    let uscita = dir.path().join("da-vuoto.csv");
    let esito = converti(&vuoto, &uscita);
    assert!(
        !esito.status.success(),
        "un foglio senza celle non ha un layout da inferire: la conversione non \
         puo' riuscire"
    );
    assert!(
        !uscita.exists(),
        "e non lascia una destinazione: dalla cornice sintetica non nasce output"
    );

    // Il controllo positivo, con lo stesso comando e una cella in piu'.
    let pieno = dir.path().join("pieno.xlsx");
    std::fs::write(
        &pieno,
        xlsx_con(
            "<row r=\"1\"><c r=\"A1\" t=\"inlineStr\"><is><t>geometry</t></is></c></row>             <row r=\"2\"><c r=\"A2\" t=\"inlineStr\"><is><t>POINT (1 2)</t></is></c></row>",
        ),
    )
    .expect("sorgente scritta");
    let uscita = dir.path().join("da-pieno.csv");
    let esito = converti(&pieno, &uscita);
    assert!(
        esito.status.success(),
        "lo stesso comando su un foglio con celle deve riuscire: {}",
        String::from_utf8_lossy(&esito.stderr)
    );
    assert!(uscita.exists(), "e lasciare la propria destinazione");
}

fn converti(sorgente: &std::path::Path, uscita: &std::path::Path) -> std::process::Output {
    Command::new(binario())
        .args([
            "convert",
            sorgente.to_str().expect("percorso rappresentabile"),
            uscita.to_str().expect("percorso rappresentabile"),
            "--assume-crs",
            "EPSG:4326",
            "--in-opt",
            "wkt_column=geometry",
        ])
        .output()
        .expect("il binario si esegue")
}
