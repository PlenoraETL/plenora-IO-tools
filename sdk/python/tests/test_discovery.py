"""La scoperta del binario, e il manifesto che lo accompagna.

Le sonde costruiscono alberi finti in una directory temporanea invece di
dipendere da un artefatto installato: la scoperta e' logica, e legarla a un
ambiente la renderebbe verde o rossa per ragioni che non la riguardano.
"""

from __future__ import annotations

import json
import os
import stat
import unittest
from pathlib import Path
from tempfile import TemporaryDirectory
from unittest import mock

from plenora_io import BinaryNotFound, LocalIoError, ManifestError, ProfileError
from plenora_io._discovery import (
    MANIFESTO,
    NOME,
    VARIABILE,
    _cerca_nel_path,
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


def _is_file_python314(percorso):
    """Riproduce anche sui Python precedenti gli OSError soppressi dal 3.14."""
    try:
        return stat.S_ISREG(percorso.stat().st_mode)
    except OSError:
        return False


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
    def test_path_stat_distingue_assenza_da_guasti(self) -> None:
        os.environ["PATH"] = str(self.tmp)
        os.environ["PATHEXT"] = ".EXE"
        for tipo in (FileNotFoundError, NotADirectoryError):
            with self.subTest(tipo=tipo), mock.patch.object(Path, "stat", side_effect=tipo()):
                self.assertIsNone(_cerca_nel_path())
        for tipo in (PermissionError, OSError, IsADirectoryError):
            with self.subTest(tipo=tipo), mock.patch.object(
                Path, "stat", side_effect=tipo(str(self.tmp))
            ):
                with self.assertRaises(LocalIoError) as preso:
                    _cerca_nel_path()
                self.assertNotIn(str(self.tmp), str(preso.exception))

    def test_path_rispetta_ordine_tipo_ed_eseguibilita(self) -> None:
        prima = self.tmp / "prima"
        seconda = self.tmp / "seconda"
        prima.mkdir()
        seconda.mkdir()
        os.environ["PATH"] = os.pathsep.join((str(prima), str(seconda)))
        os.environ["PATHEXT"] = ".EXE;.CMD"
        nome = NOME + (".EXE" if os.name == "nt" else "")
        candidato = prima / nome
        altro = seconda / nome
        altro.write_text("binario", encoding="utf-8")
        altro.chmod(0o755)
        candidato.mkdir()
        self.assertEqual(_cerca_nel_path(), altro)
        candidato.rmdir()
        candidato.write_text("binario", encoding="utf-8")
        if os.name != "nt":
            candidato.chmod(0o644)
            self.assertEqual(_cerca_nel_path(), altro)
        else:
            (prima / (NOME + ".CMD")).write_text("binario", encoding="utf-8")
        candidato.chmod(0o755)
        self.assertEqual(_cerca_nel_path(), candidato)

    def test_path_negato_non_passa_alla_seconda_directory(self) -> None:
        prima = self.tmp / "riservato"
        seconda = self.tmp / "seconda"
        prima.mkdir()
        seconda.mkdir()
        binario = seconda / (NOME + (".EXE" if os.name == "nt" else ""))
        binario.write_text("#!/bin/sh\nexit 0\n", encoding="utf-8")
        binario.chmod(0o755)
        os.environ["PATH"] = os.pathsep.join((str(prima), str(seconda)))
        os.environ["PATHEXT"] = ".EXE"
        stat_originale = Path.stat
        visitati = []

        def stat_negato(percorso, *args, **kwargs):
            percorso = Path(percorso)
            visitati.append(percorso)
            if percorso.parent == prima:
                raise PermissionError(13, "negato", str(percorso))
            return stat_originale(percorso, *args, **kwargs)

        with mock.patch("plenora_io._discovery._albero_accanto_al_pacchetto", return_value=None):
            self.assertEqual(trova_binario(), binario.resolve())
            # Python 3.10 conserva in pathlib un riferimento a os.stat:
            # sostituire os.stat non intercetta le chiamate di Path.stat.
            with mock.patch.object(Path, "stat", stat_negato):
                with self.assertRaises(LocalIoError) as preso:
                    trova_binario()
        self.assertNotIn(binario, visitati)
        self.assertNotIn(str(self.tmp), str(preso.exception))
        self.assertTrue(preso.exception.__suppress_context__)

    def test_stat_negato_non_passa_al_binario_successivo(self) -> None:
        albero = AlberoFinto(self.tmp / "riservato")
        altro = AlberoFinto(self.tmp / "altro")
        os.environ[VARIABILE] = str(altro.binario)
        stat_originale = Path.stat
        visitati = []

        def stat(percorso, *args, **kwargs):
            visitati.append(percorso)
            if percorso == albero.binario:
                raise PermissionError(13, "negato", str(percorso))
            return stat_originale(percorso, *args, **kwargs)

        with mock.patch.object(Path, "stat", stat), mock.patch.object(
            Path, "is_file", _is_file_python314
        ):
            with self.assertRaises(LocalIoError) as preso:
                trova_binario(albero.binario)
        self.assertNotIn(altro.binario, visitati)
        self.assertNotIn(str(self.tmp), str(preso.exception))
        self.assertTrue(preso.exception.__suppress_context__)

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
        # I posti, non i percorsi: il percorso indicato e il `PATH` sono della
        # macchina di chi chiama (erano nel messaggio fino alla 4.1.1).
        self.assertNotIn("niente", messaggio)
        self.assertNotIn(str(self.tmp), messaggio)
        self.assertIn("Client(binary=...)", messaggio)
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
    def test_stat_distingue_assenza_da_altri_guasti(self) -> None:
        albero = AlberoFinto(self.tmp / "albero")
        for tipo in (FileNotFoundError, NotADirectoryError):
            with self.subTest(tipo=tipo), mock.patch.object(
                Path, "stat", side_effect=tipo(str(self.tmp))
            ):
                self.assertIsNone(leggi_manifesto(albero.binario))
        for tipo in (OSError, IsADirectoryError):
            with self.subTest(tipo=tipo), mock.patch.object(
                Path, "stat", side_effect=tipo(str(self.tmp))
            ):
                with self.assertRaises(LocalIoError) as preso:
                    leggi_manifesto(albero.binario)
                self.assertNotIn(str(self.tmp), str(preso.exception))

    def test_stat_negato_non_rende_il_manifesto_assente(self) -> None:
        albero = AlberoFinto(self.tmp / "riservato")
        percorso = albero.radice / MANIFESTO
        with mock.patch.object(
            Path, "stat", side_effect=PermissionError(13, "negato", str(percorso))
        ) as stat, mock.patch.object(Path, "is_file", _is_file_python314):
            with self.assertRaises(LocalIoError) as preso:
                leggi_manifesto(albero.binario)
        stat.assert_called_once()
        self.assertNotIn(str(self.tmp), str(preso.exception))
        self.assertTrue(preso.exception.__suppress_context__)

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
