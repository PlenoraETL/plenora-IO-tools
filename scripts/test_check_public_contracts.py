#!/usr/bin/env python3
"""Sonde del verificatore del profilo pubblico.

Un gate incrementale ha una superficie di errore che un gate binario non ha:
puo' proteggere niente e sembrare verde. Le sonde qui sotto provano che le tre
regole mordono davvero, e ciascuna ha la propria controprova positiva -- senza,
«sempre rosso» sarebbe una difesa.
"""

from __future__ import annotations

import copy
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


def invocazione(
    stdout: str = "", stderr: str = "", exit_code: int = 0, guasto: str | None = None
) -> gate.Invocazione:
    return gate.Invocazione(
        argv=["finto"],
        exit_code=exit_code,
        stdout=stdout.encode("utf-8"),
        stderr=stderr.encode("utf-8"),
        guasto=guasto,
    )


class ArtefattoFinto:
    """Un artefatto che rende cio' che gli si dice, senza processi.

    Le sonde del verificatore non devono costruire un binario per provare che il
    verificatore funziona: costruirebbero il prodotto per misurare il metro.
    """

    def __init__(self, risposte: dict[tuple[str, ...], gate.Invocazione]) -> None:
        self.risposte = risposte
        self.chieste: list[tuple[str, ...]] = []
        self.guasti: list[str] = []
        #: Se il finto sappia consegnare. Spento per i finti che provano un
        #: artefatto rotto: li' `read --output` deve fallire come tutto il resto.
        self.consegna_attiva = False

    def invoca(self, *argomenti: str) -> gate.Invocazione:
        self.chieste.append(argomenti)
        corsa = self.risposte.get(argomenti) or self._consegna(argomenti)
        if corsa is None:
            corsa = invocazione(exit_code=2)
        if corsa.guasto:
            self.guasti.append(f"{' '.join(argomenti) or '(nessun argomento)'}: {corsa.guasto}")
        return corsa

    def _consegna(self, argomenti: tuple[str, ...]) -> gate.Invocazione | None:
        """Le due forme di `read`, su percorsi che il chiamante sceglie.

        Il percorso lo crea la sonda in una directory temporanea, quindi il
        finto non lo puo' avere in tabella: deve **rispondere** all'argomento
        invece di riconoscerlo.

        Scrivere davvero i byte e' cio' che rende la sonda della consegna una
        prova: un finto che dichiarasse una consegna senza produrre il file la
        farebbe passare a vuoto, che e' lo stesso difetto che quella sonda esiste
        per cogliere. E senza `--output` `delivered` vale `None`, perche' e'
        l'altra meta' di cio' che la sonda verifica.
        """
        if not self.consegna_attiva:
            return None
        if argomenti[:1] == ("write",):
            return self._pubblica(argomenti)
        if argomenti[:1] != ("read",):
            return None

        consegna = None
        if "--output" in argomenti:
            percorso = pathlib.Path(argomenti[argomenti.index("--output") + 1])
            byte = b"ARROW1" + b"\x00" * 58
            percorso.write_bytes(byte)
            consegna = {
                "content_type": "application/vnd.apache.arrow.file",
                "interchange_contract": "plenora-arrow-interchange-v1",
                "bytes_written": len(byte),
                "publish_outcome": "published",
            }

        busta = {
            "status": "ok",
            "protocol_version": 2,
            "component": "plenora-io-tools",
            "component_version": "4.0.0",
            "contract": "plenora-io-read-result-v1",
            "command": "read",
            "result": {
                "format": "geojson",
                "fidelity": {"level": "lossless"},
                "loss": {"counts": []},
                "layer": {"name": "x"},
                "rows_read": 5,
                "batches": 1,
                "truncated": False,
                "delivered": consegna,
            },
        }
        return invocazione(json.dumps(busta) + "\n")


    def _pubblica(self, argomenti: tuple[str, ...]) -> gate.Invocazione | None:
        """Le tre risposte di `write` che le sonde distinguono.

        Un finto che dicesse sempre di si' farebbe passare a vuoto le sonde del
        formato esplicito e del rollback, che esistono proprio per cogliere un
        `write` troppo accomodante. Quindi qui si rifiuta esattamente dove il
        prodotto rifiuta: senza `--to`, con un formato fuori dal catalogo, e
        quando `--to` e l'estensione della destinazione si contraddicono.
        """
        def _uso() -> gate.Invocazione:
            return invocazione(
                json.dumps(
                    {
                        "status": "error",
                        "protocol_version": 2,
                        "component": "plenora-io-tools",
                        "component_version": "4.0.0",
                        "contract": "plenora-error-v1",
                        "command": "write",
                        "error": {
                            "category": "invalid_configuration",
                            "code": "CLI_USAGE",
                            "message": "write richiede --to <formato>",
                            "phase": "validate",
                            "remote_effect": "none",
                            "retry": {"kind": "never"},
                        },
                    }
                )
                + "\n",
                exit_code=2,
            )

        if len(argomenti) < 3:
            return _uso()
        destinazione = pathlib.Path(argomenti[2])

        if "--to" not in argomenti:
            return _uso()
        formato = argomenti[argomenti.index("--to") + 1]
        if formato not in ("csv", "geojson", "gpkg", "ipc"):
            return invocazione(
                json.dumps(
                    {
                        "status": "error",
                        "protocol_version": 2,
                        "component": "plenora-io-tools",
                        "component_version": "4.0.0",
                        "contract": "plenora-error-v1",
                        "command": "write",
                        "error": {
                            "category": "unsupported",
                            "code": "UNKNOWN_FORMAT",
                            "message": "formato non riconosciuto",
                            "phase": "validate",
                            "remote_effect": "none",
                            "retry": {"kind": "never"},
                        },
                    }
                )
                + "\n",
                exit_code=3,
            )

        # Il suffisso della destinazione **non** decide piu' il rifiuto, e non
        # e' una semplificazione del finto: e' la regola. Il solo formato che lo
        # pretende qui sarebbe GeoPackage, e nessuna sonda lo esercita su
        # questo percorso.
        #
        # Resta il dataset proiettato che il sink non sa esprimere: la sonda del
        # rollback lo costruisce leggendo il GeoPackage, e li' il rifiuto e' del
        # formato, non del nome.
        if "proiettato" in destinazione.name or "mai_nato" in destinazione.name:
            return invocazione(
                json.dumps(
                    {
                        "status": "error",
                        "protocol_version": 2,
                        "component": "plenora-io-tools",
                        "component_version": "4.0.0",
                        "contract": "plenora-error-v1",
                        "command": "write",
                        "error": {
                            "category": "unsupported",
                            "code": "UNSUPPORTED",
                            "message": "il sink non regge questa destinazione",
                            "phase": "validate",
                            "remote_effect": "none",
                            "retry": {"kind": "never"},
                        },
                    }
                )
                + "\n",
                exit_code=3,
            )

        # Byte che **sembrano** il formato chiesto: la sonda del formato
        # esplicito guarda il contenuto, non la busta, e un finto che scrivesse
        # sempre la stessa cosa la farebbe passare a vuoto.
        byte = b"a,b\n1,2\n" if formato == "csv" else b'{"type":"FeatureCollection"}\n' 
        destinazione.write_bytes(byte)
        busta = {
            "status": "ok",
            "protocol_version": 2,
            "component": "plenora-io-tools",
            "component_version": "4.0.0",
            "contract": "plenora-io-write-result-v1",
            "command": "write",
            "result": {
                "format": formato,
                "input": {
                    "content_type": "application/vnd.apache.arrow.file",
                    "interchange_contract": "plenora-arrow-interchange-v1",
                },
                "layers": [{"name": "x", "rows": 1, "batches": 1}],
                "rows_written": 1,
                "bytes_written": len(byte),
                "publish_outcome": "published",
                "fidelity": {"level": "lossless", "reasons": []},
                "input_fidelity": {"level": "lossless", "reasons": []},
                "write_fidelity": {"level": "lossless", "reasons": []},
                "input_loss": {"lossless": True, "counts": []},
                "write_loss": {"lossless": True, "counts": []},
            },
        }
        return invocazione(json.dumps(busta) + "\n")


BUSTA_CONFORME = json.dumps(
    {
        "status": "ok",
        "protocol_version": 2,
        "component": "plenora-io-tools",
        "component_version": "4.0.0",
        "contract": "plenora-io-catalog-v1",
        "command": "catalog",
        # Due driver e non zero: `sonda_vincoli_del_percorso` deve vedere sia il
        # caso libero sia quello vincolato, altrimenti passerebbe a vuoto su un
        # catalogo che non dichiara niente.
        "result": {
            "drivers": [
                {
                    "id": "csv",
                    "direction": "bidirectional",
                    "recognised_suffixes": ["csv"],
                    "write_capabilities": {"sink_path": {"kind": "free"}},
                },
                {
                    "id": "gpkg",
                    "direction": "bidirectional",
                    "recognised_suffixes": ["gpkg"],
                    "write_capabilities": {
                        "sink_path": {
                            "kind": "required",
                            "suffixes": ["gpkg"],
                            "reason": "format_specification",
                        }
                    },
                },
            ]
        },
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


#: Un documento capability che soddisfa le quattro sonde: le sei operazioni
#: del catalogo, tutte disponibili, sulla sola superficie dichiarata.
CAPABILITY_CONFORME = {
    "schema_version": 2,
    "component": "plenora-io-tools",
    "component_version": "4.0.0",
    "interfaces": [
        {
            "kind": "cli",
            "contract": "plenora-cli-v2",
            "version": 2,
            "artifact": "plenora-io"
        }
    ],
    "operations": [
        {
            "id": "io.catalog",
            "version": 1,
            "status": "available",
            "surfaces": [
                "cli"
            ],
            "input": {
                "contract": "plenora-io-catalog-input-v1",
                "content_types": [
                    "application/json"
                ]
            },
            "output": {
                "contract": "plenora-io-catalog-v1",
                "content_types": [
                    "application/json"
                ]
            },
            "side_effect": "none",
            "controls": {
                "cancellation": True,
                "deadline": True,
                "idempotency_key": False
            }
        },
        {
            "id": "io.inspect",
            "version": 1,
            "status": "available",
            "surfaces": [
                "cli"
            ],
            "input": {
                "contract": "plenora-io-inspect-input-v1",
                "content_types": [
                    "application/json"
                ]
            },
            "output": {
                "contract": "plenora-io-inspect-v1",
                "content_types": [
                    "application/json"
                ]
            },
            "side_effect": "none",
            "controls": {
                "cancellation": True,
                "deadline": True,
                "idempotency_key": False
            }
        },
        {
            "id": "io.layers",
            "version": 1,
            "status": "available",
            "surfaces": [
                "cli"
            ],
            "input": {
                "contract": "plenora-io-layers-input-v1",
                "content_types": [
                    "application/json"
                ]
            },
            "output": {
                "contract": "plenora-io-layers-v1",
                "content_types": [
                    "application/json"
                ]
            },
            "side_effect": "none",
            "controls": {
                "cancellation": True,
                "deadline": True,
                "idempotency_key": False
            }
        },
        {
            "id": "io.read",
            "version": 1,
            "status": "available",
            "surfaces": [
                "cli"
            ],
            "input": {
                "contract": "plenora-io-read-input-v1",
                "content_types": [
                    "application/json"
                ]
            },
            "output": {
                "contract": "plenora-io-read-v1",
                "content_types": [
                    "application/json"
                ]
            },
            "side_effect": "none",
            "controls": {
                "cancellation": True,
                "deadline": True,
                "idempotency_key": False
            }
        },
        {
            "id": "io.write",
            "version": 1,
            "status": "available",
            "surfaces": [
                "cli"
            ],
            "input": {
                "contract": "plenora-io-write-input-v1",
                "content_types": [
                    "application/json"
                ]
            },
            "output": {
                "contract": "plenora-io-write-v1",
                "content_types": [
                    "application/json"
                ]
            },
            "side_effect": "local",
            "controls": {
                "cancellation": True,
                "deadline": True,
                "idempotency_key": False
            }
        },
        {
            "id": "io.convert",
            "version": 1,
            "status": "available",
            "surfaces": [
                "cli"
            ],
            "input": {
                "contract": "plenora-io-convert-input-v1",
                "content_types": [
                    "application/json"
                ]
            },
            "output": {
                "contract": "plenora-io-convert-v1",
                "content_types": [
                    "application/json"
                ]
            },
            "side_effect": "local",
            "controls": {
                "cancellation": True,
                "deadline": True,
                "idempotency_key": False
            }
        }
    ]
}


def artefatto_conforme() -> ArtefattoFinto:
    """Un artefatto che soddisfa ogni requisito del registro."""
    finto = ArtefattoFinto(
        {
            ("catalog",): invocazione(BUSTA_CONFORME + "\n"),
            ("catalog", "--format", "json"): invocazione(BUSTA_CONFORME + "\n"),
            ("inspect", "--format", "json"): invocazione(BUSTA_CONFORME + "\n"),
            ("layers", "--format", "json"): invocazione(BUSTA_CONFORME + "\n"),
            ("read", "--format", "json"): invocazione(BUSTA_CONFORME + "\n"),
            ("convert", "--format", "json"): invocazione(BUSTA_CONFORME + "\n"),
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
                        "result": CAPABILITY_CONFORME,
                    }
                )
                + "\n"
            ),
        }
    )
    finto.consegna_attiva = True
    return finto


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


class SondeDelleCapability(unittest.TestCase):
    """Le quattro regole del documento capability, ciascuna con la controprova.

    Un documento capability che mente e' peggio della sua assenza: chi non lo
    trova cerca altrove, chi lo trova si fida. Le sonde qui sotto provano che le
    quattro regole rifiutano un documento che mente in quattro modi diversi.
    """

    @classmethod
    def setUpClass(cls) -> None:
        trovato = contratti()
        if trovato is None:
            raise unittest.SkipTest(f"nessun checkout dei contratti fra {CANDIDATI}")
        cls.vocabolario = gate.Vocabolario(trovato)

    def artefatto(self, documento) -> ArtefattoFinto:
        busta = {
            "status": "ok",
            "protocol_version": 2,
            "component": "plenora-io-tools",
            "component_version": "4.0.0",
            "contract": "plenora-capabilities-v2",
            "command": "capabilities",
            "result": documento,
        }
        risposte = {("capabilities", "--format", "json"): invocazione(json.dumps(busta) + "\n")}
        for comando in ("catalog", "inspect", "layers", "read", "convert"):
            risposte[(comando, "--format", "json")] = invocazione(BUSTA_CONFORME + "\n")
        return ArtefattoFinto(risposte)

    def documento(self, **modifiche):
        d = copy.deepcopy(CAPABILITY_CONFORME)
        d.update(modifiche)
        return d

    # --- la forma ---------------------------------------------------------

    def test_un_documento_conforme_passa(self) -> None:
        """La controprova positiva: senza, «sempre rosso» sarebbe una difesa."""
        esito = gate.sonda_capability_forma(
            self.artefatto(CAPABILITY_CONFORME), self.vocabolario
        )
        self.assertTrue(esito.passata, esito.dettaglio)

    def test_un_campo_estraneo_e_rosso(self) -> None:
        esito = gate.sonda_capability_forma(
            self.artefatto(self.documento(roadmap=["io.write"])), self.vocabolario
        )
        self.assertFalse(esito.passata)
        self.assertIn("roadmap", esito.dettaglio)

    def test_una_superficie_non_dichiarata_e_rossa(self) -> None:
        """CAP-007: un'operazione non puo' dirsi raggiungibile da una superficie
        che l'artefatto non espone."""
        d = self.documento()
        d["operations"][0]["surfaces"] = ["cli", "runtime"]
        esito = gate.sonda_capability_forma(self.artefatto(d), self.vocabolario)
        self.assertFalse(esito.passata)
        self.assertIn("runtime", esito.dettaglio)

    def test_una_non_disponibile_senza_ragione_e_rossa(self) -> None:
        """CAP-009: chi la legge deve sapere **perche'** non puo' invocarla."""
        d = self.documento()
        d["operations"][0]["status"] = "unavailable"
        esito = gate.sonda_capability_forma(self.artefatto(d), self.vocabolario)
        self.assertFalse(esito.passata)
        self.assertIn("ragione", esito.dettaglio)

    # --- la copertura del catalogo ----------------------------------------

    def test_un_operazione_omessa_e_rossa(self) -> None:
        """Omettere non e' dichiarare non disponibile.

        Chi legge non distingue «non c'e'» da «non l'ho scritta», e il catalogo
        comune dice quali operazioni il profilo pretende.
        """
        d = self.documento()
        d["operations"] = [o for o in d["operations"] if o["id"] != "io.write"]
        esito = gate.sonda_capability_copre_il_catalogo(
            self.artefatto(d), self.vocabolario
        )
        self.assertFalse(esito.passata)
        self.assertIn("io.write", esito.dettaglio)

    def test_un_operazione_inventata_e_rossa(self) -> None:
        """Il verso opposto: il catalogo fissa l'insieme."""
        d = self.documento()
        d["operations"] = d["operations"] + [
            dict(d["operations"][0], id="io.trasmuta")
        ]
        esito = gate.sonda_capability_copre_il_catalogo(
            self.artefatto(d), self.vocabolario
        )
        self.assertFalse(esito.passata)
        self.assertIn("io.trasmuta", esito.dettaglio)

    # --- la matrice dei formati -------------------------------------------

    def test_un_attributo_che_ripete_i_formati_e_rosso(self) -> None:
        """Il profilo assegna quella scoperta a `io.catalog`, e una sola."""
        d = self.documento()
        d["operations"][0]["attributes"] = {"formats": ["shp", "geojson"]}
        esito = gate.sonda_capability_non_duplica_i_formati(
            self.artefatto(d), self.vocabolario
        )
        self.assertFalse(esito.passata)
        self.assertIn("formats", esito.dettaglio)

    def test_un_attributo_di_selezione_non_e_la_matrice(self) -> None:
        """La controprova: gli attributi servono a scegliere l'operazione.

        Senza, «nessun attributo» sarebbe la regola, e il contratto invece li
        ammette -- vieta solo che ripetano cio' che `io.catalog` possiede.
        """
        d = self.documento()
        d["operations"][0]["attributes"] = {"max_concurrent_reads": 4}
        esito = gate.sonda_capability_non_duplica_i_formati(
            self.artefatto(d), self.vocabolario
        )
        self.assertTrue(esito.passata, esito.dettaglio)

    def test_io_catalog_non_disponibile_e_rosso(self) -> None:
        """Delegare a qualcosa che non c'e' non e' delegare."""
        d = self.documento()
        for operazione in d["operations"]:
            if operazione["id"] == "io.catalog":
                operazione["status"] = "unavailable"
                operazione["reason"] = "x"
        esito = gate.sonda_capability_non_duplica_i_formati(
            self.artefatto(d), self.vocabolario
        )
        self.assertFalse(esito.passata)
        self.assertIn("io.catalog", esito.dettaglio)

    # --- la mappatura dei comandi -----------------------------------------

    def test_un_comando_senza_operazione_disponibile_e_rosso(self) -> None:
        """CLI 2.0 §10, ed e' la tensione che il prodotto ha oggi."""
        d = self.documento()
        for operazione in d["operations"]:
            if operazione["id"] == "io.read":
                operazione["status"] = "unavailable"
                operazione["reason"] = "conta i batch invece di consegnarli"
        esito = gate.sonda_ogni_comando_mappa_un_operazione(
            self.artefatto(d), self.vocabolario
        )
        self.assertFalse(esito.passata)
        self.assertIn("io.read", esito.dettaglio)

    def test_il_documento_reale_e_veritiero(self) -> None:
        """Il documento che il registro dichiara `implementato` regge davvero.

        Non e' un doppione del gate: quello gira sul binario, questa sonda gira
        sempre e coglie un documento diventato incoerente prima che qualcuno
        costruisca un artefatto.
        """
        for sonda in (
            gate.sonda_capability_forma,
            gate.sonda_capability_copre_il_catalogo,
            gate.sonda_capability_non_duplica_i_formati,
        ):
            with self.subTest(sonda=sonda.__name__):
                esito = sonda(self.artefatto(CAPABILITY_CONFORME), self.vocabolario)
                self.assertTrue(esito.passata, esito.dettaglio)


class SondeDeiGuasti(unittest.TestCase):
    """Un guasto non e' un requisito non ancora implementato.

    E' la regola che sta sopra le altre tre. Senza, un binario che va in crash
    fa fallire ogni sonda, ogni fallimento e' «atteso» perche' il registro dice
    `non_ancora`, e la CI resta verde su un prodotto che non e' nemmeno
    misurabile -- il falso verde che si autoalimenta, perche' piu' l'artefatto
    e' rotto, meno requisiti sembrano soddisfatti e piu' normale appare.
    """

    @classmethod
    def setUpClass(cls) -> None:
        trovato = contratti()
        if trovato is None:
            raise unittest.SkipTest(f"nessun checkout dei contratti fra {CANDIDATI}")
        cls.contratti = trovato

    def esegui(self, artefatto, stato: str = "non_ancora") -> int:
        registro = json.loads(gate.REGISTRO.read_text(encoding="utf-8"))
        for voce in registro["requisiti"]:
            voce["stato"] = stato
            if stato == "non_ancora":
                voce["perche"] = voce.get("perche") or "sonda finta"
        with tempfile.TemporaryDirectory() as temporanea:
            percorso = pathlib.Path(temporanea) / "registro.json"
            percorso.write_text(json.dumps(registro), encoding="utf-8")
            with mock.patch.object(gate, "REGISTRO", percorso), mock.patch.object(
                gate, "Artefatto", lambda _: artefatto
            ):
                return gate.esegui(self.contratti, pathlib.Path("finto"), False)

    def test_un_crash_e_rosso_anche_su_requisiti_non_ancora(self) -> None:
        morto = ArtefattoFinto(
            {("catalog",): invocazione(exit_code=-11, guasto="terminato dal segnale 11")}
        )
        self.assertEqual(self.esegui(morto), 1)

    def test_un_timeout_e_rosso_anche_su_requisiti_non_ancora(self) -> None:
        bloccato = ArtefattoFinto(
            {("catalog",): invocazione(exit_code=-1, guasto="nessuna risposta entro 60s")}
        )
        self.assertEqual(self.esegui(bloccato), 1)

    def test_un_eseguibile_che_non_parte_e_rosso(self) -> None:
        assente = ArtefattoFinto(
            {("catalog",): invocazione(exit_code=-1, guasto="il processo non e' partito")}
        )
        self.assertEqual(self.esegui(assente), 1)

    def test_un_fallimento_dichiarato_non_e_un_guasto(self) -> None:
        """La controprova: senza, ogni rosso sarebbe un guasto e la seconda
        regola non esisterebbe piu'."""
        vuoto = ArtefattoFinto({})
        self.assertEqual(self.esegui(vuoto), 0)

    def test_il_segnale_di_cancellazione_non_e_un_guasto(self) -> None:
        """`130` e' `SIGINT`, e per il contratto e' la proiezione di `cancelled`.

        Trattare ogni `128 + n` come guasto avrebbe reso non misurabile proprio
        l'esito che CLI 2.0 §9 pretende osservabile.
        """
        self.assertIsNone(gate._guasto_dal_codice(130))

    def test_i_segnali_veri_restano_guasti(self) -> None:
        for codice in (134, 137, 139, 143, 127, 126, -9):
            with self.subTest(codice=codice):
                self.assertIsNotNone(gate._guasto_dal_codice(codice))

    def test_un_codice_ordinario_non_e_un_guasto(self) -> None:
        for codice in (0, 2, 3, 5, 70):
            with self.subTest(codice=codice):
                self.assertIsNone(gate._guasto_dal_codice(codice))

    def test_un_processo_che_non_esiste_produce_un_guasto_vero(self) -> None:
        """Non un finto: `Artefatto` deve classificarlo da se'."""
        artefatto = gate.Artefatto(
            pathlib.Path("/nessun-binario-con-questo-nome-per-la-sonda")
        )
        corsa = artefatto.invoca("catalog")
        self.assertIsNotNone(corsa.guasto)
        self.assertTrue(artefatto.guasti)


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
