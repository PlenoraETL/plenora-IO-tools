//! Deserializzazione `GeoJSON` **limitata durante il parse** (lotto S12).
//!
//! # Perche' il cap in byte non basta
//!
//! La geometria di una feature viene gia' intercettata come `RawValue`, e la
//! sua lunghezza confrontata con il tetto per cella: e' la difesa che S5 ha
//! portato fino qui, ed e' esatta finche' la domanda e' «quanto e' lungo».
//!
//! Non e' la domanda che conta. Un megabyte di `[[1,2],[1,2],...]` sta sotto
//! qualunque cap ragionevole e produce cinquantamila `Vec` annidati, perche'
//! `serde_json::from_str::<geojson::Geometry>` costruisce **l'albero intero**
//! prima che un solo contatore lo veda. Il commento che stava qui lo diceva, e
//! rinviava a S12.
//!
//! # Che cosa fa questo modulo
//!
//! Deserializza direttamente nel nostro AST WKB, senza passare da
//! `geojson::Value`, e addebita ogni **posizione** e ogni **geometria figlia**
//! nel momento in cui serde gliela consegna. Il rifiuto arriva alla posizione
//! che supera il tetto: cio' che non e' stato letto non e' stato allocato.
//!
//! L'unita' di conteggio e' quella del bordo -- una coordinata o una geometria
//! figlia, come conta `inspect_geometry` -- la stessa del WKT progressivo e la
//! stessa lezione del lotto S11.
//!
//! # L'albero delle coordinate, e perche' esiste
//!
//! In JSON le chiavi non hanno ordine: `coordinates` puo' arrivare prima di
//! `type`, e allora non si sa ancora se quella lista sia una posizione, una
//! linea o un multipoligono. L'albero intermedio risolve l'ordine senza
//! rinunciare al confine: e' **anch'esso** limitato mentre si costruisce --
//! ogni posizione addebitata, ogni annidamento contato -- quindi un input
//! ostile paga il tetto prima di ottenere memoria.
//!
//! Una posizione e' una lista di soli numeri, ed e' li' che si addebita: e' la
//! stessa unita' che `position` in `geometry.rs` riconosce.
//!
//! # I tre rifiuti, e come si distinguono
//!
//! * **sintassi** -- JSON malformato, `type` assente o sconosciuto,
//!   dimensionalita' non uniforme: `DataMapping` con il codice del formato;
//! * **tetto** -- posizioni, figli o annidamento oltre la quota configurata:
//!   `ResourceLimit/LimitExceeded`;
//! * **cap in byte** -- invariato, applicato prima di leggere un byte.
//!
//! # Il limite effettivo sull'annidamento non e' solo il nostro
//!
//! `serde_json` ha un tetto di ricorsione suo -- 128 livelli JSON -- e ogni
//! livello `GeoJSON` ne costa due: l'oggetto e la sua lista. Il limite che un
//! input incontra davvero e' quindi il **minimo** fra `max_depth` e circa
//! sessantadue, e i due rifiuti non sono lo stesso rifiuto: il nostro porta
//! `ResourceLimit/LimitExceeded`, quello di serde arriva dal canale laterale
//! vuoto e diventa «geometria `GeoJSON` non valida», cioe' un errore di
//! formato.
//!
//! Attribuire al nostro tetto un rifiuto prodotto da serde sarebbe falso, e la
//! differenza si vede: con `max_depth` a 64 -- il valore di produzione -- e'
//! serde a rifiutare per primo, e il nostro non morde mai. Il punto d'ingresso
//! del fuzzing ne dichiara percio' 32, cosi' la campagna esercita **il
//! nostro**; una sonda misura la soglia e non la deduce.
//!
//! Sono tre cose diverse e portano tre codici diversi: dire «limite superato» a
//! chi ha scritto `"type": "Punto"` lo manderebbe ad allargare una quota che
//! non c'entra.

use std::cell::{Cell, RefCell};
use std::fmt;

use plenora_io_model::contract::CoordinateDimensions;
use plenora_io_model::limits::WkbLimits;
use plenora_io_model::wkb::{WkbCoordinate, WkbGeometry, WkbValue};
use plenora_io_model::{NumeroStrutturale, PlenoraIoError, PublicMessage};
use serde::de::{DeserializeSeed, Deserializer, Error as DeError, MapAccess, SeqAccess, Visitor};

use crate::geometry::{
    format_error as errore_di_formato, geometry_dimensions, position, require_uniform_dimensions,
};

/// Il budget della deserializzazione, piu' il canale laterale dell'errore.
///
/// L'errore non puo' uscire attraverso `serde`: il suo tipo appiattisce
/// categoria, fase e codice in un testo. E' lo stesso canale laterale che il
/// lettore delle feature usa gia' in questo driver, e per la stessa ragione.
pub struct Budget {
    componenti: Cell<usize>,
    /// Il tetto dichiarato, per poterlo **dire** nel rifiuto: `componenti`
    /// scende mentre si legge, e a chi deve allargare la quota serve il valore
    /// che ha configurato, non quello che resta.
    componenti_iniziali: usize,
    profondita_massima: usize,
    errore: RefCell<Option<PlenoraIoError>>,
}

impl Budget {
    pub const fn nuovo(limiti: &WkbLimits) -> Self {
        Self {
            componenti: Cell::new(limiti.max_components),
            componenti_iniziali: limiti.max_components,
            profondita_massima: limiti.max_depth,
            errore: RefCell::new(None),
        }
    }

    /// L'errore tipizzato, se la deserializzazione ne ha prodotto uno.
    pub fn errore(&self) -> Option<PlenoraIoError> {
        self.errore.borrow_mut().take()
    }

    fn ferma<E: DeError>(&self, errore: PlenoraIoError) -> E {
        // Il primo errore e' quello vero: quelli successivi sono la reazione
        // di serde al fatto che ci siamo fermati.
        let mut posto = self.errore.borrow_mut();
        if posto.is_none() {
            *posto = Some(errore);
        }
        E::custom("geometria GeoJSON rifiutata")
    }

    fn di_formato<E: DeError>(&self, messaggio: &PublicMessage) -> E {
        self.ferma(errore_di_formato(messaggio))
    }

    /// Addebita un componente: una posizione, o una geometria figlia.
    fn addebita<E: DeError>(&self) -> Result<(), E> {
        // `map_or_else` invece del `match`: e' la forma che clippy chiede, e
        // qui non nasconde niente -- i due rami restano quelli che erano.
        self.componenti.get().checked_sub(1).map_or_else(
            || {
                Err(
                    self.ferma(PlenoraIoError::limite_redatto(&PublicMessage::CuratedWith(
                        "componenti della geometria GeoJSON oltre il limite di",
                        NumeroStrutturale::Limite(driver_common::saturating_u64(
                            self.componenti_iniziali,
                        )),
                    ))),
                )
            },
            |rimasti| {
                self.componenti.set(rimasti);
                Ok(())
            },
        )
    }

    /// Addebita i **membri** di un aggregato, che in `GeoJSON` non hanno un
    /// oggetto proprio.
    ///
    /// In WKB un `MULTIPOINT` di due punti costa quattro componenti: due figli
    /// piu' le loro due coordinate. In `GeoJSON` i figli non esistono come
    /// oggetti -- sono le posizioni stesse -- quindi leggendo si addebitano
    /// solo le posizioni, e il conto resterebbe la meta'. Le mie sonde l'hanno
    /// trovato confrontando i due numeri.
    ///
    /// L'addebito arriva qui, a lettura finita, e non indebolisce il confine:
    /// la protezione viene dalle posizioni, che sono gia' state pagate una per
    /// una mentre si leggevano. Questo pareggia l'unita' di misura con il
    /// bordo, che e' l'altra cosa che deve valere.
    fn addebita_membri(&self, quanti: usize) -> Result<(), PlenoraIoError> {
        self.componenti.get().checked_sub(quanti).map_or_else(
            || {
                Err(PlenoraIoError::limite_redatto(&PublicMessage::CuratedWith(
                    "componenti della geometria GeoJSON oltre il limite di",
                    NumeroStrutturale::Limite(driver_common::saturating_u64(
                        self.componenti_iniziali,
                    )),
                )))
            },
            |rimasti| {
                self.componenti.set(rimasti);
                Ok(())
            },
        )
    }

    fn dentro_la_profondita<E: DeError>(&self, profondita: usize) -> Result<(), E> {
        if profondita > self.profondita_massima {
            return Err(
                self.ferma(PlenoraIoError::limite_redatto(&PublicMessage::CuratedWith(
                    "annidamento della geometria GeoJSON oltre il limite di",
                    NumeroStrutturale::Limite(driver_common::saturating_u64(
                        self.profondita_massima,
                    )),
                ))),
            );
        }
        Ok(())
    }
}

/// Quante ordinate una posizione puo' avere prima di non essere una posizione.
///
/// `position` ne ammette due o tre. La quarta e' gia' un rifiuto, e la quinta
/// serve solo a distinguere «quattro» da «di piu'»: oltre, la lista non e' una
/// posizione e non c'e' ragione di continuare a leggerla. Il numero e' piccolo
/// apposta -- e' il tetto sull'unica allocazione che il budget non copre.
const ORDINATE_MASSIME: usize = 4;

/// L'albero delle coordinate: una posizione, o una lista di alberi.
enum Albero {
    Posizione(Vec<f64>),
    Elenco(Vec<Self>),
}

/// Il seme dell'albero delle coordinate, che addebita mentre costruisce.
struct SemeAlbero<'a> {
    budget: &'a Budget,
    profondita: usize,
}

impl<'de> DeserializeSeed<'de> for SemeAlbero<'_> {
    type Value = Albero;

    fn deserialize<D: Deserializer<'de>>(self, d: D) -> Result<Self::Value, D::Error> {
        d.deserialize_any(self)
    }
}

impl<'de> Visitor<'de> for SemeAlbero<'_> {
    type Value = Albero;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("una posizione GeoJSON o una lista di posizioni")
    }

    fn visit_f64<E: DeError>(self, valore: f64) -> Result<Self::Value, E> {
        Ok(Albero::Posizione(vec![valore]))
    }

    fn visit_i64<E: DeError>(self, valore: i64) -> Result<Self::Value, E> {
        // La conversione e' esatta fino a 2^53 e approssima oltre: e' la
        // stessa che `serde_json` fa per un `f64`, e la geometria non ha un
        // modo di rappresentare un intero piu' grande.
        #[allow(clippy::cast_precision_loss)]
        self.visit_f64(valore as f64)
    }

    fn visit_u64<E: DeError>(self, valore: u64) -> Result<Self::Value, E> {
        #[allow(clippy::cast_precision_loss)]
        self.visit_f64(valore as f64)
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
        self.budget.dentro_la_profondita(self.profondita)?;
        let profondita = self.profondita.saturating_add(1);
        let mut figli: Vec<Albero> = Vec::new();
        let mut numeri = 0_usize;
        let mut elenchi = 0_usize;
        while let Some(figlio) = seq.next_element_seed(SemeAlbero {
            budget: self.budget,
            profondita,
        })? {
            // Un numero arriva come `Posizione` di un elemento solo: e' la
            // forma in cui il visitor rappresenta uno scalare, e distingue
            // una lista di numeri -- che e' una posizione -- da una lista di
            // liste.
            if matches!(&figlio, Albero::Posizione(ordinate) if ordinate.len() == 1) {
                numeri += 1;
            } else {
                elenchi += 1;
            }

            // Le due condizioni che tengono **limitata** questa lista.
            //
            // Senza, una posizione era l'unica cosa che si accumulava prima di
            // essere addebitata: `[1,1,1,...]` costava un milione di `Vec`
            // minuscoli e un componente solo, e a fermarlo restava il solo cap
            // in byte -- cioe' proprio la difesa che questo lotto esiste per
            // non lasciare da sola.
            //
            // Una lista di coordinate GeoJSON e' o tutta numeri -- una
            // posizione, al piu' quattro ordinate -- o tutta liste, e allora
            // ogni figlia si e' gia' addebitata da se'. Mescolarle non e' una
            // forma valida in nessuno dei due confini, e riconoscerlo subito
            // costa un confronto.
            if numeri > ORDINATE_MASSIME {
                return Err(self.budget.di_formato(&PublicMessage::CuratedWith(
                    "posizione GeoJSON con piu' ordinate del massimo di",
                    NumeroStrutturale::Limite(driver_common::saturating_u64(ORDINATE_MASSIME)),
                )));
            }
            if numeri > 0 && elenchi > 0 {
                return Err(self.budget.di_formato(&PublicMessage::Curated(
                    "coordinates GeoJSON con numeri e liste nella stessa lista",
                )));
            }
            figli.push(figlio);
        }

        // La terza condizione, e chiude l'ultima via in cui un nodo poteva
        // entrare nell'albero **senza pagare**.
        //
        // Una lista vuota non e' una posizione e non contiene niente che si
        // addebiti: `[[],[],[],...]` costava zero componenti e cresceva finche'
        // il cap in byte non lo fermava, cioe' dopo aver costruito tutto
        // l'albero. E' lo stesso amplificatore delle ordinate, nella variante
        // «nodi strutturali senza coordinate».
        //
        // Rifiutarla subito non cambia l'insieme accettato: una lista vuota non
        // puo' far parte di una geometria valida in nessuno dei due confini --
        // senza coordinate, `posizioni` non ha una dimensionalita' da dedurre e
        // `anelli` non ha anelli -- quindi il rifiuto arrivava comunque, solo
        // alla fine invece che qui.
        if figli.is_empty() {
            return Err(self.budget.di_formato(&PublicMessage::Curated(
                "coordinates GeoJSON con una lista vuota",
            )));
        }

        if elenchi == 0 {
            // Una posizione: e' qui che si addebita, ed e' la stessa unita'
            // che il bordo conta. L'addebito arriva dopo aver letto al piu'
            // quattro ordinate, non dopo averne lette quante ne arrivano.
            self.budget.addebita()?;
            let ordinate = figli
                .into_iter()
                .filter_map(|figlio| match figlio {
                    Albero::Posizione(ordinate) => ordinate.first().copied(),
                    Albero::Elenco(_) => None,
                })
                .collect();
            return Ok(Albero::Posizione(ordinate));
        }
        Ok(Albero::Elenco(figli))
    }
}

impl Albero {
    /// Le posizioni di una lista di posizioni: `LineString`, `MultiPoint`.
    fn posizioni(
        &self,
        budget: &Budget,
    ) -> Result<(Vec<WkbCoordinate>, CoordinateDimensions), PlenoraIoError> {
        let elenco = match self {
            Self::Elenco(rami) => rami,
            Self::Posizione(_) => {
                return Err(errore_di_formato(&PublicMessage::Curated(
                    "coordinates GeoJSON con annidamento diverso da quello del tipo",
                )))
            }
        };
        let mut lette = Vec::with_capacity(elenco.len());
        let mut dimensioni = None;
        for ramo in elenco {
            let ordinate = match ramo {
                Self::Posizione(ordinate) => ordinate,
                Self::Elenco(_) => {
                    return Err(errore_di_formato(&PublicMessage::Curated(
                        "coordinates GeoJSON con annidamento diverso da quello del tipo",
                    )))
                }
            };
            let (coordinata, corrente) = position(ordinate).map_err(|m| errore_di_formato(&m))?;
            require_uniform_dimensions(&mut dimensioni, corrente)
                .map_err(|m| errore_di_formato(&m))?;
            lette.push(coordinata);
        }
        let _ = budget;
        let dimensioni = dimensioni.ok_or_else(|| {
            errore_di_formato(&PublicMessage::Curated(
                "geometria GeoJSON senza coordinate",
            ))
        })?;
        Ok((lette, dimensioni))
    }

    /// Gli anelli di un poligono.
    fn anelli(
        &self,
        budget: &Budget,
    ) -> Result<(Vec<Vec<WkbCoordinate>>, CoordinateDimensions), PlenoraIoError> {
        let elenco = match self {
            Self::Elenco(rami) => rami,
            Self::Posizione(_) => {
                return Err(errore_di_formato(&PublicMessage::Curated(
                    "coordinates GeoJSON con annidamento diverso da quello del tipo",
                )))
            }
        };
        let mut letti = Vec::with_capacity(elenco.len());
        let mut dimensioni = None;
        for ramo in elenco {
            let (anello, corrente) = ramo.posizioni(budget)?;
            require_uniform_dimensions(&mut dimensioni, corrente)
                .map_err(|m| errore_di_formato(&m))?;
            letti.push(anello);
        }
        let dimensioni = dimensioni.ok_or_else(|| {
            errore_di_formato(&PublicMessage::Curated("Polygon GeoJSON senza anelli"))
        })?;
        Ok((letti, dimensioni))
    }

    /// Una singola posizione: `Point`.
    fn posizione(&self) -> Result<(WkbCoordinate, CoordinateDimensions), PlenoraIoError> {
        match self {
            Self::Posizione(ordinate) => position(ordinate).map_err(|m| errore_di_formato(&m)),
            Self::Elenco(_) => Err(errore_di_formato(&PublicMessage::Curated(
                "coordinates GeoJSON con annidamento diverso da quello del tipo",
            ))),
        }
    }
}

/// Il tipo dichiarato dal membro `type`.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Tipo {
    Point,
    MultiPoint,
    LineString,
    MultiLineString,
    Polygon,
    MultiPolygon,
    GeometryCollection,
}

impl Tipo {
    fn dal_nome(nome: &str) -> Option<Self> {
        match nome {
            "Point" => Some(Self::Point),
            "MultiPoint" => Some(Self::MultiPoint),
            "LineString" => Some(Self::LineString),
            "MultiLineString" => Some(Self::MultiLineString),
            "Polygon" => Some(Self::Polygon),
            "MultiPolygon" => Some(Self::MultiPolygon),
            "GeometryCollection" => Some(Self::GeometryCollection),
            _ => None,
        }
    }
}

/// Il seme di una geometria: legge l'oggetto e ne costruisce l'AST.
pub struct SemeGeometria<'a> {
    pub budget: &'a Budget,
    pub profondita: usize,
}

impl<'de> DeserializeSeed<'de> for SemeGeometria<'_> {
    type Value = WkbGeometry;

    fn deserialize<D: Deserializer<'de>>(self, d: D) -> Result<Self::Value, D::Error> {
        d.deserialize_map(self)
    }
}

impl<'de> Visitor<'de> for SemeGeometria<'_> {
    type Value = WkbGeometry;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("un oggetto geometria GeoJSON")
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
        self.budget.dentro_la_profondita(self.profondita)?;
        let mut tipo: Option<Tipo> = None;
        let mut coordinate: Option<Albero> = None;
        let mut figlie: Option<Vec<WkbGeometry>> = None;

        while let Some(chiave) = map.next_key::<String>()? {
            match chiave.as_str() {
                "type" => {
                    let nome = map.next_value::<String>()?;
                    let letto = Tipo::dal_nome(&nome).ok_or_else(|| {
                        self.budget.di_formato::<A::Error>(&PublicMessage::Curated(
                            "tipo di geometria GeoJSON non riconosciuto",
                        ))
                    })?;
                    tipo = Some(letto);
                }
                "coordinates" => {
                    coordinate = Some(map.next_value_seed(SemeAlbero {
                        budget: self.budget,
                        profondita: self.profondita,
                    })?);
                }
                "geometries" => {
                    figlie = Some(map.next_value_seed(SemeGeometrie {
                        budget: self.budget,
                        profondita: self.profondita,
                    })?);
                }
                // `bbox`, `crs` e i membri estranei restano ignorati, come li
                // ignorava la deserializzazione precedente: il lotto sposta il
                // confine, non l'insieme accettato.
                _ => {
                    map.next_value::<serde::de::IgnoredAny>()?;
                }
            }
        }

        let tipo = tipo.ok_or_else(|| {
            self.budget.di_formato::<A::Error>(&PublicMessage::Curated(
                "geometria GeoJSON senza campo 'type'",
            ))
        })?;
        costruisci(tipo, coordinate.as_ref(), figlie, self.budget)
            .map_err(|errore| self.budget.ferma(errore))
    }
}

/// Il seme della lista `geometries` di una `GeometryCollection`.
struct SemeGeometrie<'a> {
    budget: &'a Budget,
    profondita: usize,
}

impl<'de> DeserializeSeed<'de> for SemeGeometrie<'_> {
    type Value = Vec<WkbGeometry>;

    fn deserialize<D: Deserializer<'de>>(self, d: D) -> Result<Self::Value, D::Error> {
        d.deserialize_seq(self)
    }
}

impl<'de> Visitor<'de> for SemeGeometrie<'_> {
    type Value = Vec<WkbGeometry>;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("una lista di geometrie GeoJSON")
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
        let profondita = self.profondita.saturating_add(1);
        let mut figlie = Vec::new();
        loop {
            // L'addebito arriva **dopo** aver ottenuto la figlia, e non prima
            // di provare a leggerla. La prima stesura addebitava per prima
            // cosa, poi restituiva l'addebito di troppo quando la lista
            // finiva: a quota esatta pero' era il tentativo speculativo a
            // sforare, e la restituzione non arrivava mai. L'ha trovato la
            // sonda che prova il confine con `n` e con `n-1`.
            //
            // Non si perde niente: una figlia costa almeno il proprio
            // addebito -- una collezione di un milione di figlie ne paga un
            // milione -- e il budget morde comunque mentre si legge.
            let Some(letta) = seq.next_element_seed(SemeGeometria {
                budget: self.budget,
                profondita,
            })?
            else {
                break;
            };
            self.budget.addebita()?;
            figlie.push(letta);
        }
        Ok(figlie)
    }
}

/// Costruisce la geometria dai pezzi letti, con le regole di `geometry.rs`.
fn costruisci(
    tipo: Tipo,
    coordinate: Option<&Albero>,
    figlie: Option<Vec<WkbGeometry>>,
    budget: &Budget,
) -> Result<WkbGeometry, PlenoraIoError> {
    if tipo == Tipo::GeometryCollection {
        let figlie = figlie.ok_or_else(|| {
            errore_di_formato(&PublicMessage::Curated(
                "GeometryCollection GeoJSON senza 'geometries'",
            ))
        })?;
        let dimensioni = geometry_dimensions(&figlie, "GeometryCollection GeoJSON vuota")
            .map_err(|m| errore_di_formato(&m))?;
        return Ok(WkbGeometry {
            value: WkbValue::GeometryCollection(figlie),
            dimensions: dimensioni,
            srid: None,
        });
    }

    let coordinate = coordinate.ok_or_else(|| {
        errore_di_formato(&PublicMessage::Curated(
            "geometria GeoJSON senza 'coordinates'",
        ))
    })?;

    let (valore, dimensioni) = match tipo {
        Tipo::Point => {
            let (sola, dimensioni) = coordinate.posizione()?;
            (WkbValue::Point(sola), dimensioni)
        }
        Tipo::LineString => {
            let (lette, dimensioni) = coordinate.posizioni(budget)?;
            (WkbValue::LineString(lette), dimensioni)
        }
        Tipo::MultiPoint => {
            let (lette, dimensioni) = coordinate.posizioni(budget)?;
            budget.addebita_membri(lette.len())?;
            let figlie = lette
                .into_iter()
                .map(|coordinata| WkbGeometry {
                    value: WkbValue::Point(coordinata),
                    dimensions: dimensioni,
                    srid: None,
                })
                .collect();
            (WkbValue::MultiPoint(figlie), dimensioni)
        }
        Tipo::Polygon => {
            let (anelli, dimensioni) = coordinate.anelli(budget)?;
            (WkbValue::Polygon(anelli), dimensioni)
        }
        Tipo::MultiLineString => {
            let elenco = match coordinate {
                Albero::Elenco(rami) => rami,
                Albero::Posizione(_) => {
                    return Err(errore_di_formato(&PublicMessage::Curated(
                        "coordinates GeoJSON con annidamento diverso da quello del tipo",
                    )))
                }
            };
            budget.addebita_membri(elenco.len())?;
            let mut membri = Vec::with_capacity(elenco.len());
            for ramo in elenco {
                let (lette, dimensioni) = ramo.posizioni(budget)?;
                membri.push(WkbGeometry {
                    value: WkbValue::LineString(lette),
                    dimensions: dimensioni,
                    srid: None,
                });
            }
            let dimensioni = geometry_dimensions(&membri, "MultiLineString GeoJSON vuota")
                .map_err(|m| errore_di_formato(&m))?;
            (WkbValue::MultiLineString(membri), dimensioni)
        }
        Tipo::MultiPolygon => {
            let elenco = match coordinate {
                Albero::Elenco(rami) => rami,
                Albero::Posizione(_) => {
                    return Err(errore_di_formato(&PublicMessage::Curated(
                        "coordinates GeoJSON con annidamento diverso da quello del tipo",
                    )))
                }
            };
            budget.addebita_membri(elenco.len())?;
            let mut membri = Vec::with_capacity(elenco.len());
            for ramo in elenco {
                let (anelli, dimensioni) = ramo.anelli(budget)?;
                membri.push(WkbGeometry {
                    value: WkbValue::Polygon(anelli),
                    dimensions: dimensioni,
                    srid: None,
                });
            }
            let dimensioni = geometry_dimensions(&membri, "MultiPolygon GeoJSON vuota")
                .map_err(|m| errore_di_formato(&m))?;
            (WkbValue::MultiPolygon(membri), dimensioni)
        }
        Tipo::GeometryCollection => unreachable!("trattata sopra"),
    };
    Ok(WkbGeometry {
        value: valore,
        dimensions: dimensioni,
        srid: None,
    })
}

/// Analizza una geometria `GeoJSON` applicando i tetti **durante** il parse.
///
/// # Errors
///
/// JSON malformato, geometria strutturalmente invalida, o superamento di uno
/// dei tetti dichiarati in `limiti`.
pub fn analizza(testo: &str, limiti: &WkbLimits) -> plenora_io_model::Result<WkbGeometry> {
    let budget = Budget::nuovo(limiti);
    let mut deserializzatore = serde_json::Deserializer::from_str(testo);
    let esito = SemeGeometria {
        budget: &budget,
        profondita: 0,
    }
    .deserialize(&mut deserializzatore);
    match esito {
        Ok(geometria) => {
            // La coda dopo la geometria non e' geometria. Stessa scelta del
            // WKT progressivo, e per la stessa ragione.
            deserializzatore.end().map_err(|_| {
                errore_di_formato(&PublicMessage::Curated(
                    "testo residuo dopo la geometria GeoJSON",
                ))
            })?;
            Ok(geometria)
        }
        Err(_) => Err(budget.errore().unwrap_or_else(|| {
            // Il canale laterale e' vuoto: l'errore viene da serde, cioe' e'
            // JSON malformato e non una nostra regola.
            errore_di_formato(&PublicMessage::Curated("geometria GeoJSON non valida"))
        })),
    }
}

/// Quante posizioni e figlie l'analisi ha addebitato per un testo accettato.
#[cfg(test)]
fn componenti_usati(testo: &str, limiti: &WkbLimits) -> usize {
    let budget = Budget::nuovo(limiti);
    let mut deserializzatore = serde_json::Deserializer::from_str(testo);
    let esito = SemeGeometria {
        budget: &budget,
        profondita: 0,
    }
    .deserialize(&mut deserializzatore);
    assert!(esito.is_ok(), "il testo doveva essere accettato: {testo}");
    limiti.max_components - budget.componenti.get()
}

#[cfg(test)]
mod sonde {
    use super::{analizza, componenti_usati};
    use plenora_io_model::limits::WkbLimits;
    use plenora_io_model::wkb::{encode_wkb, inspect_wkb, WkbFlavor};
    use plenora_io_model::IoErrorCode;

    fn stretti(componenti: usize, profondita: usize) -> WkbLimits {
        WkbLimits {
            max_components: componenti,
            max_depth: profondita,
            ..WkbLimits::default()
        }
    }

    /// Una `LineString` con il numero di posizioni indicato.
    fn linea(posizioni: usize) -> String {
        let mut testo = String::from(r#"{"type":"LineString","coordinates":["#);
        for indice in 0..posizioni {
            if indice > 0 {
                testo.push(',');
            }
            testo.push_str("[1,2]");
        }
        testo.push_str("]}");
        testo
    }

    /// **Il fatto che il lotto S12 esiste per stabilire, per il `GeoJSON`.**
    ///
    /// Il cap in byte diceva quanto puo' essere lungo l'input;
    /// `serde_json::from_str::<geojson::Geometry>` costruiva l'albero intero
    /// prima che un contatore lo vedesse. Qui il rifiuto arriva alla posizione
    /// che supera il tetto, e la coda del testo non viene nemmeno letta.
    #[test]
    fn il_rifiuto_arriva_prima_della_fine_del_testo() {
        let testo = linea(5_000);
        let limiti = stretti(10, 64);
        let errore = analizza(&testo, &limiti).expect_err("il tetto deve fermare l'analisi");
        assert_eq!(errore.code, IoErrorCode::LimitExceeded);

        // La prova che non e' arrivata in fondo: la stessa linea con una coda
        // che non e' JSON. Se l'analisi leggesse tutto prima di misurare, il
        // rifiuto sarebbe di sintassi.
        let mut con_coda = linea(5_000);
        con_coda.push_str(" questa coda non e' JSON");
        let errore = analizza(&con_coda, &limiti).expect_err("il tetto deve fermare l'analisi");
        assert_eq!(
            errore.code,
            IoErrorCode::LimitExceeded,
            "atteso il rifiuto del tetto, non quello della coda"
        );
    }

    /// I tre rifiuti sono tre, e si distinguono.
    ///
    /// Dire «limite superato» a chi ha scritto `"type": "Punto"` lo manderebbe
    /// ad allargare una quota che non c'entra.
    #[test]
    fn i_rifiuti_portano_il_codice_della_loro_causa() {
        let limiti = WkbLimits::default();
        for (testo, atteso, perche) in [
            (
                r#"{"type":"Point","coordinates":[1,2]"#,
                IoErrorCode::Format,
                "JSON troncato",
            ),
            (
                r#"{"coordinates":[1,2]}"#,
                IoErrorCode::Format,
                "type assente",
            ),
            (
                r#"{"type":"Punto","coordinates":[1,2]}"#,
                IoErrorCode::Format,
                "type sconosciuto",
            ),
            (
                r#"{"type":"LineString","coordinates":[[1,2],[1,2,3]]}"#,
                IoErrorCode::Format,
                "dimensionalita' non uniforme",
            ),
            (
                r#"{"type":"Point","coordinates":[1,2,3,4]}"#,
                IoErrorCode::Format,
                "posizione con quattro ordinate",
            ),
            (
                r#"{"type":"Point","coordinates":[1,2]} e poi altro"#,
                IoErrorCode::Format,
                "testo residuo dopo la geometria",
            ),
        ] {
            let errore = analizza(testo, &limiti).expect_err(perche);
            assert_eq!(errore.code, atteso, "{perche}: {testo}");
        }

        // Il tetto, che e' un'altra cosa.
        let errore = analizza(&linea(50), &stretti(10, 64)).expect_err("tetto sui componenti");
        assert_eq!(errore.code, IoErrorCode::LimitExceeded);

        let annidato = r#"{"type":"GeometryCollection","geometries":[{"type":"GeometryCollection","geometries":[{"type":"Point","coordinates":[1,2]}]}]}"#;
        let errore = analizza(annidato, &stretti(1_000, 1)).expect_err("tetto sull'annidamento");
        assert_eq!(errore.code, IoErrorCode::LimitExceeded);
        assert!(analizza(annidato, &stretti(1_000, 4)).is_ok());
    }

    /// Il tetto sui componenti, provato **esattamente al confine**.
    ///
    /// Il target di fuzzing non arriva a centomila posizioni -- non stanno nel
    /// cap del harness -- quindi sotto fuzzing quel ramo non e' esercitato, e
    /// il registro della misura lo dice. Qui si prova al confine: `n` passa,
    /// `n+1` no.
    #[test]
    fn il_tetto_sui_componenti_e_esatto() {
        let casi: [(&str, usize); 5] = [
            (r#"{"type":"Point","coordinates":[1,2]}"#, 1),
            (
                r#"{"type":"LineString","coordinates":[[0,0],[1,1],[2,2]]}"#,
                3,
            ),
            (
                r#"{"type":"Polygon","coordinates":[[[0,0],[1,0],[1,1],[0,0]]]}"#,
                4,
            ),
            (r#"{"type":"MultiPoint","coordinates":[[0,0],[1,1]]}"#, 4),
            (
                r#"{"type":"GeometryCollection","geometries":[{"type":"Point","coordinates":[1,2]},{"type":"LineString","coordinates":[[0,0],[1,1]]}]}"#,
                5,
            ),
        ];
        for (testo, costo) in casi {
            let esatto = WkbLimits {
                max_components: costo,
                ..WkbLimits::default()
            };
            let stretto = WkbLimits {
                max_components: costo - 1,
                ..WkbLimits::default()
            };
            assert!(
                analizza(testo, &esatto).is_ok(),
                "{testo} costa {costo} componenti e con {costo} deve passare"
            );
            let errore = analizza(testo, &stretto)
                .expect_err(&format!("{testo} con {} deve fallire", costo - 1));
            assert_eq!(
                errore.code,
                IoErrorCode::LimitExceeded,
                "{testo}: al confine il rifiuto e' del tetto"
            );
            assert_eq!(componenti_usati(testo, &esatto), costo, "{testo}");
        }
    }

    /// La soglia oltre la quale rifiuta **serde**, misurata e non dedotta.
    ///
    /// Il limite effettivo sull'annidamento e' il minimo fra il nostro
    /// `max_depth` e il tetto di ricorsione di `serde_json`. Sono due rifiuti
    /// diversi e portano due codici diversi: attribuire al nostro tetto un
    /// rifiuto prodotto da serde sarebbe falso, e questa sonda e' il posto in
    /// cui la differenza resta scritta.
    #[test]
    fn oltre_una_certa_profondita_rifiuta_serde_e_non_noi() {
        let annidata = |livelli: usize| {
            let mut geometria = r#"{"type":"Point","coordinates":[1,2]}"#.to_owned();
            for _ in 0..livelli {
                geometria =
                    format!(r#"{{"type":"GeometryCollection","geometries":[{geometria}]}}"#);
            }
            geometria
        };
        // Con un tetto largo il nostro non morde: a rifiutare e' serde, e
        // l'errore e' di formato.
        let largo = WkbLimits {
            max_depth: 1_000,
            ..WkbLimits::default()
        };
        assert!(
            analizza(&annidata(40), &largo).is_ok(),
            "quaranta livelli passano"
        );
        let errore = analizza(&annidata(200), &largo).expect_err("serde rifiuta");
        assert_eq!(
            errore.code,
            IoErrorCode::Format,
            "il rifiuto di serde e' un errore di formato, non un nostro tetto"
        );

        // Con un tetto stretto mordiamo noi, e il codice lo dice.
        let stretto = WkbLimits {
            max_depth: 8,
            ..WkbLimits::default()
        };
        let errore = analizza(&annidata(20), &stretto).expect_err("il nostro tetto");
        assert_eq!(errore.code, IoErrorCode::LimitExceeded);
    }

    /// L'albero delle coordinate non accumula prima di addebitare.
    ///
    /// Era il buco: una lista di soli numeri costava un componente qualunque
    /// fosse la sua lunghezza, e a fermarla restava il solo cap in byte. Ora
    /// una lista e' o una posizione -- al piu' quattro ordinate -- o una lista
    /// di liste, dove ogni figlia si e' gia' addebitata.
    #[test]
    fn una_lista_di_numeri_non_cresce_oltre_una_posizione() {
        let limiti = WkbLimits::default();

        // Mille ordinate in una posizione: rifiutate dopo averne lette cinque.
        let mut ordinate = String::from(r#"{"type":"Point","coordinates":["#);
        for indice in 0..1_000 {
            if indice > 0 {
                ordinate.push(',');
            }
            ordinate.push('1');
        }
        ordinate.push_str("]}");
        let errore = analizza(&ordinate, &limiti).expect_err("non e' una posizione");
        assert_eq!(errore.code, IoErrorCode::Format);

        // Numeri e liste nella stessa lista non sono una forma valida in
        // nessuno dei due confini, e riconoscerlo subito costa un confronto.
        let misto = r#"{"type":"LineString","coordinates":[1,2,[3,4]]}"#;
        assert_eq!(
            analizza(misto, &limiti).expect_err("mista").code,
            IoErrorCode::Format
        );
        assert!(come_prima(misto).is_err(), "e il confine precedente pure");
    }

    /// Nemmeno una lista **vuota** entra nell'albero senza pagare.
    ///
    /// Era la terza via dell'amplificatore, nella variante «nodi strutturali
    /// senza coordinate»: `[[],[],[],...]` costava zero componenti e cresceva
    /// finche' il cap in byte non lo fermava -- cioe' dopo aver costruito tutto
    /// l'albero.
    ///
    /// La coda non e' JSON, ed e' li' la prova: se l'analisi arrivasse in fondo
    /// prima di rifiutare, il rifiuto sarebbe di sintassi. Siccome si ferma
    /// alla **prima** lista vuota, la coda non viene nemmeno letta.
    #[test]
    fn una_lista_vuota_ferma_l_analisi_alla_prima() {
        let limiti = WkbLimits::default();
        let mut testo = String::from(r#"{"type":"MultiPolygon","coordinates":["#);
        for indice in 0..20_000 {
            if indice > 0 {
                testo.push(',');
            }
            testo.push_str("[]");
        }
        testo.push_str("], questa coda non e' JSON e non deve essere letta");

        let errore = analizza(&testo, &limiti).expect_err("la lista vuota ferma l'analisi");
        assert_eq!(
            errore.code,
            IoErrorCode::Format,
            "atteso il rifiuto della lista vuota, non quello della coda"
        );

        // E il confine precedente la rifiutava gia', quindi l'insieme accettato
        // non cambia: cambia **quando** ci si ferma.
        for testo in [
            r#"{"type":"MultiPolygon","coordinates":[[]]}"#,
            r#"{"type":"Polygon","coordinates":[[]]}"#,
            r#"{"type":"LineString","coordinates":[[]]}"#,
        ] {
            assert!(analizza(testo, &limiti).is_err(), "{testo}");
            assert!(come_prima(testo).is_err(), "{testo}: e prima pure");
        }
    }

    /// L'unita' di conteggio e' quella del bordo, e non «una simile».
    ///
    /// Stessa sonda del WKT progressivo, stesso confronto: cio' che l'analisi
    /// addebita leggendo il testo deve essere cio' che `inspect_wkb` conta
    /// sulla stessa geometria in WKB.
    #[test]
    fn i_componenti_coincidono_con_quelli_del_parser_condiviso() {
        let limiti = WkbLimits::default();
        for testo in [
            r#"{"type":"Point","coordinates":[1,2]}"#,
            r#"{"type":"Point","coordinates":[1,2,3]}"#,
            r#"{"type":"LineString","coordinates":[[0,0],[1,1],[2,2]]}"#,
            r#"{"type":"MultiPoint","coordinates":[[0,0],[1,1]]}"#,
            r#"{"type":"Polygon","coordinates":[[[0,0],[1,0],[1,1],[0,0]]]}"#,
            r#"{"type":"Polygon","coordinates":[[[0,0],[1,0],[1,1],[0,0]],[[0,0],[1,0],[1,1],[0,0]]]}"#,
            r#"{"type":"MultiLineString","coordinates":[[[0,0],[1,1]],[[2,2],[3,3]]]}"#,
            r#"{"type":"MultiPolygon","coordinates":[[[[0,0],[1,0],[1,1],[0,0]]]]}"#,
            r#"{"type":"GeometryCollection","geometries":[{"type":"Point","coordinates":[1,2]},{"type":"LineString","coordinates":[[0,0],[1,1]]}]}"#,
        ] {
            let nostri = componenti_usati(testo, &limiti);
            let geometria = analizza(testo, &limiti).expect("campione valido");
            let byte = encode_wkb(&geometria, WkbFlavor::Iso).expect("codificabile");
            let ispezione = inspect_wkb(&byte, &limiti).expect("ispezionabile");
            assert_eq!(
                nostri, ispezione.components,
                "{testo}: l'analisi addebita {nostri}, il parser condiviso conta {}",
                ispezione.components
            );
        }
    }

    /// **La prova che la grammatica non e' cambiata.**
    ///
    /// Come per il WKT: il confronto e' con il confine **precedente** -- la
    /// deserializzazione in `geojson::Geometry` piu' `convert` -- su un corpus
    /// generato. La regola e' la stessa, e la riga che conta e' l'ultima:
    /// accettare piu' di prima e' la regressione che nessun test esistente
    /// vedrebbe.
    ///
    /// # Qui non c'e' eccezione, e non e' un caso
    ///
    /// Il WKT ne ha una -- il testo dopo la geometria, che la crate `wkt`
    /// ignorava -- e il `GeoJSON` no: `serde_json::from_str` pretende gia' che
    /// l'input sia **un** valore e nient'altro, quindi la stessa scelta era
    /// gia' quella del confine precedente. L'analisi progressiva la conserva
    /// chiamando `end()` sul deserializzatore, e i due insiemi coincidono
    /// esattamente: nessuna divergenza, in nessuna delle due direzioni.
    #[test]
    fn accetta_esattamente_cio_che_accettava_il_confine_precedente() {
        let limiti = WkbLimits::default();
        let corpus = corpus_di_confronto();
        assert!(corpus.len() > 80, "corpus troppo piccolo: {}", corpus.len());

        for testo in &corpus {
            let prima = come_prima(testo);
            let dopo = analizza(testo, &limiti);
            match (prima, dopo) {
                (Ok(prima), Ok(dopo)) => {
                    assert_eq!(prima, dopo, "geometrie diverse per «{testo}»");
                }
                (Err(_), Err(_)) => {}
                (Ok(_), Err(errore)) => {
                    panic!("«{testo}» era accettato e ora e' rifiutato: {errore}");
                }
                (Err(errore), Ok(_)) => {
                    panic!("«{testo}» era rifiutato ({errore}) e ora e' accettato");
                }
            }
        }
    }

    /// Il confine come era prima del lotto S12.
    fn come_prima(testo: &str) -> plenora_io_model::Result<plenora_io_model::wkb::WkbGeometry> {
        let gj: geojson::Geometry = serde_json::from_str(testo).map_err(|_| {
            crate::geometry::format_error(&plenora_io_model::PublicMessage::Curated(
                "geometria GeoJSON non valida",
            ))
        })?;
        crate::geometry::converti_per_confronto(&gj.value)
    }

    /// Il corpus, generato per combinazione.
    fn corpus_di_confronto() -> Vec<String> {
        let mut corpus = Vec::new();
        // Le negative ci sono per una ragione precisa: `serde_json` consegna
        // un intero non negativo come `u64` e uno negativo come `i64`, quindi
        // senza di loro meta' del visitor non veniva mai eseguita -- e le
        // longitudini negative sono la meta' del mondo. L'ha trovato la
        // copertura delle righe cambiate, non questa sonda.
        let posizioni = ["[1,2]", "[1,2,3]", "[-1,-2]", "[-1.5,2.5,-3]"];
        for posizione in posizioni {
            let due = format!("{posizione},{posizione}");
            let quattro = format!("{posizione},{posizione},{posizione},{posizione}");
            for forma in [
                format!(r#"{{"type":"Point","coordinates":{posizione}}}"#),
                format!(r#"{{"type":"MultiPoint","coordinates":[{due}]}}"#),
                r#"{"type":"MultiPoint","coordinates":[]}"#.to_owned(),
                format!(r#"{{"type":"LineString","coordinates":[{due}]}}"#),
                r#"{"type":"LineString","coordinates":[]}"#.to_owned(),
                format!(r#"{{"type":"Polygon","coordinates":[[{quattro}]]}}"#),
                format!(r#"{{"type":"Polygon","coordinates":[[{quattro}],[{quattro}]]}}"#),
                r#"{"type":"Polygon","coordinates":[]}"#.to_owned(),
                format!(r#"{{"type":"MultiLineString","coordinates":[[{due}],[{due}]]}}"#),
                r#"{"type":"MultiLineString","coordinates":[]}"#.to_owned(),
                format!(r#"{{"type":"MultiPolygon","coordinates":[[[{quattro}]]]}}"#),
                r#"{"type":"MultiPolygon","coordinates":[]}"#.to_owned(),
                format!(
                    r#"{{"type":"GeometryCollection","geometries":[{{"type":"Point","coordinates":{posizione}}}]}}"#
                ),
                r#"{"type":"GeometryCollection","geometries":[]}"#.to_owned(),
            ] {
                corpus.push(forma.clone());
                // Le stesse forme con le chiavi in ordine inverso: in JSON non
                // hanno ordine, e il confine nuovo deve reggerlo.
                corpus.push(
                    forma
                        .replace(r#"{"type":"#, r#"{"XXtype":"#)
                        .replace(r#","coordinates":"#, r#","type":"#)
                        .replace(r#"{"XXtype":"#, r#"{"coordinates":"#),
                );
                // E con un membro estraneo, che entrambi ignorano.
                corpus.push(forma.replacen('{', r#"{"bbox":[0,0,1,1],"#, 1));
            }
        }

        for storpiatura in [
            "",
            "null",
            "[]",
            "{}",
            r#"{"type":"Point"}"#,
            r#"{"coordinates":[1,2]}"#,
            r#"{"type":"Punto","coordinates":[1,2]}"#,
            r#"{"type":"Point","coordinates":[]}"#,
            r#"{"type":"Point","coordinates":[1]}"#,
            r#"{"type":"Point","coordinates":[1,2,3,4]}"#,
            r#"{"type":"Point","coordinates":[1,"due"]}"#,
            r#"{"type":"Point","coordinates":[[1,2]]}"#,
            r#"{"type":"LineString","coordinates":[[1,2],[1,2,3]]}"#,
            r#"{"type":"LineString","coordinates":[1,2]}"#,
            r#"{"type":"Polygon","coordinates":[[1,2]]}"#,
            r#"{"type":"GeometryCollection","geometries":[{"type":"Point","coordinates":[1,2]},{"type":"Point","coordinates":[1,2,3]}]}"#,
            r#"{"type":"GeometryCollection","coordinates":[1,2]}"#,
            r#"{"type":"Point","coordinates":[1,2]} e poi altro"#,
            r#"{"type":"Point","coordinates":[1,2]}{"type":"Point","coordinates":[3,4]}"#,
            r#"{"type":"Point","coordinates":[1,2],"type":"LineString"}"#,
        ] {
            corpus.push(storpiatura.to_owned());
        }
        corpus
    }
}
