#!/usr/bin/env python3
"""Policy Python derivata da .python-version; metadata, ABI, workflow e runtime."""
import argparse
from pathlib import Path
import re
import sys
import tomllib

ROOT = Path(__file__).resolve().parents[1]


def check(root: Path = ROOT, runtime: bool = False) -> list[str]:
    version = (root / ".python-version").read_text(encoding="utf-8").strip()
    if not re.fullmatch(r"3\.[0-9]+", version):
        return [".python-version: formato non valido"]
    major, minor = map(int, version.split("."))
    constraint = f">={version},<{major}.{minor + 1}"
    errors = []
    for name in ("sdk/python/pyproject.toml", "crates/plenora-io-py/pyproject.toml"):
        project = tomllib.loads((root / name).read_text(encoding="utf-8"))["project"]
        if project.get("requires-python") != constraint:
            errors.append(f"{name}: requires-python non allineato")
        versions = {c.removeprefix("Programming Language :: Python :: ") for c in project["classifiers"]
                    if re.fullmatch(r"Programming Language :: Python :: \d+\.\d+", c)}
        if versions != {version}:
            errors.append(f"{name}: classificatori non allineati")
    cargo = tomllib.loads((root / "crates/plenora-io-py/Cargo.toml").read_text(encoding="utf-8"))
    features = cargo["dependencies"]["pyo3"]["features"]
    if {f for f in features if f.startswith("abi3-py")} != {f"abi3-py{major}{minor}"}:
        errors.append("PyO3: ABI non allineata")
    for name in ("Dockerfile.dev", "Dockerfile.build-linux"):
        text = (root / name).read_text(encoding="utf-8")
        required = (
            "COPY .python-version scripts/install-python.sh /tmp/plenora-python/",
            "RUN bash /tmp/plenora-python/install-python.sh",
            'ENV PATH="/opt/plenora-python/bin:${PATH}"',
        )
        if any(line not in text for line in required):
            errors.append(f"{name}: interprete senza fonte unica")
    for path in (root / ".github/workflows").glob("*.yml"):
        text = path.read_text(encoding="utf-8")
        if re.search(r"(?m)^\s*python-version:", text) or re.search(r"(?m)^\s*python: \[", text):
            errors.append(f"{path.name}: versione Python duplicata")
        if text.count("uses: actions/setup-python@") != len(re.findall(r"python-version-file: .python-version\s*$", text, re.M)):
            errors.append(f"{path.name}: setup-python senza fonte unica")
    if runtime and sys.version_info[:2] != (major, minor):
        errors.append("interprete diverso dalla versione qualificata")
    return errors


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--runtime", action="store_true")
    args = parser.parse_args()
    errors = check(runtime=args.runtime)
    print("\n".join(errors) if errors else "Python: fonte unica, metadata, ABI e CI allineati")
    return bool(errors)


if __name__ == "__main__":
    raise SystemExit(main())
