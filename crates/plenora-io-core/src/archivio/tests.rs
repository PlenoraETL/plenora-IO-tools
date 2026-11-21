use super::*;
use plenora_io_model::{
    budget::{PipelineBudget, PipelineLimits},
    ErrorCategory,
};
use std::io::Write;

fn opts_con(limits: PipelineLimits) -> ReadOptions {
    ReadOptions::from_read_parts(
        PipelineBudget::builder()
            .limits(limits)
            .build()
            .unwrap()
            .into_read_parts(),
    )
}
fn opts() -> ReadOptions {
    opts_con(PipelineLimits::default())
}
fn u16_(n: usize) -> [u8; 2] {
    u16::try_from(n).unwrap().to_le_bytes()
}
fn u32_(n: usize) -> [u8; 4] {
    u32::try_from(n).unwrap().to_le_bytes()
}

/// Una voce del costruttore di prova: nome grezzo, flag, extra centrale.
struct V<'a> {
    nome: &'a [u8],
    dati: &'a [u8],
    flags: u16,
    extra: Vec<u8>,
}
fn v<'a>(nome: &'a str, dati: &'a [u8]) -> V<'a> {
    V {
        nome: nome.as_bytes(),
        dati,
        flags: 0x0800,
        extra: Vec::new(),
    }
}
/// ZIP minimale: `prefisso` anteposto senza correggere gli offset, `zip64`
/// con dimensioni e offset nel campo 0x0001 e record EOCD64 con locator.
fn costruisci(
    voci: &[V<'_>],
    deflate: bool,
    prefisso: &[u8],
    zip64: bool,
    commento: &[u8],
) -> Vec<u8> {
    let mut output = prefisso.to_vec();
    let base = output.len();
    let mut central = Vec::new();
    for voce in voci {
        let compressed = if deflate {
            let mut encoder =
                flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::best());
            encoder.write_all(voce.dati).unwrap();
            encoder.finish().unwrap()
        } else {
            voce.dati.to_vec()
        };
        let offset = output.len() - base;
        let crc = crc32fast::hash(voce.dati);
        let mut local = [0_u8; 30];
        local[..4].copy_from_slice(b"PK\x03\x04");
        local[4..6].copy_from_slice(&45_u16.to_le_bytes());
        local[6..8].copy_from_slice(&voce.flags.to_le_bytes());
        local[8..10].copy_from_slice(&(if deflate { 8_u16 } else { 0 }).to_le_bytes());
        local[14..18].copy_from_slice(&crc.to_le_bytes());
        local[18..22].copy_from_slice(&u32_(compressed.len()));
        local[22..26].copy_from_slice(&u32_(voce.dati.len()));
        local[26..28].copy_from_slice(&u16_(voce.nome.len()));
        let mut member = [0_u8; 46];
        member[..4].copy_from_slice(b"PK\x01\x02");
        member[6..32].copy_from_slice(&local[4..]);
        member[42..46].copy_from_slice(&u32_(offset));
        let mut extra = voce.extra.clone();
        if zip64 {
            member[20..28].copy_from_slice(&[0xff; 8]);
            member[42..46].copy_from_slice(&[0xff; 4]);
            extra.extend(1_u16.to_le_bytes());
            extra.extend(24_u16.to_le_bytes());
            extra.extend((voce.dati.len() as u64).to_le_bytes());
            extra.extend((compressed.len() as u64).to_le_bytes());
            extra.extend((offset as u64).to_le_bytes());
        }
        member[30..32].copy_from_slice(&u16_(extra.len()));
        output.extend(local);
        output.extend(voce.nome);
        output.extend(compressed);
        central.extend(member);
        central.extend(voce.nome);
        central.extend(extra);
    }
    let central_offset = output.len() - base;
    output.extend(&central);
    let mut end = [0_u8; 22];
    end[..4].copy_from_slice(b"PK\x05\x06");
    if zip64 {
        let record = output.len() - base;
        output.extend(b"PK\x06\x06");
        output.extend(44_u64.to_le_bytes());
        output.extend([45, 0, 45, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        output.extend((voci.len() as u64).to_le_bytes());
        output.extend((voci.len() as u64).to_le_bytes());
        output.extend((central.len() as u64).to_le_bytes());
        output.extend((central_offset as u64).to_le_bytes());
        output.extend(b"PK\x06\x07");
        output.extend(0_u32.to_le_bytes());
        output.extend((record as u64).to_le_bytes());
        output.extend(1_u32.to_le_bytes());
        end[8..12].copy_from_slice(&[0xff; 4]);
        end[12..20].copy_from_slice(&[0xff; 8]);
    } else {
        end[8..10].copy_from_slice(&u16_(voci.len()));
        end[10..12].copy_from_slice(&u16_(voci.len()));
        end[12..16].copy_from_slice(&u32_(central.len()));
        end[16..20].copy_from_slice(&u32_(central_offset));
    }
    end[20..22].copy_from_slice(&u16_(commento.len()));
    output.extend(end);
    output.extend(commento);
    output
}
fn zip(entries: &[(&str, &[u8])], deflate: bool) -> Vec<u8> {
    let voci: Vec<V<'_>> = entries.iter().map(|(n, d)| v(n, d)).collect();
    costruisci(&voci, deflate, &[], false, &[])
}
fn source(data: Vec<u8>) -> Sorgente {
    Sorgente::byte("input.zip", data)
}
fn situazione(error: &PlenoraIoError, attesa: Situation) {
    let rule = attesa.rule();
    assert_eq!(
        (error.category, error.code, error.message.as_str()),
        (rule.category, rule.code, rule.message),
        "{error:?}"
    );
}
fn quota(error: &PlenoraIoError) -> &str {
    &error
        .quota_details
        .as_ref()
        .expect("dettagli di quota")
        .quota
}
fn nessun_limite() -> LimitiArchivio {
    LimitiArchivio::default()
}

#[test]
fn stored_and_deflated_crc_roundtrip() {
    for deflate in [false, true] {
        let s = source(zip(&[("folder/a.csv", b"x,y\n1,2\n")], deflate));
        let entries = elenca_archivio(&s, &nessun_limite()).unwrap();
        assert_eq!(entries[0].nome, "folder/a.csv");
        assert_eq!(
            leggi_voce(&s, "FOLDER/A.CSV", &nessun_limite(), &opts()).unwrap(),
            b"x,y\n1,2\n"
        );
    }
}
#[test]
fn zip64_comment_with_fake_eocd_and_prepended_data() {
    let voci = [v("a.csv", b"x\n1\n")];
    let s = source(costruisci(&voci, true, &[], true, &[]));
    assert_eq!(
        leggi_voce(&s, "a.csv", &nessun_limite(), &opts()).unwrap(),
        b"x\n1\n"
    );
    // Una firma EOCD dentro il commento non e' l'EOCD: la lunghezza non torna.
    let s = source(costruisci(
        &voci,
        false,
        &[],
        false,
        b"PK\x05\x06xxxxxxxxxxxxxxxxxx",
    ));
    assert_eq!(elenca_archivio(&s, &nessun_limite()).unwrap().len(), 1);
    // Dati anteposti senza offset corretti (autoestraente): rifiutato.
    let s = source(costruisci(&voci, false, b"MZ-stub", false, &[]));
    situazione(
        &elenca_archivio(&s, &nessun_limite()).unwrap_err(),
        Situation::InvalidArchive,
    );
}
#[test]
fn limits_are_validated_bound_and_named() {
    for quota in [
        QuotaArchivio::Voci,
        QuotaArchivio::ByteDecompressi,
        QuotaArchivio::Rapporto,
    ] {
        let e = nessun_limite().con(quota, 0).unwrap_err();
        let rule = Situation::InvalidQuota.rule();
        assert_eq!((e.category, e.code), (rule.category, rule.code));
        assert!(e.message.starts_with(quota.regola().nome));
        assert_eq!(quota.regola().quota, quota);
    }
    let s = source(zip(&[("a", &vec![0; 100_000])], true));
    let e = elenca_archivio(&s, &nessun_limite()).unwrap_err();
    assert_eq!(quota(&e), "archive_max_ratio");
    let s = source(zip(&[("a", b"1"), ("b", b"2")], false));
    for (q, name) in [
        (QuotaArchivio::Voci, "archive_max_entries"),
        (QuotaArchivio::ByteDecompressi, "archive_max_bytes"),
    ] {
        let limits = nessun_limite().con(q, 1).unwrap();
        let e = elenca_archivio(&s, &limits).unwrap_err();
        assert_eq!(
            (e.category, quota(&e)),
            (ErrorCategory::ResourceLimit, name)
        );
    }
    // La quota della pipeline stringe il limite e si nomina: niente riduzione silenziosa.
    let stretta = opts_con(PipelineLimits::default().with_max_input_entries(1));
    let v = vincolo(QuotaArchivio::Voci, &nessun_limite(), Some(&stretta));
    assert_eq!((v.limit, v.bound_by), (1, "max_input_entries"));
    let e = elenca_archivio_con_opzioni(&s, &nessun_limite(), &stretta).unwrap_err();
    assert_eq!(quota(&e), "max_input_entries");
    let stretta = opts_con(PipelineLimits::default().with_decompression_ratio(2));
    let v = vincolo(QuotaArchivio::Rapporto, &nessun_limite(), Some(&stretta));
    assert_eq!((v.limit, v.bound_by), (2, "decompression_ratio"));
}
#[test]
fn global_ratio_counts_unselected_entries() {
    // Ogni voce sta sotto il rapporto per voce, il totale sull'archivio no.
    let dati = vec![7_u8; 4_000];
    let nomi: Vec<String> = (0..40).map(|i| format!("v{i}")).collect();
    let voci: Vec<V<'_>> = nomi.iter().map(|n| v(n, &dati)).collect();
    let archivio = costruisci(&voci, true, &[], false, &[]);
    let per_voce = 4_000_usize.div_ceil(archivio.len() / 40 - 30);
    let globale = (40 * 4_000_usize).div_ceil(archivio.len());
    assert!(globale < per_voce);
    let limits = nessun_limite()
        .con(QuotaArchivio::Rapporto, u64::try_from(globale - 1).unwrap())
        .unwrap();
    let e = elenca_archivio(&source(archivio), &limits).unwrap_err();
    assert_eq!(quota(&e), "archive_max_ratio");
}
#[test]
fn central_size_lie_stops_on_first_excess_byte() {
    let mut data = zip(&[("a", &vec![42; 100_000])], true);
    let central = data.windows(4).position(|w| w == b"PK\x01\x02").unwrap();
    data[central + 24..central + 28].copy_from_slice(&1_u32.to_le_bytes());
    let s = source(data);
    let limits = nessun_limite();
    assert!(elenca_archivio(&s, &limits).is_ok());
    situazione(
        &leggi_voce(&s, "a", &limits, &opts()).unwrap_err(),
        Situation::InvalidArchive,
    );
}
#[test]
fn local_header_must_match_central_and_entries_must_not_overlap() {
    let data = zip(&[("a.csv", b"payload")], false);
    let mut nome = data.clone();
    nome[30] = b'b';
    let mut metodo = data;
    metodo[8] = 8;
    for corrotto in [nome, metodo] {
        situazione(
            &leggi_voce(&source(corrotto), "a.csv", &nessun_limite(), &opts()).unwrap_err(),
            Situation::InvalidArchive,
        );
    }
    // Due voci centrali sullo stesso header locale: dati sovrapposti.
    let mut data = zip(&[("a", b"xx"), ("b", b"yy")], false);
    let second = data
        .windows(4)
        .enumerate()
        .filter(|(_, w)| *w == b"PK\x01\x02")
        .nth(1)
        .unwrap()
        .0;
    data[second + 42..second + 46].copy_from_slice(&0_u32.to_le_bytes());
    situazione(
        &elenca_archivio(&source(data), &nessun_limite()).unwrap_err(),
        Situation::InvalidArchive,
    );
}
#[test]
fn names_and_aliases_are_rejected_even_when_not_selected() {
    for name in [
        "../secret",
        "/etc/passwd",
        "C:/absolute",
        "a\\b",
        "a/./b",
        "a/../b",
        "CON",
        "con .txt",
        "CONIN$",
        "conout$.csv",
        "COM\u{b9}",
        "lpt\u{b3}.txt",
        "COM0",
        "a.",
        "a:b",
        "a//b",
        "a\0b",
    ] {
        situazione(
            &elenca_archivio(&source(zip(&[(name, b"x")], false)), &nessun_limite()).unwrap_err(),
            Situation::InvalidArchive,
        );
        situazione(
            &valida_nome(name).unwrap_err(),
            Situation::NonPortableSourceName,
        );
    }
    for names in [["A.csv", "a.csv"], ["dir", "dir/a"], ["\u{c9}", "\u{e9}"]] {
        situazione(
            &elenca_archivio(
                &source(zip(&[(names[0], b"x"), (names[1], b"y")], false)),
                &nessun_limite(),
            )
            .unwrap_err(),
            Situation::AmbiguousArchiveNames,
        );
    }
}
#[test]
fn names_cp437_utf8_flag_and_unicode_extra() {
    // 0x82 e' «e acuta» in CP437; senza bit 11 il nome e' CP437.
    let mut voce = V {
        nome: b"caf\x82.csv",
        dati: b"x",
        flags: 0,
        extra: Vec::new(),
    };
    let s = source(costruisci(
        std::slice::from_ref(&voce),
        false,
        &[],
        false,
        &[],
    ));
    assert_eq!(
        elenca_archivio(&s, &nessun_limite()).unwrap()[0].nome,
        "caf\u{e9}.csv"
    );
    // Info-ZIP 0x7075 vince se il CRC corrisponde al nome grezzo.
    let unicode = "caf\u{e8}.csv".as_bytes();
    voce.extra.extend(0x7075_u16.to_le_bytes());
    voce.extra.extend(u16_(5 + unicode.len()));
    voce.extra.push(1);
    voce.extra.extend(crc32fast::hash(voce.nome).to_le_bytes());
    voce.extra.extend(unicode);
    let s = source(costruisci(
        std::slice::from_ref(&voce),
        false,
        &[],
        false,
        &[],
    ));
    assert_eq!(
        elenca_archivio(&s, &nessun_limite()).unwrap()[0].nome,
        "caf\u{e8}.csv"
    );
    // Bit 11 con byte non UTF-8: archivio non valido.
    voce.flags = 0x0800;
    voce.extra.clear();
    let s = source(costruisci(&[voce], false, &[], false, &[]));
    situazione(
        &elenca_archivio(&s, &nessun_limite()).unwrap_err(),
        Situation::InvalidArchive,
    );
    for (byte, atteso) in (0x80_u8..=0xff).zip(CP437) {
        assert_eq!(
            decodifica_nome(&[byte], 0, None).unwrap(),
            atteso.to_string()
        );
    }
}
#[test]
fn prefix_compare_is_per_component_not_per_byte_length() {
    // «İ» si abbassa in tre byte: un taglio per lunghezza sposterebbe il resto.
    let radice = key("\u{130}X.gdb");
    assert_eq!(
        sotto("\u{130}x.GDB/a00000001.gdbtable", &radice),
        Some("a00000001.gdbtable")
    );
    assert_eq!(sotto("\u{130}x.gdbz/a", &radice), None);
    assert_eq!(sotto("\u{130}x.gdb", &radice), None);
}
#[test]
fn nested_missing_entry_method_and_symlink_are_typed() {
    let s = source(zip(&[("a.zip", b"x")], false)).voce("a.zip");
    situazione(
        &elenca_archivio(&s, &nessun_limite()).unwrap_err(),
        Situation::NestedArchive,
    );
    situazione(
        &leggi_voce(
            &source(zip(&[("a", b"x")], false)),
            "b",
            &nessun_limite(),
            &opts(),
        )
        .unwrap_err(),
        Situation::ArchiveEntryMissing,
    );
    let mut data = zip(&[("link", b"target")], false);
    let central = data.windows(4).position(|w| w == b"PK\x01\x02").unwrap();
    data[central + 38..central + 42].copy_from_slice(&(0xa000_u32 << 16).to_le_bytes());
    assert!(elenca_archivio(&source(data), &nessun_limite()).is_err());
    let mut data = zip(&[("a", b"x")], false);
    let central = data.windows(4).position(|w| w == b"PK\x01\x02").unwrap();
    data[central + 10] = 14;
    situazione(
        &elenca_archivio(&source(data), &nessun_limite()).unwrap_err(),
        Situation::UnsupportedArchiveFeature,
    );
}
#[test]
fn crc_and_truncation_are_rejected() {
    let mut data = zip(&[("a", b"payload")], false);
    data[31] ^= 1;
    assert!(leggi_voce(&source(data), "a", &nessun_limite(), &opts()).is_err());
    let data = zip(&[("a", b"payload")], false);
    for n in 0..data.len() {
        assert!(elenca_archivio(&source(data[..n].to_vec()), &nessun_limite()).is_err());
    }
}
#[test]
fn cancellation_between_decompression_blocks_and_memory_budget() {
    let s = source(zip(&[("a", &vec![42; 30_000])], false));
    let options = opts();
    let token = options.cancellation().clone();
    let mut archive = Archive::open(&s, &nessun_limite(), Some(&options)).unwrap();
    let mut written = 0;
    let error = archive
        .extract(0, &nessun_limite(), Some(&options), u64::MAX, |bytes| {
            written += bytes.len();
            token.cancel();
            Ok(())
        })
        .unwrap_err();
    assert_eq!(error.category, ErrorCategory::Cancelled);
    assert_eq!(written, 8192);
    let options = opts_con(
        PipelineLimits::default()
            .with_memory_bytes(1)
            .with_max_wkb_cell_bytes(1),
    );
    assert_eq!(
        leggi_voce(&s, "a", &nessun_limite(), &options)
            .unwrap_err()
            .category,
        ErrorCategory::ResourceLimit
    );
}
#[test]
fn kmz_document_is_the_first_root_kml() {
    let s = source(zip(
        &[
            ("files/x.kml", b"<a/>"),
            ("Main.KML", b"<b/>"),
            ("doc.kml", b"<c/>"),
            ("img.png", b"p"),
        ],
        false,
    ));
    assert_eq!(
        documento_kmz(&s, &nessun_limite(), &opts()).unwrap(),
        ("Main.KML".to_owned(), 2)
    );
    let s = source(zip(&[("files/x.kml", b"<a/>")], false));
    situazione(
        &documento_kmz(&s, &nessun_limite(), &opts()).unwrap_err(),
        Situation::ArchiveEntryMissing,
    );
    let s = source(zip(&[("a.kmz", b"x")], false)).voce("a.kmz");
    situazione(
        &documento_kmz(&s, &nessun_limite(), &opts()).unwrap_err(),
        Situation::NestedArchive,
    );
}
#[test]
fn prefix_reads_only_the_head_of_the_entry() {
    let s = source(zip(&[("a.arrow", b"ARROW1-rest-of-file")], true)).voce("a.arrow");
    assert_eq!(prefisso(&s, &nessun_limite(), 6).unwrap(), b"ARROW1");
    let corta = Sorgente::byte("x", &b"AR"[..]);
    assert_eq!(prefisso(&corta, &nessun_limite(), 6).unwrap(), b"AR");
}

fn estratti(dir: &Path) -> Vec<String> {
    let mut out = Vec::new();
    let mut pending = vec![dir.to_owned()];
    while let Some(d) = pending.pop() {
        for e in std::fs::read_dir(&d).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                pending.push(p);
            } else {
                let rel = p.strip_prefix(dir).unwrap().to_string_lossy();
                out.push(rel.replace('\\', "/"));
            }
        }
    }
    out.sort();
    out
}
#[test]
fn shapefile_members_follow_the_main_entry_folder() {
    let s = source(zip(
        &[
            ("uno/A.SHP", b"s"),
            ("UNO/a.dbf", b"d"),
            ("uno/a.Prj", b"p"),
            ("uno/a.cpg", b"c"),
            ("due/a.shx", b"x"),
            ("uno/b.shp", b"s2"),
            ("uno/b.dbf", b"d2"),
        ],
        false,
    ))
    .voce("uno/a.shp");
    let (path, vista) = prepara_percorso(s, &crate::dataset_files::SHP, &opts()).unwrap();
    let vista = vista.unwrap();
    let radice = vista.directory.path().to_owned();
    assert_eq!(path, radice.join("uno/a.shp"));
    // .prj e .cpg preservati; l'.shx di un'altra cartella e l'altro .shp no.
    assert_eq!(
        estratti(&radice),
        ["uno/a.Prj", "uno/a.cpg", "uno/a.dbf", "uno/a.shp"]
    );
    assert!(radice.starts_with(crate::driver::spool::spill_directory().unwrap()));
    drop(vista);
    assert!(!radice.exists(), "la vista si pulisce al drop");
}
#[test]
fn shapefile_directory_and_filegdb_subtrees() {
    let s = source(zip(
        &[
            ("x.shp.d/", b""),
            ("X.SHP.D/data.shp", b"s"),
            ("x.shp.d/data.dbf", b"d"),
            ("y.csv", b"c"),
        ],
        false,
    ))
    .voce("x.shp.d");
    let (path, vista) = prepara_percorso(s, &crate::dataset_files::SHP, &opts()).unwrap();
    let radice = vista.as_ref().unwrap().directory.path().to_owned();
    assert_eq!(path, radice.join("x.shp.d"));
    assert_eq!(estratti(&radice), ["x.shp.d/data.dbf", "x.shp.d/data.shp"]);
    let s = source(zip(
        &[("d.gdb/A00000001.GDBTABLE", b"t"), ("d.gdb/sub/x", b"y")],
        false,
    ))
    .voce("d.gdb");
    situazione(
        &prepara_percorso(s, &crate::dataset_files::FILEGDB, &opts()).unwrap_err(),
        Situation::InvalidArchive,
    );
}
#[test]
fn view_never_overwrites_a_filesystem_alias() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("a");
    crea_nuovo(&path).unwrap();
    situazione(
        &crea_nuovo(&path).unwrap_err(),
        Situation::AmbiguousArchiveNames,
    );
    // ς e σ hanno chiavi diverse: un filesystem che li confondesse da' errore,
    // mai una sovrascrittura; uno che li distingue conserva entrambi i file.
    let s = source(zip(
        &[("d.gdb/a\u{3c2}", b"1"), ("d.gdb/a\u{3c3}", b"2")],
        false,
    ))
    .voce("d.gdb");
    match prepara_percorso(s, &crate::dataset_files::FILEGDB, &opts()) {
        Ok((path, _vista)) => {
            assert_eq!(std::fs::read(path.join("a\u{3c2}")).unwrap(), b"1");
            assert_eq!(std::fs::read(path.join("a\u{3c3}")).unwrap(), b"2");
        }
        Err(e) => situazione(&e, Situation::AmbiguousArchiveNames),
    }
}
#[test]
fn bytes_name_and_spill_quota() {
    situazione(
        &prepara_percorso(
            Sorgente::byte("../x.csv", &b"a"[..]),
            &crate::dataset_files::SINGLE,
            &opts(),
        )
        .unwrap_err(),
        Situation::NonPortableSourceName,
    );
    let piccola = opts_con(PipelineLimits::default().with_spill_bytes(4));
    let e = prepara_percorso(
        Sorgente::byte("x.csv", vec![b'a'; 64]),
        &crate::dataset_files::SINGLE,
        &piccola,
    )
    .unwrap_err();
    assert_eq!(e.category, ErrorCategory::ResourceLimit);
}
