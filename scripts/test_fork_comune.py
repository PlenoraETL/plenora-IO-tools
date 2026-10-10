"""Sonde del calcolo canonico dell'impronta di un fork.

La proprieta' decisiva e' **negativa**: non deve essere possibile produrre un
lock diverso lasciando un artefatto di build dentro l'albero vendorizzato.

La prima stesura hashava tutto cio' che stava sul disco. Una `cargo package` di
verifica lascia `vendor/<crate>/target/`, e un lock ricalcolato in quello stato
avrebbe registrato un artefatto come contenuto del fork governato — cioe' la
provenienza di un pacchetto ridistribuito sarebbe stata falsata da un residuo.
"""

from __future__ import annotations

import json
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


class SondeFiniRiga(unittest.TestCase):
    """L'impronta legge il disco, e il disco puo' non essere cio' che git tiene.

    `.gitattributes` normalizza i fine riga dei sorgenti a LF. Un editor che
    riscrive un file intero con CRLF non cambia cio' che verra' committato, ma
    cambia l'impronta calcolata qui: il lock finisce per registrare un digest
    riproducibile solo sulla macchina che l'ha scritto, e in CI il gate e' rosso
    dicendo «albero diverso dal lock» -- che e' vero e non e' la ragione.
    """

    def test_i_tre_fork_sono_gia_normalizzati(self) -> None:
        """La controprova positiva: senza, «nessun divergente» sarebbe una
        difesa che non ha mai visto niente."""
        for nome in ("dxf", "gdal", "shapefile", "parquet"):
            with self.subTest(fork=nome):
                self.assertEqual(
                    calcolo.fini_riga_divergenti(calcolo.ROOT / "vendor" / nome), []
                )

    def test_un_file_riscritto_con_crlf_e_nominato(self) -> None:
        bersaglio = next(
            percorso
            for percorso in calcolo.insieme_versionato(VENDOR)
            if percorso.suffix == ".rs"
        )
        originale = bersaglio.read_bytes()
        self.assertNotIn(b"\r\n", originale, "il file di partenza dev'essere LF")
        try:
            bersaglio.write_bytes(originale.replace(b"\n", b"\r\n"))
            divergenti = calcolo.fini_riga_divergenti(VENDOR)
            self.assertIn(bersaglio.relative_to(VENDOR).as_posix(), divergenti)
        finally:
            bersaglio.write_bytes(originale)
        self.assertEqual(calcolo.fini_riga_divergenti(VENDOR), [])

    def test_il_gate_nomina_la_ragione_invece_dell_impronta(self) -> None:
        """Il valore della difesa non e' che diventi rossa: e' **che cosa dice**.

        Senza, il rosso e' quello dell'impronta, e manda a cercare una modifica
        del contenuto che non c'e' stata.
        """
        bersaglio = next(
            percorso
            for percorso in calcolo.insieme_versionato(VENDOR)
            if percorso.suffix == ".rs"
        )
        originale = bersaglio.read_bytes()
        try:
            bersaglio.write_bytes(originale.replace(b"\n", b"\r\n"))
            esito = subprocess.run(
                ["python3", str(calcolo.ROOT / "scripts" / "check_dxf_fork.py")],
                cwd=calcolo.ROOT,
                capture_output=True,
                text=True,
                check=False,
            )
        finally:
            bersaglio.write_bytes(originale)
        self.assertNotEqual(esito.returncode, 0)
        detto = esito.stdout + esito.stderr
        self.assertIn("git registrerebbe", detto)
        self.assertNotIn("diverso dal lock", detto)


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



class SondeRisoluzione(unittest.TestCase):
    """I fork entrano come dipendenze dirette con nome proprio, mai come `[patch]`.

    Le sonde lavorano su una **copia** dei soli file che la verifica legge: i
    due manifesti di workspace, i due lockfile e i manifesti dei fork. L'albero
    vero non si tocca.
    """

    FILE = (
        "Cargo.toml",
        "Cargo.lock",
        "fuzz/Cargo.toml",
        "fuzz/Cargo.lock",
        "vendor/dxf/Cargo.toml",
        "vendor/gdal/Cargo.toml",
        "vendor/shapefile/Cargo.toml",
        "vendor/parquet/Cargo.toml",
    )

    def setUp(self) -> None:
        import tempfile

        self.copia = Path(tempfile.mkdtemp(prefix="fork-risoluzione-"))
        self.addCleanup(shutil.rmtree, self.copia, ignore_errors=True)
        for relativo in self.FILE:
            destinazione = self.copia / relativo
            destinazione.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(calcolo.ROOT / relativo, destinazione)
        self.lock = {
            nome: json.loads(
                (calcolo.ROOT / "scripts" / f"{nome}-fork-lock.json").read_text(
                    encoding="utf-8"
                )
            )
            for nome in ("dxf", "gdal", "shapefile", "parquet")
        }

    def _aggiungi(self, relativo: str, testo: str) -> None:
        percorso = self.copia / relativo
        percorso.write_text(
            percorso.read_text(encoding="utf-8") + testo, encoding="utf-8"
        )

    def test_l_albero_vero_e_verde(self) -> None:
        """La controprova: senza, «sempre rosso» passerebbe per una difesa."""
        for nome, lock in self.lock.items():
            with self.subTest(fork=nome):
                self.assertEqual(calcolo.problemi_di_risoluzione(lock, self.copia), [])

    def test_una_patch_nel_workspace_e_rossa(self) -> None:
        self._aggiungi("Cargo.toml", '\n[patch.crates-io]\ndxf = { path = "vendor/dxf" }\n')
        self.assertEqual(
            calcolo.patch_presenti(self.copia), ["Cargo.toml: [patch.crates-io]"]
        )
        self.assertTrue(calcolo.problemi_di_risoluzione(self.lock["gdal"], self.copia))

    def test_una_patch_nel_fuzz_e_rossa(self) -> None:
        """Il workspace staccato conta quanto il principale."""
        self._aggiungi("fuzz/Cargo.toml", '\n[patch."https://example.invalid/x"]\nx = { path = "x" }\n')
        self.assertEqual(
            calcolo.patch_presenti(self.copia),
            ['fuzz/Cargo.toml: [patch.https://example.invalid/x]'],
        )

    def test_il_nome_upstream_nel_lockfile_e_rosso(self) -> None:
        """Il caso che la correzione chiude: qualcuno chiede ancora `dxf` per nome."""
        self._aggiungi(
            "Cargo.lock",
            '\n[[package]]\nname = "dxf"\nversion = "0.6.1"\n'
            'source = "registry+https://github.com/rust-lang/crates.io-index"\n',
        )
        problemi = calcolo.problemi_di_risoluzione(self.lock["dxf"], self.copia)
        self.assertTrue(any("upstream «dxf»" in p for p in problemi), problemi)

    def test_un_nome_non_proprio_e_rosso(self) -> None:
        lock = dict(self.lock["gdal"], fork_package="gdal")
        problemi = calcolo.problemi_di_risoluzione(lock, self.copia)
        self.assertTrue(any("nome proprio" in p for p in problemi), problemi)

    def test_la_dipendenza_per_versione_e_rossa(self) -> None:
        """Tornare a `shapefile = "=0.9.0"` rimette il crate di crates.io."""
        percorso = self.copia / "Cargo.toml"
        testo = percorso.read_text(encoding="utf-8")
        riga = next(r for r in testo.splitlines() if r.startswith("shapefile = "))
        percorso.write_text(
            testo.replace(riga, 'shapefile = { version = "=0.9.0", features = ["geo-types"] }'),
            encoding="utf-8",
        )
        problemi = calcolo.problemi_di_risoluzione(self.lock["shapefile"], self.copia)
        self.assertTrue(any("dipendenza diretta" in p for p in problemi), problemi)

    def test_il_nome_della_libreria_deve_restare_upstream(self) -> None:
        percorso = self.copia / "vendor" / "dxf" / "Cargo.toml"
        percorso.write_text(
            percorso.read_text(encoding="utf-8").replace('[lib]\nname = "dxf"', '[lib]\nname = "altro"'),
            encoding="utf-8",
        )
        problemi = calcolo.problemi_di_risoluzione(self.lock["dxf"], self.copia)
        self.assertTrue(any("[lib] name" in p for p in problemi), problemi)


if __name__ == "__main__":
    unittest.main()
