//! Il limite logico manca nella validazione di `ArrayData` di Arrow 60:
//! `check_run_ends` confronta l'ultimo valore con la lunghezza del figlio,
//! mentre `RunArray` costruisce `RunEndBuffer::new_unchecked` con quella del padre.
use super::{errore, preleva_buffer, preleva_nodo, Cursori, FbField, Read, Result, Seek, SeekFrom};

pub(super) fn verifica_estensione_run_ends(
    driver: &'static str,
    campo: FbField<'_>,
    batch: arrow_ipc::RecordBatch<'_>,
    lunghezza: u64,
    lunghezza_corpo: u64,
    cursori: &mut Cursori<'_, '_>,
) -> Result<()> {
    let Some(intero) = campo.children().and_then(|f| f.get(0).type_as_int()) else {
        // Il tipo non intero viene rifiutato dalla validazione fallibile di Arrow.
        return Ok(());
    };
    let larghezza = match intero.bitWidth() {
        16 => 2,
        32 => 4,
        64 => 8,
        _ => return Ok(()),
    };
    // Preleviamo senza consumare: la visita ricorsiva deve ancora leggere il figlio.
    let nodo = cursori.nodo;
    let buffer = cursori.buffer;
    let (quanti, _) = preleva_nodo(driver, batch, cursori)?;
    let _ = preleva_buffer(driver, batch, lunghezza_corpo, cursori)?;
    let (offset, disponibili) = preleva_buffer(driver, batch, lunghezza_corpo, cursori)?;
    cursori.nodo = nodo;
    cursori.buffer = buffer;
    // valida_buffer ha gia' verificato l'estensione di tutti i descrittori nel corpo.
    let necessari = quanti
        .checked_mul(larghezza)
        .filter(|&n| n <= disponibili)
        .ok_or_else(|| errore(driver, "buffer dei run ends Arrow insufficiente"))?;
    let ultimo = if quanti == 0 {
        0
    } else {
        let posizione = cursori
            .corpo
            .inizio
            .checked_add(offset)
            .and_then(|n| n.checked_add(necessari - larghezza))
            .ok_or_else(|| errore(driver, "posizione dei run ends Arrow non rappresentabile"))?;
        let mut byte = [0_u8; 8];
        cursori.corpo.file.seek(SeekFrom::Start(posizione))?;
        let n = usize::try_from(larghezza)
            .map_err(|_| errore(driver, "larghezza run end non rappresentabile"))?;
        cursori.corpo.file.read_exact(&mut byte[..n])?;
        // Estensione del segno prima della conversione little-endian.
        if byte[n - 1] & 0x80 != 0 {
            byte[n..].fill(255);
        }
        i64::from_le_bytes(byte)
    };
    if u64::try_from(ultimo).map_or(true, |ultimo| ultimo < lunghezza) {
        return Err(errore(
            driver,
            "run ends Arrow piu' corti della lunghezza logica",
        ));
    }
    Ok(())
}
