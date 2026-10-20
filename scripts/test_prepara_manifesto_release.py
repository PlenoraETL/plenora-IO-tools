"""Acquisizione release/draft e fallback, senza rete o credenziali."""
import contextlib
import hashlib
import io
import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

try:
    from . import prepara_manifesto_release as prepara
except ImportError:
    import prepara_manifesto_release as prepara


class Preparazione(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        (self.root / "contracts").mkdir()
        (self.root / "assurance").mkdir()
        self.candidate = {"versione_manifesto": "9.0.0", "tag_previsto": "v9.0.0",
                          "revisione_candidate": "a" * 40, "artefatti": []}
        self.byte = b"byte congelati"
        self.nomi = {f"plenora-io-9.0.0-{piattaforma}-{profilo}.{estensione}": "cli"
                     for piattaforma, estensione in (("linux-x86_64", "tar.gz"), ("windows-x86_64", "zip"))
                     for profilo in ("base", "filegdb")}
        self.nomi["plenora-io-9.0.0-src.tar.gz"] = "rust"
        for nome in self.nomi:
            self.candidate["artefatti"].append({"nome": nome, "sha256": hashlib.sha256(self.byte).hexdigest(),
                "dimensione": len(self.byte), "revisione": "a" * 40})
        self.salva_stato()
        self.manifesto = {"artifacts": [{"name": n, "surface": s, "version": "9.0.0",
                          "digest": "sha256:" + hashlib.sha256(self.byte).hexdigest()} for n, s in self.nomi.items()]}
        self.release = {"id": 12, "tag_name": "v9.0.0", "draft": False}
        self.asset = True
        self.corrotto = False
        self.comandi = []
        for target, valore in (("ROOT", self.root),):
            p = patch.object(prepara, target, valore); p.start(); self.addCleanup(p.stop)
        p = patch.object(prepara.gate, "evidenza_ancora_valida", return_value=[])
        self.legame = p.start(); self.addCleanup(p.stop)
        p = patch.object(prepara.gate, "revisione_risolta", return_value="b" * 40)
        self.tag = p.start(); self.addCleanup(p.stop)
        p = patch.object(prepara, "valida_manifesto")
        self.validazione = p.start(); self.addCleanup(p.stop)
        p = patch.object(prepara, "esegui", side_effect=self.esegui)
        p.start(); self.addCleanup(p.stop)

    def salva_stato(self):
        (self.root / "assurance/current-state.json").write_text(json.dumps({"aperto": {"candidate_release": self.candidate}}))

    def esegui(self, *args):
        self.comandi.append(args)
        if args[:2] == ("gh", "api"):
            endpoint = args[-1]
            if "releases?" in endpoint:
                return json.dumps([[self.release] if self.release else []]).encode()
            if "/assets?" in endpoint:
                return json.dumps([[{"id": 13, "name": "adoption-manifest-9.0.0.json"}] if self.asset else []]).encode()
            if "/releases/assets/" in endpoint:
                return json.dumps(self.manifesto).encode()
            if "/actions/runs/" in endpoint:
                return b'{"path":".github/workflows/distribuzione.yml","conclusion":"success"}'
        if args[:3] == ("gh", "run", "download"):
            directory = Path(args[-1]); directory.mkdir()
            for nome in self.nomi:
                (directory / nome).write_bytes(b"corrotto" if self.corrotto else self.byte)
            return b""
        if "scripts/costruisci-manifesto-adozione.py" in args:
            Path(args[-1]).write_text(json.dumps(self.manifesto))
            return b""
        raise AssertionError(args)

    def test_release_su_commit_tag_distinto_dalla_candidate(self):
        prepara.prepara("owner/repo")
        self.assertTrue((self.root / "contracts/adoption-manifest.json").is_file())
        self.legame.assert_any_call("a" * 40, "b" * 40)
        self.assertFalse(any(c[:2] == ("gh", "run") for c in self.comandi))

    def test_draft_senza_tag_con_sha_fissato(self):
        self.tag.return_value = None
        self.release.update(draft=True, target_commitish="a" * 40)
        prepara.prepara("owner/repo")

    def test_draft_su_ramo_mutabile_rifiutata(self):
        self.tag.return_value = None
        self.release.update(draft=True, target_commitish="main")
        with self.assertRaisesRegex(prepara.PreparazioneError, "non legata"):
            prepara.prepara("owner/repo")

    def test_manifesto_assente_non_e_un_successo(self):
        self.release = None
        residuo = self.root / "contracts/adoption-manifest.json"
        residuo.write_text("residuo")
        with self.assertRaisesRegex(prepara.PreparazioneError, "manifesto mancante"):
            prepara.prepara("owner/repo")
        self.assertFalse(residuo.exists())

    def test_fallback_da_run_senza_release_o_senza_asset(self):
        for release in (None, self.release):
            self.release = release; self.asset = False
            prepara.prepara("owner/repo", "123")
            self.assertTrue((self.root / "contracts/adoption-manifest.json").is_file())

    def test_run_con_byte_diversi_rifiutato(self):
        self.release = None; self.corrotto = True
        with self.assertRaisesRegex(prepara.PreparazioneError, "byte congelati"):
            prepara.prepara("owner/repo", "123")

    def test_manifesto_remoto_con_digest_diverso_non_usa_fallback(self):
        self.manifesto["artifacts"][0]["digest"] = "sha256:" + "0" * 64
        with self.assertRaisesRegex(prepara.PreparazioneError, "byte congelati"):
            prepara.prepara("owner/repo", "123")
        self.assertFalse(any(c[:2] == ("gh", "run") for c in self.comandi))

    def test_cli_non_fissato_blocca(self):
        self.candidate["artefatti"].pop(0); self.salva_stato()
        with self.assertRaisesRegex(prepara.PreparazioneError, "non tutti fissati"):
            prepara.prepara("owner/repo", "123")

    def test_verbale_storico_senza_sorgente_non_inventa_digest(self):
        self.candidate["artefatti"].pop(); self.salva_stato()
        self.manifesto["artifacts"].pop()
        self.release = None
        with patch.object(prepara.gate, "_git", return_value="jobs: {}"):
            prepara.prepara("owner/repo", "123")
        risultato = json.loads((self.root / "contracts/adoption-manifest.json").read_text())
        self.assertEqual(len(risultato["artifacts"]), 4)

    def candidate_completa(self):
        # Anche i due Python partecipano alla qualifica, pur non essendo
        # superfici del manifesto pubblico. Ogni archivio ha byte distinti.
        nomi = prepara.gate.artefatti_attesi("9.0.0", candidate=self.candidate)
        self.candidate["artefatti"] = [
            {"nome": n, "sha256": hashlib.sha256(n.encode()).hexdigest(),
             "dimensione": len(n), "revisione": "a" * 40} for n in nomi]
        for voce in self.manifesto["artifacts"]:
            voce["digest"] = "sha256:" + hashlib.sha256(voce["name"].encode()).hexdigest()
        self.salva_stato()

    def test_candidate_con_sorgente_preparatore_e_gate_verdi(self):
        self.candidate_completa()
        prepara.prepara("owner/repo")
        self.assertEqual(prepara.gate._artefatti_fissati(self.candidate, "a" * 40, "9.0.0"), [])
        self.candidate.update(stato="attiva", release_action_allowed=True)
        with patch.object(prepara.gate, "_stato_corrente", return_value=({"aperto": {"candidate_release": self.candidate}}, [])), \
             patch.object(prepara.gate, "versione_workspace", return_value="9.0.0"), \
             patch.object(prepara.gate, "revisione_risolta", return_value="a" * 40):
            self.assertEqual(prepara.gate.condizione_candidate_coerente({}), [])
        risultato = json.loads((self.root / "contracts/adoption-manifest.json").read_text())
        self.assertIn("plenora-io-9.0.0-src.tar.gz", {v["name"] for v in risultato["artifacts"]})

    def test_sorgente_congelato_invalido_blocca_entrambi(self):
        self.candidate_completa()
        sorgente = next(v for v in self.candidate["artefatti"] if v["nome"].endswith("-src.tar.gz"))
        for campo, valore in (("sha256", "errato"), ("dimensione", 0), ("revisione", "b" * 40)):
            with self.subTest(campo=campo), patch.dict(sorgente, {campo: valore}):
                self.assertTrue(prepara.gate._artefatti_fissati(self.candidate, "a" * 40, "9.0.0"))
                with self.assertRaises(prepara.PreparazioneError):
                    prepara.attesi(self.candidate)

    def test_senza_sorgente_storico_o_nuovo_o_non_verificabile(self):
        self.candidate_completa()
        self.candidate["artefatti"] = [v for v in self.candidate["artefatti"]
                                      if not v["nome"].endswith("-src.tar.gz")]
        self.salva_stato()
        self.manifesto["artifacts"].pop()
        for workflow in ("jobs: {}", "run: python3 scripts/costruisci-archivio-sorgente.py", None):
            with self.subTest(workflow=workflow), patch.object(prepara.gate, "_git", return_value=workflow) as git:
                errori = prepara.gate._artefatti_fissati(self.candidate, "a" * 40, "9.0.0")
                if workflow == "jobs: {}":
                    self.assertEqual(errori, [])
                    prepara.prepara("owner/repo")
                else:
                    self.assertTrue(errori)
                    with self.assertRaisesRegex(prepara.PreparazioneError, "sorgente"):
                        prepara.prepara("owner/repo")
                git.assert_any_call("show", "a" * 40 + ":.github/workflows/distribuzione.yml")

    def test_candidate_di_altro_checkout_blocca(self):
        self.legame.return_value = ["fuori allowlist"]
        with self.assertRaisesRegex(prepara.PreparazioneError, "checkout"):
            prepara.prepara("owner/repo")

    def test_pin_o_schema_invalidi_non_materializzano_il_manifesto(self):
        self.validazione.side_effect = prepara.PreparazioneError("adozione o pin del manifesto non validi")
        with self.assertRaisesRegex(prepara.PreparazioneError, "pin"):
            prepara.prepara("owner/repo")
        self.assertFalse((self.root / "contracts/adoption-manifest.json").exists())

    def test_run_fallito_non_passato(self):
        self.release = None
        with patch.object(prepara, "api", return_value={"path": ".github/workflows/distribuzione.yml", "conclusion": "failure"}):
            with self.assertRaisesRegex(prepara.PreparazioneError, "distribuzione riuscita"):
                prepara.prepara("owner/repo", "123")

    def test_errore_acquisizione_non_equivale_ad_asset_assente(self):
        with patch.object(prepara, "esegui", side_effect=prepara.PreparazioneError("acquisizione fallita")):
            with self.assertRaisesRegex(prepara.PreparazioneError, "acquisizione fallita"):
                prepara.prepara("owner/repo", "123")
        self.assertFalse((self.root / "contracts/adoption-manifest.json").exists())

    def test_errori_senza_dati_remoti(self):
        with patch.object(prepara, "prepara", side_effect=json.JSONDecodeError("segreto", "segreto", 0)), patch.dict(os.environ, GITHUB_REPOSITORY="owner/repo"), patch("sys.argv", ["prepara"]):
            stderr = io.StringIO()
            with contextlib.redirect_stderr(stderr):
                self.assertEqual(prepara.main(), 1)
            self.assertNotIn("segreto", stderr.getvalue())


class Workflow(unittest.TestCase):
    def test_preparazione_prima_del_gate_senza_condizioni_di_assenza(self):
        root = Path(__file__).resolve().parents[1]
        testo = (root / ".github/workflows/release-qualification.yml").read_text(encoding="utf-8")
        self.assertLess(testo.index("path: .plenora-contracts"), testo.index("scripts/prepara_manifesto_release.py"))
        self.assertLess(testo.index("scripts/prepara_manifesto_release.py"), testo.index("python3 scripts/check_release_contract.py --release"))
        self.assertIn("ref: ${{ steps.contracts-pin.outputs.revision }}", testo)
        self.assertNotIn("continue-on-error", testo)
        self.assertIn("_candidate_legata_alle_fonti", testo)
