#!/usr/bin/env python3
"""Check actual emitted native references, including APIs hidden by Rust cfg.

GDAL_VERSION must identify the installed build dependency (gdal-config in CI).
For a compile-only cross-version audit it may select older prebuilt bindings;
that is not a claim of a runtime test against that GDAL release.
"""

import json
import os
from pathlib import Path
import re
import subprocess
import shlex


def main():
    version = tuple(map(int, os.environ["GDAL_VERSION"].split(".")[:2]))
    metadata = json.loads(subprocess.check_output(
        ["cargo", "metadata", "--format-version=1", "--locked", "--features", "gdal-backend"], text=True
    ))
    sys_package = next(p for p in metadata["packages"] if p["name"] == "gdal-sys")
    bindings = Path(sys_package["manifest_path"]).parent / "prebuilt-bindings"
    functions = {}
    for directory in bindings.iterdir():
        path = directory / "gdal_x86_64-unknown-linux-gnu.rs"
        if path.exists():
            functions[tuple(map(int, directory.name.split("_")))] = set(
                re.findall(r"pub fn (\w+)", path.read_text(encoding="utf-8"))
            )
    known = set.union(*functions.values())
    source = "\n".join(
        p.read_text(encoding="utf-8")
        for root in ("vendor/gdal/src", "crates/driver-filegdb")
        for p in Path(root).rglob("*.rs")
    )
    used = set(re.findall(r"\b\w+\b", source)) & known
    introduced = {
        name: min(v for v, names in functions.items() if name in names)
        for name in used - functions[(3, 4)]
    }
    # Cargo JSON identifies this invocation's artifact, avoiding stale .ll files.
    env = os.environ.copy()
    flags = (env["CARGO_ENCODED_RUSTFLAGS"].split("\x1f")
             if "CARGO_ENCODED_RUSTFLAGS" in env
             else shlex.split(env.get("RUSTFLAGS", "")))
    env["CARGO_ENCODED_RUSTFLAGS"] = "\x1f".join([*flags, "--emit=llvm-ir"])
    output = subprocess.check_output([
        "cargo", "build", "-p", "driver-filegdb", "--lib", "--locked",
        "--features", "gdal-backend", "--message-format=json",
    ], text=True, env=env)
    artifacts = [json.loads(line) for line in output.splitlines() if line.startswith("{")]
    artifact = next(a for a in artifacts if a.get("reason") == "compiler-artifact"
                    and a["target"]["name"] == "gdal")
    rlib = next(Path(f) for f in artifact["filenames"] if f.endswith(".rlib"))
    ir = rlib.with_name(rlib.stem.removeprefix("lib") + ".ll").read_text(encoding="utf-8")
    references = set(re.findall(r"^declare\b[^\n]*@([A-Za-z0-9_]+)\(", ir, re.M)) & known
    if not {"GDALOpenEx", "OGR_F_GetFieldAsDateTimeEx"} <= references:
        raise SystemExit("Missing baseline references: invalid or incomplete LLVM audit")
    unsupported = references - functions[version]
    if unsupported:
        raise SystemExit(f"GDAL {version}: incompatible native references: {sorted(unsupported)}")
    # Positive control: the new API must not disappear on supported versions.
    if ("OGR_Fld_GetTZFlag" in references) != (version >= (3, 8)):
        raise SystemExit("Timezone accessor does not match the detected build GDAL version")
    print(f"GDAL {version}: {len(used)} source APIs, {len(references)} emitted references: OK")
    for name, since in sorted(introduced.items()):
        print(f"{name}: since {since}, emitted={name in references}")


if __name__ == "__main__":
    main()
