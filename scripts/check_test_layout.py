"""Nessun modulo di prove **dentro** un file di prodotto.

# Che cosa pretende

Che `#[cfg(test)] mod X { … }` non compaia nei sorgenti: le prove stanno in un
file loro, dichiarato `#[cfg(test)] mod X;`. Restano moduli **figli**, quindi
vedono i privati del genitore esattamente come prima e non allargano di una
riga la superficie pubblica del crate. Cambia dove sta il testo, non che cosa
compila.

# Perche'

Tre ragioni, in ordine di quanto sono verificabili.

La prima e' misurabile: senza la separazione, «quanto codice di prodotto c'e'»
non ha risposta. Trentaseimila righe di prove vivevano dentro i file di
prodotto, e qualunque budget sulle righe le contava. `code_size.py` ora
distingue, e puo' farlo perche' la distinzione esiste nel filesystem.

La seconda e' la leggibilita': `driver-shp/src/lib.rs` misurava novemila
righe, di cui piu' della meta' prove. Chi cerca il codice di produzione
scorreva prove; chi cerca una prova scorreva produzione.

La terza e' la piu' debole e va detta come tale: la convenzione Rust ammette
entrambe le forme, e nessun contratto fissato pretende questa. E' una scelta
di questo repository, non un requisito di conformita'.

# Che cosa **non** pretende

Che non ci sia `#[cfg(test)]` nei file di prodotto. Cinquantasette attributi
restano, e sono su **singoli elementi** -- un aiutante, una fixture, un
`thread_local!` che serve a una sonda, un failpoint -- che non formano un
modulo. Spostarli vorrebbe dire renderli visibili al crate per poterli
importare, cioe' allargare una superficie interna per far contento un gate. Il
gate conta quelli che restano e pretende che il numero sia dichiarato: crescono
solo per decisione.
"""

from __future__ import annotations

import json
import pathlib
import re
import sys

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))

import perimetro_dei_sorgenti as perimetro  # noqa: E402

RADICE = pathlib.Path(__file__).resolve().parent.parent
REGISTRO = RADICE / "assurance" / "registries" / "test-layout.json"

#: `plenora-fuzz` e `plenora-bench` sono attrezzaggio: le loro prove, se ce ne
#: fossero, non sono il codice che si spedisce.
FUORI = frozenset({"plenora-fuzz", "plenora-bench"})

#: Un modulo di prove **inline**: l'attributo, poi `mod nome {` con la graffa.
#: La forma che il gate vuole e' `mod nome;`, senza graffa.
MODULO_INLINE = re.compile(
    r"^#\[cfg\((?:all\()?test\b[^\]]*\]\n(?:#\[[^\]]*\]\n)*mod\s+([a-z_][a-z0-9_]*)\s*\{",
    re.M,
)

#: Un `#[cfg(test)]` che **non** dichiara un modulo di prove. Serve a contare
#: quelli che restano su singoli elementi: la dichiarazione `mod tests;` e'
#: precisamente la forma che il gate vuole, e contarla fra i superstiti direbbe
#: che ogni file separato e' un residuo.
ATTRIBUTO_SU_UN_ELEMENTO = re.compile(
    r"^\s*#\[cfg\((?:all\()?test\b[^\]]*\]\n"
    r"(?:\s*#\[[^\]]*\]\n)*"
    r"\s*(?!mod\s+[a-z_][a-z0-9_]*\s*;)",
    re.M,
)


def scansiona() -> tuple[list[str], int, int]:
    """`(violazioni, attributi superstiti, file di prove)`."""
    violazioni: list[str] = []
    superstiti = 0
    file_di_prove = 0

    for crate in perimetro.crates(FUORI):
        persi = perimetro.file_non_raggiunti(crate)
        if persi:
            violazioni.append(
                f"{crate.name}: la visita dei `mod` non raggiunge "
                f"{sorted(p.name for p in persi)}. Un file che non e' ne' prodotto "
                "ne' prove sfugge a questo gate e a quello delle dimensioni."
            )
            continue
        prodotto, prove = perimetro.classifica(crate)
        file_di_prove += len(prove)
        for percorso in sorted(prodotto):
            testo = percorso.read_text(encoding="utf-8")
            relativo = percorso.relative_to(RADICE).as_posix()
            for nome in MODULO_INLINE.findall(testo):
                violazioni.append(
                    f"{relativo}: `mod {nome}` e' un modulo di prove **dentro** un "
                    "file di prodotto. Spostalo in un file suo e dichiaralo "
                    f"`#[cfg(test)] mod {nome};`: resta un modulo figlio e vede gli "
                    "stessi privati."
                )
            superstiti += len(ATTRIBUTO_SU_UN_ELEMENTO.findall(testo))

    return violazioni, superstiti, file_di_prove


def main() -> int:
    violazioni, superstiti, file_di_prove = scansiona()
    registro = json.loads(REGISTRO.read_text(encoding="utf-8"))
    atteso = registro["attributi_su_singoli_elementi"]["quanti"]

    if superstiti != atteso:
        violazioni.append(
            f"gli `#[cfg(test)]` su singoli elementi dentro file di prodotto sono "
            f"{superstiti}, e il registro ne dichiara {atteso}. Non sono vietati -- "
            "spostarli vorrebbe dire renderli visibili al crate per importarli -- ma "
            "crescono solo per decisione: aggiorna "
            f"`{REGISTRO.relative_to(RADICE).as_posix()}` con la ragione."
        )

    if violazioni:
        for una in violazioni:
            print(una, file=sys.stderr)
        return 1

    print(
        f"disposizione delle prove verificata: nessun modulo `#[cfg(test)]` inline nei "
        f"sorgenti di prodotto, {file_di_prove} file di prove dichiarati come moduli "
        f"figli, {superstiti} attributi su singoli elementi come da registro."
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
