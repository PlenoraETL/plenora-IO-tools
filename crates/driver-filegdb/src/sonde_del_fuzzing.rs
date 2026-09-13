//! Le prove di [`super`], in un file loro.
//!
//! Restano un **modulo figlio**: vedono i privati del genitore
//! esattamente come quando stavano dentro di lui, e non allargano di
//! una riga la superficie pubblica del crate.

use super::*;

// --- la superficie del fuzz target, e la fixture che la alimenta -------
//
// Una build che compila e un replay senza crash non dimostrano che gli
// input raggiungano il driver: una `.gdb` rifiutata al riconoscimento del
// formato non fa crashare niente ed e', da fuori, indistinguibile da una
// letta per intero. Queste sonde chiamano lo **stesso** entry point del
// target sulla **stessa** fixture, e guardano che cosa ne esce.

/// La fixture vive accanto al target che la usa.
#[cfg(feature = "gdal-backend")]
fn archivio_della_fixture() -> Vec<u8> {
    let percorso = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fuzz/fixtures/filegdb/citta.gdb.bundle");
    std::fs::read(&percorso)
        .unwrap_or_else(|errore| panic!("fixture {} non leggibile: {errore}", percorso.display()))
}

/// Limiti dello stesso ordine di quelli della campagna.
///
/// Non sono gli **stessi**: `harness::limits()` vive nella crate di
/// fuzzing, e un driver non puo' dipenderne. Quel che conta e' che ci siano
/// tetti -- una sonda che leggesse senza limiti percorrerebbe una strada
/// che il fuzzer non percorre mai.
#[cfg(feature = "gdal-backend")]
fn opzioni_di_campagna() -> ReadOptions {
    let limiti = plenora_io_model::budget::PipelineLimits::default()
        .with_max_input_bytes(1_048_576)
        .with_max_rows(100_000)
        .with_memory_bytes(64 * 1024 * 1024);
    let bundle = plenora_io_model::budget::PipelineBudget::builder()
        .limits(limiti)
        .build()
        .expect("limiti della campagna validi");
    ReadOptions::from_read_parts(bundle.into_read_parts())
}

/// L'input vuoto materializza la fixture **intatta**, e la fixture si
/// legge.
///
/// E' il caso che tiene onesta l'intera campagna: senza, un verde potrebbe
/// voler dire che nessun input arriva mai al driver.
#[test]
#[cfg(feature = "gdal-backend")]
fn la_fixture_intatta_arriva_al_drenaggio() {
    let righe = __fuzz_leggi_gdb(&archivio_della_fixture(), &[], opzioni_di_campagna())
        .expect("la fixture deve essere letta: se no il target non copre il parsing");
    assert_eq!(
        righe, 2,
        "due feature nel GeoJSON di partenza: un conteggio diverso vuol dire \
             che la fixture non attraversa piu' il drenaggio"
    );
}

/// Sostituire una parte qualunque non fa panicare il driver.
///
/// Non si pretende che **tutte** falliscano: alcune tabelle di metadati
/// sono facoltative, e una `.gdb` puo' restare leggibile. Cio' che si
/// pretende e' che l'esito sia un esito -- `Ok` o `Err`, mai un panico e
/// mai un abort.
#[test]
#[cfg(feature = "gdal-backend")]
fn ogni_parte_sostituita_da_un_esito_non_un_panico() {
    let archivio = archivio_della_fixture();
    let parti = __fuzz_parti_della_fixture(&archivio).expect("archivio leggibile");
    assert!(parti.len() >= 2, "una .gdb ha piu' di una parte");

    let mut rifiutate = 0;
    for indice in 0..parti.len() {
        let mut input = vec![u8::try_from(indice).expect("meno di 256 parti")];
        input.extend_from_slice(b"NON-E-UNA-TABELLA-FILEGDB");
        if __fuzz_leggi_gdb(&archivio, &input, opzioni_di_campagna()).is_err() {
            rifiutate += 1;
        }
    }
    assert!(
        rifiutate > 0,
        "sostituire una tabella con testo non puo' lasciare la .gdb sempre \
             leggibile: se nessuna sostituzione viene rifiutata, l'input non sta \
             arrivando al driver"
    );
}

/// **Isolamento fra invocazioni**, provato dalla directory e non dall'esito.
///
/// La prima stesura di questa sonda rompeva una parte e poi rileggeva la
/// fixture intatta, aspettandosi che riuscisse. Non discriminava niente: la
/// materializzazione riscrive **tutte** le parti a ogni invocazione, quindi
/// anche riusando la stessa directory la seconda lettura sarebbe riuscita.
/// Provava che la riscrittura funziona, non che la directory sia nuova.
///
/// Cio' che conta e' la directory. Due invocazioni devono usarne due
/// diverse, e nessuna delle due deve sopravvivere alla propria chiamata --
/// altrimenti una campagna lunga riempirebbe il disco di `.gdb`.
#[test]
#[cfg(feature = "gdal-backend")]
fn ogni_invocazione_ha_la_propria_directory() {
    let archivio = archivio_della_fixture();
    let parti = __fuzz_parti_della_fixture(&archivio).expect("archivio leggibile");
    let tabella = parti
        .iter()
        .position(|(nome, _)| nome.ends_with(".gdbtable"))
        .expect("la fixture ha almeno una tabella");

    ULTIMA_DIRECTORY.with(|viste| viste.borrow_mut().clear());

    let mut rotta = vec![u8::try_from(tabella).expect("indice piccolo")];
    rotta.extend_from_slice(b"SPAZZATURA");
    let _ = __fuzz_leggi_gdb(&archivio, &rotta, opzioni_di_campagna());
    let dopo = __fuzz_leggi_gdb(&archivio, &[], opzioni_di_campagna());
    assert!(dopo.is_ok(), "la fixture intatta deve leggersi: {dopo:?}");

    let viste = ULTIMA_DIRECTORY.with(|viste| viste.borrow().clone());
    assert_eq!(viste.len(), 2, "due invocazioni, due materializzazioni");
    assert_ne!(
        viste[0], viste[1],
        "le due invocazioni hanno usato la **stessa** directory: la tabella \
             rotta della prima e' rimasta li' per la seconda, e l'esito non lo \
             mostra perche' ogni parte viene riscritta"
    );
    for percorso in &viste {
        assert!(
            !percorso.exists(),
            "la directory {} e' sopravvissuta alla propria invocazione: una \
                 campagna lunga riempirebbe il disco",
            percorso.display()
        );
    }
}

/// **I nomi dei file non vengono dal payload.**
///
/// Il primo byte sceglie un **indice**, e l'indice e' preso modulo il numero
/// di parti: nessun byte del fuzzer finisce in un percorso. La sonda prova
/// che l'insieme dei file materializzati e' sempre quello della fixture,
/// qualunque cosa l'input dica.
#[test]
#[cfg(feature = "gdal-backend")]
fn nessun_percorso_deriva_dal_payload() {
    let archivio = archivio_della_fixture();
    let parti = __fuzz_parti_della_fixture(&archivio).expect("archivio leggibile");
    let attesi: std::collections::BTreeSet<&str> =
        parti.iter().map(|(nome, _)| nome.as_str()).collect();

    let temporanea = tempfile::tempdir().expect("directory temporanea");
    // Un indice ben oltre il numero di parti, e un contenuto che somiglia a
    // un percorso: ne' l'uno ne' l'altro devono uscire dalla directory.
    let dataset = materializza_gdb(
        temporanea.path(),
        &parti,
        usize::from(u8::MAX) % parti.len(),
        b"../../fuori.txt",
    )
    .expect("materializzazione");

    let prodotti: std::collections::BTreeSet<String> = std::fs::read_dir(&dataset)
        .expect("la directory del dataset esiste")
        .map(|voce| {
            voce.expect("voce leggibile")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    let prodotti: std::collections::BTreeSet<&str> = prodotti.iter().map(String::as_str).collect();
    assert_eq!(
        prodotti, attesi,
        "i nomi materializzati sono quelli della fixture"
    );
    assert!(
        !temporanea.path().join("fuori.txt").exists(),
        "nessun file e' stato scritto fuori dalla .gdb"
    );
}

/// **Fail-closed dell'archivio**: un archivio corrotto non fa scrivere
/// niente e non panica.
///
/// L'archivio non e' l'input del fuzzer -- e' nostro, e sta nel binario --
/// ma un archivio troncato o con un nome di parte inventato non deve poter
/// far scrivere fuori dalla directory.
#[test]
#[cfg(feature = "gdal-backend")]
fn un_archivio_malformato_non_produce_parti() {
    let archivio = archivio_della_fixture();
    assert!(__fuzz_parti_della_fixture(&archivio).is_some());

    // Senza intestazione.
    assert!(__fuzz_parti_della_fixture(b"qualcosa").is_none());
    // Intestazione giusta e niente altro.
    assert!(__fuzz_parti_della_fixture(b"PLENORA-GDB-FIXTURE-1\n").is_none());
    // Troncato a meta' di una parte.
    for taglio in [24, 40, archivio.len() / 2, archivio.len() - 1] {
        assert!(
            __fuzz_parti_della_fixture(&archivio[..taglio]).is_none(),
            "troncato a {taglio} byte"
        );
    }
    // Byte di coda che l'indice non dichiara.
    let mut lungo = archivio;
    lungo.push(0);
    assert!(__fuzz_parti_della_fixture(&lungo).is_none());

    // Un nome di parte che **risale** la directory. E' il caso che la
    // prima stesura accettava, perche' `".."` e' fatto di soli caratteri
    // ammessi, ed e' il solo per cui questa funzione guarda il nome: il
    // nome finisce in un `join`, e da li' si scriverebbe fuori dalla
    // `.gdb`. Il gemello Python ha la sua sonda; questa mancava, e la
    // riga di rifiuto restava l'unica difesa di questo ramo mai eseguita.
    let mut ostile = b"PLENORA-GDB-FIXTURE-1\n".to_vec();
    ostile.extend_from_slice(&1_u32.to_le_bytes());
    ostile.extend_from_slice(&2_u16.to_le_bytes());
    ostile.extend_from_slice(b"..");
    ostile.extend_from_slice(&1_u32.to_le_bytes());
    ostile.push(b'x');
    assert!(
        __fuzz_parti_della_fixture(&ostile).is_none(),
        "un nome di parte che risale la directory non e' un nome di parte"
    );
}

/// Un archivio che dichiara **zero** parti non e' una `.gdb` vuota.
///
/// Accettarlo portava il chiamante a scegliere la parte da sostituire
/// **modulo zero**: una divisione per zero su qualunque input non vuoto.
/// L'archivio non viene dal fuzzer, ma un archivio corrotto non deve poter
/// far panicare il target.
#[test]
#[cfg(feature = "gdal-backend")]
fn un_archivio_senza_parti_e_rifiutato() {
    let mut vuoto = b"PLENORA-GDB-FIXTURE-1\n".to_vec();
    vuoto.extend_from_slice(&0_u32.to_le_bytes());
    assert!(
        __fuzz_parti_della_fixture(&vuoto).is_none(),
        "zero parti: il chiamante dividerebbe per zero"
    );

    // E il target lo rifiuta come errore tipizzato, non con un panico.
    let esito = __fuzz_leggi_gdb(&vuoto, b"\x01qualcosa", opzioni_di_campagna());
    assert!(esito.is_err(), "un archivio senza parti non e' leggibile");
}

/// I nomi ammessi sono quelli che `OpenFileGDB` scrive, e nessun altro.
#[test]
#[cfg(feature = "gdal-backend")]
fn un_nome_di_parte_non_puo_essere_un_percorso() {
    for buono in ["gdb", "a00000001.gdbtable", "timestamps", "a00000009.spx"] {
        assert!(nome_di_parte_ammesso(buono), "{buono}");
    }
    for cattivo in [
        "",
        "..",
        "../fuori",
        "a/b",
        "a\\b",
        "MAIUSCOLO",
        "con spazio",
    ] {
        assert!(!nome_di_parte_ammesso(cattivo), "{cattivo}");
    }
}

/// **Errore d'ambiente e non finding**: una radice che non esiste produce un
/// errore tipizzato, non un panico.
#[test]
#[cfg(feature = "gdal-backend")]
fn una_radice_non_creabile_e_un_errore_di_ambiente() {
    let temporanea = tempfile::tempdir().expect("directory temporanea");
    // Un **file** dove la materializzazione vuole una directory: `create_dir_all`
    // fallisce, ed e' l'ambiente a non collaborare, non il file letto.
    let ostacolo = temporanea.path().join("citta.gdb");
    std::fs::write(&ostacolo, b"non sono una directory").expect("scrittura");

    let archivio = archivio_della_fixture();
    let parti = __fuzz_parti_della_fixture(&archivio).expect("archivio leggibile");
    let errore = materializza_gdb(temporanea.path(), &parti, parti.len(), b"")
        .expect_err("una .gdb non creabile deve fallire");
    assert!(
        errore.message.contains("ambiente"),
        "un errore d'ambiente non va confuso con un difetto del file letto: {errore:?}"
    );
}
