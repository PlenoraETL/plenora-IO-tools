"""Mutation tests for real feature-on FileGDB catalog evidence."""

from __future__ import annotations

import copy
import unittest

from scripts.check_filegdb_catalog import load_protocol, validate_catalog


class FileGdbCatalogEvidenceTests(unittest.TestCase):
    def setUp(self) -> None:
        self.protocol = load_protocol()
        self.catalog = {
            "status": "ok",
            "protocol_version": 2,
            "contract": "plenora-io-catalog-v2",
            "determinism": "byte_for_byte",
            "drivers": [
                {
                    "id": "geojson",
                    "available": True,
                    "hostile_input_hardened": False,
                    "spec_version_supported": None,
                    "required_feature": None,
                },
                {
                    "id": "filegdb",
                    "available": True,
                    "hostile_input_hardened": False,
                    "spec_version_supported": None,
                    "required_feature": "gdal-backend",
                },
            ],
        }

    def validate(self, catalog, *, expected_available: bool = True) -> list[str]:
        return validate_catalog(
            catalog,
            self.protocol,
            expected_available=expected_available,
        )

    def test_accepts_feature_on_runtime_available_evidence(self) -> None:
        self.assertEqual(self.validate(self.catalog), [])

    def test_rejects_probe_that_always_returns_false(self) -> None:
        catalog = copy.deepcopy(self.catalog)
        catalog["drivers"][1]["available"] = False
        self.assertTrue(self.validate(catalog))

    def test_accepts_explicitly_expected_read_only_runtime(self) -> None:
        catalog = copy.deepcopy(self.catalog)
        catalog["drivers"][1]["available"] = False

        self.assertEqual(
            self.validate(catalog, expected_available=False),
            [],
        )
        self.assertTrue(
            self.validate(self.catalog, expected_available=False),
        )

    def test_rejects_non_boolean_true_and_wrong_feature(self) -> None:
        for field, value in (
            ("available", 1),
            ("available", "true"),
            ("required_feature", "gdal"),
            ("required_feature", None),
        ):
            with self.subTest(field=field, value=value):
                catalog = copy.deepcopy(self.catalog)
                catalog["drivers"][1][field] = value
                self.assertTrue(self.validate(catalog))

    def test_rejects_missing_or_duplicate_filegdb_entry(self) -> None:
        missing = copy.deepcopy(self.catalog)
        missing["drivers"] = [missing["drivers"][0]]
        self.assertTrue(self.validate(missing))

        duplicate = copy.deepcopy(self.catalog)
        duplicate["drivers"].append(copy.deepcopy(duplicate["drivers"][1]))
        self.assertTrue(self.validate(duplicate))

    def test_rejects_envelope_identity_drift(self) -> None:
        for field, replacement in (
            ("status", "error"),
            # Valori **sbagliati** sotto il protocollo corrente: la sonda
            # verifica il rifiuto, quindi devono restare diversi da quelli
            # buoni. Erano 2 e `-v2` quando il corrente era il v1.
            ("protocol_version", 3),
            ("contract", "plenora-io-catalog-v3"),
            ("determinism", "best_effort"),
        ):
            with self.subTest(field=field):
                catalog = copy.deepcopy(self.catalog)
                catalog[field] = replacement
                self.assertTrue(self.validate(catalog))

    def test_rejects_missing_required_top_level_field(self) -> None:
        for field in ("status", "protocol_version", "contract", "determinism"):
            with self.subTest(field=field):
                catalog = copy.deepcopy(self.catalog)
                del catalog[field]
                self.assertTrue(self.validate(catalog))

    def test_rejects_current_producer_omitting_additive_fields(self) -> None:
        for driver_index, field in (
            (0, "available"),
            (0, "required_feature"),
            (1, "available"),
            (1, "required_feature"),
        ):
            with self.subTest(driver_index=driver_index, field=field):
                catalog = copy.deepcopy(self.catalog)
                del catalog["drivers"][driver_index][field]
                self.assertTrue(self.validate(catalog))

    def test_rejects_other_driver_claiming_a_required_feature(self) -> None:
        catalog = copy.deepcopy(self.catalog)
        catalog["drivers"][0]["required_feature"] = "gdal-backend"
        self.assertTrue(self.validate(catalog))

    def test_rejects_non_object_root_and_non_array_drivers(self) -> None:
        self.assertTrue(self.validate(["plenora-io-catalog-v2"]))
        self.assertTrue(self.validate("plenora-io-catalog-v2"))

        catalog = copy.deepcopy(self.catalog)
        catalog["drivers"] = {"filegdb": {"available": True}}
        self.assertTrue(self.validate(catalog))

        catalog = copy.deepcopy(self.catalog)
        catalog["drivers"].append("filegdb")
        self.assertTrue(self.validate(catalog))


if __name__ == "__main__":
    unittest.main()
