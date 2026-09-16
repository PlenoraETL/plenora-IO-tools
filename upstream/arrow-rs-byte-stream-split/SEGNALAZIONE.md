# Out-of-bounds index in the BYTE_STREAM_SPLIT decoder on a malformed page header

A Parquet file whose data page header declares more values than the page
actually contains makes `ByteStreamSplitDecoder` index past the end of the page
buffer and panic. Reading untrusted Parquet is therefore not panic-free.

## Affected versions

Reproduced on **60.0.0** (latest at the time of writing) and on **59.3.0**. The
panic site is the same line in both.

## Expected and actual

**Expected:** the reader returns an `Err` for a file whose page header is
inconsistent with the page contents, as it does for other malformed input.

**Actual:** a panic from inside the decoder:

```
thread panicked at parquet-60.0.0/src/encodings/decoding/byte_stream_split_decoder.rs:61:38
index out of bounds: the len is 56 but the index is 56
```

## Cause

`ByteStreamSplitDecoder` derives two quantities from two different sources and
never reconciles them.

`set_data` stores the page bytes and the **declared** value count side by side:

```rust
fn set_data(&mut self, data: Bytes, num_values: usize) -> Result<()> {
    self.encoded_bytes = data;          // actual page bytes
    self.total_num_values = num_values; // declared in the page header
    self.values_decoded = 0;
    Ok(())
}
```

`get` then takes the stride from the **actual** bytes and the loop bound from
the **declared** count:

```rust
let total_remaining_values = self.values_left();          // declared - decoded
let num_values = buffer.len().min(total_remaining_values);
let stride = self.encoded_bytes.len() / type_size;        // actual
```

and `join_streams_const` indexes with no check:

```rust
let sub_src = &src[values_decoded..];
for i in 0..dst.len() / TYPE_SIZE {
    for j in 0..TYPE_SIZE {
        dst[i * TYPE_SIZE + j] = sub_src[i + j * stride];   // line 61
    }
}
```

When the declared count exceeds `encoded_bytes.len() / type_size`, the index
`i + j * stride` runs past `sub_src`.

For contrast, `VariableWidthByteStreamSplitDecoder::set_data` does validate its
input (`data.len().is_multiple_of(self.type_width)`), so the fixed-width path
looks like an omission rather than a deliberate choice.

## Minimal reproducer

`upstream/arrow-rs-byte-stream-split` in this report is a standalone Cargo
project depending only on `parquet`, `arrow-array` and `arrow-schema`.

```
cargo run
```

What it does, and why in that order:

1. writes a **valid** Parquet file with one `DOUBLE` column encoded as
   `BYTE_STREAM_SPLIT` (`WriterVersion::PARQUET_2_0` is required; v1 would
   silently fall back to another encoding and leave this decoder unreached);
2. reads it back, and stops if that fails -- without this counterfactual a
   later failure would not show that the altered byte is what causes it;
3. flips **one byte**: the zigzag varint of `num_values` in the data page
   header, from 8 to 63. 63 is the largest value a one-byte zigzag varint
   holds, so no offset in the file moves and the rest of the layout is exactly
   what the writer produced;
4. reads again.

Eight `f64` values occupy 64 bytes, so `stride` is 8. Declaring 63 values makes
the loop ask for `sub_src[62 + 7 * 8]`.

Expected output:

```
valid file reads back: 8 rows
num_values candidates at offsets [13, 17] (footer starts at 137)

--- offset 13 [data page header]: num_values 8 -> 63, one byte
    PANIC or Err: index out of bounds: the len is 56 but the index is 56  (altered-at-13.parquet)

--- offset 17 [data page header]: num_values 8 -> 63, one byte
    read 8 rows, no error  (altered-at-17.parquet)
```

The program tries **every** offset where that varint appears rather than
assuming which one matters: offset 13 is the field that triggers it, offset 17
is a different field of the same header and changes nothing. It leaves
`valid.parquet` and the altered files on disk so they can be inspected with
other tools.

The read is wrapped in `catch_unwind` only so the program can report on every
candidate instead of stopping at the first panic.

## How it was found

By fuzzing a GeoParquet reader built on this crate. The original input was
3,966 bytes of malformed GeoParquet; the reproducer above was then built from
scratch to identify which field causes it, rather than inferring it from the
panic message.

## Impact

Any caller reading Parquet from an untrusted source can be made to panic. A
caller that catches unwinding survives, but a caller compiled with
`panic = "abort"` -- or a fuzz target, where the panic hook aborts before
unwinding -- does not.

## Suggested direction

Reconciling the two sources in `set_data` -- rejecting a declared count that the
page cannot hold -- would turn this into the `Err` the reader already returns
for other malformed input. We have not proposed a patch: the right place for
the check may be wherever the crate prefers to validate page headers.
