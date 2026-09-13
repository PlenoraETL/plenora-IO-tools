"""Quanto codice di **prodotto** c'e', e quanto di prova dentro di esso.

# Perche' due numeri e non uno

«Ridurre il codice» senza un denominatore non e' verificabile, e il
denominatore ovvio -- le righe dei file sorgente -- e' quello sbagliato: in
Rust le prove vivono per convenzione dentro il file che provano, in un
`#[cfg(test)] mod`, e finiscono nel conto come se fossero prodotto. Un crate
che raddoppiasse le proprie prove sembrerebbe raddoppiato.

Qui i due numeri sono separati alla fonte: le righe dentro un modulo
`#[cfg(test)]` sono **prove**, ovunque stiano, e tutto il resto e' prodotto. I
file sotto `crates/*/tests/` sono prove per intero, e non entrano nel
denominatore del budget.

# Perche' un budget e non una fotografia

Un tetto fissato sul valore corrente non impedisce niente: qualunque crescita
lo supera, e allora o si alza il tetto senza pensarci o si smette di
guardarlo. `assurance/registries/code-size-budget.json` porta il numero **e la
ragione del numero**, e alzarlo e' un commit visibile -- che e' tutto cio' che
un budget puo' davvero garantire.

# Che cosa questo conto non dice

Non dice se il codice sia buono, e non e' una misura di complessita'. Dice
quanto ce n'e', il che serve a una domanda sola: se una riga in piu' sia stata
una decisione o un'abitudine.
"""

from __future__ import annotations

import argparse
import json
import pathlib
import re
import sys

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))

import perimetro_dei_sorgenti as perimetro  # noqa: E402

RADICE = pathlib.Path(__file__).resolve().parent.parent
BUDGET = RADICE / "assurance" / "registries" / "code-size-budget.json"

#: I crate del prodotto. `plenora-bench` e `plenora-fuzz` non ci sono: sono
#: strumenti di misura, e contarli gonfierebbe il numero che si vuole tenere
#: basso con codice che non viene spedito. (`plenora-bench` porta anche due
#: binari che la visita dei `mod` non raggiunge, e che fuori dal perimetro non
#: hanno bisogno di essere classificati.)
FUORI_DAL_PRODOTTO = {"plenora-bench", "plenora-fuzz"}

#: L'attributo che apre un modulo di prove. Si accettano le forme con
#: `cfg(test)` e `cfg(all(test, ...))`, che e' come sono scritte quelle
#: condizionate a una feature.
APRE_LE_PROVE = re.compile(r"^\s*#\[cfg\((?:all\()?test\b")


def righe_del_file(percorso: pathlib.Path) -> tuple[int, int]:
    """Righe di prodotto e righe di prova in un sorgente.

    Il conteggio segue le graffe a partire dall'attributo: un modulo di prove
    finisce dove si chiude, e cio' che viene dopo torna a essere prodotto. Un
    file che contenesse due moduli di prove separati li conta entrambi.
    """
    prodotto = 0
    prove = 0
    dentro = False
    graffe = 0
    atteso = False
    for riga in percorso.read_text(encoding="utf-8").splitlines():
        if not dentro and not atteso and APRE_LE_PROVE.match(riga):
            atteso = True
            prove += 1
            continue
        if atteso:
            prove += 1
            graffe += riga.count("{") - riga.count("}")
            if "{" in riga:
                atteso = False
                dentro = True
            continue
        if dentro:
            prove += 1
            graffe += riga.count("{") - riga.count("}")
            if graffe <= 0:
                dentro = False
            continue
        prodotto += 1
    return prodotto, prove


def sorgenti() -> list[pathlib.Path]:
    """I file di **prodotto**, chiesti al classificatore invece che indovinati.

    Finche' le prove stavano dentro il file che provano bastava saltare le
    righe fra `#[cfg(test)] mod` e la sua graffa. Da quando stanno in un file
    loro quel criterio direbbe che il prodotto e' cresciuto di trentaseimila
    righe in un commit che non ne ha scritta una: la classificazione la da'
    `perimetro_dei_sorgenti`, che legge i `mod` e sa che cosa sta sotto
    `#[cfg(test)]`.
    """
    trovati: list[pathlib.Path] = []
    for crate in perimetro.crates(frozenset(FUORI_DAL_PRODOTTO)):
        persi = perimetro.file_non_raggiunti(crate)
        if persi:
            raise SystemExit(
                f"{crate.name}: la visita dei `mod` non raggiunge "
                f"{sorted(p.name for p in persi)}. Un file che non e' ne' prodotto "
                "ne' prove non entra in nessuno dei due conti, e il totale "
                "sarebbe piu' basso del vero senza che nulla lo dica."
            )
        prodotto, _ = perimetro.classifica(crate)
        trovati.extend(prodotto)
    return sorted(trovati)


def misura() -> dict[str, int]:
    prodotto = 0
    prove = 0
    for percorso in sorgenti():
        p, t = righe_del_file(percorso)
        prodotto += p
        prove += t
    return {"prodotto": prodotto, "prove_nei_sorgenti": prove, "file": len(sorgenti())}


def main() -> int:
    argomenti = argparse.ArgumentParser(description=__doc__)
    argomenti.add_argument(
        "--aggiorna",
        action="store_true",
        help="riscrive la misura nel budget senza cambiare il tetto",
    )
    opzioni = argomenti.parse_args()

    corrente = misura()
    registro = json.loads(BUDGET.read_text(encoding="utf-8"))

    if opzioni.aggiorna:
        registro["misura_corrente"] = corrente
        BUDGET.write_bytes(
            (json.dumps(registro, ensure_ascii=False, indent=2) + "\n").encode("utf-8")
        )
        print(f"misura aggiornata: {corrente['prodotto']} righe di prodotto")
        return 0

    tetto = registro["tetto"]["righe_di_prodotto"]
    errori = []
    if corrente["prodotto"] > tetto:
        errori.append(
            f"il prodotto misura {corrente['prodotto']} righe e il budget ne ammette "
            f"{tetto}. Alzare il tetto e' una decisione da scrivere in "
            f"`{BUDGET.relative_to(RADICE).as_posix()}` con la sua ragione, non un "
            "aggiornamento automatico."
        )
    registrata = registro.get("misura_corrente", {})
    if registrata.get("prodotto") != corrente["prodotto"]:
        errori.append(
            f"la misura registrata dice {registrata.get('prodotto')} righe di prodotto e "
            f"il codice ne ha {corrente['prodotto']}. Il registro porta la misura accanto "
            "al tetto perche' si veda quanto margine resta: una misura vecchia lo fa "
            "sembrare piu' largo di quello che e'. Riallinea con `--aggiorna`."
        )

    if errori:
        for uno in errori:
            print(uno, file=sys.stderr)
        return 1

    quota = 100 * corrente["prove_nei_sorgenti"] / (
        corrente["prodotto"] + corrente["prove_nei_sorgenti"]
    )
    print(
        f"dimensione verificata: {corrente['prodotto']} righe di prodotto su un tetto di "
        f"{tetto} ({tetto - corrente['prodotto']} di margine), piu' "
        f"{corrente['prove_nei_sorgenti']} righe di prove dentro i sorgenti "
        f"({quota:.1f}% del totale), su {corrente['file']} file."
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
