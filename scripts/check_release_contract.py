#!/usr/bin/env python3
"""Verifica il **contratto corrente**: invarianti che governano ancora il codice.

# Che cosa questo gate non fa piu'

La versione precedente validava la provenienza della release 1.0.0-rc.2
leggendo **quarantuno documenti Markdown** come database. Quei documenti erano
la cronaca di come si era arrivati a una release gia' emessa, e un gate che
verifica una cronaca non verifica il codice.

Il gate legge ora un solo registro strutturato,
`assurance/registries/release-contract-current.json`, in cui una voce esiste
**solo se un test o un gate corrente la verifica**. Un'affermazione senza
verifica corrente non viene importata come verita': diventa `release_blocking`.

# Due modalita', e il verde dell'una non e' il verde dell'altra

* senza argomenti — il registro e' ben formato **e le prove dei verificati
  sono state eseguite**. Non dice che la release sia autorizzabile, e lo
  stampa;
* `--release` — le condizioni congiunte dell'autorizzazione, fra cui l'assenza
  di voci `release_blocking`.

# Un registro vuoto non e' un contratto soddisfatto

La stesura precedente iterava `invarianti` e non aveva niente da dire su una
lista vuota: **cancellare una voce era il modo piu' rapido di ottenere un
verde**, e nessuna sonda lo vedeva. Il registro dichiara percio' uno schema, e
questo gate un **insieme obbligatorio** di identificatori. Chiudere un blocco
significa portarne lo `stato` a `verified`, mai eliminarne la voce; togliere un
nome da `INVARIANTI_OBBLIGATORI` resta possibile, ed e' il punto: e' un gesto
esplicito che compare in diff, non un'omissione che passa.

# Le condizioni di autorizzazione si eseguono, non si elencano

`autorizzazione_di_release.condizioni` era prosa — cinque frasi che nessuno
lanciava — mentre `--release` guardava soltanto due cose. Le condizioni sono
ora voci strutturate, ciascuna con la propria verifica (un gate da eseguire o
una funzione di questo modulo), e `--release` le esegue **tutte**. Anche il
loro insieme e' obbligatorio: togliere una condizione per ottenere un verde e'
rosso quanto lasciarla fallire.

# Una prova non e' un percorso che esiste

La stesura precedente controllava che il file citato da una prova fosse
presente. Un gate cancellato dal disco la faceva diventare rossa, ed era il
solo modo di accorgersene: uno strumento **presente e rotto**, un test
rinominato, un test sotto `#[ignore]`, un identificatore che non appartiene ad
alcun test — tutti passavano.

Le prove sono percio' tipizzate, e ogni tipo dice come si esegue:

* `test` — crate, configurazione, bersaglio del harness e identificatori
  esatti. Il test viene eseguito, deve comparire **una volta sola**
  nell'elenco del harness e passare;
* `gate` — comando strutturato, deduplicato fra invarianti e realmente
  eseguito: exit diverso da 0 significa invariante non verificato;
* `interna` — funzione di questo gate, eseguita in linea su un artefatto
  strutturato. Serve dove il comando sarebbe questo stesso gate;
* `esterna` — owner, artefatto e stato. Senza evidenza — stato diverso da
  `passed` — un invariante non puo' risultare `verified`: e' bloccante.

I bloccanti **non** si eseguono. Un bloccante puo' avere per definizione un
gate rosso, ed e' cio' che lo rende bloccante; cio' che deve avere e' `manca`,
la condizione che lo chiuderebbe.

E' la stessa separazione di ASSURANCE-N1, e per la stessa ragione: un verde che
significa due cose a seconda di chi lo legge e' la forma di falso verde che
questo repository ha incontrato piu' volte.

Il contratto del protocollo CLI resta verificato **nel merito**, non solo
nominato: `release/cli-protocol-v1.json` e' un artefatto strutturato, e la sua
validazione e' conservata qui.
"""

from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
from pathlib import Path
from typing import Any

sys.path.insert(0, str(Path(__file__).resolve().parent))

# Lo stesso lettore che usa ASSURANCE-N1: due definizioni di «test
# eseguito» divergerebbero, e divergerebbero in silenzio.
from check_assurance_n1_prove import (  # noqa: E402
    BERSAGLI,
    BERSAGLIO_PREDEFINITO,
    CONFIGURAZIONI,
    analizza_uscita,
    comando_test,
)

ROOT = Path(__file__).resolve().parent.parent
REGISTRO = ROOT / "assurance" / "registries" / "release-contract-current.json"
CLI_PROTOCOL_V1 = ROOT / "release" / "cli-protocol-v1.json"

STATI = {"verified", "release_blocking"}
CAMPI = {"id", "superficie", "invariante", "prova", "stato"}

TIPI = {"test", "gate", "interna", "esterna"}
CAMPI_PER_TIPO = {
    "test": {"crate", "configurazione", "test"},
    "gate": {"comando"},
    "interna": {"funzione", "artefatto"},
    "esterna": {"owner", "artefatto", "stato"},
}

# Una prova esterna puo' rendere `verified` solo con questo stato: qualunque
# altro significa che l'evidenza non c'e', e un invariante senza evidenza e'
# bloccante, non vero.
STATO_ESTERNO_VALIDO = "passed"

SCHEMA_VERSIONE = 1

# --- l'insieme obbligatorio degli invarianti --------------------------------
#
# Un invariante che sparisce dal registro sparisce anche dalla tabella di
# `docs/RELEASE.md`: il documento risulta piu' verde senza che nulla sia
# cambiato nel codice. `struttura` non poteva accorgersene, perche' guarda una
# voce alla volta e su una lista vuota non ha voci da guardare.
#
# L'elenco e' chiuso e comprende anche i verificati: anche una garanzia che
# oggi passa, sparendo, smette di essere pretesa senza che nessuno lo decida.
INVARIANTI_OBBLIGATORI = frozenset(
    {
        "wire.error-v1.chiavi",
        "wire.error-v1.quartetto",
        "wire.cli-protocol-v1",
        "wire.catalog-producer",
        "errori.nessun-testo-runtime",
        "errori.static-non-promuovibile",
        "errori.tetto-messaggio",
        "budget.limiti-wkb",
        "budget.nessun-modello-legacy",
        "budget.permit-boundary",
        "difese.prevalidazione-decoder",
        "fallback.registro",
        "provenienza.fork-vendorizzati",
        "provenienza.pin",
        "identita.superficie-pubblica",
        "fuzz.quarantena",
        "copertura.rami-negativi",
        "fuzz.reader-shapefile",
        "fuzz.filegdb",
        "wire.loss-report",
        "release.candidate-non-valida-per-head",
        "lotto.s10",
        "lotto.s11",
        "lotto.s12",
        "sistema.qualifica-cross-component",
    }
)

# --- le condizioni dell'autorizzazione --------------------------------------
CAMPI_CONDIZIONE = {"id", "descrizione", "verifica"}
TIPI_VERIFICA = {"gate", "interna"}

# Congiunte: nessuna implica le altre. L'insieme e' obbligatorio perche'
# altrimenti la via piu' breve al verde sarebbe cancellare la condizione che
# non passa, e il registro continuerebbe a dichiarare cinque condizioni nella
# propria prosa.
CONDIZIONI_OBBLIGATORIE = frozenset(
    {
        "nessun-bloccante",
        "debito-n1-a-zero",
        "decisione-scritta",
        "candidate-coerente",
        "qualifica-cross-component",
    }
)

STATO_CORRENTE = ROOT / "assurance" / "current-state.json"


def _percorsi(valore: Any) -> list[str]:
    """Un artefatto puo' essere uno o piu' percorsi; qui diventa sempre lista."""
    if valore is None:
        return []
    if isinstance(valore, str):
        return [valore]
    return list(valore)


def completezza(documento: dict[str, Any]) -> list[str]:
    """Il registro nel suo insieme: schema, voci obbligatorie, condizioni.

    Separata da `struttura` perche' guarda il **registro intero**. Quella
    verifica una voce alla volta, e su una lista vuota non ha voci da
    verificare: era la ragione per cui un registro svuotato passava.
    """
    errori: list[str] = []

    if documento.get("schema_version") != SCHEMA_VERSIONE:
        errori.append(
            f"schema_version «{documento.get('schema_version')}»: attesa "
            f"{SCHEMA_VERSIONE}. Uno schema non dichiarato rende ogni altra "
            "verifica un'ipotesi sul formato."
        )

    invarianti = documento.get("invarianti")
    if not isinstance(invarianti, list) or not invarianti:
        errori.append(
            "`invarianti` assente o vuoto. Un registro vuoto non e' un "
            "contratto soddisfatto: e' un contratto che non dice niente, e "
            "non ha nulla che possa fallire."
        )
        return errori

    presenti = {voce.get("id") for voce in invarianti if isinstance(voce, dict)}
    for mancante in sorted(INVARIANTI_OBBLIGATORI - presenti):
        errori.append(
            f"{mancante}: invariante obbligatorio assente dal registro. Un "
            "blocco si chiude portandone lo `stato` a `verified`, non "
            "eliminandone la voce."
        )

    errori.extend(_condizioni_ben_formate(documento))
    return errori


def _condizioni_ben_formate(documento: dict[str, Any]) -> list[str]:
    """Le condizioni dell'autorizzazione sono strutturate ed eseguibili."""
    errori: list[str] = []
    autorizzazione = documento.get("autorizzazione_di_release")
    if not isinstance(autorizzazione, dict):
        errori.append(
            "`autorizzazione_di_release` assente. Senza, `--release` non "
            "avrebbe condizioni da verificare, e il conteggio dei bloccanti "
            "sarebbe l'unica."
        )
        return errori

    condizioni = autorizzazione.get("condizioni")
    if not isinstance(condizioni, list) or not condizioni:
        errori.append("`autorizzazione_di_release.condizioni` assente o vuoto")
        return errori

    viste: set[str] = set()
    for condizione in condizioni:
        if not isinstance(condizione, dict):
            errori.append(
                f"condizione «{condizione}» non strutturata. Le condizioni "
                "erano prosa, e la prosa non si esegue."
            )
            continue
        identita = condizione.get("id", "<senza id>")
        mancanti = CAMPI_CONDIZIONE - set(condizione)
        if mancanti:
            errori.append(f"condizione {identita}: campi mancanti {sorted(mancanti)}")
            continue
        if identita in viste:
            errori.append(f"condizione {identita}: voce duplicata")
        viste.add(identita)

        verifica = condizione["verifica"]
        tipo = verifica.get("tipo") if isinstance(verifica, dict) else None
        if tipo not in TIPI_VERIFICA:
            errori.append(
                f"condizione {identita}: tipo di verifica «{tipo}» non "
                f"ammesso; {sorted(TIPI_VERIFICA)}"
            )
            continue
        if tipo == "gate":
            errori.extend(
                f"condizione {identita}: {m}" for m in _argv_valido(verifica.get("comando"))
            )
        else:
            funzione = verifica.get("funzione")
            if not callable(globals().get(funzione)):
                errori.append(
                    f"condizione {identita}: la funzione «{funzione}» non "
                    "esiste in questo gate. Una condizione che nomina una "
                    "verifica inesistente e' una frase, non un controllo."
                )

    for mancante in sorted(CONDIZIONI_OBBLIGATORIE - viste):
        errori.append(
            f"condizione obbligatoria «{mancante}» assente. Le condizioni sono "
            "congiunte: toglierne una e' il modo piu' rapido di ottenere un "
            "verde parziale e chiamarlo verde."
        )
    return errori


def _argv_valido(comando: Any) -> list[str]:
    """Un comando e' un argv non vuoto di stringhe non vuote.

    `["scripts/gate.py --release"]` **non** lo e': e' una riga di shell scritta
    dentro un solo argomento, e `subprocess` la cercherebbe come nome di file.
    """
    if not isinstance(comando, list) or not comando:
        return ["`comando` assente o vuoto"]
    if not all(isinstance(argomento, str) and argomento for argomento in comando):
        return ["`comando` contiene un argomento che non e' una stringa non vuota"]
    if any(carattere.isspace() for carattere in comando[0]):
        return [
            f"`comando` comincia con «{comando[0]}»: e' una riga di shell "
            "scritta dentro un solo argomento, e `subprocess` la cerchera' "
            "come nome di file."
        ]
    return []


def struttura(documento: dict[str, Any]) -> list[str]:
    """Il registro e' ben formato. Non dice che le prove passino."""
    errori: list[str] = []
    visti: set[str] = set()

    for voce in documento.get("invarianti", []):
        identita = voce.get("id", "<senza id>")
        mancanti = CAMPI - set(voce)
        if mancanti:
            errori.append(f"{identita}: campi mancanti {sorted(mancanti)}")
            continue
        if identita in visti:
            errori.append(f"{identita}: voce duplicata")
        visti.add(identita)

        stato = voce["stato"]
        if stato not in STATI:
            errori.append(f"{identita}: stato «{stato}» non ammesso; {sorted(STATI)}")
            continue

        prova = voce["prova"]
        if stato == "release_blocking":
            # Un bloccante puo' avere un gate rosso, o nessuna prova ancora.
            # Cio' che deve avere e' la **condizione di chiusura**: senza,
            # nessuno sa che cosa servirebbe per toglierlo.
            if not voce.get("manca"):
                errori.append(
                    f"{identita}: `release_blocking` senza campo `manca`. Un "
                    "blocco senza la sua condizione di chiusura non si puo' "
                    "chiudere."
                )
            # `sintesi` e' la riga con cui il blocco compare nella tabella di
            # `docs/RELEASE.md`. Vive qui, e non nel documento, perche' un
            # blocco nasce e muore nel registro: scriverla nella prosa
            # creerebbe una seconda verita' libera di divergere.
            if not voce.get("sintesi"):
                errori.append(
                    f"{identita}: `release_blocking` senza campo `sintesi`. La "
                    "tabella dello stato di release ha bisogno di una riga, e "
                    "scriverla a mano nel documento sarebbe la seconda verita' "
                    "che quella tabella esiste per impedire."
                )
            continue

        if not prova:
            errori.append(
                f"{identita}: `verified` senza prova. Un invariante senza "
                "verifica corrente e' `release_blocking`, non una verita'."
            )
            continue
        if not voce.get("invariante"):
            errori.append(f"{identita}: `verified` senza invariante scritto")

        tipo = prova.get("tipo")
        if tipo not in TIPI:
            errori.append(f"{identita}: tipo di prova «{tipo}» non ammesso; {sorted(TIPI)}")
            continue
        senza = CAMPI_PER_TIPO[tipo] - set(prova)
        if senza:
            errori.append(f"{identita}: prova «{tipo}» senza {sorted(senza)}")
            continue

        if tipo == "test":
            if prova["configurazione"] not in CONFIGURAZIONI:
                errori.append(
                    f"{identita}: configurazione «{prova['configurazione']}» non ammessa"
                )
            bersaglio = prova.get("bersaglio", BERSAGLIO_PREDEFINITO)
            if bersaglio not in BERSAGLI:
                errori.append(
                    f"{identita}: bersaglio «{bersaglio}» non ammesso; "
                    f"scegliere fra {sorted(BERSAGLI)}"
                )
        if tipo == "esterna" and prova["stato"] != STATO_ESTERNO_VALIDO:
            errori.append(
                f"{identita}: prova esterna in stato «{prova['stato']}» ma "
                f"invariante `verified`. Senza evidenza — stato "
                f"«{STATO_ESTERNO_VALIDO}» — un invariante e' bloccante, non vero."
            )
        for relativo in _percorsi(prova.get("artefatto")):
            if not (ROOT / relativo).exists():
                errori.append(f"{identita}: artefatto «{relativo}» assente")
    return errori


def _comandi(prova: dict[str, Any]) -> list[list[str]]:
    return [prova["comando"], *prova.get("comandi_aggiuntivi", [])]


def esegui(documento: dict[str, Any]) -> list[str]:
    """Esegue le prove degli invarianti `verified`.

    I bloccanti non si eseguono: possono avere un gate rosso per definizione,
    ed e' cio' che li rende bloccanti. Eseguirli renderebbe il gate rosso su
    una condizione gia' dichiarata, e un rosso che si ripete smette di essere
    letto.
    """
    errori: list[str] = []
    verificati = [v for v in documento.get("invarianti", []) if v.get("stato") == "verified"]

    # --- gate: deduplicati, perche' ripetere una misura non la rende piu' vera
    visti: dict[tuple[str, ...], str] = {}
    for voce in verificati:
        prova = voce["prova"]
        if prova.get("tipo") != "gate":
            continue
        for comando in _comandi(prova):
            chiave = tuple(comando)
            if chiave in visti:
                continue
            visti[chiave] = voce["id"]
            esito = subprocess.run(comando, cwd=ROOT, capture_output=True, text=True, check=False)
            if esito.returncode != 0:
                errori.append(
                    f"{voce['id']}: la prova «{' '.join(comando)}» esce con "
                    f"{esito.returncode}. Un invariante la cui verifica fallisce "
                    "non e' verificato."
                )

    # --- test: eseguiti una volta per coppia, l'identita' deve comparire
    per_coppia: dict[tuple[str, str, str], list[dict[str, Any]]] = {}
    for voce in verificati:
        prova = voce["prova"]
        if prova.get("tipo") != "test":
            continue
        chiave = (
            prova["crate"],
            prova["configurazione"],
            prova.get("bersaglio", BERSAGLIO_PREDEFINITO),
        )
        per_coppia.setdefault(chiave, []).append(voce)

    for (crate, configurazione, bersaglio), voci in per_coppia.items():
        comando = comando_test(crate, configurazione, bersaglio)
        esito = subprocess.run(comando, cwd=ROOT, capture_output=True, text=True, check=False)
        eseguiti, duplicati = analizza_uscita(esito.stdout)
        errori.extend(f"{crate} ({configurazione}, {bersaglio}): {d}" for d in duplicati)
        if not eseguiti:
            errori.append(
                f"{crate} ({configurazione}, {bersaglio}): il harness non ha elencato alcun "
                "test. Un silenzio non e' un verde."
            )
        for voce in voci:
            for identita in voce["prova"]["test"]:
                risultato = eseguiti.get(identita)
                if risultato is None:
                    errori.append(
                        f"{voce['id']}: «{identita}» non compare fra i test "
                        f"eseguiti di {crate} ({configurazione}, {bersaglio}). Un simbolo che "
                        "esiste ma non viene eseguito non verifica niente."
                    )
                elif risultato == "ignored":
                    errori.append(f"{voce['id']}: «{identita}» e' marcato `#[ignore]`")
                elif risultato != "ok":
                    errori.append(f"{voce['id']}: «{identita}» non passa («{risultato}»)")

    # --- interna: la funzione di questo gate, in linea
    for voce in verificati:
        prova = voce["prova"]
        if prova.get("tipo") != "interna":
            continue
        funzione = globals().get(prova["funzione"])
        if funzione is None:
            errori.append(f"{voce['id']}: la funzione «{prova['funzione']}» non esiste")
            continue
        documento_artefatto = json.loads((ROOT / prova["artefatto"]).read_text(encoding="utf-8"))
        errori.extend(f"{voce['id']}: {m}" for m in funzione(documento_artefatto))

    return errori


def debito(documento: dict[str, Any]) -> list[dict[str, Any]]:
    return [v for v in documento.get("invarianti", []) if v.get("stato") == "release_blocking"]


# --- le condizioni dell'autorizzazione, eseguite ----------------------------
#
# Ogni funzione restituisce i **motivi per cui la condizione non e'
# soddisfatta**: lista vuota significa soddisfatta. Nessuna di esse legge il
# campo con cui la condizione si dichiara: le leve stanno nelle fonti.


def _stato_corrente() -> tuple[dict[str, Any] | None, list[str]]:
    if not STATO_CORRENTE.exists():
        return None, [
            f"{STATO_CORRENTE.relative_to(ROOT).as_posix()}: fonte strutturata "
            "dello stato assente"
        ]
    return json.loads(STATO_CORRENTE.read_text(encoding="utf-8")), []


def _git(*argomenti: str) -> str | None:
    """L'uscita di un comando git, o `None` se non ha acquisito."""
    esito = subprocess.run(
        ["git", *argomenti], cwd=ROOT, capture_output=True, text=True, check=False
    )
    if esito.returncode != 0:
        return None
    return esito.stdout.strip()


def versione_workspace() -> str | None:
    """La versione di `[workspace.package]`, letta da `Cargo.toml`."""
    dentro = False
    for riga in (ROOT / "Cargo.toml").read_text(encoding="utf-8").splitlines():
        nuda = riga.strip()
        if nuda.startswith("["):
            dentro = nuda == "[workspace.package]"
            continue
        if dentro:
            trovato = re.match(r'version\s*=\s*"([^"]+)"', nuda)
            if trovato:
                return trovato.group(1)
    return None


def stato_esterno_osservato(relativo: str) -> tuple[str, list[str]]:
    """`(stato, motivi)` di una qualifica esterna, **derivati dal contenuto**.

    Non si legge il campo con cui la voce si dichiara: un `passed` scritto
    accanto a un artefatto che dice `not_run` e' autocertificazione, ed e'
    esattamente la forma di falso verde che il tipo `esterna` esiste per
    escludere.
    """
    percorso = ROOT / relativo
    if not percorso.exists():
        return "assente", [f"{relativo}: artefatto della prova esterna assente"]
    try:
        documento = json.loads(percorso.read_text(encoding="utf-8"))
    except json.JSONDecodeError as errore:
        return "illeggibile", [f"{relativo}: non e' JSON leggibile ({errore})"]

    evidenza = documento.get("evidence")
    dichiarato = evidenza.get("status") if isinstance(evidenza, dict) else None
    if not isinstance(dichiarato, str):
        return "non_derivabile", [
            f"{relativo}: nessun `evidence.status` da cui derivare lo stato. "
            "Senza, l'unica fonte sarebbe il campo che la voce scrive su se "
            "stessa."
        ]

    motivi: list[str] = []
    if documento.get("status") != "satisfied":
        motivi.append(
            f"{relativo}: `status` e' «{documento.get('status')}», non «satisfied»"
        )
    if dichiarato != STATO_ESTERNO_VALIDO:
        motivi.append(f"{relativo}: `evidence.status` e' «{dichiarato}»")
    aperti = documento.get("open_blockers") or []
    if aperti:
        motivi.append(
            f"{relativo}: l'owner dichiara {len(aperti)} blocchi ancora aperti"
        )
    senza_revisione = [
        componente.get("name")
        for componente in documento.get("components", [])
        if not componente.get("revision")
    ]
    if senza_revisione:
        motivi.append(
            f"{relativo}: revisione non fissata per {senza_revisione}. Una "
            "catena qualificata senza le revisioni non dice su che cosa e' "
            "girata."
        )

    if motivi:
        return (
            dichiarato if dichiarato != STATO_ESTERNO_VALIDO else "non_superata"
        ), motivi
    return STATO_ESTERNO_VALIDO, []


def condizione_nessun_bloccante(documento: dict[str, Any]) -> list[str]:
    bloccanti = debito(documento)
    if not bloccanti:
        return []
    motivi = [f"{voce['id']}: {voce['manca']}" for voce in bloccanti]
    motivi.append(
        f"{len(bloccanti)} invarianti su {len(documento['invarianti'])} restano bloccanti"
    )
    return motivi


def condizione_decisione_scritta(documento: dict[str, Any]) -> list[str]:
    """L'unica condizione che nessun gate puo' derivare.

    `release_authorized` e' una decisione scritta, non l'esito automatico di
    caselle verdi: qui si verifica che sia stata presa, non la si calcola.
    """
    stato, errori = _stato_corrente()
    if errori:
        return errori
    if stato.get("release_authorized") is not True:
        return [
            "`release_authorized` non e' true in assurance/current-state.json: "
            "e' una decisione scritta, e non e' stata presa"
        ]
    return []


def condizione_candidate_coerente(documento: dict[str, Any]) -> list[str]:
    """Il manifesto della candidate descrive **la revisione corrente**.

    Versione del workspace, SHA di HEAD e tag non si leggono dallo stato: si
    leggono da `Cargo.toml` e da git. Uno stato che dichiarasse la coerenza
    senza averla renderebbe la condizione una copia di se stessa.
    """
    stato, errori = _stato_corrente()
    if errori:
        return errori
    candidate = stato.get("aperto", {}).get("candidate_release")
    if not isinstance(candidate, dict):
        return ["assurance/current-state.json: `aperto.candidate_release` assente"]

    motivi: list[str] = []
    versione = versione_workspace()
    if versione is None:
        motivi.append("Cargo.toml: versione di `[workspace.package]` non leggibile")
    elif candidate.get("versione_manifesto") != versione:
        motivi.append(
            f"la candidate dichiara la versione «{candidate.get('versione_manifesto')}», "
            f"il workspace e' a «{versione}»"
        )

    head = _git("rev-parse", "HEAD")
    revisione = candidate.get("revisione_manifesto")
    if head is None:
        motivi.append("git: HEAD non leggibile")
    elif not isinstance(revisione, str) or not head.startswith(revisione):
        motivi.append(
            f"la candidate e' legata a «{revisione}», HEAD e' «{(head or '')[:7]}»: "
            "quel manifesto non qualifica il codice corrente"
        )

    atteso = f"v{versione}" if versione else None
    puntato = _git("rev-parse", "--verify", atteso + "^{commit}") if atteso else None
    if atteso is None:
        pass
    elif puntato is None:
        motivi.append(f"il tag «{atteso}» non esiste")
    elif head is not None and puntato != head:
        motivi.append(f"il tag «{atteso}» punta a «{puntato[:7]}», non a HEAD")

    if candidate.get("release_action_allowed") is not True:
        motivi.append("`release_action.allowed` non e' consentita")
    return motivi


def condizione_qualifica_cross_component(documento: dict[str, Any]) -> list[str]:
    """L'esito della catena, letto **dall'artefatto dell'owner esterno**.

    L'artefatto non e' nominato qui: si prende dalla prova dell'invariante che
    lo governa, cosi' un cambio di percorso non lascia questa condizione a
    guardare un file che nessuno aggiorna piu'.
    """
    voce = next(
        (
            v
            for v in documento.get("invarianti", [])
            if v.get("id") == "sistema.qualifica-cross-component"
        ),
        None,
    )
    if voce is None:
        return ["`sistema.qualifica-cross-component` assente dal registro"]
    percorsi = _percorsi((voce.get("prova") or {}).get("artefatto"))
    if not percorsi:
        return [
            "`sistema.qualifica-cross-component`: nessun artefatto da cui "
            "leggere l'esito della qualifica"
        ]
    motivi: list[str] = []
    for relativo in percorsi:
        _, trovati = stato_esterno_osservato(relativo)
        motivi.extend(trovati)
    return motivi


def verifica_condizione(
    condizione: dict[str, Any], documento: dict[str, Any]
) -> list[str]:
    """I motivi per cui una condizione non e' soddisfatta; vuoto se lo e'."""
    verifica = condizione["verifica"]
    if verifica["tipo"] == "gate":
        comando = verifica["comando"]
        esito = subprocess.run(
            comando, cwd=ROOT, capture_output=True, text=True, check=False
        )
        if esito.returncode != 0:
            return [f"«{' '.join(comando)}» esce con {esito.returncode}"]
        return []
    return globals()[verifica["funzione"]](documento)


def validate_cli_protocol_v1(document: dict[str, Any]) -> list[str]:
    errors: list[str] = []
    if document.get("manifest_version") != 1:
        errors.append("cli-protocol-v1: manifest_version inattesa")
    if document.get("component") != "plenora-IO-tools":
        errors.append("cli-protocol-v1: componente inatteso")
    if document.get("protocol_version") != 1:
        errors.append("cli-protocol-v1: protocol_version inattesa")
    if document.get("status") != "frozen_for_1_0":
        errors.append("cli-protocol-v1: stato inatteso")
    if document.get("compatibility_scope") != "cli_json_only":
        errors.append("cli-protocol-v1: superficie non limitata alla CLI JSON")

    rust_api = document.get("rust_api", {})
    if rust_api != {
        "status": "internal_unstable",
        "semver_guarantee": False,
        "crates_publish": False,
        "reason": (
            "R15.4.1 prevede l'estrazione dei tipi di confine da "
            "plenora-io-model; una garanzia pubblica 1.x renderebbe "
            "quell'estrazione una rottura."
        ),
    }:
        errors.append("cli-protocol-v1: stato API Rust inatteso")

    expected_contracts = {
        "error": "plenora-io-error-v1",
        "catalog": "plenora-io-catalog-v1",
        "inspect": "plenora-io-inspect-v1",
        "layers": "plenora-io-layers-v1",
        "read": "plenora-io-read-v1",
        "convert": "plenora-io-convert-v1",
    }
    envelopes = document.get("envelopes", {})
    if set(envelopes) != set(expected_contracts):
        errors.append("cli-protocol-v1: devono essere dichiarate sei buste")
    for name, contract in expected_contracts.items():
        if envelopes.get(name, {}).get("contract") != contract:
            errors.append(f"cli-protocol-v1: contratto inatteso per {name}")

    error_envelope = envelopes.get("error", {})
    if error_envelope.get("optional_error_fields") != ["row_diagnostics"]:
        errors.append("cli-protocol-v1: campi errore opzionali inattesi")
    if error_envelope.get("row_diagnostics_semantics") != {
        "contract": "plenora-row-diagnostics-v1",
        "present_when": (
            "read_row_scoped_rejections_are_observed_or_write_row_scoped_"
            "rejections_are_observed_after_exact_input_total_declaration"
        ),
        "missing_write_input_total": (
            "contract_precondition_error_without_row_diagnostics"
        ),
        "absent_for_other_errors": True,
    }:
        errors.append("cli-protocol-v1: semantica row diagnostics inattesa")
    if error_envelope.get("emitted_error_codes") != [
        "CANCELLED",
        "INVALID_ROW_DIAGNOSTICS",
    ]:
        errors.append("cli-protocol-v1: token errore emessi inattesi")
    if error_envelope.get("exit_codes") != {
        "data_mapping": 2,
        "cancelled_by_caller": 130,
    }:
        errors.append("cli-protocol-v1: exit code additivi inattesi")

    catalog = envelopes.get("catalog", {})
    catalog_fields = ["available", "required_feature"]
    if catalog.get("optional_driver_fields") != catalog_fields:
        errors.append("cli-protocol-v1: campi catalogo additivi opzionali inattesi")
    if catalog.get("current_producer") != {
        "required_driver_fields": catalog_fields,
    }:
        errors.append("cli-protocol-v1: campi obbligatori del producer corrente inattesi")
    if "required_driver_fields" in catalog:
        errors.append("cli-protocol-v1: producer v1 legacy resi incompatibili")
    if catalog.get("driver_field_semantics") != {
        "available": {
            "type": "boolean",
            "true_when": "runtime_probe_satisfies_descriptor",
        },
        "required_feature": {
            "type": ["string", "null"],
            "filegdb": "gdal-backend",
            "other_drivers": None,
        },
    }:
        errors.append("cli-protocol-v1: semantica campi driver inattesa")

    convert = envelopes.get("convert", {})
    required_convert = {
        "conversion_fidelity",
        "read_fidelity",
        "write_fidelity",
        "read_loss",
        "write_loss",
    }
    if not required_convert.issubset(set(convert.get("required_top_level", []))):
        errors.append("cli-protocol-v1: osservabilità convert incompleta")
    if convert.get("forbidden_legacy_fields") != ["loss"]:
        errors.append("cli-protocol-v1: campo legacy loss non vietato")
    return errors


def main(argv: list[str] | None = None) -> int:
    argomenti = argparse.ArgumentParser(description=__doc__)
    argomenti.add_argument(
        "--release",
        action="store_true",
        help="rossa se una sola condizione dell'autorizzazione non e' soddisfatta",
    )
    opzioni = argomenti.parse_args(argv)

    if not REGISTRO.exists():
        print(f"{REGISTRO}: registro assente.", file=sys.stderr)
        return 2
    documento = json.loads(REGISTRO.read_text(encoding="utf-8"))

    # `completezza` viene per prima: se il registro e' vuoto o mutilato, gli
    # errori delle singole voci descriverebbero cio' che resta invece della
    # causa.
    errori = completezza(documento)
    if not errori:
        errori = struttura(documento)
    if not errori:
        errori = esegui(documento)

    for messaggio in errori:
        print(messaggio, file=sys.stderr)
    if errori:
        return 1

    bloccanti = debito(documento)
    totali = len(documento["invarianti"])
    if opzioni.release:
        # Le condizioni sono **congiunte** e vengono eseguite tutte, anche dopo
        # la prima che fallisce: fermarsi darebbe un elenco parziale di cio'
        # che manca, e chi legge crederebbe che il resto sia a posto.
        mancate: list[str] = []
        for condizione in documento["autorizzazione_di_release"]["condizioni"]:
            motivi = verifica_condizione(condizione, documento)
            for motivo in motivi:
                print(f"{condizione['id']}: {motivo}", file=sys.stderr)
            if motivi:
                mancate.append(f"{condizione['id']}: {condizione['descrizione']}")

        if mancate:
            print("", file=sys.stderr)
            print("release non autorizzabile:", file=sys.stderr)
            for motivo in mancate:
                print(f"  - {motivo}", file=sys.stderr)
            print("", file=sys.stderr)
            print(
                "Le condizioni sono congiunte: nessuna implica le altre, e un "
                "verde parziale non e' un verde.",
                file=sys.stderr,
            )
            return 1
        print(
            f"release autorizzabile: {totali} invarianti, nessun blocco, "
            f"{len(documento['autorizzazione_di_release']['condizioni'])} "
            "condizioni verificate."
        )
        return 0

    print(
        f"contratto corrente coerente: {totali} invarianti, "
        f"{totali - len(bloccanti)} verificati, {len(bloccanti)} bloccanti."
    )
    print("  Le prove dei verificati sono state ESEGUITE: gate con exit 0,")
    print("  test elencati dal harness una volta sola e passati. Non dice")
    print("  che la release sia autorizzabile: per quello serve --release.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
