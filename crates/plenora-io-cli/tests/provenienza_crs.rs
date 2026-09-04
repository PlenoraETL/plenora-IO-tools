//! Da quale fonte deriva ogni `Derived`: la caratterizzazione dei sei driver.
//!
//! # Il difetto che questa prova precede
//!
//! `CrsRepresentationState::Derived` dice che una rappresentazione del CRS e'
//! **ricavabile**, non da che cosa. Sei driver la dichiarano, e le origini
//! reali sono almeno quattro: la definizione emessa dal writer,
//! l'identificatore conservato, il CRS fisso del formato, il runtime GDAL.
//!
//! La conflazione non e' accademica. La regola che sembra ovvia -- «se per
//! questo piano il writer non emette nulla di `Preserved`, ogni `Derived`
//! decade ad `Absent`» -- correggerebbe lo Shapefile **inventando** una
//! perdita di CRS su `geojson`, `kml` e `filegdb`, che hanno `Derived` senza
//! alcun `Preserved` perche' derivano da altro. Tre driver rotti per
//! correggerne uno.
//!
//! # Sono state scritte prima della logica, e questo e' il loro esito
//!
//! Queste prove sono state scritte **contro il comportamento precedente**,
//! difetto compreso, ed eseguite verdi prima di toccare una riga di logica.
//! Introdotto il modello della provenienza, delle nove aspettative se n'e'
//! mossa **una sola**: lo Shapefile con un identificatore che non sa
//! sintetizzare, che ora dichiara `crs_id_not_preserved_absent` invece di
//! `..._derived`.
//!
//! Le altre otto -- `dxf` in entrambi i versi, `geojson`, `kml`, `gpkg`,
//! `filegdb`, e lo Shapefile nei due casi in cui la definizione c'e' -- non si
//! sono mosse. E' cio' che distingue una correzione locale da una regressione
//! trasversale, e resta vero solo finche' queste prove restano eseguite.
//!
//! # Perche' passa dal binario
//!
//! Il `LossReport` e' cio' che il prodotto **dichiara**, e lo dichiara sul
//! filo, in `write_loss.counts`. Una prova che chiamasse `planned_write_loss`
//! misurerebbe la funzione; qui interessa la promessa.

use std::path::{Path, PathBuf};
use std::process::Command;

const fn binario() -> &'static str {
    env!("CARGO_BIN_EXE_plenora-io")
}

/// Un CRS proiettato: identificatore senza definizione, e **nessun** writer ne
/// sintetizza il WKT. E' il piano su cui il difetto dello Shapefile si vede.
const PROIETTATO: &str = "EPSG:3003";

/// L'unico identificatore per cui `driver-shp` e `driver-dxf` sintetizzano una
/// definizione. Con questo il `.prj` c'e' anche senza WKT in ingresso.
const SINTETIZZABILE: &str = "EPSG:4326";

/// Il CRS che `geojson` e `kml` **impongono**. Non e' `EPSG:4326`: un piano
/// che lo dichiarasse cosi' viene rifiutato in validazione, e la prova
/// misurerebbe quel rifiuto invece della provenienza.
const FISSO_DEI_FORMATI: &str = "OGC:CRS84";

/// `driver-csv` non apre un CSV senza sapere quale colonna sia la geometria.
const OPZIONE_WKT: &str = "wkt_column=geom";

/// Sorgente minima: due punti e un attributo testuale.
///
/// Le coordinate sono lon/lat perche' `geojson` e `kml` fissano WGS84 e
/// rifiuterebbero un piano fuori dominio; per i bersagli proiettati il valore
/// numerico non entra in cio' che qui si misura, che e' il CRS dichiarato.
///
/// **Nessuna colonna intera**: `filegdb` round-trippa esattamente `Int32`,
/// `Float64` e `Utf8`, e l'inferenza del CSV darebbe `Int64` a una colonna di
/// numeri. Il rifiuto che ne seguirebbe e' corretto, ed e' su un tipo: non
/// direbbe niente sulla provenienza del CRS.
fn scrivi_csv(percorso: &Path) {
    std::fs::write(
        percorso,
        "nome,geom\nalfa,POINT(9.19 45.46)\nbeta,POINT(12.49 41.90)\n",
    )
    .expect("sorgente CSV scritta");
}

/// L'esito di una conversione: riuscita, e le due uscite **grezze**.
///
/// Il JSON non viene deserializzato qui, e non e' pigrizia: su un rifiuto lo
/// stdout e' vuoto e sullo stdout di un successo la busta d'errore non c'e'.
/// Un `Esito` che tenesse un `Value` dovrebbe scegliere che cosa mettere
/// nell'altro caso, e quella scelta sarebbe un **fallback**: il giorno in cui
/// la CLI smettesse di emettere il documento, la prova leggerebbe il segnaposto
/// invece di rompersi. Qui ogni lettura avviene dove la forma e' garantita, e
/// pretende di trovarla.
struct Esito {
    riuscita: bool,
    stdout: String,
    stderr: String,
}

impl Esito {
    /// Il documento di `convert`. Da chiamare solo dopo aver stabilito che la
    /// conversione e' riuscita.
    fn documento(&self) -> serde_json::Value {
        serde_json::from_str(self.stdout.trim()).expect("convert emette un documento JSON")
    }

    /// Il messaggio curato del rifiuto.
    ///
    /// Un rifiuto atteso va letto, non contato: `dxf` puo' rifiutare per molte
    /// ragioni, e una prova che si accontentasse di un'uscita diversa da zero
    /// passerebbe anche il giorno in cui rifiuta per un motivo che non c'entra.
    fn messaggio_di_errore(&self) -> String {
        let busta: serde_json::Value =
            serde_json::from_str(self.stderr.trim()).expect("il rifiuto emette una busta JSON");
        busta["error"]["message"]
            .as_str()
            .expect("la busta d'errore porta un messaggio")
            .to_owned()
    }
}

/// Esegue `convert`. `crs` diventa `--assume-crs` solo quando c'e': una
/// sorgente che porta gia' il proprio CRS non deve riceverne uno di fuori,
/// altrimenti la prova non direbbe piu' da dove il CRS e' arrivato.
fn converti(ingresso: &Path, uscita: &Path, crs: Option<&str>) -> Esito {
    let mut comando = Command::new(binario());
    comando.arg("convert").arg(ingresso).arg(uscita);
    if let Some(crs) = crs {
        comando.arg("--assume-crs").arg(crs);
    }
    if ingresso.extension().is_some_and(|ext| ext == "csv") {
        comando.arg("--in-opt").arg(OPZIONE_WKT);
    }
    let output = comando.output().expect("il binario si esegue");
    Esito {
        riuscita: output.status.success(),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

/// Le sole categorie di perdita che riguardano il CRS, ordinate.
///
/// Il filtro e' deliberato: una conversione perde anche altro -- nomi
/// troncati, tipi coercizzati -- e mescolarlo qui renderebbe l'aspettativa
/// sensibile a cambiamenti che non hanno niente a che vedere con la
/// provenienza.
fn perdite_crs(documento: &serde_json::Value) -> Vec<(String, u64)> {
    let counts = &documento["write_loss"]["counts"];
    let mut voci: Vec<(String, u64)> = match counts {
        // v2: un elenco di voci, ciascuna con la propria categoria.
        serde_json::Value::Array(elenco) => elenco
            .iter()
            .filter_map(|voce| {
                let categoria = voce["categoria"].as_str()?;
                Some((categoria.to_owned(), voce["conteggio"].as_u64()?))
            })
            .collect(),
        // v1 legacy: la mappa congelata.
        serde_json::Value::Object(mappa) => mappa
            .iter()
            .filter_map(|(categoria, conteggio)| Some((categoria.clone(), conteggio.as_u64()?)))
            .collect(),
        _ => Vec::new(),
    };
    voci.retain(|(categoria, _)| categoria.starts_with("crs_") || categoria.starts_with("srid_"));
    voci.sort();
    voci
}

/// Il confronto, con la misura stampata: una caratterizzazione che fallisce
/// deve dire **che cosa ha misurato**, non solo che non coincide.
fn caratterizza(caso: &str, esito: &Esito, atteso: &[(&str, u64)]) {
    assert!(
        esito.riuscita,
        "{caso}: conversione fallita.\nstderr: {}",
        esito.stderr
    );
    let misurato = perdite_crs(&esito.documento());
    let atteso: Vec<(String, u64)> = atteso
        .iter()
        .map(|(categoria, conteggio)| ((*categoria).to_owned(), *conteggio))
        .collect();
    assert_eq!(misurato, atteso, "{caso}: perdite CRS misurate diverse");
}

fn directory() -> tempfile::TempDir {
    tempfile::tempdir().expect("directory temporanea")
}

fn sorgente(dir: &Path) -> PathBuf {
    let percorso = dir.join("sorgente.csv");
    scrivi_csv(&percorso);
    percorso
}

/// La destinazione Shapefile nella forma **directory-dataset**.
///
/// Una destinazione `*.shp` viene rifiutata finche' non la si accetta con
/// `publish_mode=loose_shapefile_set`: pubblica quattro file sciolti e non e'
/// crash-atomic. La forma `*.shp.d` e' quella raccomandata, e qui evita che la
/// prova debba accettare una garanzia piu' debole per una ragione -- il CRS --
/// che con il publish non c'entra.
fn destinazione_shp(dir: &Path, nome: &str) -> PathBuf {
    dir.join(format!("{nome}.shp.d"))
}

/// Il `.prj` dentro un directory-dataset: il nome dei membri e' fisso.
fn prj_di(destinazione: &Path) -> PathBuf {
    destinazione.join("data.prj")
}

// --- shp: `crs_id` deriva dalla definizione emessa nel `.prj` --------------

/// Il caso del finding: identificatore senza WKT, e nessuna sintesi possibile.
///
/// Il `.prj` **non** viene scritto, quindi non esiste la definizione da cui
/// l'identificatore dovrebbe derivare, e la categoria onesta e'
/// `crs_id_not_preserved_absent`. Fino al 2026-09-04 il prodotto dichiarava
/// `..._derived`, cioe' mandava chi legge a cercare dentro il file un valore
/// che nessuno vi aveva scritto.
#[test]
fn shp_con_identificatore_non_sintetizzabile_non_scrive_il_prj() {
    let dir = directory();
    let uscita = destinazione_shp(dir.path(), "uscita");
    let esito = converti(&sorgente(dir.path()), &uscita, Some(PROIETTATO));
    assert!(
        !prj_di(&uscita).exists(),
        "senza WKT e fuori dagli identificatori sintetizzabili il .prj non esiste"
    );
    caratterizza(
        "shp <- identificatore non sintetizzabile",
        &esito,
        &[("crs_id_not_preserved_absent", 1)],
    );
}

/// Lo stesso piano con l'identificatore che il writer sa sintetizzare: il
/// `.prj` c'e', e `Derived` e' vera.
#[test]
fn shp_con_identificatore_sintetizzabile_scrive_il_prj() {
    let dir = directory();
    let uscita = destinazione_shp(dir.path(), "uscita");
    let esito = converti(&sorgente(dir.path()), &uscita, Some(SINTETIZZABILE));
    assert!(
        prj_di(&uscita).exists(),
        "per WGS84 il writer sintetizza il WKT e il .prj esiste"
    );
    caratterizza(
        "shp <- identificatore sintetizzabile",
        &esito,
        &[("crs_id_not_preserved_derived", 1)],
    );
}

/// La sorgente porta una definizione: e' il caso in cui `FromDefinition` non
/// dipende da nessuna sintesi.
#[test]
fn shp_da_una_sorgente_con_definizione_riscrive_il_prj() {
    let dir = directory();
    let primo = destinazione_shp(dir.path(), "primo");
    let esito_primo = converti(&sorgente(dir.path()), &primo, Some(SINTETIZZABILE));
    assert!(esito_primo.riuscita, "primo passo: {}", esito_primo.stderr);

    let secondo = destinazione_shp(dir.path(), "secondo");
    let esito = converti(&primo, &secondo, None);
    assert!(
        prj_di(&secondo).exists(),
        "la definizione letta dal .prj viene riscritta"
    );
    caratterizza(
        "shp <- sorgente con definizione",
        &esito,
        &[("crs_id_not_preserved_derived", 1)],
    );
}

/// La sorgente porta un **SRID**, e lo Shapefile non ha dove metterlo.
///
/// Il CSV non porta SRID, quindi il caso non si vede partendo da li': serve un
/// formato che lo conservi, e il `GeoPackage` lo fa. Qui si legge la terza riga
/// della regola -- identificatore non sintetizzabile: anche l'SRID e'
/// `Absent` -- che con una sorgente senza SRID resterebbe non provata.
#[test]
fn shp_da_una_sorgente_con_srid_dichiara_anche_la_perdita_dell_srid() {
    let dir = directory();
    let gpkg = dir.path().join("intermedio.gpkg");
    let primo = converti(&sorgente(dir.path()), &gpkg, Some(PROIETTATO));
    assert!(primo.riuscita, "primo passo: {}", primo.stderr);

    let uscita = destinazione_shp(dir.path(), "uscita");
    let esito = converti(&gpkg, &uscita, None);
    caratterizza(
        "shp <- sorgente con SRID",
        &esito,
        &[
            ("crs_id_not_preserved_absent", 1),
            ("srid_not_preserved_absent", 1),
        ],
    );
}

// --- dxf: `crs_id` deriva dalla definizione, ma il piano senza definizione
//     viene **rifiutato** invece che scritto senza --------------------------

/// La differenza fra `dxf` e `shp` non sta nella capability, che e' la stessa:
/// sta nel fatto che `dxf` rifiuta il piano che non sa scrivere, mentre `shp`
/// lo scrive senza `.prj`. E' la ragione per cui una regola generale sulla
/// provenienza non muove `dxf`.
#[test]
fn dxf_rifiuta_l_identificatore_senza_definizione() {
    let dir = directory();
    let uscita = dir.path().join("uscita.dxf");
    let esito = converti(&sorgente(dir.path()), &uscita, Some(PROIETTATO));
    assert!(
        !esito.riuscita,
        "dxf deve rifiutare il solo authority id: stdout {}",
        esito.stdout
    );
    let messaggio = esito.messaggio_di_errore();
    assert!(
        messaggio.contains("definizione"),
        "il rifiuto deve essere quello sulla definizione mancante, non un altro: «{messaggio}»"
    );
}

#[test]
fn dxf_con_identificatore_sintetizzabile_incorpora_la_definizione() {
    let dir = directory();
    let uscita = dir.path().join("uscita.dxf");
    let esito = converti(&sorgente(dir.path()), &uscita, Some(SINTETIZZABILE));
    caratterizza(
        "dxf <- identificatore sintetizzabile",
        &esito,
        &[("crs_id_not_preserved_derived", 1)],
    );
}

// --- geojson e kml: `crs_id` deriva dal CRS **fisso del formato** -----------

/// Nessuna rappresentazione `Preserved`, e `Derived` resta vera comunque: il
/// formato fissa WGS84, quindi l'identificatore e' ricavabile per costruzione.
/// E' il primo dei tre controesempi alla regola sbagliata.
#[test]
fn geojson_deriva_l_identificatore_dal_crs_fisso_del_formato() {
    let dir = directory();
    let uscita = dir.path().join("uscita.geojson");
    let esito = converti(&sorgente(dir.path()), &uscita, Some(FISSO_DEI_FORMATI));
    caratterizza(
        "geojson <- CRS fisso",
        &esito,
        &[("crs_id_not_preserved_derived", 1)],
    );
}

#[test]
fn kml_deriva_l_identificatore_dal_crs_fisso_del_formato() {
    let dir = directory();
    let uscita = dir.path().join("uscita.kml");
    let esito = converti(&sorgente(dir.path()), &uscita, Some(FISSO_DEI_FORMATI));
    caratterizza(
        "kml <- CRS fisso",
        &esito,
        &[("crs_id_not_preserved_derived", 1)],
    );
}

// --- gpkg: srid e definizione derivano dall'**identificatore conservato** ---

/// Il verso opposto a tutti gli altri: qui `crs_id` e' `Preserved` e sono le
/// altre due a derivare da lui.
#[test]
fn gpkg_conserva_l_identificatore_e_ne_deriva_le_altre_rappresentazioni() {
    let dir = directory();
    let uscita = dir.path().join("uscita.gpkg");
    let esito = converti(&sorgente(dir.path()), &uscita, Some(PROIETTATO));
    caratterizza("gpkg <- identificatore conservato", &esito, &[]);
}

// --- filegdb: entrambe derivano dal **runtime GDAL** -----------------------

/// Il terzo controesempio: `Derived` senza alcun `Preserved`, e vera, perche'
/// a ricavarla e' GDAL a runtime.
#[cfg(feature = "gdal-backend")]
#[test]
fn filegdb_deriva_le_rappresentazioni_dal_runtime_gdal() {
    let dir = directory();
    let uscita = dir.path().join("uscita.gdb");
    let esito = converti(&sorgente(dir.path()), &uscita, Some(PROIETTATO));
    caratterizza(
        "filegdb <- runtime GDAL",
        &esito,
        &[("crs_id_not_preserved_derived", 1)],
    );
}
