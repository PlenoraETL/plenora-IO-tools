# Change impact analysis — qualifica post-tag 1.0.0-rc.1

Data: 2026-07-31.

## Scopo

Questa registrazione non modifica il tag `v1.0.0-rc.1` e non promuove il
componente a RC di sistema. Collega la baseline immutabile alla prima qualifica
esterna eseguita sui tre tag dei componenti.

## Provenienza

- IO-tools: `v1.0.0-rc.1`, target
  `6e3a942dfd607c8bf4bdbe0075c8e8f5f3761842`;
- data-tools: `v0.1.0-rc1`, target
  `7d530318760ccfa93b2baa2049e181fd57deed1e`;
- database-tools: `v0.1.0-rc.1`, target
  `b541c61dd1c286cdf2e808e17eefd133d7c9ba20`;
- owner della qualifica: `plenora-contracts/conformance`, revisione
  `c3f1a8ef2e6950570a33adf5a964f7f40e9cf1ab`.

L'evidenza autorevole è committata dall'owner in:

- `conformance/evidence-2026-07-31-qualifica-roundtrip.json`;
- `conformance/evidence-2026-07-31-qualifica-chain.json`.

## Risultato

- roundtrip: `83/84`;
- catena IO → data → giudice database: `27/28`;
- varianti canoniche: `42/42` come conservazione;
- varianti GeoArrow: `41/42` come comprensione semantica.

L'unico fallimento è `crs_unresolved__geoarrow`: IO-tools rifiuta
`declared_unresolved` senza `crs_definition`. Corrisponde esattamente a R4.1.1,
già dichiarata come proposta non implementata. Non è una violazione di una
regola ratificata né un blocker retroattivo della component RC.

I casi `conflicting_crs` attraversano la catena; il centro applica la
transizione richiesta da `resolved` a `declared_unresolved` senza modificare le
rappresentazioni discordanti.

## Obbligo read_loss

Il runner esterno conserva ancora `unverified_obligation` perché non analizza
la busta `convert`. Una verifica mirata sulla stessa baseline taggata ha
osservato, per `conflicting_crs__geoarrow`:

```text
read_loss.counts.inconsistent_crs_representations = 1
conversion_fidelity.level = approximating
write_loss.counts = {}
```

La variante solo canonica produce invece `read_loss.counts = {}`: questa
differenza isola il gap R2.8, anch'esso proposta non implementata, e non
smentisce l'implementazione R4.6.1 sul contratto riconosciuto.

## Cosa resta

Il gate di sistema resta `not_satisfied`: manca la direzione
database → data → IO, manca l'esecuzione nativa Windows della catena, il runner
deve ancora consumare `read_loss`, e l'evidenza è dichiarata contro l'ICD
`v2.0-rc8` mentre la provenienza corrente del componente registra
`v2.0-rc13`.

Per la stabilizzazione del solo componente, il prossimo ingresso utile è
l'integrazione dal backend Python. Non viene implementata in anticipo né R2.8
né R4.1.1.
