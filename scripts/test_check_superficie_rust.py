"""Sonde della verifica del grafo del consumatore esterno.

La compilazione dall'archivio costa minuti e gira nel job `superficie-rust`;
qui si prova, senza cargo, la parte che decide: che cosa il gate accetta come
grafo risolto. La sonda decisiva e' **negativa** -- un grafo con il crate
upstream accanto al fork, o con il fork da un registro, deve essere rosso --,
perche' una verifica che non sa dire di no non verifica niente.
"""

from __future__ import annotations

import importlib.util
import pathlib
import tempfile
import unittest

_SPEC = importlib.util.spec_from_file_location(
    "check_superficie_rust",
    pathlib.Path(__file__).resolve().parent / "check_superficie_rust.py",
)
assert _SPEC is not None and _SPEC.loader is not None
gate = importlib.util.module_from_spec(_SPEC)
_SPEC.loader.exec_module(gate)

REGISTRO = "registry+https://github.com/rust-lang/crates.io-index"


class SondeGrafo(unittest.TestCase):
    def setUp(self) -> None:
        self.radice = pathlib.Path(tempfile.mkdtemp(prefix="archivio-"))
        for nome in ("gdal", "shapefile", "dxf"):
            (self.radice / "vendor" / nome).mkdir(parents=True)

    def _fork(self, fork: str, upstream: str) -> dict:
        return {
            "name": fork,
            "source": None,
            "manifest_path": str(self.radice / "vendor" / upstream / "Cargo.toml"),
        }

    def _grafo(self) -> list[dict]:
        return [self._fork(fork, upstream) for fork, upstream in gate.FORK.items()]

    def test_i_tre_fork_dall_archivio_sono_verdi(self) -> None:
        """La controprova: senza, «sempre rosso» passerebbe per una difesa."""
        self.assertEqual(gate.fork_nei_pacchetti(self._grafo(), self.radice), [])

    def test_il_crate_upstream_accanto_al_fork_e_rosso(self) -> None:
        """Il caso che la correzione chiude: qualcuno chiede `dxf` per nome."""
        grafo = self._grafo() + [
            {"name": "dxf", "source": REGISTRO, "manifest_path": "/registro/dxf/Cargo.toml"}
        ]
        problemi = gate.fork_nei_pacchetti(grafo, self.radice)
        self.assertTrue(any("«dxf» upstream" in p for p in problemi), problemi)

    def test_un_fork_mancante_e_rosso(self) -> None:
        grafo = [p for p in self._grafo() if p["name"] != "plenora-fork-gdal"]
        problemi = gate.fork_nei_pacchetti(grafo, self.radice)
        self.assertTrue(any("plenora-fork-gdal" in p for p in problemi), problemi)

    def test_un_fork_da_un_registro_e_rosso(self) -> None:
        grafo = self._grafo()
        grafo[0] = dict(grafo[0], source=REGISTRO)
        problemi = gate.fork_nei_pacchetti(grafo, self.radice)
        self.assertTrue(any("non viene dal `vendor/`" in p for p in problemi), problemi)

    def test_un_fork_fuori_dall_archivio_e_rosso(self) -> None:
        """Un percorso giusto nel nome e sbagliato nella radice: un altro albero."""
        altrove = pathlib.Path(tempfile.mkdtemp(prefix="altrove-"))
        grafo = self._grafo()
        grafo[1] = dict(grafo[1], manifest_path=str(altrove / "vendor" / "x" / "Cargo.toml"))
        problemi = gate.fork_nei_pacchetti(grafo, self.radice)
        self.assertTrue(any("non viene dal `vendor/`" in p for p in problemi), problemi)

    def test_le_coppie_sono_quelle_dei_lock(self) -> None:
        """Due elenchi scritti a mano divergono: questo e i lock dei fork."""
        import json

        radice = pathlib.Path(__file__).resolve().parents[1]
        dai_lock = {}
        for nome in ("gdal", "shapefile", "dxf"):
            lock = json.loads(
                (radice / "scripts" / f"{nome}-fork-lock.json").read_text(encoding="utf-8")
            )
            dai_lock[lock["fork_package"]] = lock["package"]
        self.assertEqual(gate.FORK, dai_lock)


class SondeConsumatore(unittest.TestCase):
    def test_il_consumatore_non_dichiara_patch(self) -> None:
        """La prova vale solo se il consumatore parte senza `[patch]`."""
        import tomllib

        radice = pathlib.Path(__file__).resolve().parents[1]
        manifesto = tomllib.loads(
            (radice / "conformance" / "consumatore-rust" / "Cargo.toml").read_text(
                encoding="utf-8"
            )
        )
        self.assertNotIn("patch", manifesto)
        self.assertEqual(
            set(manifesto["dependencies"]),
            {"plenora-io-tools"},
            "il consumatore deve dichiarare solo la superficie, non i fork",
        )


if __name__ == "__main__":
    unittest.main()
