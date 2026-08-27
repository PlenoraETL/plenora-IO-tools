#!/usr/bin/env python3
"""Sonde di `check_protocollo_v2.py`.

Un gate verde sul repository sano dice che oggi e' verde, non che domani
diventerebbe rosso. Ogni proprieta' che il gate afferma ha qui una sonda che la
viola su un manifesto finto e pretende il rosso.

Il manifesto finto porta numeri **tutti diversi fra loro** e diversi da quelli
veri. Con nove chiavi che valgono tutte 64 una permutazione passerebbe
inosservata, e la sonda direbbe di aver verificato un confronto che non ha
fatto.
"""

from __future__ import annotations

import copy
import unittest

from scripts import check_protocollo_v2 as gate

#: Un valore distinto per costante: vedi il docstring del modulo.
NOTE_SANE = {costante: 10 + i for i, costante in enumerate(gate.MAPPATURA.values())}
LIMITI_SANI = {chiave: NOTE_SANE[costante] for chiave, costante in gate.MAPPATURA.items()}
SONDE_SANE = {"una_sonda", "un_altra_sonda"}
REGISTRO_SANO = {"limite_di_lunghezza_byte": NOTE_SANE["MAX_BYTE_ID_CATEGORIA"]}

MANIFESTO_SANO = {
    "manifest_version": 1,
    "component": "plenora-IO-tools",
    "protocol_version": 2,
    "status": "in_qualifica",
    "compatibility_scope": "cli_json_only",
    "limiti_della_diagnostica": dict(LIMITI_SANI),
    "sonde_che_lo_provano": sorted(SONDE_SANE),
}


def esito(manifesto=None, note=None, esistenti=None, registro=None) -> list[str]:
    return gate.verifica(
        copy.deepcopy(MANIFESTO_SANO) if manifesto is None else manifesto,
        dict(NOTE_SANE) if note is None else note,
        set(SONDE_SANE) if esistenti is None else esistenti,
        dict(REGISTRO_SANO) if registro is None else registro,
    )


class IlRepositorySano(unittest.TestCase):
    def test_il_gate_passa_su_cio_che_c_e(self):
        self.assertEqual(gate.verifica(), [])

    def test_il_manifesto_finto_e_verde(self):
        # Se il caso sano fosse rosso, ogni sonda qui sotto sarebbe verde per
        # la ragione sbagliata e non proverebbe niente.
        self.assertEqual(esito(), [])

    def test_la_mappatura_copre_esattamente_le_costanti_del_budget(self):
        # Il verso inverso del gate vale solo se questi due file contengono le
        # costanti del budget e nient'altro. Se un giorno ne ospitassero una
        # estranea, il gate la chiamerebbe «non dichiarata» e avrebbe torto:
        # meglio accorgersene qui che in un rosso da interpretare.
        self.assertEqual(set(gate.costanti()), set(gate.MAPPATURA.values()))


class IDueNumeriDevonoCoincidere(unittest.TestCase):
    def test_un_limite_che_diverge_dalla_costante_e_rosso(self):
        for chiave, costante in gate.MAPPATURA.items():
            manifesto = copy.deepcopy(MANIFESTO_SANO)
            manifesto["limiti_della_diagnostica"][chiave] += 1
            errori = esito(manifesto)
            self.assertTrue(
                any(chiave in e and costante in e for e in errori),
                f"{chiave} divergente da {costante} non e' stato segnalato: {errori}",
            )

    def test_un_limite_assente_dal_manifesto_e_rosso(self):
        for chiave in gate.MAPPATURA:
            manifesto = copy.deepcopy(MANIFESTO_SANO)
            del manifesto["limiti_della_diagnostica"][chiave]
            self.assertTrue(any(chiave in e for e in esito(manifesto)))

    def test_una_costante_che_il_manifesto_non_dichiara_e_rossa(self):
        note = dict(NOTE_SANE)
        note["MAX_QUALCOSA_DI_NUOVO"] = 7
        errori = esito(note=note)
        self.assertTrue(any("MAX_QUALCOSA_DI_NUOVO" in e for e in errori), errori)

    def test_una_costante_sparita_col_limite_ancora_dichiarato_e_rossa(self):
        note = dict(NOTE_SANE)
        del note["MAX_CATEGORIE"]
        errori = esito(note=note)
        self.assertTrue(any("MAX_CATEGORIE" in e for e in errori), errori)

    def test_i_limiti_assenti_o_non_oggetto_sono_rossi(self):
        for valore in (None, [], 64, "molti"):
            manifesto = copy.deepcopy(MANIFESTO_SANO)
            manifesto["limiti_della_diagnostica"] = valore
            self.assertTrue(any("limiti_della_diagnostica" in e for e in esito(manifesto)))


class LIdentitaDelManifesto(unittest.TestCase):
    def test_un_campo_di_identita_sbagliato_e_rosso(self):
        for campo, atteso in gate.IDENTITA.items():
            manifesto = copy.deepcopy(MANIFESTO_SANO)
            manifesto[campo] = "sbagliato" if isinstance(atteso, str) else 99
            self.assertTrue(any(campo in e for e in esito(manifesto)))

    def test_un_manifesto_che_si_dichiara_v1_e_rosso(self):
        # E' il caso che conta: un v2 che si presenta come v1 verrebbe letto
        # come il contratto congelato, che promette un'altra cosa.
        manifesto = copy.deepcopy(MANIFESTO_SANO)
        manifesto["protocol_version"] = 1
        self.assertTrue(any("protocol_version" in e for e in esito(manifesto)))

    def test_dichiararsi_congelato_e_rosso(self):
        manifesto = copy.deepcopy(MANIFESTO_SANO)
        manifesto["status"] = "frozen_for_1_0"
        self.assertTrue(any("status" in e for e in esito(manifesto)))


class IlRegistroDelleCategorie(unittest.TestCase):
    def test_un_tetto_divergente_dalla_costante_e_rosso(self):
        registro = {"limite_di_lunghezza_byte": NOTE_SANE["MAX_BYTE_ID_CATEGORIA"] + 1}
        errori = esito(registro=registro)
        self.assertTrue(any("MAX_BYTE_ID_CATEGORIA" in e for e in errori), errori)

    def test_un_tetto_assente_e_rosso(self):
        self.assertTrue(any("registro" in e for e in esito(registro={})))

    def test_il_registro_reale_coincide_con_la_costante(self):
        # La catena che rende una sola l'autorita': il registro e' confrontato
        # qui con la costante Rust, e `check_categorie_di_perdita.py` confronta
        # con il registro la propria. Se questo legame si rompesse, quel gate
        # resterebbe verde mentre applica un tetto diverso dal codice.
        self.assertEqual(
            gate.registro_categorie()["limite_di_lunghezza_byte"],
            gate.costanti()["MAX_BYTE_ID_CATEGORIA"],
        )


class LeSondeNominateDalContratto(unittest.TestCase):
    def test_una_sonda_dichiarata_e_inesistente_e_rossa(self):
        manifesto = copy.deepcopy(MANIFESTO_SANO)
        manifesto["sonde_che_lo_provano"] = sorted(SONDE_SANE | {"una_sonda_mai_scritta"})
        errori = esito(manifesto)
        self.assertTrue(any("una_sonda_mai_scritta" in e for e in errori), errori)

    def test_una_sonda_esistente_e_non_nominata_e_rossa(self):
        errori = esito(esistenti=SONDE_SANE | {"una_sonda_non_nominata"})
        self.assertTrue(any("una_sonda_non_nominata" in e for e in errori), errori)

    def test_un_nome_ripetuto_e_rosso(self):
        manifesto = copy.deepcopy(MANIFESTO_SANO)
        manifesto["sonde_che_lo_provano"] = sorted(SONDE_SANE) + ["una_sonda"]
        self.assertTrue(any("due volte" in e for e in esito(manifesto)))

    def test_un_elenco_assente_o_malformato_e_rosso(self):
        for valore in (None, "una_sonda", ["una_sonda", 7], {}):
            manifesto = copy.deepcopy(MANIFESTO_SANO)
            manifesto["sonde_che_lo_provano"] = valore
            self.assertTrue(any("sonde_che_lo_provano" in e for e in esito(manifesto)))

    def test_gli_aiutanti_del_modulo_di_prova_non_sono_sonde(self):
        # `rapporto_con` costruisce il rapporto che le sonde usano e non prova
        # niente: se finisse fra le sonde, il contratto dovrebbe nominarlo.
        self.assertNotIn("rapporto_con", gate.sonde())
        self.assertIn("il_caso_peggiore_dichiarato_entra_nei_dodici_kib", gate.sonde())


class UnaCostanteDefinitaDueVolte(unittest.TestCase):
    """Il falso verde che la sovrascrittura silenziosa produceva.

    Non e' un caso teorico: prima di questa tranche `MAX_BYTE_DETTAGLIO` stava
    in `busta.rs` e la porta che lo applica sta in `loss.rs`. Se una copia
    sopravvivesse allo spostamento, il gate confronterebbe il manifesto con una
    sola delle due e sarebbe verde mentre il codice ne applica un'altra.
    """

    UNO = ("crates/uno.rs", "pub const MAX_CATEGORIE: usize = 64;\n")
    ALTRO = ("crates/altro.rs", "pub const MAX_CATEGORIE: usize = 99;\n")

    def test_due_file_che_la_dichiarano_sollevano(self):
        with self.assertRaises(ValueError) as contesto:
            gate.costanti_dai_testi([self.UNO, self.ALTRO])
        messaggio = str(contesto.exception)
        self.assertIn("MAX_CATEGORIE", messaggio)
        self.assertIn("crates/uno.rs", messaggio)
        self.assertIn("crates/altro.rs", messaggio)

    def test_vale_anche_dentro_lo_stesso_file(self):
        doppia = ("crates/uno.rs", self.UNO[1] + self.ALTRO[1])
        with self.assertRaises(ValueError):
            gate.costanti_dai_testi([doppia])

    def test_lo_stesso_valore_ripetuto_non_e_una_scusante(self):
        # Due definizioni concordi oggi sono due definizioni che domani
        # divergono: il gate rifiuta la doppiezza, non il disaccordo.
        gemella = ("crates/altro.rs", self.UNO[1])
        with self.assertRaises(ValueError):
            gate.costanti_dai_testi([self.UNO, gemella])

    def test_il_gate_lo_riporta_invece_di_esplodere(self):
        errori = gate.verifica(
            copy.deepcopy(MANIFESTO_SANO), None, set(SONDE_SANE), dict(REGISTRO_SANO)
        )
        self.assertIsInstance(errori, list)

    def test_i_sorgenti_reali_non_ne_hanno(self):
        gate.costanti()  # non solleva: nessuna costante del budget e' doppia


class IlValoreDiUnaCostante(unittest.TestCase):
    def test_risolve_i_prodotti_e_le_somme_di_nomi_noti(self):
        self.assertEqual(gate.valore("12 * 1024", {}), 12288)
        note = {"SEZIONI": 5, "BYTE_PER_SEZIONE": 12288, "BYTE_DELLA_STRUTTURA": 4096}
        self.assertEqual(
            gate.valore("SEZIONI * BYTE_PER_SEZIONE + BYTE_DELLA_STRUTTURA", note), 65536
        )

    def test_un_nome_non_ancora_noto_solleva(self):
        with self.assertRaises(ValueError):
            gate.valore("QUALCOSA + 1", {})

    def test_non_esegue_cio_che_trova(self):
        # La ragione per cui qui non c'e' `eval`: il gate legge un file che
        # qualcuno modifica, e un valutatore generico eseguirebbe qualunque
        # cosa vi trovasse. Rifiutare non e' prudenza, e' il requisito.
        for ostile in (
            '__import__("os").system("echo")',
            "open('/etc/passwd').read()",
            "1 << 64",
            "-1",
        ):
            with self.assertRaises((ValueError, SyntaxError), msg=ostile):
                gate.valore(ostile, {})


if __name__ == "__main__":
    unittest.main()
