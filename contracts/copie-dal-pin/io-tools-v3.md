# Public profile: io-tools, version 3

Profile identifier: `plenora-io-tools-profile-v3`

Normative target catalog: [`io-tools-v3.json`](../catalogs/io-tools-v3.json)

It supersedes [profile version 2](io-tools-v2.md) as the current target
([decision 0015](../decisions/0015-intentional-absences-and-planned-surfaces.md)).
Its catalog lists the operation identities of version 2, unchanged, under
Public Catalogs 2.0 (`plenora-public-catalog-v2`); this profile states the
runtime as conditional under that contract. Versions 2 and 1
([decision 0011](../decisions/0011-io-tools-v2.md)) remain available
identities: their catalogs and contracts keep their meaning. An artifact
implements exactly one of the catalog versions (Surface Bindings 1.0 §1).

## Applicable contracts

- [Public Surfaces 1.0](../specs/surfaces/PUBLIC-SURFACES-1.0.md)
- [Capability Discovery 2.0](../specs/capabilities/CAPABILITY-DISCOVERY-2.0.md)
- [Typed Errors 1.0](../specs/errors/ERRORS-1.0.md)
- [Public Security 1.0](../specs/security/PUBLIC-SECURITY-1.0.md)
- [Arrow Interchange 1.0](../specs/data/ARROW-INTERCHANGE-1.0.md)
- [Arrow Vocabulary 1.0](../specs/data/ARROW-VOCABULARY-1.0.md)
- [Row Diagnostics 1.0](../specs/diagnostics/ROW-DIAGNOSTICS-1.0.md)
- [CLI 2.0](../specs/cli/CLI-2.0.md)
- [Python SDK 1.0](../specs/sdk/PYTHON-SDK-1.0.md), when exposed
- [Public Catalogs 2.0](../specs/catalogs/PUBLIC-CATALOGS-2.0.md), for the
  catalog of this profile
- [Runtime Binding 2.0](../specs/runtime/RUNTIME-BINDING-2.0.md), when exposed
- [Surface Bindings 1.0](../specs/surfaces/SURFACE-BINDINGS-1.0.md)
- [Composition 1.0](../specs/composition/COMPOSITION-1.0.md)

## Public purpose

The component exposes discovery, inspection, reading, writing and conversion of
external datasets through format-aware public contracts.

## Required operation families

The catalog includes `io.catalog`, `io.inspect`, `io.layers`, `io.read`,
`io.write` and `io.convert`. Three operations change version from profile 1:

| operation | version | output contract | what changes |
|---|---|---|---|
| `io.catalog` | 2 | `plenora-io-catalog-v2` | each sink states whether it requires declared geometry types (IO-CAT-001) |
| `io.read` | 2 | `plenora-io-read-result-v2` | the result names the delivered serialization, IPC stream or file (IO-SER-001) |
| `io.write` | 2 | `plenora-io-write-result-v2` | the result names the received serialization, IPC stream or file (IO-SER-001) |

`io.inspect`, `io.layers` and `io.convert` keep version 1 and their contracts.
The input contracts of the three new versions are those of version 1.

A released artifact MAY omit an operation that is not part of that artifact,
but MUST NOT advertise it as available.

Format identifiers are typed operation inputs and entries in the versioned
`io.catalog` result. Format-specific behavior MUST NOT be selected by parsing
file extensions when the operation requires an explicit format.

## Component-owned wire contracts

Before an artifact claims this profile, IO-tools MUST publish immutable schemas
and conformance examples for every operation pair the catalog names:

- `plenora-io-catalog-query-v1` and `plenora-io-catalog-v2`;
- `plenora-io-inspect-input-v1` and `plenora-io-inspect-v1`;
- `plenora-io-layers-input-v1` and `plenora-io-layers-v1`;
- `plenora-io-read-input-v1` and `plenora-io-read-result-v2`;
- `plenora-io-write-input-v1` and `plenora-io-write-result-v2`;
- `plenora-io-convert-input-v1` and `plenora-io-convert-v1`.

These schemas remain owned by IO-tools. This profile fixes their public
identifiers, roles and the cross-component meaning of the members below, not
their internal implementation.

The three version 2 schemas, `plenora-io-read-result-v2`,
`plenora-io-write-result-v2` and `plenora-io-catalog-v2`, are not published in
this repository: IO-tools publishes them, with their examples, in the release
that first claims profile version 2 or this profile. Until then no artifact
can claim either, and the meaning fixed below (IO-SER-001, IO-CAT-001, IO-NULL-001)
is what those schemas MUST encode.

When an IO error includes `details`, the value MUST conform to the
component-owned `plenora-io-error-details-v1` schema. Omitting `details`
remains valid when the four common error axes and optional `code` completely
express the failure.

**IO-SER-001** — `plenora-io-read-result-v2` names the content type of the
dataset it delivered, and `plenora-io-write-result-v2` the content type of the
dataset it received. Each is one of the two the catalog declares for that
operation, `application/vnd.apache.arrow.stream` or
`application/vnd.apache.arrow.file`, and it MUST be the serialization of the
bytes actually delivered or received, never a fixed value. The version 1
results describe the file container only; an artifact on profile version 1
that delivers or receives a stream records a deviation.

**IO-CAT-001** — In `plenora-io-catalog-v2`, every writable format carries the
boolean member `requires_declared_geometry_types`. When it is `true`, the sink
refuses, with a typed failure before anything is published, a dataset whose
geometry field carries `plenora.geometry.types_declaration: unresolved`
([Arrow Vocabulary 1.0 §3](../specs/data/ARROW-VOCABULARY-1.0.md)); when it is
`false`, the sink accepts such a dataset. A consumer predicts the refusal by
reading the catalog, without invoking `io.write`. The member says nothing
about which declared types the sink accepts, which stays in the geometry
support of the same entry.

**IO-NULL-001** — In the success results of every IO operation, a member whose
value is JSON `null` means "not applicable to this result": the quantity or
reference does not exist for this dataset, format or outcome. It never means
zero, empty, unknown or failed, and a consumer MUST NOT read it as any of
them. A value that is unknown or failed is reported through a typed member or
a typed failure (Typed Errors 1.0), not through `null`.

## Public surfaces

- Rust API: required.
- CLI: required and governed by CLI 2.0.
- Python SDK: optional (`conditional` in the catalog); when published it is
  governed by Python SDK 1.0 and the binding map, binds all six operations
  and exposes `plenora_io.version()` and `Client.capabilities()`.
- Runtime: conditional. An artifact that publishes the runtime surface
  follows Runtime Binding 2.0 for every operation it exposes there; an
  artifact without it omits `runtime` from its capability document and
  records no deviation, and the catalog's runtime entries are planned for it
  (CAT-001, CAT-002).

**IO-PY-001** — The Python SDK exposes the semantics of the CLI protocol
through synchronous methods; each Python artifact records `api_modes: ["sync"]`
in its adoption manifest (Python SDK 1.0 section 4). Execution may use the
native module shipped in an abi3 wheel (`NativeRunner`) or a CLI process.
`Client.backend` reports `"native"` or `"process"`; neither path changes
operation identity, validation, results, error axes or side effects.
`plenora_io.version()` returns the installed package version;
`Client.version()` describes the selected backend. `Client.capabilities()`
MUST describe that artifact, including the Python interface and the
`python_sdk` surface on the operations it exposes (CAT-001).

`Client.read` writes the dataset to the caller's destination. For `io.read`,
the capability attribute `plenora.surface_side_effects` MUST include
`{"python_sdk": "local"}` alongside any other surface effects (SB-001);
the catalog operation retains `side_effect: none`. `Client.validate` reads
without delivering a dataset. `Client.read_table` returns `TableRead`, with
both the PyArrow table and the structured read result, including fidelity;
its temporary IPC file is cleaned up on success or failure, with cleanup
failures reported as typed errors. All three methods bind `io.read` version 2.
`Client.write` accepts an IPC path or a supported Arrow object through the
optional PyArrow adapter. Without supported PyArrow, `read_table` and Arrow
object input MUST fail with a typed `unsupported` error before execution,
never silently convert or discard fidelity information.

Controls MUST describe their actual scope. The native backend's client
timeout requests cooperative cancellation and waits for the worker to stop;
it is not a hard bound on return time. A client timeout is a `ProtocolError`,
distinct from the operation deadline in `Limits`. SIGINT handled on the main
thread requests cancellation (also on Windows with the native backend); an
operation's cancellation result is exposed as `CancelledError`, with its
error axes, rather than replaced by an untyped interruption. Cancellation
availability is reported by `Client.cancellable`; the process backend has
platform and thread restrictions. Neither timeout nor cancellation proves
rollback: a client timeout or protocol failure after a writing command has
started MUST preserve conservative `remote_effect: unknown` and `retry: never`;
a typed operation failure preserves the effect the operation reports.

## Interchange

`io.read` exposes tabular output through Arrow Interchange 1.0 when Arrow is
the declared representation. `io.write` and `io.convert` declare accepted
input and produced output content types.

The versioned `plenora-io-catalog-v2` result is the sole normative source for
format identifiers, accepted options, read/write availability, layer behavior,
geometry and CRS support, fidelity constraints and publish guarantees,
including IO-CAT-001. Capability `attributes` MUST NOT duplicate this matrix.

Loss, coercion or unsupported metadata that changes the public result MUST be
reported through a structured fidelity result or typed failure; it MUST NOT be
silent.

Row-scoped format or mapping failures use
`plenora-row-diagnostics-v1` when diagnostics are advertised.

**IO-ARROW-001** — An Arrow input whose schema carries neither
`plenora.contract.version` nor any key in the `plenora.` namespace, on the
schema or on any field, is an external Arrow source and not a Plenora
contract stream. Every IO operation that reads Arrow accepts it, although
Arrow Interchange 1.0 rejects a missing contract version with `schema`
(ARROW-001), and reads each missing declaration without asserting more than
the source carries. An input that carries the version or any `plenora.` key
is a contract stream and follows Arrow Vocabulary 1.0 and, where IO claims
it, Arrow Geometry Semantics 1.0 in full: a missing or malformed version is
then `schema` (REJ-001). Every Arrow stream IO produces declares the contract
version.

## External outcomes

Write and convert operations declare local or remote side effects and distinguish
complete publication, rollback, partial publication and unknown durability where
those states are observable.

## Cutover from profile version 1

Profile version 2, whose operations this profile keeps unchanged, was a
component major change for IO-tools: the three operations changed version and
output contract. CLI protocol v2 and the rules of profile version 1 on
deprecated command spellings continue to apply. From profile version 2 to
this profile no operation changes: the contracts 2.0, runtime applicability
and optional Python surface change.
## Not specified here

This profile does not prescribe drivers, parsers, temporary files, spooling,
batch size, GDAL integration or publish algorithms.
