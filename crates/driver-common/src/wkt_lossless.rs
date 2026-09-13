//! Conversione condivisa fra WKT dimensionale e l'AST WKB lossless.

use plenora_io_model::contract::{CoordinateDimensions, GeometryType};
use plenora_io_model::limits::WkbLimits;
use plenora_io_model::wkb::{WkbCoordinate, WkbGeometry, WkbValue};
use plenora_io_model::{NumeroStrutturale, PlenoraIoError, PublicMessage, Result};
use wkt::types::{
    Coord, Dimension, GeometryCollection, LineString, MultiLineString, MultiPoint, MultiPolygon,
    Point, Polygon,
};
use wkt::Wkt;

/// L'errore WKB di questo modulo, con il prefisso del sottosistema.
///
/// Il prefisso resta, ma non passa piu' da `format!`: e' il primo membro di
/// una `CuratedPair`, cioe' due `&'static str` scelti a compile time. La firma
/// era `impl Into<String>`, e attraverso di lei passavano ventotto chiamanti —
/// due dei quali con il testo di una dipendenza, che il censimento dei
/// costruttori non poteva vedere perche' qui dentro non c'e' nessun
/// `PlenoraIoError::`.
fn error(message: &'static str) -> PlenoraIoError {
    PlenoraIoError::wkb_redatto(&PublicMessage::CuratedPair(PREFISSO, message))
}

/// Il prefisso del sottosistema, in un posto solo.
const PREFISSO: &str = "WKT:";

fn dimension(dimensions: CoordinateDimensions) -> Result<Dimension> {
    match dimensions {
        CoordinateDimensions::Xy => Ok(Dimension::XY),
        CoordinateDimensions::Xyz => Ok(Dimension::XYZ),
        CoordinateDimensions::Xym => Ok(Dimension::XYM),
        CoordinateDimensions::Xyzm => Ok(Dimension::XYZM),
        CoordinateDimensions::Unknown => Err(error("dimensionalità ignota non serializzabile")),
    }
}

fn validate_finite_coordinate(x: f64, y: f64, z: Option<f64>, m: Option<f64>) -> Result<()> {
    if !x.is_finite()
        || !y.is_finite()
        || z.is_some_and(|value| !value.is_finite())
        || m.is_some_and(|value| !value.is_finite())
    {
        return Err(error("coordinata non finita"));
    }
    Ok(())
}

/// Analizza WKT 2D/3D/M/ZM applicando i tetti **durante** il parse.
///
/// Non esiste una variante senza tetti. C'e' stata -- `parse_wkt`, che
/// chiamava questa con i default -- e il gate dei default l'ha rifiutata: una
/// comodita' pubblica che sceglie da sola le quote riporta indietro cio' che
/// S5 ha portato fino all'inferenza. Chi analizza WKT dichiara con quali
/// limiti, anche quando sono quelli predefiniti.
///
/// Finding #6 review 2026-08-15: il parser `wkt` costruiva l'AST intero
/// prima che qualsiasi budget vedesse il risultato, e una cella CSV/XLSX
/// ostile poteva superare i limiti dichiarati dal chiamante senza che il
/// driver se ne accorgesse. Il cap in byte era l'unica difesa: esatto e
/// grossolano, perche' dice quanto puo' essere lungo l'input e non quanto
/// puo' costare.
///
/// Il lotto S12 ha chiuso il rinvio. Tutti e tre i tetti si applicano ora
/// **durante** il parse -- byte sul testo, componenti e profondita' mentre
/// si consuma -- perche' l'analisi e' progressiva: `wkt_progressivo`
/// costruisce la geometria mentre legge e addebita ogni coordinata e ogni
/// figlio nel momento in cui li legge. Cio' che non e' stato letto non e'
/// stato allocato.
///
/// # Errors
///
/// Sintassi non valida, dimensionalita' incoerente, coordinata non finita;
/// piu' `LimitExceeded` quando il testo supera
/// `max_cell_bytes`, i componenti superano `max_components` o l'annidamento
/// supera `max_depth`.
pub fn parse_wkt_bounded(text: &str, limiti: &WkbLimits) -> Result<WkbGeometry> {
    let max_bytes = limiti.max_cell_bytes;
    if text.len() > max_bytes {
        return Err(PlenoraIoError::limite_redatto(
            &PublicMessage::CuratedBetween(
                "cella WKT di",
                NumeroStrutturale::Conteggio(crate::saturating_u64(text.len())),
                "byte oltre il limite di",
                NumeroStrutturale::Limite(crate::saturating_u64(max_bytes)),
            ),
        ));
    }
    let geometria = crate::wkt_progressivo::analizza(text, limiti)?;
    // Simmetria con la scrittura: quello che accettiamo da testo deve poter
    // tornare a testo. Accettare in lettura una geometria che non sappiamo
    // riscrivere e' una trappola — si legge un CSV e poi non lo si riesce a
    // riprodurre — e non toglie nulla, perche' `POLYGON(EMPTY)` non ha mai
    // fatto round-trip: veniva riletto come poligono senza anelli.
    verifica_esprimibile(&geometria)?;
    Ok(geometria)
}

fn coordinate_to_wkt(
    coordinate: &WkbCoordinate,
    expected: CoordinateDimensions,
) -> Result<Coord<f64>> {
    let actual = match (coordinate.z.is_some(), coordinate.m.is_some()) {
        (false, false) => CoordinateDimensions::Xy,
        (true, false) => CoordinateDimensions::Xyz,
        (false, true) => CoordinateDimensions::Xym,
        (true, true) => CoordinateDimensions::Xyzm,
    };
    if actual != expected {
        // La dimensionalita' attesa e' quella della geometria, che il
        // chiamante ha in mano: nel messaggio resta quella osservata, che e'
        // l'informazione che lui non ha.
        return Err(PlenoraIoError::wkb_redatto(&PublicMessage::CuratedPair(
            "WKT: coordinata con dimensionalità incoerente con la geometria:",
            actual.nome(),
        )));
    }
    validate_finite_coordinate(coordinate.x, coordinate.y, coordinate.z, coordinate.m)?;
    Ok(Coord {
        x: coordinate.x,
        y: coordinate.y,
        z: coordinate.z,
        m: coordinate.m,
    })
}

fn coordinates_to_wkt(
    coordinates: &[WkbCoordinate],
    expected: CoordinateDimensions,
) -> Result<Vec<Coord<f64>>> {
    coordinates
        .iter()
        .map(|coordinate| coordinate_to_wkt(coordinate, expected))
        .collect()
}

fn checked_child<'a>(
    child: &'a WkbGeometry,
    parent: &WkbGeometry,
    expected_type: GeometryType,
) -> Result<&'a WkbValue> {
    if child.srid.is_some()
        || child.dimensions != parent.dimensions
        || child.geometry_type() != expected_type
    {
        return Err(error("geometria WKB annidata incoerente"));
    }
    Ok(&child.value)
}

// Dispatch esaustivo sui rami del valore WKB: la lunghezza e' nel numero di
// varianti, non in complessita' logica.
#[allow(clippy::too_many_lines)]
fn geometry_to_wkt(geometry: &WkbGeometry) -> Result<Wkt<f64>> {
    if geometry.srid.is_some() {
        return Err(error(
            "SRID embedded non rappresentabile in WKT; usare il CRS del contratto",
        ));
    }
    let dim = dimension(geometry.dimensions)?;
    Ok(match &geometry.value {
        WkbValue::Point(coordinate) => Wkt::Point(Point::new(
            Some(coordinate_to_wkt(coordinate, geometry.dimensions)?),
            dim,
        )),
        WkbValue::LineString(coordinates) => Wkt::LineString(LineString::new(
            coordinates_to_wkt(coordinates, geometry.dimensions)?,
            dim,
        )),
        WkbValue::Polygon(rings) => Wkt::Polygon(Polygon::new(
            rings
                .iter()
                .map(|ring| {
                    Ok(LineString::new(
                        coordinates_to_wkt(ring, geometry.dimensions)?,
                        dim,
                    ))
                })
                .collect::<Result<Vec<_>>>()?,
            dim,
        )),
        WkbValue::MultiPoint(children) => Wkt::MultiPoint(MultiPoint::new(
            children
                .iter()
                .map(
                    |child| match checked_child(child, geometry, GeometryType::Point)? {
                        WkbValue::Point(coordinate) => Ok(Point::new(
                            Some(coordinate_to_wkt(coordinate, geometry.dimensions)?),
                            dim,
                        )),
                        _ => Err(error("MultiPoint con membro non-Point")),
                    },
                )
                .collect::<Result<Vec<_>>>()?,
            dim,
        )),
        WkbValue::MultiLineString(children) => Wkt::MultiLineString(MultiLineString::new(
            children
                .iter()
                .map(
                    |child| match checked_child(child, geometry, GeometryType::LineString)? {
                        WkbValue::LineString(coordinates) => Ok(LineString::new(
                            coordinates_to_wkt(coordinates, geometry.dimensions)?,
                            dim,
                        )),
                        _ => Err(error("MultiLineString con membro non-LineString")),
                    },
                )
                .collect::<Result<Vec<_>>>()?,
            dim,
        )),
        WkbValue::MultiPolygon(children) => Wkt::MultiPolygon(MultiPolygon::new(
            children
                .iter()
                .map(
                    |child| match checked_child(child, geometry, GeometryType::Polygon)? {
                        WkbValue::Polygon(rings) => Ok(Polygon::new(
                            rings
                                .iter()
                                .map(|ring| {
                                    Ok(LineString::new(
                                        coordinates_to_wkt(ring, geometry.dimensions)?,
                                        dim,
                                    ))
                                })
                                .collect::<Result<Vec<_>>>()?,
                            dim,
                        )),
                        _ => Err(error("MultiPolygon con membro non-Polygon")),
                    },
                )
                .collect::<Result<Vec<_>>>()?,
            dim,
        )),
        WkbValue::GeometryCollection(children) => {
            let children = children
                .iter()
                .map(|child| {
                    if child.srid.is_some() || child.dimensions != geometry.dimensions {
                        return Err(error(
                            "GeometryCollection WKB con membri dimensionalmente incoerenti",
                        ));
                    }
                    geometry_to_wkt(child)
                })
                .collect::<Result<Vec<_>>>()?;
            Wkt::GeometryCollection(GeometryCollection::new(children, dim))
        }
        WkbValue::CircularString(_)
        | WkbValue::CompoundCurve(_)
        | WkbValue::CurvePolygon(_)
        | WkbValue::MultiCurve(_)
        | WkbValue::MultiSurface(_)
        | WkbValue::PolyhedralSurface(_)
        | WkbValue::Tin(_)
        | WkbValue::Triangle(_) => {
            return Err(error(
                "tipo WKB esteso non rappresentabile dal profilo WKT corrente",
            ))
        }
    })
}

/// Serializza l'AST WKB in WKT dimensionale usando una rappresentazione
/// numerica `f64` round-trip.
///
/// # Errors
///
/// Restituisce [`PlenoraIoError::Wkb`] se la geometria porta un SRID
/// embedded, se la dimensionalità è ignota, se una coordinata non è finita o
/// se il tipo WKB non è rappresentabile nel profilo WKT corrente.
pub fn format_wkt(geometry: &WkbGeometry) -> Result<String> {
    let mut output = String::new();
    format_wkt_into(geometry, &mut output)?;
    Ok(output)
}

/// Appende WKT dimensionale a un buffer riusabile.
///
/// La conversione viene validata prima di toccare `output`: in caso di errore
/// il contenuto precedente resta invariato.
///
/// # Errors
///
/// Restituisce gli stessi errori di [`format_wkt`], più
/// [`PlenoraIoError::Wkb`] se la scrittura sul buffer fallisce.
pub fn format_wkt_into(geometry: &WkbGeometry, output: &mut String) -> Result<()> {
    verifica_esprimibile(geometry)?;
    let mut testo = String::new();
    scrivi_geometria(geometry, &mut testo)?;
    output.push_str(&testo);
    Ok(())
}

/// Rifiuta le geometrie che il WKT non sa esprimere fedelmente.
///
/// Il modulo si chiama `wkt_lossless` e i driver dichiarano
/// [`Fidelity::Lossless`](plenora_io_core::descriptor::Fidelity): una
/// conversione che cambia la geometria senza dirlo e' peggio di una che
/// fallisce. I codec WKB sono piu' permissivi della grammatica WKT, e sulla
/// differenza c'erano quattro forme che uscivano male:
///
/// | geometria | prodotto prima | |
/// |---|---|---|
/// | `Polygon` con un anello vuoto | `POLYGON EMPTY` | rileggeva un poligono senza anelli |
/// | `Polygon` valido con un interno vuoto | `POLYGON((…),())` | non rileggibile |
/// | `MultiPolygon` con un membro dall'anello vuoto | `MULTIPOLYGON((()))` | non rileggibile |
/// | `MultiLineString` con un membro vuoto | `MULTILINESTRING(())` | non rileggibile |
///
/// Le ultime tre scrivevano in una cella CSV o XLSX del testo che il nostro
/// stesso parser rifiuta.
///
/// Il confine e' netto: **una sequenza di coordinate annidata in un
/// contenitore non puo' essere vuota**. Restano rappresentabili sia le
/// geometrie vuote di primo livello (`POLYGON EMPTY`, `LINESTRING EMPTY`,
/// `MULTIPOLYGON(EMPTY)`) sia gli anelli degeneri ma non vuoti: `POLYGON((0 0))`
/// con una sola coordinata fa round-trip corretto, quindi non lo rifiutiamo —
/// il controllo riguarda la fedelta' della conversione, non la validita' OGC
/// della geometria, che e' un'altra decisione e non e' presa qui.
pub(crate) fn verifica_esprimibile(geometry: &WkbGeometry) -> Result<()> {
    match &geometry.value {
        WkbValue::Polygon(anelli) => {
            if anelli.iter().any(Vec::is_empty) {
                return Err(error(
                    "anello senza coordinate: il WKT non lo distingue da un poligono vuoto",
                ));
            }
        }
        WkbValue::MultiLineString(membri) => {
            for membro in membri {
                if matches!(&membro.value, WkbValue::LineString(coordinate) if coordinate.is_empty())
                {
                    return Err(error(
                        "LineString senza coordinate dentro una MultiLineString: il WKT che ne \
                         risulta non e' rileggibile",
                    ));
                }
            }
        }
        WkbValue::MultiPolygon(membri) | WkbValue::GeometryCollection(membri) => {
            for membro in membri {
                verifica_esprimibile(membro)?;
            }
        }
        _ => {}
    }
    Ok(())
}

/// Serializza una geometria delegando alla crate `wkt`, tranne dove quella non
/// sa farlo.
///
/// `write_multi_polygon` (`wkt` 0.14.0, `to_wkt/geo_trait_impl.rs:240`) scrive
/// `((` e poi fa `.unwrap()` sull'anello esterno del primo poligono: apre le
/// parentesi prima di sapere se quel poligono ne ha uno. Un `MULTIPOLYGON` con
/// un membro vuoto la fa quindi panicare, e non e' solo l'`unwrap`: cosi'
/// com'e' scritta non potrebbe comunque emettere `MULTIPOLYGON(EMPTY)`.
///
/// Non e' un caso di laboratorio. Un poligono senza anelli e' rappresentabile
/// in WKB — diciotto byte per il caso minimo — e i nostri codec lo accettano in
/// entrambi i versi, quindi la geometria arriva qui dai reader, non solo dal
/// testo. I chiamanti sono i writer CSV e XLSX: prima di questa correzione
/// scrivere un dataset che la contenesse abbatteva il processo.
///
/// Trovato dallo smoke di fuzzing su `main` con l'input `MULtIpolygon(\nemPTy)`.
/// Segnalato a monte a `georust/wkt`.
///
/// Le geometrie che gia' funzionavano continuano a passare dalla crate: la
/// deviazione riguarda solo le forme che la crate non sa scrivere, e i membri
/// non vuoti sono comunque serializzati da lei, cosi' la formattazione
/// numerica resta identica byte per byte nel resto dell'output.
fn scrivi_geometria(geometry: &WkbGeometry, output: &mut String) -> Result<()> {
    use std::fmt::Write as _;

    match &geometry.value {
        WkbValue::MultiPolygon(membri) if membri.iter().any(poligono_senza_anelli) => {
            scrivi_multipoligono_con_membri_vuoti(geometry, membri, output)
        }
        WkbValue::GeometryCollection(membri)
            if membri.iter().any(contiene_multipoligono_con_membri_vuoti) =>
        {
            output.push_str("GEOMETRYCOLLECTION");
            output.push_str(suffisso_dimensionale(geometry.dimensions)?);
            output.push('(');
            for (indice, membro) in membri.iter().enumerate() {
                if membro.srid.is_some() || membro.dimensions != geometry.dimensions {
                    return Err(error(
                        "GeometryCollection WKB con membri dimensionalmente incoerenti",
                    ));
                }
                if indice > 0 {
                    output.push(',');
                }
                scrivi_geometria(membro, output)?;
            }
            output.push(')');
            Ok(())
        }
        _ => {
            let wkt = geometry_to_wkt(geometry)?;
            write!(output, "{wkt}").map_err(|_| error("serializzazione WKT fallita"))
        }
    }
}

const fn poligono_senza_anelli(membro: &WkbGeometry) -> bool {
    matches!(&membro.value, WkbValue::Polygon(anelli) if anelli.is_empty())
}

fn contiene_multipoligono_con_membri_vuoti(geometry: &WkbGeometry) -> bool {
    match &geometry.value {
        WkbValue::MultiPolygon(membri) => membri.iter().any(poligono_senza_anelli),
        WkbValue::GeometryCollection(membri) => {
            membri.iter().any(contiene_multipoligono_con_membri_vuoti)
        }
        _ => false,
    }
}

/// Il suffisso che la crate `wkt` mette dopo il nome del tipo. Fallisce sulle
/// stesse dimensionalità su cui fallisce [`dimension`], perché i due devono
/// accettare esattamente le stesse geometrie.
fn suffisso_dimensionale(dimensioni: CoordinateDimensions) -> Result<&'static str> {
    match dimensioni {
        CoordinateDimensions::Xy => Ok(""),
        CoordinateDimensions::Xyz => Ok(" Z"),
        CoordinateDimensions::Xym => Ok(" M"),
        CoordinateDimensions::Xyzm => Ok(" ZM"),
        CoordinateDimensions::Unknown => Err(error("dimensionalità ignota non serializzabile")),
    }
}

/// Scrive `MULTIPOLYGON[ Z|M|ZM](membro,membro,...)`, dove un membro e'
/// `EMPTY` oppure la parte fra parentesi di un `POLYGON`.
///
/// Ogni membro non vuoto viene serializzato dalla crate come `POLYGON…` e poi
/// privato del proprio tag: e' l'unico modo di comporre il testo senza
/// riscrivere la formattazione dei numeri, che deve restare quella usata da
/// tutto il resto dell'output.
fn scrivi_multipoligono_con_membri_vuoti(
    geometry: &WkbGeometry,
    membri: &[WkbGeometry],
    output: &mut String,
) -> Result<()> {
    use std::fmt::Write as _;

    let suffisso = suffisso_dimensionale(geometry.dimensions)?;
    output.push_str("MULTIPOLYGON");
    output.push_str(suffisso);
    output.push('(');
    for (indice, membro) in membri.iter().enumerate() {
        // Stessa validazione degli altri rami: tipo, dimensioni e assenza di
        // SRID annidati.
        match checked_child(membro, geometry, GeometryType::Polygon)? {
            WkbValue::Polygon(_) => {}
            _ => return Err(error("MultiPolygon con membro non-Polygon")),
        }
        if indice > 0 {
            output.push(',');
        }
        let poligono = geometry_to_wkt(membro)?;
        let mut testo = String::new();
        write!(testo, "{poligono}").map_err(|_| error("serializzazione WKT fallita"))?;
        let corpo = testo
            .strip_prefix("POLYGON")
            .and_then(|resto| resto.strip_prefix(suffisso))
            .ok_or_else(|| {
                // Il testo ricevuto non esce: e' il WKT generato dalla
                // geometria, cioe' un derivato del payload.
                error("POLYGON atteso dalla crate wkt, ricevuto un testo diverso")
            })?;
        // `POLYGON EMPTY` lascia uno spazio davanti a `EMPTY`.
        output.push_str(corpo.trim_start());
    }
    output.push(')');
    Ok(())
}

#[cfg(test)]
mod tests;
