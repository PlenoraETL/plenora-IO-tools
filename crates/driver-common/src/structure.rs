//! Passo strutturale unico dei dieci ingressi, indipendente dal CRS e dal mapping.
use std::fs::File;
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use plenora_io_core::{FormatDescriptor, ReadOptions, Source};
use plenora_io_model::error_table::Situation;
use plenora_io_model::{PlenoraIoError, PublicMessage, Result};

fn label(id: &str) -> &'static str {
    match id {
        "csv" => "CSV",
        "shp" => "Shapefile",
        "gpkg" => "GeoPackage",
        "filegdb" => "FileGDB",
        "geojson" => "GeoJSON",
        "kml" => "KML",
        "dxf" => "DXF",
        "geoparquet" => "Parquet",
        "ipc" => "Arrow IPC",
        "xls" => "XLSX",
        _ => "formato dichiarato",
    }
}
fn invalid(id: &str) -> PlenoraIoError {
    Situation::InvalidContent.error_with(&PublicMessage::CuratedPair(
        "contenuto vuoto o struttura non valida; formato atteso:",
        label(id),
    ))
}
fn require(valid: bool, id: &str) -> Result<()> {
    if valid {
        Ok(())
    } else {
        Err(invalid(id))
    }
}
/// Verifica il contenitore prima delle opzioni di contenuto e del CRS.
/// # Errors
/// Errori della tabella, delle quote o del filesystem.
pub fn preflight(
    descriptor: &FormatDescriptor,
    source: Source,
    opts: &mut ReadOptions,
) -> Result<PathBuf> {
    preflight_with(descriptor, source, opts, |_, _| Ok(()))
}
/// Estende il passo strutturale con il catalogo nativo del driver.
/// # Errors
/// Errori strutturali, del filesystem, delle quote o della configurazione.
pub fn preflight_with(
    descriptor: &FormatDescriptor,
    source: Source,
    opts: &mut ReadOptions,
    catalog: impl Fn(&Path, &ReadOptions) -> Result<()>,
) -> Result<PathBuf> {
    let id = descriptor.id();
    plenora_io_core::driver::preflight_source_validated(descriptor, source, opts, &|path, opts| {
        validate(id, path, opts)?;
        catalog(path, opts)
    })
    .map_err(|error| {
        if error.category == plenora_io_model::ErrorCategory::InvalidConfiguration
            && error.message == Situation::DirectoryInsteadOfFile.rule().message
        {
            Situation::DirectoryInsteadOfFile.error_with(&PublicMessage::CuratedPair(
                "directory al posto del file; formato atteso:",
                label(id),
            ))
        } else {
            error
        }
    })
}

fn header<const N: usize>(path: &Path, id: &str) -> Result<([u8; N], u64)> {
    let metadata = std::fs::symlink_metadata(path)?;
    require(metadata.is_file(), id)?;
    let mut file = File::open(path)?;
    let mut bytes = [0; N];
    file.read_exact(&mut bytes).map_err(|_| invalid(id))?;
    Ok((bytes, metadata.len()))
}
fn tail<const N: usize>(path: &Path, id: &str) -> Result<[u8; N]> {
    let mut file = File::open(path)?;
    let mut bytes = [0; N];
    let start = file
        .metadata()?
        .len()
        .checked_sub(N as u64)
        .ok_or_else(|| invalid(id))?;
    file.seek(SeekFrom::Start(start))?;
    file.read_exact(&mut bytes).map_err(|_| invalid(id))?;
    Ok(bytes)
}
const fn le32(b: &[u8]) -> u32 {
    u32::from_le_bytes([b[0], b[1], b[2], b[3]])
}
const fn le64(b: &[u8]) -> u64 {
    u64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]])
}

fn validate(id: &str, path: &Path, opts: &ReadOptions) -> Result<()> {
    opts.ensure_active()?;
    let metadata = std::fs::symlink_metadata(path)?;
    if metadata.is_file() && metadata.len() > opts.max_input_bytes() {
        return Err(
            plenora_io_model::quota::Quota::InputBytes.exceeded(plenora_io_model::ErrorPhase::Read)
        );
    }
    if opts.row_limit.is_some() && matches!(id, "geojson" | "dxf") {
        return Ok(()); // The bounded decoder validates the observed prefix.
    }
    match id {
        "shp" => shapefile(path),
        "filegdb" => filegdb(path, opts),
        "csv" => csv(path, opts),
        "geojson" => {
            let mut reader = BufReader::new(File::open(path)?);
            if reader.fill_buf()?.starts_with(b"\xef\xbb\xbf") {
                reader.consume(3);
            }
            // La passata del driver verifica poi il catalogo FeatureCollection.
            // Qui basta la sintassi JSON completa, senza materializzare le feature.
            let mut deserializer = serde_json::Deserializer::from_reader(reader);
            let envelope = <GeoJsonEnvelope as serde::Deserialize>::deserialize(&mut deserializer)
                .map_err(|_| invalid(id))?;
            if !envelope.0 {
                return Err(Situation::InvalidContent.error_with(&PublicMessage::Curated("GeoJSON: richiesta FeatureCollection; una geometria nuda non e' un dataset")));
            }
            deserializer.end().map_err(|_| invalid(id))
        }
        "kml" => xml(path, opts),
        "dxf" => dxf(path, opts),
        "geoparquet" => {
            let (h, length) = header::<4>(path, id)?;
            let t = tail::<8>(path, id)?;
            let footer = u64::from(le32(&t));
            require(
                h == *b"PAR1"
                    && &t[4..] == b"PAR1"
                    && footer > 0
                    && footer <= length.saturating_sub(12),
                id,
            )
        }
        // Il driver IPC fornisce i due validatori completi a preflight_with.
        "ipc" => Ok(()),
        "xls" => {
            let archive = zip::ZipArchive::new(File::open(path)?).map_err(|_| invalid(id))?;
            for member in [
                "[Content_Types].xml",
                "xl/workbook.xml",
                "xl/_rels/workbook.xml.rels",
            ] {
                require(
                    archive
                        .file_names()
                        .any(|name| name.is_ok_and(|name| name == member)),
                    id,
                )?;
            }
            Ok(())
        }
        "gpkg" => gpkg(path),
        _ => Err(Situation::InvalidConfiguration.error()),
    }
}

fn csv(path: &Path, opts: &ReadOptions) -> Result<()> {
    let delimiter = opts
        .format_options
        .get("delimiter")
        .filter(|s| s.len() == 1 && s.is_ascii())
        .and_then(|s| s.as_bytes().first())
        .copied()
        .unwrap_or(b',');
    let mut reader = crate::csv_wire::Reader::new(File::open(path)?, delimiter);
    let mut record = csv::StringRecord::new();
    let mut rows = 0;
    while opts.row_limit.is_none_or(|limit| rows <= limit)
        && reader
            .read_record(&mut record)
            .map_err(crate::csv_wire::read_error)?
    {
        opts.ensure_active()?;
        if record.iter().any(|s| s.contains('\0')) {
            return Err(
                Situation::InvalidContent.error_with(&PublicMessage::Curated(
                    "CSV non valido: richiesto testo UTF-8",
                )),
            );
        }
        rows += 1;
    }
    require(rows > 0, "csv")
}
fn xml(path: &Path, opts: &ReadOptions) -> Result<()> {
    use quick_xml::events::Event;
    let mut reader = quick_xml::Reader::from_reader(quick_xml::encoding::DecodingReader::new(
        BufReader::new(File::open(path)?),
    ));
    let mut buffer = Vec::new();
    let mut depth = 0_u64;
    let mut root = false;
    loop {
        opts.ensure_active()?;
        match reader
            .read_event_into(&mut buffer)
            .map_err(|_| invalid("kml"))?
        {
            Event::Start(element) if depth == 0 => {
                require(!root && element.local_name().as_ref() == "kml", "kml")?;
                root = true;
                depth = 1;
            }
            Event::Empty(element) if depth == 0 => {
                require(!root && element.local_name().as_ref() == "kml", "kml")?;
                root = true;
            }
            Event::Start(_) => depth += 1,
            Event::End(_) => {
                depth = depth.checked_sub(1).ok_or_else(|| invalid("kml"))?;
            }
            Event::Text(text) if depth == 0 => require(
                text.as_ref().as_bytes().iter().all(u8::is_ascii_whitespace),
                "kml",
            )?,
            Event::Decl(declaration) => {
                if let Some(encoding) = declaration.encoder() {
                    reader.get_mut().set_encoding(encoding);
                }
            }
            Event::Eof => return require(root && depth == 0, "kml"),
            _ => {}
        }
        if root && opts.row_limit.is_some() {
            return Ok(());
        }
        buffer.clear();
    }
}
fn dxf(path: &Path, opts: &ReadOptions) -> Result<()> {
    let (h, _) = header::<2>(path, "dxf")?;
    if h == *b"Au" {
        let (h, _) = header::<22>(path, "dxf")?;
        require(&h == b"AutoCAD Binary DXF\r\n\x1a\0", "dxf")?;
        let t = tail::<4>(path, "dxf")?;
        return require(&t == b"EOF\0", "dxf");
    }
    let mut lines = BufReader::new(File::open(path)?).lines();
    let mut section = false;
    let mut eof = false;
    while let Some(code) = lines.next() {
        opts.ensure_active()?;
        let code = code.map_err(|_| invalid("dxf"))?;
        if code.trim().is_empty() && eof {
            continue;
        }
        let code: i32 = code.trim().parse().map_err(|_| invalid("dxf"))?;
        let value = lines
            .next()
            .ok_or_else(|| invalid("dxf"))?
            .map_err(|_| invalid("dxf"))?;
        require(!eof, "dxf")?;
        section |= code == 0 && value.trim() == "SECTION";
        eof = code == 0 && value.trim() == "EOF";
    }
    require(section && eof, "dxf")
}
fn shp_header(path: &Path) -> Result<()> {
    let (h, length) = header::<100>(path, "shp")?;
    let words = u32::from_be_bytes([h[24], h[25], h[26], h[27]]);
    require(
        h[..4] == [0, 0, 0x27, 0x0a] && le32(&h[28..]) == 1000 && u64::from(words) * 2 == length,
        "shp",
    )
}
fn shapefile(path: &Path) -> Result<()> {
    use plenora_io_core::dataset_files::{companion, required, resolve, resolve_exact};
    let main = if path.is_dir() {
        resolve(&path.join("data.shp"))?
            .ok_or_else(|| std::io::Error::from(std::io::ErrorKind::NotFound))?
    } else {
        path.to_owned()
    };
    shp_header(&main)?;
    if let Some(index) = resolve_exact(&companion(&main, "shx")?)? {
        shp_header(&index)?;
    }
    let dbf = required(&companion(&main, "dbf")?)?;
    let (h, length) = header::<32>(&dbf, "shp")?;
    let start = u64::from(u16::from_le_bytes([h[8], h[9]]));
    let record = u64::from(u16::from_le_bytes([h[10], h[11]]));
    require(
        start >= 33 && record > 0 && start + record * u64::from(le32(&h[4..])) <= length,
        "shp",
    )
}
fn filegdb(path: &Path, opts: &ReadOptions) -> Result<()> {
    require(path.is_dir(), "filegdb")?;
    require(
        path.join("a00000001.gdbtable").is_file() && path.join("a00000001.gdbtablx").is_file(),
        "filegdb",
    )?;
    for entry in std::fs::read_dir(path)? {
        opts.ensure_active()?;
        let path = entry?.path();
        if path.file_name().is_some_and(|name| name == "gdb") {
            let (h, length) = header::<8>(&path, "filegdb")?;
            require(length == 8 && h[4..] == [0xde, 0xad, 0xbe, 0xef], "filegdb")?;
        }
        match path
            .extension()
            .and_then(|s| s.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref()
        {
            Some("gdbtable") => {
                let (h, length) = header::<40>(&path, "filegdb")?;
                require(
                    matches!(le32(&h), 3 | 4)
                        && le64(&h[24..]) == length
                        && le64(&h[32..]) < length,
                    "filegdb",
                )?;
            }
            Some("gdbtablx") => {
                let (h, length) = header::<16>(&path, "filegdb")?;
                let width = u64::from(le32(&h[12..]));
                require(
                    matches!(le32(&h), 3 | 4)
                        && (4..=6).contains(&width)
                        && 16 + u64::from(le32(&h[4..])) * 1024 * width <= length,
                    "filegdb",
                )?;
            }
            Some("gdbindexes") => filegdb_indexes(&path)?,
            Some("spx" | "atx") => {
                require(std::fs::symlink_metadata(&path)?.is_file(), "filegdb")?;
                let (footer, length) = (
                    tail::<22>(&path, "filegdb")?,
                    std::fs::metadata(&path)?.len(),
                );
                require(
                    length >= 22
                        && (length - 22).is_multiple_of(4096)
                        && matches!(le32(&footer[18..]), 1 | 2),
                    "filegdb",
                )?;
            }
            _ => {}
        }
    }
    Ok(())
}

struct GeoJsonEnvelope(bool);
impl<'de> serde::Deserialize<'de> for GeoJsonEnvelope {
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error> {
        struct EnvelopeVisitor;
        impl<'de> serde::de::Visitor<'de> for EnvelopeVisitor {
            type Value = GeoJsonEnvelope;
            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("oggetto GeoJSON")
            }
            fn visit_map<M: serde::de::MapAccess<'de>>(
                self,
                mut map: M,
            ) -> std::result::Result<Self::Value, M::Error> {
                let mut recognized = false;
                while let Some(key) = map.next_key::<String>()? {
                    if key == "type" {
                        let kind = map.next_value::<String>()?;
                        recognized |= matches!(kind.as_str(), "Feature" | "FeatureCollection");
                    } else {
                        map.next_value::<serde::de::IgnoredAny>()?;
                    }
                }
                Ok(GeoJsonEnvelope(recognized))
            }
        }
        deserializer.deserialize_map(EnvelopeVisitor)
    }
}
fn gpkg(path: &Path) -> Result<()> {
    let (h, length) = header::<100>(path, "gpkg")?;
    let page = u64::from(u16::from_be_bytes([h[16], h[17]]));
    let page = if page == 1 { 65536 } else { page };
    require(
        &h[..16] == b"SQLite format 3\0"
            && page >= 512
            && page.is_power_of_two()
            && length >= page
            && length.is_multiple_of(page),
        "gpkg",
    )?;
    // Con contatori concordi SQLite dichiara il numero di pagine fisiche.
    if h[24..28] == h[92..96] {
        let pages = u32::from_be_bytes([h[28], h[29], h[30], h[31]]);
        require(u64::from(pages) * page <= length, "gpkg")?;
    }
    for member in plenora_io_core::dataset_files::GPKG.members {
        if plenora_io_core::dataset_files::resolve(&member.path(path))?.is_some() {
            return Err(
                Situation::InvalidContent.error_with(&PublicMessage::Curated(
                    "companion SQLite WAL/SHM/journal non ammesso nel GeoPackage",
                )),
            );
        }
    }
    Ok(())
}

// Catalogo degli indici: nomi UTF-16 con lunghezza, descrittore e campi.
// Gli scostamenti sono verificati contro i byte fisici prima di ogni seek.
fn filegdb_indexes(path: &Path) -> Result<()> {
    let (h, length) = header::<4>(path, "filegdb")?;
    let mut file = File::open(path)?;
    let mut position = 4_u64;
    let read_u32 = |file: &mut File, position: &mut u64| -> Result<u32> {
        require(*position + 4 <= length, "filegdb")?;
        file.seek(SeekFrom::Start(*position))?;
        let mut bytes = [0; 4];
        file.read_exact(&mut bytes)
            .map_err(|_| invalid("filegdb"))?;
        *position += 4;
        Ok(le32(&bytes))
    };
    for _ in 0..le32(&h) {
        let name = u64::from(read_u32(&mut file, &mut position)?);
        position += name * 2 + 2 + 6;
        let fields = read_u32(&mut file, &mut position)?;
        for _ in 0..fields {
            let name = u64::from(read_u32(&mut file, &mut position)?);
            position += name * 2 + 2;
            require(position <= length, "filegdb")?;
        }
    }
    require(position == length, "filegdb")
}
