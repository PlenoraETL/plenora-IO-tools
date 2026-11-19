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
