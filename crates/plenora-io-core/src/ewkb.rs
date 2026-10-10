//! EWKB in ingresso verso un sink che scrive solo WKB ISO.
//!
//! # Perche' esiste
//!
//! `ARROW-VOCABULARY-1.0` ammette per `plenora.geometry.encoding` sia `wkb` sia
//! `ewkb`, e `ewkb` e' l'unica forma che plenora-database-tools emette in
//! lettura (`PostGIS`, `ST_AsEWKB`). Fino alla 4.1.1 ogni sink di questo
//! componente tranne Arrow IPC dichiarava il solo `wkb`, e una catena
//! database -> IO si fermava su un rifiuto di capacita' (`encoding: ewkb`) per
//! un dato che il contratto dichiara valido.
//!
//! # Che cosa fa, ed e' esatto per costruzione
//!
//! EWKB e WKB ISO differiscono **solo** nella parola di tipo -- i flag Z, M e
//! SRID di `PostGIS` contro gli scarti 1000/2000/3000 -- e nei quattro byte del
//! SRID che EWKB porta dopo di essa. Le coordinate sono gli stessi `f64`: la
//! conversione decodifica l'AST lossless di `plenora-io-model` e lo ricodifica
//! ISO, e le ordinate passano da `from_*_bytes` a `to_le_bytes` senza
//! aritmetica, quindi bit per bit.
//!
//! Il SRID non sparisce in silenzio. Il contratto WKB non ha dove metterlo, e
//! la conversione e' ammessa solo se il CRS del contratto lo rappresenta gia':
//!
//! * un SRID dichiarato deve coincidere con l'autorita' del CRS (`crs_id` o la
//!   definizione); se non coincide, o se il CRS non ne porta una, e' un errore
//!   esplicito, perche' scartarlo cambierebbe il significato delle coordinate;
//! * ogni geometria del payload deve portare **quel** SRID, o nessuno; uno
//!   diverso e' un errore, mai una scelta.

use std::collections::HashMap;
use std::sync::Arc;

use arrow_array::{Array, ArrayRef, BinaryArray, LargeBinaryArray, RecordBatch};
use arrow_schema::{DataType, Field, Schema, SchemaRef};
use plenora_io_model::contract::{GeometryColumnContract, GeometryEncoding};
use plenora_io_model::geometry::{PLENORA_ENCODING_KEY, PLENORA_SRID_KEY};
use plenora_io_model::limits::WkbLimits;
use plenora_io_model::wkb::{decode_wkb, encode_wkb_into, WkbFlavor, WkbGeometry, WkbValue};
use plenora_io_model::{
    CapabilityReason, ContractIdentifier, PlenoraIoError, PublicMessage, Result,
};

use crate::capabilities::comparable_crs_representations;
use crate::descriptor::FormatDescriptor;

/// La conversione da applicare a un layer, decisa una volta sul contratto.
#[derive(Clone, Debug)]
pub struct DaEwkbAWkb {
    /// La posizione della colonna geometria nello schema.
    indice: usize,
    /// Il SRID che il contratto dichiara, e che il payload deve portare.
    srid: Option<i32>,
    limiti: WkbLimits,
    schema: SchemaRef,
}

fn rifiuto(
    driver: &'static str,
    geometria: &GeometryColumnContract,
    messaggio: &'static str,
) -> PlenoraIoError {
    PlenoraIoError::capability_redatta(
        driver,
        ContractIdentifier::from_geometry_column(geometria).as_ref(),
        CapabilityReason::GeometryEncoding,
        &PublicMessage::Curated(messaggio),
    )
}

/// Se il sink non scrive EWKB e il contratto lo dichiara, il contratto da
/// pianificare e la conversione da applicare ai batch.
///
/// `None` se non c'e' niente da convertire: niente geometria, una geometria
/// gia' WKB, o un sink che EWKB lo scrive.
///
/// # Errors
///
/// Un rifiuto di capacita' (`GeometryEncoding`) se il SRID dichiarato non
/// coincide con l'autorita' del CRS, o se il CRS non ne rappresenta una: in
/// entrambi i casi passare a WKB perderebbe un'informazione che il contratto
/// non porterebbe piu'.
pub fn pianifica(
    descrittore: &FormatDescriptor,
    schema: &SchemaRef,
    geometria: Option<&GeometryColumnContract>,
    limiti: WkbLimits,
) -> Result<Option<(SchemaRef, GeometryColumnContract, DaEwkbAWkb)>> {
    let Some(geometria) = geometria else {
        return Ok(None);
    };
    if geometria.encoding != GeometryEncoding::Ewkb {
        return Ok(None);
    }
    let scrive_ewkb = descrittore
        .write_capabilities()
        .is_some_and(|caps| caps.geometry.encodings.contains(&GeometryEncoding::Ewkb));
    if scrive_ewkb {
        return Ok(None);
    }
    let driver = descrittore.id();
    if let Some(srid) = geometria.srid {
        let [dal_crs_id, _, dalla_definizione] = comparable_crs_representations(geometria);
        let autorita = [dal_crs_id, dalla_definizione];
        if autorita.iter().all(Option::is_none) {
            return Err(rifiuto(
                driver,
                geometria,
                "SRID EWKB senza un CRS che lo rappresenti: in WKB andrebbe perso",
            ));
        }
        if autorita
            .iter()
            .flatten()
            .any(|valore| *valore != i64::from(srid))
        {
            return Err(rifiuto(
                driver,
                geometria,
                "SRID EWKB diverso dal CRS dichiarato nei metadati",
            ));
        }
    }

    let indice = geometria.field_id.0 as usize;
    if indice >= schema.fields().len() {
        return Err(rifiuto(
            driver,
            geometria,
            "colonna geometria fuori dallo schema",
        ));
    }
    let schema_wkb = schema_in_wkb(schema, indice);
    let mut contratto = geometria.clone();
    contratto.encoding = GeometryEncoding::Wkb;
    contratto.srid = None;
    Ok(Some((
        Arc::clone(&schema_wkb),
        contratto,
        DaEwkbAWkb {
            indice,
            srid: geometria.srid,
            limiti,
            schema: schema_wkb,
        },
    )))
}

/// Lo schema con la colonna geometria dichiarata WKB e senza SRID: le altre
/// chiavi e gli altri campi restano quelli dell'ingresso.
fn schema_in_wkb(schema: &SchemaRef, indice: usize) -> SchemaRef {
    let campi: Vec<Field> = schema
        .fields()
        .iter()
        .enumerate()
        .map(|(i, campo)| {
            if i != indice {
                return campo.as_ref().clone();
            }
            let metadati: HashMap<String, String> = campo
                .metadata()
                .iter()
                .filter(|(chiave, _)| chiave.as_str() != PLENORA_SRID_KEY)
                .map(|(chiave, valore)| {
                    if chiave.as_str() == PLENORA_ENCODING_KEY {
                        (chiave.clone(), GeometryEncoding::Wkb.nome().to_owned())
                    } else {
                        (chiave.clone(), valore.clone())
                    }
                })
                .collect();
            campo.as_ref().clone().with_metadata(metadati)
        })
        .collect();
    Arc::new(Schema::new_with_metadata(campi, schema.metadata().clone()))
}

fn errore_payload(messaggio: &'static str) -> PlenoraIoError {
    PlenoraIoError::wkb_redatto(&PublicMessage::Curated(messaggio))
}

/// Toglie il SRID da ogni livello dell'AST, dopo averlo confrontato.
fn senza_srid(geometria: &mut WkbGeometry, atteso: Option<i32>) -> Result<()> {
    if let Some(srid) = geometria.srid.take() {
        if Some(srid) != atteso {
            return Err(errore_payload(
                "SRID del payload EWKB diverso da quello dichiarato",
            ));
        }
    }
    match &mut geometria.value {
        WkbValue::MultiPoint(figli)
        | WkbValue::MultiLineString(figli)
        | WkbValue::MultiPolygon(figli)
        | WkbValue::GeometryCollection(figli)
        | WkbValue::CompoundCurve(figli)
        | WkbValue::CurvePolygon(figli)
        | WkbValue::MultiCurve(figli)
        | WkbValue::MultiSurface(figli)
        | WkbValue::PolyhedralSurface(figli)
        | WkbValue::Tin(figli) => {
            for figlio in figli {
                senza_srid(figlio, atteso)?;
            }
        }
        WkbValue::Point(_)
        | WkbValue::LineString(_)
        | WkbValue::Polygon(_)
        | WkbValue::CircularString(_)
        | WkbValue::Triangle(_) => {}
    }
    Ok(())
}

impl DaEwkbAWkb {
    /// Un valore EWKB come WKB ISO, con le stesse coordinate bit per bit.
    fn converti_valore(&self, valore: &[u8], uscita: &mut Vec<u8>) -> Result<()> {
        let mut geometria = decode_wkb(valore, &self.limiti)?;
        senza_srid(&mut geometria, self.srid)?;
        encode_wkb_into(&geometria, WkbFlavor::Iso, uscita)
    }

    /// Il batch con la colonna geometria convertita, e lo schema del piano.
    ///
    /// # Errors
    ///
    /// Un errore WKB se un valore non si decodifica, porta un SRID diverso da
    /// quello dichiarato, o non si ricodifica; un errore di contratto se la
    /// colonna non e' binaria.
    pub fn converti(&self, batch: &RecordBatch) -> Result<RecordBatch> {
        let colonna = batch.column(self.indice);
        let mut buffer = Vec::new();
        let convertita: ArrayRef = match colonna.data_type() {
            DataType::Binary => {
                let valori = colonna
                    .as_any()
                    .downcast_ref::<BinaryArray>()
                    .ok_or_else(|| errore_payload("colonna geometria non binaria"))?;
                let mut uscita = Vec::with_capacity(valori.len());
                for valore in valori {
                    uscita.push(match valore {
                        None => None,
                        Some(byte) => {
                            self.converti_valore(byte, &mut buffer)?;
                            Some(buffer.clone())
                        }
                    });
                }
                Arc::new(BinaryArray::from_iter(uscita))
            }
            DataType::LargeBinary => {
                let valori = colonna
                    .as_any()
                    .downcast_ref::<LargeBinaryArray>()
                    .ok_or_else(|| errore_payload("colonna geometria non binaria"))?;
                let mut uscita = Vec::with_capacity(valori.len());
                for valore in valori {
                    uscita.push(match valore {
                        None => None,
                        Some(byte) => {
                            self.converti_valore(byte, &mut buffer)?;
                            Some(buffer.clone())
                        }
                    });
                }
                Arc::new(LargeBinaryArray::from_iter(uscita))
            }
            _ => {
                return Err(PlenoraIoError::contratto_redatto(&PublicMessage::Curated(
                    "colonna geometria EWKB non binaria",
                )))
            }
        };
        let mut tutte = batch.columns().to_vec();
        tutte[self.indice] = convertita;
        RecordBatch::try_new(Arc::clone(&self.schema), tutte).map_err(|_| {
            PlenoraIoError::contratto_redatto(&PublicMessage::Curated(
                "batch EWKB non riconducibile allo schema WKB del piano",
            ))
        })
    }
}

#[cfg(test)]
mod tests;
