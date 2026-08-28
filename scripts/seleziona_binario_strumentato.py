#!/usr/bin/env python3
"""Quale binario descrive un profdata, senza che lo decida il filesystem.

# Il difetto che questo modulo esiste per chiudere

`scripts/fuzz-profondita.sh` enumerava i candidati con `find` e teneva il
**primo** per cui `llvm-cov export` riusciva. Due cose sbagliate in una riga:

  * l'ordine di `find` dipende dall'ordine di directory del filesystem, che non
    e' una proprieta' dell'albero misurato;
  * fermarsi al primo successo significa non sapere mai se ce n'era un secondo,
    e se quel secondo fosse stato un binario **diverso** la misura sarebbe
    cambiata senza che nulla lo dichiarasse.

Nelle corse osservate i candidati erano due e la scelta e' sempre caduta sullo
stesso, quindi non e' la causa della bimodalita' che
`fuzz.profondita-riproducibile` ha trovato. Era una fragilita' latente, e si
chiude con quattro condizioni, tutte e quattro necessarie:

  1. **enumerazione deterministica** dei candidati: l'ordine viene da un
     ordinamento sui percorsi, non da `readdir`;
  2. **verifica di tutti** i candidati, non l'arresto al primo successo:
     altrimenti l'esistenza di un secondo binario compatibile e diverso resta
     invisibile per costruzione;
  3. **fallimento** se piu' candidati compatibili non sono byte-identici: due
     binari diversi che accettano lo stesso profdata sono due misure diverse, e
     sceglierne una sarebbe deciderla a caso;
  4. **scelta canonica soltanto fra copie identiche**: quando i compatibili
     hanno tutti la stessa impronta, quale si prenda non puo' cambiare la
     misura, e si prende il primo dell'ordine.

La terza e la quarta si tengono insieme: la quarta e' sicura *perche'* la terza
ha gia' rifiutato il caso in cui non lo sarebbe.

# Perche' e' un modulo e non tre righe di shell

Perche' le quattro condizioni sopra sono affermazioni, e un'affermazione senza
una sonda che la violi non e' verificata. In shell le sonde avrebbero dovuto
costruire binari veri e un profdata vero; qui la compatibilita' e l'impronta
sono due funzioni iniettabili, e `scripts/test_seleziona_binario_strumentato.py`
prova ciascuna condizione facendola fallire.

# Uso

    python3 scripts/seleziona_binario_strumentato.py shp_reader \\
        --radice target --radice fuzz/target \\
        --llvm-cov /percorso/llvm-cov \\
        --instr-profile fuzz/coverage/shp_reader/coverage.profdata

Stampa su stdout il percorso scelto, e nient'altro: lo consuma una sostituzione
di comando.
"""

from __future__ import annotations

import argparse
import hashlib
import subprocess
import sys
from collections.abc import Callable, Iterable
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent


class SelezioneImpossibile(Exception):
    """Nessun binario, o piu' d'uno e non intercambiabili.

    Non e' un caso da gestire scegliendo comunque: una misura attribuita al
    binario sbagliato descrive un albero che nessuno ha compilato.
    """


def candidati(radici: Iterable[str | Path], nome: str) -> list[str]:
    """I file di nome `nome` sotto le radici, in ordine **totale sui percorsi**.

    L'ordine e' l'ordinamento delle stringhe, non quello in cui il filesystem
    li restituisce: due macchine con lo stesso albero enumerano la stessa
    sequenza. La deduplicazione serve a radici che si contengono a vicenda --
    lo stesso file trovato due volte resterebbe due candidati, e un confronto
    di impronte fra un file e se stesso direbbe sempre di si'.
    """
    trovati: set[str] = set()
    for radice in radici:
        base = Path(radice)
        if not base.is_dir():
            continue
        for percorso in base.rglob(nome):
            if percorso.is_file():
                trovati.add(percorso.as_posix())
    return sorted(trovati)


def impronta_del_file(percorso: str | Path) -> str:
    """SHA-256 del contenuto: e' l'identita' con cui «byte-identici» si decide."""
    return hashlib.sha256(Path(percorso).read_bytes()).hexdigest()


def scelta(
    elenco: list[str],
    compatibile: Callable[[str], bool],
    impronta: Callable[[str], str] = impronta_del_file,
) -> str:
    """Il candidato canonico, o `SelezioneImpossibile` con il motivo.

    `compatibile` viene chiamata su **ogni** candidato, e il ciclo e' scritto
    per esteso invece che con `any` o con un `break`: la condizione da provare
    e' proprio che nessuno venga saltato, e una scorciatoia che si ferma al
    primo successo e' esattamente il difetto da cui questo modulo viene.
    """
    if not elenco:
        raise SelezioneImpossibile(
            "nessun candidato con quel nome sotto le radici dichiarate: non c'e' "
            "un binario di cui la misura possa parlare"
        )

    compatibili: list[str] = []
    for candidato in elenco:
        if compatibile(candidato):
            compatibili.append(candidato)

    if not compatibili:
        raise SelezioneImpossibile(
            f"nessuno dei {len(elenco)} candidati accetta il profdata: "
            + ", ".join(elenco)
        )

    per_impronta: dict[str, list[str]] = {}
    for candidato in compatibili:
        per_impronta.setdefault(impronta(candidato), []).append(candidato)

    if len(per_impronta) > 1:
        dettaglio = "; ".join(
            f"{digest[:12]}: {', '.join(percorsi)}"
            for digest, percorsi in sorted(per_impronta.items())
        )
        raise SelezioneImpossibile(
            f"{len(compatibili)} candidati accettano il profdata e **non** sono "
            f"byte-identici: {dettaglio}. Sono due misure diverse, e sceglierne "
            "una la deciderebbe a caso: rifare la build, o restringere le radici."
        )

    # Canonica, e sicura solo perche' il controllo sopra ha gia' rifiutato il
    # caso in cui la scelta cambierebbe qualcosa: qui i compatibili sono copie
    # identiche, e il minore dell'ordine e' uno qualunque di loro.
    #
    # `min` e non `compatibili[0]`: la scelta non deve dipendere dall'ordine in
    # cui l'elenco e' arrivato, nemmeno da un chiamante che non lo avesse
    # ordinato. Cosi' la canonicita' e' una proprieta' di questa funzione e non
    # una convenzione fra due.
    return min(compatibili)


def compatibilita_con_llvm_cov(
    llvm_cov: str, instr_profile: str, log: Path | None
) -> Callable[[str], bool]:
    """«Questo binario accetta quel profdata?», nella forma che lo decide.

    Compatibile significa due cose insieme: `llvm-cov export` esce con zero
    **e** scrive qualcosa. Un export riuscito e vuoto non e' una misura, e
    accettarlo qui avrebbe spostato il rosso a valle, dove il candidato scelto
    non e' piu' ricostruibile.
    """

    def compatibile(candidato: str) -> bool:
        esito = subprocess.run(
            [
                llvm_cov,
                "export",
                candidato,
                f"--instr-profile={instr_profile}",
                "--format=text",
                "--skip-expansions",
            ],
            capture_output=True,
            check=False,
        )
        if log is not None:
            with log.open("ab") as apri:
                apri.write(f"== {candidato}: rc={esito.returncode}\n".encode())
                apri.write(esito.stderr)
        return esito.returncode == 0 and bool(esito.stdout.strip())

    return compatibile


def main(argv: list[str] | None = None) -> int:
    argomenti = argparse.ArgumentParser(description=__doc__)
    argomenti.add_argument("nome", help="il nome del binario, cioe' quello del target")
    argomenti.add_argument(
        "--radice",
        action="append",
        default=[],
        help="una directory in cui cercarlo; ripetibile",
    )
    argomenti.add_argument("--llvm-cov", required=True, help="l'eseguibile llvm-cov")
    argomenti.add_argument(
        "--instr-profile", required=True, help="il profdata che il binario deve accettare"
    )
    argomenti.add_argument(
        "--log", type=Path, default=None, help="dove finisce lo stderr di ogni tentativo"
    )
    opzioni = argomenti.parse_args(argv)

    if not opzioni.radice:
        print("almeno una `--radice`: senza, non c'e' dove cercare", file=sys.stderr)
        return 2

    elenco = candidati(opzioni.radice, opzioni.nome)
    try:
        binario = scelta(
            elenco,
            compatibilita_con_llvm_cov(
                opzioni.llvm_cov, opzioni.instr_profile, opzioni.log
            ),
        )
    except SelezioneImpossibile as errore:
        print(str(errore), file=sys.stderr)
        return 1
    print(binario)
    return 0


if __name__ == "__main__":
    sys.exit(main())
