"""Le sonde d'integrazione, eseguite anche dal modulo nativo della wheel abi3.

# Perche' le stesse sonde e non altre

La wheel nativa promette lo stesso protocollo del binario: stesso dispatch,
stesse buste, stessi errori. Il modo di provarlo e' far passare il modulo dalle
**stesse** sonde d'integrazione che provano il binario, non scriverne di
proprie. `variante_nativa(Classe)` deriva da una classe d'integrazione una
sorella che costruisce il client senza binario, nel processo.

# Saltare, e quando invece fallire

Senza il modulo -- il pacchetto puro, o il sorgente del repository -- la
variante salta. Con `PLENORA_IO_RICHIEDI_NATIVO=1`, che la CI imposta quando
prova le wheel abi3, l'assenza e' un fallimento: una sonda che salta proprio
dove deve girare non prova niente.
"""

from __future__ import annotations

import os

from plenora_io import Client
from plenora_io.discovery import VARIABILE, modulo_nativo

#: Il segnaposto che le classi d'integrazione tengono in `binario` quando la
#: variante e' quella nativa.
NATIVO = "<modulo nativo plenora_io._native>"


def client_di_prova(binario: str) -> Client:
    """Il client per una sonda d'integrazione: col binario, o nel processo."""
    if binario != NATIVO:
        return Client(binary=binario)
    # `PLENORA_IO_BIN` vincerebbe sul modulo nativo: si toglie per la sola
    # costruzione, e la sonda verifica che il client sia davvero quello nativo.
    salvata = os.environ.pop(VARIABILE, None)
    try:
        client = Client()
    finally:
        if salvata is not None:
            os.environ[VARIABILE] = salvata
    if client.backend != "native" or client.binary is not None:
        raise AssertionError("il client della variante nativa non e' nativo")
    return client


def binario_nativo() -> str | None:
    """`NATIVO` se il modulo c'e', `None` altrimenti -- o un errore, se richiesto."""
    if modulo_nativo() is not None:
        return NATIVO
    if os.environ.get("PLENORA_IO_RICHIEDI_NATIVO") == "1":
        raise AssertionError(
            "PLENORA_IO_RICHIEDI_NATIVO=1 e il modulo plenora_io._native non c'e'"
        )
    return None


def variante_nativa(classe: type) -> type:
    """Una sottoclasse della sonda d'integrazione che esegue nel processo."""

    class Nativa(classe):  # type: ignore[valid-type, misc]
        @classmethod
        def setUpClass(cls) -> None:
            cls.binario = binario_nativo()

    Nativa.__name__ = Nativa.__qualname__ = f"{classe.__name__}Nativa"
    Nativa.__doc__ = f"{classe.__name__}, dal modulo nativo invece che dal binario."
    return Nativa
