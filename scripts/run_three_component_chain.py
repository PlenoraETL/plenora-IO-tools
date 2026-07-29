#!/usr/bin/env python3
"""Esegue una catena reale IO -> data -> database sui tre checkout fratelli."""

from __future__ import annotations

import argparse
import json
import pathlib
import subprocess
import tempfile


def run(command: list[str], cwd: pathlib.Path) -> None:
    print("+", " ".join(command), flush=True)
    subprocess.run(command, cwd=cwd, check=True)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--io-repo", type=pathlib.Path, default=pathlib.Path(__file__).resolve().parents[1])
    parser.add_argument("--data-repo", type=pathlib.Path, required=True)
    parser.add_argument("--database-repo", type=pathlib.Path, required=True)
    args = parser.parse_args()
    io_repo = args.io_repo.resolve()
    data_repo = args.data_repo.resolve()
    database_repo = args.database_repo.resolve()
    for repo in (io_repo, data_repo, database_repo):
        if not (repo / "Cargo.toml").is_file():
            raise SystemExit(f"workspace Rust non trovato: {repo}")

    with tempfile.TemporaryDirectory(prefix="plenora-three-component-") as directory:
        root = pathlib.Path(directory)
        io_output = root / "io.arrow"
        data_output = root / "data.arrow"
        plan = root / "plan.json"
        plan.write_text(
            json.dumps(
                {
                    "schema_version": 4,
                    "inputs": ["main"],
                    "nodes": [
                        {
                            "id": "positive",
                            "op": "table.filter",
                            "in": ["main"],
                            "config": {"column": "id", "operator": ">", "value": 0},
                        }
                    ],
                    "output": "positive",
                },
                separators=(",", ":"),
            ),
            encoding="utf-8",
        )

        run(
            [
                "cargo",
                "run",
                "--locked",
                "--manifest-path",
                str(io_repo / "conformance" / "three-component-chain" / "Cargo.toml"),
                "--",
                "generate",
                str(io_output),
            ],
            io_repo,
        )
        run(
            [
                "cargo",
                "run",
                "--locked",
                "-p",
                "plenora-cli",
                "--features",
                "proj-backend",
                "--",
                "run",
                "--plan",
                str(plan),
                "--inputs",
                str(io_output),
                "--output",
                str(data_output),
            ],
            data_repo,
        )
        run(
            [
                "cargo",
                "run",
                "--locked",
                "--manifest-path",
                str(io_repo / "conformance" / "three-component-chain" / "Cargo.toml"),
                "--",
                str(data_output),
            ],
            database_repo,
        )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
