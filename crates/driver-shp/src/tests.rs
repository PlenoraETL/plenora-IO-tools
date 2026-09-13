//! Le prove di [`super`], in un file loro.
//!
//! Restano un **modulo figlio**: vedono i privati del genitore
//! esattamente come quando stavano dentro di lui, e non allargano di
//! una riga la superficie pubblica del crate.

use super::*;

// --- il bundle del fuzzer, e i semi che lo alimentano -----------------
//
// Una build che compila e un replay senza crash non dimostrano che i semi
// raggiungano il parsing: un bundle rifiutato all'apertura non fa crashare
// niente ed e' indistinguibile, da fuori, da uno letto per intero. Queste
// sonde chiamano lo **stesso** entry point del target sui **semi
// committati**, e guardano che cosa ne esce.

/// I semi vivono accanto al target che li usa.
fn seme(nome: &str) -> Vec<u8> {
    let percorso = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fuzz/seeds/shp_reader")
        .join(nome);
    std::fs::read(&percorso)
        .unwrap_or_else(|errore| panic!("seme {} non leggibile: {errore}", percorso.display()))
}

/// Limiti dello stesso ordine di quelli della campagna.
///
/// Non sono gli **stessi**: `harness::limits()` vive nella crate di fuzzing,
/// e un driver non puo' dipenderne. Quel che conta e' che ci siano tetti --
/// una sonda che leggesse senza limiti percorrerebbe una strada che il
/// fuzzer non percorre mai -- e che siano stretti abbastanza da rendere
/// osservabile un input che li supera.
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

#[test]
fn il_seme_di_punti_arriva_alle_geometrie_e_agli_attributi() {
    let righe = __fuzz_leggi_bundle(
        &seme("punti-con-attributi.bundle"),
        opzioni_di_campagna().with_assume_crs("EPSG:4326"),
    )
    .expect("il seme deve essere letto: se no il target non copre il parsing");
    assert_eq!(
        righe, 2,
        "due punti e due record DBF: un conteggio diverso vuol dire che il \
             seme non attraversa piu' il drenaggio"
    );
}

/// La terza famiglia di geometria del formato.
///
/// Un multipunto dichiara il numero di punti e **non** l'indice delle
/// parti: nel driver percorre un ramo di prevalidazione che ne' il punto --
/// che non dichiara conteggi -- ne' la polilinea raggiungono. Senza un seme
/// che lo porti, quel ramo resterebbe scoperto e una delle tre famiglie del
/// formato non sarebbe esercitata dal target.
#[test]
fn il_seme_di_multipunto_arriva_al_drenaggio() {
    let righe = __fuzz_leggi_bundle(
        &seme("multipunto.bundle"),
        opzioni_di_campagna().with_assume_crs("EPSG:4326"),
    )
    .expect("il seme deve essere letto");
    assert_eq!(
        righe, 1,
        "una geometria multipunto e un record DBF: il conteggio dice che il \
             seme attraversa il drenaggio, non solo l'apertura"
    );

    // Il ramo della prevalidazione, chiamato direttamente: `SoloPunti` e'
    // una risposta diversa da `PartiEPunti`, e confondere i due tipi
    // renderebbe il limite sugli elementi sbagliato per entrambi.
    assert!(matches!(conteggi_attesi(8), ConteggiDelRecord::SoloPunti));
    for tag in [18, 28] {
        assert!(
            matches!(conteggi_attesi(tag), ConteggiDelRecord::SoloPunti),
            "il multipunto Z e M dichiarano gli stessi conteggi: {tag}"
        );
    }
    assert!(matches!(conteggi_attesi(3), ConteggiDelRecord::PartiEPunti));
    assert!(matches!(conteggi_attesi(1), ConteggiDelRecord::Nessuno));
}

/// Un valore ostile ferma la lettura **anche** in una riga cancellata.
///
/// Questa prova diceva il contrario, e la coppia di semi serviva a
/// dimostrarlo: stesso valore, un solo byte di differenza -- il marcatore --
/// ed esiti opposti. Reggeva su una premessa scritta e mai verificata,
/// «`dbase` salta i byte di una riga cancellata senza decodificarne un
/// campo». Non li salta: la fuzz smoke ha trovato una riga cancellata il cui
/// campo `D` fa panicare `Date::from_str`, attraversando l'apertura del
/// driver.
///
/// # Perche' uniforme, e che cosa costa
///
/// Panica il taglio fuori dai confini -- meno di otto byte utili -- e panica
/// un confine di carattere che cade dentro un multibyte. Il secondo dipende
/// da **dove** cade, e dove cada dipende da come `dbase` decodifica i byte
/// in stringa, cioe' da una scelta di codifica che non e' nostra. Il seme
/// multibyte qui sotto oggi non panica; un altro con l'accento spostato di
/// un byte lo farebbe.
///
/// Una regola giusta solo per le posizioni che abbiamo campionato non e' una
/// regola. La prevalidazione rifiuta quindi allo stesso modo nelle due
/// righe, e il costo va detto: un dataset con una data malformata in una
/// riga cancellata -- che prima veniva letto saltandola -- ora viene
/// rifiutato. E' il verso in cui si sbaglia meglio, perche' l'altro verso
/// non e' «leggere di piu'», e' un panico.
#[test]
fn una_data_ostile_ferma_la_lettura_anche_se_la_riga_e_cancellata() {
    for nome in [
        "dbf-data-multibyte.bundle",
        "dbf-data-multibyte-cancellata.bundle",
    ] {
        let errore = __fuzz_leggi_bundle(
            &seme(nome),
            opzioni_di_campagna().with_assume_crs("EPSG:4326"),
        )
        .expect_err("il marcatore di cancellazione non esenta il campo");
        assert_eq!(
            errore.message, "campo data DBF che il lettore non puo' interpretare",
            "{nome}"
        );
    }
}

#[test]
fn il_seme_di_polilinea_arriva_al_drenaggio() {
    let righe = __fuzz_leggi_bundle(
        &seme("polilinea.bundle"),
        opzioni_di_campagna().with_assume_crs("EPSG:4326"),
    )
    .expect("il seme deve essere letto");
    assert_eq!(righe, 1);
}

/// Conteggi disallineati **con** indice: il `.shx` risponde a
/// `shape_count()`, e il driver rifiuta all'apertura senza decodificare una
/// sola geometria. E' la difesa piu' economica del reader, e senza un seme
/// che porti l'indice sarebbe irraggiungibile da questo target.
#[test]
fn i_conteggi_disallineati_sono_rifiutati_all_apertura_quando_c_e_l_indice() {
    let errore = __fuzz_leggi_bundle(
        &seme("disallineati-con-indice.bundle"),
        opzioni_di_campagna().with_assume_crs("EPSG:4326"),
    )
    .expect_err("due forme e un solo record DBF non sono uno Shapefile coerente");
    assert_eq!(
            errore.message, "numero di geometrie diverso dal numero di record DBF",
            "il rifiuto deve venire dal confronto anticipato dei conteggi: se              cambia messaggio, il seme sta coprendo un altro ramo"
        );
}

/// Lo stesso disallineamento **senza** indice: `shape_count()` non risponde,
/// il confronto anticipato non avviene, e l'incoerenza emerge durante il
/// drenaggio. Due rami diversi dello stesso difetto del file — questo
/// attraversa il parsing delle geometrie, l'altro no.
#[test]
fn i_conteggi_disallineati_emergono_nel_drenaggio_senza_indice() {
    let errore = __fuzz_leggi_bundle(
        &seme("disallineati-senza-indice.bundle"),
        opzioni_di_campagna().with_assume_crs("EPSG:4326"),
    )
    .expect_err("l'incoerenza va rilevata anche senza `.shx`");
    assert_eq!(
            errore.message, "cardinalita' Shapefile cambiata durante la lettura",
            "senza indice il rifiuto deve arrivare dalla lettura, non              dall'apertura: {errore:?}"
        );
}

/// I due punti di arresto di `shapefile`, e la loro regressione.
///
/// La crate tratta i valori dichiarati nel file come se li avesse scritti
/// lei: raddoppia gli scostamenti dell'indice dentro un `i32`, e prenota un
/// vettore grande quanto il conteggio di punti dichiarato in un record,
/// senza legarlo alla dimensione del record. Il primo e' un panico, il
/// secondo un'allocazione che il processo non sopravvive -- e nessuno dei
/// due e' un `Err`.
#[test]
fn uno_scostamento_dell_indice_che_non_regge_il_raddoppio_e_un_errore() {
    let errore = __fuzz_leggi_bundle(
        &seme("shx-scostamento-traboccante.bundle"),
        opzioni_di_campagna().with_assume_crs("EPSG:4326"),
    )
    .expect_err("uno scostamento oltre meta' di i32::MAX non e' un indice");
    assert_eq!(
        errore.message,
        "voce dell'indice Shapefile fuori intervallo"
    );

    // Il secondo ramo: uno scostamento che regge il raddoppio e punta
    // comunque oltre la fine del `.shp`. Sono due difese diverse, e un seme
    // solo ne proverebbe una.
    let errore = __fuzz_leggi_bundle(
        &seme("shx-scostamento-fuori-dal-shp.bundle"),
        opzioni_di_campagna().with_assume_crs("EPSG:4326"),
    )
    .expect_err("un record che comincia oltre la fine del file non esiste");
    assert_eq!(
        errore.message,
        "voce dell'indice Shapefile che punta fuori dal .shp"
    );

    // Il terzo ramo, e il piu' insidioso: lo scostamento sta dentro il file
    // ma cade in mezzo al contenuto di un record, dove otto byte qualunque
    // diventano una testa. La catena sequenziale non lo vede, perche' il
    // lettore con indice non la percorre.
    let errore = __fuzz_leggi_bundle(
        &seme("shx-scostamento-dentro-un-record.bundle"),
        opzioni_di_campagna().with_assume_crs("EPSG:4326"),
    )
    .expect_err("un indice che punta dentro un record non e' un indice");
    assert_eq!(
        errore.message,
        "lunghezza del record diversa da quella dichiarata nell'indice"
    );
}

// --- il record nullo sovradimensionato, e il raddoppio non controllato ---
//
// Due difetti nello stesso reperto, e conviene tenerli distinti perche' si
// riparano in due punti diversi del fork.
//
// Il **primo** e' la desincronizzazione. `Shape::read_from` consumava, per
// un record di tipo `NullShape`, i soli quattro byte del tag e tornava
// ignorando `record_size`: ogni altro ramo consuma il contenuto dichiarato,
// quello no. Un record nullo che ne dichiari di piu' lasciava il resto
// nello stream, e l'iteratore leggeva la testa del record successivo da
// dentro il contenuto di questo.
//
// Il **secondo** e' che otto byte qualunque, letti come una testa, danno un
// `record_size` che `read_one_shape_as` raddoppiava dentro un `i32` senza
// controllo. Sotto `overflow-checks` e' il panico che la campagna ha
// archiviato; senza, e' un avvolgimento silenzioso e la lettura prosegue su
// una lunghezza che nessuno ha scritto -- il caso peggiore dei due.
//
// Il primo difetto e' cio' che **produce** l'input del secondo, ma il
// secondo esiste anche da solo: un file puo' dichiarare una lunghezza
// enorme in una testa di record perfettamente allineata. Per questo le
// prove sono separate, e una arriva alla moltiplicazione senza passare da
// un record nullo.

/// Uno `.shp` costruito a mano, con l'intestazione da cento byte.
///
/// I record arrivano gia' formati perche' le sonde qui sotto hanno bisogno
/// di scriverne alcuni che nessun writer produrrebbe.
fn shp_con_record(tipo_dichiarato: i32, record: &[u8]) -> Vec<u8> {
    let mut file = vec![0_u8; SHP_HEADER_SIZE];
    file[0..4].copy_from_slice(&9994_i32.to_be_bytes());
    // Byte prima, parole poi: la divisione converte un'unita' nell'altra,
    // e scriverla su una somma la farebbe sembrare una media.
    let byte = SHP_HEADER_SIZE + record.len();
    let parole = i32::try_from(byte / 2).expect("il file di una sonda sta in un i32");
    file[24..28].copy_from_slice(&parole.to_be_bytes());
    file[28..32].copy_from_slice(&1000_i32.to_le_bytes());
    file[32..36].copy_from_slice(&tipo_dichiarato.to_le_bytes());
    for (indice, valore) in [0.0_f64, 0.0, 10.0, 10.0].into_iter().enumerate() {
        let inizio = 36 + indice * 8;
        file[inizio..inizio + 8].copy_from_slice(&valore.to_le_bytes());
    }
    file.extend_from_slice(record);
    file
}

/// Una testa di record: numero e lunghezza del contenuto, in parole.
fn testa_di_record(numero: i32, parole: i32) -> Vec<u8> {
    let mut testa = numero.to_be_bytes().to_vec();
    testa.extend_from_slice(&parole.to_be_bytes());
    testa
}

/// Il contenuto di un punto: il tag e due coordinate.
fn contenuto_di_punto(x: f64, y: f64) -> Vec<u8> {
    let mut contenuto = 1_i32.to_le_bytes().to_vec();
    contenuto.extend_from_slice(&x.to_le_bytes());
    contenuto.extend_from_slice(&y.to_le_bytes());
    contenuto
}

fn forme_del_fork(shp: &[u8]) -> std::result::Result<Vec<Shape>, shapefile::Error> {
    shapefile::ShapeReader::new(std::io::Cursor::new(shp.to_vec()))?.read()
}

/// L'errore, o il fallimento della sonda.
///
/// `expect_err` non si puo' usare: `Vec<Shape>` non implementa `Debug`, e
/// il `Ok` va comunque nominato -- una lettura riuscita e' **il** difetto
/// che queste sonde cercano, non un caso da lasciar passare in silenzio.
fn errore_del_fork(shp: &[u8], perche: &str) -> shapefile::Error {
    match forme_del_fork(shp) {
        Err(errore) => errore,
        Ok(forme) => panic!("{perche}: invece ne sono uscite {} forme", forme.len()),
    }
}

/// Il fork rifiuta un record nullo che dichiari piu' del proprio tag.
///
/// E' la correzione alla radice, verificata **sul fork** e non attraverso
/// il driver: la difesa deve valere per ogni chiamante del reader, non solo
/// per quello che passa dal nostro validatore.
/// Il residuo di un record nullo che, letto come una testa, e' **valido**.
///
/// Serve a separare le due difese. Sul reperto della campagna il residuo
/// da' una lunghezza che non regge il raddoppio, quindi il controllo sulla
/// moltiplicazione lo ferma **anche** senza quello sul record nullo: le
/// due difese si coprono a vicenda, e una sonda costruita su quei byte
/// resta verde con la seconda rimossa: non proverebbe cio' che dice di
/// provare, e l'ho misurato disattivando la verifica.
///
/// Qui il residuo e' una testa ben formata seguita dal tag nullo che quella
/// testa promette, e il file finisce li'. Con la verifica e' un rifiuto;
/// senza, la lettura termina con **`Ok`** — che e' il difetto nella sua
/// forma piu' pericolosa: non un panico, ma un file letto per meta' e
/// consegnato come intero.
fn record_nullo_col_residuo_che_finge_una_testa() -> Vec<u8> {
    // Otto parole dichiarate, sedici byte di contenuto.
    let mut record = testa_di_record(1, 8);
    record.extend_from_slice(&0_i32.to_le_bytes());
    record.extend_from_slice(&testa_di_record(2, 2));
    record.extend_from_slice(&0_i32.to_le_bytes());
    record
}

#[test]
fn il_fork_rifiuta_un_record_nullo_sovradimensionato() {
    let errore = errore_del_fork(
        &shp_con_record(1, &record_nullo_col_residuo_che_finge_una_testa()),
        "un record nullo di sedici byte non e' un record nullo",
    );
    assert!(
        matches!(errore, shapefile::Error::InvalidShapeRecordSize),
        "il rifiuto deve nominare la lunghezza del record, non un altro \
             difetto raggiunto per caso: {errore:?}"
    );
}

/// Un record nullo conforme non sposta quello dopo di se'.
///
/// E' la meta' che rende la correzione una riparazione invece che una
/// chiusura: senza, «rifiuta il nullo sovradimensionato» sarebbe vero anche
/// di un reader che avesse smesso di leggere i record nulli.
#[test]
fn un_record_nullo_conforme_non_disallinea_quello_dopo() {
    let mut record = testa_di_record(1, 2);
    record.extend_from_slice(&0_i32.to_le_bytes());
    record.extend_from_slice(&testa_di_record(2, 10));
    record.extend_from_slice(&contenuto_di_punto(3.0, 4.0));

    let forme = forme_del_fork(&shp_con_record(1, &record))
        .expect("un nullo di quattro byte seguito da un punto e' leggibile");
    let tipi: Vec<_> = forme.iter().map(Shape::shapetype).collect();
    assert_eq!(forme.len(), 2, "due record, due forme: {tipi:?}");
    assert!(
        matches!(forme[0], Shape::NullShape),
        "il primo record e' nullo, e va letto come tale: {}",
        forme[0].shapetype()
    );
    let Shape::Point(punto) = &forme[1] else {
        panic!(
            "il secondo record e' un punto, e va letto come tale: {}",
            forme[1].shapetype()
        );
    };
    // Le coordinate, e non il solo tipo: un disallineamento di sedici byte
    // puo' produrre un punto **valido** con dentro i byte sbagliati, ed e'
    // esattamente cio' che questa sonda deve poter distinguere.
    assert!(
        (punto.x - 3.0).abs() < f64::EPSILON && (punto.y - 4.0).abs() < f64::EPSILON,
        "il punto deve arrivare intero: {punto:?}"
    );
}

/// La moltiplicazione, coperta senza passare da un record nullo.
///
/// `i32::MAX` parole non traboccherebbero se il raddoppio avvenisse in
/// `i64`; traboccano in `i32`, che e' il tipo in cui il reader lavora. La
/// sonda non passa dal validatore del prodotto -- costruisce lo `.shp` e lo
/// da' al fork -- perche' la difesa deve stare nel reader anche quando a
/// chiamarlo e' qualcun altro.
#[test]
fn il_fork_rifiuta_una_lunghezza_di_record_che_non_regge_il_raddoppio() {
    for parole in [i32::MAX, i32::MAX / 2 + 1, -1] {
        let mut record = testa_di_record(1, parole);
        record.extend_from_slice(&contenuto_di_punto(1.0, 2.0));

        let errore = errore_del_fork(
            &shp_con_record(1, &record),
            "una lunghezza fuori intervallo non e' un record",
        );
        assert!(
            matches!(errore, shapefile::Error::InvalidShapeRecordSize),
            "la lunghezza va rifiutata prima del raddoppio: parole={parole}, errore={errore:?}"
        );
    }
}

/// La controprova della verifica sul record nullo, sul suo terreno.
///
/// La sonda qui sopra dice che il file e' rifiutato. Questa dice **da che
/// cosa**, e conviene guardare i due contatori che il reader tiene, perche'
/// e' dove il difetto vive.
///
/// `ShapeIterator` avanza `current_pos` della lunghezza **dichiarata** dal
/// record, mentre lo stream avanza di quanto il ramo ha davvero consumato.
/// Con la semantica vecchia i due divergono: il record dichiara sedici byte
/// di contenuto, il ramo nullo ne consumava quattro, e restano dodici byte
/// di scarto. Qui il file finisce li', quindi `current_pos` raggiunge la
/// fine e la lettura termina con **`Ok`** e una forma — misurato
/// disattivando la verifica: la crate ritorna un file letto per meta'
/// dichiarandolo intero. Nel reperto della campagna, dove un secondo record
/// segue, lo stesso scarto porta la lettura dentro il contenuto, ed e'
/// l'altra controprova.
///
/// E' anche la ragione per cui la verifica non poteva limitarsi a saltare
/// il residuo. Saltarlo avrebbe riallineato i due contatori accettando un
/// file il cui contenuto e la cui lunghezza dichiarata non possono essere
/// veri insieme.
#[test]
fn senza_la_verifica_il_reader_accetta_un_record_nullo_che_non_ha_letto() {
    let shp = shp_con_record(1, &record_nullo_col_residuo_che_finge_una_testa());

    let dichiarate = i32::from_be_bytes([shp[24], shp[25], shp[26], shp[27]]);
    let byte_dichiarati =
        usize::try_from(dichiarate).expect("la lunghezza di una sonda e' positiva") * 2;
    let testa = &shp[SHP_HEADER_SIZE..SHP_HEADER_SIZE + 8];
    let byte_del_record =
        usize::try_from(i32::from_be_bytes([testa[4], testa[5], testa[6], testa[7]]))
            .expect("la lunghezza di una sonda e' positiva")
            * 2;

    // Il tag e nient'altro: quanto il vecchio ramo consumava.
    let consumati = std::mem::size_of::<i32>();
    assert_eq!(
        byte_del_record - consumati,
        12,
        "lo scarto fra dichiarato e consumato e' il difetto, e senza di \
             esso questa sonda non prova niente"
    );
    assert_eq!(
        SHP_HEADER_SIZE + 8 + byte_del_record,
        byte_dichiarati,
        "il file finisce col primo record: e' cio' che fa terminare la \
             lettura con Ok invece che dentro il contenuto"
    );

    // E lo stesso file, dato al fork corretto, e' un rifiuto.
    let errore = errore_del_fork(&shp, "il file non e' leggibile");
    assert!(
        matches!(errore, shapefile::Error::InvalidShapeRecordSize),
        "{errore:?}"
    );
}

/// La controprova dell'altra verifica: senza il raddoppio controllato, il
/// reperto della campagna trabocca.
///
/// Le due sonde qui sopra dicono che il reader **oggi** rifiuta. Non
/// dicono che a rifiutare sia la verifica aggiunta: lo direbbero anche se
/// l'input fosse fermato prima, da un'altra difesa, e la riga nuova fosse
/// morta.
///
/// Questa sonda ripercorre i byte del reperto con la semantica **vecchia**
/// del ramo nullo -- consuma il tag e basta -- e mostra dove finisce: la
/// testa successiva viene letta sedici byte troppo presto, e la lunghezza
/// che ne esce non regge il raddoppio in `i32`. E' il difetto, riprodotto
/// dagli stessi byte che il fork corretto rifiuta.
#[test]
fn senza_la_verifica_il_record_nullo_disallinea_e_il_raddoppio_trabocca() {
    let bundle = seme("shp-nullo-sovradimensionato.bundle");
    let parti = __fuzz_dividi_bundle(&bundle).expect("il reperto e' un bundle");
    let shp = parti.shp;

    // La catena, letta come la leggeva il ramo nullo: quattro byte di tag,
    // e avanti.
    let mut posizione = SHP_HEADER_SIZE;
    let testa = &shp[posizione..posizione + 8];
    let parole = i32::from_be_bytes([testa[4], testa[5], testa[6], testa[7]]);
    let tipo = i32::from_le_bytes([
        shp[posizione + 8],
        shp[posizione + 9],
        shp[posizione + 10],
        shp[posizione + 11],
    ]);
    assert_eq!(tipo, 0, "il primo record del reperto e' un nullo");
    assert!(
        parole * 2 > 4,
        "e dichiara piu' del proprio tag: e' la condizione del difetto"
    );

    // Il vecchio ramo consumava il tag e tornava: l'iteratore avanzava di
    // otto byte di testa piu' i quattro consumati, non di quanto il record
    // dichiarava.
    posizione += 8 + 4;
    let testa = &shp[posizione..posizione + 8];
    let parole_lette = i32::from_be_bytes([testa[4], testa[5], testa[6], testa[7]]);
    assert!(
        parole_lette.checked_mul(2).is_none(),
        "letta da dentro il contenuto, la testa successiva deve dare una \
             lunghezza che non regge il raddoppio -- e' il panico che la \
             campagna ha archiviato. Se questa asserzione cade, il reperto non \
             riproduce piu' il difetto e la sonda non lo sta piu' provando: \
             parole_lette={parole_lette}"
    );

    // E lo stesso reperto, dato al fork corretto, e' un rifiuto tipizzato.
    let errore = errore_del_fork(shp, "il reperto non e' leggibile");
    assert!(
        matches!(errore, shapefile::Error::InvalidShapeRecordSize),
        "{errore:?}"
    );
}

/// Il reperto della campagna, fino in fondo al driver.
///
/// Le sonde sul fork provano la difesa dove sta; questa prova che il
/// prodotto la **raggiunge** e la traduce, invece di cadere prima o di
/// consegnare righe. E' lo stesso entry point del target `shp_reader`, sugli
/// stessi byte che la campagna ha archiviato.
#[test]
fn il_reperto_del_nullo_sovradimensionato_e_un_rifiuto_tipizzato() {
    let errore = __fuzz_leggi_bundle(
        &seme("shp-nullo-sovradimensionato.bundle"),
        opzioni_di_campagna().with_assume_crs("EPSG:4326"),
    )
    .expect_err("un record nullo che dichiara venti byte non e' leggibile");
    // Codice, categoria e fase invece del testo: un messaggio si riscrive
    // senza accorgersene, e una sonda che lo insegue smette di dire che il
    // rifiuto e' **quello** e comincia a dire com'e' scritto.
    assert_eq!(
        errore.category,
        plenora_io_model::ErrorCategory::DataMapping,
        "{errore:?}"
    );
    assert_eq!(
        errore.phase,
        plenora_io_model::ErrorPhase::Read,
        "{errore:?}"
    );
}

#[test]
fn un_record_che_dichiara_piu_punti_di_quanti_ne_contenga_e_un_errore() {
    for nome in ["shp-punti-assurdi.bundle", "shp-punti-negativi.bundle"] {
        let errore = __fuzz_leggi_bundle(
            &seme(nome),
            opzioni_di_campagna().with_assume_crs("EPSG:4326"),
        )
        .expect_err("un conteggio slegato dal record non e' leggibile");
        assert!(
            errore.message.contains("conteggio negativo")
                || errore
                    .message
                    .contains("piu' elementi di quanti ne contenga"),
            "{nome}: {errore:?}"
        );
    }
}

/// Un record che dichiara un tipo con conteggi e non ha spazio per
/// portarli.
///
/// `read_shape_content` riceve la dimensione del record ma legge dal
/// flusso: i conteggi finiscono per venire dai byte che seguono, e il
/// vettore prenotato e' grande quanto quel numero. Non e' un panico ma una
/// richiesta di memoria che il processo non sopravvive -- e per il fuzzer
/// e' un finding come gli altri.
#[test]
fn un_record_troppo_corto_per_il_proprio_tipo_e_un_errore() {
    let errore = __fuzz_leggi_bundle(
        &seme("shp-record-troppo-corto.bundle"),
        opzioni_di_campagna().with_assume_crs("EPSG:4326"),
    )
    .expect_err("una polilinea di quattro byte non porta i propri conteggi");
    assert_eq!(
        errore.message,
        "record Shapefile troppo corto per il tipo che dichiara"
    );
}

/// L'indice delle parti, nei due modi in cui esce dai punti dichiarati.
///
/// Il lettore prende la differenza fra due voci consecutive come numero di
/// punti da leggere: una voce che scende la rende negativa, una che sale
/// oltre il numero di punti la gonfia. Nel primo caso c'e' un
/// `debug_assert!` -- panico sotto il fuzzer, niente in release -- nel
/// secondo nemmeno quello.
#[test]
fn un_indice_delle_parti_fuori_dai_punti_e_un_errore() {
    for nome in [
        "shp-parti-che-scendono.bundle",
        "shp-parti-oltre-i-punti.bundle",
    ] {
        let errore = __fuzz_leggi_bundle(
            &seme(nome),
            opzioni_di_campagna().with_assume_crs("EPSG:4326"),
        )
        .expect_err("una parte che comincia fuori dai punti non esiste");
        assert_eq!(
            errore.message, "indice delle parti Shapefile che esce dai punti dichiarati",
            "{nome}"
        );
    }
}

/// Il campo `T`, che porta due interi binari invece di otto cifre.
///
/// `julian_day_number_to_gregorian_date` lavora in `i32` e comincia con
/// `4 * jdn + 274_277`; `Time::from_word` divide e rimoltiplica passando da
/// `u32`, dove un parola-tempo negativo diventa enorme. Due traboccamenti
/// distinti, e due semi.
#[test]
fn un_campo_data_e_ora_fuori_intervallo_e_un_errore() {
    for nome in [
        "dbf-giorno-giuliano-enorme.bundle",
        "dbf-parola-tempo-negativa.bundle",
    ] {
        let errore = __fuzz_leggi_bundle(
            &seme(nome),
            opzioni_di_campagna().with_assume_crs("EPSG:4326"),
        )
        .expect_err("un istante che il lettore non sa convertire non e' leggibile");
        assert_eq!(
            errore.message, "campo data-e-ora DBF fuori dall'intervallo convertibile",
            "{nome}"
        );
    }
}

/// Il rovescio: un istante reale passa, e cosi' i due estremi ammessi.
#[test]
fn un_istante_reale_passa() {
    let istante = |giorno: i32, ora: i32| {
        let mut byte = [0_u8; 8];
        byte[..4].copy_from_slice(&giorno.to_le_bytes());
        byte[4..].copy_from_slice(&ora.to_le_bytes());
        data_e_ora_non_convertibili(&byte)
    };
    assert!(
        !istante(2_458_685, 43_200_000),
        "mezzogiorno del 2019-07-11"
    );
    assert!(!istante(0, 0));
    assert!(!istante(
        DBF_MASSIMO_GIORNO_GIULIANO,
        DBF_MILLISECONDI_DEL_GIORNO - 1
    ));

    assert!(istante(DBF_MASSIMO_GIORNO_GIULIANO + 1, 0));
    assert!(istante(-1, 0));
    assert!(istante(0, DBF_MILLISECONDI_DEL_GIORNO));
    assert!(istante(0, -1));
}

/// Un record lungo **esattamente** la propria testa, che dichiara una
/// parte.
///
/// E' il caso che ha mostrato un difetto in questa stessa verifica: il
/// confronto guardava solo gli elementi, non la testa che li precede, e i
/// quattro byte dell'indice delle parti venivano letti **oltre** il record.
/// La posizione nel file restava indietro, e da li' in poi la catena dei
/// record veniva letta sfasata.
#[test]
fn un_record_lungo_quanto_la_propria_testa_non_puo_dichiarare_parti() {
    let errore = __fuzz_leggi_bundle(
        &seme("shp-parti-oltre-la-testa.bundle"),
        opzioni_di_campagna().with_assume_crs("EPSG:4326"),
    )
    .expect_err("quarantaquattro byte non portano testa e indice delle parti");
    assert_eq!(
        errore.message,
        "record Shapefile che dichiara piu' elementi di quanti ne contenga"
    );
}

/// Un descrittore che dichiara meno byte di quanti il suo tipo ne legga.
///
/// `dbase` dichiara la dimensione fissa dei propri tipi e non la verifica:
/// affetta comunque, e la fetta esce dal campo. Sono quattro tipi -- `L`,
/// `I`, `Y`, `B`, `T` -- e la sonda ne prova uno per il seme e tutti per la
/// funzione, perche' un elenco incompleto qui sarebbe indistinguibile da
/// uno completo.
#[test]
fn un_campo_piu_corto_del_proprio_tipo_e_un_errore() {
    let errore = __fuzz_leggi_bundle(
        &seme("dbf-campo-corto.bundle"),
        opzioni_di_campagna().with_assume_crs("EPSG:4326"),
    )
    .expect_err("un intero largo due byte non e' un intero");
    assert_eq!(
        errore.message,
        "campo DBF piu' corto di quanto il suo tipo pretenda"
    );

    for (tipo, minima) in [
        (b'L', 1),
        (b'I', 4),
        (b'D', 8),
        (b'Y', 8),
        (b'B', 8),
        (b'T', 8),
    ] {
        assert_eq!(
            lunghezza_minima_del_campo(tipo),
            Some(minima),
            "tipo {}",
            char::from(tipo)
        );
    }
    // I tipi testuali non hanno una larghezza imposta: `C` e `N` la
    // dichiarano, e il loro contenuto e' un errore di parsing, non un
    // panico.
    assert_eq!(lunghezza_minima_del_campo(b'C'), None);
    assert_eq!(lunghezza_minima_del_campo(b'N'), None);
}

/// La verifica strutturale non deve rifiutare cio' che il decoder
/// accetterebbe: e' la meta' che una prevalidazione sbaglia piu' spesso, e
/// che nessun seme ostile mostrerebbe.
#[test]
fn i_semi_validi_passano_la_verifica_strutturale() {
    let temporanea = tempfile::tempdir().expect("directory temporanea");
    for nome in [
        "punti-con-attributi.bundle",
        "punti-con-prj.bundle",
        "polilinea.bundle",
    ] {
        let dati = seme(nome);
        let parti = __fuzz_dividi_bundle(&dati).expect("il seme e' un bundle");
        let radice = temporanea.path().join(nome);
        std::fs::create_dir(&radice).expect("directory del seme");
        let principale = materializza_bundle(&radice, &parti).expect("materializzazione");
        assert_eq!(valida_struttura_shp(&principale), Ok(()), "{nome}");
    }
}

/// Il primo finding del target, e la sua regressione.
///
/// `dbase::File::open` ricavava il numero di campi da
/// `offset_to_first_record` con una sottrazione non controllata: sotto la
/// soglia il processo **panicava** invece di restituire un errore. Un
/// panico attraversa il confine della libreria, e sotto `libfuzzer-sys`
/// diventa un abort che nessun `catch_unwind` vede.
///
/// Le due sonde tengono i due rami distinti: l'offset corto e il file
/// dichiarato Visual `FoxPro` con meno byte del backlink. Chiuderne uno solo
/// avrebbe lasciato l'altro raggiungibile.
#[test]
fn un_offset_del_primo_record_troppo_corto_e_un_errore_non_un_panico() {
    let errore = __fuzz_leggi_bundle(
        &seme("dbf-offset-corto.bundle"),
        opzioni_di_campagna().with_assume_crs("EPSG:4326"),
    )
    .expect_err("un offset sotto l'intestazione non e' un DBF leggibile");
    assert_eq!(
        errore.message,
        "offset del primo record DBF piu' corto dell'intestazione"
    );
}

#[test]
fn un_visual_foxpro_piu_corto_del_backlink_e_un_errore_non_un_panico() {
    let errore = __fuzz_leggi_bundle(
        &seme("dbf-visual-foxpro-corto.bundle"),
        opzioni_di_campagna().with_assume_crs("EPSG:4326"),
    )
    .expect_err("263 byte di backlink non stanno in 100");
    assert_eq!(
        errore.message, "header Visual FoxPro piu' corto del backlink",
        "il rifiuto deve venire dal ramo Visual FoxPro: {errore:?}"
    );
}

/// Il terzo punto di arresto: il terminatore dei descrittori.
///
/// `dbase` lo pretende con un `debug_assert_eq!`, quindi panica sotto il
/// fuzzer e **non** in release, dove il file verrebbe letto come se il
/// terminatore ci fosse. Rifiutarlo rende l'esito lo stesso nelle due
/// configurazioni, che e' meta' del valore della correzione.
#[test]
fn un_terminatore_dell_header_non_valido_e_un_errore_non_un_panico() {
    let errore = __fuzz_leggi_bundle(
        &seme("dbf-terminatore-non-valido.bundle"),
        opzioni_di_campagna().with_assume_crs("EPSG:4326"),
    )
    .expect_err("un DBF senza terminatore non e' leggibile");
    assert_eq!(errore.message, "terminatore header DBF non valido");
}

/// Il valore di un campo data: l'unico contenuto -- non descrittore -- che
/// puo' far panicare il lettore.
///
/// `Date::from_str` affetta la stringa a byte senza guardare ne' la
/// lunghezza ne' i confini di carattere. Sono due modi di uscirne, e
/// servono due semi: un valore multibyte e uno piu' corto di otto byte
/// utili.
#[test]
fn un_campo_data_non_interpretabile_e_un_errore_non_un_panico() {
    for nome in ["dbf-data-multibyte.bundle", "dbf-data-corta.bundle"] {
        let errore = __fuzz_leggi_bundle(
            &seme(nome),
            opzioni_di_campagna().with_assume_crs("EPSG:4326"),
        )
        .expect_err("una data che il lettore non sa affettare non e' leggibile");
        assert_eq!(
            errore.message, "campo data DBF che il lettore non puo' interpretare",
            "{nome}"
        );
    }
}

/// Una riga **cancellata** porta gli stessi panici di una viva.
///
/// La prevalidazione le saltava, sulla premessa scritta che «un record
/// cancellato non viene letto: `dbase` salta i suoi byte senza decodificarne
/// un solo campo». La premessa era falsa e non era mai stata verificata: la
/// fuzz smoke ha trovato un `.dbf` il cui unico record e' marcato `*`, e il
/// panico arriva lo stesso attraversando l'apertura del driver.
///
/// Il seme differisce da `dbf-data-corta.bundle` per **un byte** -- lo
/// spazio iniziale del record diventa `*` -- e cosi' la prova dice quale
/// proprieta' sta misurando: non «una data corta e' rifiutata», che si sa
/// gia', ma «il marcatore di cancellazione non compra l'esenzione».
#[test]
fn una_data_malformata_in_una_riga_cancellata_e_un_errore_non_un_panico() {
    let errore = __fuzz_leggi_bundle(
        &seme("dbf-data-corta-in-riga-cancellata.bundle"),
        opzioni_di_campagna().with_assume_crs("EPSG:4326"),
    )
    .expect_err("una riga cancellata non esenta il campo dal panico");
    assert_eq!(
        errore.message,
        "campo data DBF che il lettore non puo' interpretare"
    );
}

/// Il campo di controllo, fino in fondo al driver, vivo e cancellato.
///
/// Le sonde sulla condizione dicono che la difesa rifiuta quei byte; questa
/// dice che il prodotto la **raggiunge** e la traduce, invece di panicare
/// prima. E' lo stesso entry point del target `shp_reader`.
///
/// Nessuno dei due semi porta una coda oltre i record dichiarati: con una,
/// a fermarli sarebbe `valida_fine_del_dbf`, che interviene prima, e questa
/// sonda direbbe di provare una difesa mentre ne prova un'altra.
///
/// I due semi stanno insieme perche' misurano due proprieta' diverse: che
/// il campo sia rifiutato, e che il marcatore di cancellazione non compri
/// l'esenzione. Con un seme solo non si vedrebbe da quale delle due
/// dipenda il rifiuto.
#[test]
fn un_campo_data_di_soli_byte_di_controllo_e_un_rifiuto_tipizzato() {
    for nome in [
        "dbf-data-di-controllo.bundle",
        "dbf-data-di-controllo-cancellata.bundle",
    ] {
        let errore = __fuzz_leggi_bundle(
            &seme(nome),
            opzioni_di_campagna().with_assume_crs("EPSG:4326"),
        )
        .expect_err("un campo di soli byte di controllo non e' una data");
        assert_eq!(
            errore.message, "campo data DBF che il lettore non puo' interpretare",
            "il seme «{nome}» deve fermarsi sulla difesa del campo data"
        );
        assert_eq!(
            errore.category,
            plenora_io_model::ErrorCategory::DataMapping
        );
        assert_eq!(errore.phase, plenora_io_model::ErrorPhase::Read);
    }
}

/// La controprova: senza la difesa, quei byte fanno panicare `dbase`.
///
/// Le sonde qui sopra dicono che il prodotto **oggi** rifiuta. Non dicono
/// che senza la difesa panicherebbe: lo direbbero anche se `dbase` avesse
/// imparato a restituire un errore, e la nostra riga fosse diventata
/// inutile.
///
/// Qui il DBF del seme viene dato **direttamente** alla crate, saltando il
/// driver, dentro un `catch_unwind`. E' il difetto nella sua forma nuda: un
/// panico, non un `Err`, in una libreria che sta fra noi e il file.
///
/// Il seme e' quello con il record oltre il conteggio, e la ragione e'
/// misurata: un campo di controllo da solo **non** fa panicare `dbase`, che
/// su di esso restituisce un `Err(InvalidDigit)` dal `?` di
/// `s[0..4].parse()`. A panicare e' un campo che si tronca a **tre**
/// caratteri, dove gia' `s[0..4]` esce dall'intervallo -- e per arrivarci
/// serve un record che l'header non dichiara, raggiunto perche' un
/// cancellato ha spostato la finestra.
///
/// L'hook di panico resta quello di default: sopprimerlo qui zittirebbe
/// anche il panico di un altro test in parallelo, e un messaggio in piu'
/// nel log costa meno di un fallimento altrui reso muto.
#[test]
fn senza_la_difesa_dbase_accetta_quei_byte_senza_dire_niente() {
    let bundle = seme("dbf-record-oltre-il-conteggio.bundle");
    let parti = __fuzz_dividi_bundle(&bundle).expect("il seme e' un bundle");
    let dbf = parti.dbf.to_vec();

    let temporanea = tempfile::tempdir().expect("directory temporanea");
    let percorso = temporanea.path().join("solo.dbf");
    std::fs::write(&percorso, &dbf).expect("il DBF si scrive");

    // Riscritta il 2026-09-10, aggiornando il fork a shapefile 0.9.0 e
    // con esso `dbase` da 0.5.0 a 0.8.0.
    //
    // Diceva «senza la difesa questi byte fanno panicare `dbase`», e
    // avvertiva che il giorno in cui la crate avesse restituito un
    // `Err` la controprova sarebbe caduta e la difesa andava
    // riconsiderata invece che tenuta per abitudine. Quel giorno e'
    // arrivato, e l'esito e' **peggiore** di un `Err`: la 0.8.0 non
    // panica e non rifiuta, restituisce `Ok` con zero record. Cioe'
    // presenta come un file vuoto dei byte che non lo sono.
    //
    // La difesa quindi non e' meno necessaria di prima: e' piu'
    // necessaria. Prima intercettava un panico -- rumoroso, e comunque
    // catturato dalla barriera; ora intercetta un silenzio, che nessuna
    // barriera puo' vedere.
    let esito = std::panic::catch_unwind(|| {
        let mut lettore = shapefile::dbase::Reader::from_path(&percorso).expect("il DBF si apre");
        lettore.read()
    });
    let letti = esito
        .expect("`dbase` 0.8.0 non panica piu' su questi byte")
        .expect("e non li rifiuta nemmeno");
    assert!(
        letti.is_empty(),
        "senza la difesa del driver `dbase` non dice niente di questi byte: \
             ne restituisce zero record, cioe' li presenta come un file vuoto. \
             Se un giorno li rifiutasse con un `Err`, questa controprova cade \
             e la difesa va riconsiderata invece che tenuta per abitudine"
    );

    // E lo stesso file, attraverso il driver, e' un rifiuto: la difesa sta
    // fra i due, ed e' cio' che fa la differenza.
    let errore = __fuzz_leggi_bundle(&bundle, opzioni_di_campagna().with_assume_crs("EPSG:4326"))
        .expect_err("il driver rifiuta cio' che farebbe panicare la crate");
    assert_eq!(
        errore.message,
        "byte DBF oltre i record dichiarati dall'header"
    );
}

/// I byte oltre i record dichiarati sono rifiutati prima di `dbase`.
///
/// La difesa sul campo data guarda i record che l'header dichiara, ed e'
/// giusto: sono quelli che il file afferma di contenere. `dbase` pero' ne
/// legge altri -- davanti a un record cancellato fa `continue` senza
/// incrementare il proprio contatore, e la finestra scorre in avanti di uno
/// -- e su quei byte nessuno aveva niente da dire.
///
/// La controprova sta nel seme accanto: lo **stesso** file senza la coda si
/// legge. Senza, «i byte oltre il conteggio sono rifiutati» sarebbe vero
/// anche di un driver che rifiuta ogni record cancellato.
#[test]
fn i_byte_oltre_i_record_dichiarati_sono_rifiutati() {
    let errore = __fuzz_leggi_bundle(
        &seme("dbf-record-oltre-il-conteggio.bundle"),
        opzioni_di_campagna().with_assume_crs("EPSG:4326"),
    )
    .expect_err("oltre i record dichiarati non c'e' niente da leggere");
    assert_eq!(
        errore.message,
        "byte DBF oltre i record dichiarati dall'header"
    );
    assert_eq!(
        errore.category,
        plenora_io_model::ErrorCategory::DataMapping
    );

    // Lo stesso file senza la coda: un record cancellato non e' un difetto.
    let righe = match __fuzz_leggi_bundle(
        &seme("dbf-cancellato-senza-coda.bundle"),
        opzioni_di_campagna().with_assume_crs("EPSG:4326"),
    ) {
        Ok(righe) => righe,
        Err(errore) => panic!("un record cancellato si legge: {errore:?}"),
    };
    assert_eq!(righe, 0, "l'unico record e' cancellato, e non esce");
}

/// La fine del file, nelle due forme che il formato ammette.
///
/// Il terminatore `0x1A` c'e' o non c'e': la specifica lo prevede, non
/// tutti i produttori lo scrivono, e un file che finisce esattamente
/// sull'ultimo record e' altrettanto valido. Le due fixture `.dbf` di
/// questo repository lo portano, ed e' l'**unico** byte che segue i loro
/// record: la coda ammessa e' stata caratterizzata su di loro prima di
/// fissarla.
///
/// Un byte diverso non e' ammesso, e non e' pignoleria: distinguere «un
/// byte innocuo» da «il primo byte di un record» non si puo' fare
/// guardandone uno solo.
#[test]
fn la_fine_del_dbf_ammette_il_terminatore_e_niente_altro() {
    let temporanea = tempfile::tempdir().expect("directory temporanea");
    let bundle = seme("dbf-data-valida.bundle");
    let parti = __fuzz_dividi_bundle(&bundle).expect("il seme e' un bundle");
    let intero = parti.dbf.to_vec();
    assert_eq!(
        intero.last(),
        Some(&0x1A),
        "il seme di partenza porta il terminatore"
    );

    let prova = |coda: &[u8], atteso: std::result::Result<(), &str>| {
        let mut dbf = intero[..intero.len() - 1].to_vec();
        dbf.extend_from_slice(coda);
        let percorso = temporanea.path().join(format!("c{}.dbf", coda.len()));
        std::fs::write(&percorso, &dbf).expect("il DBF si scrive");
        let esito = valida_intestazione_dbf(&percorso);
        match (atteso, esito) {
            (Ok(()), Ok(())) | (Err(_), Err(_)) => {}
            (Ok(()), Err(errore)) => {
                panic!("la coda {coda:?} doveva passare: {}", errore.message)
            }
            (Err(messaggio), Ok(())) => {
                panic!("la coda {coda:?} doveva essere rifiutata: {messaggio}")
            }
        }
    };

    // Le due forme ammesse.
    prova(&[], Ok(()));
    prova(&[0x1A], Ok(()));
    // Un byte che non e' il terminatore, e un record intero.
    prova(&[0x20], Err("un byte qualunque e' l'inizio di qualcosa"));
    prova(&[0x1A, 0x1A], Err("due byte non sono un terminatore"));
    prova(
        b" 20260102",
        Err("un record completo oltre il conteggio non e' una coda"),
    );
}

/// Le due strade che la difesa non deve chiudere, dal binario in giu'.
///
/// Le sonde sulla condizione lo dicono sui byte; questa lo dice sul file.
/// Senza, «il campo data e' rifiutato» sarebbe vero anche di un driver che
/// rifiuta **ogni** DBF con un campo `D`, e nessun seme ostile se ne
/// accorgerebbe: sono tutti costruiti per essere rifiutati.
#[test]
fn una_data_valida_e_una_assente_attraversano_il_driver() {
    for nome in ["dbf-data-valida.bundle", "dbf-data-assente.bundle"] {
        // `match` e non `unwrap_or_else`: qui non c'e' nessun ripiego, e
        // il registro dei fallback conta le forme `unwrap_or*` per come
        // sono scritte, non per quel che fanno.
        let righe = match __fuzz_leggi_bundle(
            &seme(nome),
            opzioni_di_campagna().with_assume_crs("EPSG:4326"),
        ) {
            Ok(righe) => righe,
            Err(errore) => panic!("il seme «{nome}» deve leggersi: {errore:?}"),
        };
        assert_eq!(righe, 1, "una polilinea e un record: {nome}");
    }
}

/// Il rovescio: le date valide restano valide, e un campo tutto spazi e'
/// una data assente, non un rifiuto. E' la meta' che una prevalidazione
/// sbaglia piu' spesso, e che nessun seme ostile mostrerebbe.
#[test]
fn una_data_valida_e_un_campo_vuoto_passano() {
    assert!(!data_non_interpretabile(b"20260101"));
    assert!(!data_non_interpretabile(b"        "));
    assert!(!data_non_interpretabile(&[0; 8]));
    assert!(!data_non_interpretabile(b" 20260101 "));

    assert!(data_non_interpretabile(b"2026    "));
    // Una `e` accentata in UTF-8: due byte, e il taglio a `s[4..6]` cade
    // dentro il secondo.
    assert!(data_non_interpretabile("2026\u{e8}01".as_bytes()));
}

/// Otto byte ASCII che non sono cifre, e che la lunghezza non ferma.
///
/// E' il buco che la campagna del 2026-09-04 ha attraversato. La difesa
/// pretendeva otto byte ASCII e li aveva: fra i byte del campo e la stringa
/// che `Date::from_str` affetta c'e' pero' una **decodifica**, che
/// restituisce meno caratteri dei byte ricevuti -- qui nessuno -- e
/// `s[0..4]` cade fuori da una stringa vuota.
///
/// Le cifre sono l'unica classe che nessuna codifica sposta: otto cifre
/// restano otto caratteri, e la fetta e' dentro i limiti per costruzione.
#[test]
fn un_campo_ascii_che_non_e_fatto_di_cifre_e_rifiutato() {
    // La forma esatta del reperto: una cifra e sette NAK.
    assert!(data_non_interpretabile(b"2\x15\x15\x15\x15\x15\x15\x15"));
    // Otto byte di controllo, senza nemmeno una cifra.
    assert!(data_non_interpretabile(&[0x15; 8]));
    // ASCII stampabile e non numerico: la vecchia condizione lo accettava
    // per gli stessi motivi.
    assert!(data_non_interpretabile(b"abcdefgh"));
    assert!(data_non_interpretabile(b"2026-01-"));
    // Il segno che `parse::<u32>` accetterebbe e il formato no.
    assert!(data_non_interpretabile(b"+2026101"));
}

/// La vecchia condizione accettava cio' che la nuova rifiuta.
///
/// Senza, «il campo di controllo e' rifiutato» sarebbe vero anche di una
/// difesa che lo fermava gia' prima, e la condizione nuova non avrebbe
/// cambiato niente. Qui la condizione precedente e' riscritta in una riga
/// -- otto byte ASCII, com'era -- e si guarda che i due verdetti
/// **divergano** su quell'input e coincidano su tutti gli altri.
#[test]
fn la_condizione_precedente_lasciava_passare_il_reperto() {
    let vecchia = |byte: &[u8]| {
        let utile = parte_utile_del_campo(byte);
        !utile.is_empty() && (!utile.is_ascii() || utile.len() < 8)
    };

    let reperto = b"2\x15\x15\x15\x15\x15\x15\x15";
    assert!(!vecchia(reperto), "la vecchia condizione lo accettava");
    assert!(
        data_non_interpretabile(reperto),
        "e la nuova lo rifiuta: e' la differenza fra le due condizioni"
    );

    // E su tutto il resto le due coincidono: la stretta e' mirata, non un
    // giro di vite che rifiuta anche cio' che andava bene.
    for caso in [
        &b"20260101"[..],
        b"        ",
        b" 20260101 ",
        b"2026    ",
        "2026\u{e8}01".as_bytes(),
    ] {
        assert_eq!(
            vecchia(caso),
            data_non_interpretabile(caso),
            "i due verdetti divergono su {caso:?}, e non dovrebbero"
        );
    }
}

/// La prevalidazione vale per **tutte e tre** le versioni che `dbase`
/// tratta come Visual `FoxPro`, non solo per quella che scriviamo noi.
///
/// `DBF_VISUAL_FOXPRO_VERSION` e' `0x30` perche' descrive i nostri file;
/// `Version::from(u8)` della crate esterna accetta anche `0x31` e `0x32`, e
/// un seme con quei byte raggiungerebbe lo stesso `panic!`.
#[test]
fn le_tre_versioni_visual_foxpro_sono_tutte_prevalidate() {
    let temporanea = tempfile::tempdir().expect("directory temporanea");
    for versione in DBF_VERSIONI_VISUAL_FOXPRO {
        let percorso = temporanea.path().join(format!("v{versione:02x}.dbf"));
        let mut intestazione = [0_u8; DBF_HEADER_SIZE];
        intestazione[0] = versione;
        // Sotto i 263 byte del backlink, e sopra i 33 dell'intestazione:
        // cosi' a rifiutare puo' essere **solo** il ramo Visual FoxPro.
        intestazione[8..10].copy_from_slice(&100_u16.to_le_bytes());
        std::fs::write(&percorso, intestazione).expect("scrittura del DBF");

        let errore = valida_intestazione_dbf(&percorso)
            .expect_err("i 263 byte di backlink non stanno in 100");
        assert_eq!(
            errore.message, "header Visual FoxPro piu' corto del backlink",
            "versione {versione:#04x}"
        );
    }
}

/// **Isolamento fra invocazioni**, provato dal `.prj`.
///
/// Senza `assume_crs` il driver accetta solo se il `.prj` c'e'. La prima
/// lettura ne scrive uno; la seconda usa un bundle che non ne ha. Se le due
/// invocazioni condividessero la directory, la seconda troverebbe il `.prj`
/// della prima e **riuscirebbe**: il fallimento e' la prova che ogni input
/// ha la propria directory e che i fratelli della mutazione precedente non
/// sopravvivono.
#[test]
fn ogni_invocazione_ha_la_propria_directory() {
    let con_prj = __fuzz_leggi_bundle(&seme("punti-con-prj.bundle"), opzioni_di_campagna());
    assert!(
        con_prj.is_ok(),
        "il `.prj` del bundle deve bastare a risolvere il CRS: {con_prj:?}"
    );

    let senza_prj = __fuzz_leggi_bundle(&seme("punti-con-attributi.bundle"), opzioni_di_campagna());
    let errore = senza_prj.expect_err(
        "senza `.prj` e senza `assume_crs` l'apertura deve fallire; se riesce, \
             il `.prj` della lettura precedente e' sopravvissuto",
    );
    assert!(errore.message.contains("assume-crs"), "{errore:?}");
}

/// **Fail-closed della divisione**: nessun trabocco, nessuna allocazione
/// derivata dai valori dichiarati, nessun percorso costruito dal payload.
#[test]
fn la_divisione_del_bundle_satura_invece_di_fidarsi() {
    let lunghezze = |dati: &[u8]| {
        let p = __fuzz_dividi_bundle(dati).expect("l'intestazione c'e'");
        (p.shp.len(), p.shx.len(), p.dbf.len(), p.prj.len())
    };

    // Lunghezze massime su un corpo di tre byte: le fette restano dentro il
    // corpo, e non viene riservato niente per i 65 535 dichiarati.
    assert_eq!(
        lunghezze(&[0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 1, 2, 3]),
        (3, 0, 0, 0)
    );

    // Ogni campo dichiara piu' di quanto resti dopo il precedente.
    assert_eq!(
        lunghezze(&[0, 2, 0xFF, 0xFF, 0xFF, 0xFF, 1, 2, 3, 4]),
        (2, 2, 0, 0)
    );

    // Divisione esatta, con un `.prj` che e' il resto. Il confronto e' sui
    // byte e non sulle lunghezze: cosi' uno scambio fra due parti si vede.
    assert_eq!(
        __fuzz_dividi_bundle(&[0, 1, 0, 1, 0, 1, 9, 8, 7, 6, 6]),
        Some(PartiDelBundle {
            shp: &[9],
            shx: &[8],
            dbf: &[7],
            prj: &[6, 6],
        })
    );

    // Un input piu' corto dell'intestazione non e' un bundle.
    for corto in [&b""[..], &b"a"[..], &b"abcde"[..]] {
        assert!(__fuzz_dividi_bundle(corto).is_none(), "{corto:?}");
    }
    // Un bundle senza corpo e' un bundle vuoto, non un errore di divisione.
    assert_eq!(
        __fuzz_dividi_bundle(&[0, 0, 0, 0, 0, 0]),
        Some(PartiDelBundle {
            shp: b"",
            shx: b"",
            dbf: b"",
            prj: b"",
        })
    );
}

/// Un bundle degenere non deve panicare: deve tornare `Err`.
#[test]
fn un_bundle_degenere_e_un_errore_non_un_panico() {
    for degenere in [vec![], vec![0, 0, 0], vec![0, 0, 0, 0], vec![0xFF; 64]] {
        let esito = __fuzz_leggi_bundle(
            &degenere,
            opzioni_di_campagna().with_assume_crs("EPSG:4326"),
        );
        assert!(esito.is_err(), "{degenere:?} non e' uno Shapefile");
    }
}

/// **Errore d'ambiente e non finding**: una radice che non esiste produce
/// un errore tipizzato, non un panico. Provata sulla funzione che scrive,
/// perche' forzare il fallimento mutando `TMPDIR` renderebbe il difetto
/// visibile agli altri test in parallelo.
#[test]
fn una_radice_inesistente_e_un_errore_di_ambiente() {
    let temporanea = tempfile::tempdir().expect("directory temporanea");
    let inesistente = temporanea.path().join("mai-creata");

    let parti = PartiDelBundle {
        shp: b"shp",
        shx: b"",
        dbf: b"dbf",
        prj: b"",
    };
    let errore = materializza_bundle(&inesistente, &parti)
        .expect_err("scrivere in una directory che non esiste deve fallire");
    assert!(
        errore.message.contains("ambiente"),
        "un errore d'ambiente non va confuso con un difetto del file letto: {errore:?}"
    );
}

/// Opzioni di lettura sul modello unificato.
///
/// Da S4.d il percorso di lettura vive interamente li': la memoria dei
/// batch e' una `InternalMemoryLease`, che esiste solo dentro un
/// `PipelineContext`. `opzioni_lettura()` costruisce ancora il ramo
/// legacy — sparira' in S4.e — e con quello `open` fallisce chiuso.
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

/// Opzioni di scrittura con i limiti dati.
///
/// I tetti stanno nel budget, non nelle opzioni: passarli da qui e' l'unico
/// modo di provare il rifiuto sul tetto **e** l'accettazione sotto di esso
/// con lo stesso lotto.
fn opzioni_scrittura_con(limits: plenora_io_model::budget::PipelineLimits) -> WriteOptions {
    match plenora_io_model::budget::PipelineBudget::builder()
        .limits(limits)
        .build()
    {
        Ok(bundle) => WriteOptions::from_write_parts(bundle.into_write_parts()),
        Err(error) => unreachable!("bundle di test non costruibile: {error:?}"),
    }
}

/// Opzioni di scrittura che **accettano** il set di file sciolti.
///
/// Le prove che pubblicano su `*.shp` la usano al posto di
/// `opzioni_scrittura`: da questa revisione una destinazione `*.shp` non
/// deduce piu' la forma debole, la pretende dichiarata. Che le prove
/// debbano dichiararla e' il segno che il rifiuto funziona -- se potessero
/// continuare come prima, non funzionerebbe.
fn opzioni_scrittura_loose() -> WriteOptions {
    opzioni_scrittura().with_format_option("publish_mode", LOOSE_SET_MODE)
}

fn opzioni_lettura() -> ReadOptions {
    match plenora_io_model::budget::PipelineBudget::builder().build() {
        Ok(bundle) => ReadOptions::from_read_parts(bundle.into_read_parts()),
        Err(error) => unreachable!("bundle di test non costruibile: {error:?}"),
    }
}

use std::io::Write as _;

use plenora_io_core::request::{BatchTarget, ProjectionMode};
use plenora_io_core::WriteLayer;
use plenora_io_model::wkb::to_wkb;
use plenora_io_model::CancellationToken;

const EPSG_3003_WKT: &str = include_str!("../tests/fixtures/epsg3003.prj");

fn read_opts() -> ReadOptions {
    opzioni_lettura().with_assume_crs("EPSG:4326")
}

fn req() -> ReadRequest {
    ReadRequest {
        layer: LayerId(0),
        projected_fields: None,
        projection_mode: ProjectionMode::BestEffort,
        pruning_predicate: None,
        spatial_pruning_hint: None,
        scope: ReadScope::default(),
        batch_target: BatchTarget::default(),
        cancellation: CancellationToken::default(),
    }
}

fn make_polygon_ring_unclosed(path: &Path, target_record: usize) {
    let mut file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .unwrap();
    let mut record_offset = 100_u64;
    for record_index in 0..=target_record {
        file.seek(SeekFrom::Start(record_offset)).unwrap();
        let mut record_header = [0_u8; 8];
        file.read_exact(&mut record_header).unwrap();
        let content_bytes =
            u64::from(u32::from_be_bytes(record_header[4..8].try_into().unwrap())) * 2;
        let body_offset = record_offset + 8;
        if record_index == target_record {
            file.seek(SeekFrom::Start(body_offset + 36)).unwrap();
            let mut counts = [0_u8; 8];
            file.read_exact(&mut counts).unwrap();
            let part_count = u64::from(u32::from_le_bytes(counts[0..4].try_into().unwrap()));
            let point_count = u64::from(u32::from_le_bytes(counts[4..8].try_into().unwrap()));
            assert!(part_count > 0 && point_count > 1);
            let points_offset = body_offset + 44 + part_count * 4;
            let last_x_offset = points_offset + (point_count - 1) * 16;
            file.seek(SeekFrom::Start(last_x_offset)).unwrap();
            file.write_all(&1.0_f64.to_le_bytes()).unwrap();
            return;
        }
        record_offset += 8 + content_bytes;
    }
    panic!("record Shapefile {target_record} inesistente");
}

fn truncate_dbf_mid_record(path: &Path, complete_records: u64) {
    let dbf_path = path.with_extension("dbf");
    let header = std::fs::read(&dbf_path).unwrap();
    let header_length = u64::from(u16::from_le_bytes(header[8..10].try_into().unwrap()));
    let record_length = u64::from(u16::from_le_bytes(header[10..12].try_into().unwrap()));
    assert!(record_length > 1);
    let truncated_length = header_length + complete_records * record_length + record_length / 2;
    std::fs::OpenOptions::new()
        .write(true)
        .open(dbf_path)
        .unwrap()
        .set_len(truncated_length)
        .unwrap();
}

fn mark_dbf_record_deleted(path: &Path, source_index: u64) {
    let dbf_path = path.with_extension("dbf");
    let header = std::fs::read(&dbf_path).unwrap();
    let header_length = u64::from(u16::from_le_bytes(header[8..10].try_into().unwrap()));
    let record_length = u64::from(u16::from_le_bytes(header[10..12].try_into().unwrap()));
    let marker_offset = header_length + source_index * record_length;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .open(dbf_path)
        .unwrap();
    file.seek(SeekFrom::Start(marker_offset)).unwrap();
    file.write_all(b"*").unwrap();
}

fn overwrite_dbf_ascii_field(path: &Path, source_index: u64, field_name: &str, value: &str) {
    let dbf_path = path.with_extension("dbf");
    let mut file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(dbf_path)
        .unwrap();
    let mut header = [0_u8; 32];
    file.read_exact(&mut header).unwrap();
    let header_length = u64::from(u16::from_le_bytes(header[8..10].try_into().unwrap()));
    let record_length = u64::from(u16::from_le_bytes(header[10..12].try_into().unwrap()));
    let mut descriptor_offset = 32_u64;
    let mut field_offset = 1_u64;
    loop {
        file.seek(SeekFrom::Start(descriptor_offset)).unwrap();
        let mut descriptor = [0_u8; 32];
        file.read_exact(&mut descriptor).unwrap();
        assert_ne!(descriptor[0], 0x0d, "campo DBF non trovato");
        let name_end = descriptor[..11]
            .iter()
            .position(|byte| *byte == 0)
            .unwrap_or(11);
        let name = std::str::from_utf8(&descriptor[..name_end]).unwrap();
        let width = usize::from(descriptor[16]);
        if name == field_name {
            assert!(value.len() <= width);
            let mut encoded = vec![b' '; width];
            encoded[width - value.len()..].copy_from_slice(value.as_bytes());
            let record_offset = header_length + source_index * record_length + field_offset;
            file.seek(SeekFrom::Start(record_offset)).unwrap();
            file.write_all(&encoded).unwrap();
            return;
        }
        field_offset += width as u64;
        descriptor_offset += 32;
    }
}

fn consume_until_error(reader: &mut dyn LayerReader) -> (usize, PlenoraIoError) {
    let mut emitted_rows = 0;
    loop {
        match reader.next_batch() {
            Ok(Some(batch)) => emitted_rows += batch.num_rows(),
            Ok(None) => panic!("atteso rifiuto row-scoped"),
            Err(error) => return (emitted_rows, error),
        }
    }
}

#[test]
fn degenerate_polygon_rings_have_a_stable_rejection_cause() {
    let repeated = Point::new(1.0, 1.0);
    let rings = vec![PolygonRing::Outer(vec![
        repeated, repeated, repeated, repeated,
    ])];

    assert_eq!(polygon_rejection_cause(&rings), Some(DEGENERATE_RING_CAUSE));
}

// Una sola fixture copre scrittura, corruzioni mirate e le varianti di
// configurazione della diagnostica: separarle duplicherebbe la costruzione
// dello shapefile e ne perderebbe la sequenza.
#[allow(clippy::too_many_lines)]
#[test]
fn invalid_polygon_rows_return_complete_bounded_diagnostics() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("invalid-polygons.shp");
    let key_name = shapefile::dbase::FieldName::try_from("ID_PART").unwrap();
    let numeric_key_name = shapefile::dbase::FieldName::try_from("NUM_KEY").unwrap();
    let integer_value_name = shapefile::dbase::FieldName::try_from("INT_VALUE").unwrap();
    let table = TableWriterBuilder::new()
        .add_character_field(key_name, 32)
        .add_numeric_field(numeric_key_name, 20, 2)
        .add_numeric_field(integer_value_name, 18, 0);
    let mut writer = Writer::from_path(&path, table).unwrap();

    let key_base = 9_007_199_254_740_992_u64;
    for source_index in 0..128 {
        let points = vec![
            Point::new(0.0, 0.0),
            Point::new(0.0, 5.0),
            Point::new(5.0, 5.0),
            Point::new(5.0, 0.0),
            Point::new(0.0, 0.0),
        ];
        let rings = if matches!(source_index, 17 | 113) {
            vec![PolygonRing::Inner(points)]
        } else {
            vec![PolygonRing::Outer(points)]
        };
        let polygon = Polygon::with_rings(rings);
        // source_index < 128: la conversione in f64 e' esatta.
        #[allow(clippy::cast_precision_loss)]
        let numeric_value = source_index as f64;
        let mut record = Record::default();
        record.insert(
            "ID_PART".to_owned(),
            FieldValue::Character(Some((key_base + source_index).to_string())),
        );
        record.insert(
            "NUM_KEY".to_owned(),
            FieldValue::Numeric(Some(numeric_value)),
        );
        record.insert(
            "INT_VALUE".to_owned(),
            FieldValue::Numeric(Some(numeric_value)),
        );
        writer.write_shape_and_record(&polygon, &record).unwrap();
    }
    drop(writer);
    make_polygon_ring_unclosed(&path, 89);
    mark_dbf_record_deleted(&path, 20);
    for source_index in [17_u64, 89, 113] {
        overwrite_dbf_ascii_field(
            &path,
            source_index,
            "NUM_KEY",
            &format!("{}.25", key_base + source_index),
        );
    }
    std::fs::write(path.with_extension("prj"), EPSG_3003_WKT).unwrap();
    let malformed_directory = tempfile::tempdir().unwrap();
    let malformed_path = malformed_directory.path().join("invalid-attribute.shp");
    for extension in ["shp", "shx", "dbf", "prj"] {
        std::fs::copy(
            path.with_extension(extension),
            malformed_path.with_extension(extension),
        )
        .unwrap();
    }

    let mut options = read_opts();
    options
        .format_options
        .insert("row_diagnostics.examples_limit".to_owned(), "2".to_owned());
    options
        .format_options
        .insert("row_diagnostics.key_field".to_owned(), "ID_PART".to_owned());
    options
        .format_options
        .insert("row_diagnostics.key_policy".to_owned(), "emit".to_owned());
    let dataset = ShpDriver.open(Source::Path(path.clone()), options).unwrap();
    let request = ReadRequest {
        batch_target: BatchTarget {
            target_bytes: 8 * 1024 * 1024,
            max_rows: 8,
        },
        ..req()
    };
    let mut reader = dataset.open_layer_reader(&request).unwrap();
    let (emitted_rows, error) = consume_until_error(reader.as_mut());
    assert_eq!(emitted_rows, 0);
    let diagnostics = error
        .row_diagnostics
        .expect("diagnostica row-scoped mancante");
    assert_eq!(diagnostics.observed_total, 3);
    assert_eq!(diagnostics.total, Some(3));
    assert_eq!(
        diagnostics.counts.get("shapefile.inner_ring_without_outer"),
        Some(&2)
    );
    assert_eq!(diagnostics.counts.get("shapefile.unclosed_ring"), Some(&1));
    assert_eq!(diagnostics.examples_limit, 2);
    assert!(diagnostics.examples_truncated);
    assert_eq!(diagnostics.examples.len(), 2);
    assert_eq!(diagnostics.examples[0].source_index, 17);
    assert_eq!(diagnostics.examples[1].source_index, 89);
    assert_eq!(
        diagnostics.examples[0]
            .key
            .as_ref()
            .and_then(|key| key.value.as_ref()),
        Some(&plenora_io_model::RowDiagnosticKeyValue::String(
            (key_base + 17).to_string()
        ))
    );
    assert_eq!(
        diagnostics.examples[1]
            .key
            .as_ref()
            .and_then(|key| key.value.as_ref()),
        Some(&plenora_io_model::RowDiagnosticKeyValue::String(
            (key_base + 89).to_string()
        ))
    );

    let mut attribute_only_request = req();
    attribute_only_request.projected_fields = Some(vec![FieldId(1)]);
    attribute_only_request.batch_target = BatchTarget {
        target_bytes: 8 * 1024 * 1024,
        max_rows: 8,
    };
    let attribute_only_dataset = ShpDriver
        .open(Source::Path(path.clone()), read_opts())
        .unwrap();
    let mut attribute_only_reader = attribute_only_dataset
        .open_layer_reader(&attribute_only_request)
        .unwrap();
    let (attribute_rows, attribute_error) = consume_until_error(attribute_only_reader.as_mut());
    assert_eq!(attribute_rows, 0);
    assert_eq!(attribute_error.row_diagnostics.unwrap().observed_total, 3);

    let mut numeric_key_options = read_opts();
    numeric_key_options
        .format_options
        .insert("row_diagnostics.examples_limit".to_owned(), "2".to_owned());
    numeric_key_options
        .format_options
        .insert("row_diagnostics.key_field".to_owned(), "NUM_KEY".to_owned());
    numeric_key_options
        .format_options
        .insert("row_diagnostics.key_policy".to_owned(), "emit".to_owned());
    let numeric_key_dataset = ShpDriver
        .open(Source::Path(path.clone()), numeric_key_options)
        .unwrap();
    let mut numeric_key_reader = numeric_key_dataset.open_layer_reader(&request).unwrap();
    let (_, numeric_key_error) = consume_until_error(numeric_key_reader.as_mut());
    let numeric_examples = numeric_key_error.row_diagnostics.unwrap().examples;
    for (example, source_index) in numeric_examples.iter().zip([17_u64, 89]) {
        assert_eq!(
            example.key.as_ref().and_then(|key| key.value.as_ref()),
            Some(&plenora_io_model::RowDiagnosticKeyValue::String(format!(
                "{}.25",
                key_base + source_index
            )))
        );
    }

    let dataset_without_key = ShpDriver
        .open(Source::Path(path.clone()), read_opts())
        .unwrap();
    let mut reader_without_key = dataset_without_key.open_layer_reader(&request).unwrap();
    let (_, error_without_key) = consume_until_error(reader_without_key.as_mut());
    assert!(error_without_key
        .row_diagnostics
        .unwrap()
        .examples
        .iter()
        .all(|example| example.key.is_none()));

    let mut redacted_options = read_opts();
    redacted_options
        .format_options
        .insert("row_diagnostics.key_field".to_owned(), "ID_PART".to_owned());
    redacted_options
        .format_options
        .insert("row_diagnostics.key_policy".to_owned(), "redact".to_owned());
    let redacted_dataset = ShpDriver
        .open(Source::Path(path.clone()), redacted_options)
        .unwrap();
    let mut redacted_reader = redacted_dataset.open_layer_reader(&request).unwrap();
    let (_, redacted_error) = consume_until_error(redacted_reader.as_mut());
    let redacted_key = redacted_error.row_diagnostics.unwrap().examples[0]
        .key
        .clone()
        .unwrap();
    assert_eq!(redacted_key.state, RowDiagnosticKeyState::Redacted);
    assert!(redacted_key.value.is_none());

    let mut missing_policy = read_opts();
    missing_policy
        .format_options
        .insert("row_diagnostics.key_field".to_owned(), "ID_PART".to_owned());
    let Err(missing_policy_error) = ShpDriver.open(Source::Path(path.clone()), missing_policy)
    else {
        panic!("key_field senza policy deve essere rifiutato")
    };
    assert_eq!(
        missing_policy_error.category,
        plenora_io_model::ErrorCategory::InvalidConfiguration
    );

    let mut zero_limit = read_opts();
    zero_limit
        .format_options
        .insert("row_diagnostics.examples_limit".to_owned(), "0".to_owned());
    let Err(zero_limit_error) = ShpDriver.open(Source::Path(path.clone()), zero_limit) else {
        panic!("examples_limit zero deve essere rifiutato")
    };
    assert_eq!(
        zero_limit_error.category,
        plenora_io_model::ErrorCategory::InvalidConfiguration
    );

    let mut cancelled_diagnostics = ShpRowDiagnostics::new(ShpRowDiagnosticsConfig {
        examples_limit: 1,
        key: None,
    });
    cancelled_diagnostics.record(
        17,
        INNER_RING_WITHOUT_OUTER_CAUSE,
        Some(&Record::default()),
        None,
    );
    let cancellation = plenora_io_model::CancellationToken::default();
    cancellation.cancel();
    let cancellation_error =
        plenora_io_core::check_cancelled(&cancellation, plenora_io_model::ErrorPhase::Read)
            .expect_err("la cancellation richiesta deve essere osservata");
    let cancelled = cancelled_diagnostics
        .into_partial_error(cancellation_error, "shapefile.scan_cancelled")
        .row_diagnostics
        .unwrap();
    assert_eq!(cancelled.completeness, RowDiagnosticsCompleteness::Partial);
    assert_eq!(
        cancelled.knowledge_limits,
        Some(vec!["shapefile.scan_cancelled".to_owned()])
    );
    assert!(cancelled.total.is_none());

    let partial_dataset = ShpDriver
        .open(Source::Path(path.clone()), read_opts())
        .unwrap();
    truncate_dbf_mid_record(&path, 50);
    let mut partial_reader = partial_dataset.open_layer_reader(&request).unwrap();
    let (_, partial_error) = consume_until_error(partial_reader.as_mut());
    let partial = partial_error.row_diagnostics.unwrap();
    assert_eq!(partial.completeness, RowDiagnosticsCompleteness::Partial);
    assert_eq!(partial.total, None);
    assert_eq!(partial.observed_total, 1);
    assert_eq!(
        partial.knowledge_limits,
        Some(vec!["shapefile.dbf_exact_scan_interrupted".to_owned()])
    );
    assert_eq!(
        partial.counts.get("shapefile.inner_ring_without_outer"),
        Some(&1)
    );
    assert_eq!(partial.examples.len(), 1);
    assert!(!partial.examples_truncated);

    overwrite_dbf_ascii_field(&malformed_path, 42, "INT_VALUE", "not-an-integer");
    let malformed_dataset = ShpDriver
        .open(Source::Path(malformed_path), read_opts())
        .unwrap();
    let mut malformed_reader = malformed_dataset.open_layer_reader(&request).unwrap();
    let (_, malformed_error) = consume_until_error(malformed_reader.as_mut());
    let malformed = malformed_error.row_diagnostics.unwrap();
    assert_eq!(malformed.completeness, RowDiagnosticsCompleteness::Complete);
    assert_eq!(malformed.observed_total, 4);
    assert_eq!(
        malformed.counts.get(ATTRIBUTE_NUMERIC_INVALID_CAUSE),
        Some(&1)
    );
    assert!(malformed.examples.iter().any(|example| {
        example.source_index == 42 && example.cause == ATTRIBUTE_NUMERIC_INVALID_CAUSE
    }));
}

#[test]
fn accepted_rows_stops_invalid_shapefile_scan_at_active_row_limit() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("bounded-invalid.shp");
    let id = shapefile::dbase::FieldName::try_from("ID").unwrap();
    let table = TableWriterBuilder::new().add_numeric_field(id, 9, 0);
    let mut writer = Writer::from_path(&path, table).unwrap();
    for source_index in 0..4_096 {
        let points = vec![
            Point::new(0.0, 0.0),
            Point::new(0.0, 5.0),
            Point::new(5.0, 5.0),
            Point::new(5.0, 0.0),
            Point::new(0.0, 0.0),
        ];
        let rings = if matches!(source_index, 17 | 89 | 3_000) {
            vec![PolygonRing::Inner(points)]
        } else {
            vec![PolygonRing::Outer(points)]
        };
        let mut record = Record::default();
        record.insert(
            "ID".to_owned(),
            FieldValue::Numeric(Some(f64::from(source_index))),
        );
        writer
            .write_shape_and_record(&Polygon::with_rings(rings), &record)
            .unwrap();
    }
    drop(writer);
    mark_dbf_record_deleted(&path, 20);
    std::fs::write(path.with_extension("prj"), EPSG_3003_WKT).unwrap();

    // Fault-tail deterministico: il dataset e' inferito quando integro, poi
    // la coda oltre il prefisso richiesto diventa illeggibile.
    let dataset = ShpDriver
        .open(Source::Path(path.clone()), read_opts())
        .unwrap();
    truncate_dbf_mid_record(&path, 200);
    let request = ReadRequest {
        scope: plenora_io_core::ReadScope::AcceptedRows(32),
        batch_target: BatchTarget {
            target_bytes: 8 * 1024 * 1024,
            max_rows: 8,
        },
        ..req()
    };
    let mut reader = dataset.open_layer_reader(&request).unwrap();
    let (emitted_rows, error) = consume_until_error(reader.as_mut());
    assert_eq!(emitted_rows, 0);
    let diagnostics = error.row_diagnostics.as_deref().unwrap();
    assert_eq!(
        diagnostics.completeness,
        RowDiagnosticsCompleteness::Partial
    );
    assert_eq!(diagnostics.total, None);
    assert_eq!(diagnostics.observed_total, 1);
    assert_eq!(diagnostics.counts[INNER_RING_WITHOUT_OUTER_CAUSE], 1);
    assert_eq!(diagnostics.examples[0].source_index, 17);
    assert_eq!(
        diagnostics.knowledge_limits.as_deref(),
        Some(["read_scope_row_limit_reached".to_owned()].as_slice())
    );
    assert!(diagnostics.validate().is_ok());
}

#[test]
fn accepted_rows_preserves_valid_shapefile_batch_overshoot_and_skips_late_invalidity() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("late-invalid.shp");
    let id = shapefile::dbase::FieldName::try_from("ID").unwrap();
    let table = TableWriterBuilder::new().add_numeric_field(id, 9, 0);
    let mut writer = Writer::from_path(&path, table).unwrap();
    for source_index in 0..25 {
        let points = vec![
            Point::new(0.0, 0.0),
            Point::new(0.0, 5.0),
            Point::new(5.0, 5.0),
            Point::new(5.0, 0.0),
            Point::new(0.0, 0.0),
        ];
        let rings = if source_index == 20 {
            vec![PolygonRing::Inner(points)]
        } else {
            vec![PolygonRing::Outer(points)]
        };
        let mut record = Record::default();
        record.insert(
            "ID".to_owned(),
            FieldValue::Numeric(Some(f64::from(source_index))),
        );
        writer
            .write_shape_and_record(&Polygon::with_rings(rings), &record)
            .unwrap();
    }
    drop(writer);
    std::fs::write(path.with_extension("prj"), EPSG_3003_WKT).unwrap();

    let dataset = ShpDriver
        .open(Source::Path(path.clone()), read_opts())
        .unwrap();
    let request = ReadRequest {
        scope: ReadScope::AcceptedRows(10),
        batch_target: BatchTarget {
            target_bytes: 8 * 1024 * 1024,
            max_rows: 8,
        },
        ..req()
    };
    let mut reader = dataset.open_layer_reader(&request).unwrap();
    let mut rows = Vec::new();
    while let Some(batch) = reader.next_batch().unwrap() {
        rows.push(batch.num_rows());
    }
    assert_eq!(rows, vec![8, 8]);

    let complete_dataset = ShpDriver.open(Source::Path(path), read_opts()).unwrap();
    // L'accoppiamento su cui poggia il `field_index` degli esempi: lo schema
    // mette la geometria **davanti** alle colonne DBF, e l'indice pubblicato
    // e' `posizione in cols + 1`. Se l'ordine cambiasse, quegli indici
    // indicherebbero la colonna sbagliata e nessuno se ne accorgerebbe.
    {
        let contratto = &complete_dataset.layers()[0];
        assert_eq!(
            contratto.contract.schema.field(0).name(),
            &contratto
                .contract
                .geometry
                .as_ref()
                .expect("uno shapefile ha una geometria")
                .name,
            "la geometria deve essere il primo campo dello schema"
        );
    }
    let mut complete_request = request;
    complete_request.scope = ReadScope::Complete;
    let mut complete = complete_dataset
        .open_layer_reader(&complete_request)
        .unwrap();
    let (published_rows, error) = consume_until_error(complete.as_mut());
    assert_eq!(published_rows, 0);
    let diagnostics = error.row_diagnostics.as_deref().unwrap();
    assert_eq!(
        diagnostics.completeness,
        RowDiagnosticsCompleteness::Complete
    );
    assert_eq!(diagnostics.examples[0].source_index, 20);
    assert_eq!(diagnostics.total, Some(1));
}

#[test]
fn prj_authority_is_resolved_and_keeps_epsg_axis_order() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("roads.shp");
    std::fs::write(
        path.with_extension("prj"),
        "GEOGCS[\"WGS 84\",AUTHORITY[\"EPSG\",\"4326\"]]",
    )
    .unwrap();

    let crs = resolve_crs(&path, &opzioni_lettura()).unwrap();
    assert_eq!(crs.id.as_deref(), Some("EPSG:4326"));
    assert_eq!(
        crs.axis_order,
        plenora_io_model::crs::AxisOrder::LatitudeLongitude
    );
}

#[test]
fn projected_prj_with_nested_geogcs_keeps_projected_kind_and_axis_order() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("parcels.shp");
    std::fs::write(path.with_extension("prj"), EPSG_3003_WKT).unwrap();

    let crs = resolve_crs(&path, &opzioni_lettura()).unwrap();
    assert_eq!(crs.id.as_deref(), Some("EPSG:3003"));
    assert_eq!(crs.kind, CrsKind::Projected);
    assert_eq!(
        crs.axis_order,
        plenora_io_model::crs::AxisOrder::EastingNorthing
    );
}

#[test]
fn wide_zero_decimal_dbf_numeric_is_read_exactly_as_i64() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("parcels.shp");
    let field_name = shapefile::dbase::FieldName::try_from("parcel_id").unwrap();
    let table = TableWriterBuilder::new().add_numeric_field(field_name, 18, 0);
    let mut writer = Writer::from_path(&path, table).unwrap();
    for coordinate in [0.0, 1.0] {
        let mut record = Record::default();
        record.insert("parcel_id".to_owned(), FieldValue::Numeric(Some(0.0)));
        writer
            .write_shape_and_record(&Point::new(coordinate, coordinate), &record)
            .unwrap();
    }
    drop(writer);
    std::fs::write(path.with_extension("prj"), EPSG_3003_WKT).unwrap();

    // Il writer dbase accetta già f64. Si sostituiscono i byte del campo
    // con due interi ASCII distinti per riprodurre un DBF patrimoniale
    // reale prima che dbase 0.5.0 li converta nello stesso f64.
    let mut dbf = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(path.with_extension("dbf"))
        .unwrap();
    let mut header = [0_u8; 32];
    dbf.read_exact(&mut header).unwrap();
    let header_length = u64::from(u16::from_le_bytes([header[8], header[9]]));
    let record_length = u64::from(u16::from_le_bytes([header[10], header[11]]));
    for (row, value) in ["9007199254740992", "9007199254740993"]
        .into_iter()
        .enumerate()
    {
        dbf.seek(SeekFrom::Start(
            header_length + (row as u64 * record_length) + 1,
        ))
        .unwrap();
        dbf.write_all(format!("{value:>18}").as_bytes()).unwrap();
    }
    drop(dbf);

    let dataset = ShpDriver
        .open(Source::Path(path), opzioni_lettura())
        .unwrap();
    let assessment = dataset.fidelity_assessment();
    assert_eq!(assessment.level, Fidelity::Conditional);

    let mut reader = dataset.open_layer_reader(&req()).unwrap();
    let loss = reader.loss_report();
    assert!(!loss
        .counts
        .contains_key(DBF_NUMERIC_INTEGER_PRECISION_UNVERIFIABLE));
    let batch = reader.next_batch().unwrap().unwrap();
    let ids = batch
        .column(1)
        .as_any()
        .downcast_ref::<arrow_array::Int64Array>()
        .unwrap();
    assert_eq!(ids.value(0), 9_007_199_254_740_992);
    assert_eq!(ids.value(1), 9_007_199_254_740_993);
}

#[test]
fn narrow_or_decimal_dbf_numeric_keeps_float_mapping() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("numeric-shapes.shp");
    let narrow = shapefile::dbase::FieldName::try_from("narrow").unwrap();
    let decimal = shapefile::dbase::FieldName::try_from("decimal").unwrap();
    let table = TableWriterBuilder::new()
        .add_numeric_field(narrow, 9, 0)
        .add_numeric_field(decimal, 18, 2);
    let mut writer = Writer::from_path(&path, table).unwrap();
    let mut record = Record::default();
    record.insert("narrow".to_owned(), FieldValue::Numeric(Some(123.0)));
    record.insert("decimal".to_owned(), FieldValue::Numeric(Some(12.5)));
    writer
        .write_shape_and_record(&Point::new(0.0, 0.0), &record)
        .unwrap();
    drop(writer);
    std::fs::write(path.with_extension("prj"), EPSG_3003_WKT).unwrap();

    let dataset = ShpDriver
        .open(Source::Path(path), opzioni_lettura())
        .unwrap();
    let schema = &dataset.layers()[0].contract.schema;
    assert_eq!(schema.field(1).data_type(), &DataType::Float64);
    assert_eq!(schema.field(2).data_type(), &DataType::Float64);
}

#[test]
fn duplicate_dbf_field_names_are_rejected_before_record_map_collapse() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("duplicates.shp");
    let first = shapefile::dbase::FieldName::try_from("first").unwrap();
    let second = shapefile::dbase::FieldName::try_from("second").unwrap();
    let table = TableWriterBuilder::new()
        .add_character_field(first, 16)
        .add_character_field(second, 16);
    let mut writer = Writer::from_path(&path, table).unwrap();
    let mut record = Record::default();
    record.insert(
        "first".to_owned(),
        FieldValue::Character(Some("a".to_owned())),
    );
    record.insert(
        "second".to_owned(),
        FieldValue::Character(Some("b".to_owned())),
    );
    writer
        .write_shape_and_record(&Point::new(0.0, 0.0), &record)
        .unwrap();
    drop(writer);
    std::fs::write(path.with_extension("prj"), EPSG_3003_WKT).unwrap();

    let mut dbf = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(path.with_extension("dbf"))
        .unwrap();
    dbf.seek(SeekFrom::Start(
        (DBF_HEADER_SIZE + DBF_FIELD_DESCRIPTOR_SIZE) as u64,
    ))
    .unwrap();
    let mut duplicate = [0_u8; DBF_FIELD_NAME_SIZE];
    duplicate[..5].copy_from_slice(b"first");
    dbf.write_all(&duplicate).unwrap();
    drop(dbf);

    let error = ShpDriver
        .open(Source::Path(path), opzioni_lettura())
        .err()
        .expect("il DBF con nomi duplicati deve essere rifiutato");
    assert!(error.to_string().contains("nomi campo DBF duplicati"));
}

#[test]
fn unresolved_prj_is_preserved_in_typed_error() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("local.shp");
    let definition = "LOCAL_CS[\"survey-grid-secret\"]";
    std::fs::write(path.with_extension("prj"), definition).unwrap();

    let error = resolve_crs(&path, &opzioni_lettura()).unwrap_err();
    assert_eq!(error.code, plenora_io_model::IoErrorCode::CrsUnresolved);
    assert_eq!(error.driver.as_deref(), Some("shp"));
    assert!(!error.to_string().contains("survey-grid-secret"));
}

#[test]
fn assumed_unknown_epsg_does_not_invent_an_axis_order() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("no-prj.shp");
    let crs = resolve_crs(&path, &opzioni_lettura().with_assume_crs("EPSG:4258")).unwrap();
    assert_eq!(crs.kind, CrsKind::Unknown);
    assert_eq!(crs.axis_order, plenora_io_model::crs::AxisOrder::Unknown);
}

#[test]
fn resolved_crs_without_id_cannot_be_relabelled_as_unknown() {
    let crs = ResolvedCrs::new(
        None,
        CrsKind::Unknown,
        Some("LOCAL_CS[\"private\"]".to_owned()),
    );

    assert!(matches!(
        resolved_crs_id(&crs),
        Err(error) if error.code == plenora_io_model::IoErrorCode::Crs
    ));
}

#[test]
fn write_then_read_round_trip() {
    use arrow_array::{Int64Array, StringArray};
    use arrow_schema::DataType;

    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("pts.shp");

    let wkb1 = to_wkb(&geo_types::Geometry::Point(geo_types::Point::new(
        12.5, 45.9,
    )))
    .unwrap();
    let wkb2 = to_wkb(&geo_types::Geometry::Point(geo_types::Point::new(
        9.19, 45.46,
    )))
    .unwrap();
    let schema: SchemaRef = Arc::new(Schema::new(vec![
        geometry_field(GEOMETRY, "EPSG:4326"),
        Field::new("nome", DataType::Utf8, true),
        Field::new("pop", DataType::Int64, true),
    ]));
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(BinaryArray::from(vec![
                Some(wkb1.as_slice()),
                Some(wkb2.as_slice()),
            ])),
            Arc::new(StringArray::from(vec!["Roma", "Milano"])),
            Arc::new(Int64Array::from(vec![2_800_000i64, 1_400_000])),
        ],
    )
    .unwrap();

    let driver = ShpDriver;
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
        .create(Sink::Path(out.clone()), &plan, &opzioni_scrittura_loose())
        .unwrap();
    w.write(&batch).unwrap();
    w.finish().unwrap();

    // il set è stato pubblicato
    assert!(out.exists());
    assert!(out.with_extension("dbf").exists());
    assert!(out.with_extension("prj").exists());

    // rilettura
    let ds = driver.open(Source::Path(out), read_opts()).unwrap();
    let mut r = ds.open_layer_reader(&req()).unwrap();
    let rb = r.next_batch().unwrap().unwrap();
    assert_eq!(rb.num_rows(), 2);
    let nome = rb
        .column_by_name("nome")
        .unwrap()
        .as_any()
        .downcast_ref::<StringArray>()
        .unwrap();
    assert_eq!(nome.value(0), "Roma");
}

#[test]
fn writer_adapter_attributes_mixed_geometry_and_prevents_publish() {
    let dir = tempfile::tempdir().unwrap();
    let output = dir.path().join("mixed.shp");
    let point = to_wkb(&geo_types::Geometry::Point(geo_types::Point::new(1.0, 2.0))).unwrap();
    let line = to_wkb(&geo_types::Geometry::LineString(
        geo_types::LineString::from(vec![(0.0, 0.0), (1.0, 1.0)]),
    ))
    .unwrap();
    let schema: SchemaRef = Arc::new(Schema::new(vec![geometry_field(GEOMETRY, "EPSG:4326")]));
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![Arc::new(BinaryArray::from(vec![
            Some(point.as_slice()),
            Some(line.as_slice()),
        ]))],
    )
    .unwrap();
    let plan = WritePlan {
        layers: vec![WriteLayer {
            name: "mixed".to_owned(),
            contract: DataContract {
                schema,
                geometry: None,
            },
        }],
    };
    let mut writer = ShpDriver
        .create(
            Sink::Path(output.clone()),
            &plan,
            &opzioni_scrittura_loose(),
        )
        .unwrap();
    writer.declare_input_total(LayerId(0), 2).unwrap();

    let error = writer.write(&batch).unwrap_err();
    let diagnostics = error.row_diagnostics.as_deref().unwrap();
    assert_eq!(diagnostics.input_total, Some(2));
    assert_eq!(diagnostics.examples[0].source_index, 1);
    assert_eq!(diagnostics.counts["shapefile.mixed_geometry_type"], 1);
    assert!(diagnostics.validate().is_ok());
    assert!(writer.finish().is_err());
    assert!(!output.exists());
    assert!(!output.with_extension("dbf").exists());
}

#[test]
fn directory_dataset_round_trip_uses_atomic_directory_unit() {
    use arrow_array::Int64Array;
    use arrow_schema::DataType;

    let root = tempfile::tempdir().unwrap();
    let output = root.path().join("points.shp.d");
    let wkb = to_wkb(&geo_types::Geometry::Point(geo_types::Point::new(
        12.5, 45.9,
    )))
    .unwrap();
    let schema: SchemaRef = Arc::new(Schema::new(vec![
        geometry_field(GEOMETRY, "EPSG:4326"),
        Field::new("id", DataType::Int64, false),
    ]));
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(BinaryArray::from(vec![Some(wkb.as_slice())])),
            Arc::new(Int64Array::from(vec![1_i64])),
        ],
    )
    .unwrap();
    let plan = WritePlan {
        layers: vec![WriteLayer {
            name: "points".to_owned(),
            contract: DataContract {
                schema,
                geometry: None,
            },
        }],
    };
    let options = opzioni_scrittura()
        .with_durable(true)
        .with_format_option("publish_mode", DIRECTORY_DATASET_MODE);

    let driver = ShpDriver;
    let mut writer = driver
        .create(Sink::Path(output.clone()), &plan, &options)
        .unwrap();
    writer.write(&batch).unwrap();
    assert!(
        !output.exists(),
        "la directory dataset è diventata visibile prima di finish"
    );
    let published = writer.finish().unwrap();

    let expected_outcome = if cfg!(unix) {
        plenora_io_core::PublishOutcome::Published
    } else {
        plenora_io_core::PublishOutcome::PublishedButDurabilityUnconfirmed
    };
    assert_eq!(published.outcome, expected_outcome);
    assert!(output.is_dir());
    assert!(output.join("data.shp").is_file());
    assert!(output.join("data.shx").is_file());
    assert!(output.join("data.dbf").is_file());
    assert!(output.join("data.prj").is_file());

    let dataset = driver.open(Source::Path(output), read_opts()).unwrap();
    let mut reader = dataset.open_layer_reader(&req()).unwrap();
    assert_eq!(reader.next_batch().unwrap().unwrap().num_rows(), 1);
}

#[test]
fn directory_dataset_abort_removes_staging() {
    let root = tempfile::tempdir().unwrap();
    let output = root.path().join("aborted.shp.d");
    let schema: SchemaRef = Arc::new(Schema::new(vec![geometry_field(GEOMETRY, "EPSG:4326")]));
    let plan = WritePlan {
        layers: vec![WriteLayer {
            name: "points".to_owned(),
            contract: DataContract {
                schema,
                geometry: None,
            },
        }],
    };

    let writer = ShpDriver
        .create(Sink::Path(output.clone()), &plan, &opzioni_scrittura())
        .unwrap();
    drop(writer);

    assert!(!output.exists());
    assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
}

// --- ASSURANCE-N1: i rami negativi della conversione WKB -> topologia ---
//
// Tre gruppi del censimento, chiusi insieme perche' sono la stessa
// superficie: `topology_from_wkb` decide, e delega a `take_child` e
// `polygon_rings`. Il replay del fuzzer li raggiungeva gia' -- reachability
// e panic-safety erano provate -- ma nessuna prova diceva **quale** rifiuto
// arriva per **quale** input, che e' il contratto.

/// Una coordinata XY, che e' l'unica forma che queste sonde usano.
fn xy(x: f64, y: f64) -> WkbCoordinate {
    WkbCoordinate {
        x,
        y,
        z: None,
        m: None,
    }
}

/// Un anello quadrato e chiuso: quattro coordinate, la prima uguale
/// all'ultima.
fn anello_valido() -> Vec<WkbCoordinate> {
    vec![xy(0.0, 0.0), xy(1.0, 0.0), xy(1.0, 1.0), xy(0.0, 0.0)]
}

fn geometria(value: WkbValue) -> WkbGeometry {
    WkbGeometry {
        value,
        dimensions: CoordinateDimensions::Xy,
        srid: None,
    }
}

/// Ogni rifiuto di `topology_from_wkb`, con l'input che lo produce.
///
/// La sonda non verifica «fallisce»: verifica **quale** messaggio arriva.
/// Un rifiuto giusto per la ragione sbagliata manda chi legge l'errore a
/// correggere la cosa sbagliata, ed e' indistinguibile da quello giusto se
/// la sonda si accontenta di `is_err()`.
// La lunghezza e' nel numero di casi, non in complessita' logica: la
// funzione e' una tabella, e spezzarla in due meta' arbitrarie renderebbe
// piu' difficile vedere che i casi coprono ogni rifiuto.
#[allow(clippy::too_many_lines)]
#[test]
fn n1_topology_from_wkb_rifiuta_ogni_forma_non_rappresentabile() {
    let casi: Vec<(&str, WkbGeometry, &str)> = vec![
        (
            "SRID incorporato",
            WkbGeometry {
                value: WkbValue::Point(xy(1.0, 2.0)),
                dimensions: CoordinateDimensions::Xy,
                srid: Some(4326),
            },
            "SRID embedded",
        ),
        (
            "MultiPoint vuoto",
            geometria(WkbValue::MultiPoint(Vec::new())),
            "MultiPoint vuoto",
        ),
        (
            "LineString con una sola coordinata",
            geometria(WkbValue::LineString(vec![xy(0.0, 0.0)])),
            "LineString con meno di due coordinate",
        ),
        (
            "MultiLineString vuoto",
            geometria(WkbValue::MultiLineString(Vec::new())),
            "MultiLineString vuoto",
        ),
        (
            "parte di MultiLineString con una sola coordinata",
            geometria(WkbValue::MultiLineString(vec![geometria(
                WkbValue::LineString(vec![xy(0.0, 0.0)]),
            )])),
            "parte LineString con meno di due coordinate",
        ),
        (
            "MultiPolygon vuoto",
            geometria(WkbValue::MultiPolygon(Vec::new())),
            "MultiPolygon vuoto",
        ),
        (
            "GeometryCollection",
            geometria(WkbValue::GeometryCollection(Vec::new())),
            "GeometryCollection non rappresentabile",
        ),
        (
            "tipo esteso: CircularString",
            geometria(WkbValue::CircularString(Vec::new())),
            "tipo WKB esteso",
        ),
        (
            "tipo esteso: Triangle",
            geometria(WkbValue::Triangle(Vec::new())),
            "tipo WKB esteso",
        ),
        (
            "poligono senza anelli",
            geometria(WkbValue::Polygon(Vec::new())),
            "poligono vuoto",
        ),
        (
            "anello con meno di quattro coordinate",
            geometria(WkbValue::Polygon(vec![vec![
                xy(0.0, 0.0),
                xy(1.0, 0.0),
                xy(0.0, 0.0),
            ]])),
            "anello WKB non chiuso o con meno di quattro coordinate",
        ),
        (
            "anello non chiuso",
            geometria(WkbValue::Polygon(vec![vec![
                xy(0.0, 0.0),
                xy(1.0, 0.0),
                xy(1.0, 1.0),
                xy(0.0, 1.0),
            ]])),
            "anello WKB non chiuso o con meno di quattro coordinate",
        ),
        (
            "membro di MultiPoint con SRID proprio",
            geometria(WkbValue::MultiPoint(vec![WkbGeometry {
                value: WkbValue::Point(xy(1.0, 2.0)),
                dimensions: CoordinateDimensions::Xy,
                srid: Some(4326),
            }])),
            "geometria WKB annidata incoerente",
        ),
        (
            "membro di MultiPoint con dimensioni diverse dal padre",
            geometria(WkbValue::MultiPoint(vec![WkbGeometry {
                value: WkbValue::Point(xy(1.0, 2.0)),
                dimensions: CoordinateDimensions::Xyz,
                srid: None,
            }])),
            "geometria WKB annidata incoerente",
        ),
        (
            "membro di MultiPoint che non e' un Point",
            geometria(WkbValue::MultiPoint(vec![geometria(WkbValue::LineString(
                vec![xy(0.0, 0.0), xy(1.0, 1.0)],
            ))])),
            "geometria WKB annidata incoerente",
        ),
        (
            "membro di MultiLineString che non e' una LineString",
            geometria(WkbValue::MultiLineString(vec![geometria(WkbValue::Point(
                xy(0.0, 0.0),
            ))])),
            "geometria WKB annidata incoerente",
        ),
        (
            "membro di MultiPolygon che non e' un Polygon",
            geometria(WkbValue::MultiPolygon(vec![geometria(WkbValue::Point(
                xy(0.0, 0.0),
            ))])),
            "geometria WKB annidata incoerente",
        ),
        (
            "anello non chiuso dentro un MultiPolygon",
            geometria(WkbValue::MultiPolygon(vec![geometria(WkbValue::Polygon(
                vec![vec![xy(0.0, 0.0), xy(1.0, 0.0), xy(1.0, 1.0), xy(0.0, 1.0)]],
            ))])),
            "anello WKB non chiuso",
        ),
    ];

    for (nome, ingresso, atteso) in casi {
        // `let ... else` e non `unwrap_or_else(|| panic!(...))`: la seconda
        // forma **sembra** un fallback -- il registro di H-01 conta
        // `unwrap_or*` per sintassi -- mentre qui non c'e' nessun valore di
        // ripiego, solo un'asserzione che diverge.
        let Err(errore) = topology_from_wkb(ingresso) else {
            panic!("«{nome}» doveva essere rifiutato e non lo e' stato");
        };
        assert!(
            errore.message.contains(atteso),
            "«{nome}»: atteso un rifiuto che nomina «{atteso}», arrivato «{}»",
            errore.message
        );
    }
}

/// Le forme che **devono** passare, accanto a quelle che non devono.
///
/// Senza questo verso, un `topology_from_wkb` che rifiutasse tutto
/// supererebbe la sonda dei rifiuti: e' il modo piu' rapido di far passare
/// una tabella di casi negativi.
#[test]
fn n1_topology_from_wkb_accetta_le_forme_rappresentabili() {
    let casi: Vec<(&str, WkbValue)> = vec![
        ("punto", WkbValue::Point(xy(1.0, 2.0))),
        (
            "multipunto con un membro",
            WkbValue::MultiPoint(vec![geometria(WkbValue::Point(xy(1.0, 2.0)))]),
        ),
        (
            "polilinea di due coordinate",
            WkbValue::LineString(vec![xy(0.0, 0.0), xy(1.0, 1.0)]),
        ),
        (
            "multipolilinea con una parte",
            WkbValue::MultiLineString(vec![geometria(WkbValue::LineString(vec![
                xy(0.0, 0.0),
                xy(1.0, 1.0),
            ]))]),
        ),
        (
            "poligono con un anello chiuso",
            WkbValue::Polygon(vec![anello_valido()]),
        ),
        (
            "multipoligono con un membro",
            WkbValue::MultiPolygon(vec![geometria(WkbValue::Polygon(vec![anello_valido()]))]),
        ),
    ];

    for (nome, valore) in casi {
        assert!(
            topology_from_wkb(geometria(valore)).is_ok(),
            "«{nome}» doveva essere accettato"
        );
    }
}

/// Il primo anello e' l'esterno, gli altri sono interni.
///
/// E' l'unica affermazione **positiva** di `polygon_rings`, e senza di lei
/// la funzione potrebbe marcare tutti gli anelli allo stesso modo senza che
/// nessuna sonda se ne accorga: i rifiuti resterebbero tutti verdi.
#[test]
fn n1_polygon_rings_marca_esterno_solo_il_primo_anello() {
    let mut destinazione = Vec::new();
    polygon_rings(
        vec![anello_valido(), anello_valido(), anello_valido()],
        &mut destinazione,
    )
    .expect("tre anelli chiusi sono validi");

    let esterni: Vec<bool> = destinazione.iter().map(|(esterno, _)| *esterno).collect();
    assert_eq!(esterni, vec![true, false, false]);
}

/// `publish_mode`: ogni combinazione di destinazione e opzione, e il suo esito.
///
/// Il gruppo era censito come «ramo semantico negativo mai eseguito da
/// nulla». Da allora la forma sciolta e' diventata un opt-in esplicito, e
/// tre sonde ne provano i rifiuti passando dalla `create`; questa tabella
/// chiude il gruppo alla sua origine, chiamando la funzione direttamente.
///
/// Otto casi: le **sei** combinazioni fra i due suffissi supportati e i tre
/// stati dell'opzione, piu' **due** classi di destinazione invalida --
/// un'estensione estranea e un percorso senza estensione. Non e' «ogni
/// coppia possibile», che sarebbe piu' ampio di cio' che la tabella
/// enumera.
#[test]
fn n1_publish_mode_decide_per_ogni_coppia_di_destinazione_e_opzione() {
    let casi: Vec<(&str, &str, Option<&str>, Option<ShapefilePublishMode>)> = vec![
        (
            "*.shp.d senza opzione deduce il directory-dataset",
            "dati.shp.d",
            None,
            Some(ShapefilePublishMode::DirectoryDataset),
        ),
        (
            "*.shp.d con l'opzione che lo conferma",
            "dati.shp.d",
            Some(DIRECTORY_DATASET_MODE),
            Some(ShapefilePublishMode::DirectoryDataset),
        ),
        (
            "*.shp.d con l'opzione del set sciolto: contraddizione",
            "dati.shp.d",
            Some(LOOSE_SET_MODE),
            None,
        ),
        (
            "*.shp senza opzione: il set sciolto non si deduce",
            "dati.shp",
            None,
            None,
        ),
        (
            "*.shp con l'opt-in esplicito",
            "dati.shp",
            Some(LOOSE_SET_MODE),
            Some(ShapefilePublishMode::LooseSet),
        ),
        (
            "*.shp con l'opzione del directory-dataset: contraddizione",
            "dati.shp",
            Some(DIRECTORY_DATASET_MODE),
            None,
        ),
        (
            "un'estensione che non e' ne' l'una ne' l'altra",
            "dati.dbf",
            None,
            None,
        ),
        ("nessuna estensione", "dati", Some(LOOSE_SET_MODE), None),
    ];

    for (nome, destinazione, opzione, atteso) in casi {
        let mut opzioni = opzioni_scrittura();
        if let Some(valore) = opzione {
            opzioni = opzioni.with_format_option("publish_mode", valore);
        }
        let esito = publish_mode(Path::new(destinazione), &opzioni);
        match (esito, atteso) {
            (Ok(ottenuto), Some(voluto)) => assert_eq!(ottenuto, voluto, "«{nome}»"),
            (Err(_), None) => {}
            (Ok(ottenuto), None) => {
                panic!("«{nome}» doveva essere rifiutato, ha dato {ottenuto:?}")
            }
            (Err(errore), Some(_)) => panic!("«{nome}» doveva passare: {}", errore.message),
        }
    }
}

/// `shapefile_source_path`: i due rifiuti di una sorgente che e' una directory.
///
/// Un file qualunque passa senza domande -- e' il caso comune, e la
/// funzione non lo tocca. Una **directory** invece deve chiamarsi `*.shp.d`
/// e contenere `data.shp`, e i due rifiuti sono distinti: il primo dice che
/// la directory non e' un dataset, il secondo che lo e' ma e' incompleta.
#[test]
fn n1_shapefile_source_path_distingue_i_due_rifiuti_di_una_directory() {
    let radice = tempfile::tempdir().unwrap();

    let file = radice.path().join("dati.shp");
    std::fs::write(&file, b"non importa").unwrap();
    assert_eq!(
        shapefile_source_path(file.clone()).unwrap(),
        file,
        "un file non e' una directory: passa cosi' com'e'"
    );

    let inesistente = radice.path().join("assente.shp");
    assert_eq!(
        shapefile_source_path(inesistente.clone()).unwrap(),
        inesistente,
        "nemmeno un percorso inesistente e' una directory"
    );

    let non_dataset = radice.path().join("una-directory");
    std::fs::create_dir(&non_dataset).unwrap();
    let errore = shapefile_source_path(non_dataset)
        .expect_err("una directory senza il suffisso non e' un dataset");
    assert!(errore
        .message
        .contains("directory Shapefile non riconosciuta"));

    let vuoto = radice.path().join("vuoto.shp.d");
    std::fs::create_dir(&vuoto).unwrap();
    let errore =
        shapefile_source_path(vuoto.clone()).expect_err("un dataset senza data.shp e' incompleto");
    assert!(errore.message.contains("directory dataset senza data.shp"));

    std::fs::write(vuoto.join("data.shp"), b"non importa").unwrap();
    assert_eq!(
        shapefile_source_path(vuoto.clone()).unwrap(),
        vuoto.join("data.shp"),
        "con data.shp il dataset risolve al proprio marker"
    );
}

/// `declare_input_total`: il rifiuto del driver e' **irraggiungibile**, e
/// questa sonda dice da chi.
///
/// Il censimento lo dava «chiudibile con un test parametrico sulla classe
/// di equivalenza della sua precondizione». Scritto quel test, il messaggio
/// che arriva non e' quello di `ShpWriter` -- «Shapefile supporta un solo
/// layer» -- ma quello del wrapper comune, che confronta il layer con il
/// `WritePlan` **prima** di delegare.
///
/// Non e' un dettaglio da nota a pie' di pagina: significa che quelle due
/// righe del driver non sono raggiungibili dall'API pubblica, e che un test
/// che si accontentasse di `is_err()` le avrebbe dichiarate coperte
/// asserendo il rifiuto di **qualcun altro**. La sonda assevera percio' la
/// firma di chi rifiuta, non la sola presenza di un errore.
///
/// Il verso positivo sta accanto perche' senza di lui un
/// `declare_input_total` che rifiutasse **ogni** layer supererebbe la sonda
/// del rifiuto.
#[test]
fn n1_declare_input_total_e_fermato_dal_piano_prima_del_driver() {
    let radice = tempfile::tempdir().unwrap();
    let uscita = radice.path().join("punti.shp");
    let piano = piano_di_publish();
    let mut writer = ShpDriver
        .create(Sink::Path(uscita), &piano, &opzioni_scrittura_loose())
        .unwrap();

    writer
        .declare_input_total(LayerId(0), 3)
        .expect("il layer zero e' nel piano, e va accettato");

    for indice in [1_u32, 2, u32::MAX] {
        let Err(errore) = writer.declare_input_total(LayerId(indice), 3) else {
            panic!("il layer {indice} doveva essere rifiutato");
        };
        assert!(
                errore.message.contains("fuori dal WritePlan"),
                "layer {indice}: a rifiutare doveva essere il piano, non il driver;                  messaggio «{}»",
                errore.message
            );
        assert!(
                !errore.message.contains("Shapefile supporta un solo layer"),
                "layer {indice}: e' arrivato il rifiuto del driver, quindi la guardia                  del piano non lo precede piu' e il gruppo va rivisto"
            );
    }
}

/// `header_geometry`: ogni `ShapeType` che l'header puo' dichiarare.
///
/// Tredici forme si traducono, una sola no. La tabella le enumera **tutte**
/// invece di provare solo il rifiuto: un `header_geometry` che sbagliasse la
/// traduzione di `PolygonZ` -- dicendo `MultiLineString` invece di
/// `MultiPolygon` -- lascerebbe verde una sonda che guarda il solo
/// Multipatch, e produrrebbe un contratto di layer sbagliato su ogni
/// poligono 3D letto.
#[test]
fn n1_header_geometry_traduce_ogni_shape_type_e_rifiuta_multipatch() {
    let casi: Vec<(ShapeType, Option<&str>, Vec<GeometryType>)> = vec![
        (ShapeType::NullShape, None, Vec::new()),
        (
            ShapeType::Point,
            Some("point-xy"),
            vec![GeometryType::Point],
        ),
        (
            ShapeType::PointM,
            Some("point-m"),
            vec![GeometryType::Point],
        ),
        (
            ShapeType::PointZ,
            Some("point-z"),
            vec![GeometryType::Point],
        ),
        (
            ShapeType::Polyline,
            Some("polyline-xy"),
            vec![GeometryType::MultiLineString],
        ),
        (
            ShapeType::PolylineM,
            Some("polyline-m"),
            vec![GeometryType::MultiLineString],
        ),
        (
            ShapeType::PolylineZ,
            Some("polyline-z"),
            vec![GeometryType::MultiLineString],
        ),
        (
            ShapeType::Polygon,
            Some("polygon-xy"),
            vec![GeometryType::MultiPolygon],
        ),
        (
            ShapeType::PolygonM,
            Some("polygon-m"),
            vec![GeometryType::MultiPolygon],
        ),
        (
            ShapeType::PolygonZ,
            Some("polygon-z"),
            vec![GeometryType::MultiPolygon],
        ),
        (
            ShapeType::Multipoint,
            Some("multipoint-xy"),
            vec![GeometryType::MultiPoint],
        ),
        (
            ShapeType::MultipointM,
            Some("multipoint-m"),
            vec![GeometryType::MultiPoint],
        ),
        (
            ShapeType::MultipointZ,
            Some("multipoint-z"),
            vec![GeometryType::MultiPoint],
        ),
    ];

    for (shape_type, etichetta, tipi) in casi {
        let Ok((etichetta_letta, tipi_letti)) = header_geometry(shape_type) else {
            panic!("{shape_type:?} e' rappresentabile e non doveva essere rifiutato");
        };
        assert_eq!(etichetta_letta, etichetta, "etichetta di {shape_type:?}");
        assert_eq!(tipi_letti, tipi, "tipi geometrici di {shape_type:?}");
    }

    let Err(errore) = header_geometry(ShapeType::Multipatch) else {
        panic!("Multipatch non e' rappresentabile e deve essere rifiutato");
    };
    assert!(errore.message.contains("Multipatch"));
}

/// `resolve_crs`: le quattro vie di un CRS, e le due che finiscono in errore.
///
/// La funzione decide su due assi -- il `.prj` c'e' o no, `--assume-crs` c'e'
/// o no -- e i quattro incroci non danno quattro esiti uguali: con il `.prj`
/// l'identificatore puo' arrivare dall'opzione **o** dal WKT, e se non
/// arriva da nessuno dei due il rifiuto e' `CrsUnresolved`, che e' diverso
/// dal rifiuto senza `.prj`.
#[test]
fn n1_resolve_crs_copre_i_quattro_incroci_di_prj_e_assume_crs() {
    let radice = tempfile::tempdir().unwrap();
    let shp = radice.path().join("dati.shp");
    std::fs::write(&shp, b"non letto da resolve_crs").unwrap();
    let prj = radice.path().join("dati.prj");

    // 1. niente `.prj`, niente `--assume-crs`: rifiuto esplicito.
    let Err(errore) = resolve_crs(&shp, &opzioni_lettura()) else {
        panic!("senza .prj e senza --assume-crs il CRS non e' deducibile");
    };
    assert!(errore.message.contains("senza .prj"));

    // 2. niente `.prj`, ma `--assume-crs`: l'opzione decide.
    let mut con_opzione = opzioni_lettura();
    con_opzione.assume_crs = Some("EPSG:4326".to_owned());
    let risolto = resolve_crs(&shp, &con_opzione).expect("l'opzione risolve da sola");
    assert_eq!(risolto.id.as_deref(), Some("EPSG:4326"));
    assert!(
        risolto.definition.is_none(),
        "senza .prj non c'e' WKT da conservare"
    );

    // 3. `.prj` con un WKT da cui l'autorita' si ricava.
    std::fs::write(&prj, "GEOGCS[\"WGS 84\",AUTHORITY[\"EPSG\",\"4326\"]]").unwrap();
    let risolto = resolve_crs(&shp, &opzioni_lettura()).expect("l'autorita' e' nel WKT");
    assert_eq!(risolto.id.as_deref(), Some("EPSG:4326"));
    assert!(
        risolto.definition.is_some(),
        "il WKT del .prj va conservato: e' la fonte"
    );

    // 4. `.prj` con un WKT senza autorita' e senza opzione: non risolto.
    std::fs::write(&prj, "LOCAL_CS[\"senza autorita\"]").unwrap();
    let Err(errore) = resolve_crs(&shp, &opzioni_lettura()) else {
        panic!("un WKT senza autorita' e senza opzione non e' risolvibile");
    };
    assert_eq!(errore.code, plenora_io_model::IoErrorCode::CrsUnresolved);

    // 5. lo stesso WKT, ma con `--assume-crs`: l'opzione vince sul silenzio
    //    del WKT, e il WKT resta conservato accanto.
    let risolto = resolve_crs(&shp, &con_opzione).expect("l'opzione copre il WKT muto");
    assert_eq!(risolto.id.as_deref(), Some("EPSG:4326"));
    assert!(risolto.definition.is_some());
}

/// Il piano minimo delle prove sul publish: una sola colonna geometria.
fn piano_di_publish() -> WritePlan {
    let schema: SchemaRef = Arc::new(Schema::new(vec![geometry_field(GEOMETRY, "EPSG:4326")]));
    WritePlan {
        layers: vec![WriteLayer {
            name: "points".to_owned(),
            contract: DataContract {
                schema,
                geometry: None,
            },
        }],
    }
}

/// Una destinazione `*.shp` non deduce piu' la forma debole: la pretende.
///
/// La sonda guarda due cose insieme, e la seconda conta quanto la prima: il
/// rifiuto arriva **prima** che qualunque cosa tocchi il disco. Un rifiuto
/// che lasciasse dietro di se' uno staging avrebbe gia' fatto il danno che
/// esiste per evitare.
#[test]
fn una_destinazione_shp_senza_opt_in_e_rifiutata_prima_dello_staging() {
    let root = tempfile::tempdir().unwrap();

    let errore = ShpDriver
        .create(
            Sink::Path(root.path().join("points.shp")),
            &piano_di_publish(),
            &opzioni_scrittura(),
        )
        .map(|_| ())
        .unwrap_err();

    // `InvalidConfiguration` e non `Unsupported`: il prodotto sa fare questa
    // scrittura, e' la richiesta a essere incompleta. Chi automatizza deve
    // aggiungere un'opzione, non cambiare driver.
    assert_eq!(
        errore.category,
        plenora_io_model::ErrorCategory::InvalidConfiguration
    );
    assert_eq!(errore.phase, plenora_io_model::ErrorPhase::Validate);
    // Il messaggio nomina entrambe le uscite: quella che accetta il rischio
    // e quella che non lo corre.
    assert!(errore.message.contains(LOOSE_SET_MODE));
    assert!(errore.message.contains(".shp.d"));
    assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
}

/// Accettata l'opzione, il set sciolto si pubblica come prima.
///
/// Il rifiuto e' una domanda, non un divieto: chi risponde ottiene i quattro
/// file dove li ha chiesti.
#[test]
fn una_destinazione_shp_con_opt_in_pubblica_il_set_sciolto() {
    let root = tempfile::tempdir().unwrap();
    let uscita = root.path().join("points.shp");
    let piano = piano_di_publish();
    let batch = RecordBatch::new_empty(piano.layers[0].contract.schema.clone());

    let mut writer = ShpDriver
        .create(
            Sink::Path(uscita.clone()),
            &piano,
            &opzioni_scrittura_loose(),
        )
        .unwrap();
    writer.write(&batch).unwrap();
    writer.finish().unwrap();

    assert!(uscita.is_file());
    assert!(uscita.with_extension("shx").is_file());
    assert!(uscita.with_extension("dbf").is_file());
    // Nessuna directory di staging sopravvive al publish.
    assert!(std::fs::read_dir(root.path())
        .unwrap()
        .all(|voce| voce.unwrap().path().is_file()));
}

/// La contraddizione e' un errore anche nel verso opposto.
///
/// Chiedere il set sciolto su una destinazione `*.shp.d` non e' «accettare il
/// rischio»: e' descrivere male la destinazione, e vale il rifiuto come il
/// verso gia' coperto.
#[test]
fn il_set_sciolto_chiesto_su_una_directory_dataset_e_rifiutato() {
    let root = tempfile::tempdir().unwrap();

    let errore = ShpDriver
        .create(
            Sink::Path(root.path().join("points.shp.d")),
            &piano_di_publish(),
            &opzioni_scrittura_loose(),
        )
        .map(|_| ())
        .unwrap_err();

    assert_eq!(errore.code, plenora_io_model::IoErrorCode::Unsupported);
    // Il messaggio nomina il suffisso che il modo **chiesto** pretende, e
    // deve nominare quello: `contains("*.shp")` da solo sarebbe vero anche
    // per `*.shp.d`, cioe' non distinguerebbe i due versi del rifiuto.
    assert!(errore.message.ends_with("*.shp"), "{}", errore.message);
    assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
}

#[test]
fn publish_mode_must_match_destination_shape() {
    let root = tempfile::tempdir().unwrap();
    let schema: SchemaRef = Arc::new(Schema::new(vec![geometry_field(GEOMETRY, "EPSG:4326")]));
    let plan = WritePlan {
        layers: vec![WriteLayer {
            name: "points".to_owned(),
            contract: DataContract {
                schema,
                geometry: None,
            },
        }],
    };
    let mut options = opzioni_scrittura();
    options
        .format_options
        .insert("publish_mode".to_owned(), DIRECTORY_DATASET_MODE.to_owned());

    let result = ShpDriver
        .create(Sink::Path(root.path().join("points.shp")), &plan, &options)
        .map(|_| ());

    assert!(matches!(
        result,
        Err(error) if error.code == plenora_io_model::IoErrorCode::Unsupported
    ));
    assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
}

#[test]
fn rejects_long_field_name() {
    use arrow_schema::DataType;
    let schema: SchemaRef = Arc::new(Schema::new(vec![
        geometry_field(GEOMETRY, "EPSG:4326"),
        Field::new("nome_campo_troppo_lungo", DataType::Utf8, true),
    ]));
    let driver = ShpDriver;
    let plan = WritePlan {
        layers: vec![WriteLayer {
            name: "l".to_owned(),
            contract: DataContract {
                schema,
                geometry: None,
            },
        }],
    };
    let dir = tempfile::tempdir().unwrap();
    let e = driver
        .create(
            Sink::Path(dir.path().join("x.shp")),
            &plan,
            &opzioni_scrittura(),
        )
        .map(|_| ())
        .unwrap_err();
    assert_eq!(
        e.capability_reason,
        Some(plenora_io_model::CapabilityReason::FieldNameTooLong)
    );
}

#[test]
fn streams_multiple_batches() {
    use arrow_array::Int64Array;
    use arrow_schema::DataType;

    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("many.shp");
    let wkb: Vec<Vec<u8>> = (0..10)
        .map(|i| {
            to_wkb(&geo_types::Geometry::Point(geo_types::Point::new(
                f64::from(i),
                f64::from(i),
            )))
            .unwrap()
        })
        .collect();
    let schema: SchemaRef = Arc::new(Schema::new(vec![
        geometry_field(GEOMETRY, "EPSG:4326"),
        Field::new("id", DataType::Int64, true),
    ]));
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(BinaryArray::from(
                wkb.iter().map(|w| Some(w.as_slice())).collect::<Vec<_>>(),
            )),
            Arc::new(Int64Array::from((0..10i64).collect::<Vec<_>>())),
        ],
    )
    .unwrap();

    let driver = ShpDriver;
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
        .create(Sink::Path(out.clone()), &plan, &opzioni_scrittura_loose())
        .unwrap();
    w.write(&batch).unwrap();
    w.finish().unwrap();

    let ds = driver.open(Source::Path(out), read_opts()).unwrap();
    let req = ReadRequest {
        layer: LayerId(0),
        projected_fields: None,
        projection_mode: ProjectionMode::BestEffort,
        pruning_predicate: None,
        spatial_pruning_hint: None,
        scope: ReadScope::default(),
        batch_target: BatchTarget {
            target_bytes: 8 * 1024 * 1024,
            max_rows: 4,
        },
        cancellation: CancellationToken::default(),
    };
    let mut r = ds.open_layer_reader(&req).unwrap();
    let (mut total, mut batches) = (0, 0);
    while let Some(b) = r.next_batch().unwrap() {
        total += b.num_rows();
        batches += 1;
    }
    assert_eq!(total, 10);
    assert!(
        batches >= 3,
        "atteso streaming multi-batch, avuti {batches}"
    );
}

fn dimensional_point(
    dimensions: CoordinateDimensions,
    z: Option<f64>,
    m: Option<f64>,
) -> WkbGeometry {
    WkbGeometry {
        value: WkbValue::Point(WkbCoordinate {
            x: 12.5,
            y: 45.9,
            z,
            m,
        }),
        dimensions,
        srid: None,
    }
}

fn round_trip_dimensional_point(
    dimensions: CoordinateDimensions,
    geometry: &WkbGeometry,
) -> WkbGeometry {
    use arrow_array::Int64Array;

    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join(format!("{dimensions:?}.shp"));
    let bytes = encode_wkb(geometry, WkbFlavor::Iso).unwrap();
    let mut geometry_contract =
        GeometryColumnContract::wkb_xy(FieldId(0), GEOMETRY, ResolvedCrs::wgs84(), false);
    geometry_contract.dimensions = dimensions;
    geometry_contract.set_exact_geometry_types(vec![GeometryType::Point]);
    let schema: SchemaRef = Arc::new(Schema::new(vec![
        with_geometry_contract_metadata(&geometry_field(GEOMETRY, "EPSG:4326"), &geometry_contract),
        Field::new("id", DataType::Int64, false),
    ]));
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(BinaryArray::from(vec![Some(bytes.as_slice())])),
            Arc::new(Int64Array::from(vec![1_i64])),
        ],
    )
    .unwrap();
    let plan = WritePlan {
        layers: vec![WriteLayer {
            name: "points".to_owned(),
            contract: DataContract {
                schema,
                geometry: Some(geometry_contract),
            },
        }],
    };

    let driver = ShpDriver;
    let mut writer = driver
        .create(Sink::Path(out.clone()), &plan, &opzioni_scrittura_loose())
        .unwrap();
    writer.write(&batch).unwrap();
    writer.finish().unwrap();

    let mut native = shapefile::Reader::from_path(&out).unwrap();
    let (shape, _) = native.iter_shapes_and_records().next().unwrap().unwrap();
    match dimensions {
        CoordinateDimensions::Xym => assert!(matches!(shape, Shape::PointM(_))),
        CoordinateDimensions::Xyz | CoordinateDimensions::Xyzm => {
            assert!(matches!(shape, Shape::PointZ(_)));
        }
        _ => unreachable!("test solo dimensionale"),
    }

    let dataset = driver.open(Source::Path(out), read_opts()).unwrap();
    let layer = &dataset.layers()[0];
    let output_contract = layer.contract.geometry.as_ref().unwrap();
    assert_eq!(output_contract.dimensions, dimensions);
    assert_eq!(output_contract.geometry_types, vec![GeometryType::Point]);
    assert!(output_contract
        .native_metadata
        .contains_key("shp.shape_type"));
    let mut reader = dataset.open_layer_reader(&req()).unwrap();
    let batch = reader.next_batch().unwrap().unwrap();
    let geometry = batch
        .column(0)
        .as_any()
        .downcast_ref::<BinaryArray>()
        .unwrap();
    decode_wkb(geometry.value(0), &WkbLimits::default()).unwrap()
}

#[test]
fn round_trip_preserves_xyz_xym_and_xyzm_points() {
    let cases = [
        dimensional_point(CoordinateDimensions::Xyz, Some(123.25), None),
        dimensional_point(CoordinateDimensions::Xym, None, Some(7.5)),
        dimensional_point(CoordinateDimensions::Xyzm, Some(123.25), Some(7.5)),
    ];
    for expected in cases {
        let actual = round_trip_dimensional_point(expected.dimensions, &expected);
        assert_eq!(actual, expected);
    }
}

#[test]
fn direct_conversion_preserves_xyzm_multiline_and_no_data_measure() {
    let dimensions = CoordinateDimensions::Xyzm;
    let line = WkbGeometry {
        value: WkbValue::MultiLineString(vec![WkbGeometry {
            value: WkbValue::LineString(vec![
                WkbCoordinate {
                    x: 0.0,
                    y: 1.0,
                    z: Some(2.0),
                    m: Some(NO_DATA),
                },
                WkbCoordinate {
                    x: 3.0,
                    y: 4.0,
                    z: Some(5.0),
                    m: Some(6.0),
                },
            ]),
            dimensions,
            srid: None,
        }]),
        dimensions,
        srid: None,
    };
    let shape = shape_from_wkb(line.clone()).unwrap();
    assert!(matches!(shape, Shape::PolylineZ(_)));
    let decoded = shape_to_wkb(&shape, dimensions).unwrap().unwrap();
    assert_eq!(decoded, line);
}

#[test]
fn direct_conversion_preserves_xyzm_multipolygon_rings() {
    let dimensions = CoordinateDimensions::Xyzm;
    let coordinate = |x, y, z, m| WkbCoordinate {
        x,
        y,
        z: Some(z),
        m: Some(m),
    };
    let exterior = vec![
        coordinate(0.0, 0.0, 1.0, 10.0),
        coordinate(0.0, 5.0, 2.0, 11.0),
        coordinate(5.0, 5.0, 3.0, 12.0),
        coordinate(5.0, 0.0, 4.0, 13.0),
        coordinate(0.0, 0.0, 1.0, 10.0),
    ];
    let interior = vec![
        coordinate(1.0, 1.0, 5.0, 14.0),
        coordinate(4.0, 1.0, 6.0, 15.0),
        coordinate(4.0, 4.0, 7.0, 16.0),
        coordinate(1.0, 4.0, 8.0, 17.0),
        coordinate(1.0, 1.0, 5.0, 14.0),
    ];
    let polygon = WkbGeometry {
        value: WkbValue::MultiPolygon(vec![WkbGeometry {
            value: WkbValue::Polygon(vec![exterior, interior]),
            dimensions,
            srid: None,
        }]),
        dimensions,
        srid: None,
    };
    let shape = shape_from_wkb(polygon.clone()).unwrap();
    assert!(matches!(shape, Shape::PolygonZ(_)));
    let decoded = shape_to_wkb(&shape, dimensions).unwrap().unwrap();
    assert_eq!(decoded, polygon);
}

#[test]
fn geometry_collection_is_rejected_without_xy_normalization() {
    let geometry = WkbGeometry {
        value: WkbValue::GeometryCollection(Vec::new()),
        dimensions: CoordinateDimensions::Xyz,
        srid: None,
    };
    assert!(shape_from_wkb(geometry).is_err());
}

#[test]
fn declared_dimensions_without_required_ordinates_are_rejected() {
    let missing_z = WkbGeometry {
        value: WkbValue::Point(WkbCoordinate {
            x: 1.0,
            y: 2.0,
            z: None,
            m: None,
        }),
        dimensions: CoordinateDimensions::Xyz,
        srid: None,
    };
    let missing_m = WkbGeometry {
        value: WkbValue::Point(WkbCoordinate {
            x: 1.0,
            y: 2.0,
            z: Some(3.0),
            m: None,
        }),
        dimensions: CoordinateDimensions::Xyzm,
        srid: None,
    };

    assert!(shape_from_wkb(missing_z).is_err());
    assert!(shape_from_wkb(missing_m).is_err());
}

// --- ASSURANCE-N1: le funzioni pure della conversione -----------------

/// Un vertice nativo costruito a mano, per provare `native_coordinate`
/// sulle combinazioni che i tipi di `shapefile` non permettono di formare.
///
/// `Point`, `PointM` e `PointZ` hanno ciascuno le proprie ordinate fissate
/// dal tipo: da loro non si ottiene uno `ShapeZ` senza quota, che e' proprio
/// il caso che i rifiuti esistono per prendere. Il tratto e' privato del
/// modulo, quindi la sonda lo implementa dove vive.
#[derive(Clone, Copy)]
struct VerticeFinto {
    z: Option<f64>,
    m: Option<f64>,
}

impl NativePoint for VerticeFinto {
    fn x(&self) -> f64 {
        1.0
    }
    fn y(&self) -> f64 {
        2.0
    }
    fn z(&self) -> Option<f64> {
        self.z
    }
    fn m(&self) -> Option<f64> {
        self.m
    }
}

const fn vertice(z: Option<f64>, m: Option<f64>) -> VerticeFinto {
    VerticeFinto { z, m }
}

/// `native_coordinate` esige le ordinate che la dimensionalita' dichiara, e
/// nomina quale manca.
///
/// La dimensionalita' non e' una proprieta' del singolo vertice: viene dal
/// tipo di shape dichiarato nell'header, e vale per tutto il layer. Un
/// vertice che non la rispetta e' un file incoerente con la propria
/// intestazione, e i cinque messaggi distinti dicono **quale** ordinata
/// manca invece di «geometria non valida»: chi ripara il dato deve sapere
/// se aggiungere la quota o la misura.
///
/// Il caso piu' sottile e' l'ultimo dei rifiuti: un dataset `ShapeZ`
/// dichiarato XYZ che porta una misura **valida**. Non e' un dato assente
/// ma un dato in piu', e accettarlo lo butterebbe via in silenzio; il
/// confronto e' con `NO_DATA`, perche' nel formato la misura assente e' un
/// valore, non l'assenza del campo.
// Nove rifiuti e cinque accettazioni in una tabella sola: separarle in
// due test spezzerebbe la coppia che le rende interpretabili -- una
// tabella di soli negativi la passa anche una funzione che rifiuta tutto.
#[allow(clippy::too_many_lines)]
#[test]
fn n1_native_coordinate_esige_le_ordinate_che_la_dimensionalita_dichiara() {
    for (caso, punto, dimensioni, atteso) in [
        (
            "XYM senza misura",
            vertice(None, None),
            CoordinateDimensions::Xym,
            "coordinata ShapeM senza misura",
        ),
        (
            "XYZ senza quota",
            vertice(None, Some(NO_DATA)),
            CoordinateDimensions::Xyz,
            "coordinata ShapeZ senza quota",
        ),
        (
            "XYZ con una misura valida",
            vertice(Some(3.0), Some(7.0)),
            CoordinateDimensions::Xyz,
            "misura valida trovata in un dataset ShapeZ dichiarato XYZ",
        ),
        (
            "XYZM senza quota",
            vertice(None, Some(7.0)),
            CoordinateDimensions::Xyzm,
            "coordinata ShapeZ senza quota",
        ),
        (
            "XYZM senza misura",
            vertice(Some(3.0), None),
            CoordinateDimensions::Xyzm,
            "coordinata ShapeZ senza misura nativa",
        ),
        (
            "dimensionalita' non determinata",
            vertice(None, None),
            CoordinateDimensions::Unknown,
            "dimensionalità Shapefile non determinata",
        ),
        (
            "XY con una quota che non dovrebbe esserci",
            vertice(Some(3.0), None),
            CoordinateDimensions::Xy,
            "variante Shape incoerente con la dimensionalità del layer",
        ),
        (
            "XY con una misura che non dovrebbe esserci",
            vertice(None, Some(7.0)),
            CoordinateDimensions::Xy,
            "variante Shape incoerente con la dimensionalità del layer",
        ),
        (
            "XYM con una quota che non dovrebbe esserci",
            vertice(Some(3.0), Some(7.0)),
            CoordinateDimensions::Xym,
            "variante Shape incoerente con la dimensionalità del layer",
        ),
    ] {
        let Err(errore) = native_coordinate(&punto, dimensioni) else {
            panic!("{caso}: doveva essere rifiutato");
        };
        assert_eq!(errore.message, atteso, "{caso}: messaggio sbagliato");
    }

    // Le accettazioni, e cio' che ciascuna porta con se': senza, una
    // funzione che rifiutasse tutto supererebbe la tabella dei negativi, e
    // una che scartasse le ordinate passerebbe una tabella che guardasse
    // solo `is_ok`.
    for (caso, punto, dimensioni, z, m) in [
        (
            "XY puro",
            vertice(None, None),
            CoordinateDimensions::Xy,
            None,
            None,
        ),
        (
            "XYM con misura",
            vertice(None, Some(7.0)),
            CoordinateDimensions::Xym,
            None,
            Some(7.0),
        ),
        (
            "XYZ con quota e misura assente per convenzione",
            vertice(Some(3.0), Some(NO_DATA)),
            CoordinateDimensions::Xyz,
            Some(3.0),
            None,
        ),
        (
            "XYZ con quota e nessuna misura",
            vertice(Some(3.0), None),
            CoordinateDimensions::Xyz,
            Some(3.0),
            None,
        ),
        (
            "XYZM completo",
            vertice(Some(3.0), Some(7.0)),
            CoordinateDimensions::Xyzm,
            Some(3.0),
            Some(7.0),
        ),
    ] {
        let coordinata = match native_coordinate(&punto, dimensioni) {
            Ok(coordinata) => coordinata,
            Err(errore) => panic!("{caso}: doveva essere accettato: {errore:?}"),
        };
        assert_eq!(
            (coordinata.z, coordinata.m),
            (z, m),
            "{caso}: ordinate perse"
        );
        assert!(
            (coordinata.x - 1.0).abs() < f64::EPSILON && (coordinata.y - 2.0).abs() < f64::EPSILON,
            "{caso}: X e Y devono passare invariate"
        );
    }
}

/// `polygon_wkb` esige un anello esterno prima di ogni interno, e almeno
/// uno in tutto.
///
/// Nel formato Shapefile gli anelli sono una sequenza piatta, e la
/// gerarchia esiste solo nell'ordine: un anello interno appartiene
/// all'ultimo esterno visto. Un file che apre con un interno non descrive
/// un buco in niente, e la funzione non ha modo di indovinare a chi
/// appartenga -- accettarlo significherebbe scegliere un contenitore a
/// caso.
///
/// Le due accettazioni fissano proprio la gerarchia: un esterno con un
/// interno diventa un poligono di due anelli, due esterni diventano due
/// poligoni. Senza, uno scambio fra i due rami non romperebbe nulla.
#[test]
fn n1_polygon_wkb_esige_un_anello_esterno_prima_di_ogni_interno() {
    let anello = |chiuso: bool| {
        let z = if chiuso { None } else { Some(3.0) };
        vec![vertice(z, None), vertice(z, None), vertice(z, None)]
    };

    let Err(errore) = polygon_wkb(
        &[PolygonRing::Inner(anello(true))],
        CoordinateDimensions::Xy,
    ) else {
        panic!("un anello interno senza esterno non appartiene a niente");
    };
    assert_eq!(
        errore.message,
        "anello interno Shapefile senza anello esterno"
    );

    let vuoti: [PolygonRing<VerticeFinto>; 0] = [];
    let Err(errore) = polygon_wkb(&vuoti, CoordinateDimensions::Xy) else {
        panic!("un poligono senza anelli non e' un poligono");
    };
    assert_eq!(errore.message, "Polygon Shapefile senza anelli esterni");

    // Un esterno con il suo interno: un poligono, due anelli.
    let uno = polygon_wkb(
        &[
            PolygonRing::Outer(anello(true)),
            PolygonRing::Inner(anello(true)),
        ],
        CoordinateDimensions::Xy,
    )
    .expect("un esterno con il suo interno e' un poligono con un buco");
    let WkbValue::MultiPolygon(poligoni) = uno.value else {
        panic!("il risultato e' sempre un MultiPolygon");
    };
    assert_eq!(poligoni.len(), 1, "un solo esterno, un solo poligono");
    let WkbValue::Polygon(corona) = &poligoni[0].value else {
        panic!("il membro e' un poligono");
    };
    assert_eq!(corona.len(), 2, "l'interno deve stare dentro l'esterno");

    // Due esterni: due poligoni, non un poligono con due anelli.
    let due = polygon_wkb(
        &[
            PolygonRing::Outer(anello(true)),
            PolygonRing::Outer(anello(true)),
        ],
        CoordinateDimensions::Xy,
    )
    .expect("due esterni sono due poligoni");
    let WkbValue::MultiPolygon(poligoni) = due.value else {
        panic!("il risultato e' sempre un MultiPolygon");
    };
    assert_eq!(poligoni.len(), 2, "due esterni non si fondono in uno");

    // La propagazione da `native_coordinates`: un vertice incoerente con
    // la dimensionalita' ferma la conversione invece di perdere l'ordinata.
    let Err(errore) = polygon_wkb(
        &[PolygonRing::Outer(anello(false))],
        CoordinateDimensions::Xy,
    ) else {
        panic!("un vertice con una quota in un layer XY e' incoerente");
    };
    assert_eq!(
        errore.message,
        "variante Shape incoerente con la dimensionalità del layer"
    );
}

/// `shape_to_wkb` traduce l'assenza in `None` e rifiuta il Multipatch.
///
/// `NullShape` e' l'assenza di geometria dichiarata dal formato, non un
/// errore: diventa `Ok(None)`, che a valle e' una riga senza geometria.
/// Il Multipatch invece e' una forma che WKB non sa rappresentare in modo
/// univoco -- le sue parti hanno una semantica di superficie che il modello
/// piatto non porta -- e tradurlo comunque significherebbe scegliere una
/// delle letture possibili e non dirlo.
#[test]
fn n1_shape_to_wkb_rende_nulla_la_nullshape_e_rifiuta_il_multipatch() {
    assert!(
        shape_to_wkb(&Shape::NullShape, CoordinateDimensions::Xy)
            .expect("la NullShape non e' un errore")
            .is_none(),
        "la NullShape e' assenza di geometria, non un fallimento"
    );

    let multipatch = Shape::Multipatch(shapefile::Multipatch::new(
        shapefile::Patch::TriangleStrip(vec![
            shapefile::PointZ::new(0.0, 0.0, 0.0, NO_DATA),
            shapefile::PointZ::new(1.0, 0.0, 0.0, NO_DATA),
            shapefile::PointZ::new(0.0, 1.0, 0.0, NO_DATA),
        ]),
    ));
    let Err(errore) = shape_to_wkb(&multipatch, CoordinateDimensions::Xyz) else {
        panic!("il Multipatch non ha una traduzione WKB univoca");
    };
    assert_eq!(
        errore.message,
        "Multipatch non ha una conversione WKB univoca ed è rifiutato"
    );

    // Il controllo positivo: una forma traducibile passa e conserva la
    // dimensionalita' dichiarata.
    let punto = shape_to_wkb(
        &Shape::Point(shapefile::Point::new(1.0, 2.0)),
        CoordinateDimensions::Xy,
    )
    .expect("un punto XY si traduce")
    .expect("un punto non e' un'assenza");
    assert_eq!(punto.dimensions, CoordinateDimensions::Xy);
}

/// `shape_from_wkb` rifiuta la dimensionalita' ignota, ed e' la ragione per
/// cui `__fuzz_wkb_roundtrip` non puo' vedere una `NullShape`.
///
/// La funzione decide su una coppia (dimensionalita', topologia), e ogni
/// braccio costruisce una shape **concreta**: nessuno produce `NullShape`.
/// Percio' il rifiuto «la conversione di una geometria ha prodotto
/// `NullShape`» dentro il target del fuzzer non ha input -- `shape_to_wkb`
/// restituisce `None` solo per `NullShape`, e li' non ci puo' arrivare.
///
/// La sonda esegue quella precondizione su ogni combinazione che il target
/// puo' formare, invece di argomentarla: se un braccio futuro restituisse
/// `NullShape`, diventa rossa.
#[test]
fn n1_shape_from_wkb_non_produce_mai_nullshape_e_rifiuta_la_dimensionalita_ignota() {
    let coordinata = |z: Option<f64>, m: Option<f64>| WkbCoordinate {
        x: 1.0,
        y: 2.0,
        z,
        m,
    };
    let ordinate = |dimensioni: CoordinateDimensions| match dimensioni {
        CoordinateDimensions::Xy => coordinata(None, None),
        CoordinateDimensions::Xym => coordinata(None, Some(7.0)),
        CoordinateDimensions::Xyz => coordinata(Some(3.0), None),
        _ => coordinata(Some(3.0), Some(7.0)),
    };

    for dimensioni in [
        CoordinateDimensions::Xy,
        CoordinateDimensions::Xym,
        CoordinateDimensions::Xyz,
        CoordinateDimensions::Xyzm,
    ] {
        let vertice = ordinate(dimensioni);
        // I membri di un multi- sono geometrie, non coordinate: il modello
        // porta la dimensionalita' su ciascuna, e costruirli a mano e'
        // l'unico modo di formare le sedici coppie senza passare da un file.
        let membro = |valore| WkbGeometry {
            value: valore,
            dimensions: dimensioni,
            srid: None,
        };
        for (caso, valore) in [
            ("punto", WkbValue::Point(vertice)),
            (
                "multipunto",
                WkbValue::MultiPoint(vec![membro(WkbValue::Point(vertice))]),
            ),
            (
                "linea",
                WkbValue::MultiLineString(vec![membro(WkbValue::LineString(vec![
                    vertice, vertice,
                ]))]),
            ),
            (
                "poligono",
                WkbValue::MultiPolygon(vec![membro(WkbValue::Polygon(vec![vec![
                    vertice, vertice, vertice, vertice,
                ]]))]),
            ),
        ] {
            let shape = match shape_from_wkb(WkbGeometry {
                value: valore,
                dimensions: dimensioni,
                srid: None,
            }) {
                Ok(shape) => shape,
                Err(errore) => panic!("{caso} {dimensioni:?}: doveva convertirsi: {errore:?}"),
            };
            assert!(
                !matches!(shape, Shape::NullShape),
                "{caso} {dimensioni:?}: se questo fallisse, il rifiuto sulla NullShape \
                     dentro __fuzz_wkb_roundtrip diventerebbe raggiungibile"
            );
        }
    }

    // La dimensionalita' ignota: l'unico rifiuto proprio della funzione.
    let Err(errore) = shape_from_wkb(WkbGeometry {
        value: WkbValue::Point(coordinata(None, None)),
        dimensions: CoordinateDimensions::Unknown,
        srid: None,
    }) else {
        panic!("una dimensionalita' ignota non si scrive in Shapefile");
    };
    assert_eq!(
        errore.message,
        "dimensionalità WKB ignota non scrivibile in Shapefile"
    );
}

/// Un bundle in cui il `.dbf` conta piu' righe di quante geometrie abbia il
/// `.shp`.
///
/// Nessun produttore conforme lo scrive, e all'apertura non passerebbe:
/// `infer_geometry_info` confronta i due conteggi e rifiuta. Serve percio'
/// a raggiungere il parser **saltando** quel confronto, che e' l'unico modo
/// di eseguire la difesa che il parser ha per lo stesso caso -- e che esiste
/// perche' fra l'apertura e la lettura il file puo' essere cambiato.
///
/// Le due meta' vengono da due bundle scritti dal writer vero: il `.shp` e
/// lo `.shx` da quello corto, il `.dbf` da quello lungo. Nessun byte e'
/// costruito a mano, quindi l'unica incoerenza e' quella voluta.
fn bundle_disallineato(dir: &Path, nome: &str, geometrie: usize, record: usize) -> PathBuf {
    let corto = bundle_di_righe(dir, &format!("corto-{nome}"), &["NOME"], geometrie, |_| {});
    let lungo = bundle_di_righe(dir, &format!("lungo-{nome}"), &["NOME"], record, |_| {});
    std::fs::copy(lungo.with_extension("dbf"), corto.with_extension("dbf"))
        .expect("il dbf lungo prende il posto di quello corto");
    corto
}

/// `spawn_parser` si ferma quando le geometrie finiscono prima dei record,
/// e i due contatori non possono traboccare.
///
/// # Perche' passa dalla costruzione diretta dell'ingresso
///
/// Dal driver questa difesa non ha input: `infer_geometry_info` confronta
/// geometrie e record all'apertura e rifiuta prima. Resta nel parser perche'
/// fra l'apertura e la lettura il file puo' cambiare, e allora la difesa e'
/// l'ultima cosa che sta fra un troncamento e una riga con gli attributi di
/// un'altra. `ShpParserInput` e' privato del modulo, quindi la sonda lo
/// costruisce dove vive.
///
/// # I due contatori
///
/// «numero di record Shapefile fuori intervallo u64» e «numero di record DBF
/// attivi fuori intervallo u64» non hanno input, e non serve una guardia per
/// dirlo: il ciclo gira al piu' `record_count` volte, e `record_count` e' un
/// `u32` letto dall'header. Un contatore `u64` che parte da zero e cresce di
/// uno per giro arriva al massimo a `u32::MAX`.
///
/// La difesa simmetrica -- i record che finiscono prima delle geometrie --
/// e' chiusa altrove: pretenderebbe che l'iteratore di `dbase` si esaurisse
/// prima del lettore raw, e le due strade per farlo sono percorse e chiuse
/// in `n1_le_due_letture_del_dbf_non_possono_divergere`.
#[test]
fn n1_spawn_parser_si_ferma_quando_le_geometrie_finiscono_prima_dei_record() {
    let dir = tempfile::tempdir().unwrap();
    let bundle = bundle_disallineato(dir.path(), "misto.shp", 1, 3);
    let layout = read_dbf_layout(&bundle).expect("il dbf lungo e' ben formato");
    assert_eq!(layout.record_count, 3, "il dbf porta tre record");

    let schema: SchemaRef = Arc::new(Schema::new(vec![geometry_field(GEOMETRY, "EPSG:4326")]));
    let layer = LayerContract {
        id: LayerId(0),
        name: "misto".to_owned(),
        contract: DataContract::new(schema.clone(), None),
    };
    let ingresso = ShpParserInput {
        path: bundle,
        schema: schema.clone(),
        cols: Vec::new(),
        dbf_layout: layout.clone(),
        dimensions: CoordinateDimensions::Xy,
        expected_shape_type: Some("point-xy"),
        expected_active_rows: u64::from(layout.record_count),
        include_geometry: true,
        batch_sizer: plenora_io_core::AdaptiveBatchSizer::new(&schema, BatchTarget::default()),
        layer,
        loss: LossReport::default(),
        row_diagnostics: ShpRowDiagnosticsConfig::from_options(&BTreeMap::new(), &[], &layout)
            .expect("nessuna opzione di diagnostica"),
        scope: ReadScope::Complete,
        cancellation: plenora_io_model::CancellationToken::new(),
    };

    let mut lettore = spawn_parser(ingresso).expect("le validazioni pre-thread passano");
    let mut errore = None;
    loop {
        match lettore.next_batch() {
            Ok(Some(_)) => {}
            Ok(None) => break,
            Err(e) => {
                errore = Some(e);
                break;
            }
        }
    }
    let Some(errore) = errore else {
        panic!("tre record contro una geometria non sono un allineamento");
    };
    assert_eq!(
        errore.message, "numero di geometrie incoerente con i record DBF",
        "il messaggio deve dire quale dei due file e' finito prima"
    );

    // I due contatori: il ciclo gira al piu' `record_count` volte, e quello
    // e' un `u32`. Un `u64` che cresce di uno per giro non ci arriva.
    assert!(
        u64::from(u32::MAX).checked_add(1).is_some(),
        "un contatore u64 che sale fino a u32::MAX non trabocca: e' la ragione per cui \
             i due «fuori intervallo u64» non hanno input"
    );

    // Il controllo positivo, con lo stesso parser: un bundle allineato si
    // legge fino in fondo. Senza, «fallisce sempre» supererebbe la prova.
    let allineato = bundle_di_righe(dir.path(), "allineato.shp", &["NOME"], 3, |_| {});
    let layout = read_dbf_layout(&allineato).expect("il bundle e' conforme");
    let ingresso = ShpParserInput {
        path: allineato,
        schema: schema.clone(),
        cols: Vec::new(),
        dbf_layout: layout.clone(),
        dimensions: CoordinateDimensions::Xy,
        expected_shape_type: Some("point-xy"),
        expected_active_rows: u64::from(layout.record_count),
        include_geometry: true,
        batch_sizer: plenora_io_core::AdaptiveBatchSizer::new(&schema, BatchTarget::default()),
        layer: LayerContract {
            id: LayerId(0),
            name: "allineato".to_owned(),
            contract: DataContract::new(schema, None),
        },
        loss: LossReport::default(),
        row_diagnostics: ShpRowDiagnosticsConfig::from_options(&BTreeMap::new(), &[], &layout)
            .expect("nessuna opzione di diagnostica"),
        scope: ReadScope::Complete,
        cancellation: plenora_io_model::CancellationToken::new(),
    };
    let mut lettore = spawn_parser(ingresso).expect("le validazioni pre-thread passano");
    let mut righe = 0_usize;
    while let Some(lotto) = lettore.next_batch().expect("un bundle allineato si legge") {
        righe += lotto.num_rows();
    }
    assert_eq!(
        righe, 3,
        "tre record allineati a tre geometrie fanno tre righe"
    );
}

/// `descrittori_concordi` rifiuta la divergenza fra le due letture
/// dell'header, e fa uscire il conteggio che e' nostro.
///
/// Il controllo non e' ridondante, ed e' la parte che vale la pena fissare:
/// `leggi_descrittori_dbf` scorre **un descrittore per nome decodificato**,
/// quindi con due numeri diversi il lettore si fermerebbe a meta' dei
/// trentadue byte di un descrittore, e il controllo sul terminatore che
/// segue leggerebbe un byte qualunque. Il rifiuto tiene allineate le due
/// letture prima che si disallineino.
///
/// Che il messaggio porti il conteggio **decodificato** e non quello
/// dell'header e' l'altra meta': il primo lo produciamo noi, il secondo
/// viene dal file, e nessun numero letto dal payload esce dai messaggi.
#[test]
fn n1_descrittori_concordi_rifiuta_la_divergenza_e_fa_uscire_il_numero_nostro() {
    descrittori_concordi(3, 3).expect("due conteggi uguali non sono una divergenza");
    descrittori_concordi(0, 0).expect("zero descrittori concordano con zero");

    for (caso, dichiarati, decodificati) in [
        ("l'header ne dichiara uno in piu'", 2, 1),
        ("il decoder ne trova uno in piu'", 1, 2),
    ] {
        let Err(errore) = descrittori_concordi(dichiarati, decodificati) else {
            panic!("{caso}: due conteggi diversi non possono allineare due letture");
        };
        assert!(
            errore
                .message
                .starts_with("numero di descrittori DBF incoerente con l'header"),
            "{caso}: arrivato «{}»",
            errore.message
        );
        assert!(
            errore.message.ends_with(&decodificati.to_string()),
            "{caso}: deve uscire il conteggio decodificato ({decodificati}), \
                 non quello letto dall'header: «{}»",
            errore.message
        );
    }
}

/// `righe_dopo_il_lotto` ferma il contatore che traboccherebbe, e lo fa
/// **prima** che il lotto raggiunga il disco.
///
/// Il contatore serve a due cose diverse -- il totale dichiarato e la
/// posizione delle righe nelle diagnostiche -- e un valore che ricomincia
/// da zero dopo un giro completo le rovinerebbe entrambe in silenzio.
///
/// La conversione della cardinalita' del lotto resta senza input: e' una
/// `usize` verso `u64`, e su ogni bersaglio supportato non fallisce. Cio'
/// che si prova qui e' la somma, che dipende da uno stato accumulato e non
/// dal bersaglio.
#[test]
fn n1_righe_dopo_il_lotto_ferma_il_contatore_che_traboccherebbe() {
    assert_eq!(
        righe_dopo_il_lotto(7, 3).expect("dieci righe stanno in un u64"),
        10
    );
    assert_eq!(
        righe_dopo_il_lotto(u64::MAX, 0).expect("un lotto vuoto non aggiunge niente"),
        u64::MAX,
        "il confine e' incluso: e' l'ultimo valore rappresentabile, non il primo rifiutato"
    );

    let Err(errore) = righe_dopo_il_lotto(u64::MAX, 1) else {
        panic!("una riga oltre l'ultimo valore rappresentabile non si conta");
    };
    assert_eq!(errore.message, "troppe righe Shapefile");
    assert_eq!(
        errore.category,
        plenora_io_model::ErrorCategory::ResourceLimit
    );

    // La conversione infallibile: che questa riga compili e' la prova che
    // l'altro ramo dell'aiutante non ha input su questo bersaglio.
    let _: u64 = u64::try_from(usize::MAX).expect("usize sta in u64 sui bersagli supportati");
}

/// Il rifiuto sul contatore precede la scrittura, e lo staging resta come
/// era.
///
/// E' la meta' che l'aritmetica da sola non prova: che il conteggio corra
/// **prima** del ciclo che scrive le shape. Un contatore verificato dopo
/// avrebbe gia' lasciato le geometrie nel file di staging, e il rifiuto
/// sarebbe una constatazione.
#[test]
fn n1_il_contatore_che_trabocca_ferma_il_lotto_prima_di_scriverlo() {
    let dir = tempfile::tempdir().unwrap();
    let punto = to_wkb(&geo_types::Geometry::Point(geo_types::Point::new(1.0, 2.0))).unwrap();
    let schema: SchemaRef = Arc::new(Schema::new(vec![geometry_field(GEOMETRY, "EPSG:4326")]));
    let lotto = RecordBatch::try_new(
        schema,
        vec![Arc::new(BinaryArray::from(vec![Some(punto.as_slice())]))],
    )
    .unwrap();

    // Lo stato si costruisce direttamente: `create` restituisce un
    // `Box<dyn FormatWriter>`, e da li' il contatore non e' raggiungibile.
    let destinazione = dir.path().join("contatore.shp");
    let staging = create_staged_dir(&destinazione).expect("lo staging si crea");
    let percorso_shp = staging.path().join("data.shp");
    let writer = Writer::from_path(&percorso_shp, TableWriterBuilder::new())
        .expect("il writer dello staging si apre");
    let mut stato = ShpWriter {
        staging: Some(staging),
        writer: Some(writer),
        dest: destinazione,
        durable: false,
        publish_mode: ShapefilePublishMode::LooseSet,
        attrs: Vec::new(),
        geom_idx: 0,
        prj: None,
        shape_type: None,
        rows: u64::MAX,
        input_total: None,
        wkb_limits: WkbLimits::default(),
        max_output_bytes: u64::MAX,
    };

    let prima = std::fs::metadata(&percorso_shp)
        .expect("il file di staging esiste")
        .len();
    let Err(errore) = stato.write(&lotto) else {
        panic!("un lotto che fa traboccare il contatore non si scrive");
    };
    assert_eq!(errore.message, "troppe righe Shapefile");
    assert_eq!(
        std::fs::metadata(&percorso_shp)
            .expect("il file di staging esiste ancora")
            .len(),
        prima,
        "il rifiuto deve precedere la scrittura: lo staging non deve essere cresciuto"
    );
}

/// `byte_dello_staging` somma le parti senza traboccare in silenzio.
///
/// Il tetto sull'output vale sull'**insieme** delle quattro parti, e un
/// totale che ricominciasse da zero lo farebbe passare: il set verrebbe
/// pubblicato dichiarando meno byte di quanti ne occupa, che e' peggio di
/// un rifiuto.
#[test]
fn n1_byte_dello_staging_non_trabocca_in_silenzio() {
    assert_eq!(
        byte_dello_staging([1, 2, 3]).expect("tre parti si sommano"),
        6
    );
    assert_eq!(
        byte_dello_staging([]).expect("nessuna parte fa zero byte"),
        0,
        "un set senza parti presenti e' zero, non un errore"
    );
    assert_eq!(
        byte_dello_staging([u64::MAX, 0]).expect("il confine e' incluso"),
        u64::MAX
    );

    let Err(errore) = byte_dello_staging([u64::MAX, 1]) else {
        panic!("una somma oltre u64::MAX non e' un conteggio");
    };
    assert_eq!(
        errore.message,
        "overflow nel conteggio dell'output Shapefile"
    );
    assert_eq!(
        errore.category,
        plenora_io_model::ErrorCategory::ResourceLimit
    );
}

/// `next_physical` rifiuta il campo che non sta nel record, e nessun layout
/// letto da un file glielo puo' presentare.
///
/// Il metodo indicizza il buffer del record con gli offset del layout, e la
/// difesa esiste perche' quei due valori arrivano da posti diversi: il
/// buffer e' lungo `record_length`, gli offset vengono dai descrittori.
/// `read_dbf_layout` li rende coerenti per costruzione -- `record_length`
/// **e'** l'offset finale calcolato dai descrittori, non quello dichiarato
/// nell'header -- quindi da un file la difesa non ha input.
///
/// Il tipo e' privato del modulo, e la sonda costruisce il layout
/// incoerente direttamente: e' l'unico modo di eseguire quella riga senza
/// fingere che un file possa produrla. Accanto sta la prova che un layout
/// **letto davvero** e' coerente, che e' la meta' che rende interpretabile
/// la prima.
#[test]
fn n1_next_physical_rifiuta_il_campo_che_non_sta_nel_record() {
    let dir = tempfile::tempdir().unwrap();
    let bundle = bundle_con_dbf_ritoccato(dir.path(), "coerente.shp", &["CODICE"], |_| {});

    // La meta' che vale come controllo: il layout che il lettore produce ha
    // ogni campo dentro il record.
    let letto = read_dbf_layout(&bundle).expect("il bundle e' conforme");
    for campo in &letto.fields {
        assert!(
            campo.offset + campo.width <= letto.record_length,
            "`read_dbf_layout` non produce campi che escono dal record: \
                 offset {} + larghezza {} contro {}",
            campo.offset,
            campo.width,
            letto.record_length
        );
    }

    // Il layout incoerente: un campo che comincia dentro il record e finisce
    // fuori. `exact_integer_slot` deve essere pieno, altrimenti il ciclo
    // salta il campo e la riga non viene mai raggiunta.
    let incoerente = DbfLayout {
        header_length: letto.header_length,
        record_length: letto.record_length,
        record_count: 1,
        fields: vec![DbfFieldLayout {
            name: "CODICE".to_owned(),
            field_type: b'N',
            offset: 1,
            width: letto.record_length + 10,
            exact_integer_slot: Some(0),
        }],
        exact_integer_count: 1,
    };
    let mut righe = DbfExactIntegerRows::open(&bundle, &incoerente)
        .expect("l'apertura non guarda gli offset dei campi");
    let Err(errore) = righe.next_physical(None) else {
        panic!("un campo che finisce fuori dal record non e' leggibile");
    };
    assert_eq!(errore.message, "campo DBF fuori dal record");
}

/// `finish_batch` rifiuta la combinazione che Arrow non accetta.
///
/// La funzione mette insieme tre cose che arrivano da posti diversi: lo
/// schema del contratto, i costruttori delle colonne e il numero di righe
/// osservate. Arrow pretende che concordino, e la difesa esiste perche' un
/// disaccordo qui produrrebbe un batch che il chiamante crede conforme al
/// contratto e non lo e'.
///
/// Il caso provato e' il piu' semplice dei tre disaccordi possibili: uno
/// schema che dichiara una colonna in piu' di quelle costruite.
#[test]
fn n1_finish_batch_rifiuta_lo_schema_che_non_torna_con_i_costruttori() {
    let schema: SchemaRef = Arc::new(Schema::new(vec![
        geometry_field(GEOMETRY, "EPSG:4326"),
        Field::new("MANCANTE", arrow_schema::DataType::Utf8, true),
    ]));

    // Il controllo positivo: con il costruttore che lo schema dichiara, il
    // batch si forma e porta le righe dichiarate.
    let mut geometria = Some(BinaryBuilder::new());
    if let Some(builder) = geometria.as_mut() {
        builder.append_value(b"wkb");
    }
    let mut costruttori = vec![InferredColumnBuilder::new(ColType::Text)];
    costruttori[0]
        .append_str("uno")
        .expect("un testo entra in un costruttore di testo");
    let lotto = finish_batch(&schema, &mut geometria, &mut costruttori, 1)
        .expect("schema e costruttori concordano");
    assert_eq!(lotto.num_rows(), 1);
    assert_eq!(lotto.num_columns(), 2);

    // Lo stesso schema senza il costruttore della seconda colonna.
    let mut geometria = Some(BinaryBuilder::new());
    if let Some(builder) = geometria.as_mut() {
        builder.append_value(b"wkb");
    }
    let Err(errore) = finish_batch(&schema, &mut geometria, &mut [], 1) else {
        panic!("uno schema con una colonna in piu' dei costruttori non forma un batch");
    };
    assert_eq!(errore.message, "costruzione del RecordBatch fallita");
}

/// `open` propaga i rifiuti della catena e prende il nome del layer dal
/// nome del file.
///
/// L'entry point non decide quasi niente da se': la sua parte e' mettere in
/// fila le validazioni e comporre il contratto. Le due cose che vale la
/// pena fissare sono percio' che i rifiuti **escano** con il proprio
/// messaggio invece di essere riscritti, e che il nome del layer sia il
/// gambo del file -- un dettaglio che nessun errore esprime e che chi legge
/// il contratto vede per primo.
#[test]
fn n1_open_propaga_i_rifiuti_della_catena_e_nomina_il_layer_dal_file() {
    let dir = tempfile::tempdir().unwrap();
    let bundle = bundle_con_dbf_ritoccato(dir.path(), "comuni.shp", &["NOME"], |_| {});

    // Il nome del layer viene dal gambo del file, non da un letterale.
    let aperto = ShpDriver
        .open(
            Source::Path(bundle.clone()),
            opzioni_lettura().with_assume_crs("EPSG:4326"),
        )
        .expect("un bundle conforme si apre");
    assert_eq!(
        aperto.layers()[0].name,
        "comuni",
        "il nome del layer e' il gambo del file"
    );

    // La propagazione dalle opzioni di diagnostica: il messaggio e' quello
    // di `from_options`, non una riscrittura.
    let Err(errore) = ShpDriver.open(
        Source::Path(bundle),
        opzioni_lettura()
            .with_assume_crs("EPSG:4326")
            .with_format_option("row_diagnostics.key_policy", "emit"),
    ) else {
        panic!("una policy senza campo non e' una configurazione valida");
    };
    assert_eq!(
        errore.message,
        "row_diagnostics.key_policy richiede row_diagnostics.key_field"
    );

    // La propagazione dall'inferenza dello schema: un `.dbf` con l'header
    // ritoccato ferma l'apertura, e il messaggio resta quello del lettore.
    let rotto = bundle_con_dbf_ritoccato(dir.path(), "rotto.shp", &["NOME"], |byte| {
        byte[10..12].copy_from_slice(&99_u16.to_le_bytes());
    });
    let Err(errore) = ShpDriver.open(
        Source::Path(rotto),
        opzioni_lettura().with_assume_crs("EPSG:4326"),
    ) else {
        panic!("un header DBF incoerente non produce uno schema");
    };
    assert_eq!(
        errore.message,
        "lunghezza di record DBF dichiarata incoerente con i campi, byte richiesti 11",
        "il messaggio del lettore deve uscire da `open` invariato"
    );
}

/// Il conteggio dell'header e `dbase` non possono divergere: due guardie
/// diverse lo impediscono, e questa sonda le esegue entrambe.
///
/// `infer_shp_schema` scorre i record due volte in parallelo -- il lettore
/// raw degli interi esatti e l'iteratore di `dbase` -- e ha una difesa per
/// il caso in cui uno dei due finisca prima dell'altro. Entrambi sono
/// guidati dallo stesso conteggio nell'header, quindi per farli divergere
/// bisognerebbe far fermare `dbase` prima, e `dbase` si ferma in due modi
/// soli: alla fine fisica del file, o sul marcatore di fine record.
///
/// **Le due strade sono chiuse, e la sonda le percorre invece di
/// argomentarle.** Un `.dbf` troncato non arriva alla divergenza: il
/// lettore raw pretende i byte del record e rifiuta con «record DBF
/// incompleto». Un marcatore di fine file messo dove `dbase` lo
/// leggerebbe come tale cade sul flag di cancellazione, e il lettore raw
/// ammette solo lo spazio e l'asterisco: rifiuta con «marcatore di record
/// DBF non valido».
///
/// La seconda difesa -- `dbase` che ha **piu'** record del conteggio -- e'
/// chiusa da un'altra parte: `infer_geometry_info` confronta le geometrie
/// con il conteggio dichiarato e corre prima del ciclo.
///
/// Restano senza input anche «numero di record DBF fuori intervallo u64»,
/// che sorveglia una conversione da `u32`, e «schema DBF senza accumulatore
/// per un campo dichiarato», che sorveglia una mappa costruita dagli stessi
/// descrittori su cui poi si cerca: perche' un campo non vi si trovasse
/// servirebbero due descrittori omonimi, e `leggi_descrittori_dbf` li
/// rifiuta prima.
#[test]
fn n1_le_due_letture_del_dbf_non_possono_divergere() {
    let dir = tempfile::tempdir().unwrap();

    // Il controllo positivo: due righe dichiarate, due presenti, schema
    // inferito. Senza, «rifiuta sempre» supererebbe la prova.
    let intatto = bundle_di_righe(dir.path(), "due.shp", &["NOME"], 2, |_| {});
    let inferenza = infer_shp_schema(&intatto).expect("un bundle conforme si legge");
    assert_eq!(inferenza.active_row_count, 2, "due righe attive");
    assert_eq!(inferenza.cols.len(), 1, "un campo oltre la geometria");

    // Prima strada: il file finisce prima di quanto l'header prometta.
    let troncato = bundle_di_righe(dir.path(), "troncato.shp", &["NOME"], 2, |byte| {
        let lunghezza_header = usize::from(u16::from_le_bytes([byte[8], byte[9]]));
        let lunghezza_record = usize::from(u16::from_le_bytes([byte[10], byte[11]]));
        byte.truncate(lunghezza_header + lunghezza_record);
    });
    let Err(errore) = infer_shp_schema(&troncato) else {
        panic!("un header che promette piu' record di quanti ce ne sono non e' affidabile");
    };
    assert_eq!(
        errore.message, "record DBF incompleto",
        "a rifiutare e' il lettore raw, prima che le due letture divergano"
    );

    // Seconda strada: il marcatore di fine file dove `dbase` lo leggerebbe.
    // Cade sul flag di cancellazione del terzo record, e il lettore raw
    // ammette solo lo spazio e l'asterisco.
    let marcato = bundle_di_righe(dir.path(), "marcato.shp", &["NOME"], 3, |byte| {
        let lunghezza_header = usize::from(u16::from_le_bytes([byte[8], byte[9]]));
        let lunghezza_record = usize::from(u16::from_le_bytes([byte[10], byte[11]]));
        byte[lunghezza_header + 2 * lunghezza_record] = 0x1A;
    });
    let Err(errore) = infer_shp_schema(&marcato) else {
        panic!("un marcatore di fine file in mezzo ai record non e' un record");
    };
    assert_eq!(
        errore.message, "marcatore di record DBF non valido",
        "a rifiutare e' il controllo sul flag di cancellazione"
    );

    // Le due difese che restano, e le loro precondizioni.
    //
    // La prima si prova senza asserzioni: il conteggio dei record e' un
    // `u32`, e la conversione a `u64` e' **infallibile** -- `From`, non
    // `TryFrom`. Che questa riga compili e' la prova, e vale piu' di
    // un'asserzione su un valore che il compilatore conosce gia'.
    let _: u64 = u64::from(u32::MAX);
    let doppi = leggi_descrittori_dbf(
        &mut std::io::Cursor::new([descrittore(b'C', 10, 0), descrittore(b'C', 10, 0)].concat()),
        vec!["NOME".to_owned(), "nome".to_owned()],
        2,
    );
    assert!(
        doppi.is_err(),
        "due descrittori omonimi sono rifiutati prima: e' la ragione per cui la mappa \
             degli accumulatori ha una voce per ogni campo"
    );
}

/// `infer_geometry_info` pretende che i due file del bundle raccontino la
/// stessa storia.
///
/// Uno Shapefile e' due file che si contano a vicenda: il `.shp` porta le
/// geometrie, il `.dbf` gli attributi, e la riga *n* e' la coppia dei due
/// record *n*-esimi. Se i conteggi non coincidono non esiste un
/// allineamento giusto -- non si sa quale dei due file abbia la riga in
/// piu' -- e leggere comunque accoppierebbe attributi con geometrie di
/// un'altra riga. E' la perdita silenziosa che il rifiuto esiste per
/// evitare, e il messaggio dice esattamente questo invece di «file
/// corrotto».
///
/// Il conteggio ritoccato e' quello del `.dbf`, perche' e' un campo
/// dell'header e si cambia senza toccare i record: il `.shp` resta quello
/// che il writer ha prodotto, quindi l'unica cosa incoerente e' cio' che la
/// sonda ha dichiarato.
#[test]
fn n1_infer_geometry_info_rifiuta_i_conteggi_che_non_coincidono() {
    let dir = tempfile::tempdir().unwrap();

    // Il controllo positivo: un bundle intatto ha una geometria e un
    // record, e l'inferenza restituisce il tipo dell'header.
    let intatto = bundle_con_dbf_ritoccato(dir.path(), "intatto-geom.shp", &["NOME"], |_| {});
    let info = infer_geometry_info(&intatto, 1).expect("un bundle conforme si legge");
    assert_eq!(
        info.shape_type,
        Some("point-xy"),
        "il tipo viene dall'header del .shp"
    );
    assert_eq!(info.dimensions, CoordinateDimensions::Xy);

    // Il conteggio dichiarato dal chiamante non coincide con le geometrie.
    for dichiarato in [0_u32, 2] {
        let Err(errore) = infer_geometry_info(&intatto, dichiarato) else {
            panic!("{dichiarato} record DBF contro una geometria non e' un allineamento");
        };
        assert_eq!(
            errore.message, "numero di geometrie diverso dal numero di record DBF",
            "il messaggio deve dire che i due file non si contano allo stesso modo"
        );
    }

    // Lo stesso disallineamento visto dall'altra parte: e' il `.dbf` a
    // dichiarare un conteggio che il `.shp` non conferma, ed e' la strada
    // per cui il rifiuto arriva davvero da un file e non dal chiamante.
    let ritoccato = bundle_con_dbf_ritoccato(
        dir.path(),
        "conteggio.shp",
        &["NOME"],
        |byte: &mut Vec<u8>| {
            byte[4..8].copy_from_slice(&7_u32.to_le_bytes());
        },
    );
    let layout = read_dbf_layout(&ritoccato).expect("l'header resta leggibile");
    assert_eq!(layout.record_count, 7, "il ritocco e' arrivato dove doveva");
    let Err(errore) = infer_geometry_info(&ritoccato, layout.record_count) else {
        panic!("sette record dichiarati contro una geometria non e' un allineamento");
    };
    assert_eq!(
        errore.message,
        "numero di geometrie diverso dal numero di record DBF"
    );

    // La propagazione dalla validazione della struttura: senza il `.shp`
    // non c'e' niente da contare, e il rifiuto arriva prima dell'apertura.
    let assente = dir.path().join("non-esiste.shp");
    assert!(
        infer_geometry_info(&assente, 0).is_err(),
        "un bundle senza `.shp` non ha geometrie da inferire"
    );
}

/// Un descrittore di campo DBF: trentadue byte, con tipo, larghezza e
/// decimali nelle posizioni che il formato fissa.
///
/// Il nome **non** sta qui: `leggi_descrittori_dbf` lo riceve gia'
/// decodificato da `dbase`, e questa e' la separazione che rende provabile
/// il rifiuto sui nomi senza costruire un file.
fn descrittore(tipo: u8, larghezza: u8, decimali: u8) -> [u8; DBF_FIELD_DESCRIPTOR_SIZE] {
    let mut byte = [0_u8; DBF_FIELD_DESCRIPTOR_SIZE];
    byte[11] = tipo;
    byte[16] = larghezza;
    byte[17] = decimali;
    byte
}

/// `leggi_descrittori_dbf` rifiuta i nomi che perderebbero una colonna e le
/// larghezze che non descrivono niente.
///
/// I nomi duplicati sono il caso che vale di piu', e il rifiuto non e'
/// pignoleria: il DBF confronta i nomi **senza distinguere maiuscole**, e
/// due colonne omonime finirebbero nella stessa chiave. Chi legge
/// perderebbe una colonna senza che nulla lo dica -- il file si aprirebbe,
/// il conteggio dei campi tornerebbe, e mancherebbe un dato.
///
/// Il nome vuoto e la larghezza zero sono la stessa famiglia: un campo che
/// non ha un nome o non ha byte non e' una colonna, e accettarlo
/// sposterebbe tutti gli offset successivi.
///
/// Ogni messaggio porta l'**indice**, non il nome: l'indice e' prodotto
/// dalla nostra enumerazione, il nome viene dal file.
#[test]
fn n1_leggi_descrittori_dbf_rifiuta_i_nomi_che_perderebbero_una_colonna() {
    let nomi =
        |elenco: &[&str]| -> Vec<String> { elenco.iter().map(|nome| (*nome).to_owned()).collect() };
    let flusso = |quanti: usize, larghezza: u8| {
        let mut byte = Vec::new();
        for _ in 0..quanti {
            byte.extend_from_slice(&descrittore(b'C', larghezza, 0));
        }
        byte
    };

    for (caso, nomi_campi, byte, atteso) in [
        (
            "descrittore troncato",
            nomi(&["NOME"]),
            vec![0_u8; DBF_FIELD_DESCRIPTOR_SIZE - 1],
            "descrittore di campo DBF incompleto",
        ),
        (
            "nome vuoto",
            nomi(&[""]),
            flusso(1, 10),
            "nome campo DBF vuoto, indice 0",
        ),
        (
            "nomi duplicati",
            nomi(&["NOME", "NOME"]),
            flusso(2, 10),
            "nomi campo DBF duplicati; il file e' rifiutato per non perdere una colonna, \
                 secondo indice 1",
        ),
        (
            "nomi duplicati con maiuscole diverse",
            nomi(&["Nome", "NOME"]),
            flusso(2, 10),
            "nomi campo DBF duplicati; il file e' rifiutato per non perdere una colonna, \
                 secondo indice 1",
        ),
        (
            "larghezza zero",
            nomi(&["NOME"]),
            flusso(1, 0),
            "campo DBF con larghezza zero, indice 0",
        ),
    ] {
        let quanti = nomi_campi.len();
        let esito = leggi_descrittori_dbf(&mut std::io::Cursor::new(byte), nomi_campi, quanti);
        let Err(errore) = esito else {
            panic!("{caso}: doveva essere rifiutato");
        };
        assert_eq!(errore.message, atteso, "{caso}: messaggio sbagliato");
    }

    // Le accettazioni, e le tre quantita' che la funzione calcola: gli
    // offset si accumulano a partire dal deletion flag, la lunghezza di
    // record e' l'offset finale, e lo slot di intero esatto si assegna solo
    // ai campi numerici senza decimali larghi almeno dieci.
    let mut byte = Vec::new();
    byte.extend_from_slice(&descrittore(b'C', 10, 0)); // testo: nessuno slot
    byte.extend_from_slice(&descrittore(b'N', 18, 0)); // intero esatto: slot 0
    byte.extend_from_slice(&descrittore(b'N', 9, 0)); // troppo stretto
    byte.extend_from_slice(&descrittore(b'N', 20, 8)); // con decimali
    byte.extend_from_slice(&descrittore(b'N', 12, 0)); // intero esatto: slot 1
    let (campi, lunghezza, esatti) = leggi_descrittori_dbf(
        &mut std::io::Cursor::new(byte),
        nomi(&["TESTO", "GRANDE", "STRETTO", "DECIMALE", "ALTRO"]),
        5,
    )
    .expect("cinque descrittori ben formati");

    assert_eq!(
        campi.iter().map(|campo| campo.offset).collect::<Vec<_>>(),
        vec![1, 11, 29, 38, 58],
        "gli offset partono dal deletion flag e si accumulano"
    );
    assert_eq!(lunghezza, 70, "la lunghezza di record e' l'offset finale");
    assert_eq!(esatti, 2, "solo due campi sono interi esatti");
    assert_eq!(
        campi
            .iter()
            .map(|campo| campo.exact_integer_slot)
            .collect::<Vec<_>>(),
        vec![None, Some(0), None, None, Some(1)],
        "gli slot sono numerati in ordine, e solo per i campi che li meritano"
    );
}

/// L'overflow della lunghezza record DBF e' irraggiungibile: l'aritmetica
/// del formato non ci arriva.
///
/// L'offset accumula larghezze, e una larghezza sta in **un byte**: al piu'
/// 255. Il numero di descrittori e' limitato dalla lunghezza dell'header,
/// che sta in un `u16`: al piu' 65 535 byte, cioe' meno di 2048
/// descrittori da trentadue byte. Il massimo assoluto e' quindi circa mezzo
/// milione, quindici ordini di grandezza sotto `usize::MAX` su un bersaglio
/// a sessantaquattro bit. Non c'e' una guardia da eseguire: c'e' un limite
/// di formato, verificato qui invece che argomentato.
#[test]
fn n1_la_lunghezza_record_dbf_non_puo_traboccare() {
    let descrittori_massimi = usize::from(u16::MAX) / DBF_FIELD_DESCRIPTOR_SIZE;
    let larghezza_massima = usize::from(u8::MAX);
    let massimo = 1 + descrittori_massimi * larghezza_massima;
    assert!(
        massimo < usize::MAX / 1_000_000,
        "il massimo di formato ({massimo}) deve restare lontanissimo da usize::MAX: \
             se un giorno non lo fosse, l'overflow andrebbe riclassificato"
    );
    assert!(
        1_usize.checked_add(massimo).is_some(),
        "la somma che la funzione esegue non trabocca nemmeno al massimo di formato"
    );
}

/// Un bundle Shapefile valido, con il `.dbf` ritoccato in un campo solo.
///
/// Costruito con il writer vero e poi modificato: cosi' `.shp` e `.shx`
/// restano quelli di un produttore conforme, e l'unico motivo per cui la
/// lettura puo' fallire e' il byte che la sonda ha cambiato. Costruirlo a
/// mano metterebbe in gioco anche la correttezza della fixture, e un
/// rifiuto non distinguerebbe piu' le due cause.
fn bundle_con_dbf_ritoccato(
    dir: &Path,
    nome: &str,
    campi: &[&str],
    ritocco: impl FnOnce(&mut Vec<u8>),
) -> PathBuf {
    bundle_di_righe(dir, nome, campi, 1, ritocco)
}

/// Come sopra, con il numero di righe scelto.
fn bundle_di_righe(
    dir: &Path,
    nome: &str,
    campi: &[&str],
    righe: usize,
    ritocco: impl FnOnce(&mut Vec<u8>),
) -> PathBuf {
    let percorso = dir.join(nome);
    let mut tabella = TableWriterBuilder::new();
    for campo in campi {
        tabella =
            tabella.add_character_field(shapefile::dbase::FieldName::try_from(*campo).unwrap(), 10);
    }
    let mut writer = Writer::from_path(&percorso, tabella).expect("il writer si apre");
    let mut record = Record::default();
    for campo in campi {
        record.insert(
            (*campo).to_owned(),
            FieldValue::Character(Some("uno".to_owned())),
        );
    }
    for _ in 0..righe {
        writer
            .write_shape_and_record(&shapefile::Point::new(1.0, 2.0), &record)
            .expect("un punto si scrive");
    }
    drop(writer);

    let dbf = percorso.with_extension("dbf");
    let mut byte = std::fs::read(&dbf).expect("il dbf esiste");
    ritocco(&mut byte);
    std::fs::write(&dbf, byte).expect("il dbf si riscrive");
    percorso
}

/// `read_dbf_layout` rifiuta le incoerenze fra header e descrittori, e le
/// altre le trova gia' fermate da `valida_intestazione_dbf`.
///
/// Le due funzioni guardano lo stesso header e si sovrappongono di
/// proposito: `valida_intestazione_dbf` esiste per chiudere i punti in cui
/// `dbase` panica invece di tornare, quindi corre **prima** e per prima
/// rifiuta l'header troncato, il backlink `Visual FoxPro` piu' lungo
/// dell'header, l'offset del primo record piu' corto dell'intestazione e il
/// terminatore sbagliato. Le righe omonime dentro `read_dbf_layout` restano
/// come difesa di una lettura che non e' piu' isolata, e non hanno input.
///
/// Cio' che `read_dbf_layout` decide **da se'** e' quello che l'altra
/// tronca invece di rifiutare: un numero di byte di descrittori che non e'
/// multiplo di trentadue. E la lunghezza di record dichiarata che non torna
/// con la somma dei campi -- l'unica delle due che riguarda i **record** e
/// non l'header.
///
/// Resta fuori il confronto fra il conteggio dell'header e quello dei nomi
/// decodificati. Non e' dichiarato coperto e non e' dichiarato
/// irraggiungibile: i due conteggi vengono dalla stessa aritmetica
/// sull'header, e non ho saputo costruire un file in cui divergano --
/// nemmeno anticipando il terminatore fra i descrittori, che `dbase`
/// attraversa senza fermarsi. E' registrato come rischio residuo, perche'
/// dichiararlo irraggiungibile senza una prova sarebbe la supposizione che
/// questo censimento esiste per escludere.
#[test]
fn n1_read_dbf_layout_rifiuta_le_incoerenze_fra_header_e_descrittori() {
    let dir = tempfile::tempdir().unwrap();

    // Il controllo positivo: il bundle non ritoccato si legge.
    let intatto = bundle_con_dbf_ritoccato(dir.path(), "intatto.shp", &["NOME"], |_| {});
    let layout = read_dbf_layout(&intatto).expect("un bundle conforme ha un layout");
    assert_eq!(layout.fields.len(), 1, "un campo dichiarato, uno letto");
    assert_eq!(layout.record_count, 1, "un record scritto, uno dichiarato");

    for (caso, campi, ritocco, atteso) in [
        (
            "byte di descrittori non multiplo di trentadue",
            &["NOME"][..],
            Box::new(|byte: &mut Vec<u8>| {
                let lunghezza = u16::from_le_bytes([byte[8], byte[9]]) + 1;
                byte[8..10].copy_from_slice(&lunghezza.to_le_bytes());
            }) as Box<dyn FnOnce(&mut Vec<u8>)>,
            "lunghezza descrittori DBF non valida",
        ),
        (
            "lunghezza di record che non torna con i campi",
            &["NOME"][..],
            Box::new(|byte: &mut Vec<u8>| {
                byte[10..12].copy_from_slice(&99_u16.to_le_bytes());
            }),
            "lunghezza di record DBF dichiarata incoerente con i campi, byte richiesti 11",
        ),
    ] {
        let percorso =
            bundle_con_dbf_ritoccato(dir.path(), &format!("{}.shp", caso.len()), campi, ritocco);
        let Err(errore) = read_dbf_layout(&percorso) else {
            panic!("{caso}: doveva essere rifiutato");
        };
        assert_eq!(errore.message, atteso, "{caso}: messaggio sbagliato");
    }
}

/// Le righe di `read_dbf_layout` che `valida_intestazione_dbf` raggiunge
/// per prima.
///
/// Non e' una duplicazione da togliere: `read_dbf_layout` legge l'header
/// una seconda volta, con la propria aritmetica, e senza quelle guardie
/// dipenderebbe dal fatto che qualcun altro l'abbia gia' controllato. La
/// sonda esegue la **precedenza**, cioe' che a rifiutare sia la prima: se
/// l'ordine cambiasse, il messaggio cambierebbe e la sonda diventerebbe
/// rossa.
#[test]
fn n1_valida_intestazione_dbf_precede_le_guardie_omonime_di_read_dbf_layout() {
    let dir = tempfile::tempdir().unwrap();

    for (caso, ritocco, atteso) in [
        (
            "header troncato sotto i trentadue byte",
            Box::new(|byte: &mut Vec<u8>| byte.truncate(DBF_HEADER_SIZE - 1))
                as Box<dyn FnOnce(&mut Vec<u8>)>,
            "header DBF incompleto",
        ),
        (
            "offset del primo record dentro l'intestazione",
            Box::new(|byte: &mut Vec<u8>| {
                byte[8..10].copy_from_slice(&8_u16.to_le_bytes());
            }),
            "offset del primo record DBF piu' corto dell'intestazione",
        ),
        (
            "versione Visual FoxPro con header piu' corto del backlink",
            Box::new(|byte: &mut Vec<u8>| {
                byte[0] = DBF_VISUAL_FOXPRO_VERSION;
                byte[8..10].copy_from_slice(&4_u16.to_le_bytes());
            }),
            "header Visual FoxPro piu' corto del backlink",
        ),
    ] {
        let percorso = bundle_con_dbf_ritoccato(
            dir.path(),
            &format!("precedenza-{}.shp", caso.len()),
            &["NOME"],
            ritocco,
        );
        let Err(errore) = read_dbf_layout(&percorso) else {
            panic!("{caso}: doveva essere rifiutato");
        };
        assert_eq!(
            errore.message, atteso,
            "{caso}: a rifiutare deve essere la validazione dell'intestazione"
        );
    }
}

/// Un piano di scrittura con la sola geometria, piu' i campi dati.
fn piano_di_scrittura(campi: Vec<Field>) -> WritePlan {
    let mut colonne = vec![geometry_field(GEOMETRY, "EPSG:4326")];
    colonne.extend(campi);
    WritePlan {
        layers: vec![WriteLayer {
            name: "strato".to_owned(),
            contract: DataContract {
                schema: Arc::new(Schema::new(colonne)),
                geometry: None,
            },
        }],
    }
}

/// La geometria assente attraversa lo Shapefile e torna indietro identica.
///
/// # Il difetto che chiude
///
/// La specifica ESRI ammette un record con shape type 0 dentro un file che
/// ne dichiara un altro: e' cosi' che si scrive una feature senza
/// geometria. Il nostro **reader** la legge -- la fixture canonica ne
/// contiene una, scritta da OGR -- e il writer la rifiutava: non sapevamo
/// riscrivere un file che sapevamo leggere, e un round trip che deve
/// conservare i dati si fermava sulla riga che non ne aveva.
///
/// Scartare la riga sarebbe stato peggio del rifiuto: `.shp` e `.dbf` sono
/// due file paralleli, e togliere un record da uno solo li disallinea per
/// tutte le righe successive. Ogni attributo si troverebbe accanto alla
/// geometria di un'altra feature, e nessun errore lo direbbe.
///
/// # Le quattro posizioni
///
/// Prima, in mezzo, ultima e unica. Non sono lo stesso caso: la prima
/// riserva l'intestazione senza poter dichiarare un tipo, l'ultima chiude
/// il file dopo un record che non muove il bounding box, e quella sola
/// produce un file che un tipo non ce l'ha affatto.
#[test]
fn n1_la_geometria_assente_attraversa_lo_shapefile() {
    let punto = to_wkb(&geo_types::Geometry::Point(geo_types::Point::new(1.0, 2.0))).unwrap();
    let campo = || Field::new("ETICHETTA", arrow_schema::DataType::Utf8, true);

    for (caso, geometrie) in [
        (
            "prima",
            vec![None, Some(punto.as_slice()), Some(punto.as_slice())],
        ),
        (
            "intermedia",
            vec![Some(punto.as_slice()), None, Some(punto.as_slice())],
        ),
        (
            "ultima",
            vec![Some(punto.as_slice()), Some(punto.as_slice()), None],
        ),
        ("unica", vec![None]),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let destinazione = dir.path().join("nulla.shp");
        // Le etichette distinguono le righe: e' con loro che si vede se il
        // DBF e' scivolato rispetto allo `.shp`.
        let etichette: Vec<String> = (0..geometrie.len()).map(|i| format!("r{i}")).collect();
        let schema: SchemaRef = Arc::new(Schema::new(vec![
            geometry_field(GEOMETRY, "EPSG:4326"),
            campo(),
        ]));
        let lotto = RecordBatch::try_new(
            schema,
            vec![
                Arc::new(BinaryArray::from(geometrie.clone())),
                Arc::new(arrow_array::StringArray::from(
                    etichette.iter().map(String::as_str).collect::<Vec<_>>(),
                )),
            ],
        )
        .unwrap();

        let mut writer = ShpDriver
            .create(
                Sink::Path(destinazione.clone()),
                &piano_di_scrittura(vec![campo()]),
                &opzioni_scrittura_loose(),
            )
            .expect("la destinazione e' libera");
        writer
            .declare_input_total(LayerId(0), geometrie.len() as u64)
            .unwrap();
        // `assert!` e non `unwrap_or_else(|e| panic!(...))`: il nome del
        // caso deve comparire nel fallimento -- i quattro casi si
        // somigliano -- ma qui non c'e' nessun valore di ripiego, e la
        // forma `unwrap_or*` e' quella che il registro dei fallback conta.
        let esito = writer.write(&lotto);
        assert!(
            esito.is_ok(),
            "{caso}: la scrittura deve riuscire: {esito:?}"
        );
        // `Published` non e' `Debug`: si nomina il solo errore, che lo e'.
        let esito = writer.finish().err();
        assert!(
            esito.is_none(),
            "{caso}: la pubblicazione deve riuscire: {esito:?}"
        );

        // `.shx` porta una voce per record, come lo `.shp`: e' l'indice, e
        // un record scritto senza la propria voce lo renderebbe illeggibile
        // per posizione. Cento byte di intestazione, otto per voce.
        let shx = std::fs::metadata(destinazione.with_extension("shx"))
            .expect("lo .shx esiste")
            .len();
        assert_eq!(
            shx,
            100 + 8 * geometrie.len() as u64,
            "{caso}: lo .shx deve avere una voce per record"
        );

        // La rilettura: righe, geometrie e attributi, nell'ordine.
        let riletto = ShpDriver.open(Source::Path(destinazione), opzioni_lettura());
        // Come sopra: l'handle non e' `Debug`, l'errore si'.
        let motivo = riletto.as_ref().err();
        assert!(motivo.is_none(), "{caso}: il file si rilegge: {motivo:?}");
        let dataset = riletto.expect("gia' verificato subito sopra");
        let mut lettore = dataset.open_layer_reader(&req()).expect("il layer si apre");
        let mut lette = Vec::new();
        while let Some(batch) = lettore.next_batch().expect("lettura") {
            let geometrie = batch
                .column(geometry_index(&batch.schema()).expect("colonna geometria"))
                .as_any()
                .downcast_ref::<BinaryArray>()
                .expect("geometria binaria")
                .clone();
            let etichette = batch
                .column_by_name("ETICHETTA")
                .expect("l'attributo torna indietro")
                .as_any()
                .downcast_ref::<arrow_array::StringArray>()
                .expect("attributo testuale")
                .clone();
            for riga in 0..batch.num_rows() {
                lette.push((geometrie.is_null(riga), etichette.value(riga).to_owned()));
            }
        }

        let atteso: Vec<(bool, String)> = geometrie
            .iter()
            .zip(&etichette)
            .map(|(g, e)| (g.is_none(), e.clone()))
            .collect();
        assert_eq!(
            lette, atteso,
            "{caso}: ogni riga deve tornare con la propria geometria e la \
                 propria etichetta, e nessuna deve scivolare"
        );
    }
}

/// `create` rifiuta il contratto senza geometria, il nome campo che il DBF
/// non sa portare, e la destinazione gia' occupata in entrambe le forme.
///
/// I tre rifiuti arrivano da tre autorita' diverse, e distinguerli conta.
/// La colonna geometria manca nel **contratto**, cioe' in cio' che il piano
/// dichiara. Il nome campo e' un limite del **formato**: il DBF porta al
/// massimo dieci caratteri ASCII, e troncarlo produrrebbe due colonne
/// omonime da un piano che ne aveva due distinte. La destinazione occupata
/// e' lo stato del **disco**, e il no-clobber vale sull'intero set: un
/// `.shp` che non esiste ma con un `.dbf` accanto e' comunque una
/// pubblicazione che sovrascriverebbe.
///
/// Le quattro estensioni compagne sono provate una per una, e non con una
/// sola: sono quattro rami dello stesso ciclo, e una fixture sola non
/// direbbe che le altre tre sono guardate.
#[test]
fn n1_create_rifiuta_il_contratto_il_nome_campo_e_la_destinazione_occupata() {
    let dir = tempfile::tempdir().unwrap();

    // Il controllo positivo: lo stesso piano su una destinazione libera.
    assert!(
        ShpDriver
            .create(
                Sink::Path(dir.path().join("libero.shp")),
                &piano_di_scrittura(Vec::new()),
                &opzioni_scrittura_loose(),
            )
            .is_ok(),
        "un piano con la sola geometria e una destinazione libera si apre"
    );

    // Contratto senza colonna geometria.
    let senza_geometria = WritePlan {
        layers: vec![WriteLayer {
            name: "strato".to_owned(),
            contract: DataContract {
                schema: Arc::new(Schema::new(vec![Field::new(
                    "nome",
                    arrow_schema::DataType::Utf8,
                    true,
                )])),
                geometry: None,
            },
        }],
    };
    let Err(errore) = ShpDriver.create(
        Sink::Path(dir.path().join("senza-geometria.shp")),
        &senza_geometria,
        &opzioni_scrittura_loose(),
    ) else {
        panic!("uno shapefile senza colonna geometria non e' uno shapefile");
    };
    // Il rifiuto **non** e' piu' quello di `create`: dal 2026-09-04 arriva
    // prima dal capability-check, che sa dal descrittore quali bersagli
    // incorporano o fissano un CRS e quindi pretendono una geometria. E' la
    // stessa forma del tetto sui nomi di campo qui sotto -- una capability
    // del bersaglio, pronunciata in validazione con la propria ragione,
    // invece di un errore di formato scoperto mentre si scrive.
    assert_eq!(
        errore.message,
        "il formato di destinazione richiede una colonna geometrica, e il layer non ne ha"
    );
    assert_eq!(
        errore.capability_reason,
        Some(plenora_io_model::CapabilityReason::GeometryNotSupported),
        "l'asse rifiutato e' la geometria, e la ragione lo dice"
    );

    // Nome campo oltre i dieci caratteri che il DBF porta: il rifiuto
    // **non** e' quello di `create`. Arriva prima dal capability-check, che
    // conosce il tetto sui nomi dal descrittore e lo applica a ogni driver
    // allo stesso modo. La riga di `create` che costruisce il `FieldName`
    // resta come difesa di tipo e non ha input.
    let Err(errore) = ShpDriver.create(
        Sink::Path(dir.path().join("nome-lungo.shp")),
        &piano_di_scrittura(vec![Field::new(
            "un_nome_troppo_lungo",
            arrow_schema::DataType::Utf8,
            true,
        )]),
        &opzioni_scrittura_loose(),
    ) else {
        panic!("un nome oltre il limite del DBF non puo' essere troncato in silenzio");
    };
    assert_eq!(
        errore.message, "nome oltre il limite del formato",
        "il rifiuto deve venire dal capability-check, non da `create`"
    );
    assert_eq!(
        errore.capability_reason,
        Some(plenora_io_model::CapabilityReason::FieldNameTooLong),
        "la ragione dichiarata e' cio' che rende il rifiuto interpretabile"
    );

    // La forma a directory: la destinazione che esiste gia'.
    let cartella = dir.path().join("occupata.shp.d");
    std::fs::create_dir(&cartella).unwrap();
    let Err(errore) = ShpDriver.create(
        Sink::Path(cartella),
        &piano_di_scrittura(Vec::new()),
        &opzioni_scrittura(),
    ) else {
        panic!("una directory che esiste non e' una destinazione libera");
    };
    assert_eq!(
        errore.code,
        plenora_io_model::IoErrorCode::OutputExists,
        "il rifiuto e' il no-clobber: {}",
        errore.message
    );

    // La forma a file sciolti: **ciascuna** delle quattro estensioni
    // compagne blocca la pubblicazione, anche quando il `.shp` non c'e'.
    for estensione in ["shp", "shx", "dbf", "prj"] {
        let destinazione = dir.path().join(format!("occupato-{estensione}.shp"));
        std::fs::write(destinazione.with_extension(estensione), b"").unwrap();
        let Err(errore) = ShpDriver.create(
            Sink::Path(destinazione),
            &piano_di_scrittura(Vec::new()),
            &opzioni_scrittura_loose(),
        ) else {
            panic!("un compagno `.{estensione}` gia' presente blocca il set");
        };
        assert_eq!(
            errore.code,
            plenora_io_model::IoErrorCode::OutputExists,
            "«{estensione}»: {}",
            errore.message
        );
    }
}

/// `write` nomina la causa di ogni riga che non sa scrivere, e non ne
/// scrive nessuna finche' una sola e' rifiutata.
///
/// Le cause sono cinque e sono diverse per chi ripara il dato: una
/// geometria assente, un WKB che non si decodifica, una forma che
/// Shapefile non rappresenta, un tipo diverso da quello che il file ha gia'
/// -- il formato ne porta uno solo -- e una cella che il DBF non sa
/// portare. Un'unica causa «riga non scrivibile» manderebbe a cercare nel
/// posto sbagliato quattro volte su cinque.
///
/// La proprieta' che le tiene insieme e' che il rifiuto arriva **prima** di
/// scrivere: `write` prepara tutte le righe e le consegna al writer solo se
/// nessuna e' stata rifiutata. Un file con dentro meta' batch sarebbe
/// peggio di nessun file.
// Cinque cause, la colonna non binaria e il controllo positivo in un
// test solo: separarli spezzerebbe il confronto che li rende
// interpretabili -- quale causa arriva davvero, e da chi.
#[allow(clippy::too_many_lines)]
#[test]
fn n1_write_nomina_la_causa_di_ogni_riga_che_non_sa_scrivere() {
    let dir = tempfile::tempdir().unwrap();
    let punto = to_wkb(&geo_types::Geometry::Point(geo_types::Point::new(1.0, 2.0))).unwrap();
    let collezione = encode_wkb(
        &WkbGeometry {
            value: WkbValue::GeometryCollection(Vec::new()),
            dimensions: CoordinateDimensions::Xy,
            srid: None,
        },
        WkbFlavor::Iso,
    )
    .expect("una collezione vuota si codifica");

    let scrittore = |nome: &str, campi: Vec<Field>| {
        ShpDriver
            .create(
                Sink::Path(dir.path().join(nome)),
                &piano_di_scrittura(campi),
                &opzioni_scrittura_loose(),
            )
            .expect("la destinazione e' libera")
    };
    let schema_solo_geometria: SchemaRef =
        Arc::new(Schema::new(vec![geometry_field(GEOMETRY, "EPSG:4326")]));
    let lotto = |geometrie: Vec<Option<&[u8]>>| {
        RecordBatch::try_new(
            schema_solo_geometria.clone(),
            vec![Arc::new(BinaryArray::from(geometrie))],
        )
        .unwrap()
    };

    for (caso, geometrie, causa) in [
        // Il rifiuto **non** e' quello del driver. `with_write_validation`
        // decodifica il WKB prima di consegnare il lotto, e i byte che non
        // sono WKB non arrivano mai a `ShpWriter::write`: la sua causa
        // `shapefile.invalid_geometry` resta una difesa senza input. La
        // riga successiva mostra invece dove il driver decide da se': una
        // GeometryCollection e' WKB valido, e solo Shapefile sa di non
        // saperla rappresentare.
        (
            "byte che non sono WKB",
            vec![Some(b"non e' WKB".as_slice())],
            "conversion.invalid_geometry",
        ),
        (
            "forma che WKB rappresenta e Shapefile no",
            vec![Some(collezione.as_slice())],
            "shapefile.geometry_not_representable",
        ),
    ] {
        let mut writer = scrittore(&format!("{}.shp", causa.replace('.', "-")), Vec::new());
        writer
            .declare_input_total(LayerId(0), 1)
            .expect("il totale d'ingresso si dichiara prima di scrivere");
        let Err(errore) = writer.write(&lotto(geometrie)) else {
            panic!("{caso}: doveva essere rifiutata");
        };
        let diagnostica = errore
            .row_diagnostics
            .as_deref()
            .expect("un rifiuto di riga porta la propria diagnostica");
        assert_eq!(
            diagnostica.counts.get(causa),
            Some(&1),
            "{caso}: causa attesa «{causa}», arrivate {:?}",
            diagnostica.counts
        );
    }

    // La colonna geometria che non e' binaria: non e' un rifiuto di riga
    // ma del lotto, perche' non c'e' una riga da incolpare. E anche qui il
    // rifiuto **non** e' quello del driver: `with_write_validation`
    // confronta il lotto con il contratto dichiarato prima di consegnarlo,
    // e un tipo Arrow diverso non passa quel confronto. La causa
    // «colonna geometria non binaria» di `ShpWriter::write` resta una
    // difesa senza input, come `shapefile.invalid_geometry`.
    let schema_testo: SchemaRef = Arc::new(Schema::new(vec![Field::new(
        GEOMETRY,
        arrow_schema::DataType::Utf8,
        true,
    )
    .with_metadata(geometry_field(GEOMETRY, "EPSG:4326").metadata().clone())]));
    let non_binaria = RecordBatch::try_new(
        schema_testo,
        vec![Arc::new(arrow_array::StringArray::from(vec![Some(
            "POINT (1 2)",
        )]))],
    )
    .unwrap();
    let mut writer = scrittore("non-binaria.shp", Vec::new());
    writer.declare_input_total(LayerId(0), 1).unwrap();
    let Err(errore) = writer.write(&non_binaria) else {
        panic!("una colonna geometria che non e' WKB non e' una geometria");
    };
    assert_eq!(
            errore.message,
            "batch diverso dal contratto dichiarato (schema, ordine, tipi, nullability o metadata) al layer 0"
        );
    assert!(
        errore.row_diagnostics.is_none(),
        "non c'e' una riga da incolpare: e' il lotto a essere sbagliato"
    );

    // Una cella che il DBF non sa portare: il tipo Arrow non ha una resa
    // testuale decisa, e la riga viene rifiutata con la propria causa.
    //
    // Il tipo e' una **durata**, non piu' una data. Lo Shapefile dichiara
    // `TypeCoercionPolicy::ExplicitText` e ammette la classe `Temporal`:
    // dal 2026-09-04 una data diventa il proprio testo ISO, come su ogni
    // altro formato testuale, e pretendere qui il rifiuto vorrebbe dire
    // scrivere nella prova la contraddizione che la capability aveva.
    // Una durata invece resta senza resa: non e' un istante, e la sua
    // forma testuale e' una scelta di rappresentazione che nessuno ha
    // preso.
    let schema_data: SchemaRef = Arc::new(Schema::new(vec![
        geometry_field(GEOMETRY, "EPSG:4326"),
        Field::new(
            "QUANDO",
            arrow_schema::DataType::Duration(arrow_schema::TimeUnit::Second),
            true,
        ),
    ]));
    let con_data = RecordBatch::try_new(
        schema_data,
        vec![
            Arc::new(BinaryArray::from(vec![Some(punto.as_slice())])),
            Arc::new(arrow_array::DurationSecondArray::from(vec![Some(90_i64)])),
        ],
    )
    .unwrap();
    let mut writer = scrittore(
        "cella.shp",
        vec![Field::new(
            "QUANDO",
            arrow_schema::DataType::Duration(arrow_schema::TimeUnit::Second),
            true,
        )],
    );
    writer.declare_input_total(LayerId(0), 1).unwrap();
    let Err(errore) = writer.write(&con_data) else {
        panic!("una cella senza resa DBF non puo' diventare un valore approssimato");
    };
    let diagnostica = errore.row_diagnostics.as_deref().unwrap();
    assert_eq!(
        diagnostica.counts.get("shapefile.cell_not_representable"),
        Some(&1),
        "arrivate {:?}",
        diagnostica.counts
    );

    // Il controllo positivo: un lotto interamente scrivibile passa, e senza
    // di lui una tabella di soli rifiuti la passerebbe anche un `write` che
    // rifiuta tutto.
    let mut writer = scrittore("buono.shp", Vec::new());
    writer.declare_input_total(LayerId(0), 1).unwrap();
    writer
        .write(&lotto(vec![Some(punto.as_slice())]))
        .expect("un punto XY e' scrivibile");
}

/// `write_shape` scrive `NullShape` e rifiuta Multipatch, e `write` non
/// gli manda mai il secondo.
///
/// Il Multipatch e' una difesa di tipo, non un rifiuto raggiungibile:
/// `ShpWriter::write` lo ferma prima come
/// `shapefile.geometry_type_unsupported` guardando `shape_tag`, e
/// `shape_from_wkb` non lo produce comunque -- come la sonda del suo gruppo
/// verifica su tutte le coppie costruibili.
///
/// `NullShape` invece **e'** raggiungibile, ed e' voluto: lo costruisce
/// `ShpWriter::write` quando la colonna geometrica e' nulla, e finisce nel
/// file come record di shape type 0.
#[test]
fn n1_write_shape_rifiuta_nullshape_e_multipatch_ma_write_non_ce_li_manda() {
    let dir = tempfile::tempdir().unwrap();
    let percorso = dir.path().join("scarto.shp");
    let tabella = TableWriterBuilder::new();
    let mut writer = Writer::from_path(&percorso, tabella).expect("il writer si apre");
    let record = Record::default();

    // `NullShape` **si scrive**, dal fork governato in poi: e' un record
    // che la specifica ammette, e la sonda che ne pretendeva il rifiuto
    // scriveva nel test il difetto invece del contratto. La strada per
    // arrivarci resta chiusa a `shape_from_wkb`, che non lo produce mai:
    // a costruirlo e' `ShpWriter::write`, quando la colonna geometrica e'
    // nulla, e la prova di quel percorso sta nel proprio gruppo.
    write_shape(&mut writer, Shape::NullShape, &record)
        .expect("un record con geometria nulla si scrive");

    let multipatch = Shape::Multipatch(shapefile::Multipatch::new(
        shapefile::Patch::TriangleStrip(vec![
            shapefile::PointZ::new(0.0, 0.0, 0.0, NO_DATA),
            shapefile::PointZ::new(1.0, 0.0, 0.0, NO_DATA),
            shapefile::PointZ::new(0.0, 1.0, 0.0, NO_DATA),
        ]),
    ));
    let Err(errore) = write_shape(&mut writer, multipatch, &record) else {
        panic!("il Multipatch non si scrive con questo driver");
    };
    assert_eq!(
        errore.message,
        "Multipatch non supportato in scrittura Shapefile"
    );

    // L'altra meta': le due cause con cui `write` ferma prima. La geometria
    // assente e' gia' provata nella tabella delle cause; qui conta che il
    // tag `unsupported` esista e sia quello del Multipatch, perche' e' il
    // confronto che chiude la strada.
    assert_eq!(
        shape_tag(&Shape::Multipatch(shapefile::Multipatch::new(
            shapefile::Patch::TriangleStrip(vec![shapefile::PointZ::new(0.0, 0.0, 0.0, NO_DATA)]),
        ))),
        "unsupported",
        "e' il tag su cui `write` rifiuta la riga prima di costruire la shape"
    );
    assert_eq!(
        shape_tag(&Shape::NullShape),
        "",
        "la NullShape non ha tag, e `write` non la fa mai arrivare qui"
    );
}

/// `finish` conta i byte dello staging e non pubblica oltre il tetto.
///
/// Il conteggio precede la pubblicazione, e non e' un dettaglio d'ordine:
/// un tetto controllato dopo avrebbe gia' scritto i file, e «superato il
/// limite» sarebbe una constatazione invece di un rifiuto. Il tetto vale
/// sull'insieme delle quattro parti, non su ciascuna: e' il set a essere
/// l'unita' pubblicata.
#[test]
fn n1_finish_conta_i_byte_dello_staging_prima_di_pubblicare() {
    let dir = tempfile::tempdir().unwrap();
    let punto = to_wkb(&geo_types::Geometry::Point(geo_types::Point::new(1.0, 2.0))).unwrap();
    let schema: SchemaRef = Arc::new(Schema::new(vec![geometry_field(GEOMETRY, "EPSG:4326")]));
    let lotto = RecordBatch::try_new(
        schema,
        vec![Arc::new(BinaryArray::from(vec![Some(punto.as_slice())]))],
    )
    .unwrap();

    // Prima la misura: quanti byte occupa davvero il set di una riga. Il
    // tetto va scelto **sotto** quel valore e sopra cio' che la validazione
    // della scrittura deriva dall'input osservato, altrimenti a rifiutare
    // sarebbe quella e non il conteggio di `finish`.
    let riferimento = dir.path().join("riferimento.shp");
    let mut writer = ShpDriver
        .create(
            Sink::Path(riferimento.clone()),
            &piano_di_scrittura(Vec::new()),
            &opzioni_scrittura_loose(),
        )
        .expect("la destinazione e' libera");
    writer.declare_input_total(LayerId(0), 1).unwrap();
    writer.write(&lotto).expect("il punto si scrive");
    let pubblicato = writer.finish().expect("senza tetto stretto si pubblica");
    let tetto = pubblicato.bytes - 1;

    let destinazione = dir.path().join("stretto.shp");
    let mut writer = ShpDriver
        .create(
            Sink::Path(destinazione.clone()),
            &piano_di_scrittura(Vec::new()),
            &opzioni_scrittura_con(
                plenora_io_model::budget::PipelineLimits::default().with_max_output_bytes(tetto),
            )
            .with_format_option("publish_mode", LOOSE_SET_MODE),
        )
        .expect("la destinazione e' libera");
    writer.declare_input_total(LayerId(0), 1).unwrap();
    writer
        .write(&lotto)
        .expect("il punto si scrive nello staging");
    let Err(errore) = writer.finish() else {
        panic!("un set oltre il tetto non si pubblica");
    };
    assert_eq!(
        errore.category,
        plenora_io_model::ErrorCategory::ResourceLimit,
        "un tetto superato e' un limite: {}",
        errore.message
    );
    assert!(
        errore.message.contains("byte oltre il limite di"),
        "il messaggio deve dire qual e' il limite: «{}»",
        errore.message
    );
    for estensione in ["shp", "shx", "dbf", "prj"] {
        assert!(
            !destinazione.with_extension(estensione).exists(),
            "il rifiuto precede la pubblicazione: `.{estensione}` non deve esistere"
        );
    }

    // Il controllo positivo e' la corsa di riferimento qui sopra: stesso
    // lotto, stesso driver, tetto sufficiente, e il set pubblicato.
    assert!(
        riferimento.exists() && riferimento.with_extension("dbf").exists(),
        "sotto il tetto il set si pubblica per intero"
    );
    assert!(
        tetto < pubblicato.bytes,
        "il tetto della prova negativa sta sotto la misura, non accanto"
    );
}

/// Uno schema DBF minimo per provare `ShpRowDiagnosticsConfig::from_options`.
///
/// Le colonne e il layout servono a due cose diverse: le prime dicono se il
/// campo chiave esista, il secondo dice se sia numerico -- e quindi se il
/// valore grezzo vada tenuto da parte per la diagnostica. Tenerli separati
/// nella fixture riflette il fatto che nel codice sono due fonti distinte,
/// e una sola non basta a rispondere a entrambe le domande.
fn schema_di_prova() -> (Vec<ShpColumn>, DbfLayout) {
    let colonne = vec![
        ShpColumn {
            name: "NOME".to_owned(),
            column_type: ColType::Text,
            exact_integer_slot: None,
        },
        ShpColumn {
            name: "CODICE".to_owned(),
            column_type: ColType::Integer,
            exact_integer_slot: Some(0),
        },
    ];
    let layout = DbfLayout {
        header_length: 65,
        record_length: 21,
        record_count: 0,
        fields: vec![
            DbfFieldLayout {
                name: "NOME".to_owned(),
                field_type: b'C',
                offset: 1,
                width: 10,
                exact_integer_slot: None,
            },
            DbfFieldLayout {
                name: "CODICE".to_owned(),
                field_type: b'N',
                offset: 11,
                width: 10,
                exact_integer_slot: Some(0),
            },
        ],
        exact_integer_count: 1,
    };
    (colonne, layout)
}

/// `from_options` rifiuta ogni configurazione incoerente delle diagnostiche
/// di riga, e ciascun rifiuto dice che cosa correggere.
///
/// Sono opzioni che chi legge scrive a mano sulla riga di comando, e i
/// quattro messaggi separano quattro errori diversi: un limite che non e' un
/// numero, un limite fuori intervallo, una policy senza il campo a cui si
/// applica, e una policy che non esiste. «Configurazione non valida» li
/// coprirebbe tutti e non aiuterebbe nessuno.
///
/// Il caso della policy senza campo e' quello che vale di piu': non e' un
/// valore sbagliato ma una **coppia** incompleta, e senza il rifiuto la
/// policy verrebbe ignorata in silenzio. Chi l'ha scritta crederebbe di aver
/// redatto una chiave che invece non viene nemmeno emessa.
// Sei rifiuti, i due confini del limite e quattro accettazioni: la
// lunghezza e' quella della classe di equivalenza, non di un test che fa
// troppe cose.
#[allow(clippy::too_many_lines)]
#[test]
fn n1_from_options_rifiuta_ogni_configurazione_incoerente_delle_diagnostiche() {
    let (colonne, layout) = schema_di_prova();
    let opzioni = |coppie: &[(&str, &str)]| -> BTreeMap<String, String> {
        coppie
            .iter()
            .map(|(chiave, valore)| ((*chiave).to_owned(), (*valore).to_owned()))
            .collect()
    };

    for (caso, coppie, atteso) in [
        (
            "limite che non e' un intero",
            vec![("row_diagnostics.examples_limit", "molti")],
            "row_diagnostics.examples_limit deve essere un intero",
        ),
        (
            "limite negativo, che non e' un u64",
            vec![("row_diagnostics.examples_limit", "-1")],
            "row_diagnostics.examples_limit deve essere un intero",
        ),
        (
            "policy senza il campo a cui si applica",
            vec![("row_diagnostics.key_policy", "emit")],
            "row_diagnostics.key_policy richiede row_diagnostics.key_field",
        ),
        (
            "campo chiave che lo schema DBF non ha",
            vec![("row_diagnostics.key_field", "ASSENTE")],
            "row_diagnostics.key_field non esiste nello schema DBF",
        ),
        (
            "policy che non e' ne' emit ne' redact",
            vec![
                ("row_diagnostics.key_field", "NOME"),
                ("row_diagnostics.key_policy", "forse"),
            ],
            "row_diagnostics.key_policy deve essere 'emit' o 'redact'",
        ),
        (
            "campo chiave senza policy: la scelta non ha un default implicito",
            vec![("row_diagnostics.key_field", "NOME")],
            "row_diagnostics.key_policy deve essere 'emit' o 'redact'",
        ),
    ] {
        let esito = ShpRowDiagnosticsConfig::from_options(&opzioni(&coppie), &colonne, &layout);
        let Err(errore) = esito else {
            panic!("{caso}: doveva essere rifiutata");
        };
        assert_eq!(errore.message, atteso, "{caso}: messaggio sbagliato");
        assert_eq!(
            errore.category,
            plenora_io_model::ErrorCategory::InvalidConfiguration,
            "{caso}: e' una configurazione sbagliata, non un dato sbagliato"
        );
        assert_eq!(
            errore.phase,
            plenora_io_model::ErrorPhase::Validate,
            "{caso}: il rifiuto deve arrivare prima di leggere il file"
        );
    }

    // I due confini del limite, uno accanto all'altro: il rifiuto e'
    // fuori, l'accettazione dentro. Con una sola meta' non si saprebbe se
    // l'intervallo sia chiuso o aperto.
    for (caso, valore) in [
        ("zero, sotto il minimo", "0"),
        (
            "uno oltre il massimo",
            &(MAX_ROW_DIAGNOSTICS_EXAMPLES_LIMIT + 1).to_string(),
        ),
    ] {
        let esito = ShpRowDiagnosticsConfig::from_options(
            &opzioni(&[("row_diagnostics.examples_limit", valore)]),
            &colonne,
            &layout,
        );
        let Err(errore) = esito else {
            panic!("{caso}: doveva essere rifiutata");
        };
        assert!(
            errore
                .message
                .starts_with("row_diagnostics.examples_limit deve essere compreso fra 1 e"),
            "{caso}: arrivato «{}»",
            errore.message
        );
    }

    for (caso, valore, atteso) in [
        ("il minimo", "1", 1),
        (
            "il massimo",
            &MAX_ROW_DIAGNOSTICS_EXAMPLES_LIMIT.to_string(),
            MAX_ROW_DIAGNOSTICS_EXAMPLES_LIMIT,
        ),
    ] {
        let config = match ShpRowDiagnosticsConfig::from_options(
            &opzioni(&[("row_diagnostics.examples_limit", valore)]),
            &colonne,
            &layout,
        ) {
            Ok(config) => config,
            Err(errore) => panic!("{caso}: doveva essere accettato: {errore:?}"),
        };
        assert_eq!(config.examples_limit, atteso, "{caso}: limite perso");
    }

    // Nessuna opzione: il limite predefinito, e **nessuna** chiave. Il
    // commento del tipo lo dice -- «non esiste una policy implicita» -- e
    // senza questa riga un default che comparisse dal nulla passerebbe.
    let vuota = ShpRowDiagnosticsConfig::from_options(&opzioni(&[]), &colonne, &layout)
        .expect("nessuna opzione e' una configurazione valida");
    assert_eq!(vuota.examples_limit, DEFAULT_ROW_DIAGNOSTICS_EXAMPLES_LIMIT);
    assert!(
        vuota.key.is_none(),
        "senza key_field gli esempi non portano alcun oggetto chiave"
    );

    // Le due policy, e l'indice del campo numerico grezzo: `CODICE` e' di
    // tipo `N`, quindi il valore grezzo va tenuto; `NOME` e' `C`, e non
    // c'e' niente da tenere. Le due meta' vengono da fonti diverse -- le
    // colonne per l'esistenza, il layout per il tipo -- e una sola non
    // risponderebbe a entrambe.
    for (caso, campo, policy, indice_grezzo) in [
        ("campo testuale con emit", "NOME", "emit", None),
        ("campo testuale con redact", "NOME", "redact", None),
        ("campo numerico con emit", "CODICE", "emit", Some(1)),
    ] {
        let config = match ShpRowDiagnosticsConfig::from_options(
            &opzioni(&[
                ("row_diagnostics.key_field", campo),
                ("row_diagnostics.key_policy", policy),
            ]),
            &colonne,
            &layout,
        ) {
            Ok(config) => config,
            Err(errore) => panic!("{caso}: doveva essere accettato: {errore:?}"),
        };
        let Some(chiave) = config.key else {
            panic!("{caso}: la chiave doveva esserci");
        };
        assert_eq!(chiave.field, campo, "{caso}: campo perso");
        assert_eq!(
            matches!(chiave.policy, DiagnosticKeyPolicy::Emit),
            policy == "emit",
            "{caso}: policy invertita"
        );
        assert_eq!(
            chiave.raw_numeric_field_index, indice_grezzo,
            "{caso}: il valore grezzo si tiene solo per i campi numerici"
        );
    }
}

/// `__fuzz_wkb_roundtrip` propaga ogni rifiuto della catena e chiude il
/// giro su cio' che l'attraversa.
///
/// E' l'entry point del target del fuzzer, e la sua utilita' dipende da una
/// cosa sola: che i rifiuti vengano dalla catena vera -- decodifica,
/// conversione nella shape ESRI, ritorno a WKB -- e non da un controllo
/// scritto nel target. Una sonda che guardasse solo «non va in panico» non
/// distinguerebbe un target che legge davvero da uno che rifiuta tutto
/// all'ingresso.
#[test]
fn n1_fuzz_wkb_roundtrip_propaga_i_rifiuti_della_catena() {
    // Byte che non sono WKB: il rifiuto viene dalla decodifica.
    assert!(
        __fuzz_wkb_roundtrip(b"non e' WKB").is_err(),
        "una sequenza che non e' WKB non attraversa la catena"
    );
    assert!(
        __fuzz_wkb_roundtrip(&[]).is_err(),
        "zero byte non sono una geometria"
    );

    // Una geometria vera attraversa e torna: la lunghezza restituita e'
    // quella del WKB rigenerato, quindi non zero.
    let punto = encode_wkb(
        &WkbGeometry {
            value: WkbValue::Point(WkbCoordinate {
                x: 1.0,
                y: 2.0,
                z: None,
                m: None,
            }),
            dimensions: CoordinateDimensions::Xy,
            srid: None,
        },
        WkbFlavor::Iso,
    )
    .expect("un punto XY si codifica");
    assert!(
        __fuzz_wkb_roundtrip(&punto).expect("un punto attraversa la catena") > 0,
        "il giro completo deve produrre byte, non un contatore a zero"
    );

    // Una forma che WKB rappresenta e Shapefile no: il rifiuto viene dalla
    // conversione, non dalla decodifica.
    let collezione = encode_wkb(
        &WkbGeometry {
            value: WkbValue::GeometryCollection(Vec::new()),
            dimensions: CoordinateDimensions::Xy,
            srid: None,
        },
        WkbFlavor::Iso,
    )
    .expect("una collezione vuota si codifica");
    assert!(
        __fuzz_wkb_roundtrip(&collezione).is_err(),
        "una GeometryCollection non ha una shape ESRI corrispondente"
    );
}
