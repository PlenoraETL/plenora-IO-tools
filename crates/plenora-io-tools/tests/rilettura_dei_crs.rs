//! La matrice delle riletture: ogni sorgente verso ogni bersaglio, e il file
//! scritto si rilegge.
//!
//! # L'invariante
//!
//! Una scrittura che riesce dice, in `write_loss.counts`, che cosa del CRS non
//! ha conservato. Il file che produce deve dire **la stessa cosa**:
//!
//! 1. senza una perdita `crs_id_not_preserved_*`, il file si rilegge con lo
//!    **stesso** identificatore, byte per byte;
//! 2. con `crs_id_not_preserved_derived`, l'identificatore si rilegge dal file
//!    uguale per GEO-005 (autorita' senza maiuscole, codice come testo);
//! 3. senza una perdita `crs_definition_not_preserved_*`, la definizione si
//!    rilegge byte per byte (una definizione che la sorgente non aveva puo'
//!    comparire: e' informazione in piu', non una perdita);
//! 4. un file che non si rilegge senza opzioni ha dichiarato la perdita di cio'
//!    che serve a rileggerlo: dell'identificatore (`crs_id_not_preserved_absent`),
//!    che `--assume-crs` riporta e la prova lo verifica, o -- se la sorgente non
//!    ne aveva uno -- della definizione (`crs_definition_not_preserved_absent`);
//! 5. una sorgente non risolta (`missing`, `declared_unresolved`) si rilegge
//!    nello stesso stato, e una risolta si rilegge risolta;
//! 6. una definizione riletta uguale si rilegge nella **stessa** grammatica
//!    (`definition_format`): lo stesso testo etichettato altrimenti e' un'altra
//!    affermazione;
//! 7. l'ordine degli assi della sorgente, se dichiarato (non `unknown`), si
//!    rilegge uguale, o `unknown` quando la perdita della definizione che lo
//!    portava e' dichiarata -- mai un **altro** ordine. L'eccezione e' quella di
//!    docs/PRODUCT.md («`other` e `unknown` passano verso i formati con l'est per
//!    primo»), scritta qui per esteso ([`EST_PER_PRIMO`]);
//! 8. senza una perdita `srid_not_preserved_*`, l'SRID si rilegge uguale (la
//!    prova lo legge dove `io.inspect` non lo rende: nei campi di un Arrow IPC);
//! 9. un `GeoParquet` scritto porta in `ARROW:schema`, che e' cio' che pyarrow
//!    rende e dove geopandas cerca, il `geo` del footer, identico; un Arrow IPC
//!    scritto da un'altra sorgente non porta un `geo` di schema. Da IPC a IPC il
//!    `geo` passa tale e quale (ARROW-010), limite dichiarato.
//!
//! La sorgente e' cio' che `io.inspect` ne legge, non cio' che la prova crede di
//! averci scritto: cosi' la stessa regola vale ai passaggi successivi, dai file
//! che il primo ha scritto (`GeoParquet`, `GeoPackage`, Shapefile, DXF, `GeoJSON`)
//! verso ogni bersaglio, e dalle sorgenti native.
//!
//! # Perche' esiste
//!
//! La rilettura 11 ha trovato sette rilievi contro questa regola, ciascuno in una
//! conversione che la busta dichiarava `lossless` o con una perdita diversa da
//! quella vera: `EPSG:4326` che `GeoParquet` rilegge `OGC:CRS84`, un PROJJSON
//! riscritto con le chiavi riordinate e i numeri passati da `f64`, un PROJJSON
//! `EngineeringCRS` con `EPSG:4326` che il file non rilegge, un PROJJSON nella
//! colonna WKT di `gpkg_spatial_ref_sys`, un WKT anonimo che lo Shapefile
//! rilegge non risolto. Erano trovate da una matrice scritta a mano; questa e'
//! quella matrice, permanente. Su `db1307d` fallisce con trenta violazioni al
//! primo passaggio e cinquantacinque al secondo.
//!
//! La rilettura 12 ne ha trovati altri tre, fuori dalla matrice di allora, che
//! confrontava solo identificatore e definizione e partiva solo da IPC: un `geo`
//! stantio della sorgente copiato in `ARROW:schema`, il DXF che rifilava una
//! definizione chiusa da CRLF, e il `GeoPackage` che scrive `epsg:4326` come
//! `EPSG:4326` dichiarandolo conservato. Ora la matrice confronta anche
//! grammatica, assi, stato e SRID, parte anche da sorgenti native (`GeoPackage`
//! alla GDAL, Shapefile con il `.prj` chiuso da CRLF, `GeoParquet` alla
//! geopandas) e ha un terzo passaggio sulle catene di [`TERZO_PASSAGGIO`]. Su
//! `777c5a1` fallisce in tutti e tre i passaggi, e su `c5abbdb` (lo schema
//! incorporato senza `geo`) anche: un `GeoParquet` di IO, per pyarrow, non lo era.
//!
//! `FileGDB` resta fuori: scrive solo con `gdal-backend`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

const fn binario() -> &'static str {
    env!("CARGO_BIN_EXE_plenora-io")
}

// --- le definizioni ----------------------------------------------------------

const WGS84_EPSG: &str = r#"GEOGCS["WGS 84",DATUM["WGS_1984",SPHEROID["WGS 84",6378137,298.257223563,AUTHORITY["EPSG","7030"]],AUTHORITY["EPSG","6326"]],PRIMEM["Greenwich",0,AUTHORITY["EPSG","8901"]],UNIT["degree",0.0174532925199433,AUTHORITY["EPSG","9122"]],AUTHORITY["EPSG","4326"]]"#;
const WGS84_ESRI: &str = r#"GEOGCS["GCS_WGS_1984",DATUM["D_WGS_1984",SPHEROID["WGS_1984",6378137.0,298.257223563]],PRIMEM["Greenwich",0.0],UNIT["Degree",0.0174532925199433]]"#;
const UTM32_EPSG: &str = r#"PROJCS["WGS 84 / UTM zone 32N",GEOGCS["WGS 84",DATUM["WGS_1984",SPHEROID["WGS 84",6378137,298.257223563]],PRIMEM["Greenwich",0],UNIT["degree",0.0174532925199433]],PROJECTION["Transverse_Mercator"],PARAMETER["latitude_of_origin",0],PARAMETER["central_meridian",9],PARAMETER["scale_factor",0.9996],PARAMETER["false_easting",500000],PARAMETER["false_northing",0],UNIT["metre",1],AUTHORITY["EPSG","32632"]]"#;
const UTM32_ANONIMO: &str = r#"PROJCS["WGS 84 / UTM zone 32N",GEOGCS["WGS 84",DATUM["WGS_1984",SPHEROID["WGS 84",6378137,298.257223563]],PRIMEM["Greenwich",0],UNIT["degree",0.0174532925199433]],PROJECTION["Transverse_Mercator"],PARAMETER["latitude_of_origin",0],PARAMETER["central_meridian",9],PARAMETER["scale_factor",0.9996],PARAMETER["false_easting",500000],PARAMETER["false_northing",0],UNIT["metre",1]]"#;
const WGS84_WKT2: &str = r#"GEOGCRS["WGS 84",DATUM["World Geodetic System 1984",ELLIPSOID["WGS 84",6378137,298.257223563,LENGTHUNIT["metre",1]]],PRIMEM["Greenwich",0,ANGLEUNIT["degree",0.0174532925199433]],CS[ellipsoidal,2],AXIS["geodetic latitude (Lat)",north,ORDER[1],ANGLEUNIT["degree",0.0174532925199433]],AXIS["geodetic longitude (Lon)",east,ORDER[2],ANGLEUNIT["degree",0.0174532925199433]],ID["EPSG",4326]]"#;
const CRS84_WKT2: &str = r#"GEOGCRS["WGS 84 (CRS84)",DATUM["World Geodetic System 1984",ELLIPSOID["WGS 84",6378137,298.257223563,LENGTHUNIT["metre",1]]],PRIMEM["Greenwich",0,ANGLEUNIT["degree",0.0174532925199433]],CS[ellipsoidal,2],AXIS["geodetic longitude (Lon)",east,ORDER[1],ANGLEUNIT["degree",0.0174532925199433]],AXIS["geodetic latitude (Lat)",north,ORDER[2],ANGLEUNIT["degree",0.0174532925199433]],ID["OGC","CRS84"]]"#;
const LOCALE: &str =
    r#"LOCAL_CS["x",LOCAL_DATUM["d",0],UNIT["metre",1],AXIS["X",EAST],AXIS["Y",NORTH]]"#;
const PJ_4326: &str = r#"{"$schema":"https://proj.org/schemas/v0.5/projjson.schema.json","type":"GeographicCRS","name":"WGS 84","datum":{"type":"GeodeticReferenceFrame","name":"World Geodetic System 1984","ellipsoid":{"name":"WGS 84","semi_major_axis":6378137,"inverse_flattening":298.257223563}},"coordinate_system":{"subtype":"ellipsoidal","axis":[{"name":"Geodetic latitude","abbreviation":"Lat","direction":"north","unit":"degree"},{"name":"Geodetic longitude","abbreviation":"Lon","direction":"east","unit":"degree"}]},"id":{"authority":"EPSG","code":4326}}"#;
/// Il PROJJSON di EPSG:32632, con le chiavi **fuori** dall'ordine
/// lessicografico (`type` prima di `name`): una riscrittura le riordinerebbe.
const PJ_32632: &str = r#"{"$schema":"https://proj.org/schemas/v0.5/projjson.schema.json","type":"ProjectedCRS","name":"WGS 84 / UTM zone 32N","base_crs":{"name":"WGS 84","datum":{"type":"GeodeticReferenceFrame","name":"World Geodetic System 1984","ellipsoid":{"name":"WGS 84","semi_major_axis":6378137,"inverse_flattening":298.257223563}},"coordinate_system":{"subtype":"ellipsoidal","axis":[{"name":"Geodetic latitude","abbreviation":"Lat","direction":"north","unit":"degree"},{"name":"Geodetic longitude","abbreviation":"Lon","direction":"east","unit":"degree"}]}},"conversion":{"name":"UTM zone 32N","method":{"name":"Transverse Mercator","id":{"authority":"EPSG","code":9807}},"parameters":[{"name":"Latitude of natural origin","value":0,"unit":"degree"},{"name":"Longitude of natural origin","value":9,"unit":"degree"},{"name":"Scale factor at natural origin","value":0.9996,"unit":"unity"},{"name":"False easting","value":500000,"unit":"metre"},{"name":"False northing","value":0,"unit":"metre"}]},"coordinate_system":{"subtype":"Cartesian","axis":[{"name":"Easting","abbreviation":"E","direction":"east","unit":"metre"},{"name":"Northing","abbreviation":"N","direction":"north","unit":"metre"}]},"id":{"authority":"EPSG","code":32632}}"#;

/// Lo stesso PROJJSON senza `id`.
fn pj_senza_id() -> String {
    PJ_32632.replace(r#","id":{"authority":"EPSG","code":32632}}"#, "}")
}

/// Numeri che `f64` non rappresenta, un esponente, una barra in escape: tutti
/// testi che una riscrittura cambia.
fn pj_numeri() -> String {
    let barra = char::from(0x5c_u8);
    PJ_32632
        .replace(
            r#""value":0.9996"#,
            r#""value":0.99960000000000000000000001"#,
        )
        .replace(r#""value":500000"#, r#""value":500000.00000000000000001"#)
        .replace(r#""value":9,"#, r#""value":9E0,"#)
        .replace(
            r#""name":"WGS 84 / UTM"#,
            &format!(r#""name":"WGS 84 {barra}u002F UTM"#),
        )
}

/// Un `EngineeringCRS` con `id` EPSG:4326: genere contraddetto.
const PJ_ENG_4326: &str = r#"{"$schema":"https://proj.org/schemas/v0.7/projjson.schema.json","type":"EngineeringCRS","name":"x","datum":{"name":"d"},"coordinate_system":{"subtype":"Cartesian","axis":[{"name":"X","abbreviation":"X","direction":"east","unit":"metre"},{"name":"Y","abbreviation":"Y","direction":"north","unit":"metre"}]},"id":{"authority":"EPSG","code":4326}}"#;

/// I WKT come li scrive GDAL, con `AXIS`.
const GDAL_4326: &str = r#"GEOGCS["WGS 84",DATUM["WGS_1984",SPHEROID["WGS 84",6378137,298.257223563,AUTHORITY["EPSG","7030"]],AUTHORITY["EPSG","6326"]],PRIMEM["Greenwich",0,AUTHORITY["EPSG","8901"]],UNIT["degree",0.0174532925199433,AUTHORITY["EPSG","9122"]],AXIS["Latitude",NORTH],AXIS["Longitude",EAST],AUTHORITY["EPSG","4326"]]"#;
const GDAL_32632: &str = r#"PROJCS["WGS 84 / UTM zone 32N",GEOGCS["WGS 84",DATUM["WGS_1984",SPHEROID["WGS 84",6378137,298.257223563,AUTHORITY["EPSG","7030"]],AUTHORITY["EPSG","6326"]],PRIMEM["Greenwich",0,AUTHORITY["EPSG","8901"]],UNIT["degree",0.0174532925199433,AUTHORITY["EPSG","9122"]],AUTHORITY["EPSG","4326"]],PROJECTION["Transverse_Mercator"],PARAMETER["latitude_of_origin",0],PARAMETER["central_meridian",9],PARAMETER["scale_factor",0.9996],PARAMETER["false_easting",500000],PARAMETER["false_northing",0],UNIT["metre",1,AUTHORITY["EPSG","9001"]],AXIS["Easting",EAST],AXIS["Northing",NORTH],AUTHORITY["EPSG","32632"]]"#;
/// Il `geo` di schema che geopandas scrive anche in Arrow IPC (`to_feather`):
/// dichiara `EPSG:4326`, e qui contraddice il CRS del campo.
const GEO_STANTIO: &str = r#"{"version":"1.1.0","primary_column":"geometry","columns":{"geometry":{"encoding":"WKB","geometry_types":["Point"],"crs":{"type":"GeographicCRS","name":"stantio","id":{"authority":"EPSG","code":4326}}}}}"#;

// --- le sorgenti -------------------------------------------------------------

const GEO: (f64, f64) = (9.0, 45.0);
const UTM: (f64, f64) = (500_000.0, 5_000_000.0);

struct Caso {
    nome: &'static str,
    risoluzione: &'static str,
    crs_id: Option<&'static str>,
    definizione: Option<(String, &'static str)>,
    assi: Option<&'static str>,
    punto: (f64, f64),
    srid: Option<&'static str>,
    /// Un `geo` nei metadati di schema.
    geo_di_schema: bool,
}

impl Caso {
    const fn con_srid(mut self, srid: &'static str) -> Self {
        self.srid = Some(srid);
        self
    }

    const fn con_geo_di_schema(mut self) -> Self {
        self.geo_di_schema = true;
        self
    }
}

fn risolto(
    nome: &'static str,
    crs_id: Option<&'static str>,
    definizione: Option<(&str, &'static str)>,
    assi: &'static str,
    punto: (f64, f64),
) -> Caso {
    Caso {
        nome,
        risoluzione: "resolved",
        crs_id,
        definizione: definizione.map(|(testo, formato)| (testo.to_owned(), formato)),
        assi: Some(assi),
        punto,
        srid: None,
        geo_di_schema: false,
    }
}

// Una riga per caso: la lunghezza e' quella della matrice, non logica.
#[allow(clippy::too_many_lines)]
fn casi() -> Vec<Caso> {
    let pj_senza_id = pj_senza_id();
    let pj_numeri = pj_numeri();
    vec![
        risolto(
            "4326_wkt",
            Some("EPSG:4326"),
            Some((WGS84_EPSG, "wkt")),
            "lon_lat",
            GEO,
        ),
        risolto(
            "4326_esri",
            Some("EPSG:4326"),
            Some((WGS84_ESRI, "wkt")),
            "lon_lat",
            GEO,
        ),
        risolto(
            "32632_wkt",
            Some("EPSG:32632"),
            Some((UTM32_EPSG, "wkt")),
            "easting_northing",
            UTM,
        ),
        risolto(
            "anonimo_wkt",
            None,
            Some((UTM32_ANONIMO, "wkt")),
            "easting_northing",
            UTM,
        ),
        risolto(
            "4326_pj",
            Some("EPSG:4326"),
            Some((PJ_4326, "projjson")),
            "lon_lat",
            GEO,
        ),
        risolto(
            "32632_pj",
            Some("EPSG:32632"),
            Some((PJ_32632, "projjson")),
            "easting_northing",
            UTM,
        ),
        risolto(
            "32632_pj_numeri",
            Some("EPSG:32632"),
            Some((&pj_numeri, "projjson")),
            "easting_northing",
            UTM,
        ),
        risolto("4326_id", Some("EPSG:4326"), None, "lon_lat", GEO),
        risolto("32632_id", Some("EPSG:32632"), None, "unknown", UTM),
        risolto("crs84_id", Some("OGC:CRS84"), None, "lon_lat", GEO),
        risolto(
            "4326_wkt2",
            Some("EPSG:4326"),
            Some((WGS84_WKT2, "wkt2")),
            "lon_lat",
            GEO,
        ),
        risolto(
            "crs84_wkt2",
            Some("OGC:CRS84"),
            Some((CRS84_WKT2, "wkt2")),
            "lon_lat",
            GEO,
        ),
        risolto(
            "4326_locale",
            Some("EPSG:4326"),
            Some((LOCALE, "wkt")),
            "lon_lat",
            GEO,
        ),
        risolto(
            "anonimo_pj",
            None,
            Some((&pj_senza_id, "projjson")),
            "easting_northing",
            UTM,
        ),
        risolto(
            "crs84_esri",
            Some("OGC:CRS84"),
            Some((WGS84_ESRI, "wkt")),
            "lon_lat",
            GEO,
        ),
        risolto(
            "32632_wkt_anonimo",
            Some("EPSG:32632"),
            Some((UTM32_ANONIMO, "wkt")),
            "easting_northing",
            UTM,
        ),
        risolto(
            "crs84_locale",
            Some("OGC:CRS84"),
            Some((LOCALE, "wkt")),
            "lon_lat",
            GEO,
        ),
        risolto(
            "4326_pj_ingegneria",
            Some("EPSG:4326"),
            Some((PJ_ENG_4326, "projjson")),
            "lon_lat",
            GEO,
        ),
        risolto(
            "3857_geografico",
            Some("EPSG:3857"),
            Some((WGS84_ESRI, "wkt")),
            "lon_lat",
            GEO,
        ),
        Caso {
            nome: "mancante",
            risoluzione: "missing",
            crs_id: None,
            definizione: None,
            assi: None,
            punto: GEO,
            srid: None,
            geo_di_schema: false,
        },
        // Rilettura 12: identificatori minuscoli, definizioni con i bordi, un
        // ordine `other`, l'SRID, un `geo` di schema, una sorgente non risolta.
        risolto("4326_id_minuscolo", Some("epsg:4326"), None, "lon_lat", GEO),
        risolto(
            "32632_wkt_minuscolo",
            Some("epsg:32632"),
            Some((GDAL_32632, "wkt")),
            "easting_northing",
            UTM,
        ),
        risolto(
            "32632_wkt_crlf",
            Some("EPSG:32632"),
            Some((&format!("{GDAL_32632}\r\n"), "wkt")),
            "easting_northing",
            UTM,
        ),
        risolto(
            "32632_wkt_spazi",
            Some("EPSG:32632"),
            Some((&format!("  {UTM32_EPSG} "), "wkt")),
            "easting_northing",
            UTM,
        ),
        risolto(
            "4326_gdal_lat_lon",
            Some("EPSG:4326"),
            Some((GDAL_4326, "wkt")),
            "lat_lon",
            (45.0, 9.0),
        ),
        risolto(
            "32632_other",
            Some("EPSG:32632"),
            Some((GDAL_32632, "wkt")),
            "other",
            UTM,
        ),
        risolto(
            "32632_wkt_srid",
            Some("EPSG:32632"),
            Some((GDAL_32632, "wkt")),
            "easting_northing",
            UTM,
        )
        .con_srid("32632"),
        risolto(
            "32632_pj_geo_stantio",
            Some("EPSG:32632"),
            Some((PJ_32632, "projjson")),
            "easting_northing",
            UTM,
        )
        .con_geo_di_schema(),
        Caso {
            nome: "non_risolto_wkt",
            risoluzione: "declared_unresolved",
            crs_id: None,
            definizione: Some((UTM32_ANONIMO.to_owned(), "wkt")),
            assi: Some("unknown"),
            punto: UTM,
            srid: None,
            geo_di_schema: false,
        },
    ]
}

fn punto_wkb((x, y): (f64, f64)) -> Vec<u8> {
    let mut byte = vec![1_u8];
    byte.extend_from_slice(&1_u32.to_le_bytes());
    byte.extend_from_slice(&x.to_le_bytes());
    byte.extend_from_slice(&y.to_le_bytes());
    byte
}

fn sorgente_ipc(dir: &Path, caso: &Caso) -> PathBuf {
    use arrow_array::{ArrayRef, BinaryArray, RecordBatch, StringArray};
    use arrow_schema::{DataType, Field, Schema};

    let mut geometria: HashMap<String, String> = [
        ("ARROW:extension:name", "geoarrow.wkb"),
        ("plenora.field_id", "1"),
        ("plenora.geometry.encoding", "wkb"),
        ("plenora.geometry.dimensions", "xy"),
        ("plenora.geometry.spatial_semantics", "geometry"),
        ("plenora.geometry.precision", "float64"),
        ("plenora.geometry.types_declaration", "exact"),
        ("plenora.geometry.types", "point"),
        ("plenora.geometry.crs_resolution", caso.risoluzione),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_owned(), v.to_owned()))
    .collect();
    if let Some(assi) = caso.assi {
        geometria.insert("plenora.geometry.axis_order".to_owned(), assi.to_owned());
    }
    if let Some(id) = caso.crs_id {
        geometria.insert("plenora.geometry.crs_id".to_owned(), id.to_owned());
        geometria.insert("crs".to_owned(), id.to_owned());
    }
    if let Some((testo, formato)) = &caso.definizione {
        geometria.insert("plenora.geometry.crs_definition".to_owned(), testo.clone());
        geometria.insert(
            "plenora.geometry.crs_definition_format".to_owned(),
            (*formato).to_owned(),
        );
    }
    if let Some(srid) = caso.srid {
        geometria.insert("plenora.geometry.srid".to_owned(), srid.to_owned());
    }
    let mut metadati = HashMap::from([("plenora.contract.version".to_owned(), "1".to_owned())]);
    if caso.geo_di_schema {
        metadati.insert("geo".to_owned(), GEO_STANTIO.to_owned());
    }
    let schema = Arc::new(Schema::new_with_metadata(
        vec![
            Field::new("nome", DataType::Utf8, false),
            Field::new("geometry", DataType::Binary, true).with_metadata(geometria),
        ],
        metadati,
    ));
    let wkb = [Some(punto_wkb(caso.punto))];
    let colonne: Vec<ArrayRef> = vec![
        Arc::new(StringArray::from(vec!["alfa"])),
        Arc::new(wkb.iter().map(Option::as_deref).collect::<BinaryArray>()),
    ];
    let batch = RecordBatch::try_new(Arc::clone(&schema), colonne).expect("batch");
    let percorso = dir.join(format!("{}.arrow", caso.nome));
    let file = std::fs::File::create(&percorso).expect("sorgente creata");
    let mut scrittore = arrow_ipc::writer::FileWriter::try_new(file, &schema).expect("writer");
    scrittore.write(&batch).expect("batch scritto");
    scrittore.finish().expect("sorgente completata");
    percorso
}

// --- le sorgenti native ------------------------------------------------------
//
// Scritte come le scrivono gli altri: nessuna passa dai writer di IO, che la
// matrice sta provando.

/// Un `GeoPackage` come lo scrive GDAL: le righe obbligatorie, una riga
/// `(organizzazione, codice, definizione)` per il CRS del layer e un punto.
fn gpkg_nativo(
    dir: &Path,
    nome: &str,
    riga: Option<(&str, i64, &str)>,
    srs_id: i64,
    punto: (f64, f64),
) -> PathBuf {
    let percorso = dir.join(format!("{nome}.gpkg"));
    let conn = rusqlite::Connection::open(&percorso).expect("sqlite");
    conn.execute_batch(
        "PRAGMA application_id = 1196444487; PRAGMA user_version = 10300;
         CREATE TABLE gpkg_spatial_ref_sys (srs_name TEXT NOT NULL, srs_id INTEGER PRIMARY KEY,
            organization TEXT NOT NULL, organization_coordsys_id INTEGER NOT NULL,
            definition TEXT NOT NULL, description TEXT);
         CREATE TABLE gpkg_contents (table_name TEXT NOT NULL PRIMARY KEY, data_type TEXT NOT NULL,
            identifier TEXT UNIQUE, description TEXT DEFAULT '', last_change DATETIME NOT NULL
            DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')), min_x DOUBLE, min_y DOUBLE,
            max_x DOUBLE, max_y DOUBLE, srs_id INTEGER);
         CREATE TABLE gpkg_geometry_columns (table_name TEXT NOT NULL, column_name TEXT NOT NULL,
            geometry_type_name TEXT NOT NULL, srs_id INTEGER NOT NULL, z TINYINT NOT NULL,
            m TINYINT NOT NULL, PRIMARY KEY (table_name, column_name));
         CREATE TABLE punti (fid INTEGER PRIMARY KEY AUTOINCREMENT, geom POINT, nome TEXT);
         INSERT INTO gpkg_spatial_ref_sys VALUES
            ('Undefined cartesian SRS', -1, 'NONE', -1, 'undefined', NULL),
            ('Undefined geographic SRS', 0, 'NONE', 0, 'undefined', NULL);",
    )
    .expect("tabelle");
    let mut righe = vec![("EPSG", 4326, GDAL_4326)];
    righe.extend(riga);
    for (organizzazione, codice, definizione) in righe {
        conn.execute(
            "INSERT OR REPLACE INTO gpkg_spatial_ref_sys VALUES ('crs', ?1, ?2, ?3, ?4, NULL)",
            rusqlite::params![
                if codice == 4326 { 4326 } else { srs_id },
                organizzazione,
                codice,
                definizione
            ],
        )
        .expect("riga del CRS");
    }
    conn.execute(
        "INSERT INTO gpkg_contents (table_name, data_type, identifier, srs_id) \
         VALUES ('punti', 'features', 'punti', ?1)",
        [srs_id],
    )
    .expect("contents");
    conn.execute(
        "INSERT INTO gpkg_geometry_columns VALUES ('punti', 'geom', 'POINT', ?1, 0, 0)",
        [srs_id],
    )
    .expect("geometry_columns");
    let mut blob = b"GP".to_vec();
    blob.extend_from_slice(&[0, 1]);
    blob.extend_from_slice(&i32::try_from(srs_id).expect("srs_id").to_le_bytes());
    blob.extend_from_slice(&punto_wkb(punto));
    conn.execute("INSERT INTO punti (geom, nome) VALUES (?1, 'alfa')", [blob])
        .expect("punto");
    percorso
}

/// Uno Shapefile scritto byte per byte, con il `.prj` che si vuole.
fn shp_nativo(dir: &Path, nome: &str, prj: &str, (x, y): (f64, f64)) -> PathBuf {
    let cartella = dir.join(nome);
    std::fs::create_dir_all(&cartella).expect("cartella");
    let intestazione = |parole: i32| {
        let mut byte = Vec::new();
        for valore in [9994_i32, 0, 0, 0, 0, 0, parole] {
            byte.extend_from_slice(&valore.to_be_bytes());
        }
        byte.extend_from_slice(&1000_i32.to_le_bytes());
        byte.extend_from_slice(&1_i32.to_le_bytes());
        for valore in [x, y, x, y, 0.0, 0.0, 0.0, 0.0] {
            byte.extend_from_slice(&valore.to_le_bytes());
        }
        byte
    };
    let mut record = 1_i32.to_le_bytes().to_vec();
    record.extend_from_slice(&x.to_le_bytes());
    record.extend_from_slice(&y.to_le_bytes());
    let mut shp = intestazione(64);
    shp.extend_from_slice(&1_i32.to_be_bytes());
    shp.extend_from_slice(&10_i32.to_be_bytes());
    shp.extend_from_slice(&record);
    let mut shx = intestazione(54);
    shx.extend_from_slice(&50_i32.to_be_bytes());
    shx.extend_from_slice(&10_i32.to_be_bytes());
    let mut dbf = vec![3_u8, 124, 10, 24];
    dbf.extend_from_slice(&1_u32.to_le_bytes());
    dbf.extend_from_slice(&65_u16.to_le_bytes());
    dbf.extend_from_slice(&11_u16.to_le_bytes());
    dbf.extend_from_slice(&[0; 20]);
    dbf.extend_from_slice(b"NOME\0\0\0\0\0\0\0C\0\0\0\0\x0a");
    dbf.extend_from_slice(&[0; 15]);
    dbf.extend_from_slice(b"\r alfa      \x1a");
    let base = cartella.join("punti");
    for (estensione, byte) in [
        ("shp", shp),
        ("shx", shx),
        ("dbf", dbf),
        ("prj", prj.as_bytes().to_vec()),
        ("cpg", b"UTF-8".to_vec()),
    ] {
        std::fs::write(base.with_extension(estensione), byte).expect("componente");
    }
    base.with_extension("shp")
}

/// Un `GeoParquet` come lo scrive pyarrow per geopandas: `geo` nel footer e
/// nei metadati dello schema incorporato.
fn geoparquet_nativo(dir: &Path, nome: &str, crs: Option<&str>, punto: (f64, f64)) -> PathBuf {
    use arrow_array::{ArrayRef, BinaryArray, RecordBatch, StringArray};
    use arrow_schema::{DataType, Field, Schema};

    let campo_crs = crs.map_or_else(String::new, |crs| format!(r#", "crs": {crs}"#));
    let geo = format!(
        r#"{{"primary_column": "geometry", "columns": {{"geometry": {{"encoding": "WKB", "geometry_types": ["Point"]{campo_crs}}}}}, "version": "1.0.0"}}"#
    );
    let schema = Arc::new(Schema::new_with_metadata(
        vec![
            Field::new("nome", DataType::Utf8, true),
            Field::new("geometry", DataType::Binary, true),
        ],
        HashMap::from([("geo".to_owned(), geo.clone())]),
    ));
    let colonne: Vec<ArrayRef> = vec![
        Arc::new(StringArray::from(vec!["alfa"])),
        Arc::new(BinaryArray::from(vec![Some(punto_wkb(punto).as_slice())])),
    ];
    let batch = RecordBatch::try_new(Arc::clone(&schema), colonne).expect("batch");
    let proprieta = parquet::file::properties::WriterProperties::builder()
        .set_key_value_metadata(Some(vec![parquet::file::metadata::KeyValue::new(
            "geo".to_owned(),
            geo,
        )]))
        .build();
    let percorso = dir.join(format!("{nome}.parquet"));
    let file = std::fs::File::create(&percorso).expect("file");
    let mut scrittore =
        parquet::arrow::ArrowWriter::try_new(file, schema, Some(proprieta)).expect("writer");
    scrittore.write(&batch).expect("scrittura");
    scrittore.close().expect("chiusura");
    percorso
}

/// Il PROJJSON come lo serializza geopandas: `", "` e `": "`.
fn alla_geopandas(projjson: &str) -> String {
    projjson.replace(",\"", ", \"").replace("\":", "\": ")
}

/// Le sorgenti native: nome, percorso, formato.
// Una riga per sorgente: la lunghezza e' quella dell'elenco, non logica.
#[allow(clippy::too_many_lines)]
fn sorgenti_native(dir: &Path) -> Vec<(String, PathBuf, &'static str)> {
    let pj_minuscolo = PJ_4326.replace(
        r#""authority":"EPSG","code":4326"#,
        r#""authority":"epsg","code":4326"#,
    );
    let crlf = format!("{GDAL_32632}\r\n");
    let spazi = format!("  {GDAL_32632}");
    let sorgenti = vec![
        (
            "g_4326_gdal",
            gpkg_nativo(dir, "g_4326_gdal", None, 4326, GEO),
            "gpkg",
        ),
        (
            "g_32632_gdal",
            gpkg_nativo(
                dir,
                "g_32632_gdal",
                Some(("EPSG", 32632, GDAL_32632)),
                32632,
                UTM,
            ),
            "gpkg",
        ),
        (
            "g_32632_minuscolo",
            gpkg_nativo(
                dir,
                "g_32632_minuscolo",
                Some(("epsg", 32632, GDAL_32632)),
                32632,
                UTM,
            ),
            "gpkg",
        ),
        (
            "g_32632_crlf",
            gpkg_nativo(
                dir,
                "g_32632_crlf",
                Some(("EPSG", 32632, crlf.as_str())),
                32632,
                UTM,
            ),
            "gpkg",
        ),
        (
            "g_32632_undefined",
            gpkg_nativo(
                dir,
                "g_32632_undefined",
                Some(("EPSG", 32632, "undefined")),
                32632,
                UTM,
            ),
            "gpkg",
        ),
        (
            "s_esri_gcs",
            shp_nativo(dir, "s_esri_gcs", WGS84_ESRI, GEO),
            "shp",
        ),
        (
            "s_32632_crlf",
            shp_nativo(dir, "s_32632_crlf", &crlf, UTM),
            "shp",
        ),
        (
            "s_32632_spazi",
            shp_nativo(dir, "s_32632_spazi", &spazi, UTM),
            "shp",
        ),
        (
            "s_4326_wkt2",
            shp_nativo(dir, "s_4326_wkt2", WGS84_WKT2, GEO),
            "shp",
        ),
        (
            "p_4326_geopandas",
            geoparquet_nativo(dir, "p_4326_geopandas", Some(&alla_geopandas(PJ_4326)), GEO),
            "geoparquet",
        ),
        (
            "p_4326_minuscolo",
            geoparquet_nativo(dir, "p_4326_minuscolo", Some(&pj_minuscolo), GEO),
            "geoparquet",
        ),
        (
            "p_32632",
            geoparquet_nativo(dir, "p_32632", Some(PJ_32632), UTM),
            "geoparquet",
        ),
        (
            "p_crs84",
            geoparquet_nativo(dir, "p_crs84", None, GEO),
            "geoparquet",
        ),
        (
            "p_nullo",
            geoparquet_nativo(dir, "p_nullo", Some("null"), GEO),
            "geoparquet",
        ),
    ];
    sorgenti
        .into_iter()
        .map(|(nome, percorso, formato)| (nome.to_owned(), percorso, formato))
        .collect()
}

/// Tutte le sorgenti del primo passaggio: i casi IPC e le native.
fn sorgenti(dir: &Path) -> Vec<(String, PathBuf, &'static str)> {
    let mut tutte: Vec<(String, PathBuf, &'static str)> = casi()
        .iter()
        .map(|caso| (caso.nome.to_owned(), sorgente_ipc(dir, caso), "ipc"))
        .collect();
    tutte.extend(sorgenti_native(dir));
    tutte
}

// --- il binario --------------------------------------------------------------

/// I bersagli: il formato e il nome del file.
const BERSAGLI: [(&str, &str); 9] = [
    ("ipc", "x.arrow"),
    ("geoparquet", "x.parquet"),
    ("gpkg", "x.gpkg"),
    ("shp", "x.shp.d"),
    ("dxf", "x.dxf"),
    ("geojson", "x.geojson"),
    ("kml", "x.kml"),
    ("csv", "x.csv"),
    ("xls", "x.xlsx"),
];

/// I formati da cui parte il secondo passaggio.
const RILETTI: [&str; 5] = ["geoparquet", "gpkg", "shp", "dxf", "geojson"];

/// Le catene del terzo passaggio: il formato del primo file e quello del
/// secondo. Sono le catene dei rilievi delle riletture 11 e 12 (il `GeoPackage`
/// che riscrive la definizione, lo Shapefile e il DXF che la portano nel solo
/// WKT, `GeoParquet` che la porta nel solo PROJJSON); tutte e venticinque
/// moltiplicherebbero per cinque il tempo della prova.
const TERZO_PASSAGGIO: [(&str, &str); 6] = [
    ("gpkg", "shp"),
    ("geoparquet", "gpkg"),
    ("shp", "gpkg"),
    ("gpkg", "dxf"),
    ("dxf", "geoparquet"),
    ("shp", "dxf"),
];

/// I bersagli il cui ordine degli assi e' fissato dal formato, l'est per primo:
/// un `other` ci si scrive com'e' e si rilegge con l'ordine del formato
/// (docs/PRODUCT.md, «`other` e `unknown` passano verso i formati con l'est per
/// primo»). E' l'eccezione dichiarata alla regola 7, e vale solo per `other`.
const EST_PER_PRIMO: [&str; 6] = ["geoparquet", "gpkg", "shp", "dxf", "geojson", "kml"];

fn cli(argomenti: &[&str]) -> (bool, serde_json::Value) {
    let output = Command::new(binario())
        .args(argomenti)
        .output()
        .expect("il binario si esegue");
    let busta = serde_json::from_str(String::from_utf8_lossy(&output.stdout).trim())
        .expect("un documento JSON su stdout");
    (output.status.success(), busta)
}

/// Il CRS che `io.inspect` legge, e l'SRID dove un file lo porta in chiaro.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Crs {
    stato: String,
    id: Option<String>,
    definizione: Option<String>,
    formato: Option<String>,
    assi: Option<String>,
    srid: Option<String>,
}

/// L'SRID di un Arrow IPC, dal campo geometria: `io.inspect` non lo rende.
fn srid_ipc(percorso: &Path) -> Option<String> {
    let file = std::fs::File::open(percorso).ok()?;
    let lettore = arrow_ipc::reader::FileReader::try_new(file, None).ok()?;
    let schema = lettore.schema();
    let campo = schema.field_with_name("geometry").ok()?;
    campo.metadata().get("plenora.geometry.srid").cloned()
}

fn crs_letto(percorso: &Path, opzioni: &[&str]) -> Result<Crs, String> {
    let mut argomenti = vec!["inspect", percorso.to_str().expect("percorso")];
    argomenti.extend_from_slice(opzioni);
    let (riuscita, busta) = cli(&argomenti);
    if !riuscita {
        return Err(format!("{}", busta["error"]["category"]));
    }
    let geometria = &busta["result"]["layers"][0]["geometry"];
    let risoluzione = &geometria["crs_resolution"];
    let testo = |valore: &serde_json::Value| valore.as_str().map(str::to_owned);
    Ok(Crs {
        stato: testo(&risoluzione["status"]).expect("la risoluzione porta lo stato"),
        id: testo(&risoluzione["id"]),
        definizione: testo(&risoluzione["definition"]),
        formato: testo(&risoluzione["definition_format"]),
        assi: testo(&risoluzione["axis_order"]),
        srid: srid_ipc(percorso),
    })
}

/// Le perdite CRS dichiarate, e la busta per la diagnostica.
fn converti(
    sorgente: &Path,
    da: &str,
    uscita: &Path,
    verso: &str,
) -> Option<(Vec<String>, String)> {
    let (riuscita, busta) = cli(&[
        "convert",
        sorgente.to_str().expect("percorso"),
        uscita.to_str().expect("percorso"),
        "--from",
        da,
        "--to",
        verso,
    ]);
    if !riuscita {
        return None;
    }
    let perdite = busta["result"]["write_loss"]["counts"]
        .as_array()
        .expect("una scrittura riuscita porta `counts`")
        .iter()
        .filter_map(|voce| voce["categoria"].as_str())
        .filter(|c| c.starts_with("crs_") || c.starts_with("srid_"))
        .map(str::to_owned)
        .collect();
    Some((
        perdite,
        busta["result"]["conversion_fidelity"]["level"].to_string(),
    ))
}

fn stesso_identificatore(a: &str, b: &str) -> bool {
    match (a.split_once(':'), b.split_once(':')) {
        (Some((aa, ca)), Some((ab, cb))) => aa.eq_ignore_ascii_case(ab) && ca == cb,
        _ => false,
    }
}

/// Un `geo` che il file scritto porta dove non deve (regola 9).
fn geo_fuori_posto(uscita: &Path, da: &str, verso: &str) -> Option<&'static str> {
    match verso {
        "geoparquet" => {
            use parquet::file::reader::FileReader as _;
            let file = std::fs::File::open(uscita).expect("file scritto");
            let lettore = parquet::file::reader::SerializedFileReader::new(file).expect("footer");
            let metadati = lettore.metadata().file_metadata();
            let chiavi: Vec<_> = metadati
                .key_value_metadata()
                .into_iter()
                .flatten()
                .collect();
            let solo_schema: Vec<_> = chiavi
                .iter()
                .filter(|voce| voce.key == "ARROW:schema")
                .map(|voce| (*voce).clone())
                .collect();
            if solo_schema.is_empty() {
                return None;
            }
            let incorporato = parquet::arrow::parquet_to_arrow_schema(
                metadati.schema_descr(),
                Some(&solo_schema),
            )
            .expect("schema incorporato");
            let del_footer = chiavi
                .iter()
                .find(|voce| voce.key == "geo")
                .and_then(|voce| voce.value.as_ref());
            (incorporato.metadata().get("geo") != del_footer)
                .then_some("il `geo` di ARROW:schema non e' quello del footer")
        }
        "ipc" if da != "ipc" => {
            let file = std::fs::File::open(uscita).expect("file scritto");
            let lettore = arrow_ipc::reader::FileReader::try_new(file, None).expect("IPC");
            lettore
                .schema()
                .metadata()
                .contains_key("geo")
                .then_some("un `geo` di schema da una sorgente non IPC")
        }
        _ => None,
    }
}

/// I bersagli che non portano un CRS: il lettore lo chiede sempre con
/// `--assume-crs` (e la geometria con `wkt_column`). Per loro la regola e'
/// che la perdita di ogni rappresentazione sia dichiarata.
const SENZA_CRS: [&str; 2] = ["csv", "xls"];

/// Le violazioni per un file che non si rilegge senza opzioni (regola 4).
fn violazioni_senza_rilettura(
    sorgente: &Crs,
    dichiara: &dyn Fn(&str) -> bool,
    uscita: &Path,
    categoria: &str,
) -> Vec<String> {
    let mut errori = Vec::new();
    match &sorgente.id {
        Some(id) => {
            if !dichiara("crs_id_not_preserved_absent") {
                errori.push(format!(
                    "non si rilegge ({categoria}) e non dichiara crs_id_not_preserved_absent"
                ));
            }
            match crs_letto(uscita, &["--assume-crs", id]) {
                Ok(riletto)
                    if riletto
                        .id
                        .as_deref()
                        .is_some_and(|r| stesso_identificatore(r, id)) => {}
                altro => errori.push(format!(
                    "--assume-crs non riporta l'identificatore: {altro:?}"
                )),
            }
        }
        None => {
            if !dichiara("crs_definition_not_preserved_absent") {
                errori.push(format!(
                    "non si rilegge ({categoria}), e senza identificatore non dichiara perduta la definizione"
                ));
            }
        }
    }
    errori
}

/// Le regole 1 e 2: l'identificatore.
fn violazioni_dell_identificatore(
    sorgente: &Crs,
    riletto: &Crs,
    dichiara: &dyn Fn(&str) -> bool,
) -> Vec<String> {
    let mut errori = Vec::new();
    let Some(id) = &sorgente.id else {
        return errori;
    };
    if !dichiara("crs_id_not_preserved") && riletto.id.as_ref() != Some(id) {
        errori.push(format!(
            "identificatore conservato e riletto diverso: {:?}",
            riletto.id
        ));
    }
    if dichiara("crs_id_not_preserved_derived")
        && !riletto
            .id
            .as_deref()
            .is_some_and(|r| stesso_identificatore(r, id))
    {
        errori.push(format!(
            "identificatore derivato e riletto diverso: {:?}",
            riletto.id
        ));
    }
    errori
}

/// Le regole 3, 6, 7 e 8: definizione, grammatica, assi, SRID.
fn violazioni_della_definizione(
    sorgente: &Crs,
    riletto: &Crs,
    dichiara: &dyn Fn(&str) -> bool,
    verso: &str,
) -> Vec<String> {
    let mut errori = Vec::new();
    if let Some(definizione) = &sorgente.definizione {
        if !dichiara("crs_definition_not_preserved")
            && riletto.definizione.as_ref() != Some(definizione)
        {
            errori.push("definizione conservata e riletta diversa".to_owned());
        }
        if riletto.definizione.as_ref() == Some(definizione) && riletto.formato != sorgente.formato
        {
            errori.push(format!(
                "la stessa definizione si rilegge in un'altra grammatica: {:?} -> {:?}",
                sorgente.formato, riletto.formato
            ));
        }
    }
    if let Some(assi) = sorgente.assi.as_deref().filter(|assi| *assi != "unknown") {
        let letti = riletto.assi.as_deref();
        let uguale = letti == Some(assi);
        let ignoto = letti == Some("unknown") && dichiara("crs_definition_not_preserved");
        let del_formato = assi == "other"
            && EST_PER_PRIMO.contains(&verso)
            && matches!(letti, Some("easting_northing" | "longitude_latitude"));
        if !(uguale || ignoto || del_formato) {
            errori.push(format!("l'ordine degli assi {assi} si rilegge {letti:?}"));
        }
    }
    if let Some(srid) = &sorgente.srid {
        let dichiarato = dichiara("srid_not_preserved");
        if !dichiarato && verso != "ipc" {
            errori.push("SRID non scritto e non dichiarato perduto".to_owned());
        }
        if !dichiarato && verso == "ipc" && riletto.srid.as_ref() != Some(srid) {
            errori.push(format!("SRID conservato e riletto {:?}", riletto.srid));
        }
    }
    errori
}

/// Le violazioni dell'invariante per una conversione riuscita.
fn violazioni(
    sorgente: &Crs,
    perdite: &[String],
    uscita: &Path,
    da: &str,
    verso: &str,
) -> Vec<String> {
    let dichiara = |prefisso: &str| perdite.iter().any(|p| p.starts_with(prefisso));
    let mut errori = Vec::new();
    if SENZA_CRS.contains(&verso) {
        if sorgente.id.is_some() && !dichiara("crs_id_not_preserved_absent") {
            errori.push("identificatore non scritto e non dichiarato perduto".to_owned());
        }
        if sorgente.definizione.is_some() && !dichiara("crs_definition_not_preserved_absent") {
            errori.push("definizione non scritta e non dichiarata perduta".to_owned());
        }
        return errori;
    }
    if let Some(errore) = geo_fuori_posto(uscita, da, verso) {
        errori.push(errore.to_owned());
    }
    let riletto = match crs_letto(uscita, &[]) {
        Ok(riletto) => riletto,
        Err(categoria) => {
            errori.extend(violazioni_senza_rilettura(
                sorgente, &dichiara, uscita, &categoria,
            ));
            return errori;
        }
    };
    if riletto.stato != sorgente.stato {
        errori.push(format!(
            "un CRS {} si rilegge {}",
            sorgente.stato, riletto.stato
        ));
    }
    errori.extend(violazioni_dell_identificatore(
        sorgente, &riletto, &dichiara,
    ));
    errori.extend(violazioni_della_definizione(
        sorgente, &riletto, &dichiara, verso,
    ));
    errori
}

/// Ogni bersaglio da una sorgente; restituisce i file scritti, per formato.
fn verso_ogni_bersaglio(
    dir: &Path,
    nome: &str,
    sorgente: &Path,
    da: &str,
    errori: &mut Vec<String>,
) -> Vec<(&'static str, PathBuf)> {
    let Ok(crs_sorgente) = crs_letto(sorgente, &[]) else {
        return Vec::new();
    };
    let mut scritti = Vec::new();
    for (verso, file) in BERSAGLI {
        let cartella = dir.join(format!("{nome}-{verso}"));
        std::fs::create_dir_all(&cartella).expect("cartella");
        let uscita = cartella.join(file);
        let Some((perdite, livello)) = converti(sorgente, da, &uscita, verso) else {
            continue;
        };
        for errore in violazioni(&crs_sorgente, &perdite, &uscita, da, verso) {
            errori.push(format!(
                "{nome} ({da}) -> {verso}: {errore}; livello {livello}, perdite {perdite:?}"
            ));
        }
        scritti.push((verso, uscita));
    }
    scritti
}

/// Il secondo passaggio da ogni sorgente: i file scritti, con il nome della
/// catena e i due formati.
fn secondo_passaggio(
    dir: &Path,
    errori: &mut Vec<String>,
) -> Vec<(String, &'static str, &'static str, PathBuf)> {
    let mut ignorati = Vec::new();
    let mut scritti = Vec::new();
    for (nome, sorgente, da) in sorgenti(dir) {
        let primo = dir.join("primo");
        for (formato, scritto) in verso_ogni_bersaglio(&primo, &nome, &sorgente, da, &mut ignorati)
        {
            if !RILETTI.contains(&formato) {
                continue;
            }
            let catena = format!("{nome}-{formato}");
            for (secondo, file) in
                verso_ogni_bersaglio(&dir.join("secondo"), &catena, &scritto, formato, errori)
            {
                scritti.push((catena.clone(), formato, secondo, file));
            }
        }
    }
    scritti
}

#[test]
fn ogni_scrittura_si_rilegge_come_la_dichiara() {
    let dir = tempfile::tempdir().expect("directory temporanea");
    let mut errori = Vec::new();
    for (nome, sorgente, da) in sorgenti(dir.path()) {
        // Una sorgente che non si legge uscirebbe dalla matrice in silenzio.
        if let Err(categoria) = crs_letto(&sorgente, &[]) {
            errori.push(format!("{nome}: la sorgente non si legge ({categoria})"));
        }
        verso_ogni_bersaglio(dir.path(), &nome, &sorgente, da, &mut errori);
    }
    assert!(errori.is_empty(), "{}", errori.join("\n"));
}

/// Il secondo passaggio: dai file che il primo ha scritto.
#[test]
fn ogni_scrittura_si_rilegge_come_la_dichiara_anche_al_secondo_passaggio() {
    let dir = tempfile::tempdir().expect("directory temporanea");
    let mut errori = Vec::new();
    secondo_passaggio(dir.path(), &mut errori);
    assert!(errori.is_empty(), "{}", errori.join("\n"));
}

/// Il terzo passaggio, sulle catene di [`TERZO_PASSAGGIO`].
#[test]
fn ogni_scrittura_si_rilegge_come_la_dichiara_anche_al_terzo_passaggio() {
    let dir = tempfile::tempdir().expect("directory temporanea");
    let mut ignorati = Vec::new();
    let mut errori = Vec::new();
    for (catena, primo, secondo, scritto) in secondo_passaggio(dir.path(), &mut ignorati) {
        if !TERZO_PASSAGGIO.contains(&(primo, secondo)) {
            continue;
        }
        verso_ogni_bersaglio(
            &dir.path().join("terzo"),
            &format!("{catena}-{secondo}"),
            &scritto,
            secondo,
            &mut errori,
        );
    }
    assert!(errori.is_empty(), "{}", errori.join("\n"));
}
