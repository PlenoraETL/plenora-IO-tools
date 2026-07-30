# Fork governato `dxf 0.6.1`

## Provenienza

- crate: `dxf 0.6.1`;
- release crates.io SHA-256:
  `6bb070bbb077a936e2bdf95d4b39aa83e865b38c3a7054f71d704e0796da1821`;
- tag upstream: `v0.6.1`;
- oggetto tag upstream:
  `c965cc8ef139b191131780ccdcf134c220642b16`;
- commit upstream:
  `f1bca30b9d753ee53e66212b329972a3dfd46641`;
- licenza upstream: MIT (`LICENSE.txt`).

Il workspace risolve `dxf 0.6.1` esclusivamente da `vendor/dxf` tramite
`[patch.crates-io]`. `scripts/dxf-fork-lock.json` fissa identità e digest
dell'intero albero; `scripts/check_dxf_fork.py` fallisce se manifest, lockfile,
provenienza o contenuto divergono.

## Delta funzionale

- `src/code_pair_iter.rs`: iteratore di code pair su reader posseduto senza
  `read_to_end`; le righe ASCII usano `BufRead::read_until` con buffer da
  1 KiB, preservando BOM, CRLF ed encoding senza anticipare l'intero input;
- `src/drawing.rs`: `DrawingEntityReader`, reader fallibile pull-based che
  conserva metadata e blocchi ma emette una entità logica alla volta; raggruppa
  `POLYLINE`/`VERTEX`, attributi di `INSERT` e `MTEXT` come il loader upstream;
- `src/lib.rs`: esportazione pubblica di `DrawingEntityReader`.

Il fork non cambia la scrittura DXF né i tipi pubblici esistenti. Il percorso
progressivo rifiuta DXB; DXF ASCII e binary DXF restano supportati.

## Verifica e aggiornamento

Ogni aggiornamento deve:

1. ripartire da una release upstream identificata da tag, commit e checksum;
2. riapplicare e revisionare separatamente i delta sopra;
3. eseguire l'intera suite upstream, i test `driver-dxf` e il gate di
   provenienza;
4. rieseguire il benchmark interlacciato contro l'RC precedente;
5. aggiornare insieme questo documento e `scripts/dxf-fork-lock.json`.
