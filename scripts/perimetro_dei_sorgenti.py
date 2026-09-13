"""Quali file `.rs` sono prodotto e quali sono prove, letto dai `mod`.

# Perche' serve

Finche' le prove stavano dentro il file che provano, distinguerle era facile:
si cercava `#[cfg(test)]` **nella stessa riga di testo** che si stava leggendo.
Nove gate lo fanno, ciascuno a modo suo.

Spostare le prove in un file loro rompe tutti e nove nello stesso modo, e in
silenzio: il file nuovo non contiene `#[cfg(test)]`, quindi ogni gate lo legge
come prodotto. Un contatore di righe direbbe che il prodotto e' cresciuto di
trentaseimila righe; un censimento di costruttori d'errore comincerebbe a
contare quelli scritti nelle prove; un registro di messaggi non redatti si
riempirebbe di stringhe di prova.

Questo modulo e' la risposta unica a quella domanda: un file e' **prove**
quando il `mod` che lo dichiara porta `#[cfg(test)]`, ricorsivamente. Una sola
implementazione, e chi la usa non deve piu' indovinare.

# Che cosa non fa

Non e' un parser di Rust. Legge le dichiarazioni `mod nome;` e l'attributo che
le precede, che e' quanto basta per questa domanda, e non prova a capire
`#[path = "..."]`: se un giorno servisse, il gate che ne ha bisogno lo dira'
diventando rosso invece che sbagliando in silenzio -- vedi
[`file_non_raggiunti`].
"""

from __future__ import annotations

import os
import pathlib
import re

RADICE = pathlib.Path(__file__).resolve().parent.parent

#: Una dichiarazione di modulo in un file, con l'attributo che la precede.
#: `(?:pub(?:\([^)]*\))?\s+)?` copre `pub mod` e `pub(crate) mod`.
DICHIARAZIONE = re.compile(
    r"^(?P<attributi>(?:\s*#\[[^\]]*\]\s*\n)*)"
    r"\s*(?:pub(?:\([^)]*\))?\s+)?mod\s+(?P<nome>[a-z_][a-z0-9_]*)\s*;",
    re.M,
)

#: Il marcatore che rende un modulo di prove. `cfg(all(test, ...))` compreso.
E_DI_PROVA = re.compile(r"#\[cfg\((?:all\()?test\b")


def _file_del_modulo(genitore: pathlib.Path, nome: str) -> pathlib.Path | None:
    """Dove vive `mod nome;` dichiarato dentro `genitore`.

    Le due forme dell'edizione 2018: `cartella/nome.rs` e
    `cartella/nome/mod.rs`, dove `cartella` e' la directory del modulo
    genitore -- quella con lo stesso nome del file, per un modulo non radice.
    """
    if genitore.name in {"lib.rs", "main.rs", "mod.rs"}:
        cartella = genitore.parent
    else:
        cartella = genitore.parent / genitore.stem
    for candidato in (cartella / f"{nome}.rs", cartella / nome / "mod.rs"):
        if candidato.is_file():
            return candidato
    return None


def classifica(radice_del_crate: pathlib.Path) -> tuple[set[pathlib.Path], set[pathlib.Path]]:
    """I file di prodotto e quelli di prova di un crate, dalla sua radice.

    La visita parte da `lib.rs` o `main.rs` e segue i `mod`. Un modulo
    dichiarato sotto `#[cfg(test)]` mette **se stesso e tutta la sua
    discendenza** fra le prove: un sottomodulo di un modulo di prove e' prove
    anche se la sua dichiarazione non ripete l'attributo.
    """
    prodotto: set[pathlib.Path] = set()
    prove: set[pathlib.Path] = set()

    radici = [
        r for r in (radice_del_crate / "src" / "lib.rs", radice_del_crate / "src" / "main.rs")
        if r.is_file()
    ]
    da_visitare = [(r, False) for r in radici]
    visti: set[pathlib.Path] = set()

    while da_visitare:
        percorso, in_prova = da_visitare.pop()
        if percorso in visti:
            continue
        visti.add(percorso)
        (prove if in_prova else prodotto).add(percorso)

        try:
            testo = percorso.read_text(encoding="utf-8")
        except OSError:
            continue
        for trovata in DICHIARAZIONE.finditer(testo):
            figlio = _file_del_modulo(percorso, trovata.group("nome"))
            if figlio is None:
                continue
            da_visitare.append(
                (figlio, in_prova or bool(E_DI_PROVA.search(trovata.group("attributi"))))
            )

    return prodotto, prove


def file_non_raggiunti(radice_del_crate: pathlib.Path) -> set[pathlib.Path]:
    """I `.rs` del crate che la visita dei `mod` non ha toccato.

    Se non e' vuoto, la visita ha perso qualcosa -- un `#[path]`, un modulo
    dichiarato in una forma che [`DICHIARAZIONE`] non riconosce -- e il gate
    che la usa deve diventare rosso invece di classificare per difetto. Un file
    perso finirebbe in nessuna delle due categorie, e «nessuna violazione»
    sarebbe vero per omissione.
    """
    prodotto, prove = classifica(radice_del_crate)
    tutti = set()
    for cartella, sottocartelle, nomi in os.walk(radice_del_crate / "src"):
        sottocartelle[:] = [s for s in sottocartelle if s != "__pycache__"]
        tutti.update(pathlib.Path(cartella) / n for n in nomi if n.endswith(".rs"))
    return tutti - prodotto - prove


def crates(esclusi: frozenset[str] = frozenset()) -> list[pathlib.Path]:
    return sorted(
        c
        for c in (RADICE / "crates").iterdir()
        if c.is_dir() and (c / "src").is_dir() and c.name not in esclusi
    )

#: Le directory il cui contenuto e' codice di prova **per posizione**: non ha un
#: modulo `#[cfg(test)]` perche' il file intero lo e'.
PER_POSIZIONE = ("tests", "benches", "examples")

_CACHE: dict[pathlib.Path, set[pathlib.Path]] = {}


def e_codice_di_prova(percorso: pathlib.Path) -> bool:
    """Vero per un file di prove, comunque lo sia diventato.

    Due modi, e nessuno dei due si vede aprendo il file:

    * **per posizione** -- sta sotto `crates/<crate>/tests|benches|examples/`;
    * **per dichiarazione** -- il `mod` che lo porta sta sotto `#[cfg(test)]`,
      direttamente o attraverso un antenato.

    Prima che le prove uscissero dai file di prodotto la seconda domanda non
    esisteva: bastava saltare le righe fra `#[cfg(test)] mod` e la sua graffa.
    Sette gate lo facevano, ciascuno col proprio ritaglio, e spostare le prove
    li avrebbe fatti sbagliare tutti nello stesso verso -- contando come
    prodotto cio' che prodotto non e'. Questa funzione e' la risposta unica.
    """
    percorso = percorso.resolve()

    # La posizione si legge dalla **forma** del percorso e non dalla sua
    # radice: i chiamanti che costruiscono alberi finti passano percorsi che
    # sotto questo repository non stanno, e pretendere la radice reale
    # risponderebbe «non e' prove» a `crates/driver-x/tests/ostili.rs`.
    parti = percorso.parts
    for i, parte in enumerate(parti):
        if parte == "crates" and i + 2 < len(parti) and parti[i + 2] in PER_POSIZIONE:
            return True

    # La dichiarazione invece vuole il crate vero: leggere i `mod` di un albero
    # che non esiste non e' una domanda a cui si possa rispondere.
    try:
        relativo = percorso.relative_to(RADICE).parts
    except ValueError:
        return False
    if len(relativo) < 3 or relativo[0] != "crates":
        return False

    crate = RADICE / "crates" / relativo[1]
    if crate not in _CACHE:
        _, prove = classifica(crate)
        _CACHE[crate] = {p.resolve() for p in prove}
    return percorso in _CACHE[crate]
