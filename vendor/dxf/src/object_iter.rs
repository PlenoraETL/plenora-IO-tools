use crate::code_pair_put_back::{offset_dell_errore, CodePairPutBack};
use crate::objects::Object;

pub(crate) struct ObjectIter<'a> {
    pub iter: &'a mut CodePairPutBack,
}

impl Iterator for ObjectIter<'_> {
    type Item = Object;

    fn next(&mut self) -> Option<Object> {
        // Upstream trasforma l'errore in fine sequenza, e il chiamante
        // proseguiva come se la sezione fosse finita li': un'entita' illeggibile
        // dentro un BLOCK spariva, e il documento veniva accettato senza. Qui
        // l'errore ferma l'iteratore, e la lettura successiva del chiamante lo
        // porta fino al lettore esterno.
        match Object::read(self.iter) {
            Ok(Some(valore)) => Some(valore),
            Ok(None) => None,
            Err(errore) => {
                let offset = offset_dell_errore(&errore);
                let _ = self.iter.ferma(offset);
                None
            }
        }
    }
}
