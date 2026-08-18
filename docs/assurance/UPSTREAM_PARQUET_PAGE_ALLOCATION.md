# Segnalazione a monte — `parquet`: allocazione guidata da `uncompressed_page_size`

Stato: **pubblicata** il 2026-08-18 — apache/arrow-rs#10734,
https://github.com/apache/arrow-rs/issues/10734

Aperta con l'account **PlenoraETL**, indicato esplicitamente in autorizzazione.

Terza segnalazione della serie, indipendente dalle due già pubblicate
(`calamine` e `arrow-rs`).

Il testo sotto la linea è quello da inviare, ripulito da ogni riferimento
interno: nessun percorso, nessun nome di componente, nessun artefatto del
nostro fuzzing. Il reproducer è costruito da zero e non contiene input del
nostro corpus.

## Rapporto con la mitigazione locale

La correzione a monte **non sostituisce** la prevalidazione che abbiamo:
chiuderebbe questo difetto, non la classe. Si toglie per decisione separata.

---

## `SerializedPageReader` allocates `uncompressed_page_size` bytes before validating it against the column chunk

**Version:** 59.1.0
**File:** `src/file/serialized_reader.rs`

### Summary

When decompressing a page, `decode_page` reserves the full declared
uncompressed size in one allocation (line 447):

```rust
let uncompressed_page_size = usize::try_from(page_header.uncompressed_page_size)?;
...
let mut decompressed = Vec::with_capacity(uncompressed_page_size);
```

`verify_page_size` (line 900) is the only check that runs before this:

```rust
if compressed_size < 0 || compressed_size as u64 > remaining_bytes || uncompressed_size < 0 {
    return Err(eof_err!("Invalid page header"));
}
```

It bounds `compressed_page_size` by the bytes remaining in the column chunk,
but `uncompressed_page_size` is only checked for being non-negative. Nothing
relates it to `ColumnMetaData::total_uncompressed_size`, even though the format
makes the sum of the pages' uncompressed sizes equal to that total — so a
single page can never legitimately exceed it.

A file can therefore declare a column chunk of a few dozen bytes and a page
header claiming up to `i32::MAX` uncompressed bytes, and the reader will try to
allocate about 2 GiB. The consistency check happens afterwards, at line 456:

```rust
if decompressed.len() != uncompressed_page_size { ... }
```

i.e. after the allocation has already been requested.

### Why it is worth fixing rather than leaving to the caller

The failure mode is not a recoverable error. On allocation failure Rust invokes
the allocation error handler, which **aborts** the process — no unwinding, no
`Result`, nothing a `catch_unwind` can observe. A library reading untrusted
files cannot turn this into a typed error from the outside; it can only avoid
reaching it, which requires parsing the page headers independently.

That is also not possible with the public API: `PageMetadata` (the only thing
`PageReader::peek_next_page` returns) carries `num_rows`, `num_levels` and
`is_dict` but not the sizes, and `file::metadata::thrift`, where `PageHeader`
lives, is `pub(crate)` in 59.1.0. A caller who wants the check must re-implement
Thrift page-header parsing over the raw file.

### Reproducer

The script below takes any Parquet file with a compressed data page and edits a
single header field, keeping the total length unchanged so that every absolute
offset in the footer stays valid:

```
uncompressed_page_size   N -> 2_000_000_000   (varint grows by 4 bytes)
compressed_page_size     M -> M - 4           (same varint length)
page payload             4 bytes removed
```

Keeping the length invariant matters: simply inserting the 4 bytes shifts the
chunk's remaining-byte accounting, and `verify_page_size` then rejects the page
*before* the allocation — which looks like the bug being absent when it is not.

```python
import struct, sys

def varint(v):
    out = bytearray()
    while True:
        b = v & 0x7F
        v >>= 7
        if v:
            out.append(b | 0x80)
        else:
            out.append(b)
            return bytes(out)

def zigzag(n):
    return (n << 1) if n >= 0 else ((-n) << 1) - 1

class R:
    def __init__(self, d, i): self.d, self.i = d, i
    def byte(self):
        b = self.d[self.i]; self.i += 1; return b
    def var(self):
        v, s = 0, 0
        while True:
            b = self.byte(); v |= (b & 0x7F) << s
            if not b & 0x80: return v
            s += 7
    def zz(self):
        g = self.var(); return (g >> 1) ^ -(g & 1)
    def skip(self, t):
        if t in (1, 2): return
        if t == 3: self.i += 1
        elif t in (4, 5, 6): self.zz()
        elif t == 7: self.i += 8
        elif t == 8: self.i += self.var()
        elif t in (9, 10):
            h = self.byte(); n = h >> 4; e = h & 0x0F
            if n == 15: n = self.var()
            for _ in range(n): self.skip(e)
        elif t == 11:
            n = self.var()
            if n:
                p = self.byte()
                for _ in range(n): self.skip(p >> 4); self.skip(p & 0x0F)
        elif t == 12: self.struct()
        elif t == 13: self.i += 16
        else: raise ValueError(f"thrift type {t}")
    def struct(self):
        cur = 0
        while True:
            h = self.byte()
            if h == 0: return
            d, t = h >> 4, h & 0x0F
            cur = self.zz() if d == 0 else cur + d
            self.skip(t)

def page_header(d, off):
    r, cur, out = R(d, off), 0, {"offset": off}
    while True:
        h = r.byte()
        if h == 0: break
        dl, t = h >> 4, h & 0x0F
        cur = r.zz() if dl == 0 else cur + dl
        if cur == 1 and t == 5: out["type"] = r.zz()
        elif cur == 2 and t == 5:
            out["u_from"] = r.i; out["u"] = r.zz(); out["u_to"] = r.i
        elif cur == 3 and t == 5:
            out["c_from"] = r.i; out["c"] = r.zz(); out["c_to"] = r.i
        else: r.skip(t)
    out["end"] = r.i
    return out

data = bytearray(open(sys.argv[1], "rb").read())
tail = data.rindex(b"PAR1")
footer = tail - 4 - struct.unpack("<I", data[tail - 4:tail])[0]

pages, off = [], 4
while off < footer:
    try:
        p = page_header(data, off)
    except Exception:
        break                      # past the last page: index region follows
    if "c" not in p: break
    pages.append(p)
    off = p["end"] + p["c"]

target = [p for p in pages if p.get("type") == 0][int(sys.argv[3])]
new_u = varint(zigzag(2_000_000_000))
grow = len(new_u) - (target["u_to"] - target["u_from"])
new_c = target["c"] - grow
assert new_c > 0 and len(varint(zigzag(new_c))) == target["c_to"] - target["c_from"]

del data[target["end"]:target["end"] + grow]
data[target["c_from"]:target["c_to"]] = varint(zigzag(new_c))
data[target["u_from"]:target["u_to"]] = new_u
open(sys.argv[2], "wb").write(bytes(data))
```

Reading the resulting file:

```rust
let file = std::fs::File::open("patched.parquet")?;
let mut reader = ParquetRecordBatchReaderBuilder::try_new(file)?.build()?;
while let Some(batch) = reader.next() { let _ = batch?; }
```

Under a 512 MiB address-space limit (`ulimit -v 524288`):

```
memory allocation of 2000000000 bytes failed
Aborted (core dumped)          # exit 134
```

Without the limit, the allocation succeeds, the decompressor then fails on the
length mismatch, and a normal `ParquetError` is returned — so the severity
depends entirely on how much memory the process is allowed to reserve.

### Suggested fix

Reject the page before allocating, using information already available in
`SerializedPageReader`:

* compare `uncompressed_page_size` against the chunk's
  `total_uncompressed_size` (a page cannot exceed the total it belongs to), or
* accept an optional cap in `ReaderProperties` and grow the buffer instead of
  reserving it in one call.

The first is the stronger of the two, because it needs no new configuration and
rejects exactly the inconsistent files.

Making the size check reachable from outside would help independently: either
exposing the page sizes on `PageMetadata`, or making the `PageHeader` type
public, would let a caller apply its own bound without re-implementing Thrift
parsing.

### How it was found

Coverage-guided fuzzing of a Parquet reader, followed by a targeted
characterization: the crash seed was reduced to a single edited header field,
and the behaviour was measured in a subprocess under an explicit address-space
limit, because the failure aborts the process and cannot be observed in-process.
