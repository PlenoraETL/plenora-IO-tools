//! Le prove di [`super`], in un file loro.
//!
//! Restano un **modulo figlio**: vedono i privati del genitore
//! esattamente come quando stavano dentro di lui, e non allargano di
//! una riga la superficie pubblica del crate.

/// WKT con i tetti predefiniti: qui si provano le fixture, non le quote.
fn wkt(testo: &str) -> plenora_io_model::Result<plenora_io_model::wkb::WkbGeometry> {
    super::parse_wkt_bounded(testo, &plenora_io_model::limits::WkbLimits::default())
}
use super::*;

// --- ASSURANCE-N1: i rami negativi del pre-filtro sui riferimenti ---

/// `valida_valore_riferimento`: gli input che farebbero traboccare
/// `calamine` non arrivano al parser.
///
/// La sonda prova **il contratto del pre-filtro**, che non e' la conformita'
/// del riferimento. Vale la pena dire perche', perche' la prima stesura di
/// questa sonda ha sbagliato in tutti e due i versi: prima pretendeva il
/// rifiuto di `AA` -- descrivendo una grammatica che la funzione non
/// promette -- e poi, corretta, lo chiamava «riferimento valido», che e'
/// altrettanto falso. `AA` non e' un riferimento di cella: e' un token che
/// non puo' far traboccare l'accumulatore, e il pre-filtro lascia passare
/// esattamente quello.
///
/// Le tre affermazioni, separate:
///
/// 1. **fermato** cio' che eccede i conteggi del formato, con i due rifiuti
///    tenuti distinti -- forma non A1 e lunghezza oltre i limiti;
/// 2. **passano** riferimenti conformi rappresentativi, estremi inclusi;
///    il percorso end-to-end su un workbook conforme resta la prova che il
///    pre-filtro non rifiuti il caso reale;
/// 3. **passano anche** valori lessicalmente innocui la cui conformita' e'
///    falsa o dipende dal tipo dell'attributo: il prefiltro li delega al
///    parser successivo, senza affermare che il driver li accetti.
#[test]
fn n1_valida_valore_riferimento_applica_i_tetti_senza_validare_il_formato() {
    // 1. Fermato: piu' di tre lettere o piu' di sette cifre. L'overflow di
    //    `calamine` diventa possibile solo a lunghezze maggiori -- sette
    //    lettere o dieci cifre. Questi casi sono fermati perche' eccedono i
    //    massimi lessicali del formato, non perche' ciascuno di essi
    //    traboccherebbe davvero.
    let oltre_i_conteggi: Vec<&str> = vec![
        "ABCD1",
        "XFDA1",
        "AAAAAAA1",
        "A12345678",
        "A1234567890",
        "A1:ZZZZ1",
    ];
    for valore in oltre_i_conteggi {
        let Err(errore) = valida_valore_riferimento(valore.as_bytes()) else {
            panic!("«{valore}» eccede i conteggi del formato e doveva essere fermato");
        };
        assert!(
            errore.message.contains("oltre i limiti del formato"),
            "«{valore}»: atteso il rifiuto sui limiti, arrivato «{}»",
            errore.message
        );
    }

    // Il secondo rifiuto, distinto dal primo: cifre prima delle lettere, o
    // caratteri che non sono ne' l'une ne' l'altre.
    let forma_non_a1: Vec<&str> = vec!["1A", "A1B", "A-1", "A1:2B"];
    for valore in forma_non_a1 {
        let Err(errore) = valida_valore_riferimento(valore.as_bytes()) else {
            panic!("«{valore}» non ha la forma lettere-poi-cifre e doveva essere fermato");
        };
        assert!(
            errore.message.contains("atteso stile A1"),
            "«{valore}»: atteso il rifiuto di forma, arrivato «{}»",
            errore.message
        );
    }

    // 2. Passano riferimenti conformi rappresentativi, estremi compresi.
    //    L'esaustivita' non viene inventata qui: il test sul workbook
    //    conforme esercita il pre-filtro nel suo percorso reale.
    let conformi: Vec<&str> = vec!["A1", "A1:C4", "XFD1048576", "XFD1", "A1048576"];
    for valore in conformi {
        assert!(
            valida_valore_riferimento(valore.as_bytes()).is_ok(),
            "«{valore}» e' conforme e non deve essere fermato"
        );
    }

    // 3. Passano anche, ed e' **voluto**, valori che il pre-filtro puo'
    //    inoltrare senza rischio di overflow. Alcuni non sono conformi,
    //    per altri la conformita' dipende dal tipo dell'attributo: questa
    //    funzione non ha quel contesto e non emette un verdetto. Il test
    //    fissa la delega al parser, non l'accettazione da parte del driver.
    let tollerati_dal_prefiltro: Vec<(&str, &str)> = vec![
        ("", "nessun token da accumulare"),
        (
            "AA",
            "sole lettere: non e' uno ST_CellRef, gli manca la riga",
        ),
        ("12", "sole cifre: valido per row@r, non come ST_CellRef"),
        (
            "$A$1",
            "il dollaro viene separato senza decidere il tipo dell'attributo",
        ),
        (
            "A1 B2 C3",
            "la lista viene separata senza decidere il tipo dell'attributo",
        ),
        (
            "A1,B2",
            "l'unione viene separata senza decidere il tipo dell'attributo",
        ),
        ("A:A", "colonna intera: non ammessa da dimension@ref"),
        ("1:1", "riga intera: non ammessa da dimension@ref"),
        ("XFE1", "tre lettere, ma oltre l'ultima colonna XFD"),
        ("A1048577", "sette cifre, ma oltre l'ultima riga"),
    ];
    for (valore, perche) in tollerati_dal_prefiltro {
        assert!(
            valida_valore_riferimento(valore.as_bytes()).is_ok(),
            "«{valore}» ({perche}) viene oggi delegato al parser: fermarlo qui \
                 cambia il confine del pre-filtro e richiede una decisione esplicita"
        );
    }
}

/// `cell_at`: i due modi in cui una colonna puo' non esistere.
///
/// Una colonna **prima** dell'inizio dichiarato non produce un offset --
/// `checked_sub` fallisce -- e una colonna oltre la fine produce un offset
/// che la riga non contiene. Sono due rifiuti diversi, e la sonda li
/// distingue: confonderli manderebbe chi legge a cercare il difetto nel
/// posto sbagliato.
#[test]
fn n1_cell_at_distingue_la_colonna_prima_dell_inizio_da_quella_oltre_la_fine() {
    let riga = vec![Data::Int(1), Data::Int(2), Data::Int(3)];
    let bounds = SheetBounds {
        start: (0, 5),
        end: (10, 7),
    };

    for (colonna, atteso) in [(5_u32, 1_i64), (6, 2), (7, 3)] {
        let Ok(Data::Int(valore)) = cell_at(&riga, bounds, colonna) else {
            panic!("la colonna {colonna} e' dentro le dimensioni dichiarate");
        };
        assert_eq!(*valore, atteso, "colonna {colonna}");
    }

    for colonna in [0_u32, 1, 4] {
        let Err(errore) = cell_at(&riga, bounds, colonna) else {
            panic!("la colonna {colonna} precede l'inizio dichiarato");
        };
        assert!(
            errore.message.contains("indice colonna XLSX non valido"),
            "colonna {colonna}: arrivato «{}»",
            errore.message
        );
    }

    for colonna in [8_u32, 9, u32::MAX] {
        let Err(errore) = cell_at(&riga, bounds, colonna) else {
            panic!("la colonna {colonna} eccede la riga");
        };
        assert!(
            errore.message.contains("fuori dalle dimensioni dichiarate"),
            "colonna {colonna}: arrivato «{}»",
            errore.message
        );
    }
}

/// `LettoreCelleSorvegliato`: dopo un fallimento non si legge piu', e le due
/// vie di lettura lo dicono con lo stesso errore.
///
/// Il tipo promette che «dopo un panico il lettore viene scartato» sia una
/// proprieta' del **tipo** e non una convenzione da ricordare: al primo
/// fallimento il lettore cade e ogni chiamata successiva trova `None`.
/// Nessun percorso del driver ci prova -- tutti propagano -- ed e' proprio
/// per questo che le due guardie non erano mai state eseguite.
///
/// La sonda costruisce lo stato invalidato direttamente, che e' l'unico modo
/// di raggiungerle senza far panicare `calamine` davvero: il tipo e' privato
/// del modulo e il campo pure, quindi la prova vive accanto a cio' che prova
/// e non finge di passare da un'API che non lo permette.
///
/// # Che cosa **non** prova
///
/// Che la guardia non sia a consumo. Partendo da `None` un `take()` e un
/// `as_mut()` si comportano identicamente -- entrambi lasciano il campo a
/// `None` e ogni chiamata successiva fallisce -- quindi la ripetizione qui
/// sotto osserva che il rifiuto e' **stabile**, non che sia `as_mut()` a
/// produrlo. Distinguere i due vorrebbe partire da un lettore vivo e
/// invalidarlo, cioe' far fallire `calamine` davvero.
#[test]
fn n1_un_lettore_invalidato_rifiuta_entrambe_le_letture() {
    // `std::io::Cursor` soddisfa `Read + Seek` e non viene mai toccato: il
    // lettore e' gia' `None`, quindi nessuna chiamata a `calamine` parte.
    let mut invalidato: LettoreCelleSorvegliato<'_, std::io::Cursor<Vec<u8>>> =
        LettoreCelleSorvegliato { lettore: None };

    let Err(su_dimensioni) = invalidato.dimensioni() else {
        panic!("un lettore invalidato non puo' dichiarare dimensioni");
    };
    assert!(
        su_dimensioni.message.contains(LETTORE_INVALIDATO),
        "dimensioni: arrivato «{}»",
        su_dimensioni.message
    );

    let Err(su_cella) = invalidato.prossima_cella() else {
        panic!("un lettore invalidato non puo' consegnare celle");
    };
    assert!(
        su_cella.message.contains(LETTORE_INVALIDATO),
        "prossima_cella: arrivato «{}»",
        su_cella.message
    );

    // Il rifiuto e' stabile: interrogarlo di nuovo non lo fa cambiare idea.
    // Non dice quale forma abbia la guardia -- vedi «Che cosa non prova».
    assert!(invalidato.dimensioni().is_err());
    assert!(invalidato.prossima_cella().is_err());
}

/// Un central directory ZIP64 costruito a mano, che **dichiara** dimensioni
/// senza contenerle.
///
/// Non e' uno ZIP64 pienamente conforme: le voci non hanno payload, e un
/// lettore che provasse a **estrarle** fallirebbe. Cio' che la fixture
/// costruisce, e l'unica cosa che serve qui, e' un central directory che
/// `ZipArchive::new` accetta e da cui `compressed_size()` e `size()`
/// restituiscono i valori dichiarati -- che e' esattamente la superficie su
/// cui `validate_archive_ratio` lavora, perche' somma cio' che le voci
/// dichiarano senza aprirle.
///
/// Serve a misurare una cosa sola: se il crate `zip` restituisca i valori
/// dichiarati nel central directory senza confrontarli con i byte che
/// l'archivio contiene davvero. Da quella risposta dipende se i due
/// `checked_add` di `validate_archive_ratio` siano raggiungibili.
///
/// Due voci, entrambe `stored` e vuote, ciascuna con la propria coppia
/// `(compressa, decompressa)`.
///
/// Le due dimensioni sono **separate** e non e' un dettaglio di comodo:
/// `validate_archive_ratio` le somma in due accumulatori distinti, e la
/// prima stesura di questa fixture passava lo stesso valore a entrambi.
/// Con `u64::MAX` e `1` su tutti e due, alla seconda voce traboccava per
/// prima la somma dei compressi e il ramo dei decompressi non veniva mai
/// raggiunto: una sonda verde che copriva un `checked_add` su due.
fn zip64_con_dimensioni_dichiarate(voci: [(u64, u64); 2]) -> Vec<u8> {
    let mut archivio = Vec::new();
    let nomi = ["xl/worksheets/sheet1.xml", "xl/workbook.xml"];
    let mut offset_locali = Vec::new();

    for (nome, (compressa, decompressa)) in nomi.iter().zip(voci.iter()) {
        offset_locali.push(archivio.len() as u64);
        archivio.extend_from_slice(&0x0403_4b50_u32.to_le_bytes()); // firma locale
        archivio.extend_from_slice(&45_u16.to_le_bytes()); // versione: ZIP64
        archivio.extend_from_slice(&0_u16.to_le_bytes()); // flag
        archivio.extend_from_slice(&0_u16.to_le_bytes()); // metodo: stored
        archivio.extend_from_slice(&0_u16.to_le_bytes()); // ora
        archivio.extend_from_slice(&0_u16.to_le_bytes()); // data
        archivio.extend_from_slice(&0_u32.to_le_bytes()); // crc32
        archivio.extend_from_slice(&u32::MAX.to_le_bytes()); // compresso: vedi ZIP64
        archivio.extend_from_slice(&u32::MAX.to_le_bytes()); // decompresso: vedi ZIP64
        let lunghezza_nome =
            u16::try_from(nome.len()).expect("i nomi della sonda stanno in un u16");
        archivio.extend_from_slice(&lunghezza_nome.to_le_bytes());
        archivio.extend_from_slice(&20_u16.to_le_bytes()); // extra: 4 + 16
        archivio.extend_from_slice(nome.as_bytes());
        archivio.extend_from_slice(&0x0001_u16.to_le_bytes()); // tag ZIP64
        archivio.extend_from_slice(&16_u16.to_le_bytes());
        // Nessun byte di dati segue: la dichiarazione e' tutto cio' che conta.
        archivio.extend_from_slice(&decompressa.to_le_bytes());
        archivio.extend_from_slice(&compressa.to_le_bytes());
    }

    let inizio_central = archivio.len() as u64;
    for ((nome, (compressa, decompressa)), offset) in
        nomi.iter().zip(voci.iter()).zip(offset_locali.iter())
    {
        archivio.extend_from_slice(&0x0201_4b50_u32.to_le_bytes()); // firma central
        archivio.extend_from_slice(&45_u16.to_le_bytes()); // creato da
        archivio.extend_from_slice(&45_u16.to_le_bytes()); // richiede
        archivio.extend_from_slice(&0_u16.to_le_bytes()); // flag
        archivio.extend_from_slice(&0_u16.to_le_bytes()); // metodo
        archivio.extend_from_slice(&0_u16.to_le_bytes()); // ora
        archivio.extend_from_slice(&0_u16.to_le_bytes()); // data
        archivio.extend_from_slice(&0_u32.to_le_bytes()); // crc32
        archivio.extend_from_slice(&u32::MAX.to_le_bytes()); // compresso
        archivio.extend_from_slice(&u32::MAX.to_le_bytes()); // decompresso
        let lunghezza_nome =
            u16::try_from(nome.len()).expect("i nomi della sonda stanno in un u16");
        archivio.extend_from_slice(&lunghezza_nome.to_le_bytes());
        archivio.extend_from_slice(&28_u16.to_le_bytes()); // extra: 4 + 24
        archivio.extend_from_slice(&0_u16.to_le_bytes()); // commento
        archivio.extend_from_slice(&0_u16.to_le_bytes()); // disco
        archivio.extend_from_slice(&0_u16.to_le_bytes()); // attributi interni
        archivio.extend_from_slice(&0_u32.to_le_bytes()); // attributi esterni
        archivio.extend_from_slice(&u32::MAX.to_le_bytes()); // offset: vedi ZIP64
        archivio.extend_from_slice(nome.as_bytes());
        archivio.extend_from_slice(&0x0001_u16.to_le_bytes());
        archivio.extend_from_slice(&24_u16.to_le_bytes());
        archivio.extend_from_slice(&decompressa.to_le_bytes());
        archivio.extend_from_slice(&compressa.to_le_bytes());
        archivio.extend_from_slice(&offset.to_le_bytes()); // offset locale
    }
    let dimensione_central = archivio.len() as u64 - inizio_central;
    let offset_zip64_eocd = archivio.len() as u64;

    archivio.extend_from_slice(&0x0606_4b50_u32.to_le_bytes()); // ZIP64 EOCD
    archivio.extend_from_slice(&44_u64.to_le_bytes()); // dimensione residua
    archivio.extend_from_slice(&45_u16.to_le_bytes());
    archivio.extend_from_slice(&45_u16.to_le_bytes());
    archivio.extend_from_slice(&0_u32.to_le_bytes()); // disco
    archivio.extend_from_slice(&0_u32.to_le_bytes()); // disco del central
    archivio.extend_from_slice(&(nomi.len() as u64).to_le_bytes()); // voci sul disco
    archivio.extend_from_slice(&(nomi.len() as u64).to_le_bytes()); // voci totali
    archivio.extend_from_slice(&dimensione_central.to_le_bytes());
    archivio.extend_from_slice(&inizio_central.to_le_bytes());

    archivio.extend_from_slice(&0x0706_4b50_u32.to_le_bytes()); // localizzatore
    archivio.extend_from_slice(&0_u32.to_le_bytes());
    archivio.extend_from_slice(&offset_zip64_eocd.to_le_bytes());
    archivio.extend_from_slice(&1_u32.to_le_bytes());

    archivio.extend_from_slice(&0x0605_4b50_u32.to_le_bytes()); // EOCD
    archivio.extend_from_slice(&0_u16.to_le_bytes());
    archivio.extend_from_slice(&0_u16.to_le_bytes());
    archivio.extend_from_slice(&u16::MAX.to_le_bytes()); // vedi ZIP64
    archivio.extend_from_slice(&u16::MAX.to_le_bytes());
    archivio.extend_from_slice(&u32::MAX.to_le_bytes());
    archivio.extend_from_slice(&u32::MAX.to_le_bytes());
    archivio.extend_from_slice(&0_u16.to_le_bytes()); // commento
    archivio
}

/// Un caso della tabella di overflow: il nome, le due voci dell'archivio
/// come `(compressa, decompressa)`, e il messaggio che deve arrivare.
type CasoDiOverflow = (&'static str, [(u64, u64); 2], &'static str);

/// I due `checked_add` di `validate_archive_ratio` sono raggiungibili, **uno
/// per volta**, e la somma delle dimensioni dichiarate fallisce chiusa.
///
/// # Perche' serviva un archivio costruito a mano
///
/// Con ZIP32 l'overflow non e' raggiungibile per aritmetica: le dimensioni
/// stanno in campi `u32` -- al piu' 4 294 967 295 ciascuna -- e il conteggio
/// delle voci nell'EOCD e' un `u16`, al piu' 65 535. La somma massima e'
/// circa 2,8 x 10^14, cinque ordini di grandezza sotto `u64::MAX`. Nessuna
/// libreria che rispetti il formato puo' portarci.
///
/// ZIP64 cambia i due campi in `u64`, e la domanda diventa una sola: il
/// crate `zip` restituisce cio' che il central directory **dichiara**, o lo
/// confronta con i byte presenti? Questa sonda lo misura invece di
/// supporlo, ed e' la ragione per cui il gruppo non era dichiarabile
/// difensivo.
///
/// # Perche' due casi e non uno
///
/// Gli accumulatori sono due e il primo che trabocca ferma la funzione. Con
/// la stessa coppia di valori su compresso e decompresso -- come faceva la
/// prima stesura -- fallisce sempre la somma dei **compressi**, e il ramo
/// dei decompressi resta scoperto mentre la sonda e' verde. Ogni caso
/// carica quindi l'overflow su un accumulatore e tiene innocuo l'altro, e
/// l'asserzione nomina il messaggio specifico invece del prefisso comune:
/// «overflow nel conteggio dei byte» non distingue i due.
#[test]
fn n1_le_dimensioni_dichiarate_in_zip64_non_sommano_in_silenzio() {
    let dir = tempfile::tempdir().unwrap();
    let opzioni = opzioni_lettura();

    // Il controllo che rende interpretabile il resto: lo stesso archivio con
    // dimensioni piccole **passa**. Senza, un rifiuto non distinguerebbe
    // «la somma trabocca» da «il central directory costruito dalla sonda non
    // e' leggibile», e la sonda proverebbe l'incapacita' di chi l'ha scritta
    // invece del comportamento del driver.
    let innocuo = dir.path().join("innocuo.xlsx");
    std::fs::write(&innocuo, zip64_con_dimensioni_dichiarate([(2, 2), (3, 3)])).unwrap();
    validate_archive_ratio(&innocuo, opzioni.budget()).expect(
            "il central directory della sonda deve essere accettato da `ZipArchive::new`              con dimensioni piccole: se fallisce qui, a essere sbagliato e' l'archivio              costruito dalla sonda e non il driver",
        );

    // Un accumulatore per volta: `(compressa, decompressa)` per ciascuna
    // delle due voci, e il messaggio che deve arrivare.
    let casi: [CasoDiOverflow; 2] = [
        (
            "compressi",
            [(u64::MAX, 2), (1, 3)],
            "overflow nel conteggio dei byte compressi",
        ),
        (
            "decompressi",
            [(2, u64::MAX), (3, 1)],
            "overflow nel conteggio dei byte decompressi",
        ),
    ];

    for (nome, voci, atteso) in casi {
        let percorso = dir.path().join(format!("{nome}.xlsx"));
        std::fs::write(&percorso, zip64_con_dimensioni_dichiarate(voci)).unwrap();
        let Err(errore) = validate_archive_ratio(&percorso, opzioni.budget()) else {
            panic!(
                    "«{nome}»: la somma di u64::MAX e 1 deve traboccare; se passa, il crate                      `zip` sta normalizzando le dimensioni dichiarate e il ramo va                      riclassificato"
                );
        };
        assert_eq!(
            errore.category,
            plenora_io_model::ErrorCategory::ResourceLimit,
            "«{nome}»: un overflow di conteggio e' un limite, non un formato non valido"
        );
        assert!(
                errore.message.contains(atteso),
                "«{nome}»: atteso «{atteso}», arrivato «{}». Un messaggio generico non                  distinguerebbe i due accumulatori, ed e' il difetto che questo caso esiste                  per escludere",
                errore.message
            );
    }
}

/// Una parte enumerabile ma **non apribile** ferma la lettura.
///
/// # Il difetto che questa sonda ha trovato
///
/// La selezione del perimetro passava da `archive.by_index(indice).ok()?`
/// dentro un `filter_map`: qualunque errore di apertura diventava «parte
/// ignorata». Una parte con central directory leggibile e header locale
/// irraggiungibile spariva quindi dal perimetro, e
/// `valida_riferimenti_cella` restituiva `Ok` -- cioe' **fail-open**, in una
/// funzione il cui doc-comment promette che «non c'e' un ramo che, non
/// riuscendo a controllare, prosegua lo stesso».
///
/// Non era una svista di stile: `?` dentro `filter_map` scarta l'errore per
/// costruzione, e il `by_name` successivo non lo ripara, perche' cerca fra i
/// nomi **gia' filtrati**.
/// Un archivio con due parti XML vere, e l'offset dell'header locale di una
/// di esse **corrotto nel central directory**.
///
/// Costruito con il crate `zip` e poi ritoccato in un campo solo: cosi' il
/// contenuto e' valido, il central directory resta leggibile, e l'unica
/// cosa che cambia e' che quella voce non si apre. Costruirlo a mano
/// avrebbe messo in gioco anche la correttezza della fixture, e un rifiuto
/// non avrebbe piu' distinto le due cause.
fn xlsx_con_una_parte_non_apribile(corrompi: bool) -> Vec<u8> {
    let mut buffer = std::io::Cursor::new(Vec::new());
    {
        let mut scrittore = zip::ZipWriter::new(&mut buffer);
        let opzioni: zip::write::FileOptions<'_, ()> =
            zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Stored);
        for nome in ["xl/worksheets/sheet1.xml", "xl/workbook.xml"] {
            scrittore.start_file(nome, opzioni).unwrap();
            std::io::Write::write_all(&mut scrittore, b"<x r=\"A1\"/>").unwrap();
        }
        scrittore.finish().unwrap();
    }
    let mut byte = buffer.into_inner();
    if !corrompi {
        return byte;
    }

    // La prima voce del central directory: firma `PK\x01\x02`, e il
    // relative offset dell'header locale nei quattro byte a 42.
    let posizione = byte
        .windows(4)
        .position(|finestra| finestra == [0x50, 0x4b, 0x01, 0x02])
        .expect("il central directory esiste");
    byte[posizione + 42..posizione + 46].copy_from_slice(&3_u32.to_le_bytes());
    byte
}

#[test]
fn n1_una_parte_enumerabile_ma_non_apribile_ferma_la_lettura() {
    let dir = tempfile::tempdir().unwrap();
    let opzioni = opzioni_lettura();

    // Il controllo che rende interpretabile il rifiuto: lo stesso archivio
    // **non** corrotto passa.
    let integro = dir.path().join("integro.xlsx");
    std::fs::write(&integro, xlsx_con_una_parte_non_apribile(false)).unwrap();
    valida_riferimenti_cella(&integro, opzioni.budget())
        .expect("con l'offset corretto entrambe le parti si aprono e si ispezionano");

    // La voce ritoccata e' `xl/worksheets/sheet1.xml`, cioe' dentro il
    // perimetro: e' quella che non deve poter sparire in silenzio.
    let rotto = dir.path().join("rotto.xlsx");
    std::fs::write(&rotto, xlsx_con_una_parte_non_apribile(true)).unwrap();

    let Err(errore) = valida_riferimenti_cella(&rotto, opzioni.budget()) else {
        panic!(
            "una parte del perimetro che non si apre deve fermare la lettura; \
                 restituire Ok e' il fail-open che il contratto esclude"
        );
    };
    assert!(
        errore.message.contains("parte XLSX non leggibile"),
        "atteso il rifiuto sull'apertura della parte, arrivato «{}»",
        errore.message
    );
}

/// Un archivio con `quante` voci vuote e nomi **realmente distinti**.
///
/// I nomi devono essere unici: `zip` 8.6 conserva le voci in una mappa
/// indicizzata per nome, e due voci omonime si sovrascrivono invece di
/// contarsi due volte. Una fixture con nomi ripetuti proverebbe un tetto
/// piu' basso di quello dichiarato.
fn xlsx_con_parti(quante: usize) -> Vec<u8> {
    let mut buffer = std::io::Cursor::new(Vec::new());
    {
        let mut scrittore = zip::ZipWriter::new(&mut buffer);
        let opzioni: zip::write::FileOptions<'_, ()> =
            zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Stored);
        for indice in 0..quante {
            scrittore
                .start_file(format!("parte{indice}.bin"), opzioni)
                .unwrap();
        }
        scrittore.finish().unwrap();
    }
    buffer.into_inner()
}

/// Il tetto sulle parti conta **i membri dell'archivio**, non le sole parti
/// XML, e i due confini stanno uno accanto all'altro.
///
/// Il **vecchio** nome della costante diceva «parti XML» mentre il controllo
/// guarda `archive.len()`, cioe' ogni membro: un contenitore con migliaia di
/// immagini viene fermato quanto uno con migliaia di fogli. E' il
/// comportamento voluto -- il tetto difende dall'abuso del contenitore, non
/// dal numero di fogli -- e la sonda lo fissa perche' un nome non e' una
/// prova: `MAX_MEMBRI_ARCHIVIO` oggi lo dice, ma potrebbe tornare a mentire.
#[test]
fn n1_il_tetto_sulle_parti_conta_i_membri_e_ha_i_due_confini() {
    let dir = tempfile::tempdir().unwrap();
    let opzioni = opzioni_lettura();

    // Nessuna delle voci e' una parte XML del perimetro: se il tetto
    // contasse le sole parti XML, questo archivio passerebbe.
    let al_limite = dir.path().join("al-limite.xlsx");
    std::fs::write(&al_limite, xlsx_con_parti(MAX_MEMBRI_ARCHIVIO)).unwrap();
    valida_riferimenti_cella(&al_limite, opzioni.budget())
        .expect("il confine e' inclusivo: esattamente MAX_MEMBRI_ARCHIVIO membri passano");

    let oltre = dir.path().join("oltre.xlsx");
    std::fs::write(&oltre, xlsx_con_parti(MAX_MEMBRI_ARCHIVIO + 1)).unwrap();
    let Err(errore) = valida_riferimenti_cella(&oltre, opzioni.budget()) else {
        panic!("un membro oltre il tetto deve fermare la lettura");
    };
    assert_eq!(
        errore.category,
        plenora_io_model::ErrorCategory::ResourceLimit,
        "un tetto superato e' un limite, non un formato non valido"
    );
}

/// Un `.xlsx` con `righe` x `colonne` celle, piu' l'intestazione.
///
/// Passa dal writer vero invece di costruire byte a mano: qui non si prova
/// la tolleranza al formato malformato -- quello e' il mestiere delle
/// fixture di `valida_riferimenti_cella` -- ma il comportamento su un foglio
/// **valido** e semplicemente troppo grande per la quota.
fn scrivi_xlsx(percorso: &std::path::Path, righe: u32, colonne: u16) {
    let mut cartella = Workbook::new();
    let foglio = cartella.add_worksheet();
    if colonne > 0 {
        foglio.write_string(0, 0, "geometry").unwrap();
        for colonna in 1..colonne {
            foglio
                .write_string(0, colonna, format!("c{colonna}"))
                .unwrap();
        }
        for riga in 1..=righe {
            foglio.write_string(riga, 0, "POINT (1 2)").unwrap();
            for colonna in 1..colonne {
                foglio.write_string(riga, colonna, "v").unwrap();
            }
        }
    }
    cartella.save(percorso).unwrap();
}

/// Opzioni di lettura complete per un foglio con una colonna WKT.
///
/// Il driver esige sia `wkt_column` sia `assume_crs`: senza, il rifiuto
/// arriverebbe da `resolve_geometry` o dalla fase CRS, e le sonde qui sotto
/// misurerebbero una guardia diversa da quella che dichiarano.
fn opzioni_lettura_wkt() -> ReadOptions {
    opzioni_lettura_wkt_con(plenora_io_model::budget::PipelineLimits::default())
}

fn opzioni_lettura_wkt_con(limits: plenora_io_model::budget::PipelineLimits) -> ReadOptions {
    opzioni_lettura_con(limits)
        .with_assume_crs("EPSG:4326")
        .with_format_option("wkt_column", "geometry")
}

/// I rifiuti che `infer_layout` decide **da se'** e puo' pronunciare.
///
/// Il censimento del gruppo conta venticinque righe, ma solo tre sono
/// decisioni della funzione: la larghezza oltre la quota, l'altezza oltre la
/// quota, e il foglio che non produce nemmeno una cella. Tutto il resto e'
/// propagazione da un helper -- che ha il proprio gruppo, e va provato li',
/// altrimenti la stessa riga risulterebbe coperta due volte senza che
/// nessuna delle due prove dica dove -- oppure e' irraggiungibile, e le
/// quattro irraggiungibilita' hanno la loro sonda subito sotto.
///
/// I due limiti si provano dal `open` del driver e non chiamando
/// `infer_layout` a mano: la quota arriva da `ReadOptions`, e passare per
/// l'entry point verifica anche che ci arrivi davvero.
#[test]
fn n1_infer_layout_rifiuta_le_due_quote() {
    let dir = tempfile::tempdir().unwrap();

    // Il controllo positivo: lo stesso foglio con quote sufficienti passa.
    // Senza, un `open` che fallisse per qualunque altra ragione farebbe
    // verde la tabella dei rifiuti.
    let normale = dir.path().join("normale.xlsx");
    scrivi_xlsx(&normale, 3, 3);
    XlsDriver
        .open(Source::Path(normale), opzioni_lettura_wkt())
        .expect("tre righe per tre colonne stanno in qualunque quota di default");

    // Larghezza oltre la quota: `--max-columns` a 2 su un foglio da 3.
    let largo = dir.path().join("largo.xlsx");
    scrivi_xlsx(&largo, 2, 3);
    let Err(errore) = XlsDriver.open(
        Source::Path(largo),
        opzioni_lettura_wkt_con(
            plenora_io_model::budget::PipelineLimits::default().with_max_columns(2),
        ),
    ) else {
        panic!("tre colonne oltre una quota di due devono essere rifiutate");
    };
    assert!(
        errore.message.contains("colonne oltre il limite"),
        "atteso il rifiuto sulla larghezza, arrivato «{}»",
        errore.message
    );

    // Altezza oltre la quota: `--max-rows` a 1 su un foglio da 3 righe dati.
    let alto = dir.path().join("alto.xlsx");
    scrivi_xlsx(&alto, 3, 2);
    let Err(errore) = XlsDriver.open(
        Source::Path(alto),
        opzioni_lettura_wkt_con(
            plenora_io_model::budget::PipelineLimits::default().with_max_rows(1),
        ),
    ) else {
        panic!("tre righe oltre una quota di una devono essere rifiutate");
    };
    assert!(
        errore.message.contains("righe oltre il limite"),
        "atteso il rifiuto sull'altezza, arrivato «{}»",
        errore.message
    );
}

/// Il foglio senza celle: la guardia esiste ed e' corretta, ma `open` non
/// la puo' raggiungere.
///
/// # Perche' non e' un rifiuto raggiungibile dall'esterno
///
/// `observed_cells` conta **celle**, non righe: `for_each_dense_row`
/// attraversa comunque l'intervallo dichiarato dalle dimensioni, quindi la
/// riga d'intestazione viene sempre visitata, e su quella riga
/// `resolve_geometry` deve riuscire prima che il flusso arrivi al
/// controllo. Se il flusso di celle e' vuoto, ogni intestazione e' la
/// stringa vuota -- `data_to_string(Data::Empty)` la produce -- quindi
/// l'unico nome di colonna che vi si troverebbe e' `""`. E `""` non
/// arriva: `wkt_column` e' `ValoreAmmesso::Testo`, che esige testo non
/// vuoto, e il validatore centrale delle `format_options` lo ferma prima
/// che il driver apra il file.
///
/// Le due meta' si provano separatamente perche' affermano cose diverse:
/// la prima che il controllo funziona, la seconda che la strada per
/// arrivarci e' chiusa altrove. Provare solo la seconda lascerebbe il ramo
/// non eseguito; provare solo la prima direbbe che e' raggiungibile.
#[test]
fn n1_il_foglio_senza_celle_e_fermato_dallo_schema_prima_di_infer_layout() {
    let dir = tempfile::tempdir().unwrap();
    let vuoto = dir.path().join("vuoto.xlsx");
    scrivi_xlsx(&vuoto, 0, 0);

    // Meta' uno: chiamata diretta, con l'unica configurazione che porta
    // fino al controllo. Il tipo e' privato del crate, quindi la prova puo'
    // costruire cio' che l'entry point rifiuta.
    let opzioni = opzioni_lettura();
    let mut cartella: calamine::Xlsx<_> = calamine::open_workbook(&vuoto).unwrap();
    let foglio = cartella.sheet_names().first().cloned().unwrap();
    let mut nomi_colonne = BTreeMap::new();
    nomi_colonne.insert("wkt_column".to_owned(), String::new());
    let esito = infer_layout(
        &mut cartella,
        &foglio,
        &nomi_colonne,
        "EPSG:4326",
        opzioni.cancellation(),
        XlsxQuote::from_read_options(&opzioni),
        opzioni.budget(),
    );
    let Err(errore) = esito else {
        panic!("un foglio che non consegna nemmeno una cella non ha un layout da inferire");
    };
    assert!(
        errore.message.contains("foglio vuoto"),
        "atteso il rifiuto sul foglio vuoto, arrivato «{}»",
        errore.message
    );

    // Meta' due: la stessa configurazione passata da `open` non arriva al
    // driver. Il rifiuto e' dello schema delle opzioni, in fase di
    // validazione, non dell'inferenza.
    let Err(fermato) = XlsDriver.open(
        Source::Path(vuoto),
        opzioni_lettura()
            .with_assume_crs("EPSG:4326")
            .with_format_option("wkt_column", ""),
    ) else {
        panic!("un nome di colonna vuoto non e' un valore ammesso");
    };
    assert_eq!(
        fermato.phase,
        ErrorPhase::Validate,
        "il rifiuto deve venire dalla validazione delle opzioni, non dalla lettura: {}",
        fermato.message
    );
    assert!(
        fermato.message.contains("testo non vuoto"),
        "atteso il rifiuto dello schema sul valore vuoto, arrivato «{}»",
        fermato.message
    );
}

/// «intestazione XLSX assente» e «geometria XLSX non configurata» sono
/// irraggiungibili: `data_row_count` rifiuta prima le dimensioni che
/// renderebbero vuoto il ciclo.
///
/// Le tre occorrenze -- il `geom.ok_or_else` dentro il ciclo, e i due
/// `ok_or_else` su `headers` e `geom` dopo -- esistono perche' il tipo e'
/// `Option`, non perche' un input le produca. `for_each_dense_row` itera su
/// `bounds.start.0..=bounds.end.0`, e la prima riga visitata assegna
/// entrambe le variabili oppure propaga l'errore di `resolve_geometry`.
/// Perche' quell'intervallo sia vuoto servirebbe `start.0 > end.0`, e
/// `data_row_count` lo rifiuta con «dimensioni XLSX non valide».
///
/// La sonda non copre quelle righe: esegue la **guardia** che le rende
/// inarrivabili. Se la precedenza cambia -- se le dimensioni invertite
/// smettessero di essere rifiutate -- diventa rossa, e la
/// classificazione va rifatta.
#[test]
fn n1_le_dimensioni_invertite_precedono_l_intestazione_assente() {
    for invertite in [
        SheetBounds {
            start: (5, 0),
            end: (4, 3),
        },
        SheetBounds {
            start: (0, 5),
            end: (3, 4),
        },
    ] {
        let esito = data_row_count(invertite).and_then(|_| data_row_width(invertite));
        let Err(errore) = esito else {
            panic!(
                "dimensioni con inizio oltre la fine devono essere rifiutate: start {:?}, end {:?}",
                invertite.start, invertite.end
            );
        };
        assert!(
            errore.message.contains("dimensioni XLSX non valide"),
            "atteso il rifiuto sulle dimensioni, arrivato «{}»",
            errore.message
        );
    }

    // Il complemento, senza il quale la sonda direbbe solo che qualcosa
    // viene rifiutato: con dimensioni accettate l'intervallo delle righe
    // contiene almeno la riga d'intestazione, che e' l'affermazione da cui
    // dipende l'irraggiungibilita'.
    let accettate = SheetBounds {
        start: (7, 2),
        end: (7, 2),
    };
    assert!(
        data_row_count(accettate).is_ok() && accettate.start.0 <= accettate.end.0,
        "un foglio di una sola cella e' comunque un foglio con una riga da visitare"
    );
}

/// «troppe colonne XLSX» e «indice colonna XLSX fuori intervallo» sono
/// irraggiungibili: la larghezza che `data_row_width` accetta non lascia
/// spazio ne' al `try_from` ne' all'overflow.
///
/// Le quattro occorrenze -- due nel ciclo delle righe, due nel ciclo che
/// costruisce lo schema dalle intestazioni -- indicizzano una riga lunga
/// `data_row_width(bounds)`. Quella funzione calcola `end.1 - start.1 + 1`
/// in `u32`, quindi rifiuta gia' la riga larga quanto l'intero spazio delle
/// colonne: la larghezza massima accettata e' `u32::MAX`, l'offset massimo
/// `u32::MAX - 1`, e `start.1 + offset` non supera mai `end.1`.
///
/// Gli estremi del formato sono verificati invece che argomentati: sono il
/// caso peggiore rappresentabile, e se passano passa ogni foglio.
#[test]
fn n1_la_larghezza_accettata_precede_i_due_rifiuti_sull_indice_di_colonna() {
    assert!(
        data_row_width(SheetBounds {
            start: (0, 0),
            end: (0, u32::MAX),
        })
        .is_err(),
        "la riga larga quanto l'intero spazio delle colonne non e' rappresentabile"
    );

    for estreme in [
        SheetBounds {
            start: (0, 0),
            end: (0, u32::MAX - 1),
        },
        SheetBounds {
            start: (0, u32::MAX - 3),
            end: (0, u32::MAX),
        },
    ] {
        let larghezza = data_row_width(estreme).expect("le dimensioni sono valide");
        let offset_massimo = larghezza - 1;
        let Ok(offset) = u32::try_from(offset_massimo) else {
            panic!("l'offset massimo di una riga larga {larghezza} deve stare in u32");
        };
        assert!(
            estreme.start.1.checked_add(offset).is_some(),
            "start.1 + offset non puo' traboccare: e' al piu' end.1, che e' un u32"
        );
    }
}

/// Ogni rifiuto di `resolve_geometry` nomina **quale** colonna manca, e la
/// colonna WKT ha la precedenza sulla coppia x/y.
///
/// I quattro rifiuti hanno tre messaggi diversi perche' mandano chi legge a
/// correggere cose diverse: un `wkt_column` che non trova l'intestazione,
/// un'ascissa che non la trova, un'ordinata che non la trova, e la
/// configurazione che non dice niente. Il quarto messaggio copre tre
/// precondizioni distinte -- nessuna opzione, la sola ascissa, la sola
/// ordinata -- e la tabella le tiene separate: una coppia a meta' non e'
/// una configurazione assente, e se un giorno meritasse un messaggio
/// proprio la riga che lo dice e' gia' scritta.
///
/// Le colonne restituite sono **assolute**, non offset: `start_column` qui
/// e' cinque, e ogni attesa lo somma. Con uno zero al suo posto la tabella
/// resterebbe verde anche se la somma sparisse.
#[test]
fn n1_resolve_geometry_nomina_la_colonna_che_manca_e_da_precedenza_al_wkt() {
    const INIZIO: u32 = 5;
    let intestazioni: Vec<String> = ["id", "geom", "lon", "lat"]
        .iter()
        .map(|nome| (*nome).to_owned())
        .collect();
    let opzioni = |coppie: &[(&str, &str)]| -> BTreeMap<String, String> {
        coppie
            .iter()
            .map(|(chiave, valore)| ((*chiave).to_owned(), (*valore).to_owned()))
            .collect()
    };

    for (caso, coppie, atteso) in [
        (
            "wkt_column che non e' fra le intestazioni",
            vec![("wkt_column", "assente")],
            "colonna WKT assente dall'intestazione",
        ),
        (
            "ascissa assente, ordinata presente",
            vec![("x_column", "assente"), ("y_column", "lat")],
            "colonna X assente dall'intestazione",
        ),
        (
            "ascissa presente, ordinata assente",
            vec![("x_column", "lon"), ("y_column", "assente")],
            "colonna Y assente dall'intestazione",
        ),
        (
            "nessuna opzione di geometria",
            vec![],
            "specificare wkt_column, oppure x_column con y_column, in format_options",
        ),
        (
            "coppia a meta': solo l'ascissa",
            vec![("x_column", "lon")],
            "specificare wkt_column, oppure x_column con y_column, in format_options",
        ),
        (
            "coppia a meta': solo l'ordinata",
            vec![("y_column", "lat")],
            "specificare wkt_column, oppure x_column con y_column, in format_options",
        ),
    ] {
        let esito = resolve_geometry(&intestazioni, INIZIO, &opzioni(&coppie));
        let Err(errore) = esito else {
            panic!("{caso}: doveva essere rifiutata");
        };
        assert_eq!(
            errore.message, atteso,
            "{caso}: il messaggio manda a correggere la cosa sbagliata"
        );
    }

    // Le accettazioni: senza, una funzione che rifiutasse tutto passerebbe
    // la tabella dei negativi.
    let (wkt, colonne_wkt) =
        resolve_geometry(&intestazioni, INIZIO, &opzioni(&[("wkt_column", "geom")]))
            .expect("«geom» e' la seconda intestazione");
    let XlsxGeomSpec::Wkt(colonna) = wkt else {
        panic!("wkt_column deve produrre una geometria WKT");
    };
    assert_eq!(colonna, INIZIO + 1, "la colonna e' assoluta, non un offset");
    assert_eq!(colonne_wkt, BTreeSet::from([INIZIO + 1]));

    let (xy, colonne_xy) = resolve_geometry(
        &intestazioni,
        INIZIO,
        &opzioni(&[("x_column", "lon"), ("y_column", "lat")]),
    )
    .expect("«lon» e «lat» sono la terza e la quarta intestazione");
    let XlsxGeomSpec::Xy(x, y) = xy else {
        panic!("x_column con y_column deve produrre una geometria x/y");
    };
    assert_eq!((x, y), (INIZIO + 2, INIZIO + 3));
    assert_eq!(colonne_xy, BTreeSet::from([INIZIO + 2, INIZIO + 3]));

    // La precedenza: con tutte e tre le opzioni valide vince il WKT, e la
    // coppia x/y non contribuisce alle colonne consumate. Senza questa
    // riga, invertire i due `if` non romperebbe nulla.
    let (insieme, colonne_insieme) = resolve_geometry(
        &intestazioni,
        INIZIO,
        &opzioni(&[
            ("wkt_column", "geom"),
            ("x_column", "lon"),
            ("y_column", "lat"),
        ]),
    )
    .expect("le tre colonne esistono tutte");
    assert!(
        matches!(insieme, XlsxGeomSpec::Wkt(colonna) if colonna == INIZIO + 1),
        "wkt_column ha la precedenza sulla coppia x/y"
    );
    assert_eq!(colonne_insieme, BTreeSet::from([INIZIO + 1]));

    // Ascissa e ordinata sulla stessa colonna sono accettate, e l'insieme
    // ne contiene una sola. Non e' una svista: `infer_layout` dimensiona
    // gli accumulatori su `width - resolved_columns.len()` e salta le
    // colonne dell'insieme, quindi le due quantita' restano d'accordo. Se
    // un giorno l'insieme diventasse una lista con ripetizioni, questa
    // riga diventa rossa prima che gli accumulatori vadano fuori indice.
    let (_, colonne_doppie) = resolve_geometry(
        &intestazioni,
        INIZIO,
        &opzioni(&[("x_column", "lon"), ("y_column", "lon")]),
    )
    .expect("nulla vieta di leggere due volte la stessa colonna");
    assert_eq!(colonne_doppie.len(), 1, "l'insieme non ripete la colonna");
}

/// Il rifiuto di `resolve_geometry` arriva fino a chi chiama `open`.
///
/// La tabella qui sopra chiama la funzione direttamente; questa sonda
/// verifica che quel messaggio non venga riscritto o inghiottito lungo la
/// strada, che e' l'unica cosa che la chiamata diretta non puo' dire.
#[test]
fn n1_la_colonna_wkt_assente_esce_dal_driver_con_il_suo_messaggio() {
    let dir = tempfile::tempdir().unwrap();
    let percorso = dir.path().join("altra-colonna.xlsx");
    scrivi_xlsx(&percorso, 2, 2);

    let Err(errore) = XlsDriver.open(
        Source::Path(percorso),
        opzioni_lettura()
            .with_assume_crs("EPSG:4326")
            .with_format_option("wkt_column", "questa-non-c-e"),
    ) else {
        panic!("una colonna WKT che non esiste non e' una geometria");
    };
    assert_eq!(
        errore.message, "colonna WKT assente dall'intestazione",
        "il messaggio del driver deve restare quello della funzione"
    );
}

/// Un `.xlsx` vero in cui **solo** `xl/worksheets/sheet1.xml` e' sostituito.
///
/// Le dimensioni dichiarate e le celle presenti sono due cose diverse, e
/// nessuna libreria conforme le fa divergere: per costruire quella
/// divergenza il foglio va scritto a mano. Tutto il resto del contenitore
/// -- tipi di contenuto, relazioni, workbook -- resta quello prodotto da
/// `rust_xlsxwriter`, cosi' un rifiuto non puo' venire da una parte
/// malformata che non c'entra.
fn xlsx_con_foglio(dir: &std::path::Path, nome: &str, foglio_xml: &str) -> std::path::PathBuf {
    let base = dir.join("base.xlsx");
    let mut cartella = Workbook::new();
    let primo = cartella.add_worksheet();
    primo.write_string(0, 0, "segnaposto").unwrap();
    cartella.save(&base).unwrap();

    let mut archivio = zip::ZipArchive::new(std::fs::File::open(&base).unwrap()).unwrap();
    let mut buffer = std::io::Cursor::new(Vec::new());
    let mut sostituite = 0usize;
    {
        let mut scrittore = zip::ZipWriter::new(&mut buffer);
        let opzioni: zip::write::FileOptions<'_, ()> =
            zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Stored);
        for indice in 0..archivio.len() {
            let mut membro = archivio.by_index(indice).unwrap();
            let parte = membro.name().to_owned();
            scrittore.start_file(parte.clone(), opzioni).unwrap();
            if parte == "xl/worksheets/sheet1.xml" {
                sostituite += 1;
                std::io::Write::write_all(&mut scrittore, foglio_xml.as_bytes()).unwrap();
            } else {
                std::io::copy(&mut membro, &mut scrittore).unwrap();
            }
        }
        scrittore.finish().unwrap();
    }
    assert_eq!(
        sostituite, 1,
        "il foglio da sostituire deve esistere una volta sola: se il writer \
             rinomina la parte, la fixture starebbe provando un altro file"
    );

    let percorso = dir.join(nome);
    std::fs::write(&percorso, buffer.into_inner()).unwrap();
    percorso
}

/// Il corpo di un foglio con le dimensioni dichiarate e le righe date.
fn foglio_xml(riferimento: &str, righe: &str) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
             <worksheet xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\">\
             <dimension ref=\"{riferimento}\"/><sheetData>{righe}</sheetData></worksheet>"
    )
}

/// Una cella testuale in linea, senza tabella delle stringhe condivise.
fn cella(riferimento: &str, testo: &str) -> String {
    format!("<c r=\"{riferimento}\" t=\"inlineStr\"><is><t>{testo}</t></is></c>")
}

/// `for_each_dense_row` sul foglio dato, con un visitatore che non decide
/// nulla: cosi' l'esito viene dalla funzione, non dal chiamante.
fn celle_osservate(percorso: &std::path::Path) -> Result<usize> {
    let opzioni = opzioni_lettura();
    let mut cartella: calamine::Xlsx<_> = calamine::open_workbook(percorso).unwrap();
    let nome = cartella.sheet_names().first().cloned().unwrap();
    let mut lettore = LettoreCelleSorvegliato::nuovo(&mut cartella, &nome)?;
    let dimensioni = lettore.dimensioni()?;
    for_each_dense_row(
        &mut lettore,
        dimensioni,
        opzioni.cancellation(),
        |_riga, _valori| Ok(true),
    )
}

/// I due rifiuti di `for_each_dense_row`, e il conteggio che restituisce.
///
/// La funzione trasforma un flusso di celle sparse in righe dense, e per
/// farlo si fida di due cose che il file dichiara e che nessuna libreria
/// conforme fa divergere: che le celle arrivino in ordine di riga
/// crescente, e che stiano dentro le dimensioni dichiarate. Un foglio
/// costruito a mano fa divergere entrambe, e allora la funzione deve
/// fermarsi invece di scrivere fuori dal vettore della riga -- che e' cio'
/// che `values[offset]` farebbe se l'offset non fosse gia' stato
/// confrontato con le dimensioni.
///
/// Il controllo positivo non e' un contorno: fissa anche che il valore
/// restituito conti **celle** e non righe, cioe' l'unita' su cui
/// `infer_layout` decide se il foglio sia vuoto.
#[test]
fn n1_for_each_dense_row_rifiuta_le_celle_fuori_ordine_e_fuori_dimensione() {
    let dir = tempfile::tempdir().unwrap();

    // Il controllo positivo, e il conteggio: due righe per due colonne, ma
    // la seconda riga ha una sola cella. Le celle sono tre, le righe due.
    let conforme = xlsx_con_foglio(
        dir.path(),
        "conforme.xlsx",
        &foglio_xml(
            "A1:B2",
            &format!(
                "<row r=\"1\">{}{}</row><row r=\"2\">{}</row>",
                cella("A1", "geometry"),
                cella("B1", "nome"),
                cella("A2", "POINT (1 2)")
            ),
        ),
    );
    assert_eq!(
        celle_osservate(&conforme).expect("il foglio e' conforme"),
        3,
        "il conteggio e' di celle, non di righe: la riga sparsa ne porta una sola"
    );

    // Celle oltre l'ultima colonna dichiarata.
    let oltre_la_fine = xlsx_con_foglio(
        dir.path(),
        "oltre-la-fine.xlsx",
        &foglio_xml(
            "A1:B2",
            &format!(
                "<row r=\"1\">{}{}{}</row>",
                cella("A1", "geometry"),
                cella("B1", "nome"),
                cella("C1", "di troppo")
            ),
        ),
    );
    let Err(errore) = celle_osservate(&oltre_la_fine) else {
        panic!("una cella oltre l'ultima colonna dichiarata non ha un posto nella riga");
    };
    assert_eq!(
        errore.message,
        "cella XLSX fuori dalle dimensioni dichiarate"
    );

    // Celle prima della prima colonna dichiarata: l'altra meta' della
    // stessa condizione, che una sola fixture non distinguerebbe.
    let prima_dell_inizio = xlsx_con_foglio(
        dir.path(),
        "prima-dell-inizio.xlsx",
        &foglio_xml(
            "B1:C1",
            &format!(
                "<row r=\"1\">{}{}</row>",
                cella("A1", "di troppo"),
                cella("B1", "geometry")
            ),
        ),
    );
    let Err(errore) = celle_osservate(&prima_dell_inizio) else {
        panic!("una cella prima della prima colonna dichiarata non ha un posto nella riga");
    };
    assert_eq!(
        errore.message,
        "cella XLSX fuori dalle dimensioni dichiarate"
    );

    // Righe fuori ordine: la terza prima della seconda. La funzione tiene
    // una sola cella in attesa, quindi una riga che torna indietro non e'
    // recuperabile: e' il caso che il rifiuto esiste per prendere.
    let fuori_ordine = xlsx_con_foglio(
        dir.path(),
        "fuori-ordine.xlsx",
        &foglio_xml(
            "A1:A3",
            &format!(
                "<row r=\"1\">{}</row><row r=\"3\">{}</row><row r=\"2\">{}</row>",
                cella("A1", "geometry"),
                cella("A3", "terza"),
                cella("A2", "seconda")
            ),
        ),
    );
    let Err(errore) = celle_osservate(&fuori_ordine) else {
        panic!("una riga che torna indietro deve fermare la lettura");
    };
    assert_eq!(errore.message, "ordine delle celle XLSX non monotono");
}

/// «indice colonna XLSX non rappresentabile» e' irraggiungibile: la
/// piattaforma decide, e su tutte quelle supportate `usize` copre `u32`.
///
/// L'offset e' `cell_column - bounds.start.1`, cioe' un `u32`, e la
/// conversione a `usize` fallisce solo dove `usize` e' piu' stretto di
/// trentadue bit. Non c'e' una guardia a monte da eseguire: la guardia e'
/// il bersaglio di compilazione, e per questo la verifica sta in un blocco
/// `const`. Su una piattaforma dove `usize` non copre `u32` il crate non
/// compila -- che e' esattamente il momento in cui quel ramo andrebbe
/// riclassificato da irraggiungibile a coperto, e un errore di
/// compilazione lo dice a chiunque, non solo a chi esegue i test.
#[test]
fn n1_usize_copre_u32_su_ogni_piattaforma_supportata() {
    // In un blocco `const` perche' la proprieta' e' del bersaglio, non
    // dell'esecuzione: cosi' una piattaforma dove `usize` e' piu' stretto
    // di `u32` non fa passare la compilazione, invece di far fallire un
    // test che qualcuno potrebbe non eseguire.
    const {
        assert!(
            usize::BITS >= u32::BITS,
            "usize piu' stretto di u32: il ramo diventa raggiungibile"
        );
    }
    assert!(
        usize::try_from(u32::MAX).is_ok(),
        "la conversione che il ramo sorveglia non fallisce su questo bersaglio"
    );
}

/// Legge una geometria dallo spool, restituendo l'esito e cio' che il
/// costruttore ha accumulato.
fn geometria_dallo_spool(byte: &[u8]) -> Result<Vec<Option<Vec<u8>>>> {
    let mut costruttore = BinaryBuilder::new();
    let mut buffer = Vec::new();
    read_spool_geometry(
        &mut std::io::Cursor::new(byte),
        &mut costruttore,
        &mut buffer,
    )?;
    let colonna = costruttore.finish();
    Ok((0..colonna.len())
        .map(|riga| (!colonna.is_null(riga)).then(|| colonna.value(riga).to_vec()))
        .collect())
}

/// `read_spool_geometry` distingue il marcatore di nullo dal troncamento.
///
/// Lo spool e' un formato **nostro**, scritto e riletto nella stessa
/// operazione, e il primo istinto sarebbe fidarsene. Non si puo': sta su
/// disco, e un file temporaneo troncato -- disco pieno, processo ucciso fra
/// la scrittura e la rilettura -- e' l'unico modo in cui la geometria di una
/// riga puo' arrivare a meta'. Il rifiuto separa quel caso dal marcatore di
/// geometria assente, che e' una lunghezza legittima e non un errore.
#[test]
fn n1_read_spool_geometry_separa_il_nullo_dal_troncamento() {
    // Il marcatore di nullo: `u32::MAX` non e' una lunghezza, e' l'assenza.
    assert_eq!(
        geometria_dallo_spool(&SPOOL_NULL_GEOMETRY.to_le_bytes())
            .expect("il marcatore di nullo e' un valore, non un errore"),
        vec![None],
        "il marcatore deve produrre una geometria assente, non una vuota"
    );

    // Una geometria vera: la lunghezza, poi esattamente quei byte.
    let mut intera = 3_u32.to_le_bytes().to_vec();
    intera.extend_from_slice(b"abc");
    assert_eq!(
        geometria_dallo_spool(&intera).expect("lunghezza e payload sono d'accordo"),
        vec![Some(b"abc".to_vec())],
        "una geometria intera deve arrivare identica"
    );

    // Una lunghezza zero e' una geometria vuota, non un'assenza: le due
    // cose escono dallo stesso campo e solo il valore le distingue.
    assert_eq!(
        geometria_dallo_spool(&0_u32.to_le_bytes())
            .expect("zero byte di geometria e' una lunghezza legittima"),
        vec![Some(Vec::new())],
        "lunghezza zero non e' il marcatore di assenza"
    );

    for (caso, byte) in [
        ("nemmeno la lunghezza", Vec::new()),
        ("lunghezza a meta'", vec![1, 0, 0]),
        ("payload piu' corto della lunghezza dichiarata", {
            let mut byte = 4_u32.to_le_bytes().to_vec();
            byte.extend_from_slice(b"ab");
            byte
        }),
    ] {
        let Err(errore) = geometria_dallo_spool(&byte) else {
            panic!("{caso}: uno spool incompleto non e' una geometria");
        };
        assert_eq!(
            errore.message, "spool XLSX troncato o illeggibile",
            "{caso}: il messaggio deve dire che il file e' incompleto"
        );
    }
}

/// Legge un valore dallo spool nel costruttore del tipo dato.
fn dato_dallo_spool(tipo: ColType, byte: &[u8]) -> Result<()> {
    let mut costruttore = InferredColumnBuilder::new(tipo);
    let mut buffer = Vec::new();
    read_spool_data(
        &mut std::io::Cursor::new(byte),
        &mut costruttore,
        &mut buffer,
    )
}

/// `read_spool_data` rifiuta il tag sconosciuto, il booleano che non e' ne'
/// zero ne' uno, il testo non UTF-8 e ogni troncamento.
///
/// Le quattro ragioni hanno quattro messaggi perche' dicono cose diverse a
/// chi legge il log: un tag sconosciuto e' uno spool scritto da un'altra
/// versione, un booleano fuori dai due valori e' un byte corrotto, un testo
/// non UTF-8 e' un payload corrotto, e un troncamento e' un file
/// incompleto. Il formato e' nostro, ma il file sta su disco, e cio' che
/// torna dal disco non e' cio' che ci si e' scritto per definizione.
///
/// Le accettazioni stanno accanto ai rifiuti: senza, una funzione che
/// rifiutasse tutto supererebbe la tabella dei negativi.
#[test]
fn n1_read_spool_data_rifiuta_i_tag_i_booleani_e_i_testi_che_non_lo_sono() {
    for (caso, tipo, byte) in [
        ("nullo", ColType::Text, vec![SPOOL_NULL]),
        (
            "intero",
            ColType::Integer,
            [vec![SPOOL_INTEGER], 7_i64.to_le_bytes().to_vec()].concat(),
        ),
        (
            "numero",
            ColType::Number,
            [vec![SPOOL_NUMBER], 1.5_f64.to_le_bytes().to_vec()].concat(),
        ),
        ("booleano falso", ColType::Boolean, vec![SPOOL_BOOLEAN, 0]),
        ("booleano vero", ColType::Boolean, vec![SPOOL_BOOLEAN, 1]),
        (
            "testo",
            ColType::Text,
            [
                vec![SPOOL_TEXT],
                2_u32.to_le_bytes().to_vec(),
                b"ok".to_vec(),
            ]
            .concat(),
        ),
    ] {
        assert!(
            dato_dallo_spool(tipo, &byte).is_ok(),
            "{caso}: uno spool ben formato deve essere accettato"
        );
    }

    for (caso, tipo, byte, atteso) in [
        (
            "tag che nessuna versione di questo spool scrive",
            ColType::Text,
            vec![9],
            "tag spool XLSX non valido",
        ),
        (
            "booleano che non e' ne' zero ne' uno",
            ColType::Boolean,
            vec![SPOOL_BOOLEAN, 2],
            "booleano spool XLSX non valido",
        ),
        (
            "testo con un byte che UTF-8 non ammette",
            ColType::Text,
            [vec![SPOOL_TEXT], 1_u32.to_le_bytes().to_vec(), vec![0xff]].concat(),
            "testo spool XLSX non UTF-8",
        ),
        (
            "nemmeno il tag",
            ColType::Text,
            Vec::new(),
            "spool XLSX troncato o illeggibile",
        ),
        (
            "intero senza gli otto byte",
            ColType::Integer,
            vec![SPOOL_INTEGER, 1, 2, 3],
            "spool XLSX troncato o illeggibile",
        ),
        (
            "numero senza gli otto byte",
            ColType::Number,
            vec![SPOOL_NUMBER, 1, 2, 3],
            "spool XLSX troncato o illeggibile",
        ),
        (
            "booleano senza il byte del valore",
            ColType::Boolean,
            vec![SPOOL_BOOLEAN],
            "spool XLSX troncato o illeggibile",
        ),
        (
            "testo senza la lunghezza",
            ColType::Text,
            vec![SPOOL_TEXT, 1, 0],
            "spool XLSX troncato o illeggibile",
        ),
        (
            "testo piu' corto della lunghezza dichiarata",
            ColType::Text,
            [
                vec![SPOOL_TEXT],
                4_u32.to_le_bytes().to_vec(),
                b"ab".to_vec(),
            ]
            .concat(),
            "spool XLSX troncato o illeggibile",
        ),
    ] {
        let Err(errore) = dato_dallo_spool(tipo, &byte) else {
            panic!("{caso}: doveva essere rifiutato");
        };
        assert_eq!(errore.message, atteso, "{caso}: messaggio sbagliato");
    }
}

/// Le conversioni di ampiezza dello spool non falliscono su questo
/// bersaglio, e per questo i rami che le sorvegliano sono irraggiungibili.
///
/// Sono tre: `usize::try_from(u32)` nelle due letture -- geometria e testo
/// -- e `u64::try_from(usize)` nella scrittura. Nessuna guardia a monte le
/// ferma: e' la larghezza dei tipi del bersaglio a deciderlo, quindi la
/// verifica sta in un blocco `const` e una piattaforma dove non vale non
/// compila. Sono i tre punti in cui i rami andrebbero riclassificati da
/// irraggiungibili a coperti.
#[test]
fn n1_le_conversioni_di_ampiezza_dello_spool_reggono_su_questo_bersaglio() {
    const {
        assert!(
            usize::BITS >= u32::BITS,
            "le lunghezze dello spool sono u32: senza questo, le due letture possono fallire"
        );
        assert!(
            u64::BITS >= usize::BITS,
            "la lunghezza di una fetta e' usize: senza questo, la scrittura puo' fallire"
        );
    }
    assert!(usize::try_from(u32::MAX).is_ok());
    assert!(u64::try_from(usize::MAX).is_ok());
}

/// Uno spool su un file dato, con quota e stato iniziale scelti.
///
/// Il tipo e i suoi campi sono privati del modulo, quindi la sonda puo'
/// costruire lo stato che l'uso normale non produce -- un contatore gia'
/// a ridosso del massimo -- senza fingere di passare da un'API che non lo
/// permette.
fn spool_su(
    file: &std::fs::File,
    gia_scritti: u64,
    quota: u64,
    budget: OperationBudget,
) -> BoundedSpoolWriter<'_> {
    let mut spool = BoundedSpoolWriter::new(file, quota, budget);
    spool.bytes = gia_scritti;
    spool
}

/// `BoundedSpoolWriter::write` ferma la quota, l'overflow del contatore e
/// l'errore del file, e li distingue.
///
/// I tre rifiuti mandano a fare cose diverse: la quota si alza con
/// `--max-input-bytes`, l'overflow del contatore e' un difetto nostro, e
/// l'errore di scrittura e' il disco. Un messaggio solo per tutti e tre
/// manderebbe chi legge a cambiare l'opzione sbagliata.
#[test]
fn n1_bounded_spool_writer_distingue_la_quota_dall_overflow_e_dal_disco() {
    let dir = tempfile::tempdir().unwrap();
    let opzioni = opzioni_lettura();

    // Il controllo positivo: sotto la quota si scrive.
    let percorso = dir.path().join("spool.bin");
    let file = std::fs::File::create(&percorso).unwrap();
    let mut spool = spool_su(&file, 0, 16, opzioni.budget().clone());
    spool
        .write(b"otto byt")
        .expect("otto byte sotto una quota di sedici");
    spool.finish().expect("la chiusura svuota il buffer");

    // La quota: il confine e' inclusivo, e il byte successivo lo supera.
    let file = std::fs::File::create(dir.path().join("quota.bin")).unwrap();
    let mut spool = spool_su(&file, 0, 4, opzioni.budget().clone());
    spool
        .write(b"abcd")
        .expect("quattro byte in una quota di quattro");
    let Err(errore) = spool.write(b"e") else {
        panic!("il byte che supera la quota deve essere rifiutato");
    };
    assert_eq!(
        errore.category,
        plenora_io_model::ErrorCategory::ResourceLimit,
        "una quota superata e' un limite: {}",
        errore.message
    );
    assert!(
        errore.message.contains("byte oltre il limite di"),
        "il messaggio deve dire qual e' il limite, arrivato «{}»",
        errore.message
    );

    // L'overflow del contatore: raggiungibile solo da uno stato che l'uso
    // normale non produce, ed e' per questo che la sonda lo costruisce.
    // Non e' lo stesso rifiuto della quota, e con `u64::MAX` come quota il
    // controllo sul limite non puo' scattare per primo.
    let file = std::fs::File::create(dir.path().join("overflow.bin")).unwrap();
    let mut spool = spool_su(&file, u64::MAX, u64::MAX, opzioni.budget().clone());
    let Err(errore) = spool.write(b"x") else {
        panic!("un contatore che trabocca non puo' proseguire in silenzio");
    };
    assert_eq!(
        errore.message, "dimensione spool XLSX fuori intervallo",
        "l'overflow del contatore non e' il rifiuto della quota"
    );

    // L'errore del file: un descrittore aperto in sola lettura fallisce
    // alla scrittura, che e' il modo piu' vicino a un disco pieno che una
    // sonda possa produrre senza toccare il sistema.
    let sola_lettura = dir.path().join("sola-lettura.bin");
    std::fs::write(&sola_lettura, b"").unwrap();
    let file = std::fs::File::open(&sola_lettura).unwrap();
    let mut spool = spool_su(&file, 0, 1024, opzioni.budget().clone());
    // `BufWriter` accumula un byte senza toccare il file, quindi la
    // scrittura riesce e l'errore arriva allo svuotamento. Le due chiamate
    // sono separate apposta: e' l'unico modo di eseguire il `flush` di
    // `finish`, che altrimenti resterebbe un ramo dichiarato e mai visto.
    spool
        .write(b"x")
        .expect("un byte sta nel buffer: il file non e' ancora toccato");
    let Err(errore) = spool.finish() else {
        panic!("lo svuotamento su un file aperto in sola lettura deve fallire");
    };
    assert_eq!(
        errore.phase,
        ErrorPhase::Read,
        "l'errore del disco resta nella fase in cui lo spool vive: {}",
        errore.message
    );
}

/// I byte che `BoundedSpoolWriter` produce per un valore o una geometria.
fn byte_spool(scrivi: impl FnOnce(&mut BoundedSpoolWriter<'_>) -> Result<()>) -> Result<Vec<u8>> {
    let dir = tempfile::tempdir().unwrap();
    let percorso = dir.path().join("spool.bin");
    let file = std::fs::File::create(&percorso).unwrap();
    let opzioni = opzioni_lettura();
    {
        let mut spool = BoundedSpoolWriter::new(&file, u64::MAX, opzioni.budget().clone());
        scrivi(&mut spool)?;
        spool.finish()?;
    }
    Ok(std::fs::read(&percorso).unwrap())
}

/// `BoundedSpoolWriter::data` sceglie un tag per ogni variante, e cio' che
/// non e' un valore diventa nullo invece di sparire.
///
/// La scrittura e la rilettura sono due meta' dello stesso formato, e una
/// tabella che guardasse solo i byte prodotti direbbe che il codificatore
/// e' coerente con se stesso. Qui ogni caso passa da `data` e torna da
/// `read_spool_data`: se i due si separassero -- un tag rinumerato da una
/// parte sola, una lunghezza scritta con l'ampiezza sbagliata -- e' il
/// giro completo a rompersi, non l'attesa scritta a mano.
///
/// Il ramo `_` conta quanto gli altri: raccoglie il float non finito, la
/// data nativa, la cella d'errore e la cella vuota, cioe' tutto cio' che
/// non ha un tipo Arrow in cui finire. Diventano nullo, non spariscono:
/// una riga con una cella in meno sfaserebbe tutte le colonne successive.
#[test]
fn n1_bounded_spool_writer_data_da_un_tag_a_ogni_variante_e_annulla_il_resto() {
    for (caso, valore, tag, tipo) in [
        ("intero", Data::Int(-7), SPOOL_INTEGER, ColType::Integer),
        (
            "numero finito",
            Data::Float(1.5),
            SPOOL_NUMBER,
            ColType::Number,
        ),
        (
            "booleano",
            Data::Bool(true),
            SPOOL_BOOLEAN,
            ColType::Boolean,
        ),
        (
            "testo",
            Data::String("ciao".to_owned()),
            SPOOL_TEXT,
            ColType::Text,
        ),
        (
            "data ISO, che resta testo",
            Data::DateTimeIso("2026-01-01".to_owned()),
            SPOOL_TEXT,
            ColType::Text,
        ),
        (
            "durata ISO, che resta testo",
            Data::DurationIso("PT1H".to_owned()),
            SPOOL_TEXT,
            ColType::Text,
        ),
        ("cella vuota", Data::Empty, SPOOL_NULL, ColType::Text),
        (
            "numero non finito",
            Data::Float(f64::NAN),
            SPOOL_NULL,
            ColType::Number,
        ),
        (
            "cella d'errore del foglio",
            Data::Error(calamine::CellErrorType::Div0),
            SPOOL_NULL,
            ColType::Text,
        ),
    ] {
        // Un `match` e non un `unwrap_or_else`: qui non c'e' un valore di
        // ripiego, e il censimento dei fallback conta la forma sintattica.
        let byte = match byte_spool(|spool| spool.data(&valore)) {
            Ok(byte) => byte,
            Err(errore) => panic!("{caso}: la scrittura non doveva fallire: {errore:?}"),
        };
        assert_eq!(
            byte.first().copied(),
            Some(tag),
            "{caso}: il tag scritto non e' quello atteso"
        );
        assert!(
            dato_dallo_spool(tipo, &byte).is_ok(),
            "{caso}: cio' che la scrittura produce deve essere rileggibile"
        );
    }
}

/// `BoundedSpoolWriter::geometry` distingue l'assenza dalla geometria
/// vuota, e il giro completo lo conferma.
///
/// Le due cose escono dallo stesso campo di quattro byte -- `u32::MAX` per
/// l'assenza, la lunghezza altrimenti -- e confonderle darebbe una colonna
/// dove ogni riga senza geometria ha una geometria vuota, che nel contratto
/// non e' la stessa cosa.
#[test]
fn n1_bounded_spool_writer_geometry_distingue_l_assenza_dalla_geometria_vuota() {
    let assente = byte_spool(|spool| spool.geometry(None)).expect("l'assenza si scrive");
    assert_eq!(
        assente,
        SPOOL_NULL_GEOMETRY.to_le_bytes().to_vec(),
        "l'assenza e' il marcatore, e nient'altro dopo"
    );
    assert_eq!(
        geometria_dallo_spool(&assente).expect("il marcatore si rilegge"),
        vec![None]
    );

    let vuota =
        byte_spool(|spool| spool.geometry(Some(&[]))).expect("la geometria vuota si scrive");
    assert_eq!(
        vuota,
        0_u32.to_le_bytes().to_vec(),
        "una geometria vuota e' lunghezza zero, non il marcatore"
    );
    assert_eq!(
        geometria_dallo_spool(&vuota).expect("la lunghezza zero si rilegge"),
        vec![Some(Vec::new())]
    );

    let intera = byte_spool(|spool| spool.geometry(Some(b"wkb"))).expect("la geometria si scrive");
    assert_eq!(
        geometria_dallo_spool(&intera).expect("il giro completo si chiude"),
        vec![Some(b"wkb".to_vec())],
        "cio' che entra nello spool deve uscirne identico"
    );
}

/// Una cella di testo esplicitamente vuota non e' un limite superato.
///
/// # Il difetto che questa sonda ha trovato
///
/// `BoundedSpoolWriter::write` chiedeva una prenotazione di spill per ogni
/// fetta, compresa quella vuota, e `lease_spill(0)` e' un errore nel
/// modello di budget. Il risultato era che un `.xlsx` conforme -- una cella
/// `<c t="inlineStr"><is><t></t></is></c>`, che il formato ammette e che
/// `calamine` consegna come `Data::String("")` -- veniva rifiutato con
/// `ResourceLimit` e il messaggio «una lease deve essere maggiore di zero».
///
/// Non era un errore innocuo per essere fail-closed: mandava chi legge ad
/// alzare una quota che non c'entrava, per un file che non aveva niente di
/// sbagliato. Il rifiuto giusto per la ragione sbagliata e' comunque un
/// rifiuto sbagliato, e qui non era nemmeno giusto.
///
/// Il caso e' emerso da `geometry(Some(&[]))`, che nessun percorso di
/// produzione chiama; la cella di testo vuota, invece, arriva da un file
/// vero, ed e' quella che la sonda usa.
#[test]
fn n1_una_cella_di_testo_vuota_non_e_una_quota_superata() {
    let dir = tempfile::tempdir().unwrap();
    let percorso = xlsx_con_foglio(
        dir.path(),
        "cella-vuota.xlsx",
        &foglio_xml(
            "A1:B2",
            &format!(
                "<row r=\"1\">{}{}</row><row r=\"2\">{}{}</row>",
                cella("A1", "geometry"),
                cella("B1", "nome"),
                cella("A2", "POINT (1 2)"),
                cella("B2", "")
            ),
        ),
    );

    XlsDriver
        .open(
            Source::Path(percorso),
            opzioni_lettura()
                .with_assume_crs("EPSG:4326")
                .with_format_option("wkt_column", "geometry"),
        )
        .expect("una cella di testo vuota e' un valore, non una quota superata");

    // La stessa proprieta' al livello dove il difetto stava: una fetta
    // vuota non prenota e non conta.
    let file = std::fs::File::create(dir.path().join("spool.bin")).unwrap();
    let opzioni = opzioni_lettura();
    let mut spool = spool_su(&file, 0, 0, opzioni.budget().clone());
    spool
        .write(&[])
        .expect("zero byte stanno anche in una quota di zero");
    spool.finish().expect("non c'e' niente da svuotare");
}

/// `encode_geometry_cell` distingue la riga senza geometria dalla riga con
/// una geometria a meta'.
///
/// Sono due esiti che si somigliano e non lo sono: una cella WKT vuota, o
/// una coppia x/y entrambe assenti, dicono «questa riga non ha geometria» e
/// producono `Ok(false)`, che a valle diventa un nullo. Una sola delle due
/// coordinate, invece, e' un dato incompleto: accettarlo significherebbe
/// inventare l'altra meta' o buttare via quella presente, e il rifiuto
/// esiste per non fare ne' l'una ne' l'altra cosa.
///
/// Le due meta' incomplete sono provate separatamente perche' cadono sullo
/// stesso braccio `_` da due direzioni: con una fixture sola, invertire i
/// due `coordinate_cell` non romperebbe nulla.
#[test]
fn n1_encode_geometry_cell_separa_l_assenza_dalla_geometria_a_meta() {
    let dimensioni = SheetBounds {
        start: (0, 0),
        end: (0, 2),
    };
    let codifica = |riga: &[Data], geom: XlsxGeomSpec| {
        let mut viste = BTreeSet::new();
        let mut tipi = BTreeSet::new();
        let mut buffer = Vec::new();
        encode_geometry_cell(
            riga,
            dimensioni,
            geom,
            WkbLimits::default(),
            &mut viste,
            &mut tipi,
            &mut buffer,
        )
        .map(|presente| (presente, buffer.len()))
    };

    // WKT presente: la geometria c'e' e il buffer non e' vuoto.
    let (presente, byte) = codifica(
        &[
            Data::String("POINT (1 2)".to_owned()),
            Data::Empty,
            Data::Empty,
        ],
        XlsxGeomSpec::Wkt(0),
    )
    .expect("un WKT valido si codifica");
    assert!(presente && byte > 0, "una geometria vera riempie il buffer");

    // WKT vuoto o soli spazi: assenza, non errore. Lo spazio conta: e' il
    // caso che una cella «pulita a mano» produce piu' spesso del vuoto.
    for testo in ["", "   "] {
        let (presente, _) = codifica(
            &[Data::String(testo.to_owned()), Data::Empty, Data::Empty],
            XlsxGeomSpec::Wkt(0),
        )
        .expect("una cella WKT vuota non e' un errore");
        assert!(!presente, "«{testo}» non e' una geometria");
    }

    // Coppia x/y completa, e coppia interamente assente.
    let (presente, byte) = codifica(
        &[Data::Empty, Data::Float(1.0), Data::Float(2.0)],
        XlsxGeomSpec::Xy(1, 2),
    )
    .expect("una coppia completa si codifica");
    assert!(presente && byte > 0);

    let (presente, _) = codifica(
        &[Data::Empty, Data::Empty, Data::Empty],
        XlsxGeomSpec::Xy(1, 2),
    )
    .expect("una coppia interamente assente non e' un errore");
    assert!(!presente, "nessuna coordinata significa nessuna geometria");

    // Le due meta' incomplete.
    for (caso, riga) in [
        (
            "solo l'ascissa",
            [Data::Empty, Data::Float(1.0), Data::Empty],
        ),
        (
            "solo l'ordinata",
            [Data::Empty, Data::Empty, Data::Float(2.0)],
        ),
    ] {
        let Err(errore) = codifica(&riga, XlsxGeomSpec::Xy(1, 2)) else {
            panic!("{caso}: mezza coordinata non e' un punto");
        };
        assert_eq!(
            errore.message, "geometria XY incompleta: X e Y devono essere entrambi presenti",
            "{caso}: messaggio sbagliato"
        );
    }
}

/// `write_cell` scrive ogni valore che `json_from_array` sa produrre, e
/// propaga l'errore del formato invece di troncare.
///
/// Le tre righe difensive del `match` non sono raggiungibili, e la ragione
/// e' la stessa per tutte e tre: `json_from_array` non produce quei valori.
/// Restituisce `Null`, `Bool`, `Number` o `String`, oppure fallisce -- non
/// esiste un percorso che le consegni un array o un oggetto JSON, e i
/// `Number` che costruisce sono `i64`, `u64` o `f64` finito, tutti
/// convertibili in `f64`. La sonda esegue quelle due precondizioni invece
/// di argomentarle.
#[test]
fn n1_write_cell_scrive_i_valori_convertibili_e_propaga_il_rifiuto_del_formato() {
    let mut cartella = Workbook::new();
    let foglio = cartella.add_worksheet();

    for (caso, array) in [
        (
            "nullo",
            Arc::new(arrow_array::Int64Array::from(vec![None::<i64>])) as ArrayRef,
        ),
        (
            "intero",
            Arc::new(arrow_array::Int64Array::from(vec![Some(7_i64)])) as ArrayRef,
        ),
        (
            "numero",
            Arc::new(arrow_array::Float64Array::from(vec![Some(1.5_f64)])) as ArrayRef,
        ),
        (
            "booleano",
            Arc::new(arrow_array::BooleanArray::from(vec![Some(true)])) as ArrayRef,
        ),
        (
            "testo",
            Arc::new(arrow_array::StringArray::from(vec![Some("ciao")])) as ArrayRef,
        ),
    ] {
        // Un `match` e non un `unwrap_or_else`: non c'e' un valore di
        // ripiego, e il censimento dei fallback conta la forma sintattica.
        match write_cell(foglio, 0, 0, &array, 0) {
            Ok(()) => {}
            Err(errore) => panic!("{caso}: {errore:?}"),
        }
    }

    // La propagazione da `json_from_array`: un tipo Arrow che non ha una
    // resa JSON senza perdita non diventa una cella approssimata.
    //
    // Il tipo e' una **durata** e non piu' una data: dal 2026-09-04 i
    // temporali che sono istanti hanno una resa testuale deterministica, e
    // pretendere qui il rifiuto di una data scriverebbe nella prova la
    // contraddizione che la capability aveva -- `SCALAR_TYPES` ammette
    // `Temporal`, e il renderer lo rifiutava. Una durata non e' un istante
    // e resta senza resa decisa.
    let non_convertibile: ArrayRef =
        Arc::new(arrow_array::DurationSecondArray::from(vec![Some(90_i64)]));
    let Err(errore) = write_cell(foglio, 1, 0, &non_convertibile, 0) else {
        panic!("un tipo non convertibile non puo' diventare una cella");
    };
    assert_eq!(
        errore.category,
        plenora_io_model::ErrorCategory::Unsupported,
        "e' una capability mancante, non un dato sbagliato: {}",
        errore.message
    );

    // La propagazione dal formato: una stringa oltre il tetto per cella
    // non viene troncata, viene rifiutata.
    let troppo_lunga: ArrayRef = Arc::new(arrow_array::StringArray::from(vec![Some(
        "x".repeat(40_000),
    )]));
    let Err(errore) = write_cell(foglio, 2, 0, &troppo_lunga, 0) else {
        panic!("una cella oltre il tetto del formato non puo' essere troncata in silenzio");
    };
    assert!(
        errore.message.starts_with("XLSX:"),
        "il rifiuto deve venire dal formato, con la classe al posto del Display: «{}»",
        errore.message
    );
}

/// Le due precondizioni che rendono irraggiungibili i rami difensivi di
/// `write_cell`.
///
/// La prima: `json_from_array` non restituisce mai un array o un oggetto
/// JSON, quindi il braccio `other` del `match` non ha input. La seconda: i
/// soli `Number` che costruisce vengono da `i64`, `u64` o `f64` finito, e
/// `as_f64` li converte tutti -- il rifiuto «numero non rappresentabile
/// come f64 in XLSX» non ha input nemmeno lui. Gli estremi sono verificati,
/// non argomentati.
#[test]
fn n1_json_from_array_non_produce_ne_aggregati_ne_numeri_non_rappresentabili() {
    // Un tipo aggregato non diventa un `JsonValue::Array`: diventa un
    // errore. E' la guardia che svuota il braccio `other`.
    let lista: ArrayRef = Arc::new(arrow_array::ListArray::from_iter_primitive::<
        arrow_array::types::Int32Type,
        _,
        _,
    >(vec![Some(vec![Some(1), Some(2)])]));
    assert!(
        driver_common::json_from_array(&lista, 0).is_err(),
        "un aggregato deve essere rifiutato, non convertito in un array JSON"
    );

    // I tre estremi dei `Number` costruibili: il piu' grande senza segno,
    // il piu' piccolo con segno, e il float finito piu' grande.
    for (caso, array) in [
        (
            "u64::MAX",
            Arc::new(arrow_array::UInt64Array::from(vec![Some(u64::MAX)])) as ArrayRef,
        ),
        (
            "i64::MIN",
            Arc::new(arrow_array::Int64Array::from(vec![Some(i64::MIN)])) as ArrayRef,
        ),
        (
            "f64::MAX",
            Arc::new(arrow_array::Float64Array::from(vec![Some(f64::MAX)])) as ArrayRef,
        ),
    ] {
        let valore = match driver_common::json_from_array(&array, 0) {
            Ok(valore) => valore,
            Err(errore) => panic!("{caso}: doveva convertirsi: {errore:?}"),
        };
        let JsonValue::Number(numero) = valore else {
            panic!("{caso}: doveva essere un numero");
        };
        assert!(
                numero.as_f64().is_some(),
                "{caso}: se questo fallisse, «numero non rappresentabile come f64» diventerebbe raggiungibile"
            );
    }
}

/// Uno stato di scrittura XLSX con i lotti dati.
///
/// La quota WKB e' quella predefinita perche' queste sonde non provano il
/// tetto: lo attraversano. Sceglierne una diversa proverebbe la scrittura
/// sotto una quota che nessun uso reale imposta.
///
/// Restituisce gia' incassato perche' `FormatWriter::finish` prende
/// `self: Box<Self>`: il `Box` non e' un giro superfluo, e' la forma che
/// la firma del tratto richiede.
#[allow(clippy::unnecessary_box_returns)]
fn scrittore(percorso: &std::path::Path, xy: bool, lotti: Vec<RecordBatch>) -> Box<XlsWriterState> {
    Box::new(XlsWriterState {
        path: percorso.to_owned(),
        durable: false,
        xy,
        batches: lotti,
        wkb_limits: WkbLimits::default(),
        max_output_bytes: u64::MAX,
    })
}

/// Un lotto di una riga con la sola colonna geometria dichiarata.
fn lotto_geometrico(campo: Field, wkb: &[u8]) -> RecordBatch {
    RecordBatch::try_new(
        Arc::new(Schema::new(vec![campo])),
        vec![Arc::new(BinaryArray::from(vec![Some(wkb)]))],
    )
    .unwrap()
}

/// Il WKB ISO di una geometria, per le sonde di scrittura.
fn wkb_di(valore: WkbValue) -> Vec<u8> {
    encode_wkb(
        &WkbGeometry {
            value: valore,
            dimensions: CoordinateDimensions::Xy,
            srid: None,
        },
        WkbFlavor::Iso,
    )
    .expect("le forme XY di queste sonde si codificano")
}

/// Un vertice XY senza Z ne' M.
const fn vertice(x: f64, y: f64) -> WkbCoordinate {
    WkbCoordinate {
        x,
        y,
        z: None,
        m: None,
    }
}

/// `XlsWriterState::finish` rifiuta il lotto senza geometria, quello con
/// una geometria non binaria, e la geometria che l'encoding `xy` non sa
/// rappresentare.
///
/// I tre rifiuti dicono cose diverse a chi ha costruito il piano: manca la
/// colonna, la colonna c'e' ma non e' WKB, la colonna e' WKB ma contiene
/// qualcosa che due numeri non descrivono. L'ultimo e' il piu' facile da
/// confondere con una perdita accettabile -- «scrivo il centroide» -- e il
/// rifiuto e' la scelta opposta: l'encoding `xy` si chiede esplicitamente,
/// e chi lo chiede su una `LineString` ha sbagliato piano, non dato.
#[test]
fn n1_finish_rifiuta_il_lotto_senza_geometria_binaria_e_la_geometria_non_xy() {
    let dir = tempfile::tempdir().unwrap();
    let campo = geometry_field("geometry", "EPSG:4326");
    let punto = wkb_di(WkbValue::Point(vertice(1.0, 2.0)));

    // Le due accettazioni: senza, un `finish` che rifiutasse ogni
    // geometria supererebbe la tabella dei rifiuti.
    for (caso, xy) in [("wkt", false), ("xy", true)] {
        let esito = scrittore(
            &dir.path().join(format!("buono-{caso}.xlsx")),
            xy,
            vec![lotto_geometrico(campo.clone(), &punto)],
        )
        .finish();
        // Un `match` e non un `unwrap_or_else`: non c'e' un valore di
        // ripiego, e il censimento dei fallback conta la forma sintattica.
        match esito {
            Ok(_) => {}
            Err(errore) => panic!("{caso}: un punto XY si scrive: {errore:?}"),
        }
    }

    // Nessuna colonna dichiarata come geometria.
    let senza = RecordBatch::try_new(
        Arc::new(Schema::new(vec![Field::new(
            "nome",
            arrow_schema::DataType::Utf8,
            true,
        )])),
        vec![Arc::new(arrow_array::StringArray::from(vec![Some("uno")]))],
    )
    .unwrap();
    let Err(errore) = scrittore(&dir.path().join("senza.xlsx"), false, vec![senza]).finish() else {
        panic!("un foglio XLSX di questo driver ha sempre una colonna geometria");
    };
    assert_eq!(errore.message, "nessuna colonna geometria");

    // Colonna dichiarata geometria ma di tipo non binario: i metadati sono
    // gli stessi, cambia solo il tipo Arrow. E' cio' che un piano costruito
    // a mano puo' produrre, ed e' il `downcast` a doverlo fermare.
    let finta = Field::new("geometry", arrow_schema::DataType::Utf8, true)
        .with_metadata(campo.metadata().clone());
    let non_binaria = RecordBatch::try_new(
        Arc::new(Schema::new(vec![finta])),
        vec![Arc::new(arrow_array::StringArray::from(vec![Some(
            "POINT (1 2)",
        )]))],
    )
    .unwrap();
    let Err(errore) = scrittore(
        &dir.path().join("non-binaria.xlsx"),
        false,
        vec![non_binaria],
    )
    .finish() else {
        panic!("una colonna geometria che non e' WKB non e' una geometria");
    };
    assert_eq!(errore.message, "colonna geometria non binaria");

    // Encoding `xy` su una geometria che due numeri non descrivono.
    let linea = wkb_di(WkbValue::LineString(vec![
        vertice(0.0, 0.0),
        vertice(1.0, 1.0),
    ]));
    let Err(errore) = scrittore(
        &dir.path().join("linea.xlsx"),
        true,
        vec![lotto_geometrico(campo, &linea)],
    )
    .finish() else {
        panic!("l'encoding xy non puo' inventare due numeri per una linea");
    };
    assert_eq!(
        errore.message,
        "encoding xy richiede geometrie Point strettamente XY"
    );
}

/// `lunghezza_dello_spool` rifiuta cio' che non entra nei quattro byte, e
/// dice **quale** payload non ci entra.
///
/// I due chiamanti passano messaggi diversi perche' chi legge deve sapere
/// se a non entrare sia una geometria o un testo: le due cose si riducono
/// in modi diversi -- una semplificando la forma, l'altra tagliando la
/// cella -- e un messaggio solo manderebbe a fare la cosa sbagliata meta'
/// delle volte.
///
/// Il confine e' provato dai due lati, ed e' l'unico modo di sapere se sia
/// incluso: `u32::MAX` byte entrano, il successivo no. La sonda non alloca
/// niente -- la lunghezza e' un argomento -- ed e' la ragione per cui la
/// conversione e' stata estratta dai chiamanti, dove la stessa prova
/// avrebbe richiesto quattro gibibyte di memoria.
#[test]
fn n1_lunghezza_dello_spool_rifiuta_i_payload_oltre_i_quattro_byte() {
    let dentro = usize::try_from(u32::MAX).expect("usize copre u32 sui bersagli supportati");
    assert_eq!(
        lunghezza_dello_spool(dentro, "geometria XLSX troppo grande per lo spool")
            .expect("il confine e' incluso"),
        u32::MAX,
        "la lunghezza massima rappresentabile deve passare, non essere respinta di misura"
    );
    assert_eq!(
        lunghezza_dello_spool(0, "testo XLSX troppo grande per lo spool")
            .expect("zero byte sono una lunghezza"),
        0
    );

    // Il byte successivo. Su un bersaglio dove `usize` non superasse `u32`
    // questo numero non esisterebbe, e il ramo sarebbe irraggiungibile
    // invece che coperto: l'`expect` lo direbbe invece di lasciar passare
    // una prova che non prova niente.
    let oltre = dentro
        .checked_add(1)
        .expect("su un bersaglio a sessantaquattro bit c'e' spazio oltre u32::MAX");

    for (caso, messaggio) in [
        ("geometria", "geometria XLSX troppo grande per lo spool"),
        ("testo", "testo XLSX troppo grande per lo spool"),
    ] {
        let Err(errore) = lunghezza_dello_spool(oltre, messaggio) else {
            panic!("{caso}: una lunghezza oltre u32::MAX non entra in quattro byte");
        };
        assert_eq!(errore.message, messaggio, "{caso}: messaggio scambiato");
        assert_eq!(
            errore.category,
            plenora_io_model::ErrorCategory::ResourceLimit,
            "{caso}: e' un limite, non un dato malformato"
        );
    }
}

/// Lo spool XLSX nasce **nella directory scelta**, e se non la trova si
/// ferma invece di ripiegare.
///
/// # Il difetto che questa sonda ha trovato
///
/// La creazione passava da `NamedTempFile::new()`, che usa la directory
/// temporanea di sistema. `PLENORA_SPILL_DIR` dichiara di scegliere dove
/// vive lo spill -- e' scritto in `ENGINEERING.md` -- ma sul percorso XLSX
/// non la governava: chi la impostava per mettere lo spill su un volume
/// capiente se ne accorgeva a disco pieno, che e' il momento peggiore.
///
/// Il ripiego non c'e' e non deve esserci: una directory scelta e non
/// utilizzabile e' un errore di configurazione, e proseguire su un altro
/// volume metterebbe i dati dove l'operatore non ha chiesto.
#[test]
fn n1_lo_spool_nasce_nella_directory_scelta_o_non_nasce() {
    let dir = tempfile::tempdir().unwrap();

    // La directory scelta e' quella che ospita l'inode, e la prova non e'
    // che la creazione riesca ma **dove**: con `NamedTempFile::new()` la
    // riga sarebbe verde lo stesso, e il difetto sarebbe rimasto.
    let spool = crea_lo_spool(dir.path()).expect("una directory esistente ospita lo spool");
    assert_eq!(
        spool.path().parent(),
        Some(dir.path()),
        "lo spool deve stare dove l'operatore ha scelto, non nella temp di sistema"
    );

    // Una directory che non esiste: fallimento chiuso, senza ripiego.
    let assente = dir.path().join("questa-non-esiste");
    let Err(errore) = crea_lo_spool(&assente) else {
        panic!("una directory inesistente non puo' ospitare lo spool");
    };
    assert_eq!(errore.message, "spool XLSX non creabile");
    assert!(
        !assente.exists(),
        "il rifiuto non deve creare la directory che mancava: sceglierla e' \
             dell'operatore"
    );

    // La risoluzione che `infer_layout` usa e' quella del core, non una
    // copia: due risoluzioni divergerebbero, e divergerebbero in silenzio.
    assert!(
        plenora_io_core::driver::spool::spill_directory().is_ok(),
        "senza la variabile impostata la risoluzione da' la temp di sistema"
    );
}

/// `classe_xlsx` traduce ogni variante che sappiamo costruire.
///
/// Sette varianti di `XlsxError` **portano il nome del foglio come dato**,
/// e il `Display` della dipendenza lo stampava: dodici percorsi di
/// scrittura lo facevano uscire nel messaggio pubblico. La classe li tiene
/// distinti senza far uscire nulla.
///
/// Il test esiste per la lezione di `classe_sqlite`: la copertura
/// differenziale del checkpoint su `effc4ab` ha trovato quel `match` mai
/// attraversato. Un vocabolario senza test e' una tabella di traduzione di
/// cui nessuno ha mai letto una riga.
#[test]
fn la_classe_xlsx_traduce_ogni_variante_costruibile() {
    use rust_xlsxwriter::XlsxError as E;

    let campioni: Vec<(E, &str)> = vec![
        (
            E::RowColumnLimitError,
            "riga o colonna oltre il limite del formato",
        ),
        (
            E::SheetnameCannotBeBlank("Foglio segreto".to_owned()),
            "nome del foglio vuoto",
        ),
        (
            E::SheetnameLengthExceeded("Foglio segreto".to_owned()),
            "nome del foglio troppo lungo",
        ),
        (
            E::SheetnameReused("Foglio segreto".to_owned()),
            "nome del foglio gia' usato",
        ),
        (
            E::SheetnameContainsInvalidCharacter("Foglio segreto".to_owned()),
            "nome del foglio con caratteri non ammessi",
        ),
        (
            E::SheetnameStartsOrEndsWithApostrophe("Foglio segreto".to_owned()),
            "nome del foglio delimitato da apostrofi",
        ),
        (
            E::MaxStringLengthExceeded,
            "stringa oltre il limite del formato",
        ),
        (
            E::UnknownWorksheetNameOrIndex("Foglio segreto".to_owned()),
            "foglio inesistente",
        ),
        (
            E::ParameterError("dettaglio".to_owned()),
            "parametro non valido",
        ),
    ];

    let mut visti = std::collections::BTreeSet::new();
    for (errore, atteso) in &campioni {
        assert_eq!(classe_xlsx(errore), *atteso);
        visti.insert(*atteso);
    }
    assert_eq!(
        visti.len(),
        campioni.len(),
        "due varianti distinte non devono avere la stessa classe"
    );

    // Il nome del foglio non esce: e' il punto dell'intero cambiamento.
    let errore = xls_err(E::SheetnameReused("Foglio segreto".to_owned()));
    assert_eq!(errore.driver.as_deref(), Some("xls"));
    assert!(errore.message.contains("nome del foglio gia' usato"));
    assert!(
        !errore.message.contains("Foglio segreto"),
        "il nome del foglio non deve comparire nel messaggio: {}",
        errore.message
    );
}

/// Opzioni di lettura sul modello unificato.
///
/// Da S4.d il percorso di lettura vive interamente li': la memoria dei
/// batch e' una `InternalMemoryLease`, che esiste solo dentro un
/// `PipelineContext`. `opzioni_lettura()` costruisce ancora il ramo
/// legacy — sparira' in S4.e — e con quello `open` fallisce chiuso.
/// La capability `hostile_input_hardened`, provata dove S12 la sposta.
///
/// Non e' il cap in byte: quello esisteva prima del parser progressivo e
/// scatta **prima** di deserializzare, quindi un test che lo esercita
/// resterebbe verde anche rimettendo il parser vecchio. Prova nulla di
/// questo lotto.
///
/// Qui l'input sta comodamente sotto il cap in byte, e a fermarlo e' il
/// tetto sui **componenti** -- l'unita' che solo un'analisi che addebita
/// mentre consuma puo' applicare. Le tre condizioni stanno insieme
/// apposta:
///
///   * con il tetto stretto il rifiuto e' esattamente `LimitExceeded`;
///   * con il default lo stesso identico input passa, quindi il rifiuto
///     viene dal tetto e non dall'input;
///   * l'input e' molto piu' corto del cap in byte, che percio' non
///     c'entra.
///
/// E' la prova che `check_capability_input_ostile.py` esegue per questo
/// driver: cancellarla, rinominarla o indebolirla rende rossa la
/// capability nel catalogo.
#[test]
fn la_cella_wkt_e_rifiutata_per_componenti_sotto_il_cap_in_byte() {
    const COMPONENTI: usize = 5;
    let wkt = "LINESTRING (0 0,1 1,2 2,3 3,4 4)";

    let dir = tempfile::tempdir().expect("tempdir");
    let output = dir.path().join("linea.xlsx");
    let mut workbook = Workbook::new();
    let sheet = workbook.add_worksheet();
    sheet.write_string(0, 0, "geometry").expect("intestazione");
    sheet.write_string(1, 0, wkt).expect("cella");
    workbook.save(&output).expect("salvataggio");

    assert!(
        wkt.len() < plenora_io_model::limits::WkbLimits::default().max_cell_bytes / 1_000,
        "l'input deve stare comodamente sotto il cap in byte"
    );

    let opzioni = |componenti: usize| {
        opzioni_lettura_con(
            plenora_io_model::budget::PipelineLimits::default().with_max_wkb_components(componenti),
        )
        .with_assume_crs("EPSG:4326")
        .with_format_option("wkt_column", "geometry")
    };

    assert!(
        XlsDriver
            .open(Source::Path(output.clone()), opzioni(COMPONENTI))
            .is_ok(),
        "con {COMPONENTI} componenti di tetto la stessa cella deve passare"
    );

    match XlsDriver.open(Source::Path(output), opzioni(COMPONENTI - 1)) {
        Err(errore) => assert_eq!(
            errore.code,
            plenora_io_model::IoErrorCode::LimitExceeded,
            "il rifiuto deve venire dal tetto sui componenti: {}",
            errore.message
        ),
        Ok(_) => panic!("una cella oltre il tetto sui componenti deve fallire"),
    }
}

/// L'inferenza XLSX usa il tetto per cella **configurato**.
///
/// Fino a S5 `encode_geometry_cell` passava
/// `WkbLimits::default().max_cell_bytes` — 64 MiB — quindi
/// `--max-wkb-cell-bytes` non arrivava al parsing WKT, e una cella oltre
/// la soglia richiesta veniva parsata comunque.
#[test]
fn inference_uses_configured_wkt_cell_bytes_not_default() {
    let dir = tempfile::tempdir().expect("tempdir");
    let output = dir.path().join("punto.xlsx");
    let mut workbook = Workbook::new();
    let sheet = workbook.add_worksheet();
    sheet.write_string(0, 0, "geometry").expect("intestazione");
    sheet.write_string(1, 0, "POINT (1 2)").expect("cella");
    workbook.save(&output).expect("salvataggio");

    let opzioni = |byte: usize| {
        opzioni_lettura_con(
            plenora_io_model::budget::PipelineLimits::default().with_max_wkb_cell_bytes(byte),
        )
        .with_assume_crs("EPSG:4326")
        .with_format_option("wkt_column", "geometry")
    };

    assert!(
        XlsDriver
            .open(Source::Path(output.clone()), opzioni(64))
            .is_ok(),
        "una cella dentro il tetto configurato deve passare"
    );

    let esito = XlsDriver.open(Source::Path(output), opzioni(4));
    assert!(
        matches!(
            esito,
            Err(ref errore) if errore.code == plenora_io_model::IoErrorCode::LimitExceeded
                || errore.message.contains("limite")
        ),
        "una cella oltre il tetto configurato deve fallire"
    );
    assert!(
        64 < plenora_io_model::limits::WkbLimits::default().max_cell_bytes,
        "la soglia del test deve stare sotto il default"
    );
}

/// Il testo della cella sta nel tetto, il WKB codificato no.
///
/// `POINT (1 2)` occupa undici caratteri e ventuno byte in WKB: due `f64`
/// costano sedici byte da soli. Il controllo sul testo, che XLSX fa prima
/// di costruire l'AST, non e' quindi una maggiorazione della dimensione
/// codificata, e fino a S5.1 il buffer cresceva oltre il tetto prima che
/// qualcuno se ne accorgesse.
#[test]
fn il_wkb_codificato_non_supera_il_tetto_anche_se_il_testo_ci_sta() {
    // Fra la lunghezza del testo (11) e quella del WKB (21).
    const SOGLIA: usize = 15;

    let dir = tempfile::tempdir().expect("tempdir");
    let output = dir.path().join("punto-stretto.xlsx");
    let mut workbook = Workbook::new();
    let sheet = workbook.add_worksheet();
    sheet.write_string(0, 0, "geometry").expect("intestazione");
    sheet.write_string(1, 0, "POINT (1 2)").expect("cella");
    workbook.save(&output).expect("salvataggio");
    assert!(
        "POINT (1 2)".len() <= SOGLIA,
        "la premessa: il testo deve stare nel tetto"
    );

    let opzioni = opzioni_lettura_con(
        plenora_io_model::budget::PipelineLimits::default().with_max_wkb_cell_bytes(SOGLIA),
    )
    .with_assume_crs("EPSG:4326")
    .with_format_option("wkt_column", "geometry");

    let messaggio = XlsDriver
        .open(Source::Path(output), opzioni)
        .err()
        .map(|errore| errore.message);
    assert!(
        matches!(messaggio, Some(ref testo) if testo.contains("oltre il limite")),
        "la codifica WKB deve fermarsi al tetto, non il parsing del testo: {messaggio:?}"
    );
}

fn opzioni_lettura_con(limits: plenora_io_model::budget::PipelineLimits) -> ReadOptions {
    match plenora_io_model::budget::PipelineBudget::builder()
        .limits(limits)
        .build()
    {
        Ok(bundle) => ReadOptions::from_read_parts(bundle.into_read_parts()),
        Err(error) => unreachable!("bundle di test non costruibile: {error:?}"),
    }
}

/// Opzioni di scrittura sul modello unificato.
///
/// `opzioni_scrittura()` non esiste piu' (S4.e): le opzioni portano un
/// `OperationBudget`, che nasce da una costruzione che puo' fallire.
fn opzioni_scrittura() -> WriteOptions {
    match plenora_io_model::budget::PipelineBudget::builder().build() {
        Ok(bundle) => WriteOptions::from_write_parts(bundle.into_write_parts()),
        Err(error) => unreachable!("bundle di test non costruibile: {error:?}"),
    }
}

fn opzioni_lettura() -> ReadOptions {
    match plenora_io_model::budget::PipelineBudget::builder().build() {
        Ok(bundle) => ReadOptions::from_read_parts(bundle.into_read_parts()),
        Err(error) => unreachable!("bundle di test non costruibile: {error:?}"),
    }
}

use plenora_io_core::request::{BatchTarget, ProjectionMode, ReadScope};
use plenora_io_core::WriteLayer;

/// Un `.xlsx` con riferimenti oltre i limiti del formato viene rifiutato
/// **prima** che `calamine` lo veda, quindi il panico non avviene (FZ-0).
///
/// I due input sono complementari:
///
/// * `riferimento-cella-oltre-u32.xlsx` e' l'input che ha prodotto il
///   finding nello smoke del 2026-08-17, conservato intatto;
/// * `riferimento-cella-nove-lettere.xlsx` e' costruito da zero con CRC
///   corretti e un riferimento `AAAAAAAAA1`. Serve perche' il primo ha il
///   CRC rotto dalla mutazione e verrebbe fermato gia' da quello: senza il
///   secondo, il controllo sui limiti del formato non sarebbe osservato da
///   nessun test.
///
/// La verifica e' che l'errore **non** venga dalla barriera: se il
/// messaggio parlasse di panico, vorrebbe dire che `calamine` e' stato
/// raggiunto lo stesso e che la prevalidazione non serve a niente.
#[test]
fn un_riferimento_oltre_i_limiti_del_formato_e_rifiutato_prima_di_calamine() {
    let semi = [
        "riferimento-cella-oltre-u32.xlsx",
        "riferimento-cella-nove-lettere.xlsx",
    ];
    // Le stesse due dichiarazioni di geometria che usa il fuzz target: il
    // rifiuto precede la geometria, quindi vale per entrambe.
    let dichiarazioni: [Vec<(&str, &str)>; 2] = [
        vec![("wkt_column", "geometry")],
        vec![("x_column", "x"), ("y_column", "y")],
    ];

    for nome in semi {
        let percorso_seme = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fuzz/seeds/xlsx_reader")
            .join(nome);
        assert!(
            percorso_seme.is_file(),
            "seme assente: {}",
            percorso_seme.display()
        );

        for dichiarazione in &dichiarazioni {
            let mut opzioni = opzioni_lettura().with_assume_crs("EPSG:4326");
            for (chiave, valore) in dichiarazione {
                opzioni = opzioni.with_format_option(*chiave, *valore);
            }

            // Nessun panico: senza la prevalidazione questa riga abbatte
            // il processo di test invece di restituire.
            let esito = XlsDriver.open(Source::Path(percorso_seme.clone()), opzioni);

            // Nessun dataset parziale: `open` non consegna un handle a
            // meta'. La prova e' che non ne consegna affatto uno.
            let Err(errore) = esito else {
                panic!("{nome} {dichiarazione:?}: l'input doveva essere rifiutato")
            };

            // Errore tipizzato, fase Read.
            assert_eq!(errore.code, plenora_io_model::IoErrorCode::Format);
            assert_eq!(
                errore.category,
                plenora_io_model::ErrorCategory::DataMapping
            );
            assert_eq!(errore.phase, ErrorPhase::Read);
            assert_eq!(errore.driver.as_deref(), Some("xls"));

            // Il panico e' *impedito*, non catturato: se il messaggio
            // venisse dalla barriera, `calamine` sarebbe stato raggiunto.
            assert!(
                !errore.message.contains("in panico"),
                "{nome}: il rifiuto deve precedere calamine: {errore}"
            );

            // Messaggio pubblico redatto: nessun percorso, nessun valore
            // dell'input.
            let messaggio = errore.message.as_str();
            for vietato in ["riferimento-cella", "fuzz/seeds", "Bncasufw", "AAAAAAAAA"] {
                assert!(
                    !messaggio.contains(vietato),
                    "il messaggio pubblico non deve contenere {vietato:?}: {messaggio}"
                );
            }
        }
    }
}

/// Il seme conforme continua a essere letto: la prevalidazione non rifiuta
/// cio' che il formato ammette.
///
/// Senza questo, un controllo troppo severo — per esempio uno che
/// rifiutasse ogni riferimento con lettere — passerebbe il test sopra e
/// romperebbe ogni XLSX reale, senza che nessuno se ne accorgesse qui.
#[test]
fn un_xlsx_conforme_supera_la_prevalidazione() {
    let seme = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fuzz/seeds/xlsx_reader/minimal.xlsx");
    assert!(seme.is_file(), "seme assente: {}", seme.display());
    let opzioni = opzioni_lettura();
    valida_riferimenti_cella(&seme, opzioni.budget())
        .expect("un workbook conforme non deve essere rifiutato");
}

/// La barriera resta come difesa in profondita' ed e' verificata **da
/// sola**, senza dipendere da un input che faccia ancora panicare la
/// libreria.
///
/// E' la forma giusta dopo FZ-0: la prevalidazione impedisce il panico
/// noto, ma non puo' dimostrare che nessun altro percorso di `calamine`
/// ne produca uno. Se domani ne comparisse un altro, la barriera lo
/// converte comunque in errore tipizzato — e questo test lo dimostra
/// invece di dedurlo.
#[test]
fn la_barriera_converte_un_panico_di_calamine_in_errore_tipizzato() {
    let errore = leggendo_calamine::<()>(|| panic!("panico simulato della libreria"))
        .expect_err("un panico deve diventare un errore");
    assert_eq!(errore.code, plenora_io_model::IoErrorCode::Format);
    assert_eq!(
        errore.category,
        plenora_io_model::ErrorCategory::DataMapping
    );
    assert_eq!(errore.phase, ErrorPhase::Read);
    assert_eq!(errore.driver.as_deref(), Some("xls"));
    // Messaggio statico curato: niente testo del panico, niente impronta.
    assert!(
        !errore.message.contains("panico simulato"),
        "il testo del panico non deve raggiungere il messaggio pubblico: {errore}"
    );
    assert_eq!(errore.message, MESSAGGIO_PANICO_CALAMINE);

    // Percorso normale: la barriera non altera nulla.
    assert_eq!(leggendo_calamine(|| Ok(7_u8)).unwrap(), 7);
}

#[test]
fn coordinate_cells_fail_closed_on_invalid_or_lossy_values() {
    assert!(coordinate_cell(Some(&Data::String("not-a-number".to_owned())), "X").is_err());
    assert!(coordinate_cell(Some(&Data::Float(f64::INFINITY)), "X").is_err());
    assert!(coordinate_cell(Some(&Data::Int((1_i64 << 53) + 1)), "X").is_err());
    assert_eq!(coordinate_cell(Some(&Data::Empty), "X").unwrap(), None);
    assert_eq!(
        coordinate_cell(Some(&Data::String(" 12.5 ".to_owned())), "X").unwrap(),
        Some(12.5)
    );
}

// --- ASSURANCE-N1: i rami negativi -------------------------------------
//
// I rami negativi di `open`, `create` e `validate_archive_ratio`: mai
// eseguiti da nulla — ne' dai test, ne' dal replay — e preesistenti a S9.
//
// La forma e' quella indicata dal registro: **la classe di equivalenza
// della precondizione**. Non serve un file enorme per superare un tetto,
// serve un tetto stretto e un file normale; non serve un `.xls` vero per
// provare che non e' instradato, serve un nome con quell'estensione.

/// `.xls` non e' instradato in lettura, e il rifiuto e' una capability.
///
/// La distinzione conta: un `Format` direbbe «il file e' rotto», e chi
/// legge cercherebbe un errore nel proprio dato invece di convertirlo.
#[test]
fn n1_open_rifiuta_xls_come_capability_non_come_formato() {
    let dir = tempfile::tempdir().unwrap();
    let percorso = dir.path().join("storico.xls");
    std::fs::write(&percorso, b"non importa: l'estensione basta").unwrap();

    let esito = XlsDriver.open(Source::Path(percorso), opzioni_lettura());
    let Err(errore) = esito else {
        panic!(".xls non deve essere aperto da questo driver");
    };
    assert_eq!(
        errore.category,
        plenora_io_model::ErrorCategory::Unsupported,
        "e' una capability mancante, non un file malformato"
    );
    assert!(
        errore.message.contains(".xls"),
        "il messaggio deve dire quale estensione: {}",
        errore.message
    );
}

/// Un XLSX con geometria senza `assume_crs` e' rifiutato in fase CRS.
#[test]
fn n1_open_esige_assume_crs_quando_c_e_geometria() {
    let dir = tempfile::tempdir().unwrap();
    let percorso = dir.path().join("senza-crs.xlsx");
    let mut cartella = Workbook::new();
    let foglio = cartella.add_worksheet();
    foglio.write_string(0, 0, "geometry").unwrap();
    foglio.write_string(1, 0, "POINT (1 2)").unwrap();
    cartella.save(&percorso).unwrap();

    // Le stesse opzioni **senza** `with_assume_crs`: e' l'unica differenza
    // rispetto al caso che passa, ed e' cio' che rende il test una prova
    // del ramo e non di un guasto qualunque.
    let esito = XlsDriver.open(
        Source::Path(percorso.clone()),
        opzioni_lettura().with_format_option("wkt_column", "geometry"),
    );
    let Err(errore) = esito else {
        panic!("senza CRS dichiarato la geometria non e' interpretabile");
    };
    assert_eq!(errore.category, plenora_io_model::ErrorCategory::Crs);

    // Controprova: con il CRS lo stesso file si apre. Senza, il test
    // proverebbe soltanto che *qualcosa* fallisce.
    let esito = XlsDriver.open(
        Source::Path(percorso.clone()),
        opzioni_lettura()
            .with_format_option("wkt_column", "geometry")
            .with_assume_crs("EPSG:4326"),
    );
    assert!(
        esito.is_ok(),
        "con `assume_crs` lo stesso file deve aprirsi"
    );

    // E il foglio **dichiarato** invece che dedotto: e' l'altro ramo del
    // `match` su `format_options["sheet"]`, che nessun test toccava.
    // `rust_xlsxwriter` nomina il primo foglio «Sheet1».
    let esito = XlsDriver.open(
        Source::Path(percorso),
        opzioni_lettura()
            .with_format_option("wkt_column", "geometry")
            .with_format_option("sheet", "Sheet1")
            .with_assume_crs("EPSG:4326"),
    );
    assert!(
        esito.is_ok(),
        "il foglio dichiarato per nome deve essere accettato"
    );
}

/// La destinazione senza estensione `.xlsx` e' **accettata**, e la sonda
/// che diceva il contrario e' diventata questa.
///
/// # Perche' e' cambiata
///
/// Il rifiuto c'era, e non era un requisito del formato: dopo il controllo
/// l'estensione non veniva usata per niente. Era una convenzione travestita
/// da vincolo, e rendeva il formato esplicito insufficiente a scegliere la
/// destinazione -- chi pubblicava su un percorso di staging o su un nome
/// generato veniva rifiutato per il nome invece che per i dati.
///
/// # Che cosa resta vero, e dove si legge
///
/// Che un `.ods` contenente un XLSX non venga **riconosciuto** da chi lo
/// rilegge senza dichiarare il formato. Quella meta' non e' sparita: e'
/// dichiarata in `recognised_suffixes`, che il catalogo pubblica. Questa
/// sonda verifica entrambe le cose insieme, perche' separate direbbero
/// meta' della verita'.
#[test]
fn n1_create_accetta_una_destinazione_senza_estensione_xlsx() {
    let dir = tempfile::tempdir().unwrap();
    let uscita = dir.path().join("uscita.ods");
    assert!(
        !uscita.exists(),
        "la premessa: nessun conflitto di destinazione"
    );

    let schema = Arc::new(Schema::new(vec![Field::new(
        "valore",
        arrow_schema::DataType::Utf8,
        true,
    )]));
    let piano = WritePlan {
        layers: vec![WriteLayer {
            name: "foglio".to_owned(),
            contract: DataContract::new(schema, None),
        }],
    };

    let esito = XlsDriver.create(Sink::Path(uscita), &piano, &opzioni_scrittura());
    assert!(
        esito.is_ok(),
        "il percorso e' del chiamante: {:?}",
        esito.err().map(|e| e.category)
    );

    // E la meta' che resta: il suffisso con cui **questo** formato viene
    // riconosciuto e' dichiarato, quindi chi sceglie un altro nome lo fa
    // sapendo che cosa perde.
    assert_eq!(
        XlsDriver.descriptor().recognised_suffixes(),
        &["xlsx"],
        "il suffisso di riconoscimento e' dichiarato, non dedotto"
    );
    assert_eq!(
        XlsDriver.descriptor().sink_path(),
        Some(plenora_io_core::SinkPathConstraint::Free),
        "e la scrittura non pretende nulla dal percorso"
    );
}

/// Un piano senza esattamente un layer e' fermato **prima** di `create`.
///
/// La prima stesura di questo test si chiamava «`create` rifiuta un piano
/// che non ha esattamente un layer» e passava — ma la misura di copertura
/// ha mostrato che il ramo di `create` **restava scoperto**: `validate_write`
/// nel core ferma entrambe le classi prima, e l'asserzione sulla sola
/// categoria `Unsupported` era soddisfatta da un errore diverso.
///
/// Era un test verde che provava un'altra cosa. Ora prova quella giusta, ed
/// e' un contratto piu' forte: fissa la **precedenza**. Il ramo di `create`
/// resta difensivo, e questo test e' la ragione per cui possiamo dirlo
/// invece di supporlo.
#[test]
fn n1_un_piano_senza_un_solo_layer_e_fermato_prima_di_create() {
    let dir = tempfile::tempdir().unwrap();
    let schema = Arc::new(Schema::new(vec![Field::new(
        "valore",
        arrow_schema::DataType::Utf8,
        true,
    )]));
    let strato = |nome: &str| WriteLayer {
        name: nome.to_owned(),
        contract: DataContract::new(Arc::clone(&schema), None),
    };

    // Le due classi di equivalenza della precondizione `len() != 1`:
    // sotto e sopra. Provarne una sola lascerebbe l'altra a nessuno.
    for (nome, strati) in [
        ("nessun-layer", vec![]),
        ("due-layer", vec![strato("primo"), strato("secondo")]),
    ] {
        let uscita = dir.path().join(format!("{nome}.xlsx"));
        let piano = WritePlan { layers: strati };
        let Err(errore) =
            XlsDriver.create(Sink::Path(uscita.clone()), &piano, &opzioni_scrittura())
        else {
            panic!("{nome}: il piano va rifiutato");
        };
        assert_eq!(
            errore.category,
            plenora_io_model::ErrorCategory::Unsupported,
            "{nome}"
        );
        // `Capability` e non `Unsupported` come codice: e' la firma di
        // `validate_write`, ed e' cio' che dimostra **chi** ha rifiutato.
        // Senza questa riga il test tornerebbe a essere soddisfatto da
        // qualunque rifiuto.
        assert_eq!(
            errore.code,
            plenora_io_model::IoErrorCode::Capability,
            "{nome}: il rifiuto deve venire da validate_write, non da create"
        );
        assert!(
            !uscita.exists(),
            "{nome}: un rifiuto non lascia destinazione"
        );
    }
}

/// Un `geometry_encoding` non ammesso e' fermato dalla validazione delle
/// opzioni, non da `create`.
///
/// Il commento nel codice lo dichiarava «difensivo». Questo test lo
/// **misura**: l'errore che esce e' quello del validatore delle opzioni,
/// con il token dell'opzione rifiutata e l'elenco degli ammessi. Il ramo
/// dentro `create` resta percio' irraggiungibile dall'API pubblica.
#[test]
fn n1_un_geometry_encoding_non_ammesso_e_fermato_prima_di_create() {
    let dir = tempfile::tempdir().unwrap();
    let uscita = dir.path().join("encoding.xlsx");
    let schema = Arc::new(Schema::new(vec![Field::new(
        "valore",
        arrow_schema::DataType::Utf8,
        true,
    )]));
    let piano = WritePlan {
        layers: vec![WriteLayer {
            name: "foglio".to_owned(),
            contract: DataContract::new(schema, None),
        }],
    };
    let mut opzioni = opzioni_scrittura();
    opzioni
        .format_options
        .insert("geometry_encoding".to_owned(), "WKB".to_owned());

    let errore = XlsDriver
        .create(Sink::Path(uscita.clone()), &piano, &opzioni)
        .err()
        .expect("un encoding non ammesso va rifiutato");

    assert_eq!(
        errore.category,
        plenora_io_model::ErrorCategory::InvalidConfiguration,
        "e' una configurazione non valida, non una capability mancante"
    );
    // Il messaggio del validatore enumera gli ammessi: e' la firma che
    // distingue **chi** ha rifiutato, e senza di essa il test sarebbe
    // soddisfatto anche dal ramo difensivo di `create`.
    assert!(
        errore.message.contains("wkt") && errore.message.contains("xy"),
        "il rifiuto deve venire dal validatore delle opzioni: {}",
        errore.message
    );
    assert!(!uscita.exists(), "un rifiuto non lascia destinazione");
}

/// Il calcolo del rapporto di decompressione non puo' andare in overflow.
///
/// Non serve un archivio enorme: serve un **moltiplicatore** enorme.
/// `compressed.checked_mul(maximum_ratio)` con `ratio` vicino a `u64::MAX`
/// trabocca su qualunque file non vuoto, ed e' la classe di equivalenza
/// della precondizione — non un caso patologico costruito ad arte.
#[test]
fn n1_il_rapporto_di_decompressione_non_trabocca_in_silenzio() {
    let dir = tempfile::tempdir().unwrap();
    let percorso = dir.path().join("overflow.xlsx");
    let mut cartella = Workbook::new();
    let foglio = cartella.add_worksheet();
    foglio.write_string(0, 0, "valore").unwrap();
    foglio.write_string(1, 0, "uno").unwrap();
    cartella.save(&percorso).unwrap();

    let esito = XlsDriver.open(
        Source::Path(percorso),
        opzioni_lettura_con(
            plenora_io_model::budget::PipelineLimits::default().with_decompression_ratio(u64::MAX),
        ),
    );
    let Err(errore) = esito else {
        panic!("il prodotto deve traboccare e fallire chiuso");
    };
    assert_eq!(
        errore.category,
        plenora_io_model::ErrorCategory::ResourceLimit,
        "un overflow di calcolo e' un limite, non un formato non valido"
    );
    assert!(
        errore.message.contains("overflow"),
        "il messaggio deve dire che si tratta di un overflow: {}",
        errore.message
    );
}

#[test]
fn existing_destination_precedes_unsupported_xlsx_extension() {
    let dir = tempfile::tempdir().unwrap();
    let output = dir.path().join("existing.unsupported");
    std::fs::write(&output, b"sentinel").unwrap();
    let schema = Arc::new(Schema::new(vec![Field::new(
        "value",
        arrow_schema::DataType::Utf8,
        true,
    )]));
    let plan = WritePlan {
        layers: vec![WriteLayer {
            name: "sheet".to_owned(),
            contract: DataContract::new(schema, None),
        }],
    };

    let error = XlsDriver
        .create(Sink::Path(output.clone()), &plan, &opzioni_scrittura())
        .err()
        .expect("la destinazione esistente deve essere rifiutata");
    assert_eq!(error.code, plenora_io_model::IoErrorCode::OutputExists);
    assert_eq!(error.category, plenora_io_model::ErrorCategory::Conflict);
    assert_eq!(std::fs::read(output).unwrap(), b"sentinel");
}

#[test]
fn write_then_read_round_trip() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("out.xlsx");
    let wkb = encode_wkb(&wkt("POINT (12.5 45.9)").unwrap(), WkbFlavor::Iso).unwrap();
    let schema: SchemaRef = Arc::new(Schema::new(vec![
        geometry_field(GEOMETRY, "EPSG:4326"),
        Field::new("nome", arrow_schema::DataType::Utf8, true),
        Field::new("pop", arrow_schema::DataType::Int64, true),
    ]));
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(BinaryArray::from(vec![Some(wkb.as_slice())])),
            Arc::new(arrow_array::StringArray::from(vec!["Roma"])),
            Arc::new(arrow_array::Int64Array::from(vec![2_800_000i64])),
        ],
    )
    .unwrap();

    let driver = XlsDriver;
    let plan = WritePlan {
        layers: vec![WriteLayer {
            name: "l".to_owned(),
            contract: DataContract {
                schema,
                geometry: None,
            },
        }],
    };
    let mut w = driver
        .create(Sink::Path(out.clone()), &plan, &opzioni_scrittura())
        .unwrap();
    w.write(&batch).unwrap();
    w.finish().unwrap();

    let ropts = opzioni_lettura()
        .with_assume_crs("EPSG:4326")
        .with_format_option("wkt_column", "geometry");
    let ds = driver.open(Source::Path(out), ropts).unwrap();
    let mut r = ds
        .open_layer_reader(&ReadRequest {
            layer: LayerId(0),
            projected_fields: None,
            projection_mode: ProjectionMode::BestEffort,
            pruning_predicate: None,
            spatial_pruning_hint: None,
            scope: ReadScope::default(),
            batch_target: BatchTarget::default(),
            cancellation: CancellationToken::default(),
        })
        .unwrap();
    let rb = r.next_batch().unwrap().unwrap();
    assert_eq!(rb.num_rows(), 1);
    let nome = rb
        .column_by_name("nome")
        .unwrap()
        .as_any()
        .downcast_ref::<arrow_array::StringArray>()
        .unwrap();
    assert_eq!(nome.value(0), "Roma");
}

#[test]
fn xlsx_reader_emits_bounded_batches_and_preserves_sparse_rows() {
    let dir = tempfile::tempdir().unwrap();
    let output = dir.path().join("sparse.xlsx");
    let mut workbook = Workbook::new();
    let sheet = workbook.add_worksheet();
    sheet.write_string(0, 0, "name").unwrap();
    sheet.write_string(0, 1, "geometry").unwrap();
    sheet.write_string(1, 0, "first").unwrap();
    sheet.write_string(1, 1, "POINT (1 2)").unwrap();
    sheet.write_string(3, 0, "third").unwrap();
    sheet.write_string(3, 1, "POINT (3 4)").unwrap();
    workbook.save(&output).unwrap();

    let driver = XlsDriver;
    let dataset = driver
        .open(
            Source::Path(output),
            opzioni_lettura()
                .with_assume_crs("EPSG:4326")
                .with_format_option("wkt_column", "geometry"),
        )
        .unwrap();
    let mut reader = dataset
        .open_layer_reader(&ReadRequest {
            layer: LayerId(0),
            projected_fields: None,
            projection_mode: ProjectionMode::BestEffort,
            pruning_predicate: None,
            spatial_pruning_hint: None,
            scope: ReadScope::default(),
            batch_target: BatchTarget {
                target_bytes: 1024,
                max_rows: 1,
            },
            cancellation: CancellationToken::default(),
        })
        .unwrap();

    let first = reader.next_batch().unwrap().unwrap();
    let empty = reader.next_batch().unwrap().unwrap();
    let third = reader.next_batch().unwrap().unwrap();
    assert_eq!(
        [first.num_rows(), empty.num_rows(), third.num_rows()],
        [1, 1, 1]
    );
    assert!(empty
        .column(0)
        .as_any()
        .downcast_ref::<BinaryArray>()
        .unwrap()
        .is_null(0));
    assert!(empty
        .column_by_name("name")
        .unwrap()
        .as_any()
        .downcast_ref::<arrow_array::StringArray>()
        .unwrap()
        .is_null(0));
    assert!(reader.next_batch().unwrap().is_none());
}

#[test]
fn xlsx_reader_stops_after_cancellation_between_batches() {
    let dir = tempfile::tempdir().unwrap();
    let output = dir.path().join("cancel.xlsx");
    let mut workbook = Workbook::new();
    let sheet = workbook.add_worksheet();
    sheet.write_string(0, 0, "geometry").unwrap();
    for row in 1..=4 {
        sheet
            .write_string(row, 0, format!("POINT ({row} {row})"))
            .unwrap();
    }
    workbook.save(&output).unwrap();

    let driver = XlsDriver;
    let dataset = driver
        .open(
            Source::Path(output),
            opzioni_lettura()
                .with_assume_crs("EPSG:4326")
                .with_format_option("wkt_column", "geometry"),
        )
        .unwrap();
    let cancellation = CancellationToken::new();
    let mut reader = dataset
        .open_layer_reader(&ReadRequest {
            layer: LayerId(0),
            projected_fields: None,
            projection_mode: ProjectionMode::BestEffort,
            pruning_predicate: None,
            spatial_pruning_hint: None,
            scope: ReadScope::default(),
            batch_target: BatchTarget {
                target_bytes: 1024,
                max_rows: 1,
            },
            cancellation: cancellation.clone(),
        })
        .unwrap();
    assert_eq!(reader.next_batch().unwrap().unwrap().num_rows(), 1);
    cancellation.cancel();
    assert!(reader.next_batch().is_err());
}

#[test]
fn xlsx_spool_is_bounded_by_the_input_byte_limit() {
    let dir = tempfile::tempdir().unwrap();
    let output = dir.path().join("bounded.xlsx");
    let mut workbook = Workbook::new();
    let sheet = workbook.add_worksheet();
    sheet.write_string(0, 0, "name").unwrap();
    sheet.write_string(0, 1, "geometry").unwrap();
    let repeated = "x".repeat(1_000);
    for row in 1..=100 {
        sheet.write_string(row, 0, &repeated).unwrap();
        sheet.write_string(row, 1, "POINT (1 2)").unwrap();
    }
    workbook.save(&output).unwrap();
    let input_bytes = std::fs::metadata(&output).unwrap().len();

    let result = XlsDriver.open(
        Source::Path(output),
        opzioni_lettura_con(
            plenora_io_model::budget::PipelineLimits::default().with_max_input_bytes(input_bytes),
        )
        .with_assume_crs("EPSG:4326")
        .with_format_option("wkt_column", "geometry"),
    );
    let error = result.err().expect("lo spool deve rispettare il limite");
    assert_eq!(error.code, plenora_io_model::IoErrorCode::LimitExceeded);
}

#[test]
fn xlsx_decompression_ratio_is_checked_before_cell_materialization() {
    let dir = tempfile::tempdir().unwrap();
    let output = dir.path().join("compressed.xlsx");
    let mut workbook = Workbook::new();
    let sheet = workbook.add_worksheet();
    sheet.write_string(0, 0, "geometry").unwrap();
    sheet.write_string(1, 0, "POINT (1 2)").unwrap();
    workbook.save(&output).unwrap();
    let result = XlsDriver.open(
        Source::Path(output),
        opzioni_lettura_con(
            plenora_io_model::budget::PipelineLimits::default().with_decompression_ratio(1),
        )
        .with_assume_crs("EPSG:4326")
        .with_format_option("wkt_column", "geometry"),
    );
    let error = result.err().expect("il rapporto deve fallire chiuso");
    assert_eq!(
        error.category,
        plenora_io_model::ErrorCategory::ResourceLimit
    );
}

#[test]
fn xlsx_wkt_xym_round_trip_preserves_payload_and_contract() {
    let dir = tempfile::tempdir().unwrap();
    let output = dir.path().join("xym.xlsx");
    let expected = wkt("MULTILINESTRING M ((0 0 5,1 1 6))").unwrap();
    let bytes = encode_wkb(&expected, WkbFlavor::Iso).unwrap();
    let mut geometry_contract =
        GeometryColumnContract::wkb_xy(FieldId(0), GEOMETRY, ResolvedCrs::wgs84(), false);
    geometry_contract.dimensions = CoordinateDimensions::Xym;
    geometry_contract.set_exact_geometry_types(vec![GeometryType::MultiLineString]);
    let schema: SchemaRef = Arc::new(Schema::new(vec![
        with_geometry_contract_metadata(&geometry_field(GEOMETRY, "EPSG:4326"), &geometry_contract),
        Field::new("id", arrow_schema::DataType::Int64, false),
    ]));
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(BinaryArray::from(vec![Some(bytes.as_slice())])),
            Arc::new(arrow_array::Int64Array::from(vec![1_i64])),
        ],
    )
    .unwrap();
    let plan = WritePlan {
        layers: vec![WriteLayer {
            name: "layer".to_owned(),
            contract: DataContract {
                schema,
                geometry: Some(geometry_contract),
            },
        }],
    };

    let driver = XlsDriver;
    let mut writer = driver
        .create(Sink::Path(output.clone()), &plan, &opzioni_scrittura())
        .unwrap();
    writer.write(&batch).unwrap();
    writer.finish().unwrap();

    let read_options = opzioni_lettura()
        .with_assume_crs("EPSG:4326")
        .with_format_option("wkt_column", "geometry");
    let dataset = driver.open(Source::Path(output), read_options).unwrap();
    let output_contract = dataset.layers()[0].contract.geometry.as_ref().unwrap();
    assert_eq!(output_contract.dimensions, CoordinateDimensions::Xym);
    assert_eq!(
        output_contract.geometry_types,
        vec![GeometryType::MultiLineString]
    );
    assert_eq!(
        output_contract
            .native_metadata
            .get("xlsx.geometry_encoding")
            .map(String::as_str),
        Some("wkt")
    );
    let mut reader = dataset
        .open_layer_reader(&ReadRequest {
            layer: LayerId(0),
            projected_fields: None,
            projection_mode: ProjectionMode::BestEffort,
            pruning_predicate: None,
            spatial_pruning_hint: None,
            scope: ReadScope::default(),
            batch_target: BatchTarget::default(),
            cancellation: CancellationToken::default(),
        })
        .unwrap();
    let batch = reader.next_batch().unwrap().unwrap();
    let geometry = batch
        .column(0)
        .as_any()
        .downcast_ref::<BinaryArray>()
        .unwrap();
    let actual = decode_wkb(
        geometry.value(0),
        &plenora_io_model::limits::WkbLimits::default(),
    )
    .unwrap();
    assert_eq!(actual, expected);
}

// --- `<dimension>` e' opzionale, e il foglio si legge lo stesso ---------

/// Un XLSX minimo, con o senza l'elemento `<dimension>`.
///
/// Scritto a mano invece che con `rust_xlsxwriter`, che `<dimension>` lo
/// emette sempre: la variabile di queste sonde e' proprio la sua assenza, e
/// una libreria che non sa ometterlo non puo' produrre il caso.
///
/// `dichiarata` porta il `ref` da scrivere, e i tre valori che le sonde
/// usano sono: `None` -- nessun elemento, il caso della specifica; il `ref`
/// giusto; e un `ref` piu' stretto delle celle, che resta un rifiuto.
fn xlsx_con_dimension(dichiarata: Option<&str>) -> Vec<u8> {
    let dimensione = dichiarata.map_or_else(String::new, |riferimento| {
        format!("<dimension ref=\"{riferimento}\"/>")
    });
    let foglio = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
             <worksheet xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\">\
             {dimensione}<sheetData>\
             <row r=\"1\">\
             <c r=\"A1\" t=\"inlineStr\"><is><t>geometry</t></is></c>\
             <c r=\"B1\" t=\"inlineStr\"><is><t>nome</t></is></c>\
             </row>\
             <row r=\"2\">\
             <c r=\"A2\" t=\"inlineStr\"><is><t>POINT (1 2)</t></is></c>\
             <c r=\"B2\" t=\"inlineStr\"><is><t>alfa</t></is></c>\
             </row>\
             <row r=\"3\">\
             <c r=\"A3\" t=\"inlineStr\"><is><t>POINT (3 4)</t></is></c>\
             <c r=\"B3\" t=\"inlineStr\"><is><t>beta</t></is></c>\
             </row>\
             </sheetData></worksheet>"
    );
    let tipi = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
             <Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">\
             <Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/>\
             <Default Extension=\"xml\" ContentType=\"application/xml\"/>\
             <Override PartName=\"/xl/workbook.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml\"/>\
             <Override PartName=\"/xl/worksheets/sheet1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml\"/>\
             </Types>";
    let rels = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
             <Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
             <Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"xl/workbook.xml\"/>\
             </Relationships>";
    let workbook = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
             <workbook xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\" xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\">\
             <sheets><sheet name=\"foglio\" sheetId=\"1\" r:id=\"rId1\"/></sheets></workbook>";
    let workbook_rels = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
             <Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
             <Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet\" Target=\"worksheets/sheet1.xml\"/>\
             </Relationships>";

    let mut buffer = std::io::Cursor::new(Vec::new());
    {
        let mut scrittore = zip::ZipWriter::new(&mut buffer);
        let opzioni: zip::write::FileOptions<'_, ()> =
            zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Stored);
        for (nome, testo) in [
            ("[Content_Types].xml", tipi),
            ("_rels/.rels", rels),
            ("xl/workbook.xml", workbook),
            ("xl/_rels/workbook.xml.rels", workbook_rels),
            ("xl/worksheets/sheet1.xml", foglio.as_str()),
        ] {
            scrittore.start_file(nome, opzioni).unwrap();
            std::io::Write::write_all(&mut scrittore, testo.as_bytes()).unwrap();
        }
        scrittore.finish().unwrap();
    }
    buffer.into_inner()
}

/// Apre il foglio e restituisce `(righe, colonne)` del contratto.
fn apri_e_conta(byte: &[u8], nome: &str) -> Result<(usize, usize)> {
    let dir = tempfile::tempdir().unwrap();
    let percorso = dir.path().join(nome);
    std::fs::write(&percorso, byte).unwrap();
    let dataset = XlsDriver.open(Source::Path(percorso), opzioni_lettura_wkt())?;
    let colonne = dataset.layers()[0].contract.schema.fields().len();
    let mut lettore = dataset.open_layer_reader(&ReadRequest {
        layer: LayerId(0),
        projected_fields: None,
        projection_mode: ProjectionMode::BestEffort,
        pruning_predicate: None,
        spatial_pruning_hint: None,
        scope: ReadScope::default(),
        batch_target: BatchTarget::default(),
        cancellation: CancellationToken::default(),
    })?;
    let mut righe = 0usize;
    while let Some(batch) = lettore.next_batch()? {
        righe += batch.num_rows();
    }
    Ok((righe, colonne))
}

// --- H-01: che cosa la cornice degenere puo' e non puo' fare ------------

/// Un XLSX minimo con le celle date, e **senza** `<dimension>`.
///
/// Le tre sonde di H-01 variano una cosa sola -- quante celle il foglio
/// contiene -- perche' la revisione riguarda esattamente quella: la cornice
/// che `limiti_osservati` restituisce quando non ne osserva nessuna.
fn xlsx_senza_dimension_con(celle: &str) -> Vec<u8> {
    let foglio = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
             <worksheet xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\">\
             <sheetData>{celle}</sheetData></worksheet>"
    );
    let tipi = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
             <Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">\
             <Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/>\
             <Default Extension=\"xml\" ContentType=\"application/xml\"/>\
             <Override PartName=\"/xl/workbook.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml\"/>\
             <Override PartName=\"/xl/worksheets/sheet1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml\"/>\
             </Types>";
    let rels = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
             <Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
             <Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"xl/workbook.xml\"/>\
             </Relationships>";
    let workbook = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
             <workbook xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\" xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\">\
             <sheets><sheet name=\"foglio\" sheetId=\"1\" r:id=\"rId1\"/></sheets></workbook>";
    let workbook_rels = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
             <Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
             <Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet\" Target=\"worksheets/sheet1.xml\"/>\
             </Relationships>";

    let mut buffer = std::io::Cursor::new(Vec::new());
    {
        let mut scrittore = zip::ZipWriter::new(&mut buffer);
        let opzioni: zip::write::FileOptions<'_, ()> =
            zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Stored);
        for (nome, testo) in [
            ("[Content_Types].xml", tipi),
            ("_rels/.rels", rels),
            ("xl/workbook.xml", workbook),
            ("xl/_rels/workbook.xml.rels", workbook_rels),
            ("xl/worksheets/sheet1.xml", foglio.as_str()),
        ] {
            scrittore.start_file(nome, opzioni).unwrap();
            std::io::Write::write_all(&mut scrittore, testo.as_bytes()).unwrap();
        }
        scrittore.finish().unwrap();
    }
    buffer.into_inner()
}

/// H-01, prima condizione: **zero celle osservate → rifiuto tipizzato**.
///
/// La cornice degenere che `limiti_osservati` restituisce quando non osserva
/// nemmeno una cella e' il ripiego che la revisione H-01 ammette, e questa
/// sonda ne fissa il solo scopo: instradare il foglio verso un rifiuto
/// **deterministico e tipizzato**, non verso un'accettazione.
///
/// Il rifiuto si asserisce per codice, categoria e fase, non per testo: un
/// messaggio si riscrive senza accorgersene, e cio' che il chiamante
/// programma sono i primi tre.
#[test]
fn h01_un_foglio_senza_celle_e_senza_dimension_viene_rifiutato() {
    let dir = tempfile::tempdir().unwrap();
    let percorso = dir.path().join("vuoto.xlsx");
    std::fs::write(&percorso, xlsx_senza_dimension_con("")).unwrap();

    let Err(errore) = XlsDriver.open(Source::Path(percorso), opzioni_lettura_wkt()) else {
        panic!(
            "un foglio senza celle non ha un layout da inferire: la cornice \
                 degenere deve portare al rifiuto, non all'accettazione"
        );
    };
    assert_eq!(errore.code, plenora_io_model::IoErrorCode::Format);
    assert_eq!(
        errore.category,
        plenora_io_model::ErrorCategory::DataMapping
    );
    assert_eq!(errore.phase, plenora_io_model::ErrorPhase::Read);
    assert_eq!(
        errore.driver.as_deref(),
        Some("xls"),
        "il rifiuto viene dal driver, e lo dice"
    );
}

/// H-01, seconda condizione: **una vera cella `A1` non e' un foglio vuoto**.
///
/// E' il confine che rende la deroga ammissibile invece che comoda.
/// `calamine` non distingue «`<dimension>` assente» da «`A1:A1`»: in
/// entrambi i casi restituisce la cornice di una cella sola, ed e' la
/// stessa cornice che la scansione produce quando le celle sono zero.
///
/// A distinguerli non e' la cornice ma il **conteggio delle celle**, ed e'
/// per questo che il ripiego non allarga cio' che viene accettato: un
/// foglio con una cella la porta, e passa oltre.
#[test]
fn h01_una_cella_vera_non_viene_scambiata_per_foglio_vuoto() {
    let dir = tempfile::tempdir().unwrap();
    let percorso = dir.path().join("una_cella.xlsx");
    std::fs::write(
        &percorso,
        xlsx_senza_dimension_con(
            "<row r=\"1\"><c r=\"A1\" t=\"inlineStr\"><is><t>geometry</t></is></c></row>",
        ),
    )
    .unwrap();

    // Il foglio ha la sola intestazione e nessuna riga di dati: e' un
    // dataset vuoto, non un file illeggibile. Il rifiuto, se arriva, non
    // dev'essere quello del foglio senza celle.
    match XlsDriver.open(Source::Path(percorso), opzioni_lettura_wkt()) {
        Ok(dataset) => {
            assert_eq!(
                dataset.layers().len(),
                1,
                "una cella vera produce un layer, non il nulla"
            );
        }
        Err(errore) => {
            assert_ne!(
                errore.message, "foglio vuoto",
                "la cella c'e' ed e' stata osservata: il rifiuto del foglio \
                     vuoto qui sarebbe la cornice degenere scambiata per assenza"
            );
        }
    }
}

/// H-01, terza condizione: **dalla cornice sintetica non nasce niente**.
///
/// Il ripiego non deve produrre uno schema, un batch o un file: se lo
/// facesse, un foglio senza celle diventerebbe un dataset di una colonna e
/// una riga inventate, ed e' esattamente la degradazione silenziosa che il
/// registro dei fallback esiste per rendere visibile.
///
/// Qui si verificano le due meta' che il driver puo' mostrare: `open` non
/// restituisce un handle, e senza handle non esiste un contratto da cui
/// ricavare uno schema ne' un reader da cui ottenere un batch -- non e'
/// un'asserzione sul comportamento, e' la firma di `open` a non lasciare
/// altra strada. La terza meta', che nessuna **uscita** nasca, si osserva
/// dove una destinazione esiste: `plenora-io-tools/tests/foglio_vuoto.rs`.
#[test]
fn h01_dalla_cornice_sintetica_non_nasce_ne_schema_ne_batch() {
    let dir = tempfile::tempdir().unwrap();
    let percorso = dir.path().join("vuoto.xlsx");
    std::fs::write(&percorso, xlsx_senza_dimension_con("")).unwrap();

    let esito = XlsDriver.open(Source::Path(percorso), opzioni_lettura_wkt());
    let motivo = esito.as_ref().err();
    assert!(
            motivo.is_some(),
            "il foglio senza celle non produce un dataset, e senza dataset non              c'e' schema da leggere ne' batch da consegnare"
        );
}

/// Il foglio senza `<dimension>` si legge, e legge **tutto**.
///
/// L'elemento e' opzionale in ECMA-376: chi lo omette produce un file
/// valido, e Excel stesso lo tratta come un suggerimento. Il driver
/// pretendeva invece che ci fosse, e sul file conforme rifiutava con «cella
/// XLSX fuori dalle dimensioni dichiarate» -- un rifiuto che nomina
/// dimensioni che nessuno ha dichiarato.
///
/// La controprova positiva sta nella stessa sonda: lo stesso contenuto con
/// il `ref` giusto dev'essere letto **identico**. Senza, «legge due righe»
/// sarebbe vero anche di un lettore che si e' inventato i limiti.
#[test]
fn n1_il_foglio_senza_dimension_si_legge_come_quello_che_la_dichiara() {
    let (righe_dichiarate, colonne_dichiarate) =
        apri_e_conta(&xlsx_con_dimension(Some("A1:B3")), "dichiarata.xlsx")
            .expect("il foglio che dichiara le proprie dimensioni si legge");
    assert_eq!(
        (righe_dichiarate, colonne_dichiarate),
        (2, 2),
        "due righe di dati e due colonne"
    );

    let (righe, colonne) = apri_e_conta(&xlsx_con_dimension(None), "senza.xlsx")
        .expect("`<dimension>` e' opzionale: il foglio che la omette e' conforme");
    assert_eq!(
        (righe, colonne),
        (righe_dichiarate, colonne_dichiarate),
        "lo stesso contenuto, letto uguale: i limiti si ricavano dalle celle"
    );
}

/// Una `<dimension>` **dichiarata** e piu' stretta delle celle resta un
/// rifiuto.
///
/// E' il confine della correzione, e va provato perche' e' il modo in cui
/// si sarebbe potuta allargare troppo: ricavare i limiti quando mancano non
/// e' ignorarli quando ci sono. Un file che dichiara `A1:A3` e scrive in
/// `B1` si contraddice, e il driver resta fail-closed.
#[test]
fn n1_una_dimension_dichiarata_e_violata_resta_un_rifiuto() {
    let Err(errore) = apri_e_conta(&xlsx_con_dimension(Some("A1:A3")), "stretta.xlsx") else {
        panic!(
            "una `<dimension>` dichiarata e contraddetta dalle celle deve fermare la \
                 lettura: ricavare i limiti assenti non e' ignorare quelli dichiarati"
        );
    };
    assert!(
        errore
            .message
            .contains("cella XLSX fuori dalle dimensioni dichiarate"),
        "atteso il rifiuto sulle dimensioni dichiarate, arrivato «{}»",
        errore.message
    );
}

/// Le quote valgono anche sui limiti **ricavati**.
///
/// Il foglio senza `<dimension>` non ha un numero di colonne da leggere
/// prima di aprirlo: se la scansione che li ricava non fosse soggetta alle
/// stesse quote, un file ostile potrebbe farsi allocare una riga larga
/// quanto vuole proprio omettendo l'elemento che il driver pretendeva.
#[test]
fn n1_le_quote_valgono_anche_sui_limiti_ricavati() {
    let byte = xlsx_con_dimension(None);
    let dir = tempfile::tempdir().unwrap();
    let percorso = dir.path().join("quota.xlsx");
    std::fs::write(&percorso, &byte).unwrap();
    let Err(errore) = XlsDriver.open(
        Source::Path(percorso),
        opzioni_lettura_wkt_con(
            plenora_io_model::budget::PipelineLimits::default().with_max_columns(1),
        ),
    ) else {
        panic!("due colonne ricavate oltre una quota di una devono essere rifiutate");
    };
    assert!(
        errore.message.contains("colonne oltre il limite"),
        "atteso il rifiuto sulla larghezza ricavata, arrivato «{}»",
        errore.message
    );
}
