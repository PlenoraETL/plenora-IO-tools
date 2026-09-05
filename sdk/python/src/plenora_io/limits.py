"""I tetti che si passano a un comando, tipizzati invece che a stringhe.

# Perche' un tipo e non un dizionario

I limiti sono undici, hanno unita' diverse -- byte, righe, millisecondi, livelli
di annidamento -- e sbagliarne il nome non da' un errore finche' il comando non
parte. Un dizionario li accetterebbe tutti, compresi quelli scritti male; una
dataclass li nomina, e chi sbaglia lo scopre dove ha scritto.

# Il tetto e il timeout non sono la stessa cosa

`deadline` e' un **budget** che il prodotto conosce: lo rispetta, e quando scade
si ferma da se' e risponde con una busta d'errore che lo dice --
`TimeoutError`, categoria `timeout`. Il lavoro fatto fino a quel punto e'
descritto.

`Client(timeout=...)` e' un'altra cosa: uccide il processo da fuori, e quel che
resta e' un `ProtocolError` che dice **non si sa**. Nessuna busta, nessun
conteggio, nessuna garanzia su cosa sia stato scritto.

Chi vuole un limite di tempo governato passa `deadline`. Chi vuole una difesa
contro un binario che non risponde piu' passa `timeout`. Chi vuole entrambe le
cose le passa entrambe, e il primo che scatta e' quello con il numero piu'
piccolo.

# Nessuna validazione dei valori

Che `max_rows=0` abbia senso lo decide il prodotto, che ha le sue regole e le
applica in un posto solo. Ricopiarle qui produrrebbe due giudizi sullo stesso
numero, e il giorno in cui divergessero l'SDK rifiuterebbe un comando che la
CLI accetta -- o peggio il contrario.
"""

from __future__ import annotations

from dataclasses import dataclass, fields
from datetime import timedelta


@dataclass(frozen=True, kw_only=True)
class Limits:
    """I tetti di un'esecuzione, nella forma che la CLI accetta.

    Ogni campo `None` non viene passato: il prodotto ha i propri valori
    predefiniti, e riscriverli qui vorrebbe dire mantenerli in due posti.
    """

    #: Il budget di tempo che il **prodotto** rispetta, con `--deadline-ms`.
    deadline: timedelta | None = None
    max_rows: int | None = None
    max_columns: int | None = None
    max_vertices: int | None = None
    max_input_bytes: int | None = None
    max_input_entries: int | None = None
    max_output_bytes: int | None = None
    max_wkb_cell_bytes: int | None = None
    max_wkb_components: int | None = None
    max_wkb_depth: int | None = None
    memory_bytes: int | None = None

    #: Il campo che non e' un numero, e la sua opzione.
    #:
    #: `deadline` e' un `timedelta` perche' e' un tempo, e un intero nudo
    #: lascerebbe indovinare l'unita' -- che nella CLI e' il millisecondo, e
    #: nelle librerie Python quasi sempre il secondo.
    DURATA = "deadline"

    def to_argv(self) -> list[str]:
        """Gli argomenti, in ordine stabile.

        L'ordine e' quello di dichiarazione dei campi e non quello di un
        dizionario: due chiamate con gli stessi limiti producono la stessa riga,
        che e' cio' che rende confrontabili due esecuzioni.
        """
        argomenti: list[str] = []
        for campo in fields(self):
            valore = getattr(self, campo.name)
            if valore is None:
                continue
            if campo.name == self.DURATA:
                argomenti += [
                    "--deadline-ms",
                    str(int(valore.total_seconds() * 1000)),
                ]
            else:
                argomenti += [f"--{campo.name.replace('_', '-')}", str(valore)]
        return argomenti

    @classmethod
    def opzioni(cls) -> list[str]:
        """I nomi delle opzioni che questo tipo sa produrre.

        `scripts/check_sdk_python.py` li confronta con `OPZIONI_AMMESSE` della
        CLI: un tetto che l'SDK offre e la CLI non conosce e' un comando che
        fallira' sull'uso, e uno che la CLI accetta e l'SDK non offre e' un
        tetto raggiungibile solo scrivendo la riga a mano.
        """
        fuori = []
        for campo in fields(cls):
            if campo.name == cls.DURATA:
                fuori.append("--deadline-ms")
            else:
                fuori.append(f"--{campo.name.replace('_', '-')}")
        return sorted(fuori)

    def __bool__(self) -> bool:
        """Falso quando nessun tetto e' impostato."""
        return any(getattr(self, campo.name) is not None for campo in fields(self))
