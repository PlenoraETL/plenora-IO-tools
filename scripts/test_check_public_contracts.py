#!/usr/bin/env python3
"""Sonde del verificatore del profilo pubblico.

Un gate incrementale ha una superficie di errore che un gate binario non ha:
puo' proteggere niente e sembrare verde. Le sonde qui sotto provano che le tre
regole mordono davvero, e ciascuna ha la propria controprova positiva -- senza,
«sempre rosso» sarebbe una difesa.
"""

from __future__ import annotations

import json
import os
import pathlib
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))

import check_public_contracts as gate  # noqa: E402

ROOT = pathlib.Path(__file__).resolve().parents[1]

# I due posti in cui un checkout dei contratti vive: `.plenora-contracts` in CI,
# dove lo mette il job, e `.s9-checkpoint/contratti` in locale, dove lo mette chi
# lavora. Cercarne uno solo avrebbe fatto **saltare** in silenzio la meta' delle
# sonde nell'altro ambiente -- e una suite che salta e' una suite che non
# protegge, che e' peggio di una rossa perche' non si vede.
CANDIDATI = (ROOT / ".plenora-contracts", ROOT / ".s9-checkpoint" / "contratti")


def contratti() -> pathlib.Path | None:
    for percorso in CANDIDATI:
        if (percorso / "schemas" / "error-v1.schema.json").is_file():
            return percorso
    return None


def invocazione(stdout: str = "", stderr: str = "", exit_code: int = 0) -> gate.Invocazione:
    return gate.Invocazione(
        argv=["finto"],
        exit_code=exit_code,
        stdout=stdout.encode("utf-8"),
        stderr=stderr.encode("utf-8"),
    )


class ArtefattoFinto:
    """Un artefatto che rende cio' che gli si dice, senza processi.

    Le sonde del verificatore non devono costruire un binario per provare che il
    verificatore funziona: costruirebbero il prodotto per misurare il metro.
    """

    def __init__(self, risposte: dict[tuple[str, ...], gate.Invocazione]) -> None:
        self.risposte = risposte
        self.chieste: list[tuple[str, ...]] = []

    def invoca(self, *argomenti: str) -> gate.Invocazione:
        self.chieste.append(argomenti)
        return self.risposte.get(argomenti, invocazione(exit_code=2))


BUSTA_CONFORME = json.dumps(
    {
        "status": "ok",
        "protocol_version": 2,
        "component": "plenora-io-tools",
        "component_version": "4.0.0",
        "contract": "plenora-io-catalog-v1",
        "command": "catalog",
        "result": {"drivers": []},
    }
)

ERRORE_CONFORME = json.dumps(
    {
        "status": "error",
        "protocol_version": 2,
        "component": "plenora-io-tools",
        "component_version": "4.0.0",
        "contract": "plenora-error-v1",
        "command": "inspect",
        "error": {
            "category": "io",
            "phase": "read",
            "remote_effect": "none",
            "retry": {"kind": "never"},
        },
    }
)


def artefatto_conforme() -> ArtefattoFinto:
    """Un artefatto che soddisfa ogni requisito del registro."""
    return ArtefattoFinto(
        {
            ("catalog",): invocazione(BUSTA_CONFORME + "\n"),
            ("catalog", "--format", "json"): invocazione(BUSTA_CONFORME + "\n"),
            ("inspect", "/nessun-file-esistente-per-la-sonda.shp"): invocazione(
                ERRORE_CONFORME + "\n", exit_code=5
            ),
            ("--help",): invocazione("uso: ...\n"),
            ("--version", "--format", "json"): invocazione(
                json.dumps(
                    {
                        "status": "ok",
                        "protocol_version": 2,
                        "component": "plenora-io-tools",
                        "component_version": "4.0.0",
                        "contract": "plenora-io-version-v1",
                        "command": "--version",
                        "result": {"component_version": "4.0.0", "cli_protocol_version": 2},
                    }
                )
                + "\n"
            ),
            ("capabilities", "--format", "json"): invocazione(
                json.dumps(
                    {
                        "status": "ok",
                        "protocol_version": 2,
                        "component": "plenora-io-tools",
                        "component_version": "4.0.0",
                        "contract": "plenora-capabilities-v2",
                        "command": "capabilities",
                        "result": {"operations": []},
                    }
                )
                + "\n"
            ),
        }
    )


class SondeDellAmbiente(unittest.TestCase):
    """Che le sonde girino davvero, dove devono girare.

    Tutto il resto di questo file salta quando manca un checkout dei contratti,
    e saltare in locale e' ragionevole: chi non ne ha uno sta lavorando ad
    altro. In CI non lo e'. Una suite che salta e' verde, e una suite verde che
    non ha misurato niente e' il modo preciso in cui un gate smette di
    proteggere senza che nessuno se ne accorga.
    """

    def test_in_ci_il_checkout_dei_contratti_c_e(self) -> None:
        if os.environ.get("GITHUB_ACTIONS") != "true":
            self.skipTest("fuori da GitHub Actions: saltare qui e' legittimo")
        self.assertIsNotNone(
            contratti(),
            f"nessun checkout dei contratti fra {CANDIDATI}: in CI tutte le "
            "sonde del profilo pubblico salterebbero, e il job sarebbe verde "
            "senza aver misurato niente.",
        )


class SondeVocabolario(unittest.TestCase):
    """Gli enum e la tabella vengono dal contratto, non da una copia."""

    @classmethod
    def setUpClass(cls) -> None:
        trovato = contratti()
        if trovato is None:
            raise unittest.SkipTest(f"nessun checkout dei contratti fra {CANDIDATI}")
        cls.contratti = trovato
        cls.vocabolario = gate.Vocabolario(cls.contratti)

    def test_la_tabella_copre_esattamente_le_categorie(self) -> None:
        """La difesa che ha colto il difetto: `internal` non si leggeva.

        Spezzando la cella sulle virgole, la riga «`internal` or an unmapped
        category» produceva una chiave che nessuna categoria avrebbe mai
        trovato. La sonda sulla proiezione sarebbe passata per assenza di
        corrispondenza invece che per conformita'.
        """
        self.assertEqual(
            set(self.vocabolario.codici_di_uscita), set(self.vocabolario.categorie)
        )
        self.assertEqual(self.vocabolario.codici_di_uscita["internal"], 70)

    def test_la_proiezione_e_quella_del_contratto(self) -> None:
        atteso = {"io": 5, "unsupported": 3, "cancelled": 130, "invalid_configuration": 2}
        for categoria, codice in atteso.items():
            with self.subTest(categoria=categoria):
                self.assertEqual(self.vocabolario.codici_di_uscita[categoria], codice)

    def test_una_proiezione_incompleta_e_rossa(self) -> None:
        """La copia non puo' restare indietro rispetto al contratto in silenzio."""
        with tempfile.TemporaryDirectory() as temporanea:
            percorso = pathlib.Path(temporanea) / "proiezione.json"
            percorso.write_text(json.dumps({"proiezione": {"io": 5}}), encoding="utf-8")
            with mock.patch.object(gate, "PROIEZIONE", percorso):
                with self.assertRaises(RuntimeError) as contesto:
                    gate.Vocabolario._tabella_dei_codici(
                        pathlib.Path("."), frozenset({"io", "internal"})
                    )
        self.assertIn("internal", str(contesto.exception))

    def test_una_proiezione_con_categorie_inventate_e_rossa(self) -> None:
        """Il verso opposto: una voce che il contratto non conosce.

        Senza, «copre tutte le categorie» sarebbe vero anche di una tabella che
        ne aggiunge di proprie, e una categoria inventata e' il modo in cui una
        copia comincia a descrivere un contratto diverso da quello fissato.
        """
        with tempfile.TemporaryDirectory() as temporanea:
            percorso = pathlib.Path(temporanea) / "proiezione.json"
            percorso.write_text(
                json.dumps({"proiezione": {"io": 5, "format_error": 1}}),
                encoding="utf-8",
            )
            with mock.patch.object(gate, "PROIEZIONE", percorso):
                with self.assertRaises(RuntimeError) as contesto:
                    gate.Vocabolario._tabella_dei_codici(
                        pathlib.Path("."), frozenset({"io"})
                    )
        self.assertIn("format_error", str(contesto.exception))

    def test_nessun_gate_legge_la_prosa_del_contratto(self) -> None:
        """Il difetto che `check_docset.py` ha colto, e che non deve tornare.

        La stesura precedente leggeva la tabella dei codici da `CLI-2.0.md`. Un
        gate che dipende dalla prosa dipende da qualcosa che si riscrive senza
        accorgersene, e infatti il parser sbagliava gia': spezzando la cella
        sulle virgole, la riga del 70 produceva una chiave che nessuna categoria
        avrebbe mai trovato.
        """
        sorgente = (ROOT / "scripts" / "check_public_contracts.py").read_text(
            encoding="utf-8"
        )
        codice = "\n".join(
            riga for riga in sorgente.splitlines() if not riga.strip().startswith("#")
        )
        self.assertNotIn('"CLI-2.0.md"', codice)
        self.assertNotIn("'CLI-2.0.md'", codice)


class SondeRegistro(unittest.TestCase):
    """Il registro e' leggibile prima che qualcosa venga invocato."""

    def voce(self, **modifiche) -> dict:
        base = {
            "id": "cli.aiuto",
            "regola": "CLI-2.0 §3",
            "descrizione": "x",
            "stato": "implementato",
        }
        base.update(modifiche)
        return base

    def registro(self, *voci) -> dict:
        return {"requisiti": list(voci)}

    def test_il_registro_reale_e_coerente(self) -> None:
        """La controprova positiva: senza, «sempre rosso» sarebbe una difesa."""
        reale = json.loads(gate.REGISTRO.read_text(encoding="utf-8"))
        self.assertEqual(gate.registro_coerente(reale), [])

    def test_ogni_sonda_ha_una_voce_e_viceversa(self) -> None:
        """L'insieme e' esatto nei due versi.

        Una sonda senza voce gira e il suo esito non e' classificato da nessuno;
        una voce senza sonda e' una dichiarazione che nessuno misura. Sono due
        modi diversi di avere un requisito che non protegge niente.
        """
        reale = json.loads(gate.REGISTRO.read_text(encoding="utf-8"))
        self.assertEqual(
            sorted(v["id"] for v in reale["requisiti"]), sorted(gate.SONDE)
        )

    def test_un_identificatore_ripetuto_e_rosso(self) -> None:
        errori = gate.registro_coerente(self.registro(self.voce(), self.voce()))
        self.assertTrue(any("due volte" in e for e in errori), errori)

    def test_non_ancora_senza_perche_e_rosso(self) -> None:
        """Un requisito mancante senza ragione non si distingue da uno dimenticato."""
        errori = gate.registro_coerente(
            self.registro(self.voce(stato="non_ancora"))
        )
        self.assertTrue(any("perche" in e for e in errori), errori)

    def test_uno_stato_inventato_e_rosso(self) -> None:
        errori = gate.registro_coerente(self.registro(self.voce(stato="quasi")))
        self.assertTrue(any("quasi" in e for e in errori), errori)

    def test_una_voce_senza_sonda_e_rossa(self) -> None:
        errori = gate.registro_coerente(self.registro(self.voce(id="cli.inventato")))
        self.assertTrue(any("non ha una sonda" in e for e in errori), errori)

    def test_un_registro_vuoto_e_rosso(self) -> None:
        """Un elenco vuoto passerebbe ogni regola per assenza di domanda."""
        errori = gate.registro_coerente({"requisiti": []})
        self.assertTrue(errori)


class SondeDelleTreRegole(unittest.TestCase):
    """Le tre regole dell'incrementalita', ciascuna con la sua controprova."""

    @classmethod
    def setUpClass(cls) -> None:
        trovato = contratti()
        if trovato is None:
            raise unittest.SkipTest(f"nessun checkout dei contratti fra {CANDIDATI}")
        cls.contratti = trovato

    def esegui(self, registro: dict, artefatto, esigente: bool = False) -> int:
        with tempfile.TemporaryDirectory() as temporanea:
            percorso = pathlib.Path(temporanea) / "registro.json"
            percorso.write_text(json.dumps(registro), encoding="utf-8")
            with mock.patch.object(gate, "REGISTRO", percorso), mock.patch.object(
                gate, "Artefatto", lambda _: artefatto
            ):
                return gate.esegui(self.contratti, pathlib.Path("finto"), esigente)

    def registro_reale(self) -> dict:
        return json.loads(gate.REGISTRO.read_text(encoding="utf-8"))

    def tutti(self, stato: str) -> dict:
        registro = self.registro_reale()
        for voce in registro["requisiti"]:
            voce["stato"] = stato
            if stato == "non_ancora":
                voce.setdefault("perche", "sonda finta")
                voce["perche"] = voce.get("perche") or "sonda finta"
        return registro

    def test_un_artefatto_conforme_con_tutto_implementato_e_verde(self) -> None:
        """La controprova positiva del gate intero."""
        esito = self.esegui(self.tutti("implementato"), artefatto_conforme())
        self.assertEqual(esito, 0)

    def test_una_regressione_e_rossa(self) -> None:
        """Cio' che la CI protegge: un `implementato` che smette di passare."""
        rotto = artefatto_conforme()
        rotto.risposte[("catalog",)] = invocazione(BUSTA_CONFORME, stderr="rumore\n")
        esito = self.esegui(self.tutti("implementato"), rotto)
        self.assertEqual(esito, 1)

    def test_un_mancante_dichiarato_non_ferma_la_ci(self) -> None:
        """Cio' che la CI rende visibile, senza bloccare."""
        vuoto = ArtefattoFinto({})
        esito = self.esegui(self.tutti("non_ancora"), vuoto)
        self.assertEqual(esito, 0)

    def test_un_avanzamento_non_dichiarato_e_rosso(self) -> None:
        """La regola che tiene in piedi le altre due.

        Un requisito soddisfatto e dichiarato mancante lascia il registro a
        descrivere un artefatto che non esiste piu': da quel momento la prima
        regola non lo protegge, e una regressione successiva passerebbe.
        """
        esito = self.esegui(self.tutti("non_ancora"), artefatto_conforme())
        self.assertEqual(esito, 1)

    def test_in_qualifica_un_mancante_e_rosso(self) -> None:
        """`--esigente`: la conformita' parziale non qualifica."""
        vuoto = ArtefattoFinto({})
        esito = self.esegui(self.tutti("non_ancora"), vuoto, esigente=True)
        self.assertEqual(esito, 1)

    def test_in_qualifica_un_artefatto_conforme_e_verde(self) -> None:
        esito = self.esegui(
            self.tutti("implementato"), artefatto_conforme(), esigente=True
        )
        self.assertEqual(esito, 0)


class SondeDelPin(unittest.TestCase):
    """Verificare contro una revisione diversa da quella fissata non vale."""

    def test_il_pin_reale_e_quaranta_esadecimali(self) -> None:
        adozione = json.loads(gate.ADOZIONE.read_text(encoding="utf-8"))
        revisione = adozione["contracts_source"]["revision"]
        self.assertRegex(revisione, r"^[0-9a-f]{40}$")

    def test_un_checkout_diverso_dal_pin_e_rosso(self) -> None:
        trovato = contratti()
        if trovato is None or not (trovato / ".git").exists():
            self.skipTest("nessun checkout git dei contratti")
        adozione = json.loads(gate.ADOZIONE.read_text(encoding="utf-8"))
        adozione["contracts_source"]["revision"] = "0" * 40
        with tempfile.TemporaryDirectory() as temporanea:
            percorso = pathlib.Path(temporanea) / "adozione.json"
            percorso.write_text(json.dumps(adozione), encoding="utf-8")
            with mock.patch.object(gate, "ADOZIONE", percorso):
                esito = gate.esegui(trovato, pathlib.Path("finto"), False)
        self.assertEqual(esito, 1)

    def test_il_checkout_reale_coincide_col_pin(self) -> None:
        """Il pin nomina una revisione che esiste davvero sul remoto."""
        adozione = json.loads(gate.ADOZIONE.read_text(encoding="utf-8"))
        trovato = contratti()
        if trovato is None or not (trovato / ".git").exists():
            self.skipTest("nessun checkout git dei contratti")
        corrente = subprocess.run(
            ["git", "rev-parse", "HEAD"],
            cwd=trovato,
            capture_output=True,
            check=True,
            text=True,
        ).stdout.strip()
        self.assertEqual(corrente, adozione["contracts_source"]["revision"])


class SondeDelleMisure(unittest.TestCase):
    """Le sonde misurano cio' che dicono, una domanda ciascuna."""

    @classmethod
    def setUpClass(cls) -> None:
        trovato = contratti()
        if trovato is None:
            raise unittest.SkipTest(f"nessun checkout dei contratti fra {CANDIDATI}")
        cls.vocabolario = gate.Vocabolario(trovato)

    def test_lo_stream_sbagliato_non_nasconde_gli_assi(self) -> None:
        """Un errore su stderr con assi corretti: una sonda rossa, non tutte.

        E' la ragione di `documento_ovunque`. Senza, diciotto sonde
        risulterebbero fallite per la stessa causa -- lo stream -- e il registro
        direbbe «diciotto cose rotte» dove ce n'e' una.
        """
        artefatto = ArtefattoFinto(
            {
                ("inspect", "/nessun-file-esistente-per-la-sonda.shp"): invocazione(
                    stderr=ERRORE_CONFORME + "\n", exit_code=5
                )
            }
        )
        self.assertTrue(gate.sonda_quattro_assi(artefatto, self.vocabolario).passata)
        self.assertTrue(gate.sonda_categoria(artefatto, self.vocabolario).passata)
        self.assertFalse(gate.sonda_errore_su_stdout(artefatto, self.vocabolario).passata)

    def test_una_categoria_fuori_vocabolario_e_rossa(self) -> None:
        corpo = json.loads(ERRORE_CONFORME)
        corpo["error"]["category"] = "format_error"
        artefatto = ArtefattoFinto(
            {
                ("inspect", "/nessun-file-esistente-per-la-sonda.shp"): invocazione(
                    json.dumps(corpo) + "\n", exit_code=5
                )
            }
        )
        esito = gate.sonda_categoria(artefatto, self.vocabolario)
        self.assertFalse(esito.passata)
        self.assertIn("format_error", esito.dettaglio)

    def test_retry_after_senza_delay_e_rosso(self) -> None:
        corpo = json.loads(ERRORE_CONFORME)
        corpo["error"]["retry"] = {"kind": "after"}
        artefatto = ArtefattoFinto(
            {
                ("inspect", "/nessun-file-esistente-per-la-sonda.shp"): invocazione(
                    json.dumps(corpo) + "\n", exit_code=5
                )
            }
        )
        self.assertFalse(gate.sonda_retry(artefatto, self.vocabolario).passata)

    def test_effetto_ignoto_con_ritentativo_automatico_e_rosso(self) -> None:
        """ERR-006: un effetto remoto ignoto non ammette il ritentativo."""
        corpo = json.loads(ERRORE_CONFORME)
        corpo["error"]["remote_effect"] = "unknown"
        corpo["error"]["retry"] = {"kind": "after", "delay_ms": 10}
        artefatto = ArtefattoFinto(
            {
                ("inspect", "/nessun-file-esistente-per-la-sonda.shp"): invocazione(
                    json.dumps(corpo) + "\n", exit_code=5
                )
            }
        )
        self.assertFalse(gate.sonda_retry(artefatto, self.vocabolario).passata)

    def test_due_documenti_su_stdout_sono_rossi(self) -> None:
        """CLI-2.0 §4: **un** documento e una newline, non due."""
        artefatto = ArtefattoFinto(
            {("catalog",): invocazione(BUSTA_CONFORME + "\n" + BUSTA_CONFORME + "\n")}
        )
        self.assertFalse(
            gate.sonda_successo_un_documento(artefatto, self.vocabolario).passata
        )

    def test_un_campo_di_primo_livello_estraneo_e_rosso(self) -> None:
        corpo = json.loads(BUSTA_CONFORME)
        corpo["drivers"] = []
        artefatto = ArtefattoFinto({("catalog",): invocazione(json.dumps(corpo) + "\n")})
        esito = gate.sonda_dati_dentro_result(artefatto, self.vocabolario)
        self.assertFalse(esito.passata)
        self.assertIn("drivers", esito.dettaglio)

    def test_la_proiezione_confronta_la_categoria_emessa(self) -> None:
        """Non un codice fisso: la categoria decide quale codice attendersi."""
        corpo = json.loads(ERRORE_CONFORME)
        corpo["error"]["category"] = "unsupported"
        artefatto = ArtefattoFinto(
            {
                ("inspect", "/nessun-file-esistente-per-la-sonda.shp"): invocazione(
                    json.dumps(corpo) + "\n", exit_code=5
                )
            }
        )
        esito = gate.sonda_proiezione_dei_codici(artefatto, self.vocabolario)
        self.assertFalse(esito.passata)
        self.assertIn("3", esito.dettaglio)

    def test_due_versioni_di_protocollo_sono_rosse(self) -> None:
        errore = json.loads(ERRORE_CONFORME)
        errore["protocol_version"] = 1
        artefatto = ArtefattoFinto(
            {
                ("catalog",): invocazione(BUSTA_CONFORME + "\n"),
                ("inspect", "/nessun-file-esistente-per-la-sonda.shp"): invocazione(
                    json.dumps(errore) + "\n", exit_code=5
                ),
            }
        )
        esito = gate.sonda_una_sola_versione(artefatto, self.vocabolario)
        self.assertFalse(esito.passata)
        self.assertIn("1", esito.dettaglio)

    def test_l_identificatore_maiuscolo_e_rosso(self) -> None:
        """`plenora-IO-tools` non soddisfa la forma `plenora-<domain>-tools`."""
        corpo = json.loads(BUSTA_CONFORME)
        corpo["component"] = "plenora-IO-tools"
        artefatto = ArtefattoFinto({("catalog",): invocazione(json.dumps(corpo) + "\n")})
        self.assertFalse(
            gate.sonda_identificatore_del_componente(artefatto, self.vocabolario).passata
        )

    def test_l_artefatto_e_invocato_una_volta_per_argomenti(self) -> None:
        """Piu' sonde leggono la stessa invocazione: due processi darebbero due esiti."""
        artefatto = gate.Artefatto(pathlib.Path(sys.executable))
        prima = artefatto.invoca("-c", "print(1)")
        seconda = artefatto.invoca("-c", "print(1)")
        self.assertIs(prima, seconda)


if __name__ == "__main__":
    unittest.main()
