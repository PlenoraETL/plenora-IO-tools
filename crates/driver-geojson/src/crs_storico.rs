//! Il membro `crs` di `GeoJSON` 2008.
//!
//! RFC 7946 ha tolto il membro e fissato WGS84 con la longitudine prima; i file
//! scritti prima lo portano ancora, su `FeatureCollection`, `Feature` o
//! geometria. Il driver lo ignorava ed etichettava ogni documento `OGC:CRS84`:
//! un file che dichiarava `EPSG:3003` usciva in WGS84, cioe' con un CRS che il
//! file nega. `PRODUCT.md` dice «WGS84 fisso: un CRS diverso e' rifiutato», e
//! qui la lettura lo fa.
//!
//! E' ammesso soltanto un CRS con nome (`{"type":"name","properties":{"name":
//! ...}}`) che nomina CRS84 o EPSG:4326, nelle forme brevi, URN e URI
//! dell'elenco chiuso di [`nomina_wgs84`]. Ogni altra cosa si rifiuta: un altro
//! nome, un CRS per link (il driver non lo segue), `null` (che in `GeoJSON` 2008
//! vuol dire «nessun CRS», non WGS84), una forma malformata, una chiave
//! ripetuta. EPSG:4326 si accetta come CRS84 perche' `GeoJSON` 2008 fissa l'ordine
//! delle coordinate (x, y), cioe' longitudine prima, qualunque CRS dichiari: e'
//! lo stesso datum e lo stesso ordine memorizzato.

use serde::de::{DeserializeSeed, Deserializer, IgnoredAny, MapAccess, SeqAccess, Visitor};

use plenora_io_model::{PlenoraIoError, PublicMessage};

/// L'errore di un membro `crs` che non nomina WGS84.
pub fn rifiuto() -> PlenoraIoError {
    PlenoraIoError::crs_redatto(&PublicMessage::Curated(
        "GeoJSON: il membro storico `crs` non nomina WGS84 (CRS84 o EPSG:4326)",
    ))
}

/// Il nome di un CRS `GeoJSON` 2008 che e' CRS84 o EPSG:4326.
///
/// Elenco chiuso, ASCII senza maiuscole: le forme brevi, gli URN OGC (con o
/// senza versione) e gli URI `opengis.net`. Un nome fuori elenco non e'
/// «probabilmente WGS84»: e' un rifiuto.
#[must_use]
pub fn nomina_wgs84(nome: &str) -> bool {
    let nome = nome.to_ascii_lowercase();
    if matches!(
        nome.as_str(),
        "ogc:crs84"
            | "epsg:4326"
            | "http://www.opengis.net/def/crs/ogc/1.3/crs84"
            | "http://www.opengis.net/def/crs/epsg/0/4326"
    ) {
        return true;
    }
    // `urn:ogc:def:crs:<autorita'>:<versione>:<codice>`, versione vuota o fatta
    // di cifre e punti.
    let Some(resto) = nome.strip_prefix("urn:ogc:def:crs:") else {
        return false;
    };
    let mut parti = resto.split(':');
    let (Some(autorita), Some(versione), Some(codice), None) =
        (parti.next(), parti.next(), parti.next(), parti.next())
    else {
        return false;
    };
    let versione_valida = versione.chars().all(|c| c.is_ascii_digit() || c == '.');
    versione_valida && matches!((autorita, codice), ("ogc", "crs84") | ("epsg", "4326"))
}

/// Il seme del valore di un membro `crs`: `true` se nomina WGS84.
///
/// Non fallisce sulla forma: una forma inattesa e' un `false`, e il chiamante
/// lo trasforma nel rifiuto `crs`. Consuma sempre tutto il valore, cosi' che
/// il parser resti allineato fino al rifiuto.
pub struct CrsStorico;

impl<'de> DeserializeSeed<'de> for CrsStorico {
    type Value = bool;
    fn deserialize<D: Deserializer<'de>>(self, d: D) -> Result<bool, D::Error> {
        d.deserialize_any(VisitaCrs)
    }
}

struct VisitaCrs;

/// I valori scalari e le sequenze che una forma non ammette: si consumano, e
/// il visitatore rende `$esito`.
macro_rules! scarta_il_resto {
    ($valore:ty, $esito:expr) => {
        fn visit_bool<E: serde::de::Error>(self, _: bool) -> Result<$valore, E> {
            Ok($esito)
        }
        fn visit_i64<E: serde::de::Error>(self, _: i64) -> Result<$valore, E> {
            Ok($esito)
        }
        fn visit_u64<E: serde::de::Error>(self, _: u64) -> Result<$valore, E> {
            Ok($esito)
        }
        fn visit_f64<E: serde::de::Error>(self, _: f64) -> Result<$valore, E> {
            Ok($esito)
        }
        fn visit_unit<E: serde::de::Error>(self) -> Result<$valore, E> {
            Ok($esito)
        }
        fn visit_none<E: serde::de::Error>(self) -> Result<$valore, E> {
            Ok($esito)
        }
        fn visit_some<D: Deserializer<'de>>(self, d: D) -> Result<$valore, D::Error> {
            d.deserialize_any(self)
        }
        fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<$valore, A::Error> {
            while seq.next_element::<IgnoredAny>()?.is_some() {}
            Ok($esito)
        }
    };
}

/// Una stringa, oppure `None` per qualunque altro valore, consumato senza
/// materializzarlo.
struct Testo;

impl<'de> DeserializeSeed<'de> for Testo {
    type Value = Option<String>;
    fn deserialize<D: Deserializer<'de>>(self, d: D) -> Result<Option<String>, D::Error> {
        d.deserialize_any(VisitaTesto)
    }
}

struct VisitaTesto;

impl<'de> Visitor<'de> for VisitaTesto {
    type Value = Option<String>;
    fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str("una stringa")
    }
    scarta_il_resto!(Option<String>, None);
    fn visit_str<E: serde::de::Error>(self, valore: &str) -> Result<Option<String>, E> {
        Ok(Some(valore.to_owned()))
    }
    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Option<String>, A::Error> {
        while map.next_entry::<IgnoredAny, IgnoredAny>()?.is_some() {}
        Ok(None)
    }
}

impl<'de> Visitor<'de> for VisitaCrs {
    type Value = bool;
    fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str("il membro crs di GeoJSON 2008")
    }
    scarta_il_resto!(bool, false);
    fn visit_str<E: serde::de::Error>(self, _: &str) -> Result<bool, E> {
        Ok(false)
    }
    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<bool, A::Error> {
        let mut tipo_nome: Option<bool> = None;
        let mut nome: Option<bool> = None;
        let mut ammesso = true;
        while let Some(chiave) = map.next_key::<String>()? {
            match chiave.as_str() {
                "type" if tipo_nome.is_none() => {
                    tipo_nome = Some(map.next_value_seed(Testo)?.as_deref() == Some("name"));
                }
                "properties" if nome.is_none() => {
                    nome = Some(map.next_value_seed(ProprietaCrs)?);
                }
                // Una chiave ripetuta o estranea: la forma non e' quella di un
                // CRS con nome, e non si indovina che cosa volesse dire.
                _ => {
                    map.next_value::<IgnoredAny>()?;
                    ammesso = false;
                }
            }
        }
        Ok(ammesso && tipo_nome == Some(true) && nome == Some(true))
    }
}

/// Le `properties` di un CRS con nome: `{"name": <stringa>}`, nient'altro.
struct ProprietaCrs;

impl<'de> DeserializeSeed<'de> for ProprietaCrs {
    type Value = bool;
    fn deserialize<D: Deserializer<'de>>(self, d: D) -> Result<bool, D::Error> {
        d.deserialize_any(VisitaProprieta)
    }
}

struct VisitaProprieta;

impl<'de> Visitor<'de> for VisitaProprieta {
    type Value = bool;
    fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str("le properties del membro crs")
    }
    scarta_il_resto!(bool, false);
    fn visit_str<E: serde::de::Error>(self, _: &str) -> Result<bool, E> {
        Ok(false)
    }
    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<bool, A::Error> {
        let mut nome: Option<bool> = None;
        let mut ammesso = true;
        while let Some(chiave) = map.next_key::<String>()? {
            if chiave == "name" && nome.is_none() {
                nome = Some(
                    map.next_value_seed(Testo)?
                        .is_some_and(|n| nomina_wgs84(&n)),
                );
            } else {
                map.next_value::<IgnoredAny>()?;
                ammesso = false;
            }
        }
        Ok(ammesso && nome == Some(true))
    }
}
