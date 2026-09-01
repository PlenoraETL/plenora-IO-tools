"""Sonde sull'SBOM e sui digest del manifesto.

# I tre falsi verdi che chiudono

**Il primo.** L'SBOM elencava i soli componenti nativi: nel profilo base
significava **un pacchetto solo** -- il runtime C -- per un binario che porta
dentro duecento crate linkati staticamente. Il gate confrontava l'SBOM con la
provenienza, che era incompleta allo stesso modo, e restava verde: i due lati
concordavano proprio in quanto sbagliavano insieme.

**Il secondo.** L'SBOM veniva scritto e nessuno verificava che fosse SPDX. Un
documento con un campo mancante o uno `SPDXID` ripetuto passa per valido finche'
non lo si da' a uno strumento, e a quel punto e' gia' stato consegnato.

**Il terzo.** Il manifesto portava un digest per ogni file e nessuno li
rileggeva. Un digest che nessuno verifica e' un numero, e per giunta una
garanzia apparente: chi legge il manifesto suppone che qualcuno l'abbia
controllata.
"""

from __future__ import annotations

import hashlib
import importlib.util
import json
import pathlib
import shutil
import tempfile
import unittest

RADICE = pathlib.Path(__file__).resolve().parent.parent


def carica(nome: str):
    percorso = RADICE / "scripts" / nome
    spec = importlib.util.spec_from_file_location(percorso.stem.replace("-", "_"), percorso)
    modulo = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(modulo)
    return modulo


class SondeDelValidatoreSpdx(unittest.TestCase):
    def setUp(self) -> None:
        self.d = carica("distribuzione.py")

    def documento(self, **campi) -> dict:
        base = self.d.documento_spdx(
            "artefatto-di-prova",
            "a" * 64,
            [
                {
                    "SPDXID": "SPDXRef-Package-uno",
                    "name": "uno",
                    "versionInfo": "1.0",
                    "downloadLocation": "NOASSERTION",
                    "licenseConcluded": "NOASSERTION",
                    "licenseDeclared": "MIT",
                    "filesAnalyzed": False,
                }
            ],
            "prova",
        )
        base.update(campi)
        return base

    def test_un_documento_ben_formato_passa(self) -> None:
        self.d.valida_spdx(self.documento())

    def test_un_campo_obbligatorio_mancante_fa_rosso(self) -> None:
        for campo in self.d.CAMPI_DEL_DOCUMENTO:
            with self.subTest(campo=campo):
                documento = self.documento()
                del documento[campo]
                with self.assertRaises(self.d.SpdxNonValido) as contesto:
                    self.d.valida_spdx(documento)
                self.assertIn(campo, str(contesto.exception))

    def test_una_versione_diversa_da_2_3_fa_rosso(self) -> None:
        with self.assertRaises(self.d.SpdxNonValido):
            self.d.valida_spdx(self.documento(spdxVersion="SPDX-2.2"))

    def test_una_datalicense_diversa_fa_rosso(self) -> None:
        """SPDX 2.3 ammette solo CC0-1.0: non e' una preferenza."""
        with self.assertRaises(self.d.SpdxNonValido):
            self.d.valida_spdx(self.documento(dataLicense="MIT"))

    def test_uno_spdxid_ripetuto_fa_rosso(self) -> None:
        """Due componenti con la stessa identita' rendono il documento ambiguo
        proprio dove serve essere precisi."""
        documento = self.documento()
        documento["packages"] = documento["packages"] * 2
        with self.assertRaises(self.d.SpdxNonValido) as contesto:
            self.d.valida_spdx(documento)
        self.assertIn("ripetuto", str(contesto.exception))

    def test_un_sbom_vuoto_fa_rosso(self) -> None:
        """Un SBOM vuoto dice che l'artefatto non contiene niente di terzi, e
        non e' vero di nessun artefatto che spedisca qualcosa."""
        with self.assertRaises(self.d.SpdxNonValido):
            self.d.valida_spdx(self.documento(packages=[]))

    def test_un_namespace_non_uri_fa_rosso(self) -> None:
        with self.assertRaises(self.d.SpdxNonValido):
            self.d.valida_spdx(self.documento(documentNamespace="non-un-uri"))

    def test_due_build_hanno_due_namespace(self) -> None:
        """`documentNamespace` deve identificare **questo** documento, non «un
        documento per questa versione»: due build della stessa versione
        producono due SBOM, e un namespace uguale li renderebbe
        indistinguibili."""
        primo = self.d.documento_spdx("stesso-nome", "a" * 64, [{"x": 1}], "")
        secondo = self.d.documento_spdx("stesso-nome", "b" * 64, [{"x": 1}], "")
        self.assertNotEqual(primo["documentNamespace"], secondo["documentNamespace"])

    def test_gli_identificatori_di_un_espressione(self) -> None:
        """`MIT OR Apache-2.0` sono due testi e non uno: e' chi riceve a
        scegliere, e per scegliere deve averli entrambi."""
        self.assertEqual(self.d.identificatori_di("MIT OR Apache-2.0"), ["MIT", "Apache-2.0"])
        self.assertEqual(self.d.identificatori_di("MIT/Apache-2.0"), ["MIT", "Apache-2.0"])
        self.assertEqual(
            self.d.identificatori_di("Apache-2.0 WITH LLVM-exception"),
            ["Apache-2.0", "LLVM-exception"],
        )


class SondeDeiDigest(unittest.TestCase):
    def setUp(self) -> None:
        self.tmp = pathlib.Path(tempfile.mkdtemp())
        self.addCleanup(shutil.rmtree, self.tmp, ignore_errors=True)
        self.gate = carica("check-digest-manifesto.py")

    def albero(self, con_digest: bool = True, file: int = 2) -> pathlib.Path:
        albero = self.tmp / "artefatto"
        (albero / "bin").mkdir(parents=True, exist_ok=True)
        voci = []
        for n in range(file):
            percorso = albero / "bin" / f"f{n}.bin"
            percorso.write_bytes(f"contenuto {n}".encode("utf-8"))
            relativo = f"bin/f{n}.bin"
            voci.append(
                {
                    "percorso": relativo,
                    "sha256": hashlib.sha256(percorso.read_bytes()).hexdigest(),
                    "byte": percorso.stat().st_size,
                }
                if con_digest
                else relativo
            )
        (albero / "MANIFEST.json").write_text(
            json.dumps(
                {
                    "piattaforma": "linux-x86_64",
                    "profilo": "base",
                    "canale": "prova",
                    "file": voci,
                }
            ),
            encoding="utf-8",
        )
        return albero

    def test_un_albero_integro_passa(self) -> None:
        errori, misure = self.gate.verifica(self.albero())
        self.assertEqual(errori, [])
        self.assertEqual(misure["file_verificati"], 2)
        self.assertEqual(misure["digest_divergenti"], 0)

    def test_un_file_alterato_fa_rosso(self) -> None:
        """E' la ragione per cui il controllo esiste: fra il manifesto e chi
        riceve ci sono l'archiviazione, il trasporto e l'estrazione."""
        albero = self.albero()
        (albero / "bin" / "f0.bin").write_bytes(b"altro contenuto")
        errori, misure = self.gate.verifica(albero)
        self.assertTrue(errori)
        self.assertEqual(misure["digest_divergenti"], 1)

    def test_un_file_mancante_fa_rosso(self) -> None:
        albero = self.albero()
        (albero / "bin" / "f1.bin").unlink()
        errori, _ = self.gate.verifica(albero)
        self.assertTrue(errori)
        self.assertIn("assenti", " ".join(errori))

    def test_un_file_non_dichiarato_fa_rosso(self) -> None:
        """Un albero che contiene qualcosa che il manifesto non nomina e' un
        albero di cui non si sa tutto."""
        albero = self.albero()
        (albero / "bin" / "intruso.bin").write_bytes(b"non dichiarato")
        errori, misure = self.gate.verifica(albero)
        self.assertTrue(errori)
        self.assertEqual(misure["file_non_dichiarati"], 1)

    def test_un_elenco_vuoto_non_passa_in_silenzio(self) -> None:
        """Zero file superano ogni confronto senza guardare niente, ed e' il
        modo piu' comodo di rendere verde questo controllo. Il difetto era
        reale: il profilo base dichiarava zero file."""
        albero = self.albero(file=0)
        errori, _ = self.gate.verifica(albero)
        self.assertTrue(errori)
        self.assertIn("nessun file", " ".join(errori))

    def test_un_elenco_di_soli_nomi_non_e_verificabile(self) -> None:
        """Un elenco di nomi dice che cosa c'era, non che cosa c'e'. Il difetto
        era reale: il profilo pieno li elencava cosi'."""
        errori, _ = self.gate.verifica(self.albero(con_digest=False))
        self.assertTrue(errori)
        self.assertIn("senza digest", " ".join(errori))


class SondeSullaCompletezzaDellSbom(unittest.TestCase):
    """Che l'SBOM non torni a elencare i soli componenti nativi."""

    def test_il_costruttore_prende_i_crate_dal_grafo_risolto(self) -> None:
        """Non da `Cargo.lock`, che elenca anche cio' che serve solo a
        costruire: dev-dependencies e build-dependencies non finiscono nei byte
        spediti, e un SBOM che le elencasse direbbe che spediamo software che
        non spediamo."""
        sorgente = (RADICE / "scripts" / "dipendenze-rust.py").read_text(encoding="utf-8")
        self.assertIn("dep_kinds", sorgente)
        self.assertIn("--locked", sorgente)
        self.assertIn("--filter-platform", sorgente)

    def test_entrambi_i_costruttori_includono_i_crate(self) -> None:
        for nome in ("costruisci-artefatto-linux.py", "costruisci-artefatto-windows.py"):
            with self.subTest(costruttore=nome):
                sorgente = (RADICE / "scripts" / nome).read_text(encoding="utf-8")
                self.assertIn("componenti_rust", sorgente)
                self.assertIn("valida_spdx", sorgente)
                self.assertIn("crate-rust", sorgente)

    def test_il_gate_confronta_anche_i_crate(self) -> None:
        """Il difetto era che confrontava due elenchi entrambi incompleti."""
        sorgente = (RADICE / "scripts" / "check-licenze-artefatto.py").read_text(encoding="utf-8")
        self.assertIn("crate_nel_sbom", sorgente)
        self.assertIn("CRATE.json", sorgente)
        self.assertIn("valida_spdx", sorgente)


if __name__ == "__main__":
    unittest.main()
