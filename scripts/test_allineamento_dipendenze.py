"""Vincoli della finestra comune e politica del fork riutilizzabile."""

import pathlib
import tomllib
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[1]


class AllineamentoDipendenze(unittest.TestCase):
    def test_minimo_arrow_del_fork(self):
        with (ROOT / "vendor/parquet/Cargo.toml").open("rb") as stream:
            manifest = tomllib.load(stream)
        arrow = {
            name: spec["version"]
            for table in ("dependencies", "dev-dependencies")
            for name, spec in manifest[table].items()
            if name == "arrow" or name.startswith("arrow-")
        }
        self.assertEqual(len(arrow), 9)
        self.assertEqual(set(arrow.values()), {">=60.1.0, <61"})

    def test_tutti_i_lock(self):
        for relative in ("Cargo.lock", "fuzz/Cargo.lock", "vendor/parquet/Cargo.lock"):
            with self.subTest(lock=relative):
                with (ROOT / relative).open("rb") as stream:
                    packages = tomllib.load(stream)["package"]
                for package in packages:
                    name, version = package["name"], package["version"]
                    if name == "serde_json":
                        self.assertEqual(version, "1.0.154")
                    if name in ("thiserror", "thiserror-impl"):
                        self.assertIn(version, ("1.0.69", "2.0.21"))
                    if name == "rusqlite":
                        self.assertEqual(version, "0.41.1")
                    if name == "libsqlite3-sys":
                        self.assertGreaterEqual(tuple(map(int, version.split("."))), (0, 39, 0))
                    if name == "time":
                        self.assertEqual(version, "0.3.55")
                    if name.startswith("arrow-"):
                        self.assertEqual(version, "60.1.0")

    def test_consumatore_json_allineato_al_workspace(self):
        with (ROOT / "Cargo.toml").open("rb") as stream:
            workspace = tomllib.load(stream)["workspace"]["dependencies"]
        with (ROOT / "conformance/consumatore-rust/Cargo.toml").open("rb") as stream:
            consumer = tomllib.load(stream)["dependencies"]
        self.assertEqual(consumer["serde_json"], workspace["serde_json"])

    def test_codec_e_crc_espliciti(self):
        with (ROOT / "Cargo.toml").open("rb") as stream:
            parquet = tomllib.load(stream)["workspace"]["dependencies"]["parquet"]
        self.assertFalse(parquet["default-features"])
        self.assertEqual(set(parquet["features"]), {
            "arrow", "snap", "zstd", "crc", "brotli", "flate2-rust_backend", "lz4"
        })


if __name__ == "__main__":
    unittest.main()
