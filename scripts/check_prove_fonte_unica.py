"""Inventari di garanzia dai sorgenti e controlli statici per ogni PR.

Il JSON in target e' un indice riproducibile, non un'attestazione di esecuzione.
I pin storici e gli oracoli indipendenti non vengono rigenerati dall'output IO.
"""
from __future__ import annotations

import argparse
import ast
import hashlib
import json
from pathlib import Path
import re
import subprocess
import sys
import tomllib

try:
    from . import strict_json
except ImportError:
    import strict_json

ROOT = Path(__file__).resolve().parents[1]

# Comandi statici: nessuna build, rete, registrazione o misura sostituita da
# uno snapshot. La lista e' una politica di esecuzione, non un conteggio.
STATIC = [
    ["check_python_version.py", "--runtime"], ["system_qualification.py"], ["check_async_surface.py"],
    ["check_action_pins.py"], ["check_public_identity.py"],
    ["check_docset.py"], ["check_comments.py"], ["check_test_layout.py"],
    ["code_size.py"], ["check_unsafe_forbid.py"],
    ["check_dependency_pins.py"], ["check_toolchain_pins.py"],
    ["check_coverage_exclusions.py"], ["check_prevalidazione_decoder.py"],
    ["check_quarantena_fuzz.py"], ["check_riuso_evidenze.py"],
    ["check_assurance_fallbacks.py"], ["check_no_legacy_budget.py"],
    ["check_quartetto_sito.py"], ["check_assurance_n1.py", "--integrita"],
    ["check_errori_redatti.py"], ["check_wkb_limits_defaults.py"],
    ["check_permit_boundary.py"], ["check_niente_leak.py"],
    ["check_capability_input_ostile.py"],
    ["check_metadati_geoparquet.py"], ["check_schemi_geoparquet.py"],
    ["check_categorie_di_perdita.py"], ["check_protocollo_v2.py"],
    ["check_dettaglio_congelato.py"], ["check-conversioni.py"],
    ["check-fixture-canoniche.py"], ["generate_shared_wkb_corpus.py", "--check"],
    ["genera_semi_shp.py", "--verifica"],
    ["genera_fixture_filegdb.py", "--verifica"],
    ["genera_semi_filegdb.py", "--verifica"],
    ["genera_semi_s12.py", "--verifica"], ["genera_semi_gpkg.py", "--verifica"],
    ["check_cargo_deny.py"],
]


def python_tests(root: Path, native: bool = False) -> list[Path]:
    """Discovery ricorsiva; nessun elenco nominale da aggiornare in CI."""
    found = []
    for path in sorted((root / "scripts").rglob("test_*.py")):
        try:
            tree = ast.parse(path.read_text(encoding="utf-8"))
        except SyntaxError:
            # Il loader registrera' l'import fallito; non perdere gli altri
            # moduli solo perche' questo non e' analizzabile.
            tree = ast.Module(body=[], type_ignores=[])
        marked = any(isinstance(n, ast.Assign) and any(
            isinstance(t, ast.Name) and t.id == "NATIVE_PROOF" for t in n.targets)
            and isinstance(n.value, ast.Constant) and n.value.value is True for n in tree.body)
        requires_native = marked or "prove_native" in path.relative_to(root / "scripts").parts
        if requires_native == native:
            found.append(path)
    return found


def inventory(root: Path) -> dict:
    import yaml
    result: dict = {"python_tests": {}, "rust_test_functions": {},
                    "workflows": {}, "registries": {}, "fixtures": [],
                    "scripts": [], "data_sources": []}
    result["scripts"] = sorted(p.relative_to(root).as_posix()
        for p in (root / "scripts").rglob("*")
        if p.is_file() and p.suffix in (".py", ".sh", ".ps1", ".rs")
        and not {"target", "__pycache__"}.intersection(p.relative_to(root).parts))
    for folder in ("contracts", "release", "assurance", "baseline", "scripts"):
        result["data_sources"].extend(sorted(p.relative_to(root).as_posix()
            for p in (root / folder).rglob("*.json")
            if not {"target", "__pycache__"}.intersection(p.relative_to(root).parts)))
    for path in sorted(python_tests(root) + python_tests(root, native=True)):
        tree = ast.parse(path.read_text(encoding="utf-8"))
        result["python_tests"][path.relative_to(root).as_posix()] = [
            n.name for n in ast.walk(tree)
            if isinstance(n, (ast.FunctionDef, ast.AsyncFunctionDef))
            and n.name.startswith("test_")]
    # Questo indice e' sintattico: cargo --list resta l'autorita' sulle
    # configurazioni compilate e sui test generati dalle macro.
    for path in sorted((root / "crates").rglob("*.rs")):
        text = path.read_text(encoding="utf-8")
        if "#[test]" in text or "#[test_case" in text or "proptest!" in text:
            result["rust_test_functions"][path.relative_to(root).as_posix()] = re.findall(
                r"\bfn\s+([\w]+)\s*\(", text)
    for path in sorted((root / ".github/workflows").glob("*.y*ml")):
        # BaseLoader conserva 'on' come stringa anche con YAML 1.1.
        result["workflows"][path.relative_to(root).as_posix()] = yaml.load(
            path.read_text(encoding="utf-8"), Loader=yaml.BaseLoader)
    sources = {}
    for path in sorted((root / "scripts").glob("*.py")):
        if path.name.startswith("test_") or path.name == Path(__file__).name:
            continue
        tree = ast.parse(path.read_text(encoding="utf-8"))
        sources[path.relative_to(root).as_posix()] = {
            n.value for n in ast.walk(tree)
            if isinstance(n, ast.Constant) and isinstance(n.value, str)}
    for path in sorted((root / "assurance/registries").glob("*.json")):
        result["registries"][path.relative_to(root).as_posix()] = sorted(
            source for source, strings in sources.items()
            if any(path.name == s or s.endswith("/" + path.name) for s in strings))
    for directory in ("crates", "sdk", "fuzz/seeds", "conformance", "oracle",
                      "scripts/fixtures", "assurance/evidence"):
        for path in sorted((root / directory).rglob("*")):
            if path.is_file() and "__pycache__" not in path.parts and (
                directory not in ("crates", "sdk") or "fixtures" in path.parts
            ):
                result["fixtures"].append(path.relative_to(root).as_posix())
    return result


def check_checkpoint(root: Path) -> list[str]:
    script = (root / "scripts/s9-checkpoint.sh").read_text(encoding="utf-8")
    found = re.findall(r"^passo(?:_pesante(?:_in_catena)?)?\s+(\w+)\s", script, re.M)
    inline = re.findall(r"^\s+registra_passo\s+(\w+)\s", script, re.M)
    found += list(dict.fromkeys(inline))
    declared = [x["id"] for x in strict_json.loads(
        (root / "assurance/registries/passi-del-checkpoint.json").read_text(encoding="utf-8"))["passi"]]
    return [] if found == declared else [
        f"checkpoint: sequenza sorgente diversa dal registro; sorgente={found}, registro={declared}"]


def check_multigeometry(root: Path) -> list[str]:
    registry = strict_json.loads((root / "assurance/registries/io-geometrie-multiple.json").read_text(encoding="utf-8"))
    rust = "\n".join(p.read_text(encoding="utf-8") for p in (root / "crates").rglob("*.rs"))
    functions = set(re.findall(r"\bfn\s+(\w+)\s*\(", rust))
    errors = []
    # Le sezioni correzione_N descrivono corse storiche, non contatori della
    # suite corrente. Le prove dei formati invece devono ancora esistere.
    for entry in registry["formati"]:
        proofs = entry.get("prove", []) + ([entry["prova_lettura"]] if "prova_lettura" in entry else [])
        for proof in proofs:
            if ("/" in proof and not (root / proof).is_file()) or ("/" not in proof and proof not in functions):
                errors.append(f"io-geometrie-multiple: prova assente {proof}")
    proof = registry['prova_rifiuti_cli']
    path, symbol = proof.split('::', 1)
    if not (root / path).is_file() or not re.search(
            rf"\bfn\s+{re.escape(symbol)}\s*\(", (root / path).read_text(encoding='utf-8')):
        errors.append(f"io-geometrie-multiple: prova CLI assente {proof}")
    fixture = registry['fixture']
    path = root / fixture['path']
    if not path.is_file() or hashlib.sha256(path.read_bytes()).hexdigest() != fixture['sha256']:
        errors.append('io-geometrie-multiple: digest fixture diverso dal registro')
    for path in (fixture['generatore'], registry['tabella_cardinalita']):
        if not (root / path).is_file():
            errors.append(f'io-geometrie-multiple: sorgente assente {path}')
    return errors


def pr_scripts(root: Path, workflows: dict) -> set[str]:
    """Chiusura dei richiami a script e import, partendo dai comandi su PR.

    Non usa commenti, nomi di job o documentazione come prova di collegamento.
    Le condizioni runtime restano provate dalle esecuzioni dei rispettivi job.
    """
    paths = {p.name: p.relative_to(root).as_posix() for p in (root / 'scripts').glob('*')
             if p.suffix in ('.py', '.sh', '.ps1')}
    paths.update({p.name: p.relative_to(root).as_posix()
                  for p in (root / 'assurance/registries').glob('*.json')})

    def references(text):
        return {paths[name] for name in re.findall(r'[\w-]+\.(?:py|sh|ps1|json)\b', text)
                if name in paths}

    reached = {'scripts/' + c[0] for c in STATIC}
    for workflow in workflows.values():
        if 'pull_request' not in workflow.get('on', {}):
            continue
        for job in workflow.get('jobs', {}).values():
            if 'if' in job:
                continue
            for step in job.get('steps', []):
                reached |= references(step.get('run', ''))
    pending = list(reached)
    while pending:
        relative = pending.pop()
        path = root / relative
        if not path.is_file():
            continue
        text = path.read_text(encoding='utf-8')
        if path.suffix == '.py':
            tree = ast.parse(text)
            docs = {id(n.value) for n in ast.walk(tree) if isinstance(n, ast.Expr)
                    and isinstance(n.value, ast.Constant) and isinstance(n.value.value, str)}
            text = '\n'.join(n.value for n in ast.walk(tree)
                             if isinstance(n, ast.Constant) and isinstance(n.value, str) and id(n) not in docs)
            names = []
            for node in ast.walk(tree):
                if isinstance(node, (ast.Import, ast.ImportFrom)):
                    names.extend(n.name.split('.')[-1] + '.py' for n in node.names)
                    if isinstance(node, ast.ImportFrom) and node.module:
                        names.append(node.module.split('.')[-1] + '.py')
            text += '\n' + '\n'.join(names)
        elif path.suffix == '.json':
            commands = []
            def walk(value):
                if isinstance(value, dict):
                    for key, child in value.items():
                        if key in ('comando', 'comandi', 'comandi_aggiuntivi'):
                            commands.append(json.dumps(child))
                        else:
                            walk(child)
                elif isinstance(value, list):
                    for child in value:
                        walk(child)
            walk(strict_json.loads(text))
            text = '\n'.join(commands)
        else:
            text = '\n'.join(line for line in text.splitlines() if not line.lstrip().startswith('#'))
        added = references(text) - reached
        reached |= added
        pending.extend(added)
    return reached


def verify(root: Path, data: dict) -> list[str]:
    errors = check_checkpoint(root) + check_multigeometry(root)
    reached = pr_scripts(root, data['workflows'])
    if not data["python_tests"]:
        errors.append("discovery Python vuota")
    for registry, consumers in data["registries"].items():
        if not consumers and not registry.endswith("/io-geometrie-multiple.json"):
            errors.append(f"registro senza consumatore eseguibile: {registry}")
        elif consumers and not set(consumers).intersection(reached):
            errors.append(f"registro senza verificatore collegato ai comandi PR: {registry}: {consumers}")
    manifest = tomllib.loads((root / "fuzz/Cargo.toml").read_text(encoding="utf-8"))
    targets = [entry["path"] for entry in manifest["bin"]]
    actual = sorted(p.relative_to(root / "fuzz").as_posix() for p in (root / "fuzz/fuzz_targets").glob("*.rs")
                    if "fuzz_target!" in p.read_text(encoding="utf-8"))
    if len(targets) != len(set(targets)) or sorted(targets) != actual:
        errors.append(f"fuzz: manifest e sorgenti divergenti: {targets} != {actual}")
    for name, workflow in data["workflows"].items():
        for job_name, job in workflow.get("jobs", {}).items():
            if "matrix" in job.get("strategy", {}) and job["strategy"].get("fail-fast") != "false":
                errors.append(f"{name}:{job_name}: matrice senza fail-fast: false")
            for step in job.get("steps", []):
                run = step.get("run", "")
                for script in re.findall(r"(?<![\w/.-])(scripts/[\w./-]+\.(?:py|sh|ps1))\b", run):
                    if not (root / script).is_file():
                        errors.append(f"{name}:{job_name}: script assente {script}")
    ci = data["workflows"].get(".github/workflows/ci.yml", {})
    job = ci.get("jobs", {}).get("prove-fonte-unica", {})
    if ("pull_request" not in ci.get("on", {}) or ci["on"]["pull_request"]
            or "if" in job or job.get("needs")):
        errors.append("il gate fonte unica deve essere indipendente su ogni PR")
    runs = [s.get("run", "") for s in job.get("steps", [])]
    if not any("python scripts/check_prove_fonte_unica.py --run" == s.strip() for s in runs):
        errors.append("job PR senza esecuzione completa del gate fonte unica")
    native_steps = ci.get("jobs", {}).get("rust", {}).get("steps", [])
    if not any("python3 scripts/check_prove_fonte_unica.py --native-tests" == s.get("run", "").strip()
               and s.get("if") == "${{ !cancelled() }}" for s in native_steps):
        errors.append("prove native senza discovery indipendente nel job Rust su PR")
    # I runner di sonde non possono tornare a vivere soltanto nei registri.
    for script, job_id in (("check_footer_field_conflicts.py", "profilo-pubblico"),
                           ("compare_historical_readers.py", "historical-readers"),
                           ("check_fixture_generators.py", "prove-fonte-unica")):
        target = ci.get("jobs", {}).get(job_id, {})
        if "if" in target or not any("scripts/" + script in s.get("run", "")
                                    for s in target.get("steps", [])):
            errors.append(f"{script}: esecuzione PR assente")
    return errors


def run_all(commands: list[list[str]], directory: Path, root: Path = ROOT) -> list[dict]:
    """Conserva ogni log; errore di avvio/timeout non tronca la raccolta."""
    directory.mkdir(parents=True, exist_ok=True)
    results = []
    for index, command in enumerate(commands):
        log = directory / f"{index:03d}.log"
        with log.open("w", encoding="utf-8") as output:
            try:
                code = subprocess.run(command, cwd=root, stdout=output,
                                      stderr=subprocess.STDOUT, timeout=600).returncode
            except (OSError, subprocess.TimeoutExpired) as exc:
                output.write(str(exc))
                code = 125
        record = {"command": command, "exit": code, "log": str(log)}
        if command[1:3] == ["-m", "unittest"]:
            counts = re.findall(r"^Ran (\d+) tests? in ", log.read_text(encoding='utf-8', errors='replace'), re.M)
            record['unittest_cases'] = int(counts[-1]) if counts else 0
            if code in (0, 5) and not record['unittest_cases']:
                code = record['exit'] = 126
                with log.open('a', encoding='utf-8') as output:
                    output.write('\nDiscovery vuota: zero test non e un verde.\n')
        results.append(record)
        print(f"{'OK' if code == 0 else 'FAIL'} {' '.join(command)}", flush=True)
        (directory / "results.json").write_text(json.dumps(results, indent=2) + "\n", encoding="utf-8")
    return results


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--run", action="store_true", help="esegue tutti i self-test e i gate statici")
    parser.add_argument("--native-tests", action="store_true", help="sonde con build o eseguibili Unix nel job Rust")
    args = parser.parse_args()
    if args.native_tests:
        commands = [[sys.executable, "-m", "unittest", ".".join(p.relative_to(ROOT).with_suffix("").parts)]
                    for p in python_tests(ROOT, native=True)]
        commands += [["bash", p.relative_to(ROOT).as_posix()]
                     for p in sorted((ROOT / 'scripts').rglob('test_*.sh'))]
        results = run_all(commands, ROOT / "target/prove-native")
        return int(not results or any(r['exit'] for r in results))
    directory = ROOT / "target/prove-fonte-unica"
    directory.mkdir(parents=True, exist_ok=True)
    try:
        data = inventory(ROOT)
        errors = verify(ROOT, data)
    except Exception as exc:
        # Un inventario malformato e' un rosso, ma i self-test e gli altri
        # verificatori sono indipendenti e devono ancora essere eseguiti.
        data = {"inventory_error": f"{type(exc).__name__}: {exc}"}
        errors = [data['inventory_error']]
    (directory / "inventory.json").write_text(json.dumps(data, indent=2) + "\n", encoding="utf-8")
    for error in errors:
        print(error, file=sys.stderr)
    results = []
    if args.run:
        # Un processo per modulo evita che patch/globali di una sonda
        # contaminino la successiva; ogni modulo viene scoperto dai file.
        commands = [[sys.executable, "-m", "unittest", ".".join(p.relative_to(ROOT).with_suffix("").parts)]
                    for p in python_tests(ROOT)]
        commands += [[sys.executable, "scripts/" + cmd[0], *cmd[1:]] for cmd in STATIC]
        results = run_all(commands, directory)
    return int(bool(errors) or any(r["exit"] for r in results))


if __name__ == "__main__":
    raise SystemExit(main())
