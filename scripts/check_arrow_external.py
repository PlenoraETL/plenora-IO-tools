"""Guarda gli ingressi Arrow del client e il routing degli adapter Rust.

Non certifica le capacita' delle allocazioni dietro capsule C generiche.
"""
import ast
import json
import re
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]

def main(runtime=True):
    arrow = ast.parse((ROOT / "sdk/python/src/plenora_io/_arrow.py").read_text(encoding="utf-8"))
    funcs = {n.name: n for n in arrow.body if isinstance(n, ast.FunctionDef)}
    calls = lambda f: [ast.unparse(n.func) for n in ast.walk(funcs[f]) if isinstance(n, ast.Call)]
    assert calls("scrivi_ipc").count("_lettore_originale") == 1
    assert calls("scrivi_ipc").count("_batch_validati") == 1
    assert "batch.validate" in calls("_batch_validati")
    for node in ast.walk(arrow):
        if isinstance(node, ast.Call) and isinstance(node.func, ast.Attribute) and node.func.attr == "validate":
            assert any(k.arg == "full" and isinstance(k.value, ast.Constant) and k.value.value is True for k in node.keywords), ast.unparse(node)
    imports = [(f, c) for f in funcs for c in calls(f) if c == "pa.RecordBatchReader.from_stream"]
    assert imports == [("_lettore_originale", "pa.RecordBatchReader.from_stream")], imports
    source = ast.unparse(funcs["_lettore_originale"])
    assert source.index("pa.RecordBatchReader)") < source.index("pa.RecordBatchReader.from_stream")
    for original in ("pa.Table)", "pa.RecordBatch)"):
        assert source.index(original) < source.index("__arrow_c_stream__")
    limit = json.loads((ROOT / "assurance/registries/limiti-arrow-c.json").read_text(encoding="utf-8"))
    for key in ("regola", "ambito", "hazard", "condizione_di_rientro"):
        assert limit[key], key
    drivers = sorted(ROOT.glob("crates/driver-*/src/lib.rs"))
    writers = []
    for path in drivers:
        source = path.read_text(encoding="utf-8")
        if "impl FormatDriver for" in source and "fn create(" in source:
            assert "with_write_validation(" in source, path
            writers.append(path.parent.parent.name)
    assert len(writers) == 10, writers
    adapters = (ROOT / "crates/plenora-io-core/src/driver/reader_adapters.rs").read_text(encoding="utf-8")
    assert "self.inner.next_batch()" not in adapters
    assert "inner.next_batch()" not in adapters
    assert adapters.count("super::next_external_batch(") == 5
    ipc = (ROOT / "crates/driver-ipc/src/lib.rs").read_text(encoding="utf-8")
    assert "File::open(&path)?, projection" not in ipc
    assert ipc.count("valida_e_proietta(b, proiezione.as_deref())") == 2
    # Inventario chiuso dei chiamanti operativi: nuovi usi richiedono revisione.
    expected = {
        "observe": {"driver-csv": 1, "driver-dxf": 1, "driver-filegdb": 1,
                    "driver-geojson": 1, "driver-gpkg": 1, "driver-kml": 1,
                    "driver-shp": 1, "driver-xls": 1},
        "render": {"driver-csv": 1, "driver-dxf": 1, "driver-geojson": 1,
                   "driver-kml": 1, "driver-shp": 1, "driver-xls": 1},
        "memory": {"plenora-io-core/src/request.rs": 1,
                   "plenora-io-core/src/driver/reader_adapters.rs": 1,
                   "plenora-io-core/src/driver/scrittura_limitata.rs": 1},
    }
    found = {key: {} for key in expected}
    for path in sorted((ROOT / "crates").rglob("*.rs")):
        rel = path.relative_to(ROOT / "crates").as_posix()
        if "/tests/" in rel or path.name == "tests.rs" or "/tests.rs" in rel:
            continue
        source = path.read_text(encoding="utf-8")
        # I moduli di test separati non fanno parte del percorso operativo.
        source = re.sub(r"#\[cfg\(test\)\]\s*mod tests;", "", source)
        crate = rel.split("/")[0]
        for match in re.finditer(r"(?:[\w.]*batch_sizer)\.observe\([^;]*\)", source):
            found["observe"][crate] = found["observe"].get(crate, 0) + 1
            prefix = source[max(0, match.start() - 350):match.start()]
            assert "validate_external_batch(" in prefix, rel
            assert "ErrorPhase::Read" in prefix and "?;" in prefix, rel
        renders = source.count("ValidatedArray::columns(")
        if renders:
            found["render"][crate] = found["render"].get(crate, 0) + renders
            assert crate in writers, rel
            for match in re.finditer(r"ValidatedArray::columns\(", source):
                function_start = source.rfind("    fn ", 0, match.start())
                assert function_start >= 0, rel
                assert "for row in" not in source[function_start:match.start()], rel
        # Raw fallible entry points validate per invocation: operational row
        # loops must use the reusable snapshot, never invoke them per cell.
        if crate != "driver-common":
            assert not re.search(r"(?<!fn )\bjson_from_array\(", source), rel
            if crate != "driver-csv":
                assert not re.search(r"(?<!fn )\bcell_string\(", source), rel
        memory = re.findall(r"(?<!fn )\bincremental_batch_memory_size\(", source)
        if memory:
            found["memory"][rel] = len(memory)
    assert found == expected, (found, expected)
    writer = (ROOT / "crates/plenora-io-core/src/driver/scrittura_limitata.rs").read_text(encoding="utf-8")
    assert writer.index("validate_external_batch(batch") < writer.index("incremental_batch_memory_size(batch")
    assert adapters.index("super::next_external_batch(") < adapters.index("incremental_batch_memory_size(&batch")
    common = (ROOT / "crates/driver-common/src/lib.rs").read_text(encoding="utf-8")
    assert "ValidatedArray::new(array)?.json(row)" in common
    assert "ValidatedArray::new(array)?.text(row)" in common
    validation = common.index("data.validate_full().map_err(")
    snapshot = common.index("Ok(Self(arrow_array::make_array(data)))")
    assert validation < snapshot and "})?;" in common[validation:snapshot]
    for key in ("regola", "ambito", "hazard", "condizione_di_rientro"):
        assert limit["enumerazione_rust"][key], key
    helper_limit = limit["helper_rust"]
    for key in ("regola", "ambito", "hazard", "condizione_di_rientro"):
        assert helper_limit[key], key
    for file, names in {
        "crates/plenora-io-core/src/request.rs": ("observe", "incremental_batch_memory_size"),
    }.items():
        source = (ROOT / file).read_text(encoding="utf-8")
        for name in names:
            start = source.index("pub fn " + name + "(")
            assert "# Preconditions" in source[max(0, start - 650):start], name
    try:
        from scripts.arrow_public_api import check, extract
        from scripts.arrow_python_api import check as check_python
    except ModuleNotFoundError:
        from arrow_public_api import check, extract
        from arrow_python_api import check as check_python
    records = extract(ROOT)
    public_inputs = check(ROOT, records)
    check_python(ROOT, records, runtime=runtime)
    client = ast.parse((ROOT / "sdk/python/src/plenora_io/_client.py").read_text(encoding="utf-8"))
    cls = next(n for n in client.body if isinstance(n, ast.ClassDef) and n.name == "Client")
    write = next(n for n in cls.body if isinstance(n, ast.FunctionDef) and n.name == "write")
    assert sum(isinstance(n, ast.Call) and ast.unparse(n.func) == "arrow.scrivi_ipc" for n in ast.walk(write)) == 1
    assert public_inputs
    # restore e' interno al modulo privato del driver e non e' riesportato.
    xls = (ROOT / "crates/driver-xls/src/lib.rs").read_text(encoding="utf-8")
    assert "mod date_native;" in xls and "pub mod date_native" not in xls
    assert not re.search(r"pub use date_native", xls)
    for relative, count in {
        "driver/batch_worker.rs": 2, "driver/spool.rs": 1,
        "driver/validazione_geometrie.rs": 1, "ewkb.rs": 1,
    }.items():
        source = (ROOT / "crates/plenora-io-core/src" / relative).read_text(encoding="utf-8")
        assert source.count("validate_external_batch(") == count, relative
    print("Client.write -> scrivi_ipc; 10 driver -> with_write_validation; 5 adapter -> next_external_batch; IPC file/stream -> valida_e_proietta")

if __name__ == "__main__":
    import argparse
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--static-only", action="store_true", help="solo fase sorgenti; la CI deve eseguire anche arrow_python_api.py sulla wheel nativa")
    parser.add_argument("--runtime", action="store_true", help="fase completa (default)")
    main(runtime=not parser.parse_args().static_only)

