"""Generated exposure must keep aliases classified and reject new implementations."""
import functools
from types import ModuleType
import unittest

from scripts import arrow_python_api as gate


ENTRY = {'classificazione': 'ingresso guardato', 'motivazione': 'Reviewed test boundary.'}


class GeneratedExposure(unittest.TestCase):
    def setUp(self):
        self.source = ModuleType('plenora_io._client')
        exec('class Client:\n    def write(self, data): return data\n'
             '    @property\n    def backend(self): return "test"', self.source.__dict__)
        self.definitions = {self.source.__name__ + '.Client.' + name: ENTRY
                            for name in ('write', 'backend')}
        self.async_module = ModuleType('plenora_io._async_client')
        self.async_module.__dict__.update(Client=self.source.Client, _wraps=functools.wraps)
        exec('class AsyncClient: pass\n'
             'def _async_method(name):\n'
             '    @_wraps(getattr(Client, name))\n'
             '    async def call(self, *args, **kwargs):\n'
             '        return await self._invoke(name, *args, **kwargs)\n'
             '    return call\n'
             'AsyncClient.write = _async_method("write")', self.async_module.__dict__)
        self.modules = [self.source, self.async_module]
        self.registry = {'schema_version': 2, 'definitions': self.definitions, 'auxiliary': {}}

    def test_every_alias_inherits_the_same_reviewed_definition(self):
        alias = ModuleType('plenora_io')
        alias.Client = self.source.Client
        alias.AsyncClient = self.async_module.AsyncClient
        self.modules.append(alias)
        result = gate.classified_runtime(self.registry, self.modules)
        self.assertEqual(set(result), gate.runtime_members(self.modules))
        for name, entry in result.items():
            with self.subTest(exposure=name):
                self.assertEqual(entry['origine'], 'plenora_io._client.Client.' + name.rsplit('.', 1)[-1])
                self.assertEqual(entry['classificazione'], ENTRY['classificazione'])

    def test_unreviewed_sync_and_async_definitions_fail(self):
        for keyword in ('def', 'async def'):
            with self.subTest(keyword=keyword):
                exec(keyword + ' new(data): return data', self.source.__dict__)
                with self.assertRaisesRegex(AssertionError, 'non_classificati'):
                    gate.classified_runtime(self.registry, self.modules)
                del self.source.new

    def test_wrapped_attribute_cannot_forge_a_delegation(self):
        async def forged(self, data):
            return data
        self.async_module.AsyncClient.write = functools.wraps(self.source.Client.write)(forged)
        with self.assertRaisesRegex(AssertionError, 'delega async non generata'):
            gate.classified_runtime(self.registry, self.modules)

    def test_unknown_auxiliary_origin_fails_closed(self):
        self.source.external = functools.wraps
        with self.assertRaisesRegex(AssertionError, 'non_classificati'):
            gate.classified_runtime(self.registry, self.modules)

    def test_auxiliary_identity_cannot_be_forged(self):
        @functools.wraps(functools.wraps)
        def forged(data):
            return data
        self.source.external = forged
        self.registry['auxiliary']['functools.wraps'] = ENTRY
        with self.assertRaisesRegex(AssertionError, 'origine non autentica'):
            gate.classified_runtime(self.registry, self.modules)

    def test_lost_classification_fails_for_all_aliases(self):
        del self.definitions['plenora_io._client.Client.write']
        with self.assertRaisesRegex(AssertionError, 'non_classificati'):
            gate.classified_runtime(self.registry, self.modules)


if __name__ == '__main__':
    unittest.main()
