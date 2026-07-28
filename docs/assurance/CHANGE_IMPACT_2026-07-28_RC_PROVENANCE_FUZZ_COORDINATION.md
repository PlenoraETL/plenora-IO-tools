# Change impact analysis — provenienza RC e fuzzing coordinato

Data: 2026-07-28.

## Baseline

- IO-tools:
  `59369fdfb6dbb5d1d7c97a29865ca39ae21c6f76`;
- ICD candidato: `plenora-contracts@v2.0-rc3`,
  `ef2640348426425585ad228312468e7cf1d0e50f`;
- database-tools, branch osservato `assurance/ewkb-fuzzing`:
  `834fff4fbe0c62cc2f02278073e58b0cf2159f8d`;
- data-tools osservato:
  `40771dec2426ba79dc777b2243a6a16bd4a3072d`.

Il tag ICD è annotato ma non firmato. Il registro della revisione citata è
parzialmente ratificato; le sezioni adottate anticipatamente dalla precedente
modifica restano proposte.

## Modifica

La modifica non altera codice Rust, wire format o comportamento dei driver.
Introduce:

- un manifest machine-readable della provenienza contrattuale;
- la distinzione vincolata in CI fra RC di componente e RC di sistema;
- la deroga esplicita per l'emissione anticipata delle chiavi §2;
- un gate ancora non soddisfatto per il round-trip attraverso i tre componenti;
- un protocollo e uno schema di corpus condiviso per il confronto WKB/EWKB;
- test negativi del gate documentale.

## Impatto e compatibilità

Non cambia alcuna API. Il nuovo gate può invece bloccare intenzionalmente una
release se:

- viene cambiata la wire version senza aggiornare il manifest;
- il tag o la revisione ICD divergono;
- una sezione proposta viene presentata come ratificata;
- compare una dichiarazione di system RC o certificazione avionica;
- il gate di sistema viene promosso senza una revisione dedicata.

`implementation_revision` identifica la baseline funzionale esaminata. Al
freeze deve essere sostituita con la revisione finale e `freeze_status` deve
passare a `frozen` mediante una CIA successiva.

## Hazard

- H-01: il protocollo differenziale riduce divergenze non rilevate fra i codec;
- H-07: ICD e componenti sono citati con SHA completi;
- H-08: corpus e invarianti diventano riusabili e verificabili;
- H-09: stato normativo, deroga e limiti della dichiarazione non restano
  impliciti.

La modifica non chiude la readiness di sistema, la revisione indipendente o la
qualifica avionica e non dichiara di farlo.

## Verifica

- 6 test del gate release: superati;
- 18 test Python complessivi dei gate di assurance: superati;
- gate release, action pin, identità, dipendenze e fallback: superati;
- parsing dei tre documenti JSON: superato;
- `cargo fmt --all -- --check` nel container Rust 1.92: superato;
- `git diff --check`: superato.

La campagna fuzz lunga non è stata avviata: l'assenza è una precondizione
intenzionale finché i due team non accettano protocollo e schema.

## Evidenza CI post-push

La prima esecuzione della baseline `8ad2d99` ha individuato che il passaggio da
tag specializzati a uno SHA comune di `taiki-e/install-action` rimuoveva il nome
del tool implicito. Il job coverage è terminato con exit 101 prima della misura.
La correzione dichiara `with.tool` per `cargo-audit` e `cargo-llvm-cov` e
aggiunge test negativi al gate action pin. Il finding è attribuito al rinnovo
Node 24, non ai manifest RC o al codice Rust.
