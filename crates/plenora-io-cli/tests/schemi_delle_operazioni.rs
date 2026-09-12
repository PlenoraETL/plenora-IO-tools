//! Gli schemi delle operazioni, i loro esempi, e il legame con il binario.
//!
//! # Che cosa questa prova aggiunge a quelle sulla consegna
//!
//! `tests/consegna_arrow.rs` guarda i byte consegnati e dice che il prodotto fa
//! quello che fa. Non dice che sia quello che deve fare: per dirlo serve una
//! forma dichiarata **prima**, contro cui misurare. Qui ci sono i due schemi --
//! ingresso e risultato -- e ogni busta emessa da un'invocazione reale valida
//! contro quello del risultato.
//!
//! # Perche' gli esempi invalidi non bastano da soli
//!
//! Un esempio invalido che il validatore rifiuta prova soltanto che il
//! validatore sa dire di no. Per contare, ogni esempio invalido dell'ingresso
//! porta nel manifesto l'invocazione CLI corrispondente e il codice d'errore
//! che il prodotto deve rendere: lo schema e il binario devono rifiutare **lo
//! stesso** insieme di ingressi. Uno dei due che accettasse dove l'altro
//! rifiuta e' il difetto che questa prova esiste per cogliere -- ed e' come
//! sono emersi `--in-opt` scartato in silenzio e `--out-opt` senza sink.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

const BINARIO: &str = env!("CARGO_BIN_EXE_plenora-io");

fn radice() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("la radice del workspace sta due livelli sopra il crate")
        .to_path_buf()
}

fn contratti() -> PathBuf {
    radice().join("contracts")
}

fn fixture(nome: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("canoniche")
        .join(nome)
}

fn documento(percorso: &Path) -> Value {
    let testo = std::fs::read_to_string(percorso)
        .unwrap_or_else(|e| panic!("{} si legge: {e}", percorso.display()));
    serde_json::from_str(&testo).unwrap_or_else(|e| panic!("{} e' JSON: {e}", percorso.display()))
}

fn compila(nome: &str) -> jsonschema::Validator {
    let percorso = contratti()
        .join("schemas")
        .join(format!("{nome}.schema.json"));
    let schema = documento(&percorso);
    jsonschema::validator_for(&schema)
        .unwrap_or_else(|e| panic!("{nome} e' uno schema valido: {e}"))
}

fn manifesto() -> Value {
    documento(&contratti().join("esempi").join("manifesto.json"))
}

/// Ogni esempio valida -- o non valida -- come il manifesto dichiara.
///
/// Il manifesto e' la tabella, non la prova: elenca il file, lo schema e il
/// verdetto atteso. Un esempio che cambiasse verdetto senza che nessuno
/// aggiorni il manifesto e' rosso, ed e' il punto.
#[test]
fn gli_esempi_valgono_quanto_il_manifesto_dichiara() {
    let manifesto = manifesto();
    let esempi = manifesto["esempi"]
        .as_array()
        .expect("il manifesto elenca esempi");
    assert!(esempi.len() >= 24, "il manifesto non e' stato svuotato");

    let mut visti = 0usize;
    for voce in esempi {
        let file = voce["file"].as_str().expect("ogni voce nomina un file");
        let nome_schema = voce["schema"]
            .as_str()
            .expect("ogni voce nomina uno schema");
        let atteso = voce["atteso"]
            .as_str()
            .expect("ogni voce dichiara un verdetto");
        let validatore = compila(nome_schema);
        let doc = documento(&contratti().join("esempi").join(file));
        let valido = validatore.is_valid(&doc);
        match atteso {
            "valido" => assert!(
                valido,
                "{file} e' dichiarato valido ma non lo e': {:?}",
                validatore
                    .iter_errors(&doc)
                    .map(|e| e.to_string())
                    .collect::<Vec<_>>()
            ),
            "invalido" => assert!(
                !valido,
                "{file} e' dichiarato invalido e lo schema lo accetta: o l'esempio \
                 non e' piu' il caso che descriveva, o lo schema ha smesso di \
                 vincolare cio' che diceva di vincolare"
            ),
            altro => panic!("{file}: verdetto sconosciuto «{altro}»"),
        }
        visti += 1;
    }
    assert_eq!(
        visti,
        esempi.len(),
        "ogni voce del manifesto e' stata provata"
    );
}

/// Le forme reali di `io.read` validano contro lo schema del risultato.
///
/// Non un documento scritto a mano: l'uscita del binario, letta da stdout.
/// Cio' che lo schema descrive e cio' che il prodotto emette devono essere la
/// stessa cosa, e questo e' il punto in cui si vede.
#[test]
fn le_buste_reali_validano_contro_lo_schema_del_risultato() {
    let validatore = compila("plenora-io-read-result-v1");
    let temporanea = tempfile::tempdir().expect("directory temporanea");
    let uscita = temporanea.path().join("consegnata.arrow");

    let casi: Vec<Vec<String>> = vec![
        vec![
            "read".to_owned(),
            fixture("canonico.geojson").display().to_string(),
        ],
        vec![
            "read".to_owned(),
            fixture("canonico.geojson").display().to_string(),
            "--output".to_owned(),
            uscita.display().to_string(),
        ],
        vec![
            "read".to_owned(),
            fixture("canonico.gpkg").display().to_string(),
            "--layer".to_owned(),
            "0".to_owned(),
        ],
    ];

    for argomenti in casi {
        let uscita_processo = Command::new(BINARIO)
            .args(&argomenti)
            .arg("--format")
            .arg("json")
            .output()
            .expect("il binario parte");
        let stdout = String::from_utf8(uscita_processo.stdout).expect("stdout e' UTF-8");
        let busta: Value = serde_json::from_str(&stdout)
            .unwrap_or_else(|e| panic!("{argomenti:?}: stdout e' JSON: {e} -- {stdout}"));
        assert_eq!(busta["status"], "ok", "{argomenti:?}: {stdout}");
        let risultato = &busta["result"];
        assert!(
            validatore.is_valid(risultato),
            "{argomenti:?}: il risultato non valida: {:?}",
            validatore
                .iter_errors(risultato)
                .map(|e| format!("{} in {}", e, e.instance_path()))
                .collect::<Vec<_>>()
        );
    }
}

/// Cio' che lo schema d'ingresso rifiuta, il binario lo rifiuta.
///
/// # Perche' questa e' la prova che conta
///
/// Uno schema d'ingresso puo' essere scritto per descrivere quel che il
/// prodotto gia' fa, e allora non misura niente. Qui il verso e' l'opposto:
/// il manifesto porta, per ogni esempio invalido, l'invocazione equivalente e
/// il codice che il prodotto deve rendere. Se il binario accettasse un
/// ingresso che lo schema dichiara invalido, la sonda e' rossa -- e lo e' stata
/// due volte mentre questo blocco veniva scritto.
#[test]
fn il_binario_rifiuta_gli_ingressi_che_lo_schema_dichiara_invalidi() {
    let manifesto = manifesto();
    let temporanea = tempfile::tempdir().expect("directory temporanea");
    let sorgente = fixture("canonico.geojson").display().to_string();

    let mut provati = 0usize;
    for (indice, voce) in manifesto["esempi"]
        .as_array()
        .expect("esempi")
        .iter()
        .enumerate()
    {
        let Some(argomenti) = voce["invocazione_cli"].as_array() else {
            continue;
        };
        let Some(codice) = voce["codice_del_prodotto"].as_str() else {
            continue;
        };
        let uscita = temporanea.path().join(format!("u{indice}.arrow"));
        let argomenti: Vec<String> = argomenti
            .iter()
            .map(|a| match a.as_str().expect("argomento testuale") {
                "SORGENTE" => sorgente.clone(),
                "USCITA" => uscita.display().to_string(),
                altro => altro.to_owned(),
            })
            .collect();

        let esito = Command::new(BINARIO)
            .args(&argomenti)
            .arg("--format")
            .arg("json")
            .output()
            .expect("il binario parte");
        let stdout = String::from_utf8(esito.stdout).expect("stdout e' UTF-8");
        let busta: Value = serde_json::from_str(&stdout)
            .unwrap_or_else(|e| panic!("{argomenti:?}: stdout e' JSON: {e} -- {stdout}"));

        assert_eq!(
            busta["status"], "error",
            "{}: lo schema lo dichiara invalido e il binario lo accetta -- {stdout}",
            voce["file"]
        );
        assert_eq!(
            busta["error"]["code"], codice,
            "{}: rifiutato con un codice diverso da quello dichiarato -- {stdout}",
            voce["file"]
        );
        assert!(
            !uscita.exists(),
            "{}: un rifiuto non lascia una destinazione",
            voce["file"]
        );
        provati += 1;
    }
    assert!(
        provati >= 6,
        "il manifesto deve legare al binario i rifiuti noti di read e write, legati: {provati}"
    );
}

/// Gli esempi sul disco sono tutti e soli quelli del manifesto.
///
/// Senza questo, un esempio cancellato lascerebbe il manifesto verde su una
/// voce che non c'e' piu', e uno aggiunto a mano non verrebbe provato da
/// nessuno. Sono i due casi opposti, e vanno visti entrambi.
#[test]
fn il_manifesto_e_la_directory_coincidono() {
    let manifesto = manifesto();
    let dichiarati: std::collections::BTreeSet<String> = manifesto["esempi"]
        .as_array()
        .expect("esempi")
        .iter()
        .map(|v| v["file"].as_str().expect("file").to_owned())
        .collect();

    let mut trovati = std::collections::BTreeSet::new();
    for cartella in ["validi", "invalidi"] {
        let dir = contratti().join("esempi").join(cartella);
        for voce in std::fs::read_dir(&dir).expect("la directory degli esempi esiste") {
            let voce = voce.expect("voce leggibile");
            let percorso = voce.path();
            if percorso
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("json"))
            {
                let nome = voce.file_name().to_string_lossy().into_owned();
                trovati.insert(format!("{cartella}/{nome}"));
            }
        }
    }

    assert_eq!(
        dichiarati, trovati,
        "gli esempi dichiarati e quelli sul disco devono coincidere"
    );
}

/// E lo stesso per `io.write`: la busta reale contro il suo schema.
///
/// Qui la catena e' completa -- si legge una sorgente, si consegna Arrow, si
/// pubblica -- ed e' il punto: i due schemi descrivono due operazioni che si
/// incatenano, e provarli su invocazioni scollegate direbbe meno.
#[test]
fn la_busta_di_write_valida_contro_il_suo_schema() {
    let validatore = compila("plenora-io-write-result-v1");
    let temporanea = tempfile::tempdir().expect("directory temporanea");
    let arrow = temporanea.path().join("ponte.arrow");

    let consegna = Command::new(BINARIO)
        .args([
            "read",
            fixture("canonico.geojson").to_str().unwrap(),
            "--output",
            arrow.to_str().unwrap(),
            "--format",
            "json",
        ])
        .output()
        .expect("il binario parte");
    assert!(
        consegna.status.success(),
        "la consegna che alimenta la prova"
    );

    for (formato, nome) in [("csv", "uscita.csv"), ("geojson", "uscita.geojson")] {
        let destinazione = temporanea.path().join(nome);
        let uscita = Command::new(BINARIO)
            .args([
                "write",
                arrow.to_str().unwrap(),
                destinazione.to_str().unwrap(),
                "--to",
                formato,
                "--format",
                "json",
            ])
            .output()
            .expect("il binario parte");
        let stdout = String::from_utf8(uscita.stdout).expect("stdout e' UTF-8");
        let busta: Value = serde_json::from_str(&stdout)
            .unwrap_or_else(|e| panic!("{formato}: stdout e' JSON: {e} -- {stdout}"));
        assert_eq!(busta["status"], "ok", "{formato}: {stdout}");

        // Il nome del contratto e' quello del catalogo, non `-v2`: `io.write`
        // nasce con i suoi schemi pubblicati.
        assert_eq!(
            busta["contract"], "plenora-io-write-result-v1",
            "{formato}: la busta annuncia il contratto del catalogo"
        );

        let risultato = &busta["result"];
        assert!(
            validatore.is_valid(risultato),
            "{formato}: il risultato non valida: {:?}",
            validatore
                .iter_errors(risultato)
                .map(|e| format!("{} in {}", e, e.instance_path()))
                .collect::<Vec<_>>()
        );
    }
}
