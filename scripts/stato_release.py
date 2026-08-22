#!/usr/bin/env python3
"""Il blocco di stato di `docs/RELEASE.md`, reso dalle fonti strutturate.

# Perche' esiste

I numeri dello stato vivevano in due posti: `assurance/current-state.json` e la
prosa di `docs/RELEASE.md`. Il gate verificava che ogni numero della fonte
**comparisse** nel documento — una sottostringa, cercata ovunque. Bastava a
cogliere un numero cambiato in un solo posto, e non bastava a nient'altro: un
campo che la fonte dichiara e il documento non nomina passava, perche' il gate
non sapeva quali campi pretendere; e un numero giusto scritto accanto
all'etichetta sbagliata passava, perche' cercava la cifra e non la coppia.

Qui il documento non riporta i numeri: li **riceve**. Il blocco fra i due
marcatori e' reso da questo modulo e confrontato carattere per carattere, e
l'insieme dei campi resi e' un elenco chiuso — `CAMPI_RICHIESTI` — che una
sonda confronta con cio' che il renderer produce davvero. Aggiungere un campo
alla fonte senza renderlo, o renderlo senza dichiararlo, e' rosso.

# Che cosa questo modulo non fa

Non legge `docs/RELEASE.md`. Legge solo JSON e restituisce testo: la lettura e
la riscrittura del documento stanno in `check_docset.py`, che e' gia' uno dei
due validatori ammessi a leggere il docset. Tenerlo cosi' evita di allargare
quella allowlist chiusa per un modulo che non ha bisogno di aprire il file.

# Due fonti, e la ragione per cui sono due

* `assurance/current-state.json` — le misure e lo stato;
* `assurance/registries/release-contract-current.json` — l'elenco dei blocchi.

Che i numeri dello stato coincidano con le **loro** fonti — l'evidenza della
corsa, il registro di ASSURANCE-N1, `Cargo.toml`, i tag di git — non si
verifica qui: e' l'invariante `stato.fonti-legate` del contratto corrente. Qui
si rende cio' che lo stato dice; li' si verifica che lo stato non se lo sia
scritto da solo.

L'elenco dei blocchi e' del registro perche' e' li' che un blocco nasce e
muore. `current-state.json` ne conserva una copia, e questo modulo pretende che
le due coincidano: una copia che puo' divergere in silenzio dalla propria fonte
e' peggio dell'assenza di copia.
"""

from __future__ import annotations

import argparse
import json
import pathlib
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
STATO = ROOT / "assurance" / "current-state.json"
REGISTRO = ROOT / "assurance" / "registries" / "release-contract-current.json"

APERTURA = "<!-- generato da assurance/current-state.json: inizio -->"
CHIUSURA = "<!-- generato da assurance/current-state.json: fine -->"

# L'elenco chiuso dei campi che il blocco deve rendere. Un campo in piu' nella
# fonte non entra nel documento da solo, e un campo che sparisce dal renderer
# non sparisce in silenzio: le sonde confrontano questo elenco con le chiavi
# che `campi()` produce davvero.
CAMPI_RICHIESTI = (
    "baseline documentale",
    "ultima qualificata",
    "revisione misurata",
    "passi del checkpoint",
    "passi verdi",
    "passi omessi",
    "passi falliti",
    "input di replay",
    "target di replay",
    "crash di replay",
    "target di smoke eseguiti",
    "target di smoke totali",
    "finding di smoke",
    "target in quarantena",
    "copertura LCOV",
    "righe coperte LCOV",
    "righe strumentate LCOV",
    "copertura cargo",
    "soglia di copertura",
    "baseline differenziale",
    "esito differenziale",
    "gruppi ASSURANCE-N1",
    "gruppi ASSURANCE-N1 aperti",
    "blocchi",
    "candidate, versione del manifesto",
    "candidate, revisione del manifesto",
    "candidate, versione del workspace",
    "candidate, qualifica di HEAD",
    "candidate, tag previsto",
    "candidate, tag creato",
    "candidate, revisione del tag",
    "candidate, tag su HEAD",
    "candidate, release_action consentita",
    "release_authorized",
)


def _intero(valore: int) -> str:
    """`36055` diventa `36 055`. Le migliaia si separano con uno spazio."""
    return f"{valore:,}".replace(",", " ")


def _percentuale(valore: float) -> str:
    return f"{valore:.2f}".replace(".", ",") + "%"


def _booleano(valore: bool) -> str:
    return "sì" if valore else "no"


def _letterale(valore: bool) -> str:
    """`release_authorized` si rende con il proprio letterale JSON.

    E' il campo su cui altri gate fanno leva, e un «no» al suo posto
    costringerebbe a cercare la prosa invece del valore.
    """
    return "`true`" if valore else "`false`"


def campi(stato: dict) -> dict[str, str]:
    """Etichetta -> valore reso. Le chiavi sono `CAMPI_RICHIESTI`, in ordine."""
    revisioni = stato["revisioni"]
    misura = stato["ultima_misura"]
    checkpoint = misura["checkpoint"]
    fuzz = misura["fuzz"]
    copertura = misura["copertura"]
    differenziale = misura["diagnostica_differenziale"]
    n1 = stato["aperto"]["assurance_n1"]
    candidate = stato["aperto"]["candidate_release"]

    return {
        "baseline documentale": f"`{revisioni['baseline_documentale']['sha']}`",
        "ultima qualificata": f"`{revisioni['ultima_qualificata']['sha']}`",
        "revisione misurata": f"`{misura['sha']}`",
        "passi del checkpoint": _intero(checkpoint["passi_eseguiti"]),
        "passi verdi": _intero(checkpoint["passi_verdi"]),
        "passi omessi": _intero(checkpoint["passi_omessi"]),
        "passi falliti": _intero(checkpoint["passi_falliti"]),
        "input di replay": _intero(fuzz["replay_input"]),
        "target di replay": _intero(fuzz["replay_target"]),
        "crash di replay": _intero(fuzz["replay_crash"]),
        "target di smoke eseguiti": _intero(fuzz["smoke_target_eseguiti"]),
        "target di smoke totali": _intero(fuzz["smoke_target_totali"]),
        "finding di smoke": _intero(fuzz["smoke_finding"]),
        "target in quarantena": _intero(fuzz["quarantena"]),
        "copertura LCOV": _percentuale(copertura["lcov_percentuale"]),
        "righe coperte LCOV": _intero(copertura["lcov_righe_coperte"]),
        "righe strumentate LCOV": _intero(copertura["lcov_righe_strumentate"]),
        "copertura cargo": _percentuale(copertura["cargo_lines_percentuale"]),
        "soglia di copertura": _percentuale(copertura["soglia"]),
        "baseline differenziale": f"`{differenziale['baseline']}`",
        "esito differenziale": differenziale["esito"],
        "gruppi ASSURANCE-N1": _intero(n1["gruppi_totali"]),
        "gruppi ASSURANCE-N1 aperti": _intero(n1["gruppi_aperti"]),
        "blocchi": _intero(stato["blocchi"]["totale"]),
        "candidate, versione del manifesto": f"`{candidate['versione_manifesto']}`",
        "candidate, revisione del manifesto": f"`{candidate['revisione_manifesto']}`",
        "candidate, versione del workspace": f"`{candidate['versione_workspace']}`",
        "candidate, qualifica di HEAD": _booleano(candidate["qualifica_head"]),
        "candidate, tag previsto": f"`{candidate['tag_previsto']}`",
        "candidate, tag creato": _booleano(candidate["tag_creato"]),
        "candidate, revisione del tag": f"`{candidate['tag_revisione']}`",
        "candidate, tag su HEAD": _booleano(candidate["tag_su_head"]),
        "candidate, release_action consentita": _booleano(
            candidate["release_action_allowed"]
        ),
        "release_authorized": _letterale(stato["release_authorized"]),
    }


def blocchi(stato: dict, registro: dict) -> tuple[list[tuple[str, str]], list[str]]:
    """`[(id, sintesi)]` dei bloccanti, piu' gli errori di coerenza.

    L'elenco e' del **registro**: e' li' che un blocco nasce e muore. Che
    `current-state.json` ne conservi una copia va bene finche' le due
    coincidono; una copia libera di divergere e' peggio dell'assenza di copia.
    """
    errori: list[str] = []
    bloccanti = [
        v for v in registro.get("invarianti", []) if v.get("stato") == "release_blocking"
    ]
    dal_registro = [v["id"] for v in bloccanti]
    dichiarati = stato["blocchi"]["elenco"]

    if dal_registro != dichiarati:
        errori.append(
            "assurance/current-state.json: `blocchi.elenco` non coincide con i "
            f"`release_blocking` del registro. Registro: {dal_registro}; "
            f"stato: {dichiarati}."
        )
    if stato["blocchi"]["totale"] != len(dal_registro):
        errori.append(
            f"assurance/current-state.json: `blocchi.totale` vale "
            f"{stato['blocchi']['totale']}, i bloccanti del registro sono "
            f"{len(dal_registro)}."
        )

    righe: list[tuple[str, str]] = []
    for voce in bloccanti:
        sintesi = voce.get("sintesi")
        if not sintesi:
            errori.append(
                f"{voce['id']}: bloccante senza `sintesi`. La tabella dello "
                "stato ha bisogno di una riga, e scriverla a mano nel documento "
                "creerebbe la seconda verita' che questo blocco elimina."
            )
            sintesi = "—"
        righe.append((voce["id"], sintesi))
    return righe, errori


def blocco(stato: dict, registro: dict) -> tuple[str, list[str]]:
    """Il testo fra i marcatori, marcatori inclusi."""
    valori = campi(stato)
    mancanti = [c for c in CAMPI_RICHIESTI if c not in valori]
    inattesi = [c for c in valori if c not in CAMPI_RICHIESTI]
    errori = [f"campo richiesto «{c}» non reso" for c in mancanti]
    errori += [f"campo «{c}» reso ma non dichiarato in CAMPI_RICHIESTI" for c in inattesi]
    if errori:
        # Senza l'insieme dei campi non c'e' un blocco da rendere: proseguire
        # produrrebbe un testo parziale, e un testo parziale confrontato
        # carattere per carattere darebbe un rosso che parla del confronto
        # invece che della causa.
        return "", errori

    righe_blocchi, errori_blocchi = blocchi(stato, registro)
    errori.extend(errori_blocchi)

    parti = [
        APERTURA,
        "",
        "> Questo blocco è **generato**. La sua autorità è",
        "> [`assurance/current-state.json`](../assurance/current-state.json); modificarlo",
        "> a mano crea la seconda verità che esiste per impedire.",
        "",
        "| Campo | Valore |",
        "|---|---|",
    ]
    parti += [f"| {etichetta} | {valori[etichetta]} |" for etichetta in CAMPI_RICHIESTI]
    parti += [
        "",
        "I blocchi sono l'elenco esatto dei `release_blocking` del",
        "[registro del contratto corrente](../assurance/registries/release-contract-current.json)",
        "— non un riassunto:",
        "",
        "| Blocco | Sintesi |",
        "|---|---|",
    ]
    parti += [f"| `{identita}` | {sintesi} |" for identita, sintesi in righe_blocchi]
    parti += ["", CHIUSURA]
    return "\n".join(parti), errori


def main(argv: list[str] | None = None) -> int:
    argomenti = argparse.ArgumentParser(description=__doc__)
    argomenti.parse_args(argv)
    testo, errori = blocco(
        json.loads(STATO.read_text(encoding="utf-8")),
        json.loads(REGISTRO.read_text(encoding="utf-8")),
    )
    for messaggio in errori:
        print(messaggio, file=sys.stderr)
    if errori:
        return 1
    print(testo)
    return 0


if __name__ == "__main__":
    sys.exit(main())
