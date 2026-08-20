"""Sonde del divieto di promuovere testo runtime a `'static`.

La proprieta' provata non e' «zero occorrenze» ma:

> zero occorrenze non autorizzate; **una sola** dimostrazione eseguibile e
> identificata.

Le due cose si provano separatamente, perche' un gate che verificasse solo la
prima resterebbe verde se la dimostrazione sparisse, e uno che verificasse solo
la seconda lascerebbe passare un `Box::leak` in un altro esempio della
documentazione.
"""

from __future__ import annotations

import shutil
import tempfile
import unittest
from pathlib import Path

from scripts import check_niente_leak as gate

PROMOZIONE = 'let _: &\'static str = Box::leak(String::from("x").into_boxed_str());'


def doctest_di_modulo(*righe: str) -> str:
    """Un blocco doctest `//!` che deve compilare.

    Costruito riga per riga: dentro un letterale multilinea i tre backtick e le
    sequenze di escape si mescolano male, e la fixture di un test deve essere
    leggibile a colpo d'occhio.
    """
    blocco = ["//! ```"]
    blocco.extend("//! " + riga for riga in righe)
    blocco.append("//! ```")
    return "\n".join(blocco) + "\n"


# La dimostrazione autorizzata, come vive in `plenora-io-model`.
DIMOSTRAZIONE = doctest_di_modulo(
    "// " + gate.ATTESTAZIONE + " — unica occorrenza autorizzata.",
    PROMOZIONE,
)

PULITO = """use crate::error::PublicMessage;

macro_rules! otto_volte {
    ($pezzo:expr) => {
        concat!($pezzo, $pezzo, $pezzo, $pezzo, $pezzo, $pezzo, $pezzo, $pezzo)
    };
}

const LUNGO: &str = otto_volte!(otto_volte!("x"));

fn messaggio() -> PublicMessage {
    PublicMessage::Curated(LUNGO)
}
"""


class SondeLeak(unittest.TestCase):
    def albero(self, sostituzioni: dict[str, str] | None = None) -> Path:
        """Un albero che contiene **gia'** la dimostrazione attestata.

        E' lo stato normale del repository: le sonde partono da li' e mutano
        una cosa alla volta.
        """
        radice = Path(tempfile.mkdtemp())
        self.addCleanup(shutil.rmtree, radice, True)
        contenuti = {
            gate.SORGENTE_ATTESTATA: DIMOSTRAZIONE + PULITO,
            "crates/driver-finto/src/lib.rs": PULITO,
        }
        contenuti.update(sostituzioni or {})
        for relativo, testo in contenuti.items():
            percorso = radice / relativo
            percorso.parent.mkdir(parents=True, exist_ok=True)
            percorso.write_text(testo, encoding="utf-8")
        return radice

    # --- lo stato normale ---------------------------------------------------

    def test_lo_stato_normale_passa(self) -> None:
        self.assertEqual(gate.violazioni(self.albero()), [])

    # --- primo dovere: nessuna occorrenza non autorizzata -------------------

    def test_box_leak_in_produzione_e_rosso(self) -> None:
        con_leak = PULITO + """
fn promuove(testo: String) -> &'static str {
    Box::leak(testo.into_boxed_str())
}
"""
        errori = gate.violazioni(
            self.albero({"crates/driver-finto/src/lib.rs": con_leak})
        )
        self.assertTrue(
            any("::promuove" in voce for voce in errori),
            f"`Box::leak` in produzione non intercettato: {errori}",
        )

    def test_string_leak_e_rosso(self) -> None:
        con_leak = PULITO + """
fn promuove(testo: String) -> &'static str {
    String::leak(testo)
}
"""
        self.assertTrue(
            gate.violazioni(self.albero({"crates/driver-finto/src/lib.rs": con_leak})),
            "`String::leak` non intercettato",
        )

    def test_la_forma_a_metodo_e_rossa(self) -> None:
        """`testo.leak()` non nomina il tipo, e sfuggirebbe a una regex sui
        soli percorsi `Tipo::leak`."""
        con_leak = PULITO + """
fn promuove(testo: String) -> &'static str {
    testo.leak()
}
"""
        self.assertTrue(
            gate.violazioni(self.albero({"crates/driver-finto/src/lib.rs": con_leak})),
            "la forma a metodo non e' intercettata",
        )

    def test_vale_anche_nel_codice_di_test(self) -> None:
        """L'occorrenza che ha reso necessario questo gate era **in un test**.

        Un test che si costruisce lo statico a runtime dimostra proprio cio'
        che S9 non promette. Un divieto limitato alla produzione non l'avrebbe
        intercettato.
        """
        con_leak = PULITO + """
#[cfg(test)]
mod tests {
    #[test]
    fn statico_lungo() {
        let _: &'static str = Box::leak(String::from("x").into_boxed_str());
    }
}
"""
        self.assertTrue(
            gate.violazioni(self.albero({"crates/driver-finto/src/lib.rs": con_leak})),
            "il codice di test e' escluso dal divieto",
        )

    # --- i doctest sono nel perimetro ---------------------------------------

    def test_un_doctest_non_attestato_e_rosso(self) -> None:
        """Escludere i doctest in blocco sarebbe una deroga **piu' ampia** di
        una allowlist: un esempio della documentazione e' la prima cosa che un
        consumatore copia."""
        errori = gate.violazioni(
            self.albero(
                {"crates/driver-finto/src/lib.rs": doctest_di_modulo(PROMOZIONE) + PULITO}
            )
        )
        self.assertTrue(
            any("crates/driver-finto/src/lib.rs" in voce for voce in errori),
            f"il doctest non attestato non e' stato intercettato: {errori}",
        )

    def test_il_marcatore_non_autorizza_in_un_altro_file(self) -> None:
        """L'attestazione e' legata al file, non al solo marcatore: altrimenti
        chiunque potrebbe autorizzarsi copiando una riga di commento."""
        errori = gate.violazioni(
            self.albero({"crates/driver-finto/src/lib.rs": DIMOSTRAZIONE + PULITO})
        )
        self.assertTrue(
            any("crates/driver-finto/src/lib.rs" in voce for voce in errori),
            f"il marcatore ha autorizzato un file qualunque: {errori}",
        )

    def test_una_menzione_in_un_commento_non_conta(self) -> None:
        """Il gate non deve contare la propria motivazione.

        E' lo stesso difetto gia' incontrato nel registro dei fallback, dove un
        commento muoveva il contatore.
        """
        con_commento = (
            "// `Box::leak` promuoverebbe testo runtime a 'static: vietato.\n"
            "/// Vedi anche `String::leak`, che fa lo stesso.\n" + PULITO
        )
        errori = gate.violazioni(
            self.albero({"crates/driver-finto/src/lib.rs": con_commento})
        )
        self.assertEqual(errori, [], f"un commento e' stato contato: {errori}")

    def test_una_menzione_in_un_commento_dentro_un_doctest_non_conta(self) -> None:
        con_commento = (
            doctest_di_modulo("// qui `Box::leak(...)` sarebbe vietato", "let _ = 1;")
            + PULITO
        )
        errori = gate.violazioni(
            self.albero({"crates/driver-finto/src/lib.rs": con_commento})
        )
        self.assertEqual(errori, [], f"un commento nel doctest e' contato: {errori}")

    def test_una_stringa_che_contiene_leak_non_conta(self) -> None:
        con_stringa = PULITO + """
fn messaggio_di_errore() -> &'static str {
    "Box::leak(...) non e' ammesso in questo punto"
}
"""
        errori = gate.violazioni(
            self.albero({"crates/driver-finto/src/lib.rs": con_stringa})
        )
        self.assertEqual(errori, [], f"una stringa e' stata contata: {errori}")

    # --- secondo dovere: l'attestazione e' esattamente una ------------------

    def test_l_attestazione_che_sopravvive_al_proprio_codice_e_rossa(self) -> None:
        """Il caso che un semplice divieto non coprirebbe.

        Se la dimostrazione viene tolta ma l'autorizzazione resta, il gate
        autorizza qualcosa che nessuno rilegge — e la prossima occorrenza
        entrerebbe sotto una deroga scritta per un'altra.
        """
        senza_dimostrazione = "// " + gate.ATTESTAZIONE + "\n" + PULITO
        errori = gate.violazioni(
            self.albero({gate.SORGENTE_ATTESTATA: senza_dimostrazione})
        )
        self.assertTrue(
            any("non esiste piu'" in voce for voce in errori),
            f"l'attestazione fantasma non e' stata intercettata: {errori}",
        )

    def test_due_attestazioni_sono_rosse(self) -> None:
        """Una deroga che cresce non e' piu' una deroga."""
        errori = gate.violazioni(
            self.albero({gate.SORGENTE_ATTESTATA: DIMOSTRAZIONE + DIMOSTRAZIONE + PULITO})
        )
        self.assertTrue(
            any("una sola ammessa" in voce for voce in errori),
            f"la seconda attestazione non e' stata intercettata: {errori}",
        )

    def test_la_dimostrazione_resta_eseguibile(self) -> None:
        """Non e' marcata `ignore`, e non deve poterlo diventare.

        `doctest_che_devono_compilare` esclude `ignore` e `compile_fail`:
        marcare cosi' la dimostrazione la renderebbe invisibile al gate, che
        conterebbe zero attestazioni e diventerebbe rosso. La proprieta' si
        difende da sola.
        """
        marcata_ignore = DIMOSTRAZIONE.replace("//! ```\n", "//! ```ignore\n", 1)
        errori = gate.violazioni(
            self.albero({gate.SORGENTE_ATTESTATA: marcata_ignore + PULITO})
        )
        self.assertTrue(
            any("non esiste piu'" in voce for voce in errori),
            f"una dimostrazione marcata `ignore` e' passata: {errori}",
        )

    # --- perimetro ----------------------------------------------------------

    def test_i_target_di_fuzz_sono_nel_perimetro(self) -> None:
        con_leak = """fn promuove(testo: String) -> &'static str {
    Box::leak(testo.into_boxed_str())
}
"""
        errori = gate.violazioni(self.albero({"fuzz/fuzz_targets/finto.rs": con_leak}))
        self.assertTrue(
            any("fuzz/fuzz_targets/finto.rs" in voce for voce in errori),
            f"i target di fuzz restano fuori: {errori}",
        )


if __name__ == "__main__":
    unittest.main()
