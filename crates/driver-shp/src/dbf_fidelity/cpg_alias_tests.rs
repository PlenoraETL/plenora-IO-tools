use super::*;

#[test]
fn explicit_aliases_decode_cp1252_and_system_never_guesses() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("encoding.dbf");
    std::fs::write(&path, [0_u8; 32]).unwrap();
    for alias in ["ANSI 1252", " ansi  1252 ", "Cp 1252", " windows-1252 "] {
        std::fs::write(path.with_extension("cpg"), alias).unwrap();
        let encoding = TextEncoding::from_path(&path).unwrap();
        assert_eq!(encoding.decode(&[0x80, 0xe9]).unwrap(), "€é");
    }
    for alias in ["System", " system ", "ANSI", "ANSI 9999"] {
        std::fs::write(path.with_extension("cpg"), alias).unwrap();
        let error = TextEncoding::from_path(&path).unwrap_err();
        assert!(error.message.contains("codepage originale supportata"));
    }
}
