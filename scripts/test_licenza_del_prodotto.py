"""La licenza del prodotto, detta allo stesso modo in ogni posto che la dice.

Fino alla 4.1.0 `distribuzione.LICENZA_FIRST_PARTY` diceva «non dichiarata,
fuori dal perimetro», e il referto `licenze-artefatto` lo ripeteva accanto a
una wheel con `License: Proprietary`. Queste sonde legano le dichiarazioni fra
loro e al testo che deve viaggiare con gli artefatti.
"""

from __future__ import annotations

import importlib.util
import io
import pathlib
import shutil
import sys
import tarfile
import tempfile
import tomllib
import unittest
import zipfile

RADICE = pathlib.Path(__file__).resolve().parent.parent
sys.path.insert(0, str(RADICE / "scripts"))

import distribuzione  # noqa: E402 -- dopo sys.path, che e' il punto


def carica(percorso: pathlib.Path):
    spec = importlib.util.spec_from_file_location(percorso.stem.replace("-", "_"), percorso)
    modulo = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(modulo)
    return modulo


class LaDichiarazione(unittest.TestCase):
    def test_e_dichiarata_e_proprietaria(self) -> None:
        licenza = distribuzione.licenza_first_party()
        self.assertIs(licenza["dichiarata"], True)
        self.assertEqual(licenza["stato"], "proprietaria")
        self.assertEqual(licenza["identificatore_spdx"], "LicenseRef-Plenora-Proprietary")

    def test_coincide_con_il_testo_e_con_i_metadati(self) -> None:
        testo = (RADICE / "LICENSE").read_text(encoding="utf-8")
        self.assertIn("Proprietary", testo)
        self.assertIn(distribuzione.licenza_first_party()["titolare"], testo)
        pyproject = tomllib.loads((RADICE / "sdk" / "python" / "pyproject.toml").read_text(encoding="utf-8"))
        self.assertEqual(pyproject["project"]["license"], {"text": "Proprietary"})
        self.assertEqual(
            distribuzione.licenza_first_party()["metadati_python"],
            "License: Proprietary",
        )

    def test_ogni_crate_porta_il_file_di_licenza(self) -> None:
        workspace = tomllib.loads((RADICE / "Cargo.toml").read_text(encoding="utf-8"))
        self.assertEqual(workspace["workspace"]["package"]["license-file"], "LICENSE")
        for manifesto in sorted((RADICE / "crates").glob("*/Cargo.toml")):
            dati = tomllib.loads(manifesto.read_text(encoding="utf-8"))
            with self.subTest(crate=manifesto.parent.name):
                self.assertEqual(dati["package"].get("license-file"), {"workspace": True})


class IlTestoNeiPacchettiPython(unittest.TestCase):
    def setUp(self) -> None:
        self.tmp = pathlib.Path(tempfile.mkdtemp())
        self.addCleanup(shutil.rmtree, self.tmp, ignore_errors=True)
        self.costruttore = carica(RADICE / "scripts" / "costruisci-pacchetto-python.py")
        self.testo = (RADICE / "LICENSE").read_bytes()

    def test_la_wheel_porta_la_licenza_in_dist_info(self) -> None:
        wheel = self.costruttore.costruisci_wheel(self.tmp, "0.0.0")
        with zipfile.ZipFile(wheel) as archivio:
            nomi = [n for n in archivio.namelist() if n.endswith(".dist-info/licenses/LICENSE")]
            self.assertEqual(len(nomi), 1, archivio.namelist())
            self.assertEqual(archivio.read(nomi[0]), self.testo)
            record = archivio.read(next(n for n in archivio.namelist() if n.endswith("RECORD")))
            self.assertIn(nomi[0].encode("utf-8"), record)

    def test_la_sdist_porta_la_licenza_alla_radice(self) -> None:
        sdist = self.costruttore.costruisci_sdist(self.tmp, "0.0.0")
        with tarfile.open(sdist) as archivio:
            voce = next(m for m in archivio.getmembers() if m.name.endswith("/LICENSE") and m.name.count("/") == 1)
            self.assertEqual(archivio.extractfile(voce).read(), self.testo)

    def test_il_referto_delle_licenze_dichiara_la_licenza(self) -> None:
        referto = self.costruttore.licenze("0.0.0")
        self.assertIs(referto["licenza_first_party"]["dichiarata"], True)


if __name__ == "__main__":
    unittest.main()
