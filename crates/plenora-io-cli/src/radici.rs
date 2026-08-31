//! Dove le librerie spedite trovano i propri dati, quando il binario gira
//! dentro un artefatto.
//!
//! # Il problema
//!
//! Le librerie native che l'artefatto porta con se' hanno cotti dentro i
//! percorsi del prefisso in cui sono state **costruite**: `share/gdal` per i
//! dati di GDAL, `share/proj` per la griglia di PROJ, `lib/gdalplugins` per i
//! plugin. Sulla macchina di chi installa quei percorsi non esistono, e il
//! degrado non somiglia a un difetto d'installazione: somiglia a un difetto del
//! dato -- un CRS che non si risolve, un driver che non si registra.
//!
//! L'`RPATH` non copre niente di tutto questo. L'`RPATH` governa la
//! risoluzione delle **librerie**; dati e plugin li cerca ciascuna libreria per
//! conto proprio, con i propri default compilati.
//!
//! # La scelta
//!
//! I percorsi si derivano dal **binario in esecuzione**, non dall'ambiente. Un
//! artefatto che pretendesse variabili impostate a mano funzionerebbe sulla
//! macchina di chi lo ha costruito e altrove no, e chi lo installa non ha modo
//! di sapere che gli servono.
//!
//! Le variabili si impostano nell'ambiente del processo, e si impostano **in
//! cima a `main`**: le librerie native le leggono pigramente, alla prima
//! richiesta, e a quel punto devono gia' esserci. E' anche l'unico momento in
//! cui questo processo ha con certezza un thread solo.
//!
//! # Perche' vincono su cio' che l'utente ha impostato
//!
//! Perche' i dati che servono sono **quelli spediti**: sono della stessa
//! versione delle librerie spedite, e una `share/gdal` presa da un'altra
//! installazione e' la strada per un difetto che nessuno riproduce. Per
//! `GDAL_DRIVER_PATH` c'e' in piu' una ragione di postura: fissarlo alla
//! directory dei plugin spediti e' cio' che tiene fuori dal processo un plugin
//! di sistema.
//!
//! # Perche' sono soltanto tre
//!
//! I binari spediti portano cotti anche i percorsi dei certificati TLS di
//! OpenSSL, dei suoi moduli caricabili, del terminfo di ncurses e di Kerberos.
//! Nessuno di questi e' qui, e non per dimenticanza: sono classificati nel lock
//! come **non raggiungibili dall'uso**, ciascuno legato a una guardia. La CLI
//! apre soltanto sorgenti locali e non autentica nulla -- il che tiene fuori
//! TLS, i provider di OpenSSL e GSSAPI -- e non ha interfaccia interattiva, il
//! che tiene fuori ncurses.
//!
//! Dichiararli invece «coperti da una variabile» sarebbe stato promettere di
//! spedire un bundle di certificati e un database terminfo interi per librerie
//! che non chiamiamo mai. La guardia e' una garanzia piu' forte della
//! variabile, e costa meno: la variabile copre il percorso, la guardia dice che
//! nessuno ci arriva.
//!
//! # Fuori da un artefatto
//!
//! Se il layout non c'e' -- un `cargo run` nell'albero di sviluppo -- questo
//! modulo non fa **niente**, e l'ambiente resta l'unica fonte. Il
//! riconoscimento vuole il layout completo: una forma parziale non e' un
//! artefatto, e pescare dati da un albero a meta' e' peggio che non pescarne.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// Una variabile e la directory che le corrisponde dentro l'artefatto.
struct Radice {
    variabile: &'static str,
    relativo: &'static str,
}

/// Il piano, in una tabella sola.
///
/// Ciascuna di queste variabili e' cio' che rende ammissibile un percorso
/// assoluto cotto nei binari spediti, e in `scripts/linux-gdal-lock.json` e'
/// scritta come tale. Le due tabelle vanno lette insieme, e una sonda di
/// `scripts/test_distribuzione_matrice.py` diventa rossa se divergono: se una
/// riga sparisse di qui, la classificazione di la' resterebbe verde affermando
/// una copertura che non c'e' piu'.
const RADICI: &[Radice] = &[
    Radice {
        variabile: "GDAL_DATA",
        relativo: "share/gdal",
    },
    Radice {
        variabile: "PROJ_DATA",
        relativo: "share/proj",
    },
    Radice {
        variabile: "GDAL_DRIVER_PATH",
        relativo: "lib/gdalplugins",
    },
];

/// La variabile che si svuota invece di puntarla da qualche parte.
///
/// libxml2 consulta un catalogo per risolvere DTD ed entita' esterne, e il
/// default cotto dentro e' `/etc/xml/catalog` sotto il prefisso di costruzione.
/// A differenza delle altre questa e' raggiungibile: un GML o un KML in
/// ingresso con un `DOCTYPE` basta. Il vuoto e' la configurazione **voluta**,
/// non un ripiego -- l'artefatto non deve risolvere entita' esterne -- e non
/// costa nulla da spedire, perche' non c'e' niente da spedire.
const CATALOGO_XML: &str = "XML_CATALOG_FILES";

/// La radice dell'artefatto, se il binario ne fa parte.
///
/// Non basta che esista una directory con il nome giusto: si pretende il layout
/// dichiarato -- il binario in `bin/`, e accanto `lib/`, `share/gdal`,
/// `share/proj`.
fn radice_da(binario: &Path) -> Option<PathBuf> {
    let radice = binario.parent()?.parent()?;
    let completo = radice.join("lib").is_dir()
        && radice.join("share").join("gdal").is_dir()
        && radice.join("share").join("proj").is_dir();
    completo.then(|| radice.to_path_buf())
}

/// Le variabili da impostare, data la radice.
///
/// Separata dall'applicazione perche' cosi' e' verificabile senza toccare
/// l'ambiente del processo -- che e' globale, e in un test lo condividerebbe
/// con tutti gli altri.
fn piano(radice: &Path) -> Vec<(&'static str, OsString)> {
    let mut piano: Vec<(&'static str, OsString)> = Vec::new();
    for r in RADICI {
        // Una variabile che punta a una directory non spedita sarebbe peggio
        // del default: il default almeno si riconosce come tale, mentre una
        // nostra variabile rotta si legge come una configurazione voluta. Che
        // l'artefatto spedisca cio' che questa tabella nomina lo pretende il
        // gate di distribuzione; qui ci si limita a non affermarlo falso.
        let assoluto = radice.join(r.relativo);
        if !assoluto.is_dir() {
            continue;
        }
        piano.push((r.variabile, assoluto.into_os_string()));
    }
    piano.push((CATALOGO_XML, OsString::new()));
    piano
}

/// Impone all'ambiente le radici dell'artefatto. Da chiamare per prima cosa in
/// `main`, prima che esista un secondo thread.
pub fn radici_dell_artefatto() {
    let Ok(binario) = std::env::current_exe() else {
        // Senza sapere dove siamo non si puo' dedurre niente, e dedurre male
        // sarebbe peggio che lasciar stare. Non e' un errore da riportare:
        // fuori da un artefatto e' anche il caso normale.
        return;
    };
    let Some(radice) = radice_da(&binario) else {
        return;
    };
    for (variabile, valore) in piano(&radice) {
        std::env::set_var(variabile, valore);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    /// Costruisce un artefatto finto con tutte le voci della tabella.
    fn artefatto_completo(radice: &Path) {
        for r in RADICI {
            std::fs::create_dir_all(radice.join(r.relativo)).unwrap();
        }
        std::fs::create_dir_all(radice.join("bin")).unwrap();
    }

    fn mappa(piano: Vec<(&'static str, OsString)>) -> BTreeMap<String, String> {
        piano
            .into_iter()
            .map(|(k, v)| (k.to_owned(), v.to_string_lossy().into_owned()))
            .collect()
    }

    /// Ogni variabile della tabella punta dentro l'artefatto -- e il catalogo
    /// XML e' vuoto.
    #[test]
    fn ogni_radice_punta_dentro_l_artefatto() {
        let dir = tempfile::tempdir().unwrap();
        artefatto_completo(dir.path());
        let p = mappa(piano(dir.path()));

        assert_eq!(p.len(), RADICI.len() + 1, "manca una riga della tabella");
        for r in RADICI {
            let valore = p.get(r.variabile).expect(r.variabile);
            assert!(
                Path::new(valore).starts_with(dir.path()),
                "«{}» punta fuori dall'artefatto: {valore}",
                r.variabile
            );
        }
        assert_eq!(p.get(CATALOGO_XML).map(String::as_str), Some(""));
    }

    /// Una variabile la cui directory non e' spedita non si imposta.
    ///
    /// Puntare a un percorso inesistente sarebbe peggio del default: il default
    /// si riconosce come tale, una nostra variabile rotta si legge come una
    /// configurazione voluta.
    #[test]
    fn cio_che_non_e_spedito_non_si_dichiara() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("share").join("gdal")).unwrap();
        let p = mappa(piano(dir.path()));

        assert_eq!(
            p.get("GDAL_DATA").map(String::as_str),
            Some(
                dir.path()
                    .join("share")
                    .join("gdal")
                    .to_string_lossy()
                    .as_ref()
            )
        );
        assert!(!p.contains_key("PROJ_DATA"), "share/proj non e' spedito");
        assert!(
            !p.contains_key("GDAL_DRIVER_PATH"),
            "lib/gdalplugins non e' spedito"
        );
        // Il catalogo si svuota comunque: non ha una directory da spedire, e
        // non risolvere entita' esterne e' una postura che vale sempre.
        assert_eq!(p.get(CATALOGO_XML).map(String::as_str), Some(""));
    }

    /// Il layout completo e' riconosciuto.
    #[test]
    fn un_artefatto_completo_e_riconosciuto() {
        let dir = tempfile::tempdir().unwrap();
        artefatto_completo(dir.path());
        let binario = dir.path().join("bin").join("plenora-io");
        std::fs::write(&binario, b"").unwrap();
        assert_eq!(radice_da(&binario).as_deref(), Some(dir.path()));
    }

    /// Un albero a meta' non e' un artefatto.
    ///
    /// Senza questa pretesa GDAL troverebbe una `share/gdal` e non una
    /// `share/proj`, e il difetto si manifesterebbe sulle sole trasformazioni
    /// di CRS -- lontano dalla causa.
    #[test]
    fn un_layout_incompleto_non_e_un_artefatto() {
        for mancante in ["lib", "share/gdal", "share/proj"] {
            let dir = tempfile::tempdir().unwrap();
            for percorso in ["bin", "lib", "share/gdal", "share/proj"] {
                if percorso != mancante {
                    std::fs::create_dir_all(dir.path().join(percorso)).unwrap();
                }
            }
            let binario = dir.path().join("bin").join("plenora-io");
            std::fs::write(&binario, b"").unwrap();
            assert!(
                radice_da(&binario).is_none(),
                "senza «{mancante}» non e' un artefatto"
            );
        }
    }

    /// L'albero di sviluppo: `target/debug/plenora-io` ha un nonno, ma non ha
    /// il layout. Il modulo tace, e l'ambiente resta l'unica fonte.
    #[test]
    fn un_binario_fuori_da_un_artefatto_non_da_radici() {
        let dir = tempfile::tempdir().unwrap();
        let binario = dir.path().join("target").join("debug").join("plenora-io");
        std::fs::create_dir_all(binario.parent().unwrap()).unwrap();
        std::fs::write(&binario, b"").unwrap();
        assert!(radice_da(&binario).is_none());
    }

    /// Un percorso senza nonno non fa panicare niente.
    #[test]
    fn un_percorso_troppo_corto_non_da_radici() {
        assert!(radice_da(Path::new("plenora-io")).is_none());
    }
}
