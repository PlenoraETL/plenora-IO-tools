"""Parità delle operazioni e raccolta del worker anche su cancellazioni ripetute."""
import asyncio
import inspect
import threading
import unittest
from unittest import mock

from plenora_io import AsyncClient, PlenoraError
from plenora_io._async_client import cancellation_requested
from plenora_io._client import OPERAZIONI


class AsyncClientTests(unittest.IsolatedAsyncioTestCase):
    async def test_operations_derive_from_sync_catalog(self):
        for name in OPERAZIONI:
            with self.subTest(name=name):
                self.assertTrue(inspect.iscoroutinefunction(getattr(AsyncClient, name)))

    async def test_construction_and_execution_are_off_loop(self):
        loop_thread = threading.get_ident()
        threads = []

        class Backend:
            def __init__(self, *args, **kwargs):
                threads.append(threading.get_ident())

            def read_table(self, value):
                threads.append(threading.get_ident())
                return value

        with mock.patch('plenora_io._async_client.Client', Backend):
            value = await AsyncClient().read_table(42)
        self.assertEqual(value, 42)
        self.assertEqual(len(threads), 2)
        self.assertNotIn(loop_thread, threads)

    async def test_cancellation_waits_for_worker(self):
        started = threading.Event()
        finished = threading.Event()

        class Backend:
            def __init__(self, *args, **kwargs):
                pass

            def convert(self):
                started.set()
                while not cancellation_requested():
                    finished.wait(.001)
                finished.set()

        with mock.patch('plenora_io._async_client.Client', Backend):
            task = asyncio.create_task(AsyncClient().convert())
            while not started.is_set():
                await asyncio.sleep(.001)
            task.cancel()
            asyncio.get_running_loop().call_soon(task.cancel)
            with self.assertRaises(asyncio.CancelledError):
                await task
        self.assertTrue(finished.is_set())

    async def test_late_exception_is_redacted(self):
        class Backend:
            def __init__(self, *args, **kwargs):
                pass

            def read(self):
                raise ValueError('SECRET_PAYLOAD')

        with mock.patch('plenora_io._async_client.Client', Backend):
            with self.assertRaises(PlenoraError) as caught:
                await AsyncClient().read()
        self.assertNotIn('SECRET_PAYLOAD', str(caught.exception))
        self.assertIsNone(caught.exception.__context__)

    async def test_global_task_cancellation_drains_executor(self):
        started = threading.Event()
        finished = threading.Event()
        release = threading.Event()

        class Backend:
            def __init__(self, *args, **kwargs):
                pass

            def convert(self):
                started.set()
                while not cancellation_requested():
                    release.wait(.001)
                release.wait(2)
                finished.set()

        with mock.patch('plenora_io._async_client.Client', Backend):
            operation = asyncio.create_task(AsyncClient().convert())
            try:
                async with asyncio.timeout(5):
                    while not started.is_set():
                        await asyncio.sleep(.001)
                    for task in asyncio.all_tasks():
                        if task is not asyncio.current_task():
                            task.cancel()
                    await asyncio.sleep(.01)
                    self.assertFalse(operation.done())
                    release.set()
                    with self.assertRaises(asyncio.CancelledError):
                        await operation
                    self.assertTrue(finished.is_set())
            finally:
                release.set()


if __name__ == '__main__':
    unittest.main()
