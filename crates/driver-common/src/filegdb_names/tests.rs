use super::*;
#[test]
fn every_member_case_combination_has_one_identity() {
    for suffix in [
        "gdbtable",
        "gdbtablx",
        "gdbindexes",
        "spx",
        "atx",
        "freelist",
        "field.atx",
        "hbar",
        "cdf",
        "sdf",
    ] {
        let name = format!("a00abcdef.{suffix}");
        let letters: Vec<_> = name
            .bytes()
            .enumerate()
            .filter_map(|(i, b)| b.is_ascii_alphabetic().then_some(i))
            .collect();
        for mask in 0..(1usize << letters.len()) {
            let mut variant = name.as_bytes().to_vec();
            for (bit, index) in letters.iter().enumerate() {
                if mask & (1 << bit) != 0 {
                    variant[*index].make_ascii_uppercase();
                }
            }
            assert_eq!(parse(std::str::from_utf8(&variant).unwrap()), parse(&name));
        }
    }
    for name in ["gdb", "timestamps"] {
        for mask in 0..(1usize << name.len()) {
            let mut variant = name.as_bytes().to_vec();
            for (bit, byte) in variant.iter_mut().enumerate() {
                if mask & (1 << bit) != 0 {
                    byte.make_ascii_uppercase();
                }
            }
            assert_eq!(parse(std::str::from_utf8(&variant).unwrap()), parse(name));
        }
    }
    for invalid in [
        "../a00000001.gdbtable",
        "a1.gdbtable",
        "a0000000z.atx",
        "a00000001.",
        "a+0000001.gdbtable",
        "a00000001..atx",
        "a00000001.field.atx.",
        "a00000001.x/y",
        "a00000001.x\\y",
    ] {
        assert!(parse(invalid).is_none(), "{invalid}");
    }
}

#[test]
fn operation_plan_contains_only_catalog_dependencies_and_requested_owner() {
    let root = tempfile::tempdir().unwrap();
    for id in 1..=15 {
        for suffix in [
            "gdbtable",
            "gdbtablx",
            "gdbindexes",
            "field.atx",
            "freelist",
        ] {
            std::fs::write(
                root.path()
                    .join(format!("A{id:08X}.{}", suffix.to_uppercase())),
                b"not opened by planning",
            )
            .unwrap();
        }
    }
    for name in ["TIMESTAMPS", "gdb", "foreign", "a00000009-.gdbtable"] {
        std::fs::create_dir(root.path().join(name)).unwrap();
    }
    let catalog = required(root.path(), Operation::Catalog).unwrap();
    assert_eq!(catalog.len(), 5);
    assert!(catalog.iter().all(|(_, member)| member.id == Some(1)));
    for id in 9..=15 {
        let members = required(root.path(), Operation::Object { id, catalog: &[4] }).unwrap();
        assert_eq!(members.len(), 15);
        assert!(members.iter().all(|(_, member)| {
            member
                .id
                .is_some_and(|owner| owner == 1 || owner == 4 || u64::from(owner) == id)
        }));
    }
}
