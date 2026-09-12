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
    assert!(esempi.len() >= 44, "il manifesto non e' stato svuotato");

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

/// Ogni busta che il binario emette valida contro lo schema della sua
/// operazione.
///
/// # Perche' tutte e sei, e non una per volta
///
/// Il profilo pretende schemi immutabili per le sei coppie prima che un
/// artefatto reclami il profilo. Averli scritti non basta: uno schema che
/// nessuna busta attraversa e' una dichiarazione che nessuno prova, e la prima
/// divergenza la troverebbe un consumatore invece di noi.
///
/// Le invocazioni non sono scelte per far passare la sonda. Sono le forme che
/// il prodotto ha davvero -- `inspect` su un formato a layer unico e su uno
/// multi-layer, `read` con e senza consegna, `write` verso due sink diversi,
/// `convert` verso un formato che perde e verso uno che non perde -- perche' un
/// campo dichiarato su un solo valore osservato non e' dichiarato.
#[test]
// La tabella dei casi e' lunga per costruzione: undici invocazioni reali, ognuna
// con i suoi argomenti. Spezzarla in due funzioni separerebbe i casi dal
// confronto che li rende una prova sola.
#[allow(clippy::too_many_lines)]
fn ogni_busta_reale_valida_contro_lo_schema_della_sua_operazione() {
    let temporanea = tempfile::tempdir().expect("directory temporanea");
    let dove = temporanea.path();
    let arrow = dove.join("ponte.arrow");

    // La consegna che alimenta le due invocazioni di `write`.
    let preparazione = Command::new(BINARIO)
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
        preparazione.status.success(),
        "la consegna che alimenta la prova"
    );

    let casi: Vec<(&str, Vec<String>)> = vec![
        ("plenora-io-catalog-v1", vec!["catalog".to_owned()]),
        (
            "plenora-io-inspect-v1",
            vec![
                "inspect".to_owned(),
                fixture("canonico.geojson").display().to_string(),
            ],
        ),
        (
            "plenora-io-inspect-v1",
            vec![
                "inspect".to_owned(),
                fixture("canonico.gpkg").display().to_string(),
            ],
        ),
        (
            "plenora-io-layers-v1",
            vec![
                "layers".to_owned(),
                fixture("canonico.gpkg").display().to_string(),
            ],
        ),
        (
            "plenora-io-layers-v1",
            vec![
                "layers".to_owned(),
                fixture("canonico.geojson").display().to_string(),
            ],
        ),
        (
            "plenora-io-read-result-v1",
            vec![
                "read".to_owned(),
                fixture("canonico.geojson").display().to_string(),
            ],
        ),
        (
            "plenora-io-read-result-v1",
            vec![
                "read".to_owned(),
                fixture("canonico.geojson").display().to_string(),
                "--output".to_owned(),
                dove.join("consegnata.arrow").display().to_string(),
            ],
        ),
        (
            "plenora-io-write-result-v1",
            vec![
                "write".to_owned(),
                arrow.display().to_string(),
                dove.join("pubblicato.csv").display().to_string(),
                "--to".to_owned(),
                "csv".to_owned(),
            ],
        ),
        (
            "plenora-io-write-result-v1",
            vec![
                "write".to_owned(),
                arrow.display().to_string(),
                dove.join("pubblicato.geojson").display().to_string(),
                "--to".to_owned(),
                "geojson".to_owned(),
            ],
        ),
        (
            "plenora-io-convert-v1",
            vec![
                "convert".to_owned(),
                fixture("canonico.geojson").display().to_string(),
                dove.join("convertito.csv").display().to_string(),
                "--from".to_owned(),
                "geojson".to_owned(),
                "--to".to_owned(),
                "csv".to_owned(),
            ],
        ),
        (
            "plenora-io-convert-v1",
            vec![
                "convert".to_owned(),
                fixture("canonico.geojson").display().to_string(),
                dove.join("convertito.geojson").display().to_string(),
                "--from".to_owned(),
                "geojson".to_owned(),
                "--to".to_owned(),
                "geojson".to_owned(),
            ],
        ),
    ];

    let mut coperti = std::collections::BTreeSet::new();
    for (schema, argomenti) in &casi {
        let validatore = compila(schema);
        let uscita = Command::new(BINARIO)
            .args(argomenti)
            .arg("--format")
            .arg("json")
            .output()
            .expect("il binario parte");
        let stdout = String::from_utf8(uscita.stdout).expect("stdout e' UTF-8");
        let busta: Value = serde_json::from_str(&stdout)
            .unwrap_or_else(|e| panic!("{argomenti:?}: stdout e' JSON: {e} -- {stdout}"));
        assert_eq!(busta["status"], "ok", "{argomenti:?}: {stdout}");

        // Il nome annunciato e' quello dello schema contro cui si valida: se
        // divergessero, la sonda validerebbe contro il documento sbagliato e
        // passerebbe per la ragione sbagliata.
        assert_eq!(
            busta["contract"], *schema,
            "{argomenti:?}: la busta annuncia un contratto diverso da quello atteso"
        );

        let risultato = &busta["result"];
        assert!(
            validatore.is_valid(risultato),
            "{argomenti:?} contro {schema}: {:?}",
            validatore
                .iter_errors(risultato)
                .map(|e| format!("{} in {}", e, e.instance_path()))
                .collect::<Vec<_>>()
        );
        coperti.insert((*schema).to_owned());
    }

    assert_eq!(
        coperti.len(),
        6,
        "le sei uscite del catalogo sono coperte tutte: {coperti:?}"
    );
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
        provati >= 11,
        "il manifesto deve legare al binario i rifiuti noti di tutte e sei le          operazioni, legati: {provati}"
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
