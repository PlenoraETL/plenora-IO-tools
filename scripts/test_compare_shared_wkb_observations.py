from __future__ import annotations

import unittest

from scripts.compare_shared_wkb_observations import compare


class SharedWkbObservationTests(unittest.TestCase):
    def test_only_declared_capability_difference_is_accepted(self) -> None:
        manifest = {
            "corpus_id": "test",
            "cases": [
                {"path": "same", "expectation": "accepted"},
                {
                    "path": "different",
                    "expectation": "implementation_defined",
                    "known_difference": {
                        "dimension": "acceptance",
                        "classification": "intentional_capability_difference",
                        "reason": "different supported type sets",
                    },
                },
            ],
        }
        io_report = {
            "component": "io",
            "cases": [
                {
                    "path": "same",
                    "accepted": True,
                    "geometry_type": "point",
                    "dimensions": "xy",
                    "srid": None,
                },
                {"path": "different", "accepted": False},
            ],
        }
        database_report = {
            "component": "database",
            "cases": [
                {
                    "path": "same",
                    "accepted": True,
                    "geometry_type": "point",
                    "dimensions": "xy",
                    "srid": None,
                },
                {"path": "different", "accepted": True},
            ],
        }
        result = compare(manifest, io_report, database_report)
        self.assertEqual(result["status"], "pass")
        self.assertEqual(len(result["classified_differences"]), 1)

    def test_unclassified_difference_fails(self) -> None:
        manifest = {
            "corpus_id": "test",
            "cases": [{"path": "case", "expectation": "accepted"}],
        }
        io_report = {
            "component": "io",
            "cases": [{"path": "case", "accepted": False}],
        }
        database_report = {
            "component": "database",
            "cases": [{"path": "case", "accepted": True}],
        }
        result = compare(manifest, io_report, database_report)
        self.assertEqual(result["status"], "fail")
        self.assertEqual(len(result["errors"]), 1)


if __name__ == "__main__":
    unittest.main()
