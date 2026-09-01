//! Un artefatto rilocato deve poter dire a PROJ dove sono i propri dati.
//!
//! # Che cosa prova
//!
//! Che `OSRSetPROJSearchPaths` decide davvero dove PROJ cerca: con un percorso
//! inesistente il CRS **non** si risolve, e con quelli veri torna a risolversi.
//! E' il meccanismo su cui la distribuzione conta, ed e' l'unico che si comporti
//! allo stesso modo su ogni piattaforma e ogni GDAL che spediamo.
//!
//! # Perche' prova questo e non altro
//!
//! Due formulazioni piu' dirette sono state provate e sono cadute, ciascuna
//! insegnando qualcosa che vale la pena tenere scritto.
//!
//! **Avvelenare l'ambiente e vedere se il CRS si rompe.** Su Linux funziona; su
//! Windows no, e non per un difetto della prova: `std::env::set_var` chiama
//! `SetEnvironmentVariableW`, il runtime C tiene una copia propria e `getenv`
//! legge quella. E' esattamente il difetto che tutto questo lavoro cura, e una
//! prova che ci si appoggia sopra e' rossa sulla piattaforma che le interessa.
//!
//! **Asserire che la config option `PROJ_DATA` non arriva a PROJ.** Su GDAL 3.9
//! di conda non ci arriva -- ed e' il motivo per cui la prima cura non ha
//! chiuso il difetto -- ne' sul GDAL 3.6 del container. Sul runner della CI
//! ordinaria, con un GDAL diverso, ci arriva. E' un comportamento che dipende
//! dalla versione: fissarlo in una prova non misura un invariante nostro, e la
//! prova diventa rossa affermando qualcosa di falso su quella macchina. Resta
//! come ragione storica per cui il codice non ci conta sopra.
//!
//! # Il momento conta, e questa prova ne dipende
//!
//! I percorsi si cambiano finche' PROJ non ha aperto il proprio database:
//! dopo, la connessione resta aperta e spostarli non ha piu' effetto. E' la
//! ragione per cui `radici::applica` sta in `main` e non dentro l'apertura di
//! un dataset, ed e' anche il motivo per cui il primo passo qui sotto e' quello
//! che rompe: da un contesto gia' caldo non si dimostrerebbe niente.
//!
//! # Perche' un binario di test tutto suo
//!
//! I percorsi di ricerca sono stato globale di GDAL. Nella finestra in cui sono
//! rotti, qualunque altra prova che tocchi un CRS fallirebbe. Qui l'unico
//! `#[test]` del file e' questo.

#![cfg(feature = "gdal-backend")]

use gdal::spatial_ref::{get_proj_search_paths, set_proj_search_paths, SpatialRef};

/// I percorsi di ricerca decidono se PROJ trova i propri dati.
#[test]
fn l_api_dei_percorsi_decide_dove_proj_cerca() {
    // I percorsi effettivi **prima** di toccare qualunque cosa: con nulla di
    // configurato GDAL riporta quelli con cui PROJ e' stato costruito, ed e' il
    // modo di sapere dove stanno i dati senza scriverne uno a mano -- che
    // renderebbe la prova valida su una macchina sola.
    let veri = get_proj_search_paths();
    assert!(
        !veri.is_empty(),
        "GDAL non riporta nessun percorso di ricerca per PROJ: senza, questa \
         prova non saprebbe dove sono i dati e non proverebbe niente"
    );

    // Rompere per primo, a freddo. E' la condizione di un artefatto rilocato che
    // non sa dire dove ha messo i propri dati.
    set_proj_search_paths(&["/percorso/che/non/esiste/mai/proj"]).unwrap();
    assert!(
        SpatialRef::from_definition("EPSG:4326").is_err(),
        "con i percorsi di ricerca rotti il CRS si risolve lo stesso: o PROJ \
         aveva gia' aperto il proprio database, e allora questa prova non parte \
         a freddo, oppure `OSRSetPROJSearchPaths` non decide piu' -- e la cura \
         della distribuzione poggia su un meccanismo che non regge"
    );

    // E riportarli: e' cio' che `radici::applica` fa all'avvio quando il binario
    // gira dentro un artefatto.
    let percorsi: Vec<&str> = veri.iter().map(String::as_str).collect();
    set_proj_search_paths(&percorsi).unwrap();
    let srs = SpatialRef::from_definition("EPSG:4326")
        .expect("`OSRSetPROJSearchPaths` non ha riportato PROJ ai propri dati");
    assert_eq!(srs.auth_code().ok(), Some(4326));
}
