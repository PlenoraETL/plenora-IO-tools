import sys
from pathlib import Path
import unittest
sys.path.insert(0, str(Path(__file__).resolve().parent))
from check_tempo_remoto import violations
class TemporalRules(unittest.TestCase):
    def test_rejects_new_sites_and_aliases(self):
        for source in ["NaiveDate::from_ymd(year, month, day)", "TimeDelta::milliseconds(input)", "Duration::from_secs_f64(input)", "chrono::Duration::seconds(input)",
                       "use chrono::Duration as Delta;\nDelta::days(input)",
                       "use chrono::{DateTime, Duration as Delta};\nDelta::days(input)",
                       "Utc.timestamp_opt(input, 0).unwrap()", "DateTime::from_timestamp(input, 0).expect(\"date\")"]:
            self.assertTrue(violations(source), source)
    def test_accepts_checked_conversion(self):
        self.assertFalse(violations("Duration::try_from_secs_f64(input)?; TimeDelta::try_seconds(input).ok_or(error)?"))
    def test_excludes_test_module(self):
        self.assertFalse(violations("#[cfg(test)]\nmod tests {\n Duration::from_secs_f64(input);\n}"))
