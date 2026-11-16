#!/usr/bin/env python3
"""IO 9: lavoro finito = ogni seme versionato, una volta, sotto ASan.

Il timeout e' un watchdog per input, non un criterio di successo. Nessuna
mutazione casuale: la profondita' e il confine GDAL hanno verificatori propri.
"""
from __future__ import annotations
import argparse
import hashlib
import glob
import platform
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import time
import tomllib

ROOT = Path(__file__).resolve().parents[1]
POLICY = "io-9-replay-v1"
REGISTRO = "assurance/registries/prove-release.json"


def digest(data):
    return hashlib.sha256(data).hexdigest()


def command_for(toolchain, target, files, target_dir=None):
    options = ["--target-dir", target_dir] if target_dir else []
    return ["cargo", "+" + toolchain, "fuzz", "run", *options,
            "--sanitizer", "address", target, *files, "--", "-runs=1",
            "-timeout=15", "-rss_limit_mb=2048"]


def inventory(root=ROOT):
    manifest = tomllib.loads((root / "fuzz/Cargo.toml").read_text(encoding="utf-8"))
    policy = json.loads((root / REGISTRO).read_text(encoding="utf-8"))
    result = {}
    for target in sorted(b["name"] for b in manifest["bin"]):
        directories = [root / "fuzz/seeds" / target]
        if target in policy["corpus_condiviso"]:
            directories.append(root / policy["corpus_condiviso"][target])
        files = {p.relative_to(root).as_posix(): digest(p.read_bytes())
                 for directory in directories for p in sorted(directory.rglob("*")) if p.is_file()}
        if not files:
            raise ValueError(f"{target}: nessun seme versionato")
        result[target] = files
    all_files = {p for files in result.values() for p in files}
    if not set(policy["finding_obbligatori"]) <= all_files:
        raise ValueError("finding storico obbligatorio assente")
    if not result:
        raise ValueError("manifest senza target")
    return result


def verify(data, expected, revision=None):
    errors = []
    if not expected:
        errors.append("inventario vuoto")
    if data.get("policy") != POLICY:
        errors.append("policy di replay assente: una campagna a tempo non e' questa prova")
    if data.get("inventario") != expected:
        errors.append("inventario semi incompleto o digest diversi")
    if revision and data.get("revisione") != revision:
        errors.append("revisione diversa")
    if data.get("esito") != "PASS":
        errors.append("replay non completato")
    if any(data.get(k) != [] for k in ("fermati_a_finding_noto", "falliti_su_finding_nuovo", "illeggibili")):
        errors.append("finding o esiti illeggibili presenti/non dichiarati")
    if data.get("bersagli_dichiarati") != sorted(expected) or data.get("hanno_finito") != sorted(expected):
        errors.append("target non tutti completati")
    if type(data.get("budget_esecuzioni")) is not int or data.get("budget_esecuzioni") != sum(map(len, expected.values())):
        errors.append("budget diverso dall'inventario")
    receipts = data.get("ricevute", {})
    if set(receipts) != set(expected):
        errors.append("ricevute target mancanti o estranee")
    for target, files in expected.items():
        receipt = receipts.get(target, {})
        if receipt.get("comando") != command_for(data.get("toolchain", ""), target, files, data.get("target_dir")):
            errors.append(f"{target}: comando/budget/strumentazione diversi")
        log = receipt.get("log", "")
        executed = re.findall(r"^Executed (.+) in [0-9]+ ms", log, re.M)
        # cargo-fuzz puo risolvere i percorsi prima di passarli al binario.
        # Accettare solo il suffisso completo di un input esplicitamente richiesto.
        executed = [next((p for p in files if name == p or name.replace('\\', '/').endswith('/' + p)), name)
                    for name in executed]
        if (receipt.get("exit") != 0 or sorted(executed) != sorted(files)
                or receipt.get("log_sha256") != digest(log.encode())
                or receipt.get("esecuzioni") != len(files)):
            errors.append(f"{target}: esecuzioni/log mancanti, duplicate o fallite")
    if not data.get("toolchain") or not data.get("ambiente"):
        errors.append("identita ambiente/toolchain assente")
    return errors


def environment(toolchain):
    libraries = sorted(set(glob.glob('/usr/lib/*/libgdal.so*') + glob.glob('/lib/*/libgdal.so*')))
    if not libraries:
        raise ValueError("libreria GDAL non identificata; qualifica ASan richiesta su Linux")
    return {"piattaforma": sys.platform, "machine": platform.machine(),
            "os_release": Path('/etc/os-release').read_text(),
            "rustc": subprocess.check_output(["rustc", "+" + toolchain, "-vV"], text=True),
            "gdal": subprocess.check_output(["gdal-config", "--version"], text=True).strip(),
            "gdal_sha256": sorted(set(digest(Path(p).read_bytes()) for p in libraries)),
            "config": {k: os.environ.get(k, '') for k in (
                'RUSTFLAGS', 'CARGO_ENCODED_RUSTFLAGS', 'CARGO_BUILD_TARGET',
                'ASAN_OPTIONS', 'LD_LIBRARY_PATH', 'GDAL_DATA', 'PROJ_DATA')}}


def reuse(source, expected, revision, toolchain, ambient):
    data = json.loads(source.read_text(encoding="utf-8"))
    errors = verify(data, expected, revision)
    if data.get('toolchain') != toolchain or data.get('ambiente') != ambient:
        errors.append('ricevuta CI fuzz: ambiente/toolchain diversi')
    if errors:
        raise ValueError('; '.join(errors))
    data['riuso'] = {'source_sha256': digest(source.read_bytes()), 'revisione': revision,
                     'nota': 'eseguito in CI; non rieseguito nel checkpoint'}
    return data


def execute(destination, root=ROOT):
    dirty = subprocess.check_output(["git", "status", "--porcelain", "--untracked-files=all"], cwd=root)
    if dirty.strip():
        raise ValueError("replay di release richiede albero pulito e semi committati")
    expected = inventory(root)
    toolchain = re.search(r"^PLENORA_RUST_NIGHTLY=(.+)$", (root / "scripts/toolchain-pins.env").read_text(), re.M)[1]
    revision = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=root, text=True).strip()
    data = {"schema_version": 2, "policy": POLICY, "revisione": revision,
            "inventario": expected, "budget_esecuzioni": sum(map(len, expected.values())),
            "esito": "NON ESEGUITO", "ricevute": {}, "toolchain": toolchain,
            "target_dir": os.environ.get("PLENORA_FUZZ_TARGET_DIR"),
            "ambiente": environment(toolchain),
            "bersagli_dichiarati": sorted(expected), "hanno_finito": [],
            "fermati_a_finding_noto": [], "falliti_su_finding_nuovo": [],
            "illeggibili": [], "secondi_per_bersaglio": None}
    def save():
        destination.parent.mkdir(parents=True, exist_ok=True)
        destination.write_text(json.dumps(data, indent=2) + "\n", encoding="utf-8")
    source = os.environ.get('S9_CI_FUZZ_RECEIPT')
    if source:
        data = reuse(Path(source), expected, revision, toolchain, data['ambiente'])
        save()
        print("RIUSATO CI, NON RIESEGUITO: " + json.dumps(data['riuso']))
        print(f"=== build strumentata (tutti i {len(expected)} target dichiarati) ===")
        print(f"replay completato: {data['budget_esecuzioni']} input rieseguiti su {len(expected)} target, nessun crash")
        return 0
    save()
    start = time.monotonic()
    options = (["--target-dir", os.environ["PLENORA_FUZZ_TARGET_DIR"]]
               if os.environ.get("PLENORA_FUZZ_TARGET_DIR") else [])
    build = subprocess.run(["cargo", "+" + toolchain, "fuzz", "build", *options, "--sanitizer", "address"], cwd=root)
    data["build_secondi"] = time.monotonic() - start
    if build.returncode:
        data["esito"] = "FAIL"
        save()
        return 1
    print(f"=== build strumentata (tutti i {len(expected)} target dichiarati) ===")
    for target, files in expected.items():
        start = time.monotonic()
        command = command_for(toolchain, target, files, data["target_dir"])
        # File espliciti: libFuzzer esegue ogni file una volta, senza fuzzing.
        # Il watchdog esterno comprende avvio e lettura; non produce PASS.
        try:
            run = subprocess.run(command, cwd=root, stdout=subprocess.PIPE,
                                 stderr=subprocess.STDOUT, text=True,
                                 timeout=60 + 20 * len(files))
            code, log = run.returncode, run.stdout
        except subprocess.TimeoutExpired as exc:
            code = 124
            log = exc.stdout or b""
            if isinstance(log, bytes):
                log = log.decode(errors="replace")
        print(log, end="")
        data["ricevute"][target] = {"exit": code, "log": log,
            "log_sha256": digest(log.encode()), "comando": command,
            "esecuzioni": len(re.findall(r"^Executed .+ in [0-9]+ ms", log, re.M)),
            "secondi": time.monotonic() - start}
        save()
    data["esito"] = "PASS"
    unchanged = (subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=root, text=True).strip() == revision
                 and not subprocess.check_output(["git", "status", "--porcelain", "--untracked-files=all"], cwd=root).strip()
                 and inventory(root) == expected)
    if not unchanged:
        data["esito"] = "FAIL"
    data["hanno_finito"] = sorted(expected)
    if verify(data, expected, revision):
        data["esito"] = "FAIL"
        data["hanno_finito"] = []
    save()
    if data["esito"] != "PASS":
        return 1
    print(f"replay completato: {data['budget_esecuzioni']} input rieseguiti su {len(expected)} target, nessun crash")
    return 0


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--execute", action="store_true")
    parser.add_argument("--check-policy", action="store_true")
    parser.add_argument("--receipt", type=Path, default=Path(os.environ.get(
        "PLENORA_VERBALE_CAMPAGNA", "target/fuzz-release/fuzz-smoke-ultima.json")))
    parser.add_argument("--revision")
    args = parser.parse_args(argv)
    try:
        if args.check_policy:
            from check_release_contract import verifica_policy_prove
            errors = verifica_policy_prove(ROOT)
            print("\n".join(errors) if errors else "policy prove IO 9 coerente")
            return int(bool(errors))
        if args.execute:
            source = os.environ.get('S9_CI_FUZZ_RECEIPT')
            if source and Path(source).resolve() == args.receipt.resolve():
                raise ValueError("ricevuta CI e destinazione devono essere distinte")
            # Non lasciare un PASS precedente se la preparazione fallisce.
            args.receipt.parent.mkdir(parents=True, exist_ok=True)
            args.receipt.write_text('{"esito":"NON ESEGUITO"}\n', encoding="utf-8")
            return execute(args.receipt)
        data = json.loads(args.receipt.read_text(encoding="utf-8"))
        errors = verify(data, inventory(), args.revision)
        if not args.revision:
            from check_release_contract import evidenza_ancora_valida
            errors += evidenza_ancora_valida(data.get("revisione", ""))
        if errors:
            print("\n".join(errors), file=sys.stderr)
            return 1
        n = len(data["inventario"])
        print(f"smoke fuzz completato senza finding su {n} target eseguiti (0 in quarantena, comunque compilati)")
        print("Ricevuta di replay ASan verificata; nessuna campagna a tempo eseguita.")
        return 0
    except (OSError, ValueError, subprocess.SubprocessError) as exc:
        print(str(exc), file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
