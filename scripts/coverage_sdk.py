"""Run the existing SDK suite against the CLI and binding built with LLVM coverage.

The staged package lives under target: no extension is copied into SDK sources,
and no installed wheel (possibly built without instrumentation) is imported.
Run after cargo llvm-cov --no-report and before cargo llvm-cov report.
"""
from __future__ import annotations

import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile


def main() -> None:
    root = Path(__file__).resolve().parents[1]
    env = os.environ.copy()
    metadata = json.loads(subprocess.check_output(
        ["cargo", "metadata", "--no-deps", "--format-version", "1", "--locked"],
        cwd=root, env=env, text=True,
    ))
    target = Path(env.get("CARGO_LLVM_COV_TARGET_DIR",
                          str(Path(metadata["target_directory"]) / "llvm-cov-target")))
    env["CARGO_TARGET_DIR"] = str(target)
    # The unadorned show-env format is KEY=VALUE, including Windows paths and
    # Cargo's unit-separated flags. Do not interpret it as shell code.
    settings = subprocess.check_output(
        ["cargo", "llvm-cov", "show-env"], cwd=root, env=env, text=True,
    )
    for line in settings.split("\n"):
        if line.strip():
            key, value = line.rstrip("\r").split("=", 1)
            env[key] = value
    env["PYO3_PYTHON"] = sys.executable
    subprocess.run(
        ["cargo", "build", "--locked", "--workspace", "--all-features"],
        cwd=root, env=env, check=True,
    )
    # A fresh staging directory also prevents cached, removed Python modules
    # from being imported after a source change. Keep it as test evidence.
    stage = Path(tempfile.mkdtemp(prefix="coverage-sdk-", dir=target))
    # Repeated runs (and PID reuse after a reboot) must not reopen a profile
    # left by a terminated SDK process. Keep llvm-cov's directory and tokens,
    # but give this collection its own filename prefix.
    profile = Path(env["LLVM_PROFILE_FILE"])
    env["LLVM_PROFILE_FILE"] = str(profile.with_name(f"{stage.name}-{profile.name}"))
    package = stage / "plenora_io"
    shutil.copytree(root / "sdk/python/src/plenora_io", package,
                    dirs_exist_ok=True, ignore=shutil.ignore_patterns("__pycache__", "*.pyd", "*.so"))
    if sys.platform == "win32":
        library, module = "plenora_io_native.dll", "_native.pyd"
    elif sys.platform == "darwin":
        library, module = "libplenora_io_native.dylib", "_native.so"
    else:
        library, module = "libplenora_io_native.so", "_native.so"
    shutil.copy2(target / "debug" / library, package / module)
    if sys.platform == "win32":
        # Python's extension loader does not use PATH for dependent DLLs.
        # Use the CLI's first GDAL directory and this interpreter's ABI DLL.
        # Adding every PATH directory can accidentally load another Python's
        # python3.dll and fail with "procedure not found".
        gdal_bin = next(
            (str(Path(p).resolve()) for p in env.get("PATH", "").split(os.pathsep)
             if p and (Path(p) / "gdal.dll").is_file()),
            None,
        )
        dll_dirs = [sys.base_prefix] + ([gdal_bin] if gdal_bin else [])
        (stage / "sitecustomize.py").write_text(
            "import ctypes, os, pathlib, sys\n"
            f"_coverage_dll_handles = [os.add_dll_directory(p) for p in {dll_dirs!r}]\n"
            "_coverage_python_abi = ctypes.WinDLL(str(pathlib.Path(sys.base_prefix) / 'python3.dll'))\n",
            encoding="utf-8",
        )
    env["PYTHONPATH"] = str(stage)
    env["PLENORA_IO_BIN"] = str(target / "debug" / (
        "plenora-io.exe" if sys.platform == "win32" else "plenora-io"))
    env["PATH"] = str(target / "debug") + os.pathsep + env.get("PATH", "")
    env["PLENORA_IO_RICHIEDI_NATIVO"] = "1"
    env["PYTHONDONTWRITEBYTECODE"] = "1"
    print(f"Instrumented CLI: {env['PLENORA_IO_BIN']}", flush=True)
    subprocess.run(
        [sys.executable, "-c", "import pyarrow; import plenora_io._native as n; "
         "print(n.__file__); print('PyArrow', pyarrow.__version__)"],
        cwd=stage, env=env, check=True,
    )
    subprocess.run(
        [sys.executable, "-m", "unittest", "discover", "-s", ".", "-p", "test_*.py", "-v"],
        cwd=root / "sdk/python/tests", env=env, check=True,
    )


if __name__ == "__main__":
    main()
