use super::*;

#[test]
fn enumeration_covers_every_declared_member() {
    for spec in FILE_SETS {
        for member in spec.members {
            let dir = tempfile::tempdir().unwrap();
            let main = dir.path().join("dataset.main");
            let logical = member.path(&main);
            assert!(resolve(&logical).unwrap().is_none());
            spec.ensure_absent(&main).unwrap();
            let upper = logical.with_file_name(
                logical
                    .file_name()
                    .unwrap()
                    .to_str()
                    .unwrap()
                    .to_uppercase(),
            );
            std::fs::write(&upper, b"foreign-data").unwrap();
            assert_eq!(resolve(&logical).unwrap(), Some(upper.clone()));
            assert_eq!(spec.ensure_absent(&main).is_err(), logical.exists());
            assert_eq!(std::fs::read(&upper).unwrap(), b"foreign-data");
            // The name enumerator is the filesystem seam: both candidates
            // are supplied even on a case-insensitive filesystem.
            assert_eq!(
                select_name(
                    logical.file_name().unwrap(),
                    [
                        Ok(upper.file_name().unwrap().to_owned()),
                        Ok(logical.file_name().unwrap().to_owned())
                    ],
                )
                .unwrap(),
                Some(logical.file_name().unwrap().to_owned())
            );
            std::fs::remove_file(&upper).unwrap();
            std::fs::write(&logical, b"member").unwrap();
            assert_eq!(resolve(&logical).unwrap(), Some(logical));
        }
        for suffix in spec.staging_suffixes {
            let dir = tempfile::tempdir().unwrap();
            let main = dir.path().join("dataset.gdb");
            let stage = dir
                .path()
                .join(format!("DATASET.PLENORA-TMP-42{suffix}").to_uppercase());
            std::fs::write(stage, b"orphan").unwrap();
            assert_eq!(
                spec.ensure_absent(&main).unwrap_err().code,
                plenora_io_model::IoErrorCode::OutputExists
            );
        }
    }
}

#[test]
fn primary_and_required_member_presence_follow_the_table() {
    for spec in FILE_SETS {
        let dir = tempfile::tempdir().unwrap();
        let main = dir.path().join("main.data");
        assert!(resolve(&main).unwrap().is_none());
        spec.ensure_absent(&main).unwrap();
        let upper = dir.path().join("MAIN.DATA");
        std::fs::write(&upper, b"main").unwrap();
        assert_eq!(resolve(&main).unwrap(), Some(upper.clone()));
        assert_eq!(spec.ensure_absent(&main).is_err(), main.exists());
        assert_eq!(
            select_name(
                main.file_name().unwrap(),
                [
                    Ok(OsString::from("main.data")),
                    Ok(OsString::from("MAIN.DATA"))
                ]
            )
            .unwrap(),
            Some(OsString::from("main.data"))
        );
        for member in spec.members {
            std::fs::write(member.path(&upper), b"data").unwrap();
        }
        for member in spec.members {
            let path = member.path(&upper);
            std::fs::remove_file(&path).unwrap();
            assert_eq!(
                spec.paths(&upper).is_err(),
                member.read == ReadMember::Required
            );
            std::fs::write(path, b"data").unwrap();
        }
    }
}

#[test]
fn ambiguous_fallback_requires_no_exact_name() {
    assert!(select_name(
        std::ffi::OsStr::new("a.PrJ"),
        [Ok(OsString::from("a.prj")), Ok(OsString::from("a.PRJ"))]
    )
    .is_err());
}

#[test]
fn native_case_rules_for_sqlite_and_unrelated_datasets() {
    let dir = tempfile::tempdir().unwrap();
    let main = dir.path().join("a.gpkg");
    std::fs::write(dir.path().join("A.GPKG"), b"database").unwrap();
    assert_eq!(GPKG.ensure_absent(&main).is_err(), main.exists());
    std::fs::remove_file(dir.path().join("A.GPKG")).unwrap();
    std::fs::write(dir.path().join("a.gpkg-WAL"), b"journal").unwrap();
    assert_eq!(
        GPKG.ensure_absent(&main).is_err(),
        dir.path().join("a.gpkg-wal").exists()
    );
}

#[test]
fn output_keeps_requested_case() {
    for (main, companion_name) in [
        ("VEC.SHP", "VEC.DBF"),
        ("VEC.ShP", "VEC.DbF"),
        ("vec.shp", "vec.dbf"),
    ] {
        assert_eq!(
            output_companion(Path::new(main), "dbf"),
            Path::new(companion_name)
        );
        assert_eq!(output_companion(Path::new(main), "shp"), Path::new(main));
    }
}

#[test]
fn case_sensitive_filesystem_keeps_exact_names_and_rejects_ambiguous_fallback() {
    let directory = tempfile::tempdir().unwrap();
    let lower = directory.path().join("a.prj");
    let upper = directory.path().join("a.PRJ");
    std::fs::write(&lower, b"lower").unwrap();
    if upper.exists() {
        return;
    } // Windows case-insensitive volume; exercised on Linux.
    std::fs::write(&upper, b"upper").unwrap();
    assert_eq!(resolve(&lower).unwrap(), Some(lower.clone()));
    assert_eq!(resolve(&upper).unwrap(), Some(upper.clone()));
    assert!(resolve(&directory.path().join("a.PrJ")).is_err());
    assert_eq!(std::fs::read(lower).unwrap(), b"lower");
    assert_eq!(std::fs::read(upper).unwrap(), b"upper");
    std::fs::write(directory.path().join("a.shp"), b"first").unwrap();
    std::fs::write(directory.path().join("A.shp"), b"second").unwrap();
    assert_eq!(
        required(&directory.path().join("a.shp")).unwrap(),
        directory.path().join("a.shp")
    );
    assert_eq!(
        required(&directory.path().join("A.shp")).unwrap(),
        directory.path().join("A.shp")
    );
}

#[cfg(unix)]
#[test]
fn directory_symlink_is_rejected_before_the_structural_decoder() {
    let directory = tempfile::tempdir().unwrap();
    let real = directory.path().join("real");
    std::fs::create_dir(&real).unwrap();
    std::fs::write(real.join("data.gpkg"), b"data").unwrap();
    let link = directory.path().join("link");
    std::os::unix::fs::symlink(&real, &link).unwrap();
    let error = GPKG
        .input_validated(&link.join("data.gpkg"), |_| {
            panic!("decoder must not follow link")
        })
        .unwrap_err();
    assert!(error.message.contains("symlink"));
}
