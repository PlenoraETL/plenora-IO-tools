# Protocollo coordinato di fuzzing WKB/EWKB

Stato: **replay differenziale operativo; smoke deterministico completato;
campagna lunga coverage-guided non ancora avviata**.

Baseline verificate il 2026-07-28:

- IO-tools `1c37fb5d525647b264ce977e26fc07b346bb7914`,
  codec lossless `plenora-io-model::wkb_lossless` e adattatore XY `wkb`;
- database-tools `ef18e80c798126f872fd366c36ee96a029598958`, scanner
  `plenora-database-core::ewkb`;
- ICD `plenora-contracts@v2.0-rc8`,
  revisione `62b12e3496466d2c908dac3cc098640b99b52e21`.

## Proprietà del corpus

Il corpus condiviso contiene soltanto payload WKB/EWKB grezzi. Byte di
controllo, limiti e metadati esterni non vengono anteposti al file: ciascun
harness li deriva da parametri separati. Questo rende riutilizzabili i casi
dagli harness dei due componenti.

I 18 casi deterministici sono in `fuzz/shared-corpus/cases/`. Ogni caso è
identificato da SHA-256 e descritto da `fuzz/shared-corpus/manifest.json`,
conforme a `fuzz/shared-corpus-manifest.schema.json`. Il generatore
`scripts/generate_shared_wkb_corpus.py` ricostruisce il corpus senza dipendenze
esterne; la modalità `--check` rileva modifiche a payload, manifest o hash.

La sede futura proposta resta
`plenora-contracts/conformance/wkb-ewkb/`; fino all'accettazione dei tre owner,
la copia in IO-tools è un corpus candidato, non una fonte contrattuale
autorevole.

## Invarianti comuni

Entrambe le implementazioni devono verificare:

1. nessun panic, hang o crescita non bounded su input arbitrario;
2. consumo completo dell'input e rifiuto dei trailing bytes;
3. controllo di byte, componenti, conteggi e profondità prima
   dell'allocazione corrispondente;
4. aritmetica controllata su offset, lunghezze e prodotti;
5. stabilità dell'esito quando i limiti accettati vengono allentati;
6. identità di tipo, dimensioni, endianess normalizzata e SRID dopo un
   round-trip lossless;
7. coerenza tra SRID EWKB incorporato e SRID/CRS dichiarato nei metadati;
8. rifiuto fail-closed di type word, flag dimensionali o gerarchie incoerenti.

I seed coprono WKB ed EWKB, little/big endian, geometrie vuote, i sette tipi
semplici, XY/XYZ/XYM/XYZM, conteggi avversari, troncamenti, trailing bytes,
byte-order non valido, SRID e una curva estesa.

## Oracolo differenziale

Per ogni payload i due replay registrano separatamente:

- accettato/rifiutato;
- categoria portabile dell'errore;
- tipo geometrico canonico;
- dimensioni;
- SRID radice;
- componenti e profondità osservate, quando disponibili.

`scripts/compare_shared_wkb_observations.py` confronta i report e fallisce se
trova una divergenza non dichiarata. Una differenza ammessa deve comparire nel
manifest come `known_difference`, con dimensione interessata, classificazione
e motivazione. Le classificazioni ammesse sono:

- `defect_io`;
- `defect_database`;
- `intentional_capability_difference`;
- `ambiguous_or_noncanonical_input`;
- `contract_gap`.

Nessun caso può essere rimosso dal corpus per far passare una campagna: viene
corretto il codec, aggiornata l'aspettativa con motivazione oppure aperto un
gap contrattuale.

## Evidenza del replay

Il replay del 2026-07-28 ha confrontato tutti i 18 casi ed è terminato con
`status: pass`, senza differenze non classificate. Restano due differenze
esplicite:

1. database-tools accetta `CircularString`, mentre IO-tools espone soltanto i
   sette tipi WKB semplici: `intentional_capability_difference`;
2. un `LineString` simultaneamente troncato e oltre il limite di componenti è
   classificato `data_mapping` da IO-tools e `resource_limit` da
   database-tools: `ambiguous_or_noncanonical_input`, perché l'ICD non
   definisce la precedenza tra le due violazioni.

I report macchina sono artefatti rigenerabili in `target/` e non vengono
versionati. I test unitari del comparatore e il controllo deterministico del
corpus sono eseguiti anche dalla CI.

## Evidenza dello smoke

La campagna locale bounded del 2026-07-28, con seed `20260728`, ha eseguito
68.740.000 mutazioni in 60 secondi, senza finding:

```text
PLENORA_FUZZ_SECONDS=60
PLENORA_FUZZ_SEED=20260728
FINE: iter=68740000 findings=0 durata=60s
```

Questo risultato è una regressione riproducibile del componente e non
sostituisce una campagna lunga coverage-guided.

## Sequenza residua

1. pubblicare il medesimo corpus nella sede condivisa dopo l'approvazione
   degli owner;
2. collegare entrambi gli harness coverage-guided allo stesso corpus;
3. eseguire la campagna lunga con budget, toolchain e retention dei finding
   dichiarati;
4. quando §15.3 viene ratificata e il codec migra nel crate condiviso,
   trasferire corpus e invarianti senza cambiarne il formato.
