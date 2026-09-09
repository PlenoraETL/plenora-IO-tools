//! Quali cicli di `kml` non tornano su un documento troncato.
//!
//! L'assenza di un ramo `Eof` non prova un ciclo infinito: il ciclo potrebbe
//! essere irraggiungibile a fine file, oppure `next_event` potrebbe restituire
//! un errore invece di `Eof`. Qui si misura, un elemento per volta.
//!
//! Ogni caso gira in un thread suo, e il verdetto e' se torna entro il tetto.
//! Un thread che non torna resta appeso: e' il motivo per cui il processo esce
//! con `std::process::exit` invece di aspettarli tutti.

use std::sync::mpsc;
use std::thread;
use std::time::Duration;

/// Gli elementi che `read_elements` sa dispacciare, uno per ciclo interno.
const ELEMENTI: &[&str] = &[
    "Point",
    "LineString",
    "LinearRing",
    "Polygon",
    "MultiGeometry",
    "Placemark",
    "Location",
    "Orientation",
    "Scale",
    "Folder",
    "Document",
    "Style",
    "StyleMap",
    "Pair",
    "IconStyle",
    "Icon",
    "Link",
    "ResourceMap",
    "Alias",
    "Data",
    "SchemaData",
    "SimpleArrayData",
    "BalloonStyle",
    "LabelStyle",
    "LineStyle",
    "ListStyle",
    "PolyStyle",
    "ExtendedData",
    "outerBoundaryIs",
    "innerBoundaryIs",
    "coordinates",
];

fn misura(documento: String, tetto: Duration) -> Option<bool> {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let esito = documento.parse::<kml::Kml>();
        let _ = tx.send(esito.is_ok());
    });
    match rx.recv_timeout(tetto) {
        Ok(ok) => Some(ok),
        Err(_) => None,
    }
}

fn main() {
    let tetto = Duration::from_secs(3);
    println!("kml {} -- tetto {:?} per caso", env!("CARGO_PKG_VERSION"), tetto);
    println!();

    let mut appesi = Vec::new();
    let mut tornati = Vec::new();

    // 1. L'elemento aperto e mai chiuso, da solo.
    println!("--- `<X>` troncato ---");
    for e in ELEMENTI {
        let doc = format!("<{e}>");
        match misura(doc, tetto) {
            Some(ok) => {
                tornati.push(*e);
                println!("  torna ({})   {e}", if ok { "Ok" } else { "Err" });
            }
            None => {
                appesi.push(*e);
                println!("  NON TORNA    {e}");
            }
        }
    }

    // 2. Il caso vero trovato dal fuzzing: dentro un documento, con del testo.
    println!();
    println!("--- il riproduttore del fuzzing, ridotto ---");
    let caso = "<xjA:Point>0j><xjA:Point>";
    match misura(caso.to_owned(), tetto) {
        Some(ok) => println!("  torna ({})   {caso}", if ok { "Ok" } else { "Err" }),
        None => println!("  NON TORNA    {caso}"),
    }

    // 3. Un documento ben formato: la controprova che il banco non dice
    //    «non torna» per qualunque cosa.
    println!();
    println!("--- controprova: un documento chiuso ---");
    let buono = "<Placemark><name>x</name><Point><coordinates>1,2</coordinates></Point></Placemark>";
    match misura(buono.to_owned(), tetto) {
        Some(ok) => println!("  torna ({})   documento ben formato", if ok { "Ok" } else { "Err" }),
        None => println!("  NON TORNA    documento ben formato -- il banco e' rotto"),
    }

    println!();
    println!("appesi: {} su {}", appesi.len(), ELEMENTI.len());
    println!("  {appesi:?}");
    println!("tornati: {}", tornati.len());
    println!("  {tornati:?}");
    std::process::exit(0);
}
