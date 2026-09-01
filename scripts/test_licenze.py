"""Sonde sulla completezza di `LICENSES/`.

# Il difetto che chiudono

Il costruttore accettava tre pacchetti che mettevano byte nell'artefatto --
`libgcc_s.so.1`, `libstdc++.so.6`, `libsqlite3.so` -- portando soltanto la
licenza **dichiarata**. Erano nominati in `PROVENIENZA.json`, il che evitava il
silenzio: chi leggeva sapeva che mancava qualcosa. Ma sapere che manca una
licenza non e' consegnarla, e cio' che una licenza obbliga a distribuire e' il
testo.

# Le tre domande

Il committente le ha poste cosi', e sono tre perche' un solo controllo le
confonderebbe:

1. **testo rimosso -> rosso.** Un artefatto a cui si toglie un testo dev'essere
   rifiutato. E' la domanda su cio' che *c'e'*, e la fa il gate.
2. **byte e sola dichiarazione -> rosso.** Un componente che spedisce file e non
   riesce a procurarsi un testo deve fermare la costruzione. E' la domanda su
   cio' che si sta *facendo*, e la fa il costruttore.
3. **metapacchetto senza byte -> ammesso.** Un pacchetto che non mette file
   nell'albero non ha nulla da licenziare. Serve che questa terza resti
   verificata insieme alle altre due: senza, basterebbe declassare un componente
   a metapacchetto per farlo tacere.
"""

from __future__ import annotations

import importlib.util
import json
import pathlib
import shutil
import subprocess
import sys
import tempfile
import unittest

RADICE = pathlib.Path(__file__).resolve().parent.parent
GATE = RADICE / "scripts" / "check-licenze-artefatto.py"
COSTRUTTORE = RADICE / "scripts" / "costruisci-artefatto-linux.py"
LOCK = RADICE / "scripts" / "linux-gdal-lock.json"
TESTI_DI_LICENZA = RADICE / "scripts" / "testi-di-licenza.json"


def carica(percorso: pathlib.Path):
    spec = importlib.util.spec_from_file_location(percorso.stem.replace("-", "_"), percorso)
    modulo = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(modulo)
    return modulo


def artefatto_finto(radice: pathlib.Path, componenti: dict[str, list[str]]) -> pathlib.Path:
    """Un albero con la forma di un artefatto, e nient'altro.

    Non serve un artefatto vero: il gate guarda `LICENSES/`, il SBOM, la
    provenienza e il manifesto, e su quelli si puo' mentire deliberatamente per
    vedere se se ne accorge.
    """
    albero = radice / "artefatto"
    (albero / "LICENSES").mkdir(parents=True)
    for nome, testi in componenti.items():
        directory = albero / "LICENSES" / nome
        directory.mkdir()
        for testo in testi:
            (directory / testo).write_text("testo della licenza\n", encoding="utf-8")
    # L'SBOM finto e' un SPDX **valido**: il gate lo valida, ed e' giusto che
    # lo faccia. Un artefatto di prova malformato renderebbe rossa la sonda per
    # una ragione che non e' quella che sta provando.
    (albero / "SBOM.spdx.json").write_text(
        json.dumps(
            {
                "spdxVersion": "SPDX-2.3",
                "dataLicense": "CC0-1.0",
                "SPDXID": "SPDXRef-DOCUMENT",
                "name": "artefatto-di-prova",
                "documentNamespace": "https://plenora.invalid/sbom/prova/" + "a" * 16,
                "creationInfo": {"creators": ["Tool: sonde"]},
                "packages": [
                    {
                        "SPDXID": f"SPDXRef-Package-{n}",
                        "name": n,
                        "versionInfo": "1.0",
                        "downloadLocation": "NOASSERTION",
                        "licenseConcluded": "NOASSERTION",
                        "licenseDeclared": "MIT",
                        "filesAnalyzed": False,
                        "comment": "pacchetto nativo, build 0",
                    }
                    for n in componenti
                ]
                + [
                    {
                        "SPDXID": "SPDXRef-Crate-esempio-1-0",
                        "name": "esempio",
                        "versionInfo": "1.0",
                        "downloadLocation": "registry+https://github.com/rust-lang/crates.io-index",
                        "licenseConcluded": "NOASSERTION",
                        "licenseDeclared": "MIT",
                        "filesAnalyzed": False,
                        "comment": "crate Rust linkato staticamente nel binario",
                    }
                ],
            }
        ),
        encoding="utf-8",
    )
    crate = albero / "LICENSES" / "crate-rust"
    crate.mkdir(parents=True, exist_ok=True)
    (crate / "MIT.txt").write_text("testo MIT", encoding="utf-8")
    (crate / "CRATE.json").write_text(
        json.dumps(
            {
                "profilo": "base",
                "linkati": 1,
                "di_terzi": 1,
                "identificatori": ["MIT"],
                "pacchetti": [
                    {"nome": "esempio", "versione": "1.0", "licenza": "MIT", "nostro": False}
                ],
            }
        ),
        encoding="utf-8",
    )
    (albero / "LICENSES" / "PROVENIENZA.json").write_text(
        json.dumps({"pacchetti": [{"nome": n} for n in componenti]}), encoding="utf-8"
    )
    (albero / "MANIFEST.json").write_text(
        json.dumps(
            {
                "licenze": {
                    "con_testo_proprio": len(componenti),
                    "con_testo_canonico": 0,
                    "senza_testo": 0,
                }
            }
        ),
        encoding="utf-8",
    )
    return albero


class SondeDelGate(unittest.TestCase):
    """La domanda su cio' che c'e'."""

    def setUp(self) -> None:
        self.tmp = pathlib.Path(tempfile.mkdtemp())
        self.addCleanup(shutil.rmtree, self.tmp, ignore_errors=True)
        self.gate = carica(GATE)

    def test_un_artefatto_completo_passa(self) -> None:
        albero = artefatto_finto(self.tmp, {"libfoo": ["LICENSE"], "libbar": ["COPYING"]})
        self.assertEqual(self.gate.verifica(albero), [])

    def test_testo_rimosso_fa_rosso(self) -> None:
        """La prima domanda del committente.

        Un artefatto assemblato bene puo' essere estratto e rimpacchettato, e
        fra il costruttore e chi riceve c'e' spazio perche' un file sparisca."""
        albero = artefatto_finto(self.tmp, {"libfoo": ["LICENSE"], "libbar": ["COPYING"]})
        shutil.rmtree(albero / "LICENSES" / "libbar")
        errori = self.gate.verifica(albero)
        self.assertTrue(errori, "un testo rimosso deve fare rosso")
        self.assertIn("libbar", " ".join(errori))

    def test_un_testo_vuoto_non_conta_come_testo(self) -> None:
        """«Esiste» e' una soglia troppo bassa: un file di zero byte la supera
        e non consegna niente."""
        albero = artefatto_finto(self.tmp, {"libfoo": ["LICENSE"]})
        (albero / "LICENSES" / "libfoo" / "LICENSE").write_text("", encoding="utf-8")
        errori = self.gate.verifica(albero)
        self.assertTrue(errori)
        self.assertIn("vuot", " ".join(errori).lower())

    def test_un_manifesto_che_dichiara_senza_testo_fa_rosso(self) -> None:
        """Dichiararlo evitava il silenzio; non consegnava la licenza."""
        albero = artefatto_finto(self.tmp, {"libfoo": ["LICENSE"]})
        manifesto = json.loads((albero / "MANIFEST.json").read_text(encoding="utf-8"))
        manifesto["licenze"]["senza_testo"] = 1
        (albero / "MANIFEST.json").write_text(json.dumps(manifesto), encoding="utf-8")
        self.assertTrue(self.gate.verifica(albero))

    def test_sbom_e_provenienza_devono_coincidere(self) -> None:
        """Sono due viste della stessa cosa: divergono solo se una mente."""
        albero = artefatto_finto(self.tmp, {"libfoo": ["LICENSE"]})
        sbom = json.loads((albero / "SBOM.spdx.json").read_text(encoding="utf-8"))
        sbom["packages"].append({"name": "libfantasma"})
        (albero / "SBOM.spdx.json").write_text(json.dumps(sbom), encoding="utf-8")
        self.assertTrue(self.gate.verifica(albero))


class SondeDelCostruttore(unittest.TestCase):
    """La domanda su cio' che si sta facendo."""

    def setUp(self) -> None:
        self.tmp = pathlib.Path(tempfile.mkdtemp())
        self.addCleanup(shutil.rmtree, self.tmp, ignore_errors=True)
        self.costruttore = carica(COSTRUTTORE)

    def prefisso_finto(self, pacchetti: list[dict]) -> pathlib.Path:
        """Un prefisso con il solo `conda-meta`, che e' cio' che il costruttore
        legge per sapere chi ha messo cosa."""
        prefisso = self.tmp / "prefisso"
        (prefisso / "conda-meta").mkdir(parents=True)
        for p in pacchetti:
            (prefisso / "conda-meta" / f"{p['name']}-1.0-0.json").write_text(
                json.dumps(p), encoding="utf-8"
            )
        return prefisso

    def test_un_metapacchetto_senza_byte_non_e_un_componente(self) -> None:
        """La terza domanda del committente.

        `mappa_dei_pacchetti` associa i file ai pacchetti; un pacchetto che non
        ne ha non compare, e quindi non gli si chiede una licenza. Non e'
        un'esenzione: e' il criterio stesso -- «ha messo un file in questo
        albero» -- ed e' verificato qui perche' resti tale."""
        prefisso = self.prefisso_finto(
            [
                {"name": "libvero", "version": "1.0", "build": "0", "license": "MIT",
                 "files": ["lib/libvero.so"]},
                {"name": "meta", "version": "1.0", "build": "0", "license": "",
                 "files": []},
            ]
        )
        mappa = self.costruttore.mappa_dei_pacchetti(prefisso)
        self.assertIn("lib/libvero.so", mappa)
        self.assertNotIn("meta", {v["nome"] for v in mappa.values()})

    def test_gli_identificatori_di_un_espressione_spdx(self) -> None:
        """`GPL-3.0-only WITH GCC-exception-3.1` sono **due** testi.

        La seconda e' cio' che rende distribuibile un binario linkato alla
        prima: consegnare solo la GPL sarebbe consegnare meta' della ragione per
        cui l'artefatto puo' esistere."""
        self.assertEqual(
            self.costruttore.identificatori_spdx("GPL-3.0-only WITH GCC-exception-3.1"),
            ["GPL-3.0-only", "GCC-exception-3.1"],
        )
        self.assertEqual(self.costruttore.identificatori_spdx("blessing"), ["blessing"])
        self.assertEqual(
            self.costruttore.identificatori_spdx("(MIT OR Apache-2.0)"),
            ["MIT", "Apache-2.0"],
        )

    def componenti(self, licenza: str, quanti_file: int = 1):
        pacchetti = {
            "libqualcosa": {
                "nome": "libqualcosa",
                "versione": "1.0",
                "build": "0",
                "licenza": licenza,
                # Nessuna directory estratta: il pacchetto non porta il proprio
                # testo, che e' esattamente il caso in questione.
                "directory_estratta": "",
            }
        }
        per_pacchetto = {"libqualcosa": [f"lib/f{i}.so" for i in range(quanti_file)]}
        return pacchetti, per_pacchetto

    def test_byte_e_sola_dichiarazione_fermano_la_costruzione(self) -> None:
        """La seconda domanda del committente.

        Un componente che mette file nell'albero, non porta il proprio testo e
        dichiara una licenza il cui testo non e' fissato nel lock: il
        costruttore deve **fermarsi**. Prima lo nominava in un elenco, il che
        evitava il silenzio e non consegnava la licenza."""
        pacchetti, per_pacchetto = self.componenti("Licenza-Che-Nessuno-Ha-Fissato-1.0", quanti_file=3)
        licenze = self.tmp / "LICENSES"
        licenze.mkdir()
        with self.assertRaises(SystemExit) as contesto:
            self.costruttore.testi_di_licenza(
                pacchetti, per_pacchetto, {"identificatori": {}}, licenze, self.tmp / "cache"
            )
        messaggio = str(contesto.exception)
        self.assertIn("Licenza-Che-Nessuno-Ha-Fissato-1.0", messaggio)
        self.assertIn("3 file", messaggio, "il rifiuto deve dire quanti byte sono in gioco")

    def test_byte_e_nessuna_licenza_dichiarata_fermano_la_costruzione(self) -> None:
        """Il caso peggiore: spedisce file e non dice nemmeno sotto che cosa."""
        pacchetti, per_pacchetto = self.componenti("")
        licenze = self.tmp / "LICENSES2"
        licenze.mkdir()
        with self.assertRaises(SystemExit):
            self.costruttore.testi_di_licenza(
                pacchetti, per_pacchetto, {"identificatori": {}}, licenze, self.tmp / "cache"
            )

    def test_un_componente_con_il_proprio_testo_lo_usa(self) -> None:
        """Il testo che il pacchetto porta con se' vince su quello canonico: e'
        piu' vicino a cio' che ha effettivamente spedito."""
        estratta = self.tmp / "estratto"
        (estratta / "info" / "licenses").mkdir(parents=True)
        (estratta / "info" / "licenses" / "COPYING").write_text("il suo testo" + chr(10), encoding="utf-8")
        pacchetti = {
            "libsuo": {
                "nome": "libsuo", "versione": "1.0", "build": "0",
                "licenza": "GPL-3.0-only", "directory_estratta": str(estratta),
            }
        }
        licenze = self.tmp / "LICENSES3"
        licenze.mkdir()
        propri, canonici = self.costruttore.testi_di_licenza(
            pacchetti, {"libsuo": ["lib/x.so"]}, {"identificatori": {}}, licenze, self.tmp / "cache"
        )
        self.assertEqual((propri, canonici), (1, []))
        self.assertEqual(
            (licenze / "libsuo" / "COPYING").read_text(encoding="utf-8"), "il suo testo" + chr(10)
        )

    def test_i_testi_non_stanno_in_nessun_lock(self) -> None:
        """Un digest in due posti e' una seconda verita', e i lock sono due."""
        for nome in ("linux-gdal-lock.json", "windows-gdal-lock.json"):
            percorso = RADICE / "scripts" / nome
            if not percorso.exists():
                continue
            with self.subTest(lock=nome):
                self.assertNotIn(
                    "testi_di_licenza_esterni",
                    json.loads(percorso.read_text(encoding="utf-8")),
                )

    def test_gli_identificatori_mancanti_si_elencano_tutti(self) -> None:
        """Fermarsi al primo costringe a un giro di costruzione per ciascuno, e
        dove ogni giro e' venti minuti di runner la differenza fra «uno alla
        volta» e «tutti insieme» e' un pomeriggio."""
        pacchetti = {
            "primo": {"nome": "primo", "versione": "1", "build": "0",
                      "licenza": "Licenza-A", "directory_estratta": ""},
            "secondo": {"nome": "secondo", "versione": "1", "build": "0",
                        "licenza": "Licenza-B WITH Eccezione-C", "directory_estratta": ""},
        }
        licenze = self.tmp / "LICENSES-molti"
        licenze.mkdir()
        with self.assertRaises(SystemExit) as contesto:
            self.costruttore.testi_di_licenza(
                pacchetti,
                {"primo": ["a"], "secondo": ["b", "c"]},
                {"identificatori": {}},
                licenze,
                self.tmp / "cache",
            )
        messaggio = str(contesto.exception)
        for atteso in ("Licenza-A", "Licenza-B", "Eccezione-C", "primo", "secondo"):
            self.assertIn(atteso, messaggio, "il rifiuto deve elencarli tutti")

    def test_un_testo_che_non_corrisponde_al_checksum_ferma_tutto(self) -> None:
        """La verifica e' la stessa che si fa sui pacchetti, e per la stessa
        ragione: un testo che cambia sotto un checksum fissato deve far fallire
        il checksum, non entrare nell'artefatto perche' l'URL rispondeva."""
        cache = self.tmp / "cache"
        cache.mkdir()
        (cache / "MIT.txt").write_text("non e' il testo atteso", encoding="utf-8")
        fonte = {"url": "https://esempio.invalid/MIT.txt", "dimensione": 22, "sha256": "0" * 64}
        with self.assertRaises(SystemExit) as contesto:
            self.costruttore.procurati_testo("MIT", fonte, cache)
        self.assertIn("sha256", str(contesto.exception))

    def test_ogni_identificatore_fissato_e_verificabile(self) -> None:
        """Senza URL, dimensione e sha256 il costruttore dovrebbe fidarsi.

        I testi stanno in un file **comune** e non nei lock: non dipendono dalla
        piattaforma, e tenerne una copia per lock vorrebbe dire due digest per
        lo stesso file. Il difetto si e' visto costruendo su Windows, dove il
        lock non li aveva affatto."""
        testi = json.loads(TESTI_DI_LICENZA.read_text(encoding="utf-8"))
        self.assertTrue(testi["identificatori"])
        for identificatore, fonte in testi["identificatori"].items():
            with self.subTest(identificatore=identificatore):
                self.assertTrue(fonte["url"].startswith("https://"))
                self.assertEqual(len(fonte["sha256"]), 64)
                self.assertIsInstance(fonte["dimensione"], int)
                self.assertIn(testi["tag"], fonte["url"], "l'URL non porta il tag fissato")


class SondaSullArtefattoVero(unittest.TestCase):
    """Se un artefatto costruito e' a portata di mano, lo si verifica.

    Le sonde sopra lavorano su alberi finti, che e' cio' che le rende
    eseguibili in L1. Questa chiude il cerchio quando l'artefatto c'e'
    davvero, e si salta quando non c'e': una sonda che pretendesse un
    artefatto renderebbe L1 dipendente da una costruzione.
    """

    def test_l_artefatto_indicato_dall_ambiente_e_completo(self) -> None:
        import os

        percorso = os.environ.get("PLENORA_ARTEFATTO")
        if not percorso:
            self.skipTest("PLENORA_ARTEFATTO non impostata")
        esito = subprocess.run(
            [sys.executable, str(GATE), "--albero", percorso],
            capture_output=True,
            text=True,
        )
        self.assertEqual(esito.returncode, 0, esito.stdout + esito.stderr)


if __name__ == "__main__":
    unittest.main()
