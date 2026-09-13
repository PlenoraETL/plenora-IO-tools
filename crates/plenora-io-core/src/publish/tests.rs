//! Le prove di [`super`], in un file loro.
//!
//! Restano un **modulo figlio**: vedono i privati del genitore
//! esattamente come quando stavano dentro di lui, e non allargano di
//! una riga la superficie pubblica del crate.

use std::io::Write;

use super::*;

fn expected_durable_outcome() -> PublishOutcome {
    if cfg!(unix) {
        PublishOutcome::Published
    } else {
        PublishOutcome::PublishedButDurabilityUnconfirmed
    }
}

#[test]
fn staging_helpers_use_destination_parent_and_requested_suffix() {
    let directory = tempfile::tempdir().unwrap();
    let destination = directory.path().join("dataset.gpkg");
    let file = create_staged_file_with_suffix(&destination, ".gpkg").unwrap();
    assert_eq!(file.path().parent(), Some(directory.path()));
    assert_eq!(
        file.path().extension().and_then(|value| value.to_str()),
        Some("gpkg")
    );

    let staging = create_staged_dir(&destination).unwrap();
    assert_eq!(staging.path().parent(), Some(directory.path()));
}

#[test]
fn staged_file_owns_publish_lifecycle() {
    let directory = tempfile::tempdir().unwrap();
    let destination = directory.path().join("output.bin");
    let mut staging = StagedFile::new(&destination, false, 16).unwrap();
    let staging_path = staging.path().unwrap().to_owned();
    staging
        .as_file_mut()
        .unwrap()
        .write_all(b"payload")
        .unwrap();

    let (bytes, outcome) = staging.publish().unwrap();

    assert_eq!(bytes, 7);
    assert_eq!(outcome, PublishOutcome::Published);
    assert_eq!(std::fs::read(&destination).unwrap(), b"payload");
    assert!(!staging_path.exists());
    assert!(
        matches!(staging.path(), Err(error) if error.code == plenora_io_model::IoErrorCode::Contract)
    );
    assert!(
        matches!(staging.reopen(), Err(error) if error.code == plenora_io_model::IoErrorCode::Contract)
    );
    assert!(matches!(
        staging.publish(),
        Err(error) if error.code == plenora_io_model::IoErrorCode::Contract
    ));
}

#[test]
fn staged_file_limit_failure_is_terminal_and_never_publishes() {
    let directory = tempfile::tempdir().unwrap();
    let destination = directory.path().join("output.bin");
    let mut staging = StagedFile::new(&destination, false, 7).unwrap();
    staging
        .as_file_mut()
        .unwrap()
        .write_all(&[0_u8; 8])
        .unwrap();

    let result = staging.publish();

    assert!(
        matches!(result, Err(error) if error.code == plenora_io_model::IoErrorCode::LimitExceeded)
    );
    assert!(!destination.exists());
    assert!(matches!(
        staging.publish(),
        Err(error) if error.code == plenora_io_model::IoErrorCode::Contract
    ));
}

#[test]
fn unpublished_staged_file_is_removed_on_drop() {
    let directory = tempfile::tempdir().unwrap();
    let destination = directory.path().join("output.gpkg");
    let staging = StagedFile::with_suffix(&destination, ".gpkg", false, 16).unwrap();
    let staging_path = staging.path().unwrap().to_owned();
    assert_eq!(
        staging_path.extension().and_then(|value| value.to_str()),
        Some("gpkg")
    );

    drop(staging);

    assert!(!staging_path.exists());
    assert!(!destination.exists());
}

#[test]
fn output_limit_is_checked_before_publish() {
    let directory = tempfile::tempdir().unwrap();
    let destination = directory.path().join("output.bin");
    let mut temp = tempfile::NamedTempFile::new_in(directory.path()).unwrap();
    temp.write_all(&[0_u8; 8]).unwrap();
    let result = publish_file_atomic_limited(temp, &destination, false, 7);
    assert!(
        matches!(result, Err(error) if error.code == plenora_io_model::IoErrorCode::LimitExceeded)
    );
    assert!(!destination.exists());
}

#[cfg(any(
    target_os = "linux",
    target_os = "android",
    target_vendor = "apple",
    windows
))]
#[test]
fn directory_dataset_is_published_with_one_rename() {
    let root = tempfile::tempdir().unwrap();
    let staging = tempfile::Builder::new().tempdir_in(root.path()).unwrap();
    std::fs::write(staging.path().join("data.shp"), b"shape").unwrap();
    std::fs::create_dir(staging.path().join("nested")).unwrap();
    std::fs::write(staging.path().join("nested").join("index"), b"index").unwrap();
    let destination = root.path().join("dataset.shp.d");

    let outcome = publish_dir_atomic(staging.path(), &destination, true).unwrap();

    assert_eq!(outcome, expected_durable_outcome());
    assert_eq!(
        std::fs::read(destination.join("data.shp")).unwrap(),
        b"shape"
    );
    assert_eq!(
        std::fs::read(destination.join("nested").join("index")).unwrap(),
        b"index"
    );
}

#[test]
fn directory_publish_is_no_clobber() {
    let root = tempfile::tempdir().unwrap();
    let staging = tempfile::Builder::new().tempdir_in(root.path()).unwrap();
    std::fs::write(staging.path().join("data.shp"), b"new").unwrap();
    let destination = root.path().join("dataset.shp.d");
    std::fs::create_dir(&destination).unwrap();
    std::fs::write(destination.join("sentinel"), b"existing").unwrap();

    let result = publish_dir_atomic(staging.path(), &destination, false);

    assert!(
        matches!(result, Err(error) if error.code == plenora_io_model::IoErrorCode::OutputExists)
    );
    assert_eq!(
        std::fs::read(destination.join("sentinel")).unwrap(),
        b"existing"
    );
    assert!(!destination.join("data.shp").exists());
}

#[test]
fn atomic_noclobber_refuses_a_file_created_after_preflight() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("staged.dbf");
    let destination = root.path().join("dataset.dbf");
    std::fs::write(&source, b"new").unwrap();
    ensure_destination_absent(&destination).unwrap();

    std::fs::write(&destination, b"concurrent").unwrap();
    let result = rename_noclobber(&source, &destination);

    assert!(
        matches!(result, Err(error) if error.code == plenora_io_model::IoErrorCode::OutputExists)
    );
    assert_eq!(std::fs::read(&destination).unwrap(), b"concurrent");
    assert_eq!(std::fs::read(&source).unwrap(), b"new");
}

#[cfg(any(
    target_os = "linux",
    target_os = "android",
    target_vendor = "apple",
    windows
))]
#[test]
fn atomic_noclobber_refuses_a_directory_created_after_preflight() {
    let root = tempfile::tempdir().unwrap();
    let staging = tempfile::Builder::new().tempdir_in(root.path()).unwrap();
    std::fs::write(staging.path().join("data"), b"new").unwrap();
    let destination = root.path().join("dataset");
    ensure_destination_absent(&destination).unwrap();

    std::fs::create_dir(&destination).unwrap();
    std::fs::write(destination.join("sentinel"), b"concurrent").unwrap();
    let result = rename_noclobber(staging.path(), &destination);

    assert!(
        matches!(result, Err(error) if error.code == plenora_io_model::IoErrorCode::OutputExists)
    );
    assert_eq!(
        std::fs::read(destination.join("sentinel")).unwrap(),
        b"concurrent"
    );
    assert_eq!(std::fs::read(staging.path().join("data")).unwrap(), b"new");
}

#[cfg(unix)]
#[test]
fn directory_publish_rejects_symlinks_even_when_not_durable() {
    use std::os::unix::fs::symlink;

    let root = tempfile::tempdir().unwrap();
    let staging = tempfile::Builder::new().tempdir_in(root.path()).unwrap();
    let target = root.path().join("outside");
    std::fs::write(&target, b"outside").unwrap();
    symlink(&target, staging.path().join("link")).unwrap();
    let destination = root.path().join("dataset");

    let result = publish_dir_atomic(staging.path(), &destination, false);

    assert!(matches!(
        result,
        Err(error) if error.code == plenora_io_model::IoErrorCode::Io
    ));
    assert!(!destination.exists());
    assert_eq!(std::fs::read(target).unwrap(), b"outside");
}

#[test]
fn loose_set_preflight_fails_before_first_rename() {
    let root = tempfile::tempdir().unwrap();
    let staging = tempfile::Builder::new().tempdir_in(root.path()).unwrap();
    let first_source = staging.path().join("data.dbf");
    std::fs::write(&first_source, b"dbf").unwrap();
    let files = vec![
        (first_source, root.path().join("data.dbf")),
        (
            staging.path().join("missing.shp"),
            root.path().join("data.shp"),
        ),
    ];

    assert!(publish_files_ordered_limited(&files, false, u64::MAX).is_err());
    assert!(!root.path().join("data.dbf").exists());
    assert!(!root.path().join("data.shp").exists());
}

/// Una destinazione occupata **prima** del preflight non arriva ai rename.
///
/// Il nome precedente di questa sonda — «error after first rename rolls
/// back» — descriveva un caso che il corpo non produceva: il preflight
/// falliva, quindi nessun rename avveniva e nessun rollback veniva
/// esercitato. La proprieta' che il corpo prova e' vera e vale la pena
/// tenerla, ma e' un'altra: l'ordine preflight → ciclo fa si' che un
/// `OutputExists` sul secondo file lasci lo staging intatto.
///
/// Il caso che il vecchio nome prometteva e' provato dalle due sonde qui
/// sotto, che i rename li fanno davvero.
#[test]
fn loose_set_occupied_destination_fails_before_any_rename() {
    let root = tempfile::tempdir().unwrap();
    let staging = tempfile::Builder::new().tempdir_in(root.path()).unwrap();
    let source_dbf = staging.path().join("data.dbf");
    let source_shp = staging.path().join("data.shp");
    std::fs::write(&source_dbf, b"dbf").unwrap();
    std::fs::write(&source_shp, b"shape").unwrap();
    // Occupiamo la seconda destinazione con una directory: il
    // preflight `ensure_destination_absent` restituisce `OutputExists`
    // e fallisce prima del primo rename. Questo garantisce che il
    // primo rename NON avvenga, quindi non c'e' rollback e non
    // c'e' pubblicazione parziale. Il test blocca l'invariante
    // "un OutputExists sul secondo file lascia lo staging intatto",
    // che e' la conseguenza dell'ordine preflight→loop.
    let destination_dbf = root.path().join("dataset.dbf");
    let destination_shp = root.path().join("dataset.shp");
    std::fs::create_dir(&destination_shp).unwrap();
    let files = vec![
        (source_dbf.clone(), destination_dbf.clone()),
        (source_shp.clone(), destination_shp),
    ];
    let result = publish_files_ordered_limited(&files, false, u64::MAX);
    let error = result.expect_err("il preflight deve fallire su destinazione occupata");
    // Nessun rename e' avvenuto: RemoteEffect resta None, i sorgenti
    // sono ancora nello staging, la prima destinazione non esiste.
    assert_eq!(error.remote_effect, RemoteEffect::None);
    assert!(
        source_dbf.exists(),
        "il source_dbf deve restare nello staging"
    );
    assert!(
        source_shp.exists(),
        "il source_shp deve restare nello staging"
    );
    assert!(
        !destination_dbf.exists(),
        "il rename non deve essere avvenuto"
    );
}

/// Un set loose con due file diretti alla **stessa** destinazione.
///
/// E' il modo per far fallire un rename *intermedio* su un filesystem vero,
/// senza finestre di corsa: il preflight vede la destinazione libera due
/// volte — perche' libera lo e' davvero, in quel momento — e il ciclo la
/// occupa al primo rename. Il secondo trova un `renameat` no-clobber che
/// rifiuta, ed e' il rifiuto che la produzione incontrerebbe se qualcuno
/// scrivesse quel nome fra preflight e publish.
fn set_con_destinazione_duplicata(staging: &Path, destinazione: &Path) -> Vec<(PathBuf, PathBuf)> {
    let primo = staging.join("data.dbf");
    let secondo = staging.join("data.shp");
    std::fs::write(&primo, b"dbf").unwrap();
    std::fs::write(&secondo, b"shape").unwrap();
    let unica = destinazione.join("dataset.dbf");
    vec![(primo, unica.clone()), (secondo, unica)]
}

/// Il rollback che non riesce mai.
///
/// Sostituisce cio' che nessun permesso di filesystem sa produrre: andata e
/// ritorno attraversano le stesse due directory, quindi togliere il diritto
/// al ritorno lo toglie anche all'andata, e il rename intermedio non
/// avverrebbe.
fn rollback_impossibile(_: &Path, _: &Path) -> std::io::Result<()> {
    Err(std::io::Error::from(std::io::ErrorKind::PermissionDenied))
}

/// Rollback riuscito: il rename intermedio fallisce, niente resta visibile.
#[test]
fn loose_set_intermediate_rename_failure_rolls_back_and_reports_none() {
    let root = tempfile::tempdir().unwrap();
    let staging = tempfile::Builder::new().tempdir_in(root.path()).unwrap();
    let files = set_con_destinazione_duplicata(staging.path(), root.path());
    let destinazione = files[0].1.clone();

    let error = publish_files_ordered_limited(&files, false, u64::MAX)
        .expect_err("il secondo rename deve trovare la destinazione occupata");

    assert_eq!(error.code, plenora_io_model::IoErrorCode::OutputExists);
    // Le tre affermazioni che rendono vero `RemoteEffect::None`: nessuna
    // destinazione visibile, ed entrambi i sorgenti tornati allo staging.
    assert_eq!(error.remote_effect, RemoteEffect::None);
    assert_eq!(error.retry, RetryDisposition::Never);
    assert!(
        !destinazione.exists(),
        "il rollback non ha tolto la destinazione gia' pubblicata"
    );
    assert!(
        files[0].0.exists(),
        "il primo file non e' tornato in staging"
    );
    assert!(
        files[1].0.exists(),
        "il secondo file non deve essere partito"
    );
}

/// Rollback fallito: l'errore dichiara `Partial`, e la dichiarazione e'
/// vera.
///
/// L'asserzione che conta e' l'ultima: la destinazione **e' rimasta sul
/// disco**. Senza di essa la sonda direbbe soltanto che `with_effect` scrive
/// due campi, che e' cio' che la versione precedente verificava — e un
/// `RemoteEffect::Partial` dichiarato su un filesystem pulito sarebbe un
/// allarme falso, cioe' il difetto opposto e altrettanto grave.
#[test]
fn loose_set_rollback_failure_reports_partial_and_leaves_the_destination() {
    let root = tempfile::tempdir().unwrap();
    let staging = tempfile::Builder::new().tempdir_in(root.path()).unwrap();
    let files = set_con_destinazione_duplicata(staging.path(), root.path());
    let destinazione = files[0].1.clone();

    let error = commit_ordered_renames(&files, rollback_impossibile)
        .expect_err("il secondo rename deve trovare la destinazione occupata");

    assert_eq!(error.code, plenora_io_model::IoErrorCode::OutputExists);
    assert_eq!(error.remote_effect, RemoteEffect::Partial);
    assert_eq!(error.retry, RetryDisposition::RequiresRecovery);
    assert!(
        destinazione.exists(),
        "l'errore dichiara Partial ma il disco e' pulito: la dichiarazione sarebbe falsa"
    );
    assert!(
        !files[0].0.exists(),
        "il primo file e' ancora in staging: non c'e' niente di parziale da dichiarare"
    );
}

#[test]
fn loose_set_durable_publish_preserves_ordered_files() {
    let root = tempfile::tempdir().unwrap();
    let staging = tempfile::Builder::new().tempdir_in(root.path()).unwrap();
    let source_dbf = staging.path().join("data.dbf");
    let source_shp = staging.path().join("data.shp");
    std::fs::write(&source_dbf, b"dbf").unwrap();
    std::fs::write(&source_shp, b"shape").unwrap();
    let destination_dbf = root.path().join("dataset.dbf");
    let destination_shp = root.path().join("dataset.shp");
    let files = vec![
        (source_dbf, destination_dbf.clone()),
        (source_shp, destination_shp.clone()),
    ];

    let (bytes, outcome) = publish_files_ordered_limited(&files, true, u64::MAX).unwrap();

    assert_eq!(bytes, 8);
    assert_eq!(outcome, expected_durable_outcome());
    assert_eq!(std::fs::read(destination_dbf).unwrap(), b"dbf");
    assert_eq!(std::fs::read(destination_shp).unwrap(), b"shape");
}

#[cfg(windows)]
#[test]
fn durable_file_publish_reports_unconfirmed_parent_directory() {
    let root = tempfile::tempdir().unwrap();
    let destination = root.path().join("output.bin");
    let mut temp = NamedTempFile::new_in(root.path()).unwrap();
    temp.write_all(b"durable").unwrap();

    let (_, outcome) = publish_file_atomic(temp, &destination, true).unwrap();

    assert_eq!(outcome, PublishOutcome::PublishedButDurabilityUnconfirmed);
    assert_eq!(std::fs::read(destination).unwrap(), b"durable");
}

#[cfg(any(target_os = "linux", windows))]
#[test]
fn cross_filesystem_publish_is_rejected_before_any_output_is_visible() {
    let Some(cross_root) = std::env::var_os("PLENORA_CROSS_FS_TEST_ROOT") else {
        return;
    };
    let source_root = tempfile::tempdir_in(cross_root).unwrap();
    let destination_root = tempfile::tempdir().unwrap();
    assert!(
        !same_filesystem(source_root.path(), destination_root.path()).unwrap(),
        "PLENORA_CROSS_FS_TEST_ROOT deve indicare un filesystem distinto"
    );

    let staging = tempfile::Builder::new()
        .prefix("directory-")
        .tempdir_in(source_root.path())
        .unwrap();
    std::fs::write(staging.path().join("data"), b"directory").unwrap();
    let directory_destination = destination_root.path().join("dataset");
    let directory_result = publish_dir_atomic(staging.path(), &directory_destination, false);
    assert!(matches!(
        directory_result,
        Err(error)
            if error.code == plenora_io_model::IoErrorCode::Unsupported
                && error.message.contains("cross-filesystem")
    ));
    assert!(staging.path().join("data").exists());
    assert!(!directory_destination.exists());

    let mut temp = NamedTempFile::new_in(source_root.path()).unwrap();
    temp.write_all(b"single-file").unwrap();
    let file_destination = destination_root.path().join("output.bin");
    let file_result = publish_file_atomic(temp, &file_destination, false);
    assert!(matches!(
        file_result,
        Err(error)
            if error.code == plenora_io_model::IoErrorCode::Unsupported
                && error.message.contains("cross-filesystem")
    ));
    assert!(!file_destination.exists());

    let loose_source = source_root.path().join("data.shp");
    std::fs::write(&loose_source, b"shape").unwrap();
    let loose_destination = destination_root.path().join("data.shp");
    let loose_result = publish_files_ordered_limited(
        &[(loose_source.clone(), loose_destination.clone())],
        false,
        u64::MAX,
    );
    assert!(matches!(
        loose_result,
        Err(error)
            if error.code == plenora_io_model::IoErrorCode::Unsupported
                && error.message.contains("cross-filesystem")
    ));
    assert!(loose_source.exists());
    assert!(!loose_destination.exists());
}
