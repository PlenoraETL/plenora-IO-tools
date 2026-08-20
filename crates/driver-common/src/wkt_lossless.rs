//! Conversione condivisa fra WKT dimensionale e l'AST WKB lossless.

use plenora_io_model::contract::{CoordinateDimensions, GeometryType};
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

const fn contract_dimensions(dimension: Dimension) -> CoordinateDimensions {
    match dimension {
        Dimension::XY => CoordinateDimensions::Xy,
        Dimension::XYZ => CoordinateDimensions::Xyz,
        Dimension::XYM => CoordinateDimensions::Xym,
        Dimension::XYZM => CoordinateDimensions::Xyzm,
    }
}

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

fn coordinate_from_wkt(
    coordinate: &Coord<f64>,
    expected: CoordinateDimensions,
) -> Result<WkbCoordinate> {
    let actual = contract_dimensions(coordinate.dimension());
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
    Ok(WkbCoordinate {
        x: coordinate.x,
        y: coordinate.y,
        z: coordinate.z,
        m: coordinate.m,
    })
}

fn coordinates_from_wkt(
    coordinates: &[Coord<f64>],
    expected: CoordinateDimensions,
) -> Result<Vec<WkbCoordinate>> {
    coordinates
        .iter()
        .map(|coordinate| coordinate_from_wkt(coordinate, expected))
        .collect()
}

// Dispatch esaustivo sui rami del tipo WKT: la lunghezza e' nel numero di
// varianti, non in complessita' logica.
#[allow(clippy::too_many_lines)]
fn geometry_from_wkt(value: &Wkt<f64>) -> Result<WkbGeometry> {
    let (value, dimensions) = match value {
        Wkt::Point(point) => {
            let dimensions = contract_dimensions(point.dimension());
            let coordinate = point
                .coord()
                .ok_or_else(|| error("POINT EMPTY non rappresentabile nel core WKB"))?;
            (
                WkbValue::Point(coordinate_from_wkt(coordinate, dimensions)?),
                dimensions,
            )
        }
        Wkt::LineString(line) => {
            let dimensions = contract_dimensions(line.dimension());
            (
                WkbValue::LineString(coordinates_from_wkt(line.coords(), dimensions)?),
                dimensions,
            )
        }
        Wkt::Polygon(polygon) => {
            let dimensions = contract_dimensions(polygon.dimension());
            let rings = polygon
                .rings()
                .iter()
                .map(|ring| {
                    if contract_dimensions(ring.dimension()) != dimensions {
                        return Err(error("anello Polygon con dimensionalità incoerente"));
                    }
                    coordinates_from_wkt(ring.coords(), dimensions)
                })
                .collect::<Result<Vec<_>>>()?;
            (WkbValue::Polygon(rings), dimensions)
        }
        Wkt::MultiPoint(multipoint) => {
            let dimensions = contract_dimensions(multipoint.dimension());
            let children = multipoint
                .points()
                .iter()
                .map(|point| {
                    if contract_dimensions(point.dimension()) != dimensions {
                        return Err(error("Point annidato con dimensionalità incoerente"));
                    }
                    let coordinate = point
                        .coord()
                        .ok_or_else(|| error("POINT EMPTY annidato non rappresentabile"))?;
                    Ok(WkbGeometry {
                        value: WkbValue::Point(coordinate_from_wkt(coordinate, dimensions)?),
                        dimensions,
                        srid: None,
                    })
                })
                .collect::<Result<Vec<_>>>()?;
            (WkbValue::MultiPoint(children), dimensions)
        }
        Wkt::MultiLineString(multiline) => {
            let dimensions = contract_dimensions(multiline.dimension());
            let children = multiline
                .line_strings()
                .iter()
                .map(|line| {
                    if contract_dimensions(line.dimension()) != dimensions {
                        return Err(error("LineString annidata con dimensionalità incoerente"));
                    }
                    Ok(WkbGeometry {
                        value: WkbValue::LineString(coordinates_from_wkt(
                            line.coords(),
                            dimensions,
                        )?),
                        dimensions,
                        srid: None,
                    })
                })
                .collect::<Result<Vec<_>>>()?;
            (WkbValue::MultiLineString(children), dimensions)
        }
        Wkt::MultiPolygon(multipolygon) => {
            let dimensions = contract_dimensions(multipolygon.dimension());
            let children = multipolygon
                .polygons()
                .iter()
                .map(|polygon| {
                    if contract_dimensions(polygon.dimension()) != dimensions {
                        return Err(error("Polygon annidato con dimensionalità incoerente"));
                    }
                    let rings = polygon
                        .rings()
                        .iter()
                        .map(|ring| {
                            if contract_dimensions(ring.dimension()) != dimensions {
                                return Err(error(
                                    "anello MultiPolygon con dimensionalità incoerente",
                                ));
                            }
                            coordinates_from_wkt(ring.coords(), dimensions)
                        })
                        .collect::<Result<Vec<_>>>()?;
                    Ok(WkbGeometry {
                        value: WkbValue::Polygon(rings),
                        dimensions,
                        srid: None,
                    })
                })
                .collect::<Result<Vec<_>>>()?;
            (WkbValue::MultiPolygon(children), dimensions)
        }
        Wkt::GeometryCollection(collection) => {
            let dimensions = contract_dimensions(collection.dimension());
            let children = collection
                .geometries()
                .iter()
                .map(|child| {
                    let child = geometry_from_wkt(child)?;
                    if child.dimensions != dimensions {
                        return Err(error(
                            "GeometryCollection con dimensionalità annidate differenti",
                        ));
                    }
                    Ok(child)
                })
                .collect::<Result<Vec<_>>>()?;
            (WkbValue::GeometryCollection(children), dimensions)
        }
    };
    Ok(WkbGeometry {
        value,
        dimensions,
        srid: None,
    })
}

/// Analizza WKT 2D/3D/M/ZM senza proiettarlo su `geo-types`.
///
/// # Errors
///
/// Restituisce [`PlenoraIoError::Wkb`] se la sintassi WKT non è valida, se la
/// dimensionalità è incoerente fra geometria e coordinate, se una coordinata
/// non è finita o se la geometria non è rappresentabile nel core WKB.
pub fn parse_wkt(text: &str) -> Result<WkbGeometry> {
    parse_wkt_bounded(text, usize::MAX)
}

/// Come [`parse_wkt`] ma rifiuta a monte i testi WKT piu' lunghi di
/// `max_bytes`.
///
/// Finding #6 review 2026-08-15: il parser `wkt` costruisce l'AST intero
/// prima che qualsiasi budget veda il risultato; una singola cella
/// CSV/XLSX ostile puo' superare i limiti dichiarati dal chiamante senza
/// che il driver se ne accorga. Il cap sulla lunghezza del testo e' una
/// prima linea di difesa: e' esatto quando la cella e' UTF-8 e non ha
/// bisogno di conoscere la struttura interna.
///
/// La bound completa (`max_components`/`max_depth` applicati durante il
/// parse e non solo su `verifica_esprimibile` / `inspect_wkb` a valle)
/// richiede un parser progressivo diverso da `wkt 0.14.0`. Vedi lotto L6
/// di `docs/ROADMAP-1.1.0.md`.
///
/// # Errors
///
/// Le stesse di [`parse_wkt`] piu' `LimitExceeded` se `text.len() > max_bytes`.
pub fn parse_wkt_bounded(text: &str, max_bytes: usize) -> Result<WkbGeometry> {
    if text.len() > max_bytes {
        return Err(PlenoraIoError::limite_redatto(
            &PublicMessage::CuratedBetween(
                "cella WKT di",
                NumeroStrutturale::Conteggio(u64::try_from(text.len()).unwrap_or(u64::MAX)),
                "byte oltre il limite di",
                NumeroStrutturale::Limite(u64::try_from(max_bytes).unwrap_or(u64::MAX)),
            ),
        ));
    }
    let parsed: Wkt<f64> = text
        .parse()
        // Il testo della crate `wkt` non esce: e' testo di dipendenza, ed
        // e' il caso da cui INV-10 e' partito. La posizione dell'errore di
        // sintassi non e' recuperabile senza farlo uscire, e non la si
        // inventa.
        .map_err(|_| error("sintassi WKT non valida"))?;
    let geometria = geometry_from_wkt(&parsed)?;
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
fn verifica_esprimibile(geometry: &WkbGeometry) -> Result<()> {
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
mod tests {
    use super::*;

    #[test]
    fn round_trips_all_dimensions_and_nested_geometry() {
        for text in [
            "POINT(1 2)",
            "LINESTRING Z(0 1 2,3 4 5)",
            "MULTIPOINT M((1 2 3),(4 5 6))",
            "MULTIPOLYGON ZM(((0 0 1 10,0 2 2 11,2 0 3 12,0 0 1 10)))",
            "GEOMETRYCOLLECTION Z(POINT Z(1 2 3),LINESTRING Z(0 0 0,1 1 1))",
        ] {
            let geometry = parse_wkt(text).unwrap();
            let encoded = format_wkt(&geometry).unwrap();
            assert_eq!(parse_wkt(&encoded).unwrap(), geometry, "{encoded}");
        }
    }

    /// Un `MULTIPOLYGON` con un membro senza anelli deve serializzarsi, non
    /// abbattere il processo.
    ///
    /// Il percorso di partenza e' il WKB, non il testo: e' quello che i writer
    /// CSV e XLSX ricevono dai reader, e la geometria e' rappresentabile in
    /// diciotto byte. Prima della correzione la crate `wkt` faceva `.unwrap()`
    /// sull'anello esterno del primo poligono e il processo moriva.
    #[test]
    fn un_multipoligono_con_membro_vuoto_si_serializza_invece_di_panicare() {
        let vuoto = |dimensioni| WkbGeometry {
            value: WkbValue::Polygon(vec![]),
            dimensions: dimensioni,
            srid: None,
        };
        let quadrato = WkbGeometry {
            value: WkbValue::Polygon(vec![vec![
                WkbCoordinate {
                    x: 0.0,
                    y: 0.0,
                    z: None,
                    m: None,
                },
                WkbCoordinate {
                    x: 1.0,
                    y: 0.0,
                    z: None,
                    m: None,
                },
                WkbCoordinate {
                    x: 1.0,
                    y: 1.0,
                    z: None,
                    m: None,
                },
                WkbCoordinate {
                    x: 0.0,
                    y: 0.0,
                    z: None,
                    m: None,
                },
            ]]),
            dimensions: CoordinateDimensions::Xy,
            srid: None,
        };

        for (membri, atteso) in [
            (vec![vuoto(CoordinateDimensions::Xy)], "MULTIPOLYGON(EMPTY)"),
            (
                vec![quadrato.clone(), vuoto(CoordinateDimensions::Xy)],
                "MULTIPOLYGON(((0 0,1 0,1 1,0 0)),EMPTY)",
            ),
            (
                vec![vuoto(CoordinateDimensions::Xy), quadrato],
                "MULTIPOLYGON(EMPTY,((0 0,1 0,1 1,0 0)))",
            ),
        ] {
            let geometria = WkbGeometry {
                value: WkbValue::MultiPolygon(membri),
                dimensions: CoordinateDimensions::Xy,
                srid: None,
            };
            let testo = format_wkt(&geometria).expect("serializzazione");
            assert_eq!(testo, atteso);
            assert_eq!(
                parse_wkt(&testo).expect("rilettura"),
                geometria,
                "round-trip di {testo}"
            );
        }
    }

    /// Lo stesso caso annidato in una `GEOMETRYCOLLECTION`: la deviazione deve
    /// propagarsi ai figli, altrimenti il panico resta raggiungibile passando
    /// da un livello in piu'.
    #[test]
    fn il_membro_vuoto_e_gestito_anche_dentro_una_geometrycollection() {
        let vuoto = WkbGeometry {
            value: WkbValue::Polygon(vec![]),
            dimensions: CoordinateDimensions::Xy,
            srid: None,
        };
        let geometria = WkbGeometry {
            value: WkbValue::GeometryCollection(vec![
                WkbGeometry {
                    value: WkbValue::Point(WkbCoordinate {
                        x: 1.0,
                        y: 2.0,
                        z: None,
                        m: None,
                    }),
                    dimensions: CoordinateDimensions::Xy,
                    srid: None,
                },
                WkbGeometry {
                    value: WkbValue::MultiPolygon(vec![vuoto]),
                    dimensions: CoordinateDimensions::Xy,
                    srid: None,
                },
            ]),
            dimensions: CoordinateDimensions::Xy,
            srid: None,
        };
        let testo = format_wkt(&geometria).expect("serializzazione");
        assert_eq!(testo, "GEOMETRYCOLLECTION(POINT(1 2),MULTIPOLYGON(EMPTY))");
        assert_eq!(parse_wkt(&testo).expect("rilettura"), geometria);
    }

    /// Le quattro forme che il WKT non sa esprimere fedelmente devono
    /// fallire in serializzazione, e tutto il resto deve continuare a passare.
    ///
    /// La tabella e' la mappa misurata delle perdite: senza il controllo, le
    /// prime tre righe producevano testo che il nostro stesso parser rifiuta,
    /// e la quarta rileggeva una geometria diversa da quella scritta.
    #[test]
    fn le_forme_non_esprimibili_in_wkt_falliscono_invece_di_uscire_sbagliate() {
        let c = |x: f64, y: f64| WkbCoordinate {
            x,
            y,
            z: None,
            m: None,
        };
        let quadrato = vec![c(0.0, 0.0), c(1.0, 0.0), c(1.0, 1.0), c(0.0, 0.0)];
        let xy = |value| WkbGeometry {
            value,
            dimensions: CoordinateDimensions::Xy,
            srid: None,
        };

        for (nome, value) in [
            ("anello vuoto", WkbValue::Polygon(vec![vec![]])),
            (
                "interno vuoto",
                WkbValue::Polygon(vec![quadrato.clone(), vec![]]),
            ),
            (
                "membro con anello vuoto",
                WkbValue::MultiPolygon(vec![xy(WkbValue::Polygon(vec![vec![]]))]),
            ),
            (
                "membro LineString vuoto",
                WkbValue::MultiLineString(vec![xy(WkbValue::LineString(vec![]))]),
            ),
        ] {
            let errore = format_wkt(&xy(value)).expect_err(nome);
            assert!(
                errore.to_string().contains("senza coordinate"),
                "{nome}: messaggio inatteso {errore}"
            );
        }

        // Il controllo non deve allargarsi: queste passano e fanno round-trip.
        for (nome, value) in [
            ("poligono senza anelli", WkbValue::Polygon(vec![])),
            ("linestring vuota", WkbValue::LineString(vec![])),
            (
                "anello di una sola coordinata",
                WkbValue::Polygon(vec![vec![c(0.0, 0.0)]]),
            ),
            (
                "anello non chiuso",
                WkbValue::Polygon(vec![vec![c(0.0, 0.0), c(1.0, 0.0), c(1.0, 1.0)]]),
            ),
            (
                "multipoligono con membro senza anelli",
                WkbValue::MultiPolygon(vec![xy(WkbValue::Polygon(vec![]))]),
            ),
            ("poligono valido", WkbValue::Polygon(vec![quadrato])),
        ] {
            let geometria = xy(value);
            let testo = format_wkt(&geometria).expect(nome);
            assert_eq!(
                parse_wkt(&testo).expect(nome),
                geometria,
                "{nome}: round-trip di {testo}"
            );
        }
    }

    /// Lettura e scrittura devono avere lo stesso perimetro: se non sappiamo
    /// riscrivere una geometria, non dobbiamo accettarla nemmeno da testo.
    ///
    /// Non toglie nulla che funzionasse: `POLYGON(EMPTY)` non ha mai fatto
    /// round-trip, veniva riletto come poligono senza anelli. Il fuzz target
    /// `wkt_parse` asserisce esattamente questa simmetria — «WKT accettato deve
    /// essere serializzabile» — ed e' cosi' che l'asimmetria e' venuta fuori.
    #[test]
    fn cio_che_accettiamo_da_testo_lo_sappiamo_riscrivere() {
        for testo in [
            "POLYGON(EMPTY)",
            "MULTIPOLYGON((EMPTY))",
            "MULTILINESTRING(EMPTY)",
        ] {
            let esito = parse_wkt(testo);
            if let Ok(geometria) = &esito {
                format_wkt(geometria).unwrap_or_else(|errore| {
                    panic!("{testo}: accettato in lettura ma non riscrivibile: {errore}")
                });
            }
        }

        // Le forme vuote di primo livello restano accettate e riscrivibili.
        for testo in ["POLYGON EMPTY", "LINESTRING EMPTY", "MULTIPOLYGON(EMPTY)"] {
            let geometria = parse_wkt(testo).unwrap_or_else(|errore| panic!("{testo}: {errore}"));
            let riscritto =
                format_wkt(&geometria).unwrap_or_else(|errore| panic!("{testo}: {errore}"));
            assert_eq!(
                parse_wkt(&riscritto).unwrap_or_else(|errore| panic!("{testo}: {errore}")),
                geometria,
                "{testo}: round-trip via {riscritto}"
            );
        }
    }

    #[test]
    fn rejects_empty_point_and_mixed_collection_dimensions() {
        assert!(parse_wkt("POINT EMPTY").is_err());
        assert!(parse_wkt("GEOMETRYCOLLECTION(POINT(1 2),POINT Z(1 2 3))").is_err());
    }

    #[test]
    fn rejects_non_finite_coordinates_from_text_and_wkb() {
        assert!(parse_wkt("POINT (2e308 -1e-308)").is_err());
        assert!(parse_wkt("POINT ZM (1 2 NaN 4)").is_err());

        let geometry = WkbGeometry {
            value: WkbValue::Point(WkbCoordinate {
                x: f64::INFINITY,
                y: 2.0,
                z: None,
                m: None,
            }),
            dimensions: CoordinateDimensions::Xy,
            srid: None,
        };
        assert!(format_wkt(&geometry).is_err());
    }

    #[test]
    fn reusable_formatter_appends_and_preserves_buffer_on_error() {
        let geometry = parse_wkt("LINESTRING Z(0 1 2,3 4 5)").unwrap();
        let mut output = "prefix:".to_owned();
        format_wkt_into(&geometry, &mut output).unwrap();
        assert_eq!(
            parse_wkt(output.strip_prefix("prefix:").unwrap()).unwrap(),
            geometry
        );

        let invalid = WkbGeometry {
            value: WkbValue::Point(WkbCoordinate {
                x: f64::INFINITY,
                y: 2.0,
                z: None,
                m: None,
            }),
            dimensions: CoordinateDimensions::Xy,
            srid: None,
        };
        let before = output.clone();
        assert!(format_wkt_into(&invalid, &mut output).is_err());
        assert_eq!(output, before);
    }
}
