"""Copertura delle sole righe toccate da un intervallo di commit.

# Perche' esiste

La copertura totale di un workspace risponde a una domanda che non e' quella che
serve dopo un refactor. Fra il checkpoint su `107b7b5` e quello su `effc4ab` la
copertura di riga e' scesa di 0,48 punti, e la causa era **meccanica**: i
messaggi curati occupano piu' righe dei `format!` che sostituiscono, e quattro
funzioni sono state estratte. Il denominatore cresce, il numeratore no, e la
percentuale scende senza che nessun ramo sia diventato meno verificato.

Il rischio non e' quella discesa. Il rischio e' che dentro una discesa
meccanica passi inosservata una discesa **semantica**: un ramo nuovo che nessun
test esercita.

Le due si distinguono guardando **solo le righe cambiate**. Una riga di
formattazione in piu' dentro una funzione gia' coperta risulta coperta; un ramo
nuovo mai eseguito no.

# Che cosa non e'

Non e' un gate. Non fallisce sotto una soglia, e non ne ha una: e' una misura
**diagnostica**, che dice dove guardare. Trasformarla in un gate senza averla
prima osservata su qualche checkpoint significherebbe scegliere una soglia
senza sapere che cosa misura.

# Uso

    python3 scripts/coverage_diff.py --lcov lcov.info --base <ref> --head <ref>

`--base` e `--head` sono qualunque cosa `git diff` accetti.
"""

from __future__ import annotations

import argparse
import collections
import pathlib
import re
import subprocess
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent

# Lo stesso perimetro della soglia: se qui entrassero crate che la soglia
# esclude, le due misure parlerebbero di insiemi diversi con lo stesso nome.
ESCLUSI = ("plenora-bench", "plenora-fuzz", "plenora-io-cli")

INTESTAZIONE_DIFF = re.compile(r"^\+\+\+ b/(.+)$")
INTERVALLO = re.compile(r"^@@ -\S+ \+(\d+)(?:,(\d+))? @@")


def righe_cambiate(base: str, head: str) -> dict[str, set[int]]:
    """Righe aggiunte o modificate, per file, fra due revisioni."""
    diff = subprocess.run(
        ["git", "diff", "--unified=0", "--no-color", f"{base}..{head}", "--", "*.rs"],
        cwd=ROOT,
        capture_output=True,
        text=True,
        check=True,
    ).stdout

    per_file: dict[str, set[int]] = collections.defaultdict(set)
    corrente: str | None = None
    for riga in diff.splitlines():
        intestazione = INTESTAZIONE_DIFF.match(riga)
        if intestazione:
            corrente = intestazione.group(1)
            continue
        intervallo = INTERVALLO.match(riga)
        if intervallo and corrente:
            inizio = int(intervallo.group(1))
            quante = int(intervallo.group(2) or "1")
            per_file[corrente].update(range(inizio, inizio + quante))
    return per_file


def copertura_lcov(percorso: pathlib.Path) -> dict[str, dict[int, int]]:
    """Conteggio di esecuzioni per riga, per file, da un report LCOV."""
    per_file: dict[str, dict[int, int]] = {}
    corrente: dict[int, int] | None = None
    for riga in percorso.read_text(encoding="utf-8").splitlines():
        if riga.startswith("SF:"):
            nome = riga[3:].replace("\\", "/")
            # LCOV puo' portare percorsi assoluti: si normalizza sulla radice.
            for prefisso in (str(ROOT).replace("\\", "/") + "/", "/work/"):
                if nome.startswith(prefisso):
                    nome = nome[len(prefisso):]
            corrente = per_file.setdefault(nome, {})
        elif riga.startswith("DA:") and corrente is not None:
            numero, _, conteggio = riga[3:].partition(",")
            try:
                corrente[int(numero)] = int(conteggio.split(",")[0])
            except ValueError:
                continue
        elif riga == "end_of_record":
            corrente = None
    return per_file


def rilevante(percorso: str) -> bool:
    parti = percorso.split("/")
    if len(parti) < 2 or parti[0] != "crates":
        return False
    return parti[1] not in ESCLUSI


def main() -> int:
    argomenti = argparse.ArgumentParser(description=__doc__)
    argomenti.add_argument("--lcov", required=True, type=pathlib.Path)
    argomenti.add_argument("--base", required=True)
    argomenti.add_argument("--head", default="HEAD")
    argomenti.add_argument(
        "--mostra",
        type=int,
        default=20,
        help="quante righe scoperte elencare (0 per tutte)",
    )
    opzioni = argomenti.parse_args()

    if not opzioni.lcov.exists():
        print(f"{opzioni.lcov}: report LCOV assente.", file=sys.stderr)
        return 2

    cambiate = righe_cambiate(opzioni.base, opzioni.head)
    copertura = copertura_lcov(opzioni.lcov)

    coperte = 0
    scoperte: list[tuple[str, int]] = []
    # Una riga cambiata che LCOV non nomina non e' ne' coperta ne' scoperta:
    # non e' codice eseguibile — una parentesi, un commento, una firma. Contarla
    # come scoperta gonfierebbe il problema; contarla come coperta lo
    # nasconderebbe. Sta fuori dalla misura, e il conteggio lo dichiara.
    non_eseguibili = 0

    for percorso, righe in sorted(cambiate.items()):
        if not rilevante(percorso):
            continue
        conteggi = copertura.get(percorso, {})
        for numero in sorted(righe):
            if numero not in conteggi:
                non_eseguibili += 1
            elif conteggi[numero] > 0:
                coperte += 1
            else:
                scoperte.append((percorso, numero))

    eseguibili = coperte + len(scoperte)
    print(f"copertura delle righe cambiate fra {opzioni.base} e {opzioni.head}")
    print(f"  righe cambiate ed eseguibili: {eseguibili}")
    print(f"  coperte:                      {coperte}")
    print(f"  scoperte:                     {len(scoperte)}")
    print(f"  cambiate ma non eseguibili:   {non_eseguibili} (fuori misura)")
    if eseguibili:
        print(f"  percentuale:                  {100 * coperte / eseguibili:.2f}%")
    else:
        print("  percentuale:                  n/d (nessuna riga eseguibile cambiata)")

    if scoperte:
        limite = len(scoperte) if opzioni.mostra == 0 else opzioni.mostra
        print()
        print("righe cambiate e mai eseguite:")
        for percorso, numero in scoperte[:limite]:
            print(f"  {percorso}:{numero}")
        if len(scoperte) > limite:
            print(f"  ... e altre {len(scoperte) - limite}")

    # Diagnostica, non gate: l'esito non dipende dalla percentuale.
    return 0


if __name__ == "__main__":
    sys.exit(main())
