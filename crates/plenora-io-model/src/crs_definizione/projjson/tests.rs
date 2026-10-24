//! Le prove di [`super`], in un file loro: il controllo dei doppioni alla
//! chiusura di un oggetto, che dalle prove pubbliche si raggiunge solo quando
//! due chiavi diverse hanno la stessa impronta, cioe' quasi mai.

use super::*;

fn chiavi_con_la_stessa_impronta(testo: &str) -> Vec<Chiave> {
    // Tutte le posizioni delle virgolette d'apertura di una chiave: in questi
    // documenti ogni chiave e' seguita da `:`.
    let byte = testo.as_bytes();
    let mut chiavi = Vec::new();
    let mut pos = 0;
    while pos < byte.len() {
        if byte[pos] == b'"' {
            let mut cursore = Cursore { byte, pos };
            cursore.salta_stringa().unwrap();
            if byte.get(cursore.pos) == Some(&b':') {
                chiavi.push(Chiave {
                    impronta: 7,
                    posizione: pos,
                });
            }
            pos = cursore.pos;
        } else {
            pos += 1;
        }
    }
    chiavi
}

/// Con tutte le impronte uguali il verdetto resta esatto: chiavi diverse
/// passano, la stessa chiave scritta in due grafie no.
#[test]
fn le_impronte_uguali_non_cambiano_il_verdetto() {
    let diverse = r#"{"a":1,"b":2,"ab":3,"\u0062a":4}"#;
    let mut chiavi = chiavi_con_la_stessa_impronta(diverse);
    assert_eq!(chiavi.len(), 4);
    assert_eq!(nessun_doppione(diverse.as_bytes(), &mut chiavi), Ok(()));

    let doppione = r#"{"a":1,"b":2,"\u0061":3}"#;
    let mut chiavi = chiavi_con_la_stessa_impronta(doppione);
    assert_eq!(chiavi.len(), 3);
    assert_eq!(
        nessun_doppione(doppione.as_bytes(), &mut chiavi),
        Err(Malformata)
    );

    // Un surrogato isolato e' un punto di codice come gli altri: uguale solo a
    // se stesso.
    let surrogati = r#"{"\ud800":1,"\udc00":2,"\ud800\udc00":3}"#;
    let mut chiavi = chiavi_con_la_stessa_impronta(surrogati);
    assert_eq!(nessun_doppione(surrogati.as_bytes(), &mut chiavi), Ok(()));
}

/// La prima chiave di ogni oggetto e' segnata: le chiavi di un oggetto chiuso
/// non si confondono con quelle dell'oggetto che lo contiene.
#[test]
fn le_chiavi_dell_oggetto_chiuso_non_contano_nel_contenitore() {
    assert_eq!(valida(br#"{"a":{"a":1},"b":{"a":{"a":2}}}"#), Ok(()));
    assert_eq!(valida(br#"{"a":{"b":1},"a":2}"#), Err(Malformata));
    assert_eq!(valida(br#"{"a":{"b":1,"b":2},"c":3}"#), Err(Malformata));
    assert_eq!(valida(br#"[{"a":1},{"a":1}]"#), Ok(()));
}

/// Della radice si tengono `id` e `ids`; un membro qualunque, anche enorme o
/// profondissimo, si attraversa senza materializzarlo.
#[test]
fn i_membri_qualunque_della_radice_non_si_materializzano() {
    let profondo = format!(
        r#"{{"x":{}{},"id":{{"authority":"EPSG","code":4326,"nota":[1,{{"y":2}}]}}}}"#,
        "[".repeat(100_000),
        "]".repeat(100_000)
    );
    let coppie = identificatori(&profondo).unwrap();
    assert_eq!(coppie.len(), 1);
    assert!(coppie[0].0.uguale_a("EPSG"));
    assert!(coppie[0].1.uguale_a("4326"));
}
