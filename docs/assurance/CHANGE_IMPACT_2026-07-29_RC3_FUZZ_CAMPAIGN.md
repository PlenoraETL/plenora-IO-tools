# RC3 fuzz campaign change impact analysis

Date: 2026-07-29

Status: smoke passed; long campaign pending a committed RC3 baseline

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

The smoke run is a launch/readiness check only. It demonstrates that every
target builds, starts, remains within its configured bounded-work model, and
places evidence in the expected isolated location. It is not the long RC3
campaign.

## Performance and compatibility

The campaign script affects assurance execution only; it is absent from
runtime library paths. Production performance and public APIs are unchanged.

## Next required evidence

1. Commit and push the RC3 baseline.
2. Run the long campaign without modifying its mounted source tree.
3. Record the exact revision, run identifier, durations, execution counts,
   findings, artifact hashes when applicable, and resource limits.
4. Re-run the release gate after updating the development manifest.

