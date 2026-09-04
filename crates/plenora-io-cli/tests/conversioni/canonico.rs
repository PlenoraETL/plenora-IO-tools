//! Il dataset canonico, e gli strumenti con cui le conversioni lo confrontano.
//!
//! # Perche' un dataset solo
//!
//! Il confronto e' sempre contro **lo stesso** scenario, non contro l'uscita di
//! un'altra conversione. Confrontare due conversioni fra loro proverebbe che
//! sbagliano allo stesso modo.
//!
//! # Che cosa e' «normalizzato», e perche' passa dal filo
//!
//! I valori si leggono facendo scrivere al prodotto un CSV dell'uscita, e i
//! tipi si leggono da `plenora-io read`. Entrambi passano dalla busta pubblica,
//! che e' cio' di cui il prodotto risponde: una verifica che aprisse i file con
//! la libreria misurerebbe una superficie che il contratto non promette, e
//! davanti a un cambio del filo resterebbe verde.
//!
//! Il CSV e' il normalizzatore perche' e' l'unico formato che rende **ogni**
//! valore come testo, con una forma dichiarata: numeri per intero, temporali in
//! ISO, geometrie in WKT, la cella vuota per il null. Cio' che perde -- i tipi
//! e il CRS -- lo legge l'altro comando, e nessuno dei due basta da solo.

use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;

pub const fn binario() -> &'static str {
    env!("CARGO_BIN_EXE_plenora-io")
}

/// La directory delle fixture canoniche.
pub fn fixture(nome: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("canoniche")
        .join(nome)
}

/// L'esito di un comando: uscita, documento e diagnostica **grezzi**.
///
/// Il JSON non viene deserializzato qui: su un rifiuto lo stdout e' vuoto e su
/// un successo la busta d'errore non c'e'. Un tipo che tenesse un `Value`
/// dovrebbe scegliere che cosa mettere nell'altro caso, e quella scelta sarebbe
/// un ripiego che nasconde il giorno in cui il documento smette di uscire.
pub struct Esito {
    pub riuscito: bool,
    pub stdout: String,
    pub stderr: String,
}

impl Esito {
    pub fn documento(&self) -> serde_json::Value {
        serde_json::from_str(self.stdout.trim()).expect("il comando emette un documento JSON")
    }

    pub fn errore(&self) -> serde_json::Value {
        let busta: serde_json::Value =
            serde_json::from_str(self.stderr.trim()).expect("il rifiuto emette una busta JSON");
        busta["error"].clone()
    }

    pub fn messaggio(&self) -> String {
        self.errore()["message"]
            .as_str()
            .expect("la busta d'errore porta un messaggio")
            .to_owned()
    }

    /// Le categorie di perdita dichiarate in scrittura, con i conteggi.
    pub fn perdite(&self) -> BTreeMap<String, u64> {
        let counts = &self.documento()["write_loss"]["counts"];
        let mut mappa = BTreeMap::new();
        if let serde_json::Value::Array(voci) = counts {
            for voce in voci {
                let categoria = voce["categoria"]
                    .as_str()
                    .expect("ogni voce porta la propria categoria");
                let conteggio = voce["conteggio"]
                    .as_u64()
                    .expect("ogni voce porta il proprio conteggio");
                mappa.insert(categoria.to_owned(), conteggio);
            }
        }
        mappa
    }

    /// Le righe che la conversione dichiara di aver scritto.
    pub fn righe(&self) -> u64 {
        self.documento()["total_rows"]
            .as_u64()
            .expect("la busta di convert dichiara il totale delle righe")
    }
}

/// Esegue un comando della CLI.
pub fn cli(argomenti: &[&str]) -> Esito {
    let output = Command::new(binario())
        .args(argomenti)
        .output()
        .expect("il binario si esegue");
    Esito {
        riuscito: output.status.success(),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

/// Converte una fixture, con le opzioni che il registro dichiara per il caso.
pub fn converti(sorgente: &str, uscita: &Path, opzioni: &[&str]) -> Esito {
    let percorso = fixture(sorgente);
    let mut argomenti = vec![
        "convert",
        percorso.to_str().expect("percorso rappresentabile"),
        uscita.to_str().expect("percorso rappresentabile"),
    ];
    argomenti.extend_from_slice(opzioni);
    cli(&argomenti)
}

/// I campi dello schema che un file dichiara: nome, tipo, nullabilita'.
///
/// Vengono da `plenora-io read`, cioe' dalla busta che il prodotto pubblica:
/// sono la meta' che il CSV normalizzato non puo' dire, perche' li' e' tutto
/// testo.
pub fn schema(percorso: &Path, opzioni: &[&str]) -> Vec<(String, String, bool)> {
    let mut argomenti = vec!["read", percorso.to_str().expect("percorso rappresentabile")];
    argomenti.extend_from_slice(opzioni);
    let esito = cli(&argomenti);
    assert!(
        esito.riuscito,
        "la rilettura dello schema deve riuscire: {}",
        esito.stderr
    );
    let documento = esito.documento();
    let campi = documento["layer"]["fields"]
        .as_array()
        .expect("il layer dichiara i propri campi");
    let mut letti: Vec<(String, String, bool)> = Vec::with_capacity(campi.len());
    for campo in campi {
        letti.push((
            campo["name"].as_str().expect("nome").to_owned(),
            campo["type"].as_str().expect("tipo").to_owned(),
            campo["nullable"].as_bool().expect("nullabilita'"),
        ));
    }
    letti
}

/// I valori di un'uscita, normalizzati in testo attraverso il CSV.
///
/// Ogni riga e' una mappa `colonna -> valore`, e il valore e' `None` quando la
/// cella e' vuota. Le righe tornano nell'ordine del file: l'ordine **non** e'
/// garantito da nessun driver, quindi i confronti appaiano per `id` e non per
/// posizione -- appaiare per posizione proverebbe l'ordine invece dei valori.
pub fn valori(percorso: &Path, opzioni: &[&str]) -> Vec<BTreeMap<String, Option<String>>> {
    let temporanea = tempfile::tempdir().expect("directory temporanea");
    let csv = temporanea.path().join("normalizzato.csv");
    let mut argomenti = vec![
        "convert",
        percorso.to_str().expect("percorso rappresentabile"),
        csv.to_str().expect("percorso rappresentabile"),
    ];
    argomenti.extend_from_slice(opzioni);
    let esito = cli(&argomenti);
    assert!(
        esito.riuscito,
        "la normalizzazione in CSV deve riuscire: {}",
        esito.stderr
    );

    let testo = std::fs::read_to_string(&csv).expect("il CSV normalizzato si legge");
    let mut righe = testo.lines();
    let intestazione: Vec<String> = campi_csv(righe.next().expect("il CSV ha un'intestazione"));
    righe
        .map(|riga| {
            campi_csv(riga)
                .into_iter()
                .enumerate()
                .map(|(i, valore)| {
                    // Un campo oltre l'intestazione non e' una colonna senza
                    // nome: e' un CSV che il normalizzatore ha scritto male, e
                    // inventargli un nome nasconderebbe proprio quello.
                    let Some(nome) = intestazione.get(i).cloned() else {
                        panic!("il CSV normalizzato ha piu' campi dell'intestazione")
                    };
                    (
                        nome,
                        if valore.is_empty() {
                            None
                        } else {
                            Some(valore)
                        },
                    )
                })
                .collect()
        })
        .collect()
}

/// I campi di una riga CSV, con le virgolette del formato.
///
/// Scritto a mano invece di prendere un parser: la riga da leggere l'ha scritta
/// il prodotto, e un parser condiviso fra chi scrive e chi verifica
/// nasconderebbe un difetto simmetrico -- che e' la stessa ragione per cui le
/// fixture non nascono da plenora-io.
fn campi_csv(riga: &str) -> Vec<String> {
    let mut campi = Vec::new();
    let mut corrente = String::new();
    let mut fra_virgolette = false;
    let mut caratteri = riga.chars().peekable();
    while let Some(carattere) = caratteri.next() {
        match carattere {
            '"' if fra_virgolette && caratteri.peek() == Some(&'"') => {
                caratteri.next();
                corrente.push('"');
            }
            '"' => fra_virgolette = !fra_virgolette,
            ',' if !fra_virgolette => campi.push(std::mem::take(&mut corrente)),
            altro => corrente.push(altro),
        }
    }
    campi.push(corrente);
    campi
}

/// Le righe di un'uscita, indicizzate per `id`.
pub fn per_id(
    percorso: &Path,
    opzioni: &[&str],
) -> BTreeMap<String, BTreeMap<String, Option<String>>> {
    valori(percorso, opzioni)
        .into_iter()
        .map(|riga| {
            let id = riga
                .get("id")
                .cloned()
                .flatten()
                .expect("ogni riga del dataset canonico porta il proprio id");
            (id, riga)
        })
        .collect()
}

// --- lo scenario, riga per riga ---------------------------------------------

/// Il valore che ogni riga porta in ciascuna colonna, come testo.
///
/// E' la forma normalizzata dello scenario, non una seconda copia dei dati: i
/// generatori delle fixture hanno la propria, e queste sono le stesse cifre
/// **lette attraverso il CSV**. Se una conversione le altera, e' qui che si
/// vede.
pub struct Attesa {
    pub id: &'static str,
    pub codice: &'static str,
    pub etichetta: Option<&'static str>,
    pub intero_largo: &'static str,
    pub misura: Option<&'static str>,
    pub istante: &'static str,
    pub geometria: Option<&'static str>,
}

/// Le cinque righe, nella variante proiettata.
pub const ATTESE: &[Attesa] = &[
    Attesa {
        id: "r1",
        codice: "A-1",
        etichetta: Some("città"),
        // 2^53+1, che un double non rappresenta: se una conversione lo fa
        // passare da un float64 torna indietro come ...992, e la differenza di
        // uno e' esattamente cio' che un confronto per valore trova.
        intero_largo: "9007199254740993",
        misura: Some("1.5"),
        istante: "2026-01-15",
        geometria: Some("POINT (1650000 4850000)"),
    },
    Attesa {
        id: "r2",
        codice: "B-2",
        // Il null, che non e' la stringa vuota.
        etichetta: None,
        intero_largo: "-9007199254740993",
        misura: Some("-0.125"),
        istante: "2026-02-28",
        geometria: Some("LINESTRING (1650000 4850000, 1650100 4850100)"),
    },
    Attesa {
        id: "r3",
        codice: "Ç-3",
        etichetta: Some("naïve"),
        intero_largo: "0",
        misura: None,
        istante: "2026-03-01",
        geometria: Some(
            "POLYGON ((1651000 4851000, 1651100 4851000, 1651100 4851100, 1651000 4851100, 1651000 4851000))",
        ),
    },
    Attesa {
        id: "r4",
        codice: "D-4",
        // La stringa vuota **e'** un valore, e nel CSV non si distingue dal
        // null: e' la ragione per cui il confronto sulla stringa vuota si fa
        // dove il formato le distingue, non qui.
        etichetta: Some(""),
        intero_largo: "1",
        misura: Some("3.141592653589793"),
        istante: "2026-12-31",
        geometria: Some("POINT Z (1652000 4852000 125.5)"),
    },
    Attesa {
        id: "r5",
        codice: "E-5",
        etichetta: Some("senza geometria"),
        intero_largo: "9007199254740992",
        misura: Some("0"),
        istante: "2026-06-30",
        geometria: None,
    },
];

/// Due WKT che descrivono la stessa geometria, confrontabili.
///
/// Le sole differenze ammesse sono gli spazi **fra i token** che la grammatica
/// WKT lascia liberi: quello fra la parola del tipo e la parentesi, e quello
/// dopo una virgola. OGR li scrive, il nostro writer no, e `POINT (1 2)` e
/// `POINT(1 2)` sono la stessa geometria.
///
/// Lo spazio **dentro** una coordinata non viene toccato, ed e' la parte che
/// conta: `1 2` separa x da y, e normalizzarlo accetterebbe due geometrie
/// diverse come se fossero la stessa.
#[must_use]
pub fn stessa_geometria(uno: Option<&str>, altro: Option<&str>) -> bool {
    fn senza_spazi_fra_i_token(wkt: &str) -> String {
        wkt.replace(" (", "(").replace(", ", ",")
    }
    match (uno, altro) {
        (None, None) => true,
        (Some(uno), Some(altro)) => senza_spazi_fra_i_token(uno) == senza_spazi_fra_i_token(altro),
        _ => false,
    }
}

/// La riga attesa con quell'id.
pub fn attesa(id: &str) -> &'static Attesa {
    ATTESE
        .iter()
        .find(|a| a.id == id)
        .expect("l'id e' fra quelli dello scenario canonico")
}
