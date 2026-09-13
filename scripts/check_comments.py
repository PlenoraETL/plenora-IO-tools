"""Due regole sui commenti, e una terza che si e' deciso di non adottare.

# Perche' esiste

Un commento invecchia in due modi diversi, e solo uno dei due e' un difetto.

Il primo e' il **debito anonimo**: `TODO`, `FIXME`, `HACK`, `XXX`. Nomina un
lavoro senza dire chi lo fara' ne' quando, e resta li' finche' qualcuno
riscrive il file per un'altra ragione. Di questi, in questo repository, ce ne
sono **zero**: il gate non serve a ripulire, serve a impedire che il primo
entri.

Il secondo e' la **cronaca di processo**: un commento che parla del lavoro
invece che del codice. Qui la distinzione e' piu' sottile di quanto sembri, e
adottare un elenco di marcatori senza guardarli uno per uno avrebbe cancellato
proprio la prosa che spiega **perche'** il codice e' come e'.

# La decisione, marcatore per marcatore

Sono stati misurati tredici marcatori su tutto l'albero commentabile: 167
occorrenze. Di quelle, la stragrande maggioranza introduce una motivazione
ancora vera -- «la prima stesura la trattava come un difetto, e sbagliava»
dice perche' il codice fa quel che fa, e toglierla lascerebbe il lettore senza
la ragione. Quelle **restano**, ed e' una scelta, non un'omissione.

Quello che il gate vieta e' piu' stretto: il commento che nomina un
**artefatto di processo che il lettore non puo' risolvere**. Un numero di
tranche non sta scritto da nessuna parte; «la regressione della tranche 2» non
si puo' andare a guardare, e nei casi reali il difetto era descritto nella
riga accanto -- il numero non aggiungeva niente e prometteva un riferimento
che non esiste.

Sono vietati quindi:

* `tranche <numero>`, `questa tranche`, `la stessa tranche` -- il lotto di
  lavoro numerato, che nessun documento elenca;
* `pre-fix` -- «prima della correzione», dove la correzione non e' nominata.

E **non** sono vietati, con la ragione:

* `roadmap` -- le occorrenze sono un campo del documento capability, una
  sezione viva di `docs/RELEASE.md` e la motivazione di un'allowlist. Nessuna
  e' cronaca;
* `tranche` da sola -- `docs/ENGINEERING.md` la **definisce** come termine del
  metodo di lavoro («una tranche per commit»), ed e' vocabolario corrente;
* `prima stesura`, `versione precedente`, `da allora`, `qui c'era`,
  `prima era`, `diceva il contrario`, `era stato aggiunto/rimosso`,
  `nello stesso commit` -- introducono una motivazione o una regola ancora
  valida. Un gate che le vietasse otterrebbe commenti piu' corti e meno utili.

# Che cosa il gate non guarda

`vendor/` e `target/`, che non sono nostri; `docs/PIANO-4.0.0.md`, che e' il
documento in cui la decisione **si scrive**; questo file, che la definisce; e
le sue sonde, che costruiscono apposta i commenti da respingere. Tutti e tre
devono poter nominare cio' che vietano, ed e' la stessa ragione per cui
`check_errori_redatti.py` si spoglia prima di contarsi.
"""

from __future__ import annotations

import os
import pathlib
import re
import sys

RADICE = pathlib.Path(__file__).resolve().parent.parent

#: Le estensioni che portano commenti e che scriviamo noi.
ESTENSIONI = {".rs", ".py", ".sh", ".toml", ".md"}

#: Fuori dal perimetro: codice di terzi, prodotti di build, e il documento che
#: decide questa regola.
ESCLUSI_PREFISSO = (
    "vendor/",
    "target/",
    "fuzz/target/",
    ".git/",
    ".plenora-contracts/",
    ".s9-checkpoint/",
)
#: Tre file soli, e ciascuno per la stessa ragione: contengono i marcatori
#: perche' e' il loro mestiere nominarli. Il piano li **decide**, il gate li
#: **definisce**, le sue sonde li **usano come esemplari** -- senza quelle, un
#: gate verde non avrebbe dimostrato di saper dire di no.
ESCLUSI_ESATTI = {
    "docs/PIANO-4.0.0.md",
    "scripts/check_comments.py",
    "scripts/test_check_comments.py",
}

#: Il debito anonimo. `\b` ai due capi perche' `METODO` contiene `TODO`, e un
#: gate che lo contasse accuserebbe una costante di essere un promemoria.
DEBITO = re.compile(r"\b(TODO|FIXME|HACK|XXX)\b")

#: La cronaca che nomina un artefatto di processo irrisolvibile. Vedi la
#: decisione qui sopra per che cosa **non** c'e' in questo elenco.
CRONACA = re.compile(
    r"(tranche\s+\d+|questa\s+tranche|stessa\s+tranche|pre-fix)",
    re.IGNORECASE,
)


#: Le directory che non si scendono affatto. Potarle **durante** la discesa e
#: non dopo non e' un dettaglio di prestazioni: `target/` porta centinaia di
#: migliaia di file, e un gate che li enumeri per scartarli costa minuti invece
#: di millisecondi -- cioe' non gira nel checkpoint.
NON_SI_SCENDE = {".git", "target", "node_modules", "__pycache__", ".plenora-contracts", ".s9-checkpoint", "vendor"}


def file_commentabili() -> list[pathlib.Path]:
    trovati = []
    for cartella, sottocartelle, nomi in os.walk(RADICE):
        sottocartelle[:] = [s for s in sottocartelle if s not in NON_SI_SCENDE]
        for nome in nomi:
            percorso = pathlib.Path(cartella) / nome
            if percorso.suffix not in ESTENSIONI:
                continue
            relativo = percorso.relative_to(RADICE).as_posix()
            if relativo.startswith(ESCLUSI_PREFISSO) or relativo in ESCLUSI_ESATTI:
                continue
            trovati.append(percorso)
    return sorted(trovati)


def violazioni(percorsi: list[pathlib.Path]) -> list[str]:
    trovate: list[str] = []
    for percorso in percorsi:
        relativo = percorso.relative_to(RADICE).as_posix()
        try:
            righe = percorso.read_text(encoding="utf-8").splitlines()
        except (UnicodeDecodeError, OSError) as errore:
            trovate.append(f"{relativo}: illeggibile ({errore})")
            continue
        for numero, riga in enumerate(righe, 1):
            for regola, nome in ((DEBITO, "debito anonimo"), (CRONACA, "cronaca di processo")):
                trovato = regola.search(riga)
                if trovato:
                    trovate.append(
                        f"{relativo}:{numero}: {nome} «{trovato.group(0)}» -- "
                        f"{riga.strip()[:90]}"
                    )
    return trovate


def main() -> int:
    percorsi = file_commentabili()
    trovate = violazioni(percorsi)
    if trovate:
        for una in trovate:
            print(una, file=sys.stderr)
        print(
            f"\n{len(trovate)} commenti fuori regola su {len(percorsi)} file. "
            "Il debito anonimo non entra; la cronaca che nomina un lotto di "
            "lavoro numerato va detta per esteso, perche' il numero non si "
            "puo' risolvere.",
            file=sys.stderr,
        )
        return 1
    print(
        f"commenti verificati: {len(percorsi)} file, nessun debito anonimo e "
        "nessuna cronaca che nomini un artefatto di processo irrisolvibile."
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
