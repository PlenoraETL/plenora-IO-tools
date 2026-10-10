"""L'adattatore Arrow facoltativo: `pyarrow` al confine, file IPC nel mezzo.

# Che cosa fa, e che cosa non fa

L'SDK parla con la CLI per **percorsi**: `read()` consegna un file Arrow IPC,
`write()` ne legge uno. PYTHON-SDK-1.0 §3 chiede che un SDK tabellare usi
oggetti PyArrow o Arrow IPC al confine pubblico; l'IPC c'e' gia', e questo
modulo aggiunge gli oggetti PyArrow sopra lo stesso confine, senza cambiarlo:

* `Client.read_table()` legge in un file IPC temporaneo e lo restituisce come
  `pa.Table`;
* `Client.write()` accetta, al posto del percorso, un oggetto che espone
  `__arrow_c_stream__` -- `pa.Table`, `pa.RecordBatch`, `pa.RecordBatchReader`,
  o qualunque produttore della Arrow PyCapsule Interface -- e lo scrive in un
  file IPC temporaneo prima di passarlo alla CLI.

Il file temporaneo e' il prezzo di un wrapper di processo: i dati attraversano
il disco una volta in piu'. Non c'e' un canale in memoria fra due processi che
il protocollo v2 dichiari, e inventarne uno qui sarebbe una superficie che
nessun contratto descrive.

# Perche' `pyarrow` e' un extra e non una dipendenza

Chi lavora per percorsi -- il caso per cui l'SDK e' nato -- non ne ha bisogno,
e una dipendenza da centinaia di megabyte per un wrapper della libreria
standard sarebbe un costo senza ritorno. Si installa con
`plenora-io[pyarrow]`, nella serie `>=25,<26` che plenora-data-tools fissa:
una sola serie di pyarrow nello stesso ambiente per tutte le librerie Plenora.

L'import e' **pigro**: avviene alla prima chiamata che ne ha bisogno, e se
manca la chiamata fallisce con `OptionalDependencyError` (`unsupported`) prima
di eseguire niente.

# La serie, verificata e non solo dichiarata

`>=25,<26` e' scritto nei metadati, ma un ambiente puo' avere un'altra serie
installata accanto, per esempio con `--no-deps`. Qui la versione importata si
confronta con la serie dichiarata, e una serie diversa e' `unsupported`
invece di un adattatore che gira su una libreria mai provata.
"""

from __future__ import annotations

import os
import tempfile
from pathlib import Path
from typing import Any

from .errors import InvalidArgumentError, OptionalDependencyError, ProtocolError

#: La serie di pyarrow dichiarata dall'extra e provata dalla CI.
SERIE_PYARROW = 25


def pyarrow() -> Any:
    """Il modulo `pyarrow`, o `OptionalDependencyError`.

    Il messaggio non riporta il testo dell'`ImportError`: dice che cosa manca
    e come installarlo, che e' tutto cio' che serve a chi lo legge.
    """
    try:
        import pyarrow as pa  # noqa: PLC0415 - import pigro, e' il punto
        import pyarrow.ipc  # noqa: F401, PLC0415
    except ImportError as errore:
        raise OptionalDependencyError(
            "l'adattatore Arrow richiede pyarrow, che non e' installato: "
            "installa l'extra `plenora-io[pyarrow]` (pyarrow>=25,<26)."
        ) from errore
    maggiore = str(getattr(pa, "__version__", "")).split(".", 1)[0]
    if maggiore != str(SERIE_PYARROW):
        raise OptionalDependencyError(
            f"l'adattatore Arrow e' provato con pyarrow {SERIE_PYARROW}.x e "
            f"l'ambiente ne ha un'altra serie ({maggiore or 'sconosciuta'}.x): "
            "installa l'extra `plenora-io[pyarrow]`."
        )
    return pa


def e_un_percorso(valore: Any) -> bool:
    """Una sorgente che la CLI legge da se': `str` o `os.PathLike`."""
    return isinstance(valore, (str, os.PathLike))


def scrivi_ipc(dati: Any, cartella: Path) -> Path:
    """Scrive un produttore di stream Arrow in un file IPC dentro `cartella`.

    Il produttore si consuma **a batch**: `RecordBatchReader.from_stream`
    legge lo stream che l'oggetto espone, e ogni batch va su disco prima del
    successivo. Una tabella grande non si materializza una seconda volta.

    `pa.RecordBatch` non sempre espone `__arrow_c_stream__` (solo
    `__arrow_c_array__`): lo si avvolge in una tabella di un batch, senza
    copiare i buffer.
    """
    pa = pyarrow()
    if isinstance(dati, pa.RecordBatch):
        dati = pa.Table.from_batches([dati])
    if not hasattr(dati, "__arrow_c_stream__"):
        raise InvalidArgumentError(
            "la sorgente non e' un percorso e non espone `__arrow_c_stream__`: "
            "write() accetta un file Arrow IPC o un oggetto Arrow "
            "(pa.Table, pa.RecordBatch, pa.RecordBatchReader)."
        )
    destinazione = cartella / "ingresso.arrow"
    # Un errore di pyarrow non attraversa il confine pubblico (PYTHON-SDK-1.0
    # §6): diventa un errore tipizzato, e il suo testo -- che puo' citare
    # valori della sorgente -- resta nella catena `__cause__`, non nel
    # messaggio.
    try:
        lettore = pa.RecordBatchReader.from_stream(dati)
        with pa.OSFile(str(destinazione), "wb") as uscita:
            with pa.ipc.new_file(uscita, lettore.schema) as scrittore:
                for batch in lettore:
                    scrittore.write_batch(batch)
    except pa.ArrowException as errore:
        raise InvalidArgumentError(
            "la sorgente Arrow non si e' potuta scrivere come file IPC: lo "
            "stream che espone non e' leggibile da pyarrow."
        ) from errore
    return destinazione


def leggi_ipc(percorso: Path, content_type: str) -> Any:
    """Il file consegnato da `read()`, come `pa.Table` in memoria.

    La serializzazione si sceglie da cio' che la busta dichiara di aver
    consegnato, non dal nome del file ne' provando l'una e poi l'altra. Il file
    si legge per intero in memoria e si chiude prima di tornare: il chiamante
    lo cancella subito dopo, e su Windows un file aperto non si cancella.
    """
    pa = pyarrow()
    if content_type == "application/vnd.apache.arrow.file":
        apri = pa.ipc.open_file
    elif content_type == "application/vnd.apache.arrow.stream":
        apri = pa.ipc.open_stream
    else:
        raise ProtocolError(
            "la consegna dichiara un content type che non e' una delle due "
            "serializzazioni Arrow IPC: l'adattatore non indovina come leggerla."
        )
    try:
        with pa.OSFile(str(percorso), "rb") as sorgente:
            return apri(sorgente).read_all()
    except pa.ArrowException as errore:
        raise ProtocolError(
            "il file che `read` dichiara di aver consegnato non si legge come "
            "Arrow IPC nella serializzazione dichiarata."
        ) from errore


def cartella_temporanea(temp_dir: str | os.PathLike[str] | None) -> Any:
    """Una directory temporanea, cancellata all'uscita dal blocco.

    `temp_dir` sceglie **dove**: con dati grandi conta che stia su un disco
    con spazio, e che sia lo stesso filesystem della destinazione non conta,
    perche' la CLI pubblica la propria uscita da se'.
    """
    return tempfile.TemporaryDirectory(
        prefix="plenora-io-arrow-",
        dir=None if temp_dir is None else os.fspath(temp_dir),
    )
