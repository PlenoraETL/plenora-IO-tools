# Change impact analysis — identità pubbliche R8

Data: 2026-07-27

## Baseline e requisito

Fonte autorevole: `plenora-contracts` tag annotato `v2.0-rc2`, oggetto tag
`6c93d5458e7e4fd216116840732aa0488fef9535`, commit
`0faeadbcd34b924430b39647e78e31b34b11bd24`.

Le regole ratificate R8.1 e R8.4 vietano rispettivamente package distinti con
lo stesso nome e tipi pubblici omonimi con forme diverse. Alla baseline
precedente, IO-tools e data-tools esponevano entrambi un package
`plenora-core` e un enum `PlenoraError`, ma con implementazioni indipendenti.

Il confronto cross-repository usa per data-tools il commit
`07f6823` (`ICD R6: zero primitive di panic nei target lib + gate CI
bloccante`). Le modifiche non committate presenti nella sua working copy non
sono parte di questa baseline.

## Modifica

- il package locale `plenora-core` diventa `plenora-io-model`;
- il crate importabile diventa `plenora_io_model`;
- la directory del package diventa `crates/plenora-io-model`;
- l’enum pubblico `PlenoraError` diventa `PlenoraIoError`;
- tutti i manifest, import, pattern match, test e target fuzz sono migrati;
- i lockfile principale e fuzz registrano la nuova identità;
- `check_public_identity.py` rifiuta il package riservato `plenora-core`,
  package duplicati nel workspace e ogni ricomparsa del token
  `PlenoraError` nei sorgenti Rust;
- quattro test del gate coprono identità valida, package riservato, duplicato e
  vecchio errore pubblico.

Non cambiano varianti, messaggi, conversioni, contratti dati, formati, CRS,
limiti, publish o failure mode. È un rename a semantica zero.

## Impatto di compatibilità

Il cambio è source-breaking per un consumatore che importi direttamente il
vecchio crate o il vecchio enum. Non viene mantenuto un alias deprecato:
reintrodurrebbe precisamente l’identità pubblica vietata da R8.4. Tutti i crate
del repository sono `0.0.0` e `publish = false`; non esiste una release pubblica
cui garantire compatibilità.

Il futuro crate condiviso `plenora-contracts` non viene creato: §15.3 resta
proposta e richiede che l’API sia ratificata e completa prima dell’estrazione.

## Hazard e verifica

- requisiti: R8.1, R8.4; PLN-ASR-014;
- hazard: H-01, confusione fra tipi di confine; H-07, dipendenza risolta verso
  il package errato;
- piattaforme: tutte;
- verifica richiesta: gate identità e relative regressioni, metadata `--locked`
  per entrambi i workspace, test workspace, Clippy completo, safety gate,
  build release, FileGDB/GDAL e CI Linux/Windows/macOS/coverage.

Risultati locali con Rust `1.92.0` in container Linux x86_64:

- 4 regressioni del gate identità superate;
- gate identità superato su 16 manifest e 34 sorgenti Rust;
- gate pin dipendenze superato su 18 manifest;
- registro fallback verificato a 95 occorrenze;
- `actionlint` e formattazione superati;
- risoluzione completa `--locked` superata per workspace principale e fuzz;
- test workspace completi superati;
- Clippy workspace completo e safety gate superati;
- build release superata;
- FileGDB/GDAL: 21 test superati, 2 helper ignorati.

Evidenza remota acquisita per il commit di implementazione
`98ee65dc764578b5876c48432a4ca974ca5d29b5`: GitHub Actions run
`30283970910`, conclusione `success` per i job `rust`, `windows`,
`macos-publish` e `coverage`.

## Residui

- R8.3 resta aperta fino alla ratifica e adozione del crate condiviso;
- il rename non qualifica Cargo, rustc o gli strumenti di verifica;
- manca una revisione indipendente cross-team.

Questa evidenza non costituisce certificazione né dichiarazione di conformità
DO-178C.
