use super::*;
use plenora_io_model::{
    budget::{PipelineBudget, PipelineLimits},
    ErrorCategory,
};
use std::io::Write;

fn opts() -> ReadOptions {
    ReadOptions::from_read_parts(PipelineBudget::builder().build().unwrap().into_read_parts())
}
fn zip(entries: &[(&str, &[u8])], deflate: bool) -> Vec<u8> {
    let mut output = Vec::new();
    let mut central = Vec::new();
    for (name, data) in entries {
        let compressed = if deflate {
            let mut encoder =
                flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::best());
            encoder.write_all(data).unwrap();
            encoder.finish().unwrap()
        } else {
            data.to_vec()
        };
        let crc = crc32fast::hash(data);
        let mut local = [0_u8; 30];
        local[..4].copy_from_slice(b"PK\x03\x04");
        local[4..6].copy_from_slice(&20_u16.to_le_bytes());
        local[8..10].copy_from_slice(&(if deflate { 8_u16 } else { 0 }).to_le_bytes());
        local[14..18].copy_from_slice(&crc.to_le_bytes());
        local[18..22].copy_from_slice(&(compressed.len() as u32).to_le_bytes());
        local[22..26].copy_from_slice(&(data.len() as u32).to_le_bytes());
        local[26..28].copy_from_slice(&(name.len() as u16).to_le_bytes());
        let mut member = [0_u8; 46];
        member[..4].copy_from_slice(b"PK\x01\x02");
        member[6..32].copy_from_slice(&local[4..]);
        member[42..46].copy_from_slice(&(output.len() as u32).to_le_bytes());
        output.extend(local);
        output.extend(name.as_bytes());
        output.extend(compressed);
        central.extend(member);
        central.extend(name.as_bytes());
    }
    let mut end = [0_u8; 22];
    end[..4].copy_from_slice(b"PK\x05\x06");
    end[8..10].copy_from_slice(&(entries.len() as u16).to_le_bytes());
    end[10..12].copy_from_slice(&(entries.len() as u16).to_le_bytes());
    end[12..16].copy_from_slice(&(central.len() as u32).to_le_bytes());
    end[16..20].copy_from_slice(&(output.len() as u32).to_le_bytes());
    output.extend(central);
    output.extend(end);
    output
}
fn source(data: Vec<u8>) -> Source {
    Source::Bytes {
        nome: "input.zip".into(),
        dati: data.into(),
    }
}
#[test]
fn stored_and_deflated_crc_roundtrip() {
    for deflate in [false, true] {
        let s = source(zip(&[("folder/a.csv", b"x,y\n1,2\n")], deflate));
        let entries = elenca_archivio(&s, &LimitiArchivio::default()).unwrap();
        assert_eq!(entries[0].nome, "folder/a.csv");
        assert_eq!(
            leggi_voce(&s, "FOLDER/A.CSV", &LimitiArchivio::default(), &opts()).unwrap(),
            b"x,y\n1,2\n"
        );
    }
}
#[test]
fn bomb_ratio_count_and_declared_total_rejected() {
    let s = source(zip(&[("a", &[0; 100_000])], true));
    assert_eq!(
        elenca_archivio(&s, &LimitiArchivio::default())
            .unwrap_err()
            .category,
        ErrorCategory::ResourceLimit
    );
    let s = source(zip(&[("a", b"1"), ("b", b"2")], false));
    for limits in [
        LimitiArchivio {
            max_voci: 1,
            ..LimitiArchivio::default()
        },
        LimitiArchivio {
            max_byte_decompressi: 1,
            ..LimitiArchivio::default()
        },
    ] {
        assert_eq!(
            elenca_archivio(&s, &limits).unwrap_err().category,
            ErrorCategory::ResourceLimit
        );
    }
}
#[test]
fn central_size_lie_stops_on_first_excess_byte() {
    let mut data = zip(&[("a", &[42; 100_000])], true);
    let central = data.windows(4).position(|w| w == b"PK\x01\x02").unwrap();
    data[central + 24..central + 28].copy_from_slice(&1_u32.to_le_bytes());
    let s = source(data);
    let limits = LimitiArchivio::default();
    assert!(elenca_archivio(&s, &limits).is_ok());
    assert_eq!(
        leggi_voce(&s, "a", &limits, &opts()).unwrap_err().category,
        ErrorCategory::ResourceLimit
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
        "a.",
        "a:b",
        "a//b",
        "a\0b",
    ] {
        assert!(
            elenca_archivio(
                &source(zip(&[(name, b"x")], false)),
                &LimitiArchivio::default()
            )
            .is_err(),
            "{name:?}"
        );
    }
    for names in [["A.csv", "a.csv"], ["dir", "dir/a"], ["É", "é"]] {
        assert!(elenca_archivio(
            &source(zip(&[(names[0], b"x"), (names[1], b"y")], false)),
            &LimitiArchivio::default()
        )
        .is_err());
    }
}
#[test]
fn nested_source_and_symlink_are_rejected() {
    let s = Source::Archivio {
        archivio: Box::new(source(zip(&[("a.zip", b"x")], false))),
        voce: "a.zip".into(),
    };
    assert_eq!(
        elenca_archivio(&s, &LimitiArchivio::default())
            .unwrap_err()
            .category,
        ErrorCategory::Unsupported
    );
    let mut data = zip(&[("link", b"target")], false);
    let central = data.windows(4).position(|w| w == b"PK\x01\x02").unwrap();
    data[central + 38..central + 42].copy_from_slice(&(0xa000_u32 << 16).to_le_bytes());
    assert!(elenca_archivio(&source(data), &LimitiArchivio::default()).is_err());
}
#[test]
fn crc_and_truncation_are_rejected() {
    let mut data = zip(&[("a", b"payload")], false);
    data[31] ^= 1;
    assert!(leggi_voce(&source(data), "a", &LimitiArchivio::default(), &opts()).is_err());
    let data = zip(&[("a", b"payload")], false);
    for n in 0..data.len() {
        assert!(elenca_archivio(&source(data[..n].to_vec()), &LimitiArchivio::default()).is_err());
    }
}
#[test]
fn cancellation_between_decompression_blocks_and_memory_budget() {
    let s = source(zip(&[("a", &[42; 30_000])], false));
    let options = opts();
    let token = options.cancellation().clone();
    let mut archive = Archive::open(&s, &LimitiArchivio::default(), Some(&options)).unwrap();
    let mut written = 0;
    let error = archive
        .extract(0, &LimitiArchivio::default(), Some(&options), |bytes| {
            written += bytes.len();
            token.cancel();
            Ok(())
        })
        .unwrap_err();
    assert_eq!(error.category, ErrorCategory::Cancelled);
    assert_eq!(written, 8192);
    let limits = PipelineLimits::default()
        .with_memory_bytes(1)
        .with_max_wkb_cell_bytes(1);
    let options = ReadOptions::from_read_parts(
        PipelineBudget::builder()
            .limits(limits)
            .build()
            .unwrap()
            .into_read_parts(),
    );
    assert_eq!(
        leggi_voce(&s, "a", &LimitiArchivio::default(), &options)
            .unwrap_err()
            .category,
        ErrorCategory::ResourceLimit
    );
}
