//! Un consumatore esterno della superficie Rust pubblica.
//!
//! # Perché sta fuori dal workspace
//!
//! Perché un test interno non prova che l'API sia usabile da fuori.
//! `SURFACE-BINDINGS-1.0 §2` chiede di verificare la mappatura «through a
//! consumer crate that imports only those documented exports», e un crate
//! dentro `crates/` sarebbe compilato insieme al resto: vedrebbe gli `pub(crate)`,
//! erediterebbe i lint del workspace, e passerebbe anche su una superficie che
//! nessun estraneo può raggiungere.
//!
//! Questo crate viene compilato **dall'archivio sorgente distribuito**, non
//! dall'albero di lavoro: `scripts/check_superficie_rust.py` estrae l'archivio
//! in una directory temporanea, mette questo file accanto e compila. Se un
//! export documentato diventasse privato, o cambiasse firma, o l'archivio non
//! lo contenesse, la compilazione fallirebbe — ed è l'unico modo in cui una
//! promessa su un'API si può provare.
//!
//! # Che cosa importa, e che cosa no
//!
//! Solo i nomi elencati in `contracts/superficie-rust.json`. Non
//! `plenora_io_core`, non i `driver_*`, non `plenora_io_cli::busta`: quelli
//! sono il motore, e un consumatore che li importasse dipenderebbe
//! dall'implementazione invece che dalle operazioni. Il gate confronta questo
//! elenco con la mappatura nei due versi, così che un export documentato e mai
//! importato si veda come uno importato e mai documentato.

use plenora_io_cli::operazioni::{self, Esito, Richiesta};

/// Ogni operazione della mappatura, invocata almeno una volta.
///
/// Non verifica che i risultati siano giusti — le prove sul comportamento
/// stanno nel repository, con le fixture. Verifica che i nomi esistano, che le
/// firme siano quelle documentate e che l'ingresso si possa costruire dall'esterno
/// senza conoscere la struttura degli argomenti della CLI.
fn main() {
    // `io.catalog` non prende ingresso: il suo contratto d'ingresso è vuoto.
    let catalogo = operazioni::catalog();
    assert!(
        catalogo.get("drivers").is_some(),
        "il catalogo rende i driver dell'artefatto"
    );

    let capacita = operazioni::capabilities();
    assert!(
        capacita.get("operations").is_some(),
        "il documento capability elenca le operazioni"
    );

    // Le cinque che hanno un ingresso. Il percorso non esiste, e va bene: qui
    // si prova la **superficie**, non il comportamento. Ciò che deve reggere è
    // che l'errore arrivi come documento del contratto comune invece che come
    // panico o come codice d'uscita.
    let inesistente = "/nessun-file-per-il-consumatore-esterno.geojson";

    for (nome, esito) in [
        (
            "io.inspect",
            operazioni::inspect(Richiesta::sulla_sorgente(inesistente)),
        ),
        (
            "io.layers",
            operazioni::layers(Richiesta::sulla_sorgente(inesistente)),
        ),
        (
            "io.read",
            operazioni::read(Richiesta::sulla_sorgente(inesistente)),
        ),
        (
            "io.write",
            operazioni::write(
                Richiesta::sulla_sorgente(inesistente)
                    .con_destinazione("/nessuna-destinazione.csv")
                    .con_formato_destinazione("csv"),
            ),
        ),
        (
            "io.convert",
            operazioni::convert(
                Richiesta::sulla_sorgente(inesistente)
                    .con_destinazione("/nessuna-destinazione.csv")
                    .con_formato_sorgente("geojson")
                    .con_formato_destinazione("csv"),
            ),
        ),
    ] {
        verifica_gli_assi(nome, &esito);
    }

    println!("consumatore esterno: sette export documentati, tutti raggiungibili");
}

/// L'errore arriva come documento del contratto comune, con i quattro assi.
///
/// È la parte di `SURF-017` che un consumatore può verificare da solo: le due
/// superfici devono rendere gli stessi assi d'errore, e da qui si vede che la
/// libreria li rende senza passare per un codice d'uscita — che è del processo,
/// non dell'operazione.
fn verifica_gli_assi(operazione: &str, esito: &Esito) {
    let Err(documento) = esito else {
        panic!("{operazione}: un percorso inesistente non può riuscire");
    };
    let errore = documento
        .get("error")
        .unwrap_or_else(|| panic!("{operazione}: la busta d'errore non ha `error`"));
    for asse in ["category", "phase", "remote_effect", "retry"] {
        assert!(
            errore.get(asse).is_some(),
            "{operazione}: manca l'asse `{asse}`"
        );
    }
}
