# Change impact analysis — busta d'errore machine-readable R9

Data: 2026-07-31.

## Baseline e perimetro

Lo sviluppo `0.1.0-rc.5` parte dal tag annotato e immutabile
`v0.1.0-rc.4`, target
`8d3f25f109f6ea8910da71e098db6924438e481c`. La baseline di implementazione
contenuta nella release è
`dc85f5163860bd16c4cf0bfa1066276980d38e8c`.

Questo incremento modifica soltanto il protocollo JSON d'errore della CLI.
Non modifica driver, payload Arrow, formati su disco, capability o semantica
di retry.

## Problema

`PlenoraIoError` possiede già quattro assi indipendenti e serializzabili, ma la
CLI pubblicava soltanto `code` e il risultato di `Display`. Categoria, fase,
effetto remoto e retry erano quindi incorporati in prosa italiana. Un chiamante
Python avrebbe dovuto interpretare il messaggio per decidere se riprovare,
contrariamente a R9.2.

Serializzare `retry` come semplice stringa sarebbe inoltre incompleto:
`RetryDisposition::After(u64)` deve conservare la durata.

## Decisione

La busta esterna conserva `status`, `protocol_version`, `contract`, `code` e
`message`. L'oggetto `error` pubblica inoltre:

- `category`, `phase` e `remote_effect` in `snake_case`;
- `retry` come oggetto taggato, usando direttamente la serializzazione di
  `RetryDisposition`;
- `delay_ms` intero soltanto per `{"kind":"after"}`.

La forma deriva dai tipi già presenti in `plenora-io-model` e dalla capability
condivisa `error_envelope_json` registrata in
`plenora-contracts/conformance/components.json`. Non viene introdotto un
secondo DTO con una tassonomia divergente.

`message` contiene ora soltanto il testo diagnostico redatto. Non ripete gli
assi e non costituisce un'interfaccia macchina.

Anche gli errori prodotti direttamente dalla CLI — usage, estensione non
supportata, layer inesistente e sink single-layer — ricevono assi espliciti,
così non esistono due forme sotto lo stesso contratto
`plenora-io-error-v1`.

## Compatibilità e hazard

- H-01/H-08: il retry non viene più dedotto da testo localizzato.
- `After` conserva esattamente `delay_ms`; `Never`, `Safe`,
  `RequiresIdempotencyKey` e `RequiresRecovery` restano oggetti distinti.
- Exit code, `code`, protocol version e nome del contratto restano invariati.
- La rimozione del prefisso prodotto da `Display` dentro `message` è
  intenzionale: quel prefisso duplicava campi macchina e non era un contratto
  stabile.
- R9 resta proposta; questo record dichiara implementazione della capability
  condivisa, non ratifica normativa.

## Verifica

- golden test delle cinque varianti di `RetryDisposition`;
- test CLI completo di `After(2750)` con conservazione della durata;
- test `Never` su `READER_BUSY`;
- test degli assi anche sulla busta `CLI_USAGE`;
- round-trip e rifiuto dei campi futuri sul tipo interno già esistenti;
- rustfmt, Clippy, test workspace e gate del contratto di release.
