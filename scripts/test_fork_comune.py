"""Sonde del calcolo canonico dell'impronta di un fork.

La proprieta' decisiva e' **negativa**: non deve essere possibile produrre un
lock diverso lasciando un artefatto di build dentro l'albero vendorizzato.

La prima stesura hashava tutto cio' che stava sul disco. Una `cargo package` di
verifica lascia `vendor/<crate>/target/`, e un lock ricalcolato in quello stato
avrebbe registrato un artefatto come contenuto del fork governato — cioe' la
provenienza di un pacchetto ridistribuito sarebbe stata falsata da un residuo.
"""

from __future__ import annotations

import shutil
import subprocess
import unittest
from pathlib import Path

from scripts import fork_comune as calcolo

VENDOR = calcolo.ROOT / "vendor" / "dxf"


class SondeImpronta(unittest.TestCase):
    def test_l_insieme_versionato_e_quello_di_git(self) -> None:
        atteso = subprocess.run(
            ["git", "ls-files", "-z", "--", "vendor/dxf"],
            cwd=calcolo.ROOT,
            capture_output=True,
            check=True,
        )
        quanti = len([n for n in atteso.stdout.decode("utf-8").split("\0") if n])
        self.assertEqual(len(calcolo.insieme_versionato(VENDOR)), quanti)

    def test_l_impronta_e_stabile(self) -> None:
        self.assertEqual(calcolo.impronta(VENDOR), calcolo.impronta(VENDOR))

    def test_un_albero_pulito_non_ha_estranei(self) -> None:
        """La controprova positiva: senza, «sempre estranei» sarebbe una difesa."""
        self.assertEqual(calcolo.artefatti_estranei(VENDOR), [])


class SondeArtefatto(unittest.TestCase):
    """`vendor/dxf/target/` presente: l'impronta non cambia, l'estraneo si vede."""

    ARTEFATTO = VENDOR / "target" / "package" / "residuo.crate"

    def setUp(self) -> None:
        self.prima = calcolo.impronta(VENDOR)
        self.ARTEFATTO.parent.mkdir(parents=True, exist_ok=True)
        self.ARTEFATTO.write_bytes(b"artefatto di una cargo package di verifica")
        self.addCleanup(shutil.rmtree, VENDOR / "target", ignore_errors=True)

    def test_l_impronta_non_cambia(self) -> None:
        """**La sonda decisiva.**

        Se questa fallisse, sarebbe possibile produrre un lock diverso — e
        quindi una provenienza diversa per un pacchetto ridistribuito —
        semplicemente dimenticando un `--target-dir`.
        """
        self.assertEqual(
            calcolo.impronta(VENDOR),
            self.prima,
            "un artefatto di build ha alterato l'impronta del fork governato",
        )

    def test_il_conteggio_non_cambia(self) -> None:
        self.assertEqual(calcolo.impronta(VENDOR)[0], self.prima[0])

    def test_l_artefatto_e_segnalato(self) -> None:
        """L'altra meta': non alterare l'impronta non vuol dire ignorare."""
        estranei = calcolo.artefatti_estranei(VENDOR)
        self.assertIn("target/package/residuo.crate", estranei)

    def test_il_gate_diventa_rosso(self) -> None:
        esito = subprocess.run(
            ["python3", str(calcolo.ROOT / "scripts" / "check_dxf_fork.py")],
            cwd=calcolo.ROOT,
            capture_output=True,
            text=True,
            check=False,
        )
        self.assertNotEqual(esito.returncode, 0, "il gate ha ignorato un estraneo")
        self.assertIn("artefatti estranei", esito.stderr + esito.stdout)


class SondeComandoPackage(unittest.TestCase):
    def test_il_target_e_fuori_dall_albero_vendorizzato(self) -> None:
        """Non e' un consiglio: senza, l'operazione di verifica sporca cio'
        che sta verificando."""
        comando = calcolo.comando_package(VENDOR)
        self.assertIn("--target-dir", comando)
        bersaglio = Path(comando[comando.index("--target-dir") + 1])
        self.assertFalse(
            str(bersaglio).startswith(str(VENDOR)),
            f"il target {bersaglio} e' dentro l'albero vendorizzato",
        )

    def test_il_target_distingue_i_due_fork(self) -> None:
        dxf = calcolo.comando_package(calcolo.ROOT / "vendor" / "dxf")
        gdal = calcolo.comando_package(calcolo.ROOT / "vendor" / "gdal")
        self.assertNotEqual(
            dxf[dxf.index("--target-dir") + 1],
            gdal[gdal.index("--target-dir") + 1],
        )


if __name__ == "__main__":
    unittest.main()
