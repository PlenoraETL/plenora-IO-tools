#!/usr/bin/env python3
"""Nessun percorso raggiunge il decoder esterno senza prevalidazione (FZ-0).

`arrow-ipc` e `parquet` convertono schema e corpo con funzioni **infallibili**:
dove l'input non e' quello che si aspettano chiamano `panic!`, `assert!` o
`unwrap()`. La barriera `catch_unwind` converte il panico in errore tipizzato e
resta come difesa in profondita', ma non chiude il difetto — sotto
`libfuzzer-sys` il panico diventa `abort()` prima dell'unwinding, e in
produzione un panico attraversa comunque il confine della libreria. Per questo
i driver prevalidano l'input **prima** di consegnarlo.

Una prevalidazione vale pero' quanto la sua copertura: basta un percorso nuovo
che costruisca il reader senza passarci, e il difetto rientra dalla porta di
servizio. Il compilatore non lo impedisce — sono due chiamate di funzione
qualsiasi, e nulla lega la seconda alla prima.

## Come misura

Per ogni costruttore di decoder sorvegliato, il gate individua **la funzione
che lo contiene** e pretende che la prevalidazione corrispondente compaia nella
stessa funzione, **prima** della chiamata. Due proprieta', ed entrambe servono:

* **presenza** — ogni costruzione e' preceduta dalla verifica. Senza questa,
  aggiungere un percorso nuovo che non prevalida passerebbe;
* **esclusivita'** — nessuna crate diversa da quelle dichiarate costruisce quei
  decoder. Senza questa, spostare la chiamata altrove aggirerebbe il gate
  restando verde.

L'ordine conta ed e' verificato: prevalidare *dopo* aver costruito il reader
non serve a niente, perche' il panico avviene durante la costruzione.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


class Sorvegliato:
    """Un costruttore di decoder esterno e la verifica che deve precederlo."""

    def __init__(
        self,
        costruttore: str,
        prevalidazione: str,
        file_ammessi: tuple[str, ...],
        nota: str,
    ) -> None:
        self.costruttore = costruttore
        self.prevalidazione = prevalidazione
        self.file_ammessi = file_ammessi
        self.nota = nota


SORVEGLIATI = (
    Sorvegliato(
        "FileReader::try_new",
        "valida_file_ipc",
        ("crates/driver-ipc/src/lib.rs",),
        "arrow-ipc converte lo schema e affetta il corpo senza restituire "
        "errore sui valori che non riconosce",
    ),
    Sorvegliato(
        "ParquetRecordBatchReaderBuilder::try_new",
        "valida_schema_arrow_incorporato",
        ("crates/driver-geoparquet/src/lib.rs",),
        "il footer Parquet puo' portare ARROW:schema, che passa dalla stessa "
        "conversione infallibile, e i suoi offset Thrift vengono usati senza "
        "controlli",
    ),
)

# Le crate che possono legittimamente nominare i decoder sorvegliati. Un uso
# altrove sfuggirebbe alla verifica per costruzione.
CRATE_AMMESSE = {
    "FileReader::try_new": ("crates/driver-ipc",),
    "ParquetRecordBatchReaderBuilder::try_new": ("crates/driver-geoparquet",),
}

INIZIO_FUNZIONE = re.compile(r"^\s*(?:pub(?:\([^)]*\))?\s+)?(?:async\s+)?fn\s+(\w+)")


def righe_di_test(testo: str) -> set[int]:
    """Le righe che stanno dentro un modulo `#[cfg(test)]`.

    Il perimetro del gate e' il percorso di **produzione**: un test costruisce
    da se' il file che poi rilegge, quindi non ha un input non fidato da
    prevalidare. L'esclusione e' dichiarata qui e contata nell'esito, cosi' non
    diventa un buco silenzioso: se un giorno la parte esclusa crescesse, si
    vedrebbe nel numero.
    """
    righe = testo.splitlines()
    dentro: set[int] = set()
    numero = 0
    while numero < len(righe):
        if righe[numero].strip().startswith("#[cfg(test)]"):
            # Salta fino alla graffa di apertura, poi conta fino alla chiusura.
            apertura = numero
            while apertura < len(righe) and "{" not in righe[apertura]:
                apertura += 1
            if apertura >= len(righe):
                break
            profondita = 0
            corrente = apertura
            while corrente < len(righe):
                profondita += righe[corrente].count("{") - righe[corrente].count("}")
                dentro.add(corrente)
                if profondita <= 0 and corrente >= apertura:
                    break
                corrente += 1
            numero = corrente + 1
            continue
        numero += 1
    return dentro


def funzioni(testo: str) -> list[tuple[str, int, int]]:
    """Spezza un sorgente Rust in (nome, riga_inizio, riga_fine).

    Il confine e' l'indentazione della firma: una funzione finisce dove
    comincia la successiva allo stesso livello o piu' esterno. E' grossolano
    quanto basta — serve ad attribuire una chiamata alla propria funzione, non
    ad analizzare il linguaggio.
    """
    righe = testo.splitlines()
    inizi: list[tuple[str, int, int]] = []
    for numero, riga in enumerate(righe):
        corrispondenza = INIZIO_FUNZIONE.match(riga)
        if corrispondenza:
            indentazione = len(riga) - len(riga.lstrip())
            inizi.append((corrispondenza.group(1), numero, indentazione))

    risultato: list[tuple[str, int, int]] = []
    for posizione, (nome, inizio, indentazione) in enumerate(inizi):
        fine = len(righe)
        for successivo_nome, successivo_inizio, successiva_indentazione in inizi[posizione + 1 :]:
            del successivo_nome
            if successiva_indentazione <= indentazione:
                fine = successivo_inizio
                break
        risultato.append((nome, inizio, fine))
    return risultato


def verifica(radice: Path) -> list[str]:
    """Restituisce l'elenco delle violazioni; vuoto se ogni percorso prevalida."""
    errori: list[str] = []
    esclusi_di_test = 0

    for sorvegliato in SORVEGLIATI:
        trovato_almeno_uno = False
        for relativo in sorvegliato.file_ammessi:
            percorso = radice / relativo
            if not percorso.is_file():
                errori.append(f"{relativo}: manca, ma deve contenere la prevalidazione.")
                continue
            testo = percorso.read_text(encoding="utf-8")
            righe = testo.splitlines()
            blocchi = funzioni(testo)
            solo_test = righe_di_test(testo)

            for numero, riga in enumerate(righe):
                if sorvegliato.costruttore not in riga:
                    continue
                # I commenti che *citano* il costruttore non lo costruiscono.
                if riga.lstrip().startswith("//"):
                    continue
                if numero in solo_test:
                    esclusi_di_test += 1
                    continue
                trovato_almeno_uno = True
                contenitore = next(
                    (blocco for blocco in blocchi if blocco[1] <= numero < blocco[2]),
                    None,
                )
                if contenitore is None:
                    errori.append(
                        f"{relativo}:{numero + 1}: `{sorvegliato.costruttore}` fuori da "
                        "qualunque funzione: il gate non sa cosa dovrebbe precederlo."
                    )
                    continue
                nome, inizio, _ = contenitore
                # I commenti non prevalidano: un `// qui andrebbe valida_...`
                # soddisferebbe una ricerca testuale senza fare niente.
                prima = "\n".join(
                    testo_riga
                    for testo_riga in righe[inizio:numero]
                    if not testo_riga.lstrip().startswith("//")
                )
                if sorvegliato.prevalidazione not in prima:
                    errori.append(
                        f"{relativo}:{numero + 1}: `{nome}` costruisce "
                        f"`{sorvegliato.costruttore}` senza che "
                        f"`{sorvegliato.prevalidazione}` lo preceda nella stessa "
                        f"funzione. Divergenza: {sorvegliato.nota}."
                    )

        if not trovato_almeno_uno:
            errori.append(
                f"nessuna costruzione di `{sorvegliato.costruttore}` trovata in "
                f"{', '.join(sorvegliato.file_ammessi)}: se il percorso e' stato "
                "spostato, il gate va aggiornato insieme."
            )

    # Esclusivita': il costruttore non deve comparire fuori dalle crate ammesse.
    for costruttore, ammesse in CRATE_AMMESSE.items():
        for sorgente in sorted((radice / "crates").rglob("*.rs")):
            relativo = sorgente.relative_to(radice).as_posix()
            if any(relativo.startswith(f"{crate}/") for crate in ammesse):
                continue
            testo = sorgente.read_text(encoding="utf-8")
            solo_test = righe_di_test(testo)
            for numero, riga in enumerate(testo.splitlines()):
                if costruttore not in riga or riga.lstrip().startswith("//"):
                    continue
                if numero in solo_test:
                    esclusi_di_test += 1
                    continue
                errori.append(
                    f"{relativo}:{numero + 1}: `{costruttore}` costruito fuori "
                    f"da {', '.join(ammesse)}, dove la prevalidazione non "
                    "arriva."
                )
    if errori:
        return errori
    return [f"__esclusi__{esclusi_di_test}"]


def main() -> int:
    esito = verifica(ROOT)
    errori = [e for e in esito if not e.startswith("__esclusi__")]
    esclusi = next(
        (int(e.removeprefix("__esclusi__")) for e in esito if e.startswith("__esclusi__")),
        0,
    )
    if errori:
        for messaggio in errori:
            print(messaggio, file=sys.stderr)
        return 1
    print(
        "prevalidazione dei decoder verificata: "
        + ", ".join(
            f"{s.costruttore} preceduto da {s.prevalidazione}" for s in SORVEGLIATI
        )
        + f"; {esclusi} costruzioni escluse perche' in codice di test"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
