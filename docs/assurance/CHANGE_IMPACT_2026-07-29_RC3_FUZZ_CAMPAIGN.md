# RC3 fuzz campaign change impact analysis

Date: 2026-07-29

Status: release-evidence repeat passed with no findings

Scope:

- the six libFuzzer targets in `fuzz/fuzz_targets`;
- the structured differential driver in `crates/plenora-fuzz`;
- WKB/EWKB extended-type decoding introduced for RC3;
- run-specific isolation of logs, artifacts, and structured findings.

## Change

`scripts/fuzz-campaign.sh` now assigns every execution a UTC run identifier
and writes each target's outputs beneath that run. libFuzzer receives an
explicit, run-specific artifact prefix. This prevents a later campaign from
silently mixing evidence or crash artifacts with an earlier one.

The finite traversal functions used by the fuzz oracles now cover the extended
WKB geometry families. This keeps adversarial nesting bounded after adding the
canonical curve and surface types.

## Hazards and controls

| Hazard | Control |
| --- | --- |
| Finding attributed to the wrong revision | Start the long campaign only from a committed, pushed RC3 baseline and record its full revision. |
| Findings from separate campaigns are mixed | Per-run log, artifact, and structured-finding directories. |
| Extended child geometries evade the finite-work oracle | Recursive traversal covers all new aggregate variants and remains subject to decoder depth/count limits. |
| A smoke run is mistaken for assurance evidence | The development manifest says `long_campaign_pending`; smoke results do not satisfy the RC3 campaign gate. |
| Concurrent fuzz targets exhaust the host | Campaign RSS limit is explicit and the container remains observable and terminable. |

## Smoke evidence

Run identifier: `20260729T111852Z`

Environment:

- image: `plenora-rust:nightly-fuzz-ready`;
- cargo-fuzz: `0.13.2`;
- toolchain: `nightly-2026-07-21`;
- duration: 10 seconds per target;
- RSS limit: 1024 MiB.

Results:

| Target | Executions | Findings |
| --- | ---: | ---: |
| `from_wkb` | completed | 0 |
| `geojson_reader` | 93,681 | 0 |
| `wkt_parse` | 33,482 | 0 |
| `kml_reader` | 190,747 | 0 |
| `shp_wkb` | 18,909 | 0 |
| `dxf_reader` | 538,637 | 0 |
| structured differential driver | 19,260,000 | 0 |

The smoke run was a launch/readiness check only. It demonstrates that every
target builds, starts, remains within its configured bounded-work model, and
places evidence in the expected isolated location. It is not the long RC3
campaign.

## Diagnostic long run

The first 3,600-second run completed without a technical finding, but the
post-run working-tree check found that two finite-work oracle updates had not
been included in the baseline commit:

- `fuzz/fuzz_targets/from_wkb.rs`, SHA-256
  `759421a3d6249cf6898f3f4ca58ef99c975cd493b7f42c320716276cfc741ee9`;
- `fuzz/fuzz_targets/wkt_parse.rs`, SHA-256
  `6af6c2adeaa8efe5db0c21f2d987fdd26ca5d6f6b8ae0d71fd9f39edff16c9af`.

The library sources were at the committed baseline, but the full fuzz harness
was not. The run is therefore retained as diagnostic evidence and MUST NOT be
used as RC3 release evidence. A clean committed-baseline repeat is required.

Baseline:

- library revision: `f8a89170785c938a9105deae6cc479576abb969a`;
- pushed branch: `main`;
- CI run: `30447756574`, successful on Linux, Windows, macOS and coverage;
- run identifier: `20260729T113223Z`;
- duration: 3,600 seconds per target;
- container exit code: 0;
- provenance status: `not_release_evidence_uncommitted_harness`.

Results:

| Target | Executions | New corpus units | Peak RSS MiB | Findings |
| --- | ---: | ---: | ---: | ---: |
| `from_wkb` | 47,195,033 | 1,401 | 523 | 0 |
| `geojson_reader` | 9,846,500 | 7,196 | 516 | 0 |
| `wkt_parse` | 19,669,244 | 2,891 | 514 | 0 |
| `kml_reader` | 7,399,741 | 14,467 | 513 | 0 |
| `shp_wkb` | 183,799,884 | 1,770 | 520 | 0 |
| `dxf_reader` | 3,458,829 | 14,341 | 518 | 0 |
| **libFuzzer total** | **271,369,231** | **42,066** | — | **0** |
| structured differential driver | 5,720,360,000 | — | — | 0 |

The run-specific artifact and structured-finding directories contain zero
files. Every target reached its time limit normally; there were no crashes,
timeouts, out-of-memory exits, sanitizer findings, or structured differential
findings. These results do not override the provenance defect.

## Performance and compatibility

The campaign script affects assurance execution only; it is absent from
runtime library paths. Production performance and public APIs are unchanged.

## Valid committed-baseline repeat

The required repeat ran from a clean working tree after the finite-work oracle
updates were committed and pushed.

Baseline:

- revision: `2353e32da15cf25537a79c3a7dd507054c013764`;
- CI run: `30457744328`, successful on Linux, Windows, macOS and coverage;
- run identifier: `20260729T181308Z`;
- duration: 3,600 seconds per target;
- container image:
  `sha256:bf24b399447ea3b8cd68c4d248cf42bc6bc0f9a7e895c50f2b55ab58cbfbbe65`;
- working tree: clean before and after;
- container exit code: 0.

Results:

| Target | Executions | New corpus units | Peak RSS MiB | Findings |
| --- | ---: | ---: | ---: | ---: |
| `from_wkb` | 28,848,680 | 516 | 517 | 0 |
| `geojson_reader` | 4,533,100 | 2,865 | 501 | 0 |
| `wkt_parse` | 10,981,930 | 1,424 | 530 | 0 |
| `kml_reader` | 4,823,944 | 4,864 | 514 | 0 |
| `shp_wkb` | 159,272,378 | 719 | 518 | 0 |
| `dxf_reader` | 1,658,014 | 6,406 | 520 | 0 |
| **libFuzzer total** | **210,118,046** | **16,794** | — | **0** |
| structured differential driver | 5,705,840,000 | — | — | 0 |

Both run-specific finding locations contain zero files. Every process reached
the configured duration and returned normally. This repeat closes the RC3
long-fuzz workstream for the recorded component revision.

It does not qualify the three-component system, replace an independent review,
or satisfy the external native-Windows GDAL/FileGDB gate.
