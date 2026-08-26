//! I metadati `geo` di `GeoParquet`, validati per intero.
//!
//! # Che cosa c'era prima
//!
//! `serde_json::from_str(&raw).ok()`. Un metadato `geo` malformato diventava
//! `None`, cioe' **indistinguibile da un file che `geo` non ce l'ha**: il
//! driver passava a indovinare la colonna geometria fra `geometry`, `geom` e
//! `wkb`, e la colonna indovinata poteva non essere quella che
//! `primary_column` dichiarava. Un `GeoParquet` corrotto veniva letto come
//! Parquet semplice, e nessuno lo sapeva.
//!
//! Dei campi del documento ne venivano consultati cinque -- `primary_column`,
//! `columns`, `crs`, `geometry_types`, `covering.bbox`. `version`, `encoding`,
//! `edges`, `orientation`, `epoch` e il `bbox` di colonna non venivano guardati
//! affatto. Le due conseguenze che pesavano:
//!
//! * una colonna con `encoding` nativo `GeoArrow` -- valida in `GeoParquet` 1.1
//!   -- veniva consegnata a valle etichettata `geoarrow.wkb`, cioe' **letta
//!   come se fosse WKB**;
//! * `edges: "spherical"` veniva trattato come planare, e i bordi sferici non
//!   sono una nota di stile: cambiano il significato geometrico dei dati.
//!
//! # Le due famiglie di rifiuto
//!
//! Sono distinte, e la distinzione e' il contratto di questo modulo:
//!
//! * **non conforme** -- il documento non rispetta la specifica. Errore di
//!   formato, `IoErrorCode::Format`.
//! * **valida e non supportata** -- il documento e' corretto e chiede una
//!   semantica che questa libreria non implementa. `IoErrorCode::Unsupported`.
//!
//! Confonderle manderebbe chi legge a correggere un file che non ha niente che
//! non va, o ad aspettare da noi una funzione che non abbiamo.
//!
//! # Il perimetro dichiarato
//!
//! Si accettano **esattamente** `1.0.0` e `1.1.0`, i due valori che gli schemi
//! ufficiali fissano; la correzione `v1.1.0+p1` tiene `1.1.0` nei metadati.
//! Qualunque altra versione, `2.x` compresa, e' una versione non supportata --
//! non un JSON malformato. Il supporto arriva fino a 1.1 e si ferma li', e lo
//! dice invece di lasciarlo dedurre.

use std::collections::BTreeMap;

use plenora_io_model::contract::{CoordinateDimensions, GeometryType};
use plenora_io_model::{NumeroStrutturale, PlenoraIoError, PublicMessage, Result};

/// Le versioni di `GeoParquet` che questa libreria legge, per intero.
///
/// Vengono dal modulo che porta gli schemi: `"version"` e' un `const` negli
/// schemi ufficiali, e queste sono le due versioni per cui un validatore e'
/// incorporato. Riscriverle qui vorrebbe dire avere due elenchi da tenere
/// allineati a mano.
pub use crate::schema_ufficiale::VERSIONI_SUPPORTATE;

/// Le codifiche native introdotte da `GeoParquet` 1.1.
///
/// Sono valide, e questa libreria non ne implementa la semantica: decodificarle
/// come WKB darebbe dati sbagliati in silenzio, che e' peggio del non leggerle.
const CODIFICHE_NATIVE: [&str; 6] = [
    "point",
    "linestring",
    "polygon",
    "multipoint",
    "multilinestring",
    "multipolygon",
];

/// I nomi di tipo geometrico ammessi in `geometry_types`, senza suffisso.
const NOMI_DI_TIPO: [(&str, GeometryType); 7] = [
    ("Point", GeometryType::Point),
    ("LineString", GeometryType::LineString),
    ("Polygon", GeometryType::Polygon),
    ("MultiPoint", GeometryType::MultiPoint),
    ("MultiLineString", GeometryType::MultiLineString),
    ("MultiPolygon", GeometryType::MultiPolygon),
    ("GeometryCollection", GeometryType::GeometryCollection),
];

/// I suffissi di dimensionalita' ammessi dallo schema ufficiale.
///
/// Sono due, e non quattro. Il pattern e'
/// `^(GeometryCollection|(Multi)?(Point|LineString|Polygon))( Z)?$` in
/// entrambi gli schemi, 1.0.0 e 1.1.0: **`" M"` e `" ZM"` non esistono** in
/// `GeoParquet`.
///
/// La prima stesura li ammetteva, e la ragione che ci aveva scritto accanto --
/// «il nostro writer li emette, rifiutarli renderebbe illeggibili i file che
/// abbiamo scritto noi» -- era il ragionamento sbagliato: il writer emetteva
/// metadati non conformi, e la conclusione giusta era correggere il writer, non
/// allargare il lettore. Ora il writer rifiuta di scrivere geometrie XYM e
/// XYZM, e il lettore rifiuta le loro etichette.
const SUFFISSI: [(&str, CoordinateDimensions); 2] = [
    (" Z", CoordinateDimensions::Xyz),
    ("", CoordinateDimensions::Xy),
];

/// Gli spigoli del `covering.bbox`, nell'ordine in cui il pruning li usa.
pub const SPIGOLI: [&str; 4] = ["xmin", "ymin", "xmax", "ymax"];

/// Un documento che non rispetta la specifica.
fn non_conforme(messaggio: &PublicMessage) -> PlenoraIoError {
    PlenoraIoError::formato_redatto("geoparquet", messaggio)
}

/// Un documento corretto che chiede una semantica che non implementiamo.
fn non_supportata(messaggio: &PublicMessage) -> PlenoraIoError {
    PlenoraIoError::non_supportato_redatto(messaggio)
}

/// I bordi dichiarati da una colonna.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Bordi {
    /// `planar`, o assente: e' il default della specifica.
    Planari,
}

/// L'orientamento degli anelli dichiarato da una colonna.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Orientamento {
    Antiorario,
}

/// Come il documento e' stato accettato.
///
/// Esiste perche' una via di compatibilita' che non si vede diventa il
/// comportamento normale: chi legge il contratto deve poter sapere che quel
/// file **non** e' conforme, anche quando lo abbiamo letto lo stesso.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Conformita {
    /// Il documento rispetta lo schema ufficiale della versione che dichiara.
    Conforme,
    /// Il documento e' conforme **tranne** che nel `crs`, che porta la forma
    /// storica `{"id": {...}}` prodotta da questo repository prima del lotto
    /// S10. Accettato solo su richiesta esplicita.
    CrsStoricoSoloIdentificatore,
}

/// Il sistema di riferimento dichiarato da una colonna.
///
/// Sono **tre** stati, e la specifica li distingue: il campo assente vuol dire
/// `OGC:CRS84`, il campo `null` vuol dire che il CRS non c'e' -- mancante o
/// sconosciuto -- e un documento PROJJSON vuol dire quel CRS.
///
/// La prima stesura ne aveva due: assente e `null` finivano tutt'e due in
/// `None`, e il driver li trasformava tutt'e due in CRS84. Un file che dichiara
/// esplicitamente di **non sapere** il proprio CRS veniva letto come se avesse
/// dichiarato di essere in WGS84, che e' un'affermazione che nessuno aveva
/// fatto.
#[derive(Clone, Debug, PartialEq)]
pub enum Crs {
    /// Il campo non c'e': la specifica dice `OGC:CRS84`.
    Assente,
    /// Il campo c'e' e vale `null`: il CRS non e' dichiarato.
    Nullo,
    /// Un documento PROJJSON.
    Documento(serde_json::Value),
    /// La forma storica `{"id": {"authority": ..., "code": ...}}`, che PROJJSON
    /// non e' e che questo repository ha scritto fino a S10.
    ///
    /// Porta l'identificatore composto -- `EPSG:4326` -- perche' e' cio' che
    /// quel file dichiarava: ignorarlo vorrebbe dire dichiarare CRS84 un dato
    /// che CRS84 non e'.
    StoricoSoloIdentificatore(String),
}

/// I metadati `geo` di un file, validati.
///
/// La colonna primaria sta in un campo suo, e non dentro la mappa, perche' la
/// specifica pretende che esista e la validazione lo verifica: tenendola nella
/// mappa, ogni chiamante avrebbe dovuto trattare un `None` che non puo'
/// accadere, e un ramo impossibile non e' un controllo -- e' una promessa.
#[derive(Clone, Debug)]
pub struct MetadatiGeo {
    /// La versione dichiarata, gia' ristretta a `VERSIONI_SUPPORTATE`.
    pub versione: &'static str,
    /// Se il documento e' conforme, o accettato per compatibilita'.
    pub conformita: Conformita,
    pub nome_primaria: String,
    pub primaria: ColonnaGeo,
    /// Le altre colonne geometriche, validate come la prima.
    pub secondarie: BTreeMap<String, ColonnaGeo>,
}

/// I metadati di una colonna geometrica, validati.
#[derive(Clone, Debug)]
pub struct ColonnaGeo {
    /// Il documento JSON della colonna, conservato per `geoparquet.column`.
    ///
    /// E' una superficie di contratto gia' pubblicata: la validazione la
    /// verifica, non la riscrive.
    pub grezza: serde_json::Value,
    /// I tipi dichiarati. Puo' essere vuoto: la specifica lo ammette, e vuol
    /// dire «non vincolato», non «nessuna geometria».
    pub tipi: Vec<(GeometryType, CoordinateDimensions)>,
    /// Il `crs` dichiarato, nei suoi tre stati distinti.
    pub crs: Crs,
    pub bordi: Bordi,
    pub orientamento: Option<Orientamento>,
    pub bbox: Option<Vec<f64>>,
    pub epoch: Option<f64>,
    /// I quattro **percorsi di colonna** del `covering.bbox`, in ordine
    /// `xmin, ymin, xmax, ymax`.
    ///
    /// Percorsi, non nomi: lo schema 1.1.0 pretende esattamente due segmenti
    /// per spigolo, il secondo uguale al nome dello spigolo -- `["bbox",
    /// "xmin"]` -- cioe' una colonna struct con quattro figli. La prima stesura
    /// leggeva e scriveva un solo segmento, e quei documenti **non erano
    /// `GeoParquet` 1.1 validi**.
    ///
    /// Vale `None` quando il covering non c'e', e quando c'e' in un documento
    /// **1.0.0**: li' la specifica non gli attribuisce alcun significato, e
    /// darglielo lo darebbe noi. Restare senza pruning e' il verso in cui
    /// questo driver sbaglia per contratto.
    pub covering: Option<[Vec<String>; 4]>,
}

/// Analizza il documento `geo` di un file, per intero.
///
/// # Errors
///
/// `Format` se il documento non rispetta la specifica, `Unsupported` se chiede
/// una versione o una semantica che questa libreria non implementa.
pub fn analizza(grezzo: &str, accetta_crs_storico: bool) -> Result<MetadatiGeo> {
    let documento: serde_json::Value = serde_json::from_str(grezzo).map_err(|_| {
        non_conforme(&PublicMessage::Curated(
            "metadato `geo` GeoParquet che non e' JSON",
        ))
    })?;
    let oggetto = documento.as_object().ok_or_else(|| {
        non_conforme(&PublicMessage::Curated(
            "metadato `geo` GeoParquet che non e' un oggetto",
        ))
    })?;

    // La versione decide **quale** schema e' l'autorita', quindi si legge
    // prima: senza, non si saprebbe contro cosa validare.
    let versione = versione(oggetto)?;

    // Da qui in poi ci sono due giudizi, e non sono lo stesso giudizio.
    //
    // Lo **schema ufficiale** dice se il documento e' conforme: e' l'autorita',
    // e il verdetto e' suo. La **lettura** trasforma il documento in una
    // struttura tipizzata, e sa dire con precisione che cosa non andava --
    // «manca uno spigolo del covering» invece di «non rispetta lo schema».
    //
    // Tenerli separati e' cio' che impedisce alla nostra prosa di diventare la
    // specifica. Le quattro combinazioni sono tutte trattate, e due di esse
    // sono divergenze che vale la pena poter vedere:
    //
    //   schema rifiuta + lettura rifiuta  -> il caso normale, e si da' il
    //                                        messaggio preciso;
    //   schema rifiuta + lettura accetta  -> la nostra regola e' piu' **larga**
    //                                        dello schema: vince lo schema, e
    //                                        il documento e' rifiutato lo stesso;
    //   schema accetta + lettura rifiuta  -> la nostra regola e' piu' **stretta**
    //                                        dello schema. Rifiutiamo, perche'
    //                                        non sappiamo costruire la struttura,
    //                                        e il messaggio lo dice;
    //   accettano entrambi                -> si legge.
    let mut esito_schema = crate::schema_ufficiale::valida(&documento, versione);
    let mut conformita = Conformita::Conforme;

    // La via di compatibilita', **stretta e spenta per default**.
    //
    // Vale solo per 1.0.0 -- la versione che questo repository scriveva -- e
    // solo se, tolti quei `crs`, il documento e' conforme: cosi' l'opzione
    // tollera esattamente cio' che dichiara di tollerare, e un file con altri
    // difetti non passa perche' qualcuno l'ha accesa.
    if esito_schema.is_err() && accetta_crs_storico && versione == "1.0.0" {
        if let Some(senza) = senza_crs_storici(&documento) {
            if crate::schema_ufficiale::valida(&senza, versione).is_ok() {
                esito_schema = Ok(());
                conformita = Conformita::CrsStoricoSoloIdentificatore;
            }
        }
    }

    match (esito_schema, leggi(oggetto, versione, conformita)) {
        (Ok(()), letto) => letto,
        (Err(dello_schema), Err(preciso)) => {
            // Il verdetto e' dello schema, la ragione e' la nostra: e' il caso
            // in cui i due giudizi coincidono, che e' il caso normale.
            let _ = dello_schema;
            Err(preciso)
        }
        (Err(dello_schema), Ok(_)) => Err(dello_schema),
    }
}

/// Legge un documento gia' conforme, e ne costruisce la struttura tipizzata.
fn leggi(
    oggetto: &serde_json::Map<String, serde_json::Value>,
    versione: &'static str,
    conformita: Conformita,
) -> Result<MetadatiGeo> {
    let colonna_primaria = stringa_obbligatoria(oggetto, "primary_column")?;

    let colonne_grezze = oggetto
        .get("columns")
        .ok_or_else(|| {
            non_conforme(&PublicMessage::CuratedPair(
                "metadato `geo` GeoParquet senza il campo obbligatorio",
                "columns",
            ))
        })?
        .as_object()
        .ok_or_else(|| {
            non_conforme(&PublicMessage::CuratedPair(
                "metadato `geo` GeoParquet con un campo che non e' un oggetto",
                "columns",
            ))
        })?;

    let grezza_primaria = colonne_grezze.get(&colonna_primaria).ok_or_else(|| {
        non_conforme(&PublicMessage::Curated(
            "metadato `geo` GeoParquet in cui `primary_column` non compare fra `columns`",
        ))
    })?;
    let primaria = colonna(grezza_primaria, versione, conformita)?;

    let mut secondarie = BTreeMap::new();
    for (nome, grezza) in colonne_grezze {
        if nome == &colonna_primaria {
            continue;
        }
        secondarie.insert(nome.clone(), colonna(grezza, versione, conformita)?);
    }

    Ok(MetadatiGeo {
        versione,
        conformita,
        nome_primaria: colonna_primaria,
        primaria,
        secondarie,
    })
}

/// La forma storica **esatta** che questo repository scriveva nel campo `crs`.
///
/// `{"id": {"authority": <stringa non vuota>, "code": <stringa o intero>}}`, e
/// nient'altro: nessuna chiave in piu' al primo livello ne' dentro `id`. E' una
/// via di compatibilita', non un modo di accettare PROJJSON invalidi in
/// generale -- se fosse larga, sarebbe un buco travestito da cortesia.
fn identificatore_storico(crs: &serde_json::Value) -> Option<String> {
    let oggetto = crs.as_object()?;
    if oggetto.len() != 1 {
        return None;
    }
    let id = oggetto.get("id")?.as_object()?;
    if id.len() != 2 {
        return None;
    }
    let autorita = id.get("authority")?.as_str().filter(|s| !s.is_empty())?;
    let codice = match id.get("code")? {
        serde_json::Value::String(s) if !s.is_empty() => s.clone(),
        serde_json::Value::Number(n) if n.is_i64() || n.is_u64() => n.to_string(),
        _ => return None,
    };
    Some(format!("{autorita}:{codice}"))
}

/// Il documento con i `crs` storici sostituiti da `null`, se ce n'e' almeno uno.
///
/// Serve a chiedere all'autorita' una domanda precisa: «a parte quei `crs`,
/// questo documento e' conforme?». Se la risposta e' si', l'unica non
/// conformita' e' quella che l'opzione dichiara di tollerare; se e' no, il file
/// ha altro che non va e l'opzione non lo salva.
fn senza_crs_storici(documento: &serde_json::Value) -> Option<serde_json::Value> {
    let mut copia = documento.clone();
    let colonne = copia.get_mut("columns")?.as_object_mut()?;
    let mut trovato = false;
    for (_, colonna) in colonne.iter_mut() {
        let Some(oggetto) = colonna.as_object_mut() else {
            continue;
        };
        let storico = oggetto
            .get("crs")
            .and_then(identificatore_storico)
            .is_some();
        if storico {
            oggetto.insert("crs".to_owned(), serde_json::Value::Null);
            trovato = true;
        }
    }
    trovato.then_some(copia)
}

/// La versione dichiarata, ristretta ai due valori degli schemi ufficiali.
fn versione(oggetto: &serde_json::Map<String, serde_json::Value>) -> Result<&'static str> {
    let dichiarata = stringa_obbligatoria(oggetto, "version")?;
    VERSIONI_SUPPORTATE
        .into_iter()
        .find(|supportata| *supportata == dichiarata)
        .ok_or_else(|| {
            // Non e' un formato invalido: e' un documento che si dichiara di
            // un'altra versione, e leggerlo con le regole di questa vorrebbe
            // dire applicargli regole che non sono le sue.
            non_supportata(&PublicMessage::CuratedPair(
                "versione GeoParquet non supportata: sono supportate",
                "1.0.0 e 1.1.0",
            ))
        })
}

/// Una stringa obbligatoria del documento.
fn stringa_obbligatoria(
    oggetto: &serde_json::Map<String, serde_json::Value>,
    campo: &'static str,
) -> Result<String> {
    let valore = oggetto.get(campo).ok_or_else(|| {
        non_conforme(&PublicMessage::CuratedPair(
            "metadato `geo` GeoParquet senza il campo obbligatorio",
            campo,
        ))
    })?;
    valore
        .as_str()
        .filter(|testo| !testo.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| {
            non_conforme(&PublicMessage::CuratedPair(
                "metadato `geo` GeoParquet con un campo che non e' una stringa non vuota",
                campo,
            ))
        })
}

/// I metadati di una colonna.
fn colonna(
    grezza: &serde_json::Value,
    versione: &'static str,
    conformita: Conformita,
) -> Result<ColonnaGeo> {
    let oggetto = grezza.as_object().ok_or_else(|| {
        non_conforme(&PublicMessage::Curated(
            "colonna GeoParquet che non e' un oggetto",
        ))
    })?;

    encoding(oggetto, versione)?;
    let tipi = geometry_types(oggetto)?;
    let crs = crs(oggetto, conformita)?;
    let bordi = bordi(oggetto)?;
    let orientamento = orientamento(oggetto)?;
    let bbox = bbox(oggetto)?;
    let epoch = epoch(oggetto)?;
    let covering = covering(oggetto, versione)?;

    Ok(ColonnaGeo {
        grezza: grezza.clone(),
        tipi,
        crs,
        bordi,
        orientamento,
        bbox,
        epoch,
        covering,
    })
}

/// La codifica della colonna.
///
/// Solo `WKB` e' leggibile qui. Le codifiche native esistono da 1.1: in un
/// documento che si dichiara 1.0.0 non sono valide, in uno 1.1.0 sono valide e
/// non supportate -- ed e' la versione dichiarata a dire quale delle due cose
/// sono, che e' esattamente il servizio che quel campo rende.
fn encoding(
    oggetto: &serde_json::Map<String, serde_json::Value>,
    versione: &'static str,
) -> Result<()> {
    let dichiarata = stringa_obbligatoria(oggetto, "encoding")?;
    if dichiarata == "WKB" {
        return Ok(());
    }
    if versione == "1.1.0" && CODIFICHE_NATIVE.contains(&dichiarata.as_str()) {
        return Err(non_supportata(&PublicMessage::CuratedPair(
            "codifica nativa GeoParquet 1.1 non supportata: e' letta soltanto",
            "WKB",
        )));
    }
    Err(non_conforme(&PublicMessage::CuratedPair(
        "colonna GeoParquet con una codifica che la sua versione non ammette; e' letta soltanto",
        "WKB",
    )))
}

/// I tipi geometrici dichiarati.
///
/// Il campo e' obbligatorio; l'elenco puo' essere vuoto, e vuol dire «non
/// vincolato». Le etichette vengono da un insieme **chiuso**: prima venivano
/// filtrate con `filter_map`, cioe' un'etichetta che non si riconosceva
/// spariva e il contratto della colonna usciva piu' povero senza che nulla lo
/// dicesse.
fn geometry_types(
    oggetto: &serde_json::Map<String, serde_json::Value>,
) -> Result<Vec<(GeometryType, CoordinateDimensions)>> {
    let valore = oggetto.get("geometry_types").ok_or_else(|| {
        non_conforme(&PublicMessage::CuratedPair(
            "colonna GeoParquet senza il campo obbligatorio",
            "geometry_types",
        ))
    })?;
    let elenco = valore.as_array().ok_or_else(|| {
        non_conforme(&PublicMessage::CuratedPair(
            "colonna GeoParquet con un campo che non e' un elenco",
            "geometry_types",
        ))
    })?;

    let mut tipi = Vec::with_capacity(elenco.len());
    for voce in elenco {
        let etichetta = voce.as_str().ok_or_else(|| {
            non_conforme(&PublicMessage::Curated(
                "colonna GeoParquet con un `geometry_types` che non e' una stringa",
            ))
        })?;
        let letto = etichetta_di_tipo(etichetta).ok_or_else(|| {
            non_conforme(&PublicMessage::Curated(
                "colonna GeoParquet con un tipo geometrico che non appartiene alla specifica",
            ))
        })?;
        // `uniqueItems: true`. La prima stesura deduplicava in silenzio, cioe'
        // accettava un documento che lo schema rifiuta e ne nascondeva la
        // ragione: chi lo ha scritto continuava a scriverlo.
        if tipi.contains(&letto) {
            return Err(non_conforme(&PublicMessage::Curated(
                "colonna GeoParquet con un tipo geometrico ripetuto in `geometry_types`",
            )));
        }
        tipi.push(letto);
    }
    Ok(tipi)
}

/// Un'etichetta di tipo, dall'insieme chiuso.
#[must_use]
pub fn etichetta_di_tipo(etichetta: &str) -> Option<(GeometryType, CoordinateDimensions)> {
    for (suffisso, dimensioni) in SUFFISSI {
        let Some(nome) = etichetta.strip_suffix(suffisso) else {
            continue;
        };
        if let Some((_, tipo)) = NOMI_DI_TIPO.iter().find(|(atteso, _)| *atteso == nome) {
            return Some((*tipo, dimensioni));
        }
    }
    None
}

/// Il `crs` della colonna, nei suoi tre stati.
///
/// Lo schema dice `oneOf: [PROJJSON, null]`, e il campo e' opzionale: assente,
/// `null` e documento sono tre cose diverse, e la differenza fra le prime due
/// e' quella che il driver sbagliava.
fn crs(
    oggetto: &serde_json::Map<String, serde_json::Value>,
    conformita: Conformita,
) -> Result<Crs> {
    let Some(valore) = oggetto.get("crs") else {
        return Ok(Crs::Assente);
    };
    if valore.is_null() {
        return Ok(Crs::Nullo);
    }
    // La forma storica si riconosce **solo** se il documento e' stato accettato
    // per compatibilita': altrimenti non e' mai arrivato fin qui. Si legge una
    // volta e si usa quella -- rileggerla dopo una guardia `is_some()` avrebbe
    // richiesto un ripiego per un caso che la guardia ha gia' escluso, e un
    // ripiego che non puo' scattare non e' una difesa.
    if conformita == Conformita::CrsStoricoSoloIdentificatore {
        if let Some(id) = identificatore_storico(valore) {
            return Ok(Crs::StoricoSoloIdentificatore(id));
        }
    }
    if valore.is_object() {
        return Ok(Crs::Documento(valore.clone()));
    }
    Err(non_conforme(&PublicMessage::Curated(
        "colonna GeoParquet con un `crs` che non e' ne' un oggetto PROJJSON ne' null",
    )))
}

/// I bordi della colonna.
fn bordi(oggetto: &serde_json::Map<String, serde_json::Value>) -> Result<Bordi> {
    let Some(valore) = oggetto.get("edges") else {
        return Ok(Bordi::Planari);
    };
    let dichiarati = valore.as_str().ok_or_else(|| {
        non_conforme(&PublicMessage::CuratedPair(
            "colonna GeoParquet con un campo che non e' una stringa",
            "edges",
        ))
    })?;
    match dichiarati {
        "planar" => Ok(Bordi::Planari),
        // Validi, e non implementati: i bordi sferici non sono una nota di
        // stile, cambiano il significato geometrico dei dati.
        "spherical" => Err(non_supportata(&PublicMessage::CuratedPair(
            "bordi sferici GeoParquet non supportati: sono letti soltanto i bordi",
            "planar",
        ))),
        _ => Err(non_conforme(&PublicMessage::Curated(
            "colonna GeoParquet con `edges` fuori dall'insieme della specifica",
        ))),
    }
}

/// L'orientamento degli anelli.
///
/// La specifica ne ammette un valore solo. Il driver non riordina gli anelli --
/// il WKB passa cosi' com'e' -- quindi il campo si valida e si conserva, non si
/// onora: dichiararlo onorato sarebbe una garanzia che nessuno mantiene.
fn orientamento(
    oggetto: &serde_json::Map<String, serde_json::Value>,
) -> Result<Option<Orientamento>> {
    let Some(valore) = oggetto.get("orientation") else {
        return Ok(None);
    };
    let dichiarato = valore.as_str().ok_or_else(|| {
        non_conforme(&PublicMessage::CuratedPair(
            "colonna GeoParquet con un campo che non e' una stringa",
            "orientation",
        ))
    })?;
    if dichiarato == "counterclockwise" {
        Ok(Some(Orientamento::Antiorario))
    } else {
        Err(non_conforme(&PublicMessage::Curated(
            "colonna GeoParquet con `orientation` fuori dall'insieme della specifica",
        )))
    }
}

/// Il `bbox` della colonna: quattro numeri, o sei con la quota.
fn bbox(oggetto: &serde_json::Map<String, serde_json::Value>) -> Result<Option<Vec<f64>>> {
    let Some(valore) = oggetto.get("bbox") else {
        return Ok(None);
    };
    let elenco = valore.as_array().ok_or_else(|| {
        non_conforme(&PublicMessage::CuratedPair(
            "colonna GeoParquet con un campo che non e' un elenco",
            "bbox",
        ))
    })?;
    if elenco.len() != 4 && elenco.len() != 6 {
        return Err(non_conforme(&PublicMessage::CuratedBetween(
            "colonna GeoParquet con un `bbox` che non ha",
            NumeroStrutturale::Limite(4),
            "ne'",
            NumeroStrutturale::Limite(6),
        )));
    }

    let mut numeri = Vec::with_capacity(elenco.len());
    for voce in elenco {
        // `is_finite` non serve, e tenerlo sarebbe una guardia che non puo'
        // scattare: JSON non sa scrivere un infinito, e `serde_json` rifiuta
        // da se' i letterali che traboccherebbero in `f64` -- `1e400` non
        // arriva mai qui, il documento e' gia' stato respinto come non-JSON.
        // La sonda `json_non_sa_esprimere_un_numero_non_finito` fissa questo
        // fatto della dipendenza, cosi' il giorno in cui cambiasse sarebbe
        // rossa invece che silenziosa.
        let letto = voce.as_f64().ok_or_else(|| {
            non_conforme(&PublicMessage::Curated(
                "colonna GeoParquet con un `bbox` che contiene un valore che non e' un numero",
            ))
        })?;
        numeri.push(letto);
    }

    // Il minimo **puo'** superare il massimo, e lo schema non lo vieta: un
    // riquadro che attraversa l'antimeridiano si scrive proprio cosi'. La
    // prima stesura lo rifiutava, cioe' dichiarava non conforme un documento
    // che la specifica accetta -- una divergenza nostra nel verso del rifiuto,
    // che e' comunque una divergenza.
    //
    // Cio' che quel riquadro non e' e' **usabile per il pruning** con la
    // semplice intersezione di rettangoli: `interpretabile_per_il_pruning` lo
    // dice al chiamante, che spegne il pruning invece di rifiutare il file.
    Ok(Some(numeri))
}

/// L'epoca delle coordinate.
fn epoch(oggetto: &serde_json::Map<String, serde_json::Value>) -> Result<Option<f64>> {
    let Some(valore) = oggetto.get("epoch") else {
        return Ok(None);
    };
    valore.as_f64().map(Some).ok_or_else(|| {
        non_conforme(&PublicMessage::CuratedPair(
            "colonna GeoParquet con un campo che non e' un numero",
            "epoch",
        ))
    })
}

/// Il `covering.bbox`, come lo schema 1.1.0 lo definisce.
///
/// # Due segmenti, non uno
///
/// Ogni spigolo e' un percorso di **esattamente due** segmenti, il secondo
/// uguale al nome dello spigolo: `["bbox", "xmin"]`. E' la forma di una colonna
/// **struct** con quattro figli, ed e' l'unica che lo schema ammette --
/// `minItems: 2, maxItems: 2`, con `{"const": "xmin"}` in seconda posizione.
///
/// La prima stesura accettava un solo segmento e lo chiamava «utilizzabile»,
/// trattando quello a due segmenti come «valido e inutilizzabile»: esattamente
/// al contrario. E il writer emetteva la forma piatta, cioe' scriveva documenti
/// che `GeoParquet` 1.1 non ammette.
///
/// # E solo in 1.1.0
///
/// `covering` non esiste nello schema 1.0.0. Un documento 1.0.0 che lo porta
/// resta **valido** -- l'oggetto-colonna non ha `additionalProperties: false`
/// in nessuna delle due versioni, quindi le chiavi in piu' sono ammesse -- e
/// quel campo non ha significato: attribuirglielo lo attribuiremmo noi. Viene
/// percio' ignorato, e nemmeno validato nella forma, perche' non c'e' una forma
/// che quella versione gli imponga.
///
/// Chi ha file scritti con le quattro colonne piatte ha l'opzione che gia'
/// esisteva, `bbox_legacy_by_name`: e' li' che quel riconoscimento vive, ed e'
/// esplicito.
fn covering(
    oggetto: &serde_json::Map<String, serde_json::Value>,
    versione: &'static str,
) -> Result<Option<[Vec<String>; 4]>> {
    let Some(valore) = oggetto.get("covering") else {
        return Ok(None);
    };
    if versione != "1.1.0" {
        return Ok(None);
    }
    let covering = valore.as_object().ok_or_else(|| {
        non_conforme(&PublicMessage::CuratedPair(
            "colonna GeoParquet con un campo che non e' un oggetto",
            "covering",
        ))
    })?;
    // `required: ["bbox"]`: un covering senza `bbox` non e' conforme. La prima
    // stesura lo accettava dicendo che «la specifica non chiude l'insieme delle
    // chiavi di covering» -- vero per le chiavi in piu', falso per quella che
    // manca.
    let riquadro = covering
        .get("bbox")
        .ok_or_else(|| {
            non_conforme(&PublicMessage::CuratedPair(
                "colonna GeoParquet con un `covering` senza il campo obbligatorio",
                "bbox",
            ))
        })?
        .as_object()
        .ok_or_else(|| {
            non_conforme(&PublicMessage::CuratedPair(
                "colonna GeoParquet con un campo che non e' un oggetto",
                "covering.bbox",
            ))
        })?;

    let mut percorsi: [Vec<String>; 4] = [const { Vec::new() }; 4];
    for (posizione, spigolo) in SPIGOLI.into_iter().enumerate() {
        let segmenti = riquadro
            .get(spigolo)
            .ok_or_else(|| {
                non_conforme(&PublicMessage::Curated(
                    "colonna GeoParquet con un `covering.bbox` a cui manca uno spigolo",
                ))
            })?
            .as_array()
            .ok_or_else(|| {
                non_conforme(&PublicMessage::Curated(
                    "colonna GeoParquet con uno spigolo di `covering.bbox` che non e' un percorso",
                ))
            })?;
        if segmenti.len() != 2 {
            return Err(non_conforme(&PublicMessage::CuratedWith(
                "colonna GeoParquet con uno spigolo di `covering.bbox` i cui segmenti non sono",
                NumeroStrutturale::Limite(2),
            )));
        }
        let colonna = segmenti[0]
            .as_str()
            .filter(|testo| !testo.is_empty())
            .ok_or_else(|| {
                non_conforme(&PublicMessage::Curated(
                    "colonna GeoParquet con un `covering.bbox` il cui primo segmento non e' un nome",
                ))
            })?;
        // Il secondo segmento e' `const`: deve essere **quello** spigolo.
        if segmenti[1].as_str() != Some(spigolo) {
            return Err(non_conforme(&PublicMessage::Curated(
                "colonna GeoParquet con un `covering.bbox` il cui secondo segmento non nomina il proprio spigolo",
            )));
        }
        percorsi[posizione] = vec![colonna.to_owned(), spigolo.to_owned()];
    }
    Ok(Some(percorsi))
}

/// Un `bbox` di colonna si puo' usare per il pruning con la sola intersezione?
///
/// No, se un minimo supera il proprio massimo: il riquadro attraversa
/// l'antimeridiano, ed e' un documento **valido** che la semplice intersezione
/// di rettangoli interpreterebbe al contrario -- leggendo **meno** del dovuto,
/// che e' l'unico verso in cui il pruning non puo' sbagliare.
#[must_use]
pub fn interpretabile_per_il_pruning(riquadro: &[f64]) -> bool {
    let meta = riquadro.len() / 2;
    (0..meta).all(|asse| riquadro[asse] <= riquadro[asse + meta])
}

#[cfg(test)]
mod sonde {
    use super::*;
    use plenora_io_model::IoErrorCode;
    use serde_json::json;

    /// La colonna minima che la specifica ammette: i due campi obbligatori.
    ///
    /// `geometry_types` vuoto e' legittimo e vuol dire «non vincolato»: e' il
    /// caso che un writer usa quando non ha ispezionato i dati.
    fn colonna_minima() -> serde_json::Value {
        json!({"encoding": "WKB", "geometry_types": []})
    }

    /// Un documento 1.1.0 con una sola colonna, `geometry`.
    fn documento(colonna: &serde_json::Value) -> String {
        con_versione("1.1.0", colonna)
    }

    fn con_versione(versione: &str, colonna: &serde_json::Value) -> String {
        json!({
            "version": versione,
            "primary_column": "geometry",
            "columns": {"geometry": colonna},
        })
        .to_string()
    }

    /// Una colonna minima con un campo in piu'.
    fn con_campo(campo: &str, valore: serde_json::Value) -> String {
        let mut colonna = colonna_minima();
        colonna[campo] = valore;
        documento(&colonna)
    }

    #[track_caller]
    fn accettato(testo: &str) -> MetadatiGeo {
        analizza(testo, false).expect("il documento e' conforme e supportato")
    }

    #[track_caller]
    fn non_conforme_con(testo: &str) -> PlenoraIoError {
        let errore = analizza(testo, false).expect_err("il documento non e' conforme");
        assert_eq!(
            errore.code,
            IoErrorCode::Format,
            "un documento non conforme e' un errore di formato: {}",
            errore.message
        );
        assert_eq!(errore.driver.as_deref(), Some("geoparquet"));
        errore
    }

    #[track_caller]
    fn non_supportato_con(testo: &str) -> PlenoraIoError {
        let errore = analizza(testo, false).expect_err("la funzionalita' non e' supportata");
        assert_eq!(
            errore.code,
            IoErrorCode::Unsupported,
            "una funzionalita' valida e non implementata non e' un errore di formato: {}",
            errore.message
        );
        errore
    }

    // --- il documento --------------------------------------------------

    #[test]
    fn documento_minimo_e_accettato() {
        let letti = accettato(&documento(&colonna_minima()));
        assert_eq!(letti.versione, "1.1.0");
        assert_eq!(letti.nome_primaria, "geometry");
        assert!(letti.primaria.tipi.is_empty());
        assert!(letti.secondarie.is_empty());
        assert_eq!(letti.primaria.bordi, Bordi::Planari);
        assert_eq!(letti.primaria.crs, Crs::Assente);
        assert!(letti.primaria.covering.is_none());
    }

    #[test]
    fn documento_che_non_e_json_o_non_e_un_oggetto_e_non_conforme() {
        for testo in ["", "{", "non json", "[]", "\"stringa\"", "7", "null"] {
            let errore = non_conforme_con(testo);
            assert!(errore.message.contains("geo"), "{}", errore.message);
        }
    }

    // --- version -------------------------------------------------------

    #[test]
    fn version_dei_due_schemi_ufficiali_e_accettata() {
        for versione in VERSIONI_SUPPORTATE {
            let letti = accettato(&con_versione(versione, &colonna_minima()));
            assert_eq!(letti.versione, versione);
        }
    }

    #[test]
    fn version_di_un_altro_schema_e_valida_e_non_supportata() {
        // La distinzione e' il punto: questi documenti sono corretti, e
        // dichiarano una versione che non leggiamo. Mandare chi legge a
        // correggere un file che non ha niente che non va sarebbe il danno.
        for versione in [
            "0.4.0",
            "1.0.1",
            "1.1.1",
            "1.1",
            "1.2.0",
            "2.0.0",
            "1.0.0-rc1",
        ] {
            let errore = non_supportato_con(&con_versione(versione, &colonna_minima()));
            assert!(
                errore.message.contains("1.0.0 e 1.1.0"),
                "il rifiuto dice fin dove arriviamo: {}",
                errore.message
            );
        }
    }

    #[test]
    fn version_assente_vuota_o_non_stringa_e_non_conforme() {
        for valore in [json!(null), json!(1.1), json!(""), json!(["1.1.0"])] {
            let testo = json!({
                "version": valore,
                "primary_column": "geometry",
                "columns": {"geometry": colonna_minima()},
            })
            .to_string();
            non_conforme_con(&testo);
        }
        let senza = json!({
            "primary_column": "geometry",
            "columns": {"geometry": colonna_minima()},
        })
        .to_string();
        assert!(non_conforme_con(&senza).message.contains("version"));
    }

    // --- primary_column e columns --------------------------------------

    #[test]
    fn primary_column_assente_vuota_o_non_stringa_e_non_conforme() {
        for valore in [json!(null), json!(""), json!(7), json!(["geometry"])] {
            let testo = json!({
                "version": "1.1.0",
                "primary_column": valore,
                "columns": {"geometry": colonna_minima()},
            })
            .to_string();
            non_conforme_con(&testo);
        }
        let senza = json!({
            "version": "1.1.0",
            "columns": {"geometry": colonna_minima()},
        })
        .to_string();
        assert!(non_conforme_con(&senza).message.contains("primary_column"));
    }

    #[test]
    fn columns_assente_o_non_oggetto_e_non_conforme() {
        for valore in [json!(null), json!([]), json!("geometry")] {
            let testo = json!({
                "version": "1.1.0",
                "primary_column": "geometry",
                "columns": valore,
            })
            .to_string();
            non_conforme_con(&testo);
        }
        let senza = json!({"version": "1.1.0", "primary_column": "geometry"}).to_string();
        assert!(non_conforme_con(&senza).message.contains("columns"));
    }

    #[test]
    fn primary_column_che_nomina_una_colonna_presente_e_accettata() {
        // Il verso positivo del campo: la colonna nominata c'e', e il nome che
        // esce e' quello che il documento dichiarava -- non uno indovinato.
        let testo = json!({
            "version": "1.1.0",
            "primary_column": "la_mia_geometria",
            "columns": {
                "la_mia_geometria": colonna_minima(),
                "geometry": colonna_minima(),
            },
        })
        .to_string();
        let letti = accettato(&testo);
        assert_eq!(letti.nome_primaria, "la_mia_geometria");
        assert!(letti.secondarie.contains_key("geometry"));
    }

    #[test]
    fn primary_column_assente_da_columns_e_non_conforme() {
        let testo = json!({
            "version": "1.1.0",
            "primary_column": "geometry",
            "columns": {"altra": colonna_minima()},
        })
        .to_string();
        let errore = non_conforme_con(&testo);
        assert!(
            errore.message.contains("primary_column"),
            "{}",
            errore.message
        );
    }

    #[test]
    fn columns_con_una_colonna_che_non_e_un_oggetto_e_non_conforme() {
        // La forma della colonna e' controllata, e nessuna sonda la esercitava:
        // il gate pretende i due versi **per campo**, e questa e' la forma del
        // contenitore, non di un campo. Una lacuna che l'elenco dei campi non
        // poteva vedere.
        for valore in [json!(7), json!("WKB"), json!([]), json!(null)] {
            let testo = json!({
                "version": "1.1.0",
                "primary_column": "geometry",
                "columns": {"geometry": valore},
            })
            .to_string();
            assert!(non_conforme_con(&testo)
                .message
                .contains("non e' un oggetto"));
        }
    }

    #[test]
    fn columns_con_una_secondaria_malformata_e_non_conforme() {
        // Una colonna che non e' la primaria puo' essere letta da un
        // consumatore diverso: lasciarla passare malformata vorrebbe dire
        // validare solo cio' che usiamo noi.
        let testo = json!({
            "version": "1.1.0",
            "primary_column": "geometry",
            "columns": {
                "geometry": colonna_minima(),
                "altra": {"encoding": "WKB"},
            },
        })
        .to_string();
        assert!(non_conforme_con(&testo).message.contains("geometry_types"));
    }

    #[test]
    fn columns_con_una_secondaria_valida_e_accettato() {
        let buono = json!({
            "version": "1.1.0",
            "primary_column": "geometry",
            "columns": {
                "geometry": colonna_minima(),
                "altra": colonna_minima(),
            },
        })
        .to_string();
        assert_eq!(accettato(&buono).secondarie.len(), 1);
    }

    // --- encoding ------------------------------------------------------

    #[test]
    fn encoding_wkb_e_accettato() {
        accettato(&con_campo("encoding", json!("WKB")));
    }

    #[test]
    fn encoding_nativo_e_valido_e_non_supportato() {
        // Le codifiche native sono valide **da 1.1**: in un documento 1.1.0
        // sono una funzionalita' che non implementiamo...
        for nativa in CODIFICHE_NATIVE {
            let errore = non_supportato_con(&con_versione(
                "1.1.0",
                &json!({"encoding": nativa, "geometry_types": []}),
            ));
            assert!(errore.message.contains("WKB"), "{}", errore.message);
        }
        // ...e in un documento 1.0.0 non sono nemmeno valide, perche' quella
        // versione ammette solo WKB. E' la versione dichiarata a dire quale
        // delle due cose sono, ed e' il servizio che quel campo rende.
        for nativa in CODIFICHE_NATIVE {
            non_conforme_con(&con_versione(
                "1.0.0",
                &json!({"encoding": nativa, "geometry_types": []}),
            ));
        }
    }

    #[test]
    fn encoding_assente_vuoto_o_sconosciuto_e_non_conforme() {
        for valore in [json!(null), json!(""), json!("wkb"), json!("WKT"), json!(7)] {
            non_conforme_con(&con_campo("encoding", valore));
        }
        let senza = documento(&json!({"geometry_types": []}));
        assert!(non_conforme_con(&senza).message.contains("encoding"));
    }

    // --- geometry_types ------------------------------------------------

    #[test]
    fn geometry_types_dall_insieme_chiuso_e_accettato() {
        let letti = accettato(&con_campo(
            "geometry_types",
            json!(["Point", "Point Z", "GeometryCollection", "MultiPolygon Z"]),
        ));
        assert_eq!(letti.primaria.tipi.len(), 4);
        assert_eq!(
            letti.primaria.tipi[0],
            (GeometryType::Point, CoordinateDimensions::Xy)
        );
        assert_eq!(
            letti.primaria.tipi[1],
            (GeometryType::Point, CoordinateDimensions::Xyz)
        );
    }

    #[test]
    fn geometry_types_con_la_misura_m_e_non_conforme() {
        // Il pattern dello schema, in **entrambe** le versioni, e'
        // `^(GeometryCollection|(Multi)?(Point|LineString|Polygon))( Z)?$`:
        // `" M"` e `" ZM"` non esistono in GeoParquet.
        //
        // La prima stesura li ammetteva, e la ragione che ci aveva scritto
        // accanto -- «il nostro writer li emette» -- era il ragionamento
        // sbagliato: il writer emetteva metadati non conformi, e la
        // conclusione giusta era correggere il writer.
        for etichetta in ["Point M", "Point ZM", "LineString M", "MultiPolygon ZM"] {
            non_conforme_con(&con_campo("geometry_types", json!([etichetta])));
        }
    }

    #[test]
    fn geometry_types_fuori_dalla_specifica_e_non_conforme() {
        // Era il difetto: `filter_map` scartava l'etichetta, il contratto della
        // colonna usciva piu' povero, e nulla lo diceva.
        for etichetta in [
            "Punto", "point", "Point X", "POINT", "Point  Z", "Curve", "",
        ] {
            let errore = non_conforme_con(&con_campo("geometry_types", json!([etichetta])));
            assert!(errore.message.contains("tipo geometrico"), "{etichetta}");
        }
    }

    #[test]
    fn geometry_types_assente_o_non_elenco_di_stringhe_e_non_conforme() {
        for valore in [
            json!(null),
            json!("Point"),
            json!({}),
            json!([7]),
            json!([null]),
        ] {
            non_conforme_con(&con_campo("geometry_types", valore));
        }
        let senza = documento(&json!({"encoding": "WKB"}));
        assert!(non_conforme_con(&senza).message.contains("geometry_types"));
    }

    #[test]
    fn geometry_types_ripetuto_e_non_conforme() {
        // `uniqueItems: true`. La prima stesura deduplicava in silenzio: cioe'
        // accettava un documento che lo schema rifiuta, e ne nascondeva la
        // ragione -- chi lo aveva scritto continuava a scriverlo.
        let errore = non_conforme_con(&con_campo("geometry_types", json!(["Point", "Point"])));
        assert!(errore.message.contains("ripetuto"), "{}", errore.message);
    }

    // --- crs -----------------------------------------------------------

    #[test]
    fn crs_assente_nullo_o_oggetto_e_accettato() {
        // Tre stati distinti, e la distinzione e' il rilievo: assente vuol dire
        // CRS84, `null` vuol dire che il CRS non c'e'. La prima stesura li
        // riduceva tutt'e due a «niente», e il driver li trasformava tutt'e due
        // in CRS84 -- mettendo in bocca a chi ha scritto il file
        // un'affermazione che non aveva fatto.
        assert_eq!(
            accettato(&documento(&colonna_minima())).primaria.crs,
            Crs::Assente
        );
        assert_eq!(
            accettato(&con_campo("crs", json!(null))).primaria.crs,
            Crs::Nullo
        );
        // Un PROJJSON **vero**: lo schema referenziato lo pretende completo, e
        // `{"type": ..., "name": ...}` da solo non lo e'. E' la differenza che
        // solo l'autorita' sa fare, e che la nostra prosa non sapeva.
        let letti = accettato(&con_campo(
            "crs",
            json!({
                "type": "GeographicCRS",
                "name": "WGS 84 (CRS84)",
                "datum": {
                    "type": "GeodeticReferenceFrame",
                    "name": "World Geodetic System 1984",
                    "ellipsoid": {
                        "name": "WGS 84",
                        "semi_major_axis": 6_378_137,
                        "inverse_flattening": 298.257_223_563
                    }
                },
                "coordinate_system": {
                    "subtype": "ellipsoidal",
                    "axis": [
                        {"name": "Geodetic longitude", "abbreviation": "Lon", "direction": "east", "unit": "degree"},
                        {"name": "Geodetic latitude", "abbreviation": "Lat", "direction": "north", "unit": "degree"}
                    ]
                }
            }),
        ));
        assert!(matches!(letti.primaria.crs, Crs::Documento(_)));
    }

    #[test]
    fn crs_che_non_e_un_oggetto_e_non_conforme() {
        // Una stringa li' vorrebbe dire che qualcuno ha scritto un WKT dove va
        // un documento PROJJSON.
        for valore in [json!("EPSG:4326"), json!(4326), json!([]), json!(true)] {
            non_conforme_con(&con_campo("crs", valore));
        }
    }

    // --- edges ---------------------------------------------------------

    #[test]
    fn edges_planari_o_assenti_sono_accettati() {
        assert_eq!(
            accettato(&documento(&colonna_minima())).primaria.bordi,
            Bordi::Planari
        );
        assert_eq!(
            accettato(&con_campo("edges", json!("planar")))
                .primaria
                .bordi,
            Bordi::Planari
        );
    }

    #[test]
    fn edges_sferici_sono_validi_e_non_supportati() {
        // Validi, e non implementati: i bordi sferici cambiano il significato
        // geometrico dei dati, non la loro presentazione.
        let errore = non_supportato_con(&con_campo("edges", json!("spherical")));
        assert!(errore.message.contains("planar"), "{}", errore.message);
    }

    #[test]
    fn edges_fuori_dalla_specifica_e_non_conforme() {
        for valore in [json!("toroidal"), json!("Planar"), json!(7), json!(null)] {
            non_conforme_con(&con_campo("edges", valore));
        }
    }

    // --- orientation ---------------------------------------------------

    #[test]
    fn orientation_assente_o_antiorario_e_accettato() {
        assert!(accettato(&documento(&colonna_minima()))
            .primaria
            .orientamento
            .is_none());
        assert_eq!(
            accettato(&con_campo("orientation", json!("counterclockwise")))
                .primaria
                .orientamento,
            Some(Orientamento::Antiorario)
        );
    }

    #[test]
    fn orientation_fuori_dalla_specifica_e_non_conforme() {
        for valore in [json!("clockwise"), json!("ccw"), json!(7), json!(null)] {
            non_conforme_con(&con_campo("orientation", valore));
        }
    }

    // --- bbox ----------------------------------------------------------

    #[test]
    fn bbox_di_quattro_o_sei_numeri_e_accettato() {
        assert!(accettato(&documento(&colonna_minima()))
            .primaria
            .bbox
            .is_none());
        assert_eq!(
            accettato(&con_campo("bbox", json!([0.0, 0.0, 1.0, 1.0])))
                .primaria
                .bbox,
            Some(vec![0.0, 0.0, 1.0, 1.0])
        );
        assert_eq!(
            accettato(&con_campo("bbox", json!([0, 0, 0, 1, 1, 1])))
                .primaria
                .bbox
                .map(|b| b.len()),
            Some(6)
        );
    }

    #[test]
    fn bbox_di_lunghezza_sbagliata_o_non_numerico_e_non_conforme() {
        for valore in [
            json!([]),
            json!([0, 0, 1]),
            json!([0, 0, 1, 1, 1]),
            json!([0, 0, 0, 1, 1, 1, 1]),
            json!("0,0,1,1"),
            json!([0, 0, 1, "1"]),
            json!([0, 0, 1, null]),
        ] {
            non_conforme_con(&con_campo("bbox", valore));
        }
    }

    #[test]
    fn bbox_con_un_minimo_oltre_il_proprio_massimo_e_accettato() {
        // Lo schema non lo vieta, e un riquadro che attraversa l'antimeridiano
        // si scrive proprio cosi'. La prima stesura lo rifiutava: dichiarava
        // non conforme un documento che la specifica accetta, cioe' divergeva
        // dall'autorita' nel verso del rifiuto -- che e' comunque divergere.
        for valore in [
            json!([1.0, 0.0, 0.0, 1.0]),
            json!([0.0, 1.0, 1.0, 0.0]),
            json!([0, 0, 5, 1, 1, 1]),
        ] {
            accettato(&con_campo("bbox", valore));
        }
        accettato(&con_campo("bbox", json!([1.0, 1.0, 1.0, 1.0])));
    }

    #[test]
    fn bbox_invertito_non_interpretabile_per_il_pruning_e_accettato() {
        // Cio' che quel riquadro non e' e' **usabile** con la semplice
        // intersezione di rettangoli: chi lo usasse leggerebbe meno del dovuto.
        // Il file resta valido e il pruning si spegne, che e' il verso in cui
        // questo driver sbaglia per contratto.
        assert!(interpretabile_per_il_pruning(&[0.0, 0.0, 1.0, 1.0]));
        assert!(interpretabile_per_il_pruning(&[1.0, 1.0, 1.0, 1.0]));
        assert!(!interpretabile_per_il_pruning(&[1.0, 0.0, 0.0, 1.0]));
        assert!(!interpretabile_per_il_pruning(&[
            0.0, 0.0, 5.0, 1.0, 1.0, 1.0
        ]));
    }

    #[test]
    fn bbox_non_numerico_e_non_conforme_e_non_finito_non_e_esprimibile() {
        // Il `bbox` e l'`epoch` non filtrano `is_finite`, e la ragione e'
        // questa: JSON non ha `NaN` ne' infinito, e `serde_json` rifiuta da se'
        // i letterali che traboccherebbero in `f64`. Un filtro li' sarebbe una
        // guardia che non puo' scattare, e una guardia che non puo' scattare
        // non e' una difesa: e' una riga che nessuno potra' mai provare.
        //
        // La sonda fissa il fatto invece di fidarsene. Se un giorno la
        // dipendenza accettasse `1e400`, il documento arriverebbe alla
        // validazione con un infinito dentro e questa sonda sarebbe rossa --
        // che e' il momento giusto per rimettere il filtro.
        for letterale in ["1e400", "-1e400", "1e-400"] {
            let testo = format!(
                r#"{{"version":"1.1.0","primary_column":"geometry","columns":{{"geometry":{{"encoding":"WKB","geometry_types":[],"bbox":[0,0,{letterale},1]}}}}}}"#
            );
            let esito: std::result::Result<serde_json::Value, _> = serde_json::from_str(&testo);
            if let Ok(documento) = esito {
                // `1e-400` puo' arrotondare a zero invece di traboccare: e'
                // finito, e va bene. Cio' che non deve mai accadere e' un
                // valore non finito che arriva alla validazione.
                let letto = documento["columns"]["geometry"]["bbox"][2].as_f64();
                assert!(
                    letto.is_some_and(f64::is_finite),
                    "{letterale} e' arrivato non finito: il filtro va rimesso"
                );
            }
        }
        // E un `bbox` con un valore che numero non e' resta rifiutato.
        assert!(non_conforme_con(&con_campo("bbox", json!([0, 0, "1", 1])))
            .message
            .contains("bbox"));
    }

    // --- epoch ---------------------------------------------------------

    #[test]
    fn epoch_assente_o_numerico_e_accettato() {
        assert!(accettato(&documento(&colonna_minima()))
            .primaria
            .epoch
            .is_none());
        assert_eq!(
            accettato(&con_campo("epoch", json!(2021.5))).primaria.epoch,
            Some(2021.5)
        );
    }

    #[test]
    fn epoch_che_non_e_un_numero_e_non_conforme() {
        for valore in [json!("2021.5"), json!(null), json!([2021]), json!({})] {
            non_conforme_con(&con_campo("epoch", valore));
        }
    }

    // --- covering ------------------------------------------------------

    /// Il covering nella forma che lo schema 1.1.0 designa: due segmenti per
    /// spigolo, il secondo uguale al nome dello spigolo.
    fn covering_conforme() -> serde_json::Value {
        json!({"bbox": {
            "xmin": ["bbox", "xmin"],
            "ymin": ["bbox", "ymin"],
            "xmax": ["bbox", "xmax"],
            "ymax": ["bbox", "ymax"],
        }})
    }

    /// La forma che questo repository emetteva prima di S10: un segmento solo.
    fn covering_piatto() -> serde_json::Value {
        json!({"bbox": {
            "xmin": ["_bbox_minx"],
            "ymin": ["_bbox_miny"],
            "xmax": ["_bbox_maxx"],
            "ymax": ["_bbox_maxy"],
        }})
    }

    #[test]
    fn covering_di_due_segmenti_e_accettato() {
        let letti = accettato(&con_campo("covering", covering_conforme()));
        assert_eq!(
            letti.primaria.covering,
            Some([
                vec!["bbox".to_owned(), "xmin".to_owned()],
                vec!["bbox".to_owned(), "ymin".to_owned()],
                vec!["bbox".to_owned(), "xmax".to_owned()],
                vec!["bbox".to_owned(), "ymax".to_owned()],
            ])
        );
    }

    #[test]
    fn covering_di_un_solo_segmento_e_non_conforme() {
        // E' la forma che questo writer emetteva, dichiarando 1.1.0: quei file
        // **non erano** GeoParquet 1.1 validi. Lo schema vuole
        // `minItems: 2, maxItems: 2`, e la prima stesura di questo modulo
        // chiamava «utilizzabile» proprio la forma sbagliata e «valida e
        // inutilizzabile» quella giusta -- esattamente al contrario.
        let errore = non_conforme_con(&con_campo("covering", covering_piatto()));
        assert!(errore.message.contains("segmenti"), "{}", errore.message);
    }

    #[test]
    fn covering_col_secondo_segmento_sbagliato_e_non_conforme() {
        // Il secondo segmento e' un `const`: deve nominare **quello** spigolo.
        // Uno scambio qui darebbe al pruning le colonne incrociate.
        let scambiato = json!({"bbox": {
            "xmin": ["bbox", "ymin"],
            "ymin": ["bbox", "xmin"],
            "xmax": ["bbox", "xmax"],
            "ymax": ["bbox", "ymax"],
        }});
        let errore = non_conforme_con(&con_campo("covering", scambiato));
        assert!(errore.message.contains("spigolo"), "{}", errore.message);
    }

    #[test]
    fn covering_senza_bbox_e_non_conforme() {
        // `required: ["bbox"]`. La prima stesura lo accettava dicendo che «la
        // specifica non chiude l'insieme delle chiavi di covering»: vero per le
        // chiavi in piu', falso per quella che manca.
        let errore = non_conforme_con(&con_campo("covering", json!({"altro": {}})));
        assert!(errore.message.contains("bbox"), "{}", errore.message);
    }

    #[test]
    fn covering_malformato_e_non_conforme() {
        let mancante = json!({"bbox": {
            "xmin": ["bbox", "xmin"],
            "ymin": ["bbox", "ymin"],
            "xmax": ["bbox", "xmax"],
        }});
        assert!(non_conforme_con(&con_campo("covering", mancante))
            .message
            .contains("spigolo"));

        for valore in [
            json!("bbox"),
            json!([]),
            json!({"bbox": "niente"}),
            json!({"bbox": {"xmin": "a", "ymin": ["b", "ymin"], "xmax": ["c", "xmax"], "ymax": ["d", "ymax"]}}),
            json!({"bbox": {"xmin": [], "ymin": ["b", "ymin"], "xmax": ["c", "xmax"], "ymax": ["d", "ymax"]}}),
            json!({"bbox": {"xmin": [7, "xmin"], "ymin": ["b", "ymin"], "xmax": ["c", "xmax"], "ymax": ["d", "ymax"]}}),
            json!({"bbox": {"xmin": ["", "xmin"], "ymin": ["b", "ymin"], "xmax": ["c", "xmax"], "ymax": ["d", "ymax"]}}),
        ] {
            non_conforme_con(&con_campo("covering", valore));
        }
    }

    #[test]
    fn covering_in_un_documento_1_0_0_ignorato_e_accettato() {
        // `covering` non esiste nello schema 1.0.0, e un documento 1.0.0 che lo
        // porta resta **valido**: l'oggetto-colonna non ha
        // `additionalProperties: false` in nessuna delle due versioni, quindi le
        // chiavi in piu' sono ammesse.
        //
        // Non ha pero' significato, e attribuirglielo lo attribuiremmo noi:
        // viene ignorato, e nemmeno validato nella forma -- non c'e' una forma
        // che quella versione gli imponga. Chi ha file scritti con le quattro
        // colonne piatte usa `bbox_legacy_by_name`, che e' esplicito.
        for forma in [covering_conforme(), covering_piatto(), json!({"bbox": {}})] {
            let mut colonna = colonna_minima();
            colonna["covering"] = forma;
            let letti = accettato(&con_versione("1.0.0", &colonna));
            assert_eq!(letti.versione, "1.0.0");
            assert!(letti.primaria.covering.is_none());
        }
    }

    // --- la via storica, stretta e spenta per default ---------------------

    /// Il `crs` che questo repository scriveva fino a S10: non e' PROJJSON.
    fn crs_storico() -> serde_json::Value {
        json!({"id": {"authority": "EPSG", "code": 4326}})
    }

    fn documento_storico() -> String {
        let mut colonna = colonna_minima();
        colonna["crs"] = crs_storico();
        con_versione("1.0.0", &colonna)
    }

    #[test]
    fn crs_storico_senza_opt_in_e_non_conforme() {
        // Senza opzione resta `Format`, ed e' il default: la via di
        // compatibilita' che si accende da sola non e' una via, e' il
        // comportamento normale.
        let errore = analizza(&documento_storico(), false).expect_err("non conforme");
        assert_eq!(errore.code, plenora_io_model::IoErrorCode::Format);
    }

    #[test]
    fn crs_storico_con_opt_in_conserva_l_identificatore_ed_e_accettato() {
        let letti = analizza(&documento_storico(), true).expect("accettato per compatibilita'");
        assert_eq!(letti.conformita, Conformita::CrsStoricoSoloIdentificatore);
        assert_eq!(
            letti.primaria.crs,
            Crs::StoricoSoloIdentificatore("EPSG:4326".to_owned())
        );
    }

    #[test]
    fn crs_storico_in_un_documento_1_1_0_e_non_conforme() {
        // Vale solo per 1.0.0, che e' la versione che questo repository
        // scriveva: un 1.1.0 con quel `crs` non e' un nostro file storico, e
        // non c'e' ragione di tollerarlo.
        let mut colonna = colonna_minima();
        colonna["crs"] = crs_storico();
        let errore = analizza(&con_versione("1.1.0", &colonna), true).expect_err("non conforme");
        assert_eq!(errore.code, plenora_io_model::IoErrorCode::Format);
    }

    #[test]
    fn crs_storico_di_forma_diversa_e_non_conforme() {
        // Non e' un permesso di accettare PROJJSON invalidi: e' il permesso di
        // accettare **quella** forma. Se fosse largo, sarebbe un buco travestito
        // da cortesia.
        for finto in [
            json!({"id": {"authority": "EPSG", "code": 4326}, "type": "GeographicCRS"}),
            json!({"id": {"authority": "", "code": 4326}}),
            json!({"id": {"authority": "EPSG"}}),
            json!({"id": {"authority": "EPSG", "code": 4326, "extra": 1}}),
            json!({"id": {"authority": "EPSG", "code": [4326]}}),
            json!({"identifier": {"authority": "EPSG", "code": 4326}}),
            json!({"type": "GeographicCRS", "name": "incompleto"}),
        ] {
            let mut colonna = colonna_minima();
            colonna["crs"] = finto.clone();
            let errore =
                analizza(&con_versione("1.0.0", &colonna), true).expect_err("{finto} non passa");
            assert_eq!(
                errore.code,
                plenora_io_model::IoErrorCode::Format,
                "{finto}"
            );
        }
    }

    #[test]
    fn crs_storico_con_altri_difetti_e_non_conforme() {
        // Tolti i `crs` storici il documento deve essere conforme: l'opzione
        // tollera esattamente cio' che dichiara di tollerare.
        let mut colonna = colonna_minima();
        colonna["crs"] = crs_storico();
        colonna["geometry_types"] = json!(["Point M"]);
        let errore = analizza(&con_versione("1.0.0", &colonna), true).expect_err("non conforme");
        assert_eq!(errore.code, plenora_io_model::IoErrorCode::Format);
    }

    #[test]
    fn documento_conforme_con_opt_in_acceso_e_accettato() {
        // L'opzione non cambia il giudizio su cio' che e' gia' conforme.
        let letti = analizza(&documento(&colonna_minima()), true).expect("conforme");
        assert_eq!(letti.conformita, Conformita::Conforme);
    }

    // --- l'insieme chiuso, letto da fuori --------------------------------

    #[test]
    fn geometry_types_i_quattordici_nomi_sono_accettati() {
        let mut quante = 0;
        for (nome, tipo) in NOMI_DI_TIPO {
            for (suffisso, dimensioni) in SUFFISSI {
                let etichetta = format!("{nome}{suffisso}");
                assert_eq!(
                    etichetta_di_tipo(&etichetta),
                    Some((tipo, dimensioni)),
                    "{etichetta}"
                );
                quante += 1;
            }
        }
        assert_eq!(quante, 14, "sette nomi per due dimensionalita': XY e Z");
        for storta in ["", " Z", "Point Q", "pointz", "Point-Z"] {
            assert!(etichetta_di_tipo(storta).is_none(), "{storta}");
        }
    }
}
