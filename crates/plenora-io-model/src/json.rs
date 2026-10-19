//! Lettura dei documenti di controllo senza perdita silenziosa di membri.
use serde::de::{DeserializeOwned, MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer};
use serde_json::{Map, Value};

/// Verifica ogni oggetto, anche dentro array e campi ignorati dal destinatario.
///
/// # Errors
/// Rifiuta JSON malformato, dati successivi e membri duplicati. Il messaggio
/// non contiene mai chiavi o valori del documento.
pub fn ensure_no_duplicate_keys(bytes: &[u8]) -> Result<(), &'static str> {
    serde_json::from_slice::<SenzaDoppioni>(bytes)
        .map(|_| ())
        .map_err(|_| "documento JSON non valido o con membri duplicati")
}

/// Deserializza solo dopo aver verificato tutti i membri del documento.
///
/// # Errors
/// JSON non valido, duplicati o struttura incompatibile, senza dati nei messaggi.
pub fn from_slice<T: DeserializeOwned>(bytes: &[u8]) -> Result<T, &'static str> {
    ensure_no_duplicate_keys(bytes)?;
    serde_json::from_slice(bytes).map_err(|_| "struttura JSON non valida")
}

/// Variante UTF-8 di [`from_slice`].
///
/// # Errors
/// Gli stessi errori redatti di [`from_slice`].
pub fn from_str<T: DeserializeOwned>(text: &str) -> Result<T, &'static str> {
    from_slice(text.as_bytes())
}

/// Un JSON qualunque, con le chiavi ripetute rifiutate a ogni livello.
fn senza_chiavi_ripetute<'de, D>(deserializer: D) -> Result<Value, D::Error>
where
    D: Deserializer<'de>,
{
    deserializer.deserialize_any(ValoreSenzaDoppioni)
}

struct ValoreSenzaDoppioni;

impl<'de> Visitor<'de> for ValoreSenzaDoppioni {
    type Value = Value;

    fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("un valore JSON senza chiavi ripetute")
    }

    fn visit_bool<E>(self, valore: bool) -> Result<Value, E> {
        Ok(Value::Bool(valore))
    }

    fn visit_i64<E>(self, valore: i64) -> Result<Value, E> {
        Ok(Value::from(valore))
    }

    fn visit_u64<E>(self, valore: u64) -> Result<Value, E> {
        Ok(Value::from(valore))
    }

    fn visit_f64<E: serde::de::Error>(self, valore: f64) -> Result<Value, E> {
        // Un numero non finito non e' JSON: `null` qui sarebbe un valore
        // inventato, e si rifiuta invece di convertirlo.
        serde_json::Number::from_f64(valore)
            .map(Value::Number)
            .ok_or_else(|| serde::de::Error::custom("numero non rappresentabile"))
    }

    fn visit_str<E>(self, valore: &str) -> Result<Value, E> {
        Ok(Value::String(valore.to_owned()))
    }

    fn visit_string<E>(self, valore: String) -> Result<Value, E> {
        Ok(Value::String(valore))
    }

    fn visit_unit<E>(self) -> Result<Value, E> {
        Ok(Value::Null)
    }

    fn visit_none<E>(self) -> Result<Value, E> {
        Ok(Value::Null)
    }

    fn visit_some<D>(self, deserializer: D) -> Result<Value, D::Error>
    where
        D: Deserializer<'de>,
    {
        senza_chiavi_ripetute(deserializer)
    }

    fn visit_seq<A>(self, mut sequenza: A) -> Result<Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let mut elementi = Vec::new();
        while let Some(SenzaDoppioni(elemento)) = sequenza.next_element()? {
            elementi.push(elemento);
        }
        Ok(Value::Array(elementi))
    }

    fn visit_map<A>(self, mut mappa: A) -> Result<Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut campi = Map::new();
        while let Some((chiave, SenzaDoppioni(valore))) =
            mappa.next_entry::<String, SenzaDoppioni>()?
        {
            if campi.insert(chiave, valore).is_some() {
                return Err(serde::de::Error::custom("membro JSON duplicato"));
            }
        }
        Ok(Value::Object(campi))
    }
}

struct SenzaDoppioni(Value);

impl<'de> Deserialize<'de> for SenzaDoppioni {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        senza_chiavi_ripetute(deserializer).map(SenzaDoppioni)
    }
}

#[cfg(test)]
mod tests;
