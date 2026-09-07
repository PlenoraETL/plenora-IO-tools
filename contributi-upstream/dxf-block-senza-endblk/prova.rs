// Da aggiungere in fondo a `src/block.rs`, dentro il `mod tests` che c'e' gia'.
//
// Usa gli stessi mattoni degli altri test del file: coppie costruite a mano,
// `DirectCodePairIter`, `Drawing::load_from_iter`. L'unica differenza e' che
// non passa da `drawing_from_pairs`, che fa `unwrap` e qui farebbe panicare
// invece di dire che l'esito e' un errore.
//
// Serve un import in piu' in cima al `mod tests`:
//
//     use crate::code_pair_iter::DirectCodePairIter;

#[test]
fn read_block_without_endblk_terminates() {
    // A `BLOCK` that never reaches its `ENDBLK` used to keep the reader looping
    // forever.  `Entity::read()` returns `Ok(None)` without consuming the
    // `0/ENDSEC` pair, so `read_block()` picked the same pair up on every trip
    // around its loop, allocating each time.
    //
    // Note what this test asserts: that the call *returns*.  The assertion on
    // the result comes second.  If the defect came back this test would not
    // fail - it would hang, and CI would time out rather than go red.  That is
    // a property of the defect, not of the test.
    let pairs = vec![
        CodePair::new_str(0, "SECTION"),
        CodePair::new_str(2, "BLOCKS"),
        CodePair::new_str(0, "BLOCK"),
        CodePair::new_str(2, "block-without-endblk"),
        CodePair::new_str(0, "ENDSEC"),
        CodePair::new_str(0, "EOF"),
    ];
    let iter = Box::new(DirectCodePairIter::new(pairs));
    let result = Drawing::load_from_iter(iter);
    assert!(
        result.is_err(),
        "a BLOCK without its ENDBLK is not a readable drawing"
    );
}

// La stessa prova sul percorso dei byte, se preferite provare il lettore
// completo invece dell'iteratore diretto. E' il file che sta in
// `riproduttore.dxf`, cinquantasette byte.
#[test]
fn read_block_without_endblk_terminates_from_bytes() {
    let input = "0\nSECTION\n2\nBLOCKS\n0\nBLOCK\n2\nblock-without-endblk\n0\nENDSEC\n0\nEOF\n";
    let result = Drawing::load(&mut input.as_bytes());
    assert!(
        result.is_err(),
        "a BLOCK without its ENDBLK is not a readable drawing"
    );
}
