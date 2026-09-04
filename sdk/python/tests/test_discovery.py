"""La scoperta del binario, e il manifesto che lo accompagna.

Le sonde costruiscono alberi finti in una directory temporanea invece di
dipendere da un artefatto installato: la scoperta e' logica, e legarla a un
ambiente la renderebbe verde o rossa per ragioni che non la riguardano.
"""

from __future__ import annotations

import json
import os
import unittest
from pathlib import Path
from tempfile import TemporaryDirectory

from plenora_io import BinaryNotFound, ManifestError, ProfileError
from plenora_io.discovery import (
    MANIFESTO,
    NOME,
    VARIABILE,
    leggi_manifesto,
    trova_binario,
    verifica_profilo,
)

MANIFESTO_SANO = {
    "nome": "plenora-io",
    "versione": "2.0.0",
    "piattaforma": "linux-x86_64",
    "profilo": "base",
    "canale": "prova",
    "non_release": True,
    "revisione": "a" * 40,
}


class AlberoFinto:
    """Un albero distribuito: `bin/plenora-io` e `MANIFEST.json` accanto."""

    def __init__(self, radice: Path, manifesto: dict | str | None = MANIFESTO_SANO):
        self.radice = radice
        (radice / "bin").mkdir(parents=True, exist_ok=True)
        self.binario = radice / "bin" / NOME
        self.binario.write_text("#!/bin/sh\nexit 0\n", encoding="utf-8")
        self.binario.chmod(0o755)
        if manifesto is not None:
            testo = (
                manifesto
                if isinstance(manifesto, str)
                else json.dumps(manifesto, ensure_ascii=False)
            )
            (radice / MANIFESTO).write_text(testo, encoding="utf-8")


class SenzaAmbiente(unittest.TestCase):
    """`PLENORA_IO_BIN` e `PATH` fuori dai piedi.

    Senza, una sonda passerebbe o cadrebbe a seconda di che cosa ha la macchina
    che la esegue, ed e' esattamente il difetto che rende inutile una sonda
    sull'ambiente.
    """

    def setUp(self) -> None:
        self._ambiente = dict(os.environ)
        os.environ.pop(VARIABILE, None)
        os.environ["PATH"] = ""
        self._temporanea = TemporaryDirectory(prefix="plenora-sdk-")
        self.tmp = Path(self._temporanea.name)

    def tearDown(self) -> None:
        os.environ.clear()
        os.environ.update(self._ambiente)
        self._temporanea.cleanup()


class LaScoperta(SenzaAmbiente):
    def test_il_percorso_esplicito_vince(self) -> None:
        albero = AlberoFinto(self.tmp / "albero")
        altro = AlberoFinto(self.tmp / "altro")
        os.environ[VARIABILE] = str(altro.binario)
        self.assertEqual(trova_binario(albero.binario), albero.binario.resolve())

    def test_l_ambiente_viene_dopo_l_esplicito_e_prima_del_path(self) -> None:
        albero = AlberoFinto(self.tmp / "albero")
        os.environ[VARIABILE] = str(albero.binario)
        self.assertEqual(trova_binario(), albero.binario.resolve())

    def test_un_esplicito_inesistente_non_zittisce_l_ambiente(self) -> None:
        """L'ordine e' una preferenza, non un'esclusione.

        Un percorso indicato e assente non deve interrompere la ricerca: chi lo
        ha scritto ha detto «prova prima qui», non «solo qui». Ma deve comparire
        fra i posti guardati, se poi non si trova niente.
        """
        albero = AlberoFinto(self.tmp / "albero")
        os.environ[VARIABILE] = str(albero.binario)
        self.assertEqual(
            trova_binario(self.tmp / "non-esiste"), albero.binario.resolve()
        )

    def test_senza_niente_dice_dove_ha_cercato(self) -> None:
        """Il valore dell'errore non e' che ci sia: e' che sia leggibile.

        «non trovato» lascerebbe indovinare se la variabile sia stata letta, se
        il PATH sia quello giusto, se il nome sia quello atteso.
        """
        with self.assertRaises(BinaryNotFound) as preso:
            trova_binario(self.tmp / "niente")
        messaggio = str(preso.exception)
        self.assertIn("niente", messaggio)
        self.assertIn(VARIABILE, messaggio)
        self.assertIn("PATH", messaggio)
        self.assertIn("non lo scarica", messaggio)
        self.assertEqual(len(preso.exception.searched), 4)

    def test_una_directory_bin_qualunque_non_e_un_albero(self) -> None:
        """I due indizi, e perche' ce ne vogliono due.

        Una `bin/plenora-io` senza manifesto accanto non e' un artefatto
        distribuito, e prenderla per tale farebbe eseguire un binario altrui --
        il caso vero e' un ambiente virtuale che ne contiene uno di un'altra
        installazione.
        """
        AlberoFinto(self.tmp / "senza", manifesto=None)
        with self.assertRaises(BinaryNotFound):
            trova_binario()


class IlManifesto(SenzaAmbiente):
    def test_assente_non_e_un_errore(self) -> None:
        """Un binario costruito da cargo non ha un manifesto ed e' usabile."""
        albero = AlberoFinto(self.tmp / "cargo", manifesto=None)
        self.assertIsNone(leggi_manifesto(albero.binario))

    def test_letto_intero(self) -> None:
        albero = AlberoFinto(self.tmp / "albero")
        manifesto = leggi_manifesto(albero.binario)
        assert manifesto is not None
        self.assertEqual(manifesto.profile, "base")
        self.assertEqual(manifesto.version, "2.0.0")
        self.assertEqual(manifesto.revision, "a" * 40)
        # `non_release: true` diventa `release: false`: il verso negativo del
        # wire non arriva a chi legge.
        self.assertFalse(manifesto.release)
        self.assertEqual(manifesto.raw["canale"], "prova")

    def test_rotto_non_e_assente(self) -> None:
        """Trattare un manifesto illeggibile come mancante nasconderebbe il
        guasto: l'artefatto e' rotto, e va detto."""
        albero = AlberoFinto(self.tmp / "rotto", manifesto="{non json")
        with self.assertRaises(ManifestError) as preso:
            leggi_manifesto(albero.binario)
        self.assertIn("non e' un manifesto assente", str(preso.exception))

    def test_incompleto_e_un_errore(self) -> None:
        parziale = {k: v for k, v in MANIFESTO_SANO.items() if k != "profilo"}
        albero = AlberoFinto(self.tmp / "parziale", manifesto=parziale)
        with self.assertRaises(ManifestError) as preso:
            leggi_manifesto(albero.binario)
        self.assertIn("profilo", str(preso.exception))

    def test_una_lista_al_posto_di_un_oggetto(self) -> None:
        albero = AlberoFinto(self.tmp / "lista", manifesto="[]")
        with self.assertRaises(ManifestError):
            leggi_manifesto(albero.binario)


class IlProfilo(SenzaAmbiente):
    def test_quello_giusto_passa(self) -> None:
        albero = AlberoFinto(self.tmp / "albero")
        verifica_profilo(leggi_manifesto(albero.binario), "base")

    def test_quello_sbagliato_solleva_prima_di_eseguire(self) -> None:
        albero = AlberoFinto(self.tmp / "albero")
        with self.assertRaises(ProfileError) as preso:
            verifica_profilo(leggi_manifesto(albero.binario), "filegdb")
        self.assertEqual(preso.exception.required, "filegdb")
        self.assertEqual(preso.exception.actual, "base")

    def test_senza_manifesto_la_risposta_e_no(self) -> None:
        """Non «forse».

        Un binario di cui non si sa il profilo non si puo' dichiarare adatto:
        dirlo adatto per non bloccare chi sta provando trasformerebbe la
        verifica in un augurio, e il fallimento tornerebbe piu' avanti con un
        altro nome.
        """
        albero = AlberoFinto(self.tmp / "cargo", manifesto=None)
        with self.assertRaises(ProfileError) as preso:
            verifica_profilo(leggi_manifesto(albero.binario), "base")
        self.assertIsNone(preso.exception.actual)
        self.assertIn("nessun manifesto", str(preso.exception))

    def test_un_profilo_fuori_vocabolario_e_rifiutato(self) -> None:
        albero = AlberoFinto(self.tmp / "albero")
        with self.assertRaises(ProfileError):
            verifica_profilo(leggi_manifesto(albero.binario), "inventato")


if __name__ == "__main__":
    unittest.main()
