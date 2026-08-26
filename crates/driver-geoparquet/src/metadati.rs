//! I metadati `geo` di GeoParquet, validati per intero.
//!
//! # Che cosa c'era prima
//!
//! `serde_json::from_str(&raw).ok()`. Un metadato `geo` malformato diventava
//! `None`, cioe' **indistinguibile da un file che `geo` non ce l'ha**: il
//! driver passava a indovinare la colonna geometria fra `geometry`, `geom` e
//! `wkb`, e la colonna indovinata poteva non essere quella che
//! `primary_column` dichiarava. Un GeoParquet corrotto veniva letto come
//! Parquet semplice, e nessuno lo sapeva.
//!
//! Dei campi del documento ne venivano consultati cinque -- `primary_column`,
//! `columns`, `crs`, `geometry_types`, `covering.bbox`. `version`, `encoding`,
//! `edges`, `orientation`, `epoch` e il `bbox` di colonna non venivano guardati
//! affatto. Le due conseguenze che pesavano:
//!
//! * una colonna con `encoding` nativo `GeoArrow` -- valida in GeoParquet 1.1
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
/// Non `1.0.x` e `1.1.x`: gli schemi ufficiali fissano questi due valori esatti.
pub const VERSIONI_SUPPORTATE: [&str; 2] = ["1.0.0", "1.1.0"];

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

/// I suffissi di dimensionalita', **in ordine di priorita'**.
///
/// `" ZM"` prima di `" Z"` e `" M"`: la catena e' ordinata, e l'ordine e' il
/// contratto. `" M"` e `" ZM"` sono ammessi perche' il nostro writer li emette
/// quando i dati hanno quelle dimensioni -- rifiutarli renderebbe illeggibili i
/// file che abbiamo scritto noi.
const SUFFISSI: [(&str, CoordinateDimensions); 4] = [
    (" ZM", CoordinateDimensions::Xyzm),
    (" Z", CoordinateDimensions::Xyz),
    (" M", CoordinateDimensions::Xym),
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
    /// Il `crs` dichiarato, se presente e non nullo.
    pub crs: Option<serde_json::Value>,
    pub bordi: Bordi,
    pub orientamento: Option<Orientamento>,
    pub bbox: Option<Vec<f64>>,
    pub epoch: Option<f64>,
    /// I quattro nomi di colonna del `covering.bbox`, se dichiarato e
    /// **utilizzabile** per il pruning.
    ///
    /// Un covering ben formato ma con percorsi annidati e' valido e non
    /// utilizzabile: qui vale `None` e non e' un errore. La differenza con
    /// `encoding` e `edges` e' netta e sta nel danno -- quelli cambiano il
    /// significato dei dati, questo toglie soltanto un'ottimizzazione, e il
    /// pruning di questo driver e' fail-open per contratto: una statistica che
    /// non si sa usare fa leggere di piu', mai di meno.
    pub covering: Option<[String; 4]>,
}

/// Analizza il documento `geo` di un file, per intero.
///
/// # Errors
///
/// `Format` se il documento non rispetta la specifica, `Unsupported` se chiede
/// una versione o una semantica che questa libreria non implementa.
pub fn analizza(grezzo: &str) -> Result<MetadatiGeo> {
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

    let versione = versione(oggetto)?;
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
    let primaria = colonna(grezza_primaria, versione)?;

    let mut secondarie = BTreeMap::new();
    for (nome, grezza) in colonne_grezze {
        if nome == &colonna_primaria {
            continue;
        }
        secondarie.insert(nome.clone(), colonna(grezza, versione)?);
    }

    Ok(MetadatiGeo {
        versione,
        nome_primaria: colonna_primaria,
        primaria,
        secondarie,
    })
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
fn colonna(grezza: &serde_json::Value, versione: &'static str) -> Result<ColonnaGeo> {
    let oggetto = grezza.as_object().ok_or_else(|| {
        non_conforme(&PublicMessage::Curated(
            "colonna GeoParquet che non e' un oggetto",
        ))
    })?;

    encoding(oggetto, versione)?;
    let tipi = geometry_types(oggetto)?;
    let crs = crs(oggetto)?;
    let bordi = bordi(oggetto)?;
    let orientamento = orientamento(oggetto)?;
    let bbox = bbox(oggetto)?;
    let epoch = epoch(oggetto)?;
    let covering = covering(oggetto)?;

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
        if !tipi.contains(&letto) {
            tipi.push(letto);
        }
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

/// Il `crs` della colonna.
///
/// Assente o `null` restano cio' che erano: la semantica del CRS non cambia in
/// questo lotto, che valida la forma. Presente e non nullo deve essere un
/// oggetto -- PROJJSON e' un oggetto, e una stringa li' vorrebbe dire che
/// qualcuno ha scritto un WKT dove va un documento.
fn crs(oggetto: &serde_json::Map<String, serde_json::Value>) -> Result<Option<serde_json::Value>> {
    match oggetto.get("crs") {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(valore) if valore.is_object() => Ok(Some(valore.clone())),
        Some(_) => Err(non_conforme(&PublicMessage::Curated(
            "colonna GeoParquet con un `crs` che non e' un oggetto PROJJSON",
        ))),
    }
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

    // Un `bbox` in cui il minimo supera il massimo non descrive un rettangolo:
    // descrive due numeri scambiati. Il pruning che lo credesse leggerebbe
    // **meno** del dovuto, ed e' l'unico verso in cui il pruning non puo'
    // sbagliare.
    let meta = numeri.len() / 2;
    for asse in 0..meta {
        if numeri[asse] > numeri[asse + meta] {
            return Err(non_conforme(&PublicMessage::Curated(
                "colonna GeoParquet con un `bbox` in cui un minimo supera il proprio massimo",
            )));
        }
    }
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

/// Il `covering.bbox`, validato nella forma e usato solo se utilizzabile.
///
/// Un covering malformato e' un documento che non rispetta la specifica, e
/// quindi un errore. Un covering ben formato con percorsi annidati e' valido:
/// non e' un errore, e semplicemente non serve al pruning, che qui lavora sui
/// campi radice. La differenza sta nel danno -- `encoding` ed `edges` cambiano
/// il significato dei dati, un covering inutilizzabile toglie
/// un'ottimizzazione, e il pruning di questo driver e' fail-open per contratto.
fn covering(oggetto: &serde_json::Map<String, serde_json::Value>) -> Result<Option<[String; 4]>> {
    let Some(valore) = oggetto.get("covering") else {
        return Ok(None);
    };
    let covering = valore.as_object().ok_or_else(|| {
        non_conforme(&PublicMessage::CuratedPair(
            "colonna GeoParquet con un campo che non e' un oggetto",
            "covering",
        ))
    })?;
    let Some(riquadro) = covering.get("bbox") else {
        // `covering` senza `bbox` non dichiara niente che questo driver usi, e
        // la specifica non chiude l'insieme delle sue chiavi.
        return Ok(None);
    };
    let riquadro = riquadro.as_object().ok_or_else(|| {
        non_conforme(&PublicMessage::CuratedPair(
            "colonna GeoParquet con un campo che non e' un oggetto",
            "covering.bbox",
        ))
    })?;

    let mut nomi = Vec::with_capacity(SPIGOLI.len());
    for spigolo in SPIGOLI {
        let percorso = riquadro
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
        if percorso.is_empty() {
            return Err(non_conforme(&PublicMessage::Curated(
                "colonna GeoParquet con uno spigolo di `covering.bbox` dal percorso vuoto",
            )));
        }
        let mut segmenti = Vec::with_capacity(percorso.len());
        for segmento in percorso {
            let letto = segmento
                .as_str()
                .filter(|testo| !testo.is_empty())
                .ok_or_else(|| {
                    non_conforme(&PublicMessage::Curated(
                        "colonna GeoParquet con un segmento di `covering.bbox` che non e' un nome",
                    ))
                })?;
            segmenti.push(letto.to_owned());
        }
        nomi.push(segmenti);
    }

    // Ben formato ma annidato: valido, e non utilizzabile per il pruning.
    if nomi.iter().any(|segmenti| segmenti.len() != 1) {
        return Ok(None);
    }
    let mut piatti = nomi
        .into_iter()
        .filter_map(|segmenti| segmenti.into_iter().next());
    // I quattro nomi si prendono insieme, e non uno per volta con un ripiego a
    // testa: `unwrap_or_default()` avrebbe messo una stringa vuota al posto di
    // un nome mancante, cioe' avrebbe dato al pruning una colonna che non
    // esiste. Qui o ci sono tutti e quattro, o il covering resta spento -- che
    // e' il verso in cui questo driver sbaglia per contratto.
    match (piatti.next(), piatti.next(), piatti.next(), piatti.next()) {
        (Some(xmin), Some(ymin), Some(xmax), Some(ymax)) => Ok(Some([xmin, ymin, xmax, ymax])),
        _ => Ok(None),
    }
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
        analizza(testo).expect("il documento e' conforme e supportato")
    }

    #[track_caller]
    fn non_conforme_con(testo: &str) -> PlenoraIoError {
        let errore = analizza(testo).expect_err("il documento non e' conforme");
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
        let errore = analizza(testo).expect_err("la funzionalita' non e' supportata");
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
        assert!(letti.primaria.crs.is_none());
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
            json!([
                "Point",
                "Point Z",
                "Point M",
                "Point ZM",
                "GeometryCollection"
            ]),
        ));
        assert_eq!(letti.primaria.tipi.len(), 5);
        assert_eq!(
            letti.primaria.tipi[0],
            (GeometryType::Point, CoordinateDimensions::Xy)
        );
        assert_eq!(
            letti.primaria.tipi[3],
            (GeometryType::Point, CoordinateDimensions::Xyzm)
        );
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
    fn geometry_types_ripetuto_e_accettato() {
        // Accettato, e contato una volta sola: il contratto della colonna non
        // deve ereditare le ripetizioni del documento.
        let letti = accettato(&con_campo("geometry_types", json!(["Point", "Point"])));
        assert_eq!(letti.primaria.tipi.len(), 1);
    }

    // --- crs -----------------------------------------------------------

    #[test]
    fn crs_assente_nullo_o_oggetto_e_accettato() {
        assert!(accettato(&documento(&colonna_minima()))
            .primaria
            .crs
            .is_none());
        assert!(accettato(&con_campo("crs", json!(null)))
            .primaria
            .crs
            .is_none());
        let letti = accettato(&con_campo(
            "crs",
            json!({"id": {"authority": "EPSG", "code": 4326}}),
        ));
        assert!(letti.primaria.crs.is_some());
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
    fn bbox_con_un_minimo_oltre_il_proprio_massimo_e_non_conforme() {
        // Non descrive un rettangolo: descrive due numeri scambiati. Un
        // pruning che gli credesse leggerebbe **meno** del dovuto, ed e'
        // l'unico verso in cui il pruning non puo' sbagliare.
        for valore in [
            json!([1.0, 0.0, 0.0, 1.0]),
            json!([0.0, 1.0, 1.0, 0.0]),
            json!([0, 0, 5, 1, 1, 1]),
        ] {
            let errore = non_conforme_con(&con_campo("bbox", valore));
            assert!(errore.message.contains("minimo"), "{}", errore.message);
        }
        // Il confine: minimo uguale al massimo e' un rettangolo degenere, ed e'
        // legittimo -- una sola geometria puntuale lo produce.
        accettato(&con_campo("bbox", json!([1.0, 1.0, 1.0, 1.0])));
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

    fn covering_piatto() -> serde_json::Value {
        json!({"bbox": {
            "xmin": ["_bbox_minx"],
            "ymin": ["_bbox_miny"],
            "xmax": ["_bbox_maxx"],
            "ymax": ["_bbox_maxy"],
        }})
    }

    #[test]
    fn covering_piatto_utilizzabile_e_accettato() {
        let letti = accettato(&con_campo("covering", covering_piatto()));
        assert_eq!(
            letti.primaria.covering,
            Some([
                "_bbox_minx".to_owned(),
                "_bbox_miny".to_owned(),
                "_bbox_maxx".to_owned(),
                "_bbox_maxy".to_owned(),
            ])
        );
    }

    #[test]
    fn covering_annidato_inutilizzabile_e_accettato() {
        // La distinzione che questo modulo tiene: `encoding` ed `edges`
        // cambiano il significato dei dati e fermano il file; un covering che
        // non sappiamo usare toglie solo un'ottimizzazione, e il pruning di
        // questo driver e' fail-open per contratto.
        let annidato = json!({"bbox": {
            "xmin": ["bbox", "xmin"],
            "ymin": ["bbox", "ymin"],
            "xmax": ["bbox", "xmax"],
            "ymax": ["bbox", "ymax"],
        }});
        let letti = accettato(&con_campo("covering", annidato));
        assert!(letti.primaria.covering.is_none());
    }

    #[test]
    fn covering_malformato_e_non_conforme() {
        let mancante = json!({"bbox": {"xmin": ["a"], "ymin": ["b"], "xmax": ["c"]}});
        assert!(non_conforme_con(&con_campo("covering", mancante))
            .message
            .contains("spigolo"));

        for valore in [
            json!("bbox"),
            json!([]),
            json!({"bbox": "niente"}),
            json!({"bbox": {"xmin": "a", "ymin": ["b"], "xmax": ["c"], "ymax": ["d"]}}),
            json!({"bbox": {"xmin": [], "ymin": ["b"], "xmax": ["c"], "ymax": ["d"]}}),
            json!({"bbox": {"xmin": [7], "ymin": ["b"], "xmax": ["c"], "ymax": ["d"]}}),
            json!({"bbox": {"xmin": [""], "ymin": ["b"], "xmax": ["c"], "ymax": ["d"]}}),
        ] {
            non_conforme_con(&con_campo("covering", valore));
        }
    }

    #[test]
    fn covering_senza_bbox_e_accettato() {
        // La specifica non chiude l'insieme delle chiavi di `covering`: una
        // chiave che non usiamo non e' un file rotto.
        let letti = accettato(&con_campo("covering", json!({"altro": {}})));
        assert!(letti.primaria.covering.is_none());
    }

    #[test]
    fn covering_in_un_documento_1_0_0_e_accettato() {
        // `covering` esiste da 1.1, e i nostri file scritti prima di questo
        // lotto dichiarano 1.0.0 **e** portano il covering. Rifiutarli
        // renderebbe illeggibile cio' che abbiamo scritto noi, e la specifica
        // 1.0 non chiude l'insieme delle chiavi.
        let mut colonna = colonna_minima();
        colonna["covering"] = covering_piatto();
        let letti = accettato(&con_versione("1.0.0", &colonna));
        assert_eq!(letti.versione, "1.0.0");
        assert!(letti.primaria.covering.is_some());
    }

    // --- l'insieme chiuso, letto da fuori --------------------------------

    #[test]
    fn geometry_types_i_ventotto_nomi_sono_accettati() {
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
        assert_eq!(quante, 28, "sette nomi per quattro dimensionalita'");
        for storta in ["", " Z", "Point Q", "pointz", "Point-Z"] {
            assert!(etichetta_di_tipo(storta).is_none(), "{storta}");
        }
    }
}
