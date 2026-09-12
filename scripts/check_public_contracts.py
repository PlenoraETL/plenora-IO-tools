#!/usr/bin/env python3
"""Black-box del profilo pubblico contro il pin di `plenora-contracts`.

# Che cosa verifica, e da dove

Ogni sonda invoca **un processo** e legge cio' che esce da stdout, da stderr e
dal codice d'uscita. Nessuna legge una struttura interna, un campo privato o un
sorgente: e' il confine che `ADOPTION.md` chiede, ed e' l'unico da cui si possa
dire qualcosa su un artefatto invece che sul codice che lo ha prodotto.

Il vocabolario non e' ricopiato qui. Gli enum chiusi delle categorie, delle fasi
e degli effetti remoti si leggono da `schemas/error-v1.schema.json` del checkout
fissato: una copia locale diventerebbe una seconda fonte, e due fonti dello
stesso vocabolario divergono al primo aggiornamento del contratto.

La proiezione dei codici d'uscita e' l'eccezione, e obtorto collo: nel
repository comune esiste solo come tabella Markdown, e un gate non legge la
prosa. Sta quindi in `contracts/proiezione-codici-uscita.json` come dato
dichiarato, con la completezza verificata contro l'enum chiuso -- che prosa
non e'. La duplicazione finisce quando il repository comune la pubblichera' in
forma macchina.

# Che cosa **non** verifica: la provenienza del binario

Il verificatore riceve un percorso e interroga cio' che ci trova. Non giudica da
dove venga, e rifiutare `target/debug` non proverebbe niente: un percorso non e'
un'identita', e chiunque puo' copiare un binario dove vuole.

L'identita' e' un'altra domanda, e ha una risposta vera -- il digest -- che si
verifica in **qualifica**, estraendo il binario dall'archivio che il digest
identifica e ricalcolandolo prima di invocare. Le due fasi hanno due domande:

* in CI, «il codice di questo commit rispetta i contratti?», e si interroga il
  binario appena costruito;
* in qualifica, «l'artefatto che spediamo li rispetta?», e si interroga quello
  estratto dall'archivio.

Un verificatore solo, due fasi. Confonderle darebbe una CI che non puo' girare
prima di una distribuzione, oppure una qualifica che si accontenta di un
binario qualunque.

# L'incrementalita', e perche' ha tre regole invece di una

`contracts/requisiti-pubblici.json` attribuisce a ogni requisito uno stato.

* **`implementato` che fallisce**: rosso. E' cio' che la CI protegge.
* **`non_ancora` che fallisce**: riportato e non bloccante. E' cio' che la CI
  rende visibile, e l'alternativa -- una CI rossa dal primo giorno fino alla
  fine dell'adozione -- e' una CI che nessuno guarda.
* **`non_ancora` che passa**: **rosso**. Sembra perverso, ed e' la regola che
  tiene in piedi le altre due. Un requisito soddisfatto ma dichiarato mancante
  lascia il registro a descrivere un artefatto che non esiste piu': da quel
  momento la prima regola non lo protegge, e una regressione successiva
  passerebbe inosservata. Chi implementa un requisito lo dichiara nello stesso
  commit, e il gate lo obbliga.

Con `--esigente` la seconda regola cade: qualunque `non_ancora` e' rosso. E' la
forma che il gate assume quando si qualifica una candidate, dove la conformita'
parziale non qualifica.

# La regola che sta sopra le altre tre

Crash, timeout e guasti d'ambiente sono **rossi sempre**, qualunque stato il
registro attribuisca al requisito. Un artefatto che muore, che si blocca o che
non parte non sta dicendo «questo requisito non e' ancora implementato»: non sta
dicendo niente, e la seconda regola lo leggerebbe come un piano di lavoro. La
CI resterebbe verde mentre il prodotto non e' nemmeno misurabile, che e' il
peggiore dei falsi verdi -- quello che si autoalimenta, perche' piu' l'artefatto
e' rotto, meno requisiti sembrano soddisfatti, e piu' «normale» appare.
"""

from __future__ import annotations

import argparse
import json
import subprocess
import sys
import tempfile
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Callable

ROOT = Path(__file__).resolve().parents[1]
ADOZIONE = ROOT / "contracts" / "adoption-source.json"
REGISTRO = ROOT / "contracts" / "requisiti-pubblici.json"
PROIEZIONE = ROOT / "contracts" / "proiezione-codici-uscita.json"

STATI = ("implementato", "non_ancora")


@dataclass(frozen=True)
class Esito:
    """Il risultato di una sonda: passa, oppure dice perche' no."""

    passata: bool
    dettaglio: str = ""


# Quanto si aspetta un'invocazione prima di chiamarla bloccata.
#
# Nessun comando di scoperta o di lettura di un file inesistente puo' onestamente
# metterci tanto. Un timeout qui non e' una misura di prestazione: e' la
# differenza fra «l'artefatto ha risposto qualcosa» e «l'artefatto non risponde»,
# e la seconda non e' un requisito non soddisfatto.
ATTESA_MASSIMA = 60


@dataclass
class Invocazione:
    """Cio' che un processo ha prodotto, senza interpretazione.

    `guasto` distingue **non conforme** da **non misurabile**. Un binario che
    risponde male dice qualcosa sul contratto; uno che va in crash, che si blocca
    o che non parte non dice niente, e trattare la sua assenza di risposta come
    un requisito «non ancora implementato» sarebbe leggere un guasto come un
    piano di lavoro.
    """

    argv: list[str]
    exit_code: int
    stdout: bytes
    stderr: bytes
    guasto: str | None = None

    def documento(self) -> dict[str, Any] | None:
        """Il JSON su stdout, o `None` se stdout non ne porta uno."""
        try:
            valore = json.loads(self.stdout.decode("utf-8"))
        except (UnicodeDecodeError, json.JSONDecodeError):
            return None
        return valore if isinstance(valore, dict) else None

    def documento_ovunque(self) -> dict[str, Any] | None:
        """Il JSON su stdout **o** su stderr.

        Serve alle sonde che devono leggere una busta d'errore mentre lo stream
        su cui esce e' ancora sbagliato: senza, ogni sonda sugli assi d'errore
        risulterebbe fallita per la stessa ragione -- lo stream -- e nasconderebbe
        se gli assi siano giusti o no. Un requisito per volta, altrimenti il
        registro dice «diciotto cose rotte» dove ce n'e' una.
        """
        return self.documento() or self._da_stderr()

    def _da_stderr(self) -> dict[str, Any] | None:
        try:
            valore = json.loads(self.stderr.decode("utf-8"))
        except (UnicodeDecodeError, json.JSONDecodeError):
            return None
        return valore if isinstance(valore, dict) else None


class Artefatto:
    """Il binario sotto esame, invocato e non ispezionato."""

    def __init__(self, percorso: Path) -> None:
        self.percorso = percorso
        self._cache: dict[tuple[str, ...], Invocazione] = {}
        self.guasti: list[str] = []

    def invoca(self, *argomenti: str) -> Invocazione:
        """Esegue una volta per combinazione di argomenti.

        La memoria non e' un'ottimizzazione: piu' sonde leggono la **stessa**
        invocazione da angoli diversi -- lo stream, il codice d'uscita, gli assi
        -- e rieseguirla darebbe a ciascuna un processo suo, cioe' la
        possibilita' di misurare esiti diversi e di contraddirsi.
        """
        chiave = argomenti
        if chiave in self._cache:
            return self._cache[chiave]

        argv = [str(self.percorso), *argomenti]
        try:
            completato = subprocess.run(
                argv, capture_output=True, check=False, timeout=ATTESA_MASSIMA
            )
        except subprocess.TimeoutExpired:
            corsa = Invocazione(
                argv=argv,
                exit_code=-1,
                stdout=b"",
                stderr=b"",
                guasto=f"nessuna risposta entro {ATTESA_MASSIMA}s",
            )
        except OSError as errore:
            corsa = Invocazione(
                argv=argv,
                exit_code=-1,
                stdout=b"",
                stderr=b"",
                guasto=f"il processo non e' partito: {errore.strerror or errore}",
            )
        else:
            corsa = Invocazione(
                argv=argv,
                exit_code=completato.returncode,
                stdout=completato.stdout,
                stderr=completato.stderr,
                guasto=_guasto_dal_codice(completato.returncode),
            )
        if corsa.guasto:
            self.guasti.append(f"{' '.join(argomenti) or '(nessun argomento)'}: {corsa.guasto}")
        self._cache[chiave] = corsa
        return corsa


def _guasto_dal_codice(codice: int) -> str | None:
    """Distingue un fallimento **dichiarato** da un processo che e' morto.

    Un codice d'uscita negativo e' un segnale su POSIX; `128 + n` e' la stessa
    cosa vista attraverso una shell. `126` e `127` dicono che l'eseguibile non
    era eseguibile o non c'era: sono guasti d'ambiente, non risposte.

    Le due eccezioni sono deliberate. `130` e' `SIGINT`, e per il contratto e'
    la proiezione di `cancelled`: una cancellazione cooperativa e' una risposta,
    non un guasto. `137` e `143` -- `SIGKILL` e `SIGTERM` -- restano guasti:
    nessuno dei due e' un esito che l'artefatto sceglie.
    """
    if codice < 0:
        return f"terminato dal segnale {-codice}"
    if codice in {126, 127}:
        return f"eseguibile non avviabile (exit {codice})"
    if codice > 128 and codice != 130:
        return f"terminato dal segnale {codice - 128} (exit {codice})"
    return None


class Vocabolario:
    """Gli enum chiusi, letti dallo schema del contratto e non ricopiati."""

    def __init__(self, contratti: Path) -> None:
        #: Il checkout fissato, per le sonde che leggono uno schema invece di
        #: ricopiarne i requisiti.
        self.contracts = contratti
        schema = json.loads(
            (contratti / "schemas" / "error-v1.schema.json").read_text(encoding="utf-8")
        )
        self.categorie = frozenset(self._enum(schema, "category"))
        self.fasi = frozenset(self._enum(schema, "phase"))
        self.effetti = frozenset(self._enum(schema, "remote_effect"))
        self.codici_di_uscita = self._tabella_dei_codici(contratti, self.categorie)
        self.capability = json.loads(
            (contratti / "schemas" / "capabilities-v2.schema.json").read_text(
                encoding="utf-8"
            )
        )
        self.catalogo = json.loads(
            (contratti / "catalogs" / "io-tools-v1.json").read_text(encoding="utf-8")
        )

    @staticmethod
    def _enum(nodo: Any, campo: str) -> list[str]:
        """Il primo `enum` sotto una proprieta' col nome cercato."""
        if isinstance(nodo, dict):
            for chiave, valore in nodo.items():
                if chiave == campo and isinstance(valore, dict) and "enum" in valore:
                    return list(valore["enum"])
                trovato = Vocabolario._enum(valore, campo)
                if trovato:
                    return trovato
        elif isinstance(nodo, list):
            for valore in nodo:
                trovato = Vocabolario._enum(valore, campo)
                if trovato:
                    return trovato
        return []

    @staticmethod
    def _tabella_dei_codici(_: Path, categorie: frozenset[str]) -> dict[str, int]:
        """La proiezione categoria -> codice, da dato dichiarato e non da prosa.

        # Perche' non si legge dal contratto

        La proiezione vive in una tabella Markdown di CLI 2.0, e nel repository
        comune non esiste in forma macchina. La stesura precedente la leggeva da
        li', e `check_docset.py` l'ha rifiutata: un gate che dipende dalla prosa
        dipende da qualcosa che nessuno puo' validare e che si riscrive senza
        accorgersene. Aveva ragione, e il difetto non era teorico -- il parser
        spezzava la cella sulle virgole e la riga del 70 produceva la chiave
        «internal` or an unmapped category», che nessuna categoria avrebbe mai
        trovato: la sonda sulla proiezione sarebbe passata per assenza di
        corrispondenza invece che per conformita'.

        # Che cosa resta a difendere la copia

        Il file dichiarato e' una **seconda fonte**, ed e' il difetto che resta.
        Cio' che lo tiene onesto e' la completezza verificata contro l'enum
        chiuso delle categorie, che viene da `error-v1.schema.json` -- dato
        macchina, non prosa. Una categoria senza proiezione o una proiezione
        senza categoria fanno **rosso**: la copia non puo' restare indietro
        rispetto al contratto in silenzio.

        La duplicazione finisce quando la proiezione sara' pubblicata in forma
        macchina nel repository comune. E' la riga HB2 del piano 4.0.0.
        """
        documento = json.loads(PROIEZIONE.read_text(encoding="utf-8"))
        tabella = {
            nome: int(codice) for nome, codice in documento["proiezione"].items()
        }
        senza_proiezione = sorted(categorie - set(tabella))
        sconosciute = sorted(set(tabella) - categorie)
        if senza_proiezione or sconosciute:
            raise RuntimeError(
                f"«{PROIEZIONE.name}» non coincide con l'enum chiuso delle "
                f"categorie del contratto: senza proiezione {senza_proiezione}, "
                f"non fra le categorie {sconosciute}. La proiezione dichiarata "
                "e' una copia, e una copia che resta indietro senza dirlo e' "
                "peggio di una mancante."
            )
        return tabella


# --------------------------------------------------------------------- sonde
#
# Ogni sonda risponde a **una** domanda. Una sonda che ne verificasse due
# renderebbe illeggibile il registro: «fallita» non direbbe quale.


def _busta_di_successo(artefatto: Artefatto) -> Invocazione:
    """Un'invocazione che riesce, su cui misurare la forma del successo."""
    return artefatto.invoca("catalog")


def _busta_di_errore(artefatto: Artefatto) -> Invocazione:
    """Un'invocazione che fallisce su un percorso di prodotto, non d'uso.

    L'errore d'uso passa da un ramo suo e non attraversa il codice
    dell'operazione: misurarlo direbbe come il CLI rifiuta gli argomenti, non
    come il prodotto riporta un fallimento.
    """
    return artefatto.invoca("inspect", "/nessun-file-esistente-per-la-sonda.shp")


def sonda_quattro_assi(artefatto: Artefatto, vocabolario: Vocabolario) -> Esito:
    corsa = _busta_di_errore(artefatto)
    documento = corsa.documento_ovunque()
    if documento is None:
        return Esito(False, "il fallimento non ha prodotto un documento JSON")
    errore = documento.get("error")
    if not isinstance(errore, dict):
        return Esito(False, "nessun oggetto `error` nella busta")
    mancanti = [a for a in ("category", "phase", "remote_effect", "retry") if a not in errore]
    if mancanti:
        return Esito(False, f"assi mancanti: {mancanti}")
    return Esito(True)


def sonda_categoria(artefatto: Artefatto, vocabolario: Vocabolario) -> Esito:
    documento = _busta_di_errore(artefatto).documento_ovunque() or {}
    valore = (documento.get("error") or {}).get("category")
    if valore in vocabolario.categorie:
        return Esito(True)
    return Esito(False, f"`category` vale «{valore}», fuori dall'enum del contratto")


def sonda_fase(artefatto: Artefatto, vocabolario: Vocabolario) -> Esito:
    documento = _busta_di_errore(artefatto).documento_ovunque() or {}
    valore = (documento.get("error") or {}).get("phase")
    if valore in vocabolario.fasi:
        return Esito(True)
    return Esito(False, f"`phase` vale «{valore}», fuori dall'enum del contratto")


def sonda_retry(artefatto: Artefatto, vocabolario: Vocabolario) -> Esito:
    documento = _busta_di_errore(artefatto).documento_ovunque() or {}
    errore = documento.get("error") or {}
    retry = errore.get("retry")
    if not isinstance(retry, dict):
        return Esito(False, "`retry` assente o non oggetto")
    genere = retry.get("kind")
    if genere == "after" and "delay_ms" not in retry:
        return Esito(False, "`retry.kind: after` senza `delay_ms`")
    if genere != "after" and "delay_ms" in retry:
        return Esito(False, f"`retry.kind: {genere}` con `delay_ms`, che non gli spetta")
    if errore.get("remote_effect") == "unknown" and genere in {"immediate", "after"}:
        return Esito(
            False,
            "`remote_effect: unknown` con ritentativo automatico: ERR-006 lo vieta",
        )
    return Esito(True)


def sonda_successo_un_documento(artefatto: Artefatto, vocabolario: Vocabolario) -> Esito:
    corsa = _busta_di_successo(artefatto)
    if corsa.stdout.count(b"\n") != 1:
        return Esito(False, f"stdout porta {corsa.stdout.count(chr(10).encode())} newline, non una")
    if corsa.documento() is None:
        return Esito(False, "stdout non e' un documento JSON object")
    return Esito(True)


def sonda_successo_stderr_vuoto(artefatto: Artefatto, vocabolario: Vocabolario) -> Esito:
    """Stderr vuoto **su un'invocazione riuscita**, non su una che non parte.

    La stesura precedente guardava il solo stderr, e passava su un artefatto che
    non produceva niente: zero byte su stderr e' vero anche di un binario che
    fallisce prima di scrivere. Era un verde per assenza di output, cioe' la
    stessa famiglia dell'elenco vuoto che passa per assenza di domanda.
    """
    corsa = _busta_di_successo(artefatto)
    if corsa.exit_code != 0 or corsa.documento() is None:
        return Esito(
            False,
            f"l'invocazione non e' riuscita (exit {corsa.exit_code}): non c'e' un "
            "successo su cui misurare lo stream",
        )
    if corsa.stderr:
        return Esito(False, f"stderr porta {len(corsa.stderr)} byte")
    return Esito(True)


def sonda_successo_exit_zero(artefatto: Artefatto, vocabolario: Vocabolario) -> Esito:
    corsa = _busta_di_successo(artefatto)
    documento = corsa.documento() or {}
    if documento.get("status") == "ok" and corsa.exit_code == 0:
        return Esito(True)
    return Esito(
        False, f"`status` vale «{documento.get('status')}» con exit {corsa.exit_code}"
    )


def sonda_errore_exit_non_zero(artefatto: Artefatto, vocabolario: Vocabolario) -> Esito:
    corsa = _busta_di_errore(artefatto)
    documento = corsa.documento_ovunque() or {}
    if documento.get("status") == "error" and corsa.exit_code != 0:
        return Esito(True)
    return Esito(
        False, f"`status` vale «{documento.get('status')}» con exit {corsa.exit_code}"
    )


def sonda_errore_su_stdout(artefatto: Artefatto, vocabolario: Vocabolario) -> Esito:
    corsa = _busta_di_errore(artefatto)
    if corsa.documento() is None:
        return Esito(False, "stdout non porta la busta d'errore")
    if corsa.stderr:
        return Esito(False, f"stderr porta {len(corsa.stderr)} byte")
    return Esito(True)


def sonda_aiuto(artefatto: Artefatto, vocabolario: Vocabolario) -> Esito:
    corsa = artefatto.invoca("--help")
    if corsa.exit_code != 0:
        return Esito(False, f"`--help` esce {corsa.exit_code}")
    return Esito(True)


def sonda_versione_json(artefatto: Artefatto, vocabolario: Vocabolario) -> Esito:
    corsa = artefatto.invoca("--version", "--format", "json")
    if corsa.exit_code != 0:
        return Esito(False, f"`--version --format json` esce {corsa.exit_code}")
    documento = corsa.documento()
    if documento is None:
        return Esito(False, "nessun documento JSON su stdout")
    risultato = documento.get("result")
    if not isinstance(risultato, dict) or "component_version" not in risultato:
        return Esito(False, "`result.component_version` assente")
    return Esito(True)


def sonda_capabilities(artefatto: Artefatto, vocabolario: Vocabolario) -> Esito:
    corsa = artefatto.invoca("capabilities", "--format", "json")
    if corsa.exit_code != 0:
        return Esito(False, f"`capabilities --format json` esce {corsa.exit_code}")
    documento = corsa.documento()
    if documento is None or "operations" not in documento.get("result", documento):
        return Esito(False, "la risposta non porta un elenco di operazioni")
    return Esito(True)


def sonda_formato_json_esplicito(artefatto: Artefatto, vocabolario: Vocabolario) -> Esito:
    corsa = artefatto.invoca("catalog", "--format", "json")
    if corsa.exit_code != 0:
        return Esito(False, f"`catalog --format json` esce {corsa.exit_code}")
    return Esito(True)


def sonda_identita_nella_busta(artefatto: Artefatto, vocabolario: Vocabolario) -> Esito:
    documento = _busta_di_successo(artefatto).documento() or {}
    mancanti = [c for c in ("component", "component_version", "command") if c not in documento]
    if mancanti:
        return Esito(False, f"campi d'identita' mancanti: {mancanti}")
    return Esito(True)


def sonda_dati_dentro_result(artefatto: Artefatto, vocabolario: Vocabolario) -> Esito:
    documento = _busta_di_successo(artefatto).documento() or {}
    ammessi = {
        "status",
        "protocol_version",
        "component",
        "component_version",
        "contract",
        "command",
        "result",
        "error",
    }
    estranei = sorted(set(documento) - ammessi)
    if estranei:
        return Esito(False, f"campi di primo livello estranei alla busta: {estranei}")
    if "result" not in documento:
        return Esito(False, "nessun `result`")
    return Esito(True)


def sonda_una_sola_versione(artefatto: Artefatto, vocabolario: Vocabolario) -> Esito:
    viste = set()
    for corsa in (_busta_di_successo(artefatto), _busta_di_errore(artefatto)):
        documento = corsa.documento_ovunque()
        if documento is not None and "protocol_version" in documento:
            viste.add(documento["protocol_version"])
    if viste == {2}:
        return Esito(True)
    return Esito(False, f"versioni di protocollo osservate: {sorted(viste)}")


def sonda_proiezione_dei_codici(artefatto: Artefatto, vocabolario: Vocabolario) -> Esito:
    corsa = _busta_di_errore(artefatto)
    documento = corsa.documento_ovunque() or {}
    categoria = (documento.get("error") or {}).get("category")
    atteso = vocabolario.codici_di_uscita.get(categoria)
    if atteso is None:
        return Esito(False, f"la tabella del contratto non proietta «{categoria}»")
    if corsa.exit_code != atteso:
        return Esito(
            False,
            f"`{categoria}` esce {corsa.exit_code}, il contratto proietta {atteso}",
        )
    return Esito(True)


def sonda_identificatore_del_componente(artefatto: Artefatto, vocabolario: Vocabolario) -> Esito:
    documento = _busta_di_successo(artefatto).documento() or {}
    valore = documento.get("component")
    if valore == "plenora-io-tools":
        return Esito(True)
    return Esito(False, f"`component` vale «{valore}»")


def sonda_versione_dell_artefatto(artefatto: Artefatto, vocabolario: Vocabolario) -> Esito:
    documento = _busta_di_successo(artefatto).documento() or {}
    if isinstance(documento.get("component_version"), str):
        return Esito(True)
    return Esito(False, "nessuna versione di componente nella busta")


def _documento_capability(artefatto: Artefatto) -> dict[str, Any] | None:
    """Il documento capability, o `None` se il comando non risponde."""
    corsa = artefatto.invoca("capabilities", "--format", "json")
    if corsa.exit_code != 0:
        return None
    busta = corsa.documento()
    risultato = busta.get("result") if isinstance(busta, dict) else None
    return risultato if isinstance(risultato, dict) else None


def sonda_capability_forma(artefatto: Artefatto, vocabolario: Vocabolario) -> Esito:
    """Il documento rispetta lo schema comune, e le due regole che lo rendono utile.

    Non e' una validazione JSON Schema completa -- quella vuole una dipendenza
    che questo gate non ha -- ma le proprieta' che un consumatore usa per
    **selezionare**: i campi obbligatori, nessun campo estraneo, e le due regole
    che il contratto scrive per esteso.

    * `CAP-007`: ogni superficie nominata da un'operazione sta anche fra le
      interfacce. Senza, un'operazione potrebbe dichiararsi raggiungibile da una
      superficie che l'artefatto non espone.
    * `CAP-009`: un'operazione `unavailable` porta una ragione. Senza, chi la
      legge sa che non puo' invocarla e non sa se valga la pena aspettare.
    """
    documento = _documento_capability(artefatto)
    if documento is None:
        return Esito(False, "`capabilities --format json` non rende un documento")

    schema = vocabolario.capability
    mancanti = [c for c in schema["required"] if c not in documento]
    if mancanti:
        return Esito(False, f"campi obbligatori mancanti: {mancanti}")
    estranei = sorted(set(documento) - set(schema["properties"]))
    if estranei:
        return Esito(False, f"campi estranei allo schema: {estranei}")
    if documento.get("schema_version") != 2:
        return Esito(False, f"`schema_version` vale {documento.get('schema_version')}")

    forma = schema["$defs"]["operation"]
    superfici = set(schema["$defs"]["surface"]["enum"])
    interfacce = {
        i.get("kind") for i in documento.get("interfaces", []) if isinstance(i, dict)
    }
    if not interfacce:
        return Esito(False, "nessuna interfaccia dichiarata")

    for operazione in documento.get("operations", []):
        identita = operazione.get("id", "(senza id)")
        mancanti = [c for c in forma["required"] if c not in operazione]
        if mancanti:
            return Esito(False, f"{identita}: campi mancanti {mancanti}")
        estranei = sorted(set(operazione) - set(forma["properties"]))
        if estranei:
            return Esito(False, f"{identita}: campi estranei {estranei}")
        if operazione["status"] not in forma["properties"]["status"]["enum"]:
            return Esito(False, f"{identita}: stato «{operazione['status']}»")
        if operazione["status"] == "unavailable" and not operazione.get("reason"):
            return Esito(
                False,
                f"{identita}: dichiarata non disponibile senza una ragione "
                "(CAP-009): chi la legge non sa perche'",
            )
        for superficie in operazione["surfaces"]:
            if superficie not in superfici:
                return Esito(False, f"{identita}: superficie «{superficie}» inventata")
            if superficie not in interfacce:
                return Esito(
                    False,
                    f"{identita}: dichiara la superficie «{superficie}», che non "
                    "e' fra le interfacce dell'artefatto (CAP-007)",
                )
    return Esito(True)


def sonda_capability_copre_il_catalogo(
    artefatto: Artefatto, vocabolario: Vocabolario
) -> Esito:
    """Ogni operazione del catalogo comune compare, con uno stato dichiarato.

    Ometterne una non e' la stessa cosa che dichiararla non disponibile: chi
    legge non distingue «non c'e'» da «non l'ho scritta», e il catalogo comune
    dice quali operazioni il profilo pretende. Dichiararle tutte, con lo stato
    che ciascuna ha davvero, e' cio' che rende il documento leggibile.
    """
    documento = _documento_capability(artefatto)
    if documento is None:
        return Esito(False, "`capabilities --format json` non rende un documento")
    attese = {o["id"] for o in vocabolario.catalogo["operations"]}
    dichiarate = {o.get("id") for o in documento.get("operations", [])}
    mancanti = sorted(attese - dichiarate)
    if mancanti:
        return Esito(False, f"operazioni del catalogo non dichiarate: {mancanti}")
    inventate = sorted(dichiarate - attese)
    if inventate:
        return Esito(
            False,
            f"operazioni che il catalogo comune non conosce: {inventate}. Il "
            "catalogo fissa l'insieme, e un'operazione in piu' non e' "
            "un'operazione in piu' del profilo.",
        )
    return Esito(True)


def sonda_capability_non_duplica_i_formati(
    artefatto: Artefatto, vocabolario: Vocabolario
) -> Esito:
    """Gli `attributes` non ripetono la matrice dei formati.

    Il profilo assegna a `io.catalog` la scoperta dettagliata, e lo dice per
    esteso: il suo risultato versionato e' la **sola** fonte normativa per
    formati, opzioni, disponibilita', layer, geometria, CRS e fedelta', e gli
    attributi non devono duplicarla. Due fonti dello stesso fatto divergono, e la
    seconda non ha nemmeno uno schema che la governi.
    """
    documento = _documento_capability(artefatto)
    if documento is None:
        return Esito(False, "`capabilities --format json` non rende un documento")

    # `io.catalog` dev'esserci ed essere invocabile: e' la fonte a cui il
    # documento delega, e delegare a qualcosa che non c'e' non e' delegare.
    catalogo = next(
        (o for o in documento.get("operations", []) if o.get("id") == "io.catalog"),
        None,
    )
    if catalogo is None:
        return Esito(False, "`io.catalog` non e' dichiarata")
    if catalogo.get("status") != "available":
        return Esito(
            False,
            "`io.catalog` non e' disponibile: il profilo le assegna la scoperta "
            "dei formati, e senza di lei quel dettaglio non ha una fonte.",
        )

    sospette = ("format", "driver", "crs", "geometry", "layer", "extension", "codec")
    for operazione in documento.get("operations", []):
        attributi = operazione.get("attributes")
        if not isinstance(attributi, dict):
            continue
        for chiave in attributi:
            if any(s in str(chiave).lower() for s in sospette):
                return Esito(
                    False,
                    f"{operazione.get('id')}: l'attributo «{chiave}» ripete la "
                    "matrice che `io.catalog` possiede. Gli attributi "
                    "descrivono la selezione dell'operazione, non le capacita' "
                    "dei formati.",
                )
    return Esito(True)


def sonda_ogni_comando_mappa_un_operazione(
    artefatto: Artefatto, vocabolario: Vocabolario
) -> Esito:
    """Ogni comando che invoca il dominio mappa un'operazione **disponibile**.

    E' CLI 2.0 §10. Oggi non e' vero, ed e' deliberato: `read` esiste come
    comando e `io.read` e' dichiarata non disponibile, perche' conta i batch
    invece di consegnarli. La sonda rende visibile la tensione invece di
    nasconderla: finche' resta, il comando c'e' e l'operazione no.
    """
    documento = _documento_capability(artefatto)
    if documento is None:
        return Esito(False, "`capabilities --format json` non rende un documento")
    disponibili = {
        o["id"] for o in documento.get("operations", []) if o.get("status") == "available"
    }
    # I comandi di dominio, e l'operazione che ciascuno pretende di servire.
    comandi = {
        "catalog": "io.catalog",
        "inspect": "io.inspect",
        "layers": "io.layers",
        "read": "io.read",
        "convert": "io.convert",
    }
    scoperti = []
    for comando, operazione in comandi.items():
        corsa = artefatto.invoca(comando, "--format", "json")
        esiste = corsa.exit_code == 0 or (corsa.documento() or {}).get("command") == comando
        if esiste and operazione not in disponibili:
            scoperti.append(f"`{comando}` -> {operazione}")
    if scoperti:
        return Esito(
            False,
            "comandi che invocano il dominio senza un'operazione disponibile: "
            + ", ".join(scoperti),
        )
    return Esito(True)


def _fixture(nome: str) -> Path:
    return (
        ROOT / "crates" / "plenora-io-cli" / "tests" / "fixtures" / "canoniche" / nome
    )


def _dataset_arrow(artefatto: Artefatto, dove: Path) -> tuple[Path | None, str]:
    """Il dataset Arrow da cui partono le sonde di `write`.

    Prodotto da `read --output` e non costruito qui: e' il contratto fra le due
    operazioni, e un file sintetico proverebbe che `write` legge *qualcosa*
    invece che l'uscita di `read`.
    """
    sorgente = _fixture("canonico.geojson")
    if not sorgente.is_file():
        return None, f"la fixture «{sorgente.name}» non c'e'"
    arrow = dove / "ponte.arrow"
    corsa = artefatto.invoca(
        "read", str(sorgente), "--output", str(arrow), "--format", "json"
    )
    if corsa.exit_code != 0:
        return None, f"la consegna che alimenta la sonda esce {corsa.exit_code}"
    return arrow, ""


def sonda_nomi_dei_contratti(artefatto: Artefatto, vocabolario: Vocabolario) -> Esito:
    """Ogni busta annuncia il nome che il contratto fissato le assegna.

    # Perche' il confronto dev'essere eseguibile

    CLI-2.0 §5 chiama `contract` «operation-specific, namespaced contract
    identifier» e §10 esige che il contratto d'uscita resti equivalente fra le
    superfici. Fino alla 3.0.0 tutte le buste portavano il suffisso `v2`, che e'
    quello del **protocollo**: la coincidenza reggeva finche' nessuno
    confrontava.

    Confrontare a mano e' il modo di sbagliare: due nomi su nove smentiscono la
    somiglianza. `io.read` rende `plenora-io-read-result-v1` e non
    `plenora-io-read-v1` -- il catalogo distingue l'ingresso dall'uscita -- e la
    busta d'errore e' `plenora-error-v1` **senza** `io`, perche' SURF-015 mappa
    i fallimenti sul contratto d'errore comune. Una sostituzione meccanica
    `-v2` -> `-v1` le avrebbe sbagliate entrambe.

    Qui i nomi arrivano da tre fonti del checkout fissato -- il catalogo per le
    sei operazioni, ERRORS-1.0 e CAPABILITY-DISCOVERY-2.0 per i due condivisi --
    e si confrontano con cio' che il binario emette davvero.
    """
    registro_percorso = ROOT / "contracts" / "nomi-dei-contratti.json"
    if not registro_percorso.is_file():
        return Esito(False, "il registro dei nomi non c'e'")
    registro = json.loads(registro_percorso.read_text(encoding="utf-8"))

    catalogo_percorso = vocabolario.contracts / "catalogs" / "io-tools-v1.json"
    if not catalogo_percorso.is_file():
        return Esito(False, "il catalogo io-tools non e' nel pin")
    catalogo = json.loads(catalogo_percorso.read_text(encoding="utf-8"))
    dal_catalogo = {
        voce["id"]: voce["output"]["contract"] for voce in catalogo["operations"]
    }

    # I due contratti condivisi, letti dalla riga «Contract identifier» della
    # loro specifica invece che ricopiati.
    def identificatore(relativo: str) -> str | None:
        percorso = vocabolario.contracts / relativo
        if not percorso.is_file():
            return None
        for riga in percorso.read_text(encoding="utf-8").splitlines():
            if riga.lower().startswith("contract identifier:"):
                return riga.split("`")[1] if "`" in riga else None
        return None

    dalle_specifiche = {
        "errore": identificatore("specs/errors/ERRORS-1.0.md"),
        "capabilities": identificatore(
            "specs/capabilities/CAPABILITY-DISCOVERY-2.0.md"
        ),
    }

    # Che cosa il binario emette, comando per comando.
    emesso: dict[str, str] = {}
    for comando, argomenti in (
        ("catalog", ("catalog", "--format", "json")),
        ("capabilities", ("capabilities", "--format", "json")),
        ("version", ("--version", "--format", "json")),
        # La stessa invocazione che le sonde degli assi d'errore usano: un
        # secondo percorso inventato avrebbe fatto due contratti dove ne basta uno.
        ("errore", ("inspect", "/nessun-file-esistente-per-la-sonda.shp")),
    ):
        corsa = artefatto.invoca(*argomenti)
        documento = corsa.documento() or {}
        nome = documento.get("contract")
        if not isinstance(nome, str):
            return Esito(False, f"`{comando}` non annuncia un contratto")
        emesso[comando] = nome

    problemi: list[str] = []
    for voce in registro["buste"]:
        comando = voce["comando"]
        dichiarato = voce["contratto"]

        atteso = dal_catalogo.get(f"io.{comando}")
        if atteso is None:
            atteso = dalle_specifiche.get(comando)
        if atteso is None:
            if comando != "version":
                problemi.append(f"{comando}: nessuna fonte fissata trovata")
            continue
        if dichiarato != atteso:
            problemi.append(
                f"{comando}: il registro dichiara «{dichiarato}» e la fonte "
                f"fissata dice «{atteso}»"
            )

    for comando, nome in emesso.items():
        dichiarato = next(
            (v["contratto"] for v in registro["buste"] if v["comando"] == comando),
            None,
        )
        if dichiarato is None:
            problemi.append(f"{comando}: il binario lo emette e il registro tace")
        elif nome != dichiarato:
            problemi.append(
                f"{comando}: il binario annuncia «{nome}» e il registro «{dichiarato}»"
            )

    if problemi:
        return Esito(False, "; ".join(problemi))
    return Esito(
        True,
        f"{len(registro['buste'])} nomi confrontati con catalogo e specifiche del pin",
    )


def sonda_diagnostica_di_riga(artefatto: Artefatto, vocabolario: Vocabolario) -> Esito:
    """La diagnostica di riga e' il documento condiviso, e lo dice il pin.

    # Che cosa questa sonda ha stabilito, prima di poter esistere

    La domanda era se `loss` -- la sezione di perdita dei risultati riusciti --
    dovesse conformarsi a `plenora-row-diagnostics-v1`. La risposta e' no, e non
    per comodita': `loss` e' indicizzata per **layer, campo e classe di tipo**,
    e la sua struttura interna non ha proprio un posto dove mettere un indice di
    riga. Non e' diagnostica di riga mancata: e' un'altra cosa -- che cosa il
    formato non ha saputo rappresentare -- e il profilo la governa con l'obbligo
    di riportare perdita e coercizione, che e' soddisfatto altrove.

    Rinominarla sarebbe stato il modo peggiore di chiudere la domanda: due
    documenti con lo stesso nome che rispondono a domande diverse, e chi legge
    che cerca `source_index` dove non ce n'e' mai stato uno.

    # Che cosa la sonda verifica

    Il documento che **e'** row-scoped: `row_diagnostics` nella busta d'errore.
    I campi obbligatori non sono scritti qui -- si leggono dallo schema del
    checkout fissato -- perche' una copia locale dell'elenco si allineerebbe da
    sola il giorno in cui il contratto cambia, ed e' esattamente cio' che il pin
    esiste per impedire.
    """
    schema_percorso = (
        vocabolario.contracts / "schemas" / "row-diagnostics-v1.schema.json"
    )
    if not schema_percorso.is_file():
        return Esito(False, f"lo schema «{schema_percorso.name}» non e' nel pin")
    schema = json.loads(schema_percorso.read_text(encoding="utf-8"))
    obbligatori = schema.get("required") or []
    if not obbligatori:
        return Esito(False, "lo schema del pin non dichiara campi obbligatori")

    sorgente = _fixture_ostile("lettura.kml")
    if sorgente is None:
        return Esito(False, "la fixture ostile che produce diagnostica di riga non c'e'")

    corsa = artefatto.invoca("read", str(sorgente), "--format", "json")
    if corsa.exit_code == 0:
        return Esito(False, "la sorgente ostile non ha prodotto un errore")
    documento = corsa.documento() or {}
    diagnostica = (documento.get("error") or {}).get("row_diagnostics")
    if not isinstance(diagnostica, dict):
        return Esito(
            False, "l'errore su righe rifiutate non porta `row_diagnostics`"
        )

    mancanti = [campo for campo in obbligatori if campo not in diagnostica]
    if mancanti:
        return Esito(
            False,
            "il documento non porta i campi che lo schema del pin dichiara "
            f"obbligatori: {', '.join(sorted(mancanti))}",
        )
    if diagnostica.get("contract") != "plenora-row-diagnostics-v1":
        return Esito(
            False,
            f"si annuncia «{diagnostica.get('contract')}» invece del contratto "
            "condiviso",
        )

    # I due assi chiusi: un valore fuori enum e' un vocabolario inventato.
    proprieta = schema.get("properties") or {}
    for campo in ("index_basis", "completeness"):
        ammessi = (proprieta.get(campo) or {}).get("enum")
        if ammessi and diagnostica.get(campo) not in ammessi:
            return Esito(
                False,
                f"`{campo}` vale «{diagnostica.get(campo)}», fuori dall'enum del pin",
            )

    return Esito(
        True,
        f"conforme ai {len(obbligatori)} campi obbligatori dello schema fissato",
    )


def _fixture_ostile(nome: str) -> Path | None:
    percorso = (
        ROOT / "crates" / "plenora-io-cli" / "tests" / "fixtures" / "ostili" / nome
    )
    return percorso if percorso.is_file() else None


def sonda_write_pubblica(artefatto: Artefatto, vocabolario: Vocabolario) -> Esito:
    """`write` pubblica davvero, e dichiara come e' finita.

    # Che cosa si guarda, e perche' non la sola busta

    Che il file esista, che i byte dichiarati siano quelli sul disco, e che
    l'esito di pubblicazione stia nel vocabolario. E' la stessa lezione di
    `read`: una busta puo' essere giusta mentre il file non c'e', e un gate che
    guardasse solo il riassunto misurerebbe la cosa che non era bastata.
    """
    with tempfile.TemporaryDirectory(prefix="plenora-write-") as temporanea:
        dove = Path(temporanea)
        arrow, problema = _dataset_arrow(artefatto, dove)
        if arrow is None:
            return Esito(False, problema)

        destinazione = dove / "pubblicato.csv"
        corsa = artefatto.invoca(
            "write", str(arrow), str(destinazione), "--to", "csv", "--format", "json"
        )
        if corsa.exit_code != 0:
            return Esito(False, f"`write --to csv` esce {corsa.exit_code}")
        risultato = (corsa.documento() or {}).get("result") or {}
        if not destinazione.is_file():
            return Esito(False, "la busta dice pubblicato e il file non c'e'")
        byte = destinazione.stat().st_size
        if risultato.get("bytes_written") != byte:
            return Esito(
                False,
                f"la busta dichiara {risultato.get('bytes_written')} byte e il "
                f"file ne ha {byte}",
            )
        esito = risultato.get("publish_outcome")
        if esito not in ("published", "published_durability_unconfirmed"):
            return Esito(False, f"esito di pubblicazione fuori vocabolario: {esito}")
        if risultato.get("format") != "csv":
            return Esito(
                False,
                f"il formato dichiarato non e' quello chiesto: {risultato.get('format')}",
            )
    return Esito(True, "pubblica, e i byte dichiarati sono quelli sul disco")


def sonda_write_formato_esplicito(
    artefatto: Artefatto, vocabolario: Vocabolario
) -> Esito:
    """Il formato viene da `--to`, e l'estensione non decide al suo posto.

    # La sola forma in cui la regola si osserva

    Il profilo io-tools vieta di **scegliere** il comportamento di un formato
    analizzando l'estensione quando l'operazione richiede un formato esplicito.
    Quando `--to` e il nome della destinazione concordano le due strade portano
    allo stesso posto e non si distingue niente: qui si fanno disaccordare, e il
    **contenuto** deve smentire il nome.

    E senza `--to` si rifiuta invece di indovinare: un default sarebbe la scelta
    implicita che il profilo esclude.
    """
    with tempfile.TemporaryDirectory(prefix="plenora-esplicito-") as temporanea:
        dove = Path(temporanea)
        arrow, problema = _dataset_arrow(artefatto, dove)
        if arrow is None:
            return Esito(False, problema)

        travestito = dove / "travestito.geojson"
        corsa = artefatto.invoca(
            "write", str(arrow), str(travestito), "--to", "csv", "--format", "json"
        )
        if corsa.exit_code != 0:
            return Esito(
                False,
                "`--to csv` su una destinazione `.geojson` e' stato rifiutato: il "
                "nome ha deciso al posto del formato",
            )
        if not travestito.is_file():
            return Esito(False, "la busta dice pubblicato e il file non c'e'")
        testo = travestito.read_text(encoding="utf-8", errors="replace").lstrip()
        if testo.startswith("{"):
            return Esito(
                False,
                "il contenuto e' JSON: l'estensione ha scelto il driver invece di "
                "`--to`",
            )
        if "," not in testo.splitlines()[0]:
            return Esito(False, "il contenuto non ha l'aspetto di un CSV")

        senza = dove / "senza.csv"
        corsa = artefatto.invoca("write", str(arrow), str(senza), "--format", "json")
        if corsa.exit_code == 0:
            return Esito(False, "`write` senza `--to` e' riuscito indovinando")
        if senza.exists():
            return Esito(False, "il rifiuto senza `--to` ha lasciato una destinazione")

        ignoto = dove / "ignoto.shp"
        corsa = artefatto.invoca(
            "write", str(arrow), str(ignoto), "--to", "shapefile", "--format", "json"
        )
        errore = (corsa.documento() or {}).get("error") or {}
        if errore.get("code") != "UNKNOWN_FORMAT":
            return Esito(
                False,
                "un formato fuori dal catalogo non e' rifiutato come tale: "
                f"{errore.get('code')}",
            )
    return Esito(True, "il contenuto segue `--to`, non il nome della destinazione")


def sonda_vincoli_del_percorso(artefatto: Artefatto, vocabolario: Vocabolario) -> Esito:
    """Ogni driver scrivibile dichiara che cosa pretende dal percorso, e perche'.

    # Che cosa questa sonda difende

    Che un vincolo non rientri in silenzio. Toglierli dove erano convenzione ha
    reso `--to` il selettore vero; se domani un driver ne riaggiungesse uno
    senza dichiararlo, il formato esplicito tornerebbe insufficiente e nessuno
    se ne accorgerebbe fino al primo rifiuto. Qui il vincolo o non c'e', o e'
    dichiarato con la sua ragione e i suoi suffissi.

    Il suffisso di **riconoscimento** e' dichiarato sempre, vincolo o no: e'
    l'altra meta' della verita', e senza di essa togliere il vincolo avrebbe
    tolto anche l'informazione su come l'artefatto viene riletto.
    """
    corsa = artefatto.invoca("catalog", "--format", "json")
    if corsa.exit_code != 0:
        return Esito(False, f"`catalog` esce {corsa.exit_code}")
    drivers = ((corsa.documento() or {}).get("result") or {}).get("drivers")
    if not isinstance(drivers, list) or not drivers:
        return Esito(False, "il catalogo non elenca driver")

    vincolati = []
    for driver in drivers:
        identita = driver.get("id")
        if not driver.get("recognised_suffixes"):
            return Esito(False, f"{identita}: nessun suffisso di riconoscimento")
        capacita = driver.get("write_capabilities")
        if not isinstance(capacita, dict):
            continue
        vincolo = capacita.get("sink_path")
        if not isinstance(vincolo, dict) or "kind" not in vincolo:
            return Esito(False, f"{identita}: vincolo sul percorso non dichiarato")
        if vincolo["kind"] == "required":
            if not vincolo.get("reason"):
                return Esito(False, f"{identita}: vincolo senza ragione")
            if not vincolo.get("suffixes"):
                return Esito(False, f"{identita}: vincolo senza suffissi")
            vincolati.append(identita)
    return Esito(
        True,
        "i vincoli sono dichiarati con la loro ragione"
        + (f"; vincolati: {', '.join(sorted(vincolati))}" if vincolati else ""),
    )


def sonda_write_rollback(artefatto: Artefatto, vocabolario: Vocabolario) -> Esito:
    """Una pubblicazione fallita non lascia byte sulla destinazione.

    Il profilo chiede di distinguere pubblicazione completa, rollback, parziale
    e durabilita' ignota **dove sono osservabili**. Il rollback lo e', e si
    osserva esattamente qui: una destinazione a meta' sarebbe indistinguibile,
    per chi guarda la directory, da una riuscita.
    """
    with tempfile.TemporaryDirectory(prefix="plenora-rollback-") as temporanea:
        dove = Path(temporanea)
        sorgente = _fixture("canonico.gpkg")
        if not sorgente.is_file():
            return Esito(False, f"la fixture «{sorgente.name}» non c'e'")
        arrow = dove / "proiettato.arrow"
        corsa = artefatto.invoca(
            "read",
            str(sorgente),
            "--layer",
            "0",
            "--output",
            str(arrow),
            "--format",
            "json",
        )
        if corsa.exit_code != 0:
            return Esito(False, f"la consegna proiettata esce {corsa.exit_code}")

        # GeoJSON impone WGS84: il dataset proiettato non si puo' esprimere.
        destinazione = dove / "mai_nato.geojson"
        corsa = artefatto.invoca(
            "write", str(arrow), str(destinazione), "--to", "geojson", "--format", "json"
        )
        if corsa.exit_code == 0:
            return Esito(
                False,
                "un sink che non puo' esprimere il CRS ha pubblicato lo stesso",
            )
        errore = (corsa.documento() or {}).get("error") or {}
        if errore.get("category") not in vocabolario.categorie:
            return Esito(False, f"categoria fuori enum: {errore.get('category')}")
        if destinazione.exists():
            return Esito(False, "la pubblicazione fallita ha lasciato una destinazione")
    return Esito(True, "il rifiuto e' tipizzato e non lascia byte")


def sonda_read_consegna(artefatto: Artefatto, vocabolario: Vocabolario) -> Esito:
    """`read --output` consegna byte Arrow, e senza `--output` lo dichiara.

    # Perche' il gate guarda i byte e non la busta

    Fino alla 4.0.0 la busta di `read` diceva righe e batch mentre i dati non
    uscivano affatto: il riassunto era giusto e la consegna non c'era. Guardare
    di nuovo il riassunto sarebbe misurare la stessa cosa che non bastava.

    Qui si guarda il **magic number** del file prodotto -- `ARROW1`, i primi sei
    byte di un IPC file -- e la coerenza fra i byte scritti e quelli dichiarati.
    Non e' una validazione Arrow completa: quella la fanno le sonde Rust, che
    aprono il file con `arrow-ipc`. E' il minimo che distingua «c'e' un file» da
    «c'e' un file Arrow», ed e' verificabile dal confine senza dipendenze.
    """
    with tempfile.TemporaryDirectory(prefix="plenora-consegna-") as temporanea:
        uscita = Path(temporanea) / "consegnato.arrow"
        # La sorgente e' una fixture del repository: il gate gira accanto ad
        # essa, e una sorgente sintetica proverebbe meno.
        sorgente = (
            ROOT
            / "crates"
            / "plenora-io-cli"
            / "tests"
            / "fixtures"
            / "canoniche"
            / "canonico.geojson"
        )
        if not sorgente.is_file():
            return Esito(False, f"la fixture «{sorgente.name}» non c'e'")

        corsa = artefatto.invoca(
            "read", str(sorgente), "--output", str(uscita), "--format", "json"
        )
        if corsa.exit_code != 0:
            return Esito(False, f"`read --output` esce {corsa.exit_code}")
        risultato = (corsa.documento() or {}).get("result") or {}
        consegna = risultato.get("delivered")
        if not isinstance(consegna, dict):
            return Esito(False, "la busta non dichiara una consegna")
        if not uscita.is_file():
            return Esito(False, "la busta dichiara una consegna e il file non c'e'")

        byte = uscita.read_bytes()
        if byte[:6] != b"ARROW1":
            return Esito(
                False,
                f"i byte consegnati non cominciano con `ARROW1`: {byte[:6]!r}",
            )
        if consegna.get("bytes_written") != len(byte):
            return Esito(
                False,
                f"la busta dichiara {consegna.get('bytes_written')} byte e il "
                f"file ne ha {len(byte)}",
            )
        if consegna.get("content_type") != "application/vnd.apache.arrow.file":
            return Esito(
                False, f"content type «{consegna.get('content_type')}»"
            )

    # E senza `--output` la consegna non c'e', e il campo lo dice: un
    # consumatore non deve dedurre dall'assenza di un campo se i dati esistano.
    senza = artefatto.invoca("read", str(sorgente), "--format", "json")
    if senza.exit_code != 0:
        return Esito(False, f"`read` senza consegna esce {senza.exit_code}")
    corpo = (senza.documento() or {}).get("result") or {}
    if "delivered" not in corpo:
        return Esito(False, "senza consegna il campo `delivered` manca del tutto")
    if corpo["delivered"] is not None:
        return Esito(False, "senza `--output` non ci puo' essere una consegna")
    return Esito(True)


SONDE: dict[str, Callable[[Artefatto, Vocabolario], Esito]] = {
    "errore.quattro-assi": sonda_quattro_assi,
    "errore.categoria-nel-vocabolario": sonda_categoria,
    "errore.fase-nel-vocabolario": sonda_fase,
    "errore.retry-coerente": sonda_retry,
    "cli.successo-un-documento": sonda_successo_un_documento,
    "cli.successo-stderr-vuoto": sonda_successo_stderr_vuoto,
    "cli.successo-exit-zero": sonda_successo_exit_zero,
    "cli.errore-exit-non-zero": sonda_errore_exit_non_zero,
    "cli.errore-su-stdout": sonda_errore_su_stdout,
    "cli.aiuto": sonda_aiuto,
    "cli.versione-json": sonda_versione_json,
    "cli.capabilities": sonda_capabilities,
    "read.consegna-arrow": sonda_read_consegna,
    "busta.nomi-dal-contratto-fissato": sonda_nomi_dei_contratti,
    "diagnostica.riga-e-il-contratto-condiviso": sonda_diagnostica_di_riga,
    "write.pubblica": sonda_write_pubblica,
    "write.formato-esplicito": sonda_write_formato_esplicito,
    "write.vincoli-del-percorso": sonda_vincoli_del_percorso,
    "write.rollback-osservabile": sonda_write_rollback,
    "capabilities.forma": sonda_capability_forma,
    "capabilities.copre-il-catalogo": sonda_capability_copre_il_catalogo,
    "capabilities.non-duplica-i-formati": sonda_capability_non_duplica_i_formati,
    "capabilities.ogni-comando-mappa-un-operazione": sonda_ogni_comando_mappa_un_operazione,
    "cli.formato-json-esplicito": sonda_formato_json_esplicito,
    "cli.identita-nella-busta": sonda_identita_nella_busta,
    "cli.dati-dentro-result": sonda_dati_dentro_result,
    "cli.una-sola-versione-di-protocollo": sonda_una_sola_versione,
    "cli.proiezione-dei-codici": sonda_proiezione_dei_codici,
    "superficie.identificatore-del-componente": sonda_identificatore_del_componente,
    "superficie.versione-dell-artefatto": sonda_versione_dell_artefatto,
}


# ------------------------------------------------------------------ il gate


def registro_coerente(registro: dict[str, Any]) -> list[str]:
    """Il registro e' leggibile prima che qualcosa venga invocato.

    Un registro malformato darebbe sonde saltate in silenzio, che e' il modo in
    cui un requisito smette di essere verificato senza che nessuno lo decida.
    """
    errori: list[str] = []
    voci = registro.get("requisiti")
    if not isinstance(voci, list) or not voci:
        return ["`requisiti` assente o vuoto: non c'e' niente da verificare"]

    visti: set[str] = set()
    for voce in voci:
        identita = voce.get("id") if isinstance(voce, dict) else None
        if not isinstance(identita, str) or not identita:
            errori.append(f"requisito senza identificatore leggibile: {voce!r}")
            continue
        if identita in visti:
            errori.append(f"«{identita}» dichiarato due volte")
        visti.add(identita)
        stato = voce.get("stato")
        if stato not in STATI:
            errori.append(f"«{identita}»: stato «{stato}», fuori da {list(STATI)}")
        if stato == "non_ancora" and not str(voce.get("perche", "")).strip():
            errori.append(
                f"«{identita}» e' `non_ancora` senza `perche`. Un requisito "
                "mancante senza la ragione scritta non si distingue da uno "
                "dimenticato, e nessuno sa che cosa cercare per chiuderlo."
            )
        if identita not in SONDE:
            errori.append(
                f"«{identita}» non ha una sonda. Un requisito che nessuno "
                "misura e' una dichiarazione: o si scrive la sonda, o la voce "
                "non appartiene a questo registro."
            )
    orfane = sorted(set(SONDE) - visti)
    if orfane:
        errori.append(
            f"sonde senza voce nel registro: {orfane}. Una sonda che gira e "
            "il cui esito nessuno classifica non protegge e non mostra niente."
        )
    return errori


def esegui(contratti: Path, binario: Path, esigente: bool) -> int:
    adozione = json.loads(ADOZIONE.read_text(encoding="utf-8"))
    atteso = adozione["contracts_source"]["revision"]
    corrente = subprocess.run(
        ["git", "-c", f"safe.directory={contratti.as_posix()}", "rev-parse", "HEAD"],
        cwd=contratti,
        capture_output=True,
        check=True,
        text=True,
    ).stdout.strip()
    if corrente != atteso:
        print(
            f"il checkout dei contratti e' su «{corrente[:12]}», il pin dice "
            f"«{atteso[:12]}». Verificare contro una revisione diversa da quella "
            "fissata direbbe che siamo conformi a qualcosa che non abbiamo "
            "dichiarato.",
            file=sys.stderr,
        )
        return 1

    registro = json.loads(REGISTRO.read_text(encoding="utf-8"))
    guasti = registro_coerente(registro)
    if guasti:
        for messaggio in guasti:
            print(f"registro: {messaggio}", file=sys.stderr)
        return 1

    vocabolario = Vocabolario(contratti)
    artefatto = Artefatto(binario)

    regressioni: list[str] = []
    avanzamenti: list[str] = []
    mancanti: list[str] = []
    protetti = 0

    for voce in registro["requisiti"]:
        identita = voce["id"]
        esito = SONDE[identita](artefatto, vocabolario)
        if voce["stato"] == "implementato":
            if esito.passata:
                protetti += 1
            else:
                regressioni.append(f"{identita} ({voce['regola']}): {esito.dettaglio}")
        elif esito.passata:
            avanzamenti.append(f"{identita} ({voce['regola']})")
        else:
            mancanti.append(f"{identita} ({voce['regola']}): {esito.dettaglio}")

    # I guasti vengono prima di tutto, e valgono anche per i `non_ancora`.
    #
    # Un artefatto che va in crash, che si blocca o che non parte non sta
    # dicendo «questo requisito non e' ancora implementato»: non sta dicendo
    # niente. Classificarlo con lo stato del registro trasformerebbe un binario
    # rotto in un piano di lavoro, e la CI resterebbe verde mentre il prodotto
    # non e' misurabile.
    if artefatto.guasti:
        for riga in artefatto.guasti:
            print(f"GUASTO {riga}", file=sys.stderr)
        print(
            f"l'artefatto non e' misurabile: {len(artefatto.guasti)} invocazioni "
            "sono morte, si sono bloccate o non sono partite. Un guasto non e' "
            "un requisito non ancora implementato, e non si classifica col "
            "registro.",
            file=sys.stderr,
        )
        return 1

    for riga in regressioni:
        print(f"REGRESSIONE {riga}", file=sys.stderr)
    for riga in avanzamenti:
        print(
            f"DA DICHIARARE {riga}: la sonda passa, il registro dice "
            "`non_ancora`. Aggiornare lo stato nello stesso commit che lo ha "
            "implementato: finche' resta `non_ancora`, una regressione "
            "successiva non sarebbe rossa.",
            file=sys.stderr,
        )

    totale = len(registro["requisiti"])
    if regressioni or avanzamenti:
        print(
            f"profilo pubblico: {len(regressioni)} regressioni, "
            f"{len(avanzamenti)} da dichiarare.",
            file=sys.stderr,
        )
        return 1

    if esigente and mancanti:
        for riga in mancanti:
            print(f"NON SODDISFATTO {riga}", file=sys.stderr)
        print(
            f"profilo pubblico non completo: {len(mancanti)} requisiti su "
            f"{totale} non soddisfatti. La conformita' parziale non qualifica.",
            file=sys.stderr,
        )
        return 1

    print(
        f"profilo pubblico: {protetti} requisiti verificati e protetti su {totale}"
        + (f", {len(mancanti)} ancora da implementare." if mancanti else ".")
    )
    for riga in mancanti:
        print(f"  DA IMPLEMENTARE {riga}")
    return 0


def main(argv: list[str] | None = None) -> int:
    argomenti = argparse.ArgumentParser(description=__doc__)
    argomenti.add_argument(
        "--contracts",
        required=True,
        type=Path,
        help="checkout di plenora-contracts, la cui HEAD deve coincidere col pin",
    )
    argomenti.add_argument(
        "--cli",
        required=True,
        type=Path,
        help="percorso del binario da interrogare; la provenienza non e' giudicata qui",
    )
    argomenti.add_argument(
        "--esigente",
        action="store_true",
        help="rossa se un solo requisito applicabile non e' soddisfatto (qualifica)",
    )
    opzioni = argomenti.parse_args(argv)

    if not opzioni.cli.is_file():
        print(f"{opzioni.cli}: binario assente", file=sys.stderr)
        return 2
    if not (opzioni.contracts / ".git").exists():
        print(f"{opzioni.contracts}: non e' un checkout git", file=sys.stderr)
        return 2

    return esegui(opzioni.contracts, opzioni.cli, opzioni.esigente)


if __name__ == "__main__":
    raise SystemExit(main())
