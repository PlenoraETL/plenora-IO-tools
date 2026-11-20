import unittest

from scripts.audit_loss_enumeration import audit, causes, index


def cell(status, failures=()):
    return dict(driver="example", direction="Read", element="Utf8",
                expected="Preserved", status=status, failures=list(failures))


class AuditTests(unittest.TestCase):
    def test_missing_evidence_never_becomes_a_pass(self):
        for status in ("failed", "missing-generator", "missing-native-witness", "not-executed"):
            with self.subTest(status=status):
                result = audit([cell("passed")], [cell(status)])
                self.assertEqual(result["baseline_positives_now_passed"], 0)
                self.assertEqual(len(result["remaining"]), 1)

    def test_duplicate_cells_are_rejected(self):
        with self.assertRaisesRegex(ValueError, "duplicate"):
            index([cell("passed"), cell("failed")])

    def test_changed_product_is_rejected(self):
        with self.assertRaisesRegex(ValueError, "product changed"):
            audit([cell("passed")], [])

    def test_real_row_loss_is_not_classified_only_as_comparator(self):
        row = cell("failed", [dict(observed=["UNMAPPED"], declared=[],
                                  differences='[Difference { detail: "row count" }]')])
        self.assertEqual(causes(row), [{"cause": "perdita vera non dichiarata", "rule": "row-cardinality"}])

    def test_unknown_failures_remain_unclassified(self):
        result = audit([cell("failed", [{"new_failure": True}])], [cell("passed")])
        self.assertEqual(result["unclassified_baseline_failures"], 1)


if __name__ == "__main__":
    unittest.main()
