"""Client awaitable: tutto il lavoro sincrono rimane nel worker fino alla chiusura."""
from __future__ import annotations

import asyncio
from contextvars import ContextVar, copy_context
from functools import wraps
import threading
from typing import Any

from ._client import Client, OPERAZIONI

_cancellation: ContextVar[threading.Event | None] = ContextVar("io_cancellation", default=None)


def cancellation_requested() -> bool:
    event = _cancellation.get()
    return event is not None and event.is_set()


class AsyncClient:
    """Gli stessi argomenti di Client; scoperta e I/O avvengono nel worker.

    La cancellazione attende sempre la fine dell'operazione. Il backend nativo
    usa il token cooperativo; il processo CLI viene terminato e raccolto.
    Una cancellazione concorrente alla pubblicazione non ne annulla l'effetto.
    """

    def __init__(self, binary: Any = None, *, timeout: float | None = None) -> None:
        self._binary = binary
        self._timeout = timeout

    async def _invoke(self, method: str, *args: Any, **kwargs: Any) -> Any:
        event = threading.Event()

        def invoke() -> Any:
            token = _cancellation.set(event)
            try:
                if event.is_set():
                    raise asyncio.CancelledError
                client = Client(self._binary, timeout=self._timeout)
                if event.is_set():
                    raise asyncio.CancelledError
                return getattr(client, method)(*args, **kwargs)
            finally:
                _cancellation.reset(token)

        # A Future, not a Task: shutdown cancels all Tasks, including an
        # internal to_thread Task, before the underlying thread has stopped.
        worker = asyncio.get_running_loop().run_in_executor(None, copy_context().run, invoke)
        try:
            return await asyncio.shield(worker)
        except asyncio.CancelledError:
            event.set()
            # Anche cancellazioni ripetute devono lasciare il worker raccolto.
            while not worker.done():
                try:
                    await asyncio.shield(worker)
                except asyncio.CancelledError:
                    event.set()
                except Exception:
                    break
            if not worker.cancelled():
                worker.exception()
            raise


def _async_method(name: str):
    @wraps(getattr(Client, name))
    async def call(self: AsyncClient, *args: Any, **kwargs: Any) -> Any:
        return await self._invoke(name, *args, **kwargs)
    return call


# La copertura delle operazioni deriva dal catalogo del client sincrono.
for _name in (*OPERAZIONI, "version", "capabilities"):
    setattr(AsyncClient, _name, _async_method(_name))
