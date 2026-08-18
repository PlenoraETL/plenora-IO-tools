# Segnalazione a monte — `parquet`: bit width degli indici di dizionario non validato

Stato: **pubblicata** il 2026-08-18 — apache/arrow-rs#10722, https://github.com/apache/arrow-rs/issues/10722

`parquet` prende il bit width degli indici di dizionario dal primo byte della
sezione valori di una data page senza validarlo.

Il testo sotto la linea e' quello inviato, ripulito da ogni riferimento interno:
nessun percorso, nessun nome di componente, nessun artefatto del nostro
fuzzing. Il reproducer e' costruito da zero e non contiene input del nostro
corpus.

## Rapporto con la mitigazione locale

La correzione a monte **non sostituisce** la difesa che abbiamo: chiuderebbe
questo difetto, non la classe. La prevalidazione e la barriera si tolgono per
decisione separata, se e quando la libreria dichiara fallibile la conversione.

---

## Unvalidated dictionary index bit width panics the Arrow reader

**Version:** 59.1.0 (`parquet`)
**File:** `src/arrow/decoder/dictionary_index.rs`, `DictIndexDecoder::new`

### Summary

`DictIndexDecoder::new` takes the RLE bit width straight from the first byte of
a dictionary-encoded data page's value section, with no range check and no
emptiness check:

```rust
pub fn new(data: Bytes, num_levels: usize, num_values: Option<usize>) -> Result<Self> {
    let bit_width = data[0];              // <- no bounds check, no range check
    let mut decoder = RleDecoder::new(bit_width);
    decoder.set_data(data.slice(1..))?;
    ...
}
```

The Parquet specification bounds the dictionary index bit width to `0..=32` for
`i32` indices. A file that declares a larger value reaches
`BitReader::get_batch::<i32>`, which is documented to panic:

```
/// # Panics
///
/// This function panics if
/// - `num_bits` is larger than the bit-capacity of `T`
pub fn get_batch<T: FromBitpacked>(&mut self, batch: &mut [T], num_bits: usize) -> usize {
    debug_assert!(num_bits <= size_of::<T>() * 8);
```

Two distinct defects at the same line:

1. **`bit_width` is not range-checked.** With `debug_assertions` on this is the
   `debug_assert!` above. With `debug_assertions` off but `overflow-checks` on —
   a combination we ship deliberately, because silently wrapping arithmetic on
   untrusted input is worse than failing — the panic simply moves a few lines
   down, inside `get_batch`. Either way an untrusted file panics the reader.
2. **`data[0]` is not bounds-checked.** An empty value section indexes out of
   range and panics before the bit width is even read.

Both are reachable from a plain `ParquetRecordBatchReader` over a crafted file:
no unsafe code, no unusual reader configuration.

### Impact

Callers that read untrusted Parquet cannot get an error back — they get a panic
crossing the library boundary. Libraries built on top must wrap every read in
`catch_unwind` to preserve their own error contract, which is what we do, and
which cannot be the intended integration.

### Reproducer

Any dictionary-encoded column whose data page value section begins with a byte
greater than 32. Reading it with:

```rust
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;

let reader = ParquetRecordBatchReaderBuilder::try_new(std::fs::File::open("crafted.parquet")?)?
    .build()?;
for batch in reader {
    let _ = batch?;
}
```

Observed with `debug_assertions` on:

```
thread 'main' panicked at parquet-59.1.0/src/util/bit_util.rs:697:
assertion failed: num_bits <= size_of::<T>() * 8
   get_batch<i32>
   DictIndexDecoder::read
   ByteArrayColumnValueDecoder::read
   read_records_with_reservation
```

With `debug_assertions` off and `overflow-checks` on, the same input panics a
few lines later inside `get_batch`.

### Suggested fix

Make `DictIndexDecoder::new` fallible on both counts — it already returns
`Result`:

```rust
let bit_width = *data.first().ok_or_else(|| {
    ParquetError::General("dictionary index page is empty".into())
})?;
if bit_width > 32 {
    return Err(ParquetError::General(format!(
        "dictionary index bit width {bit_width} exceeds the maximum of 32"
    )));
}
```

The same bound would be worth asserting in `RleDecoder::new`, so that other
callers cannot construct a decoder that is guaranteed to panic on first use.

### How it was found

Coverage-guided fuzzing of a GeoParquet reader built on `parquet`, under
AddressSanitizer, with `overflow-checks = true`. The finding reproduces in about
25 ms and is deterministic.
