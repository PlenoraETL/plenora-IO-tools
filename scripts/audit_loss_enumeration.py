"""Compare generated loss-enumeration runs without treating missing cells as passes."""
import argparse
from collections import Counter
import json
from pathlib import Path


def key(row):
    return row["driver"], row["direction"], row["element"]


def index(rows):
    result = {key(row): row for row in rows}
    if len(result) != len(rows):
        raise ValueError("duplicate enumeration cell")
    return result


def causes(row):
    """Classify evidence by discrepancy, not by a list of individual cells."""
    found = set()
    for failure in row.get("failures", []):
        differences = failure.get("differences", "")
        if 'detail: "row count"' in differences:
            found.add(("perdita vera non dichiarata", "row-cardinality"))
        if failure.get("unexpected_acceptance"):
            found.add(("cella della tabella sbagliata", "accepted-native-representation"))
        if "unexpected_rejection" in failure:
            error = failure["unexpected_rejection"].get("error", {})
            if row["expected"] == "Preserved" and "non finita" in error.get("message", ""):
                found.add(("cella della tabella sbagliata", "non-finite-point-outside-WKT-domain"))
            elif error.get("category") == "schema":
                found.add(("confrontatore che classifica male", "fixture-metadata-targeted-by-position"))
            else:
                found.add(("confrontatore che classifica male", "type-versus-value-and-variant-rejection"))
        if "observed" in failure:
            observed = set(failure["observed"])
            declared = set(failure["declared"])
            if "crs_id_not_preserved_derived" in observed - declared:
                found.add(("confrontatore che classifica male", "CRS-representation-category"))
            if "metadata " in differences:
                found.add(("confrontatore che classifica male", "Arrow-keys-versus-geometric-contract"))
            if "missing field" in differences:
                found.add(("confrontatore che classifica male", "omitted-attribute-and-its-metadata"))
            if "additional fields" in differences:
                found.add(("confrontatore che classifica male", "generated-native-fields"))
            if "WKB dimensions" in differences:
                found.add(("confrontatore che classifica male", "multipart-node-count-is-not-dimension-change"))
            if "geometry null" in differences:
                found.add(("confrontatore che classifica male", "empty-geometry-versus-missing-value"))
    return [{"cause": cause, "rule": rule} for cause, rule in sorted(found)]


def audit(baseline, current):
    old, new = index(baseline), index(current)
    if old.keys() != new.keys():
        raise ValueError("enumeration product changed")
    failed = [row for row in baseline if row["status"] == "failed"]
    positives = [row for row in baseline if row["status"] == "passed"]
    counts = Counter(row["status"] for row in current)
    classified = [{
        "cell": dict(zip(("driver", "direction", "element"), key(row))),
        "causes": causes(row),
        "current_status": new[key(row)]["status"],
        "baseline_evidence": row["failures"],
        "current_evidence": new[key(row)].get("failures", []),
        "variant_constraints": new[key(row)].get("variant_constraints", {}),
    } for row in failed]
    priority = ["perdita vera non dichiarata", "cella della tabella sbagliata",
                "osservatore che dichiara senza confronto", "confrontatore che classifica male"]
    primary = Counter({cause: 0 for cause in priority})
    for item in classified:
        labels = {cause["cause"] for cause in item["causes"]}
        item["primary_cause"] = next((cause for cause in priority if cause in labels), None)
        if item["primary_cause"] is not None:
            primary[item["primary_cause"]] += 1
    return {
        "total": len(current),
        "counts": dict(sorted(counts.items())),
        "baseline_failures": len(failed),
        "baseline_failures_now_passed": sum(new[key(row)]["status"] == "passed" for row in failed),
        "baseline_positives": len(positives),
        "baseline_positives_now_passed": sum(new[key(row)]["status"] == "passed" for row in positives),
        "unclassified_baseline_failures": sum(not item["causes"] for item in classified),
        "primary_causes": dict(primary),
        "classification": classified,
        "remaining": [row for row in current if row["status"] != "passed"],
    }


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("baseline", type=Path)
    parser.add_argument("current", type=Path)
    parser.add_argument("output", type=Path)
    args = parser.parse_args(argv)
    result = audit(json.loads(args.baseline.read_text(encoding="utf-8")),
                   json.loads(args.current.read_text(encoding="utf-8")))
    args.output.write_text(json.dumps(result, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
    print(json.dumps({k: v for k, v in result.items() if k not in {"classification", "remaining"}}))
    return int(bool(result["remaining"] or result["unclassified_baseline_failures"]))


if __name__ == "__main__":
    raise SystemExit(main())
