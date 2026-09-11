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
"""

from __future__ import annotations

import argparse
import json
import subprocess
import sys
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


@dataclass
class Invocazione:
    """Cio' che un processo ha prodotto, senza interpretazione."""

    argv: list[str]
    exit_code: int
    stdout: bytes
    stderr: bytes

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

    def invoca(self, *argomenti: str) -> Invocazione:
        """Esegue una volta per combinazione di argomenti.

        La memoria non e' un'ottimizzazione: piu' sonde leggono la **stessa**
        invocazione da angoli diversi -- lo stream, il codice d'uscita, gli assi
        -- e rieseguirla darebbe a ciascuna un processo suo, cioe' la
        possibilita' di misurare esiti diversi e di contraddirsi.
        """
        chiave = argomenti
        if chiave not in self._cache:
            completato = subprocess.run(
                [str(self.percorso), *argomenti], capture_output=True, check=False
            )
            self._cache[chiave] = Invocazione(
                argv=[str(self.percorso), *argomenti],
                exit_code=completato.returncode,
                stdout=completato.stdout,
                stderr=completato.stderr,
            )
        return self._cache[chiave]


class Vocabolario:
    """Gli enum chiusi, letti dallo schema del contratto e non ricopiati."""

    def __init__(self, contratti: Path) -> None:
        schema = json.loads(
            (contratti / "schemas" / "error-v1.schema.json").read_text(encoding="utf-8")
        )
        self.categorie = frozenset(self._enum(schema, "category"))
        self.fasi = frozenset(self._enum(schema, "phase"))
        self.effetti = frozenset(self._enum(schema, "remote_effect"))
        self.codici_di_uscita = self._tabella_dei_codici(contratti, self.categorie)

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
