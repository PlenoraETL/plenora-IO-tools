//! Dove le librerie native trovano i propri dati, quando il binario gira dentro
//! un artefatto.
//!
//! # Il problema
//!
//! Le librerie che l'artefatto porta con se' hanno cotti dentro i percorsi del
//! prefisso in cui sono state **costruite**: `share/gdal` per i dati di GDAL,
//! `share/proj` per la griglia di PROJ, `lib/gdalplugins` per i plugin. Sulla
//! macchina di chi installa quei percorsi non esistono, e il degrado non
//! somiglia a un difetto d'installazione: somiglia a un difetto del dato -- un
//! CRS che non si risolve, un driver che non si registra.
//!
//! # Perche' il piano sta qui e l'applicazione altrove
//!
//! Perche' applicarlo **una volta sola** non basta, e le due vie non sono
//! equivalenti.
//!
//! Su Linux impostare l'ambiente del processo funziona: `setenv` aggiorna
//! `environ`, ed e' esattamente cio' che `getenv` legge -- quindi anche le
//! librerie native lo vedono.
//!
//! Su Windows no. `std::env::set_var` chiama `SetEnvironmentVariableW`, che
//! aggiorna il blocco d'ambiente **del processo**; il runtime C mantiene una
//! propria copia, e `getenv` legge quella. GDAL e PROJ chiamano `getenv`: la
//! variabile impostata da Rust non la vedono affatto.
//!
//! Il difetto e' stato trovato dal relocation smoke su Windows, e non altrove:
//! con l'ambiente del runner ancora intatto i dati si trovavano lo stesso, e
//! nulla lo mostrava. Con le sentinelle al posto delle radici, l'artefatto
//! falliva alla prima conversione con un CRS.
//!
//! La cura e' applicare il piano **anche** dentro GDAL. Non pero' con un
//! meccanismo solo: le config option bastano per cio' che legge GDAL, e non
//! bastano per PROJ.
//!
//! # Che cosa e' stato misurato, invece che supposto
//!
//! Con il default nascosto e l'ambiente avvelenato, su GDAL 3.6 in container:
//!
//! - `GDAL_DATA` come config option: **funziona**, purche' impostata prima del
//!   primo `CPLFindFile` -- GDAL inizializza il proprio finder una volta sola.
//! - `PROJ_DATA` come config option: **non funziona**. Nemmeno `PROJ_LIB`.
//!   GDAL non la inoltra a PROJ, e la ricerca resta quella cotta dentro PROJ.
//! - `OSRSetPROJSearchPaths`: **funziona**. E' l'unica via, ed e' per questo
//!   che il fork governato la espone.
//!
//! La prima stesura di questa cura applicava solo le config option, e
//! sembrava ragionevole. Il relocation smoke Windows e' rimasto rosso con lo
//! stesso identico messaggio, ed e' cosi' che si e' scoperto che GDAL e PROJ
//! non condividono quella tabella.
//!
//! # `XML_CATALOG_FILES` su Windows resta scoperta
//!
//! libxml2 legge `getenv` e non ha ne' config option ne' un'API di percorso.
//! Su Windows la variabile impostata da Rust non la vede, e il catalogo resta
//! quello cotto nel prefisso di costruzione. Quel percorso su una macchina che
//! installa non esiste, quindi l'entita' esterna **non** si risolve lo stesso:
//! l'esito voluto arriva per assenza invece che per configurazione. E' piu'
//! debole di come suonerebbe dire che la variabile e' impostata, e vale la pena
//! scriverlo invece di lasciarlo credere.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// Una variabile e la directory che le corrisponde dentro l'artefatto.
pub struct Radice {
    /// Il nome della variabile d'ambiente, che e' anche quello della config
    /// option di GDAL: i due vocabolari coincidono per queste tre.
    pub variabile: &'static str,
    /// Il percorso, relativo alla radice dell'artefatto.
    pub relativo: &'static str,
}

/// Il piano, in una tabella sola.
///
/// Ciascuna di queste variabili e' cio' che rende ammissibile un percorso
/// assoluto cotto nei binari spediti, e nei lock e' scritta come tale. Le due
/// tabelle vanno lette insieme, e una sonda diventa rossa se divergono: se una
/// riga sparisse di qui, la classificazione di la' resterebbe verde affermando
/// una copertura che non c'e' piu'.
pub const RADICI: &[Radice] = &[
    Radice {
        variabile: "GDAL_DATA",
        relativo: "share/gdal",
    },
    Radice {
        variabile: "PROJ_DATA",
        relativo: "share/proj",
    },
    // `PROJ_LIB` e' il nome storico della stessa cosa: PROJ lo legge fino alla
    // 9.0 e `PROJ_DATA` dalla 9.1. Impostare entrambe non e' incertezza -- e'
    // che il nome e' cambiato -- e costa una riga.
    Radice {
        variabile: "PROJ_LIB",
        relativo: "share/proj",
    },
    Radice {
        variabile: "GDAL_DRIVER_PATH",
        relativo: "lib/gdalplugins",
    },
];

/// La riga che non passa dalle config option.
///
/// PROJ non legge la tabella di GDAL: le sue risorse si indirizzano con
/// `OSRSetPROJSearchPaths` e con nient'altro. Il nome sta qui, accanto alla
/// tabella, perche' chi applica il piano deve poter riconoscere **questa**
/// riga e trattarla a parte -- e perche' se la riga sparisse dalla tabella,
/// questa costante resterebbe a indicare qualcosa che non c'e' piu', e la
/// sonda che le confronta diventerebbe rossa.
pub const RADICE_DI_PROJ: &str = "PROJ_DATA";

/// La directory dei dati di PROJ dentro l'artefatto, se spedita.
///
/// Separata dal resto del piano perche' il suo destinatario e' un'altra API.
#[must_use]
pub fn proj_del_processo() -> Option<OsString> {
    piano_del_processo()
        .into_iter()
        .find(|(variabile, _)| *variabile == RADICE_DI_PROJ)
        .map(|(_, valore)| valore)
}

/// La variabile che si svuota invece di puntarla da qualche parte.
///
/// libxml2 consulta un catalogo per risolvere DTD ed entita' esterne, e il
/// default cotto dentro e' `/etc/xml/catalog` sotto il prefisso di costruzione.
/// A differenza delle altre questa e' raggiungibile: un GML o un KML in ingresso
/// con un `DOCTYPE` basta. Il vuoto e' la configurazione **voluta**, non un
/// ripiego -- l'artefatto non deve risolvere entita' esterne.
pub const CATALOGO_XML: &str = "XML_CATALOG_FILES";

/// La radice dell'artefatto, se il binario ne fa parte.
///
/// Non basta che esista una directory con il nome giusto: si pretende il layout
/// dichiarato -- il binario in `bin/`, e accanto `share/gdal` e `share/proj`.
///
/// La directory delle librerie **non** entra nel criterio: la mette ciascuna
/// piattaforma dove il proprio caricatore guarda -- `lib/` su Linux, dentro
/// `bin/` su Windows. Pretenderla era un criterio scritto guardando una
/// piattaforma sola, e rendeva irriconoscibile il layout dell'altra.
#[must_use]
pub fn radice_da(binario: &Path) -> Option<PathBuf> {
    let radice = binario.parent()?.parent()?;
    let completo =
        radice.join("share").join("gdal").is_dir() && radice.join("share").join("proj").is_dir();
    completo.then(|| radice.to_path_buf())
}

/// Le variabili da impostare, data la radice.
///
/// Separata dall'applicazione perche' cosi' e' verificabile senza toccare
/// l'ambiente del processo -- che e' globale, e in un test lo condividerebbe con
/// tutti gli altri.
#[must_use]
pub fn piano(radice: &Path) -> Vec<(&'static str, OsString)> {
    let mut piano: Vec<(&'static str, OsString)> = Vec::new();
    for r in RADICI {
        // Una variabile che punta a una directory non spedita sarebbe peggio del
        // default: il default almeno si riconosce come tale, mentre una nostra
        // variabile rotta si legge come una configurazione voluta.
        let assoluto = radice.join(r.relativo);
        if !assoluto.is_dir() {
            continue;
        }
        piano.push((r.variabile, assoluto.into_os_string()));
    }
    piano.push((CATALOGO_XML, OsString::new()));
    piano
}

/// Il piano per il binario in esecuzione, se sta dentro un artefatto.
///
/// Vuoto quando non ci sta: fuori da un artefatto -- un `cargo run` nell'albero
/// di sviluppo -- non c'e' niente da imporre, e l'ambiente resta l'unica fonte.
#[must_use]
pub fn piano_del_processo() -> Vec<(&'static str, OsString)> {
    let Ok(binario) = std::env::current_exe() else {
        // Senza sapere dove siamo non si puo' dedurre niente, e dedurre male
        // sarebbe peggio che lasciar stare.
        return Vec::new();
    };
    // Un'uscita anticipata, come la riga sopra, e non un `unwrap_or_default`
    // ne' un `map_or_else`. Il censimento dei fallback conta ogni `unwrap_or*`
    // perche' ciascuno e' un modo di non accorgersi di qualcosa, e qui non c'e'
    // niente da non accorgersi: fuori da un artefatto il vuoto e' la risposta
    // giusta, non un ripiego. Scriverla cosi' la dice, e lascia le due strade
    // alla stessa altezza dell'altra uscita di questa funzione.
    let Some(radice) = radice_da(&binario) else {
        return Vec::new();
    };
    piano(&radice)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

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

    /// Ogni variabile della tabella punta dentro l'artefatto, e il catalogo XML
    /// e' vuoto.
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

    /// I due nomi di PROJ puntano alla stessa directory.
    ///
    /// Non sono due cose: sono lo stesso dato con due nomi, perche' PROJ ha
    /// cambiato quello che legge fra la 9.0 e la 9.1.
    #[test]
    fn i_due_nomi_di_proj_puntano_allo_stesso_posto() {
        let dir = tempfile::tempdir().unwrap();
        artefatto_completo(dir.path());
        let p = mappa(piano(dir.path()));
        assert_eq!(p.get("PROJ_DATA"), p.get("PROJ_LIB"));
    }

    /// Una variabile la cui directory non e' spedita non si imposta.
    #[test]
    fn cio_che_non_e_spedito_non_si_dichiara() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("share").join("gdal")).unwrap();
        let p = mappa(piano(dir.path()));

        assert!(p.contains_key("GDAL_DATA"));
        assert!(!p.contains_key("PROJ_DATA"), "share/proj non e' spedito");
        assert!(
            !p.contains_key("GDAL_DRIVER_PATH"),
            "lib/gdalplugins non e' spedito"
        );
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

    /// Il layout di Windows: le DLL stanno in `bin/`, e `lib/` non c'e'.
    ///
    /// E' il difetto che il relocation smoke ha trovato. Il criterio pretendeva
    /// `lib/`, l'artefatto Windows non l'aveva, e il binario non riconosceva il
    /// proprio layout.
    #[test]
    fn il_layout_di_windows_e_riconosciuto() {
        let dir = tempfile::tempdir().unwrap();
        let radice = dir.path();
        for percorso in ["bin", "share/gdal", "share/proj"] {
            std::fs::create_dir_all(radice.join(percorso)).unwrap();
        }
        std::fs::write(radice.join("bin").join("gdal.dll"), b"").unwrap();
        let binario = radice.join("bin").join("plenora-io.exe");
        std::fs::write(&binario, b"").unwrap();

        assert_eq!(
            radice_da(&binario).as_deref(),
            Some(radice),
            "senza `lib/` il layout resta quello di un artefatto"
        );
    }

    /// Un albero a meta' non e' un artefatto.
    #[test]
    fn un_layout_incompleto_non_e_un_artefatto() {
        for mancante in ["share/gdal", "share/proj"] {
            let dir = tempfile::tempdir().unwrap();
            for percorso in ["bin", "share/gdal", "share/proj"] {
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

    /// L'albero di sviluppo non e' un artefatto.
    #[test]
    fn un_binario_fuori_da_un_artefatto_non_da_radici() {
        let dir = tempfile::tempdir().unwrap();
        let binario = dir.path().join("target").join("debug").join("plenora-io");
        std::fs::create_dir_all(binario.parent().unwrap()).unwrap();
        std::fs::write(&binario, b"").unwrap();
        assert!(radice_da(&binario).is_none());
    }

    /// `RADICE_DI_PROJ` nomina una riga che nella tabella c'e' davvero.
    ///
    /// E' la costante che chi applica il piano tratta a parte, perche' PROJ non
    /// legge le config option di GDAL. Se la riga sparisse, l'applicazione
    /// smetterebbe di indirizzare PROJ **in silenzio**: continuerebbe a
    /// impostare tutto il resto, e il difetto tornerebbe a somigliare a un dato
    /// rotto invece che a una configurazione mancante.
    #[test]
    fn la_radice_di_proj_e_una_riga_della_tabella() {
        let dir = tempfile::tempdir().unwrap();
        artefatto_completo(dir.path());
        let p = mappa(piano(dir.path()));
        let valore = p
            .get(RADICE_DI_PROJ)
            .expect("`RADICE_DI_PROJ` non nomina nessuna riga della tabella");
        assert!(Path::new(valore).ends_with("proj"));
    }

    /// Un percorso senza nonno non fa panicare niente.
    #[test]
    fn un_percorso_troppo_corto_non_da_radici() {
        assert!(radice_da(Path::new("plenora-io")).is_none());
    }
}
