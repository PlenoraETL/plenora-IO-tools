"""La guardia di routing deve rilevare bypass e nuovi chiamanti."""
import contextlib
import io
from pathlib import Path
import shutil
import tempfile
import unittest
from unittest.mock import patch

from scripts import check_arrow_external as guard


class RoutingArrow(unittest.TestCase):
    def setUp(self):
        (guard.ROOT / "target").mkdir(exist_ok=True)
        self.directory = tempfile.TemporaryDirectory(dir=guard.ROOT / "target")
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        paths = list((guard.ROOT / "crates").rglob("*.rs"))
        paths += list((guard.ROOT / "sdk/python/src/plenora_io").rglob("*.py"))
        paths += [guard.ROOT / name for name in (
            "assurance/registries/ingressi-arrow-python.json",
            "sdk/python/src/plenora_io/_client.py",
            "sdk/python/src/plenora_io/_arrow.py",
            "assurance/registries/limiti-arrow-c.json",
            "assurance/registries/ingressi-arrow-rust.json",
        )]
        for source in paths:
            target = self.root / source.relative_to(guard.ROOT)
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(source, target)

    def run_guard(self):
        with patch.object(guard, "ROOT", self.root), contextlib.redirect_stdout(io.StringIO()):
            guard.main(runtime=False)

    def change(self, name, old, new):
        path = self.root / name
        source = path.read_text(encoding="utf-8")
        self.assertIn(old, source)
        path.write_text(source.replace(old, new), encoding="utf-8")

    def test_current_routes(self):
        self.run_guard()

    def test_missing_sizer_guard(self):
        self.change("crates/driver-csv/src/lib.rs", "validate_external_batch(", "bypass(")
        with self.assertRaises(AssertionError):
            self.run_guard()

    def test_new_memory_caller(self):
        path = self.root / "crates/driver-csv/src/new.rs"
        path.write_text("fn bypass(batch: &RecordBatch) { incremental_batch_memory_size(batch); }")
        with self.assertRaises(AssertionError):
            self.run_guard()

    def test_missing_writer_guard(self):
        self.change("crates/driver-csv/src/lib.rs", "with_write_validation(", "bypass(")
        with self.assertRaises(AssertionError):
            self.run_guard()

    def test_reader_bypass(self):
        self.change("crates/plenora-io-core/src/driver/reader_adapters.rs",
                    "super::next_external_batch(self.inner.as_mut())", "self.inner.next_batch()")
        with self.assertRaises(AssertionError):
            self.run_guard()

    def test_python_missing_validation(self):
        self.change("sdk/python/src/plenora_io/_arrow.py", "batch.validate(full=True)", "pass")
        with self.assertRaises(AssertionError):
            self.run_guard()

    def test_python_shallow_validation(self):
        self.change("sdk/python/src/plenora_io/_arrow.py", "full=True", "full=False")
        with self.assertRaises(AssertionError):
            self.run_guard()

    def test_new_public_input(self):
        path = self.root / "crates/plenora-io-core/src/new.rs"
        path.write_text("pub fn external(batch: &RecordBatch) {}")
        with self.assertRaises(AssertionError):
            self.run_guard()


    def test_public_signatures_by_construction(self):
        variants = [
            'pub fn bypass(a: &arrow_array::Int64Array) {}',
            'use arrow_array::RecordBatch; pub async fn bypass(a: &RecordBatch) {}',
            'pub struct Probe; impl Probe { pub fn bypass(&self, a: &arrow_array::Int64Array) {} }',
            'use arrow_array::Array; struct Thing<T>(T); impl<T: Array> Thing<T> { pub fn ingest(&self, x: &T) {} }',
            'use arrow_array::Int64Array as Values; pub fn ingest(x: &Values) {}',
            'use arrow_array::Array; pub fn bypass<T: Array>(a: &T) {}',
            'use arrow_array::Array; pub fn bypass<T>(a: &T) where T: Array {}',
            'use arrow_array::Array; pub fn bypass(a: &impl Array) {}',
            'use arrow_array::Int64Array as Renamed; pub fn bypass(a: Option<Vec<&[Renamed]>>) {}',
            'extern crate arrow_array as renamed; pub fn bypass(a: &renamed::Int64Array) {}',
            'use arrow_array as renamed; pub fn bypass(a: &renamed::Int64Array) {}',
            'use arrow_array::*; pub fn bypass(a: &Int64Array) {}',
            'pub struct Payload { a: arrow_array::ArrayRef } pub fn bypass(a: &Payload) {}',
            'type Payload = Option<Vec<arrow_array::ArrayRef>>; pub fn bypass(a: &Payload) {}',
            'pub struct Payload { a: arrow_array::ArrayRef } impl Payload { pub fn bypass(a: Self) {} }',
            'pub fn bypass(a: &parquet::file::metadata::ParquetMetaData) {}',
            'pub mod inner { pub use arrow_array as a; } use inner::*; pub fn bypass(a: &a::Int64Array) {}',
            'pub mod inner { pub use arrow_array as a; } pub fn bypass(a: &inner::a::Int64Array) {}',
            'pub mod inner { use arrow_array::ArrayRef; pub fn bypass(a: ArrayRef) {} }',
        ]
        from scripts.arrow_public_api import check
        path = self.root / 'crates/plenora-io-core/src/new.rs'
        for source in variants:
            with self.subTest(source=source):
                path.write_text(source, encoding='utf-8')
                with self.assertRaisesRegex(AssertionError, 'non_classificati|re-export'):
                    check(self.root)

    def test_cross_file_use_and_wrapper(self):
        from scripts.arrow_public_api import check
        directory = self.root / 'crates/plenora-io-core/src'
        (directory / 'payload.rs').write_text('pub struct Payload { a: arrow_array::ArrayRef }')
        (directory / 'new.rs').write_text('use crate::payload::Payload as Imported; pub fn bypass(a: &Imported) {}')
        with self.assertRaisesRegex(AssertionError, 'non_classificati'):
            check(self.root)

    def assert_forbidden_api(self, source):
        from scripts.arrow_public_api import check
        path = self.root / 'crates/plenora-io-core/src/new.rs'
        path.write_text(source, encoding='utf-8')
        with self.assertRaisesRegex(AssertionError, 'alias o bound Arrow vietati'):
            check(self.root)

    def test_impl_associated_arrow_type_exact_scenario(self):
        self.assert_forbidden_api('impl Sink for Thing { type Payload = arrow_array::ArrayRef; fn ingest(&self, x: Self::Payload) {} }')

    def test_trait_associated_arrow_default(self):
        self.assert_forbidden_api('pub trait Sink { type Payload = arrow_array::ArrayRef; fn ingest(&self, x: Self::Payload); }')

    def test_trait_associated_arrow_bound(self):
        self.assert_forbidden_api('pub trait Sink { type Payload: Send + arrow_array::Array; fn ingest(&self, x: Self::Payload); }')

    def test_public_trait_method_without_pub_is_enumerated(self):
        from scripts.arrow_public_api import extract, inventory
        path = self.root / 'crates/plenora-io-core/src/new.rs'
        path.write_text('pub trait Sink { fn ingest(&self, x: arrow_array::ArrayRef); }')
        self.assertIn('plenora-io-core/src/new.rs::plenora_io_core::new::Sink::ingest', inventory(extract(self.root)))

    def test_trait_impl_method_without_pub_is_enumerated(self):
        from scripts.arrow_public_api import extract, inventory
        path = self.root / 'crates/plenora-io-core/src/new.rs'
        path.write_text('pub trait Sink {} impl Sink for Thing { fn ingest(&self, x: arrow_array::ArrayRef) {} }')
        self.assertIn('plenora-io-core/src/new.rs::plenora_io_core::new::Thing[Sink]::ingest', inventory(extract(self.root)))

    def test_arrow_alias_forms_are_forbidden(self):
        for source in (
            'use arrow_array::ArrayRef as Payload; pub fn ingest(x: Payload) {}',
            'pub use arrow_array::ArrayRef as Payload;',
            'pub type Payload = arrow_array::ArrayRef;',
            'type Payload = arrow_array::ArrayRef; pub fn ingest(x: Payload) {}',
            'mod nested { pub type Payload = arrow_array::ArrayRef; } pub fn ingest(x: nested::Payload) {}',
            'type Payload = arrow_array::ArrayRef; impl Sink for Thing { type Input = Payload; fn ingest(x: Self::Input) {} }',
        ):
            with self.subTest(source=source):
                self.assert_forbidden_api(source)

    def test_arrow_bounds_in_every_position_are_forbidden(self):
        for source in (
            'pub fn ingest<T: arrow_array::Array + Send>(x: T) {}',
            'pub fn ingest<T: Send + arrow_array::Array>(x: T) {}',
            'pub fn ingest<T>(x: T) where T: Send + arrow_array::Array {}',
            'pub fn ingest(x: impl Send + arrow_array::Array) {}',
            'impl<T: Send + arrow_array::Array> Thing<T> { pub fn ingest(x: T) {} }',
            'impl<T> Thing<T> where T: Send + arrow_array::Array { pub fn ingest(x: T) {} }',
            'pub trait Sink<T: Send + arrow_array::Array> { fn ingest(x: T); }',
            'pub fn ingest<T: Into<arrow_array::ArrayRef>>(x: T) {}',
            'pub struct Thing<T: Send + arrow_array::Array>(T);',
            'pub trait Sink: Send + arrow_array::Array {}',
            'pub trait Sink { fn ingest<T: Send + arrow_array::Array>(x: T); }',
            'impl Sink for Thing { fn ingest(x: impl Send + arrow_array::Array) {} }',
        ):
            with self.subTest(source=source):
                self.assert_forbidden_api(source)

    def test_catalog_comes_from_locked_dependencies(self):
        from scripts.arrow_public_api import extract
        catalog = next(r['items'] for r in extract(self.root) if r['kind'] == 'catalog')
        self.assertIn('ArrayRef', catalog['arrow_array'])
        self.assertIn('DataType', catalog['arrow_schema'])
        self.assertIn('ParquetMetaData', catalog['parquet'])
        self.assertGreater(sum(map(len, catalog.values())), 500)


    def test_renderer_validation_cannot_move_into_row_loop(self):
        self.change('crates/driver-csv/src/lib.rs',
                    'let cells = ValidatedArray::columns(batch.columns())?;',
                    'for row in 0..batch.num_rows() { let cells = ValidatedArray::columns(batch.columns())?; }')
        with self.assertRaises(AssertionError):
            self.run_guard()

    def test_raw_renderer_cannot_return_to_row_loop(self):
        self.change('crates/driver-csv/src/lib.rs', 'col.json(row)?',
                    'driver_common::json_from_array(col, row)?')
        with self.assertRaises(AssertionError):
            self.run_guard()


    def test_macro_cannot_hide_new_public_input(self):
        from scripts.arrow_public_api import check
        path = self.root / 'crates/plenora-io-core/src/new.rs'
        path.write_text('macro_rules! generate { () => { pub fn bypass(a: &arrow_array::Int64Array) {} }; } generate!();')
        with self.assertRaisesRegex(AssertionError, 'macro non classificata'):
            check(self.root)


    def test_missing_renderer_full_validation(self):
        self.change('crates/driver-common/src/lib.rs',
                    'data.validate_full().map_err(', 'data.validate().map_err(')
        with self.assertRaises((AssertionError, ValueError)):
            self.run_guard()

    def test_public_reexport_requires_review(self):
        from scripts.arrow_public_api import check
        path = self.root / 'crates/plenora-io-core/src/new.rs'
        path.write_text('pub use arrow_array::Int64Array;')
        with self.assertRaisesRegex(AssertionError, 're-export'):
            check(self.root)

    def test_python_new_public_callable(self):
        for source in ('def ingest(data): return data.to_batches()',
                       'async def ingest(data): return data.to_batches()',
                       'class New:\n    def ingest(self, data): return data.to_batches()'):
            with self.subTest(source=source):
                path = self.root / 'sdk/python/src/plenora_io/new.py'
                path.write_text(source)
                with self.assertRaisesRegex(AssertionError, 'non_classificati'):
                    self.run_guard()

    def test_native_new_function(self):
        path = self.root / 'crates/plenora-io-py/src/new.rs'
        path.write_text('#[pyfunction] fn ingest(data: PyObject) {}')
        with self.assertRaisesRegex(AssertionError, 'non_classificati'):
            self.run_guard()

    def test_runtime_new_callable_and_coroutine(self):
        from types import ModuleType
        from scripts.arrow_python_api import runtime_members, check_entries
        module = ModuleType('plenora_io.probe')
        exec('def ingest(data): return data.to_batches()\n'
             'async def ingest_async(data): return data.to_batches()\n'
             'class Thing:\n    async def ingest(self, data): return data.to_batches()',
             module.__dict__)
        found = runtime_members([module])
        self.assertEqual(found, {'plenora_io.probe.ingest',
                                'plenora_io.probe.ingest_async',
                                'plenora_io.probe.Thing.ingest'})
        with self.assertRaisesRegex(AssertionError, 'non_classificati'):
            check_entries({}, found, 'runtime')
