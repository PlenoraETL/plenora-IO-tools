"""Conservative temporal rule gate, using the repository production/test classifier.
Not a taint proof: rejects infallible constructors even before external reachability.
Vendor scope includes the production modules of the three corrected forks.
Legacy raw INT96 wrapping is outside these constructor rules; IO rejects INT96. Tests mutate rules, not line numbers.
"""
from pathlib import Path
import re
import perimetro_dei_sorgenti as perimetro
from check_prevalidazione_decoder import righe_di_test
ROOT = Path(__file__).resolve().parents[1]
RULES = (
    ("calendar constructor must be fallible", r"\b(?:NaiveDate|NaiveTime|NaiveDateTime)::(?:from_ymd|from_yo|from_isoywd|from_hms|from_hms_milli|from_hms_micro|from_hms_nano|from_timestamp)\s*\("),
    ("float duration must be fallible", r"\b(?:\w*Duration|TimeDelta)::from_secs_f(?:32|64)\s*\("),
    ("chrono duration must be fallible", r"\b(?:Duration|ChronoDuration|TimeDelta|chrono::Duration)::(?:milliseconds|seconds|minutes|hours|days|weeks)\s*\("),
    ("calendar conversion must handle None", r"\.(?:timestamp_opt|timestamp_millis_opt|timestamp_micros|with_ymd_and_hms)\([^;]*?\.\s*(?:unwrap|expect)\s*\("),
    ("calendar conversion must handle None", r"::(?:from_timestamp\w*|from_ymd_opt|from_num_days_from_ce_opt)\([^;]*?\.\s*(?:unwrap|expect)\s*\("),
)
def violations(text):
    text = re.sub(r"#\[cfg\(test\)\]\s*(?:use [^;]+;|mod \w+;)", "", text)
    excluded = righe_di_test(text)
    text = "\n".join("" if n in excluded or l.lstrip().startswith("//") else l for n,l in enumerate(text.splitlines()))
    # Imported chrono Duration, including an alias, has the chrono domain.
    rules = list(RULES)
    for alias in re.findall(r"\bDuration(?: as (\w+))?\s*[,;}]", text):
        rules.append(("chrono duration must be fallible", rf"\b{alias or 'Duration'}::(?:milliseconds|seconds|minutes|hours|days|weeks)\s*\("))
    return [reason for reason, pattern in rules if re.search(pattern, text)]
def verify():
    files = set()
    for name in ("dxf", "gdal", "parquet"):
        product, _ = perimetro.classifica(ROOT / "vendor" / name)
        files.update(product)
    for crate in perimetro.crates(frozenset({"plenora-fuzz", "plenora-bench"})):
        product, _ = perimetro.classifica(crate)
        files.update(product)
    errors = [f"{p.relative_to(ROOT)}: {v}" for p in sorted(files) for v in violations(p.read_text(encoding="utf-8"))]
    for name in ("HeaderVariablesSpec.xml", "ObjectsSpec.xml"):
        p = ROOT / "vendor/dxf/spec" / name
        if re.search(r'ReadConverter="(?:as_datetime_\w+|as_duration)\([^"?]*\)"', p.read_text()):
            errors.append(f"{p}: temporal converter must propagate Result")
    return errors
if __name__ == "__main__":
    errors = verify()
    print("\n".join(errors) if errors else "temporal checked conversion rules: OK")
    raise SystemExit(bool(errors))
