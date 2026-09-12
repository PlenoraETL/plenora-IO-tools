//! Che cosa il driver `FileGDB` **dichiara** sul percorso di destinazione.
//!
//! # Perché una prova d'integrazione e non una unità
//!
//! Due ragioni, e la seconda non è di stile. La prima: qui si guarda la
//! superficie pubblica — il descrittore che `io.catalog` pubblica — e una prova
//! d'integrazione la raggiunge come la raggiunge chi ci consuma, importando il
//! crate invece di stare dentro di esso.
//!
//! La seconda: `crates/driver-filegdb/src` sta nel perimetro della misura di
//! profondità del fuzzing, e `tests/` no. Una riga aggiunta al sorgente
//! invaliderebbe quella misura e quella del confine `ASan`, che condivide la
//! stessa impronta — due corse per una sonda che non tocca il codice che il
//! fuzzer attraversa. Non è aggirare il gate: è non chiedergli di rispondere a
//! una domanda che non gli è stata posta.

use plenora_io_core::{FormatDriver, SinkPathConstraint};

/// Il percorso di destinazione è del chiamante, e il suffisso governa il
/// **riconoscimento**.
///
/// # Che cosa ha deciso questa dichiarazione
///
/// D10 ha separato i vincoli che il formato impone davvero da quelli che erano
/// abitudine. `FileGDB` era il caso che non si poteva decidere a tavolino: è un
/// dataset a **directory**, il suo staging si chiama `.gdb`, e la tentazione
/// era di dichiararlo vincolato per simmetria con `GeoPackage` e `Shapefile`.
/// Sarebbe stato inventare un requisito.
///
/// La misura, con `gdal-backend` acceso:
///
/// ```text
/// write g.arrow senza_suffisso --to filegdb   -> ok, 2 righe
///   inspect senza_suffisso                    -> estensione non riconosciuta
/// write g.arrow con.gdb        --to filegdb   -> ok, 2 righe
///   inspect con.gdb                           -> ok
/// ```
///
/// Scrivere su un nome qualunque **riesce** e produce un dataset valido; è
/// rileggerlo per deduzione che non si può. È il comportamento dei sette driver
/// liberi, non quello dei due vincolati — dove un nome sbagliato produrrebbe un
/// artefatto fuori specifica (`GeoPackage`) o lascerebbe i companion senza un
/// posto da cui prendere il nome (`Shapefile`).
///
/// La sonda gira in entrambe le build, perché il descrittore è statico: ciò che
/// la feature cambia è se il driver sappia lavorare, non che cosa dichiari.
#[test]
fn il_percorso_e_libero_e_il_suffisso_serve_al_riconoscimento() {
    let descrittore = driver_filegdb::FileGdbDriver.descriptor();

    assert_eq!(
        descrittore.recognised_suffixes(),
        &["gdb"],
        "il suffisso con cui un percorso viene riconosciuto come FileGDB"
    );
    assert_eq!(
        descrittore.sink_path(),
        Some(SinkPathConstraint::Free),
        "la scrittura non pretende il suffisso: è la rilettura per deduzione \
         che lo usa, e quella è un'altra cosa"
    );
}
