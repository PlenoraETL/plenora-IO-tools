# Change impact analysis — contratto `convert` e superficie compatibile 1.x

Data: 2026-07-31.

## Baseline e decisione

La modifica appartiene allo sviluppo `0.1.0-rc.5` successivo al tag immutabile
`v0.1.0-rc.4`. Non apre né modifica `1.0.0-rc.1`: prepara la forma che quella
baseline potrà congelare. La successiva proposta R4.6.5 ha reso implementabile
la decisione sul comando che combina bordo di lettura e scrittura; il relativo
intervento è registrato in una CIA separata.

La superficie candidata alla compatibilità 1.x è soltanto il protocollo JSON
della CLI. Le API Rust di `plenora-io-model`, `plenora-io-core` e dei driver
restano interne, non pubblicate e senza garanzia SemVer. Questa scelta preserva
la possibilità prevista da R15.4.1 di estrarre i tipi di confine da
`plenora-io-model` dopo la ratifica di §15.3.

Le sei buste candidate e le regole di compatibilità sono registrate in
`release/cli-protocol-v1.json`.

## Problema

Il documento `plenora-io-convert-v1` esponeva:

- `read_fidelity`, valutazione della lettura;
- `write_fidelity`, valutazione della scrittura;
- `loss.lossless` e `loss.counts`, riferiti soltanto al writer.

Il nome `lossless` poteva essere letto come giudizio end-to-end anche quando
`read_fidelity` era `approximating`. Inoltre il `LossReport` osservato durante
la lettura non usciva dal processo: il runner esterno poteva verificare la
preservazione di rappresentazioni CRS discordanti, ma non la loro dichiarazione
richiesta dal secondo obbligo di R4.6.1.

## Forma nuova

`convert` rimuove il campo legacy `loss` e pubblica:

- `read_loss`, con `lossless` e `counts` riferiti al solo reader;
- `write_loss`, con la stessa forma riferita al solo writer;
- `conversion_fidelity`, valutazione complessiva che assume il livello peggiore
  fra lettura e scrittura e unisce le motivazioni bounded;
- i preesistenti `read_fidelity` e `write_fidelity`, che restano disponibili per
  localizzare l'origine dell'approssimazione.

Il comando aggrega il `LossReport` dopo l'EOF di ogni layer reader. Una perdita
osservata promuove `read_fidelity` e quindi `conversion_fidelity` ad
`approximating`.

## CRS combinato

Questa modifica osserva e pubblica l'incoerenza, ma non decide se una
destinazione debba accettarla. La successiva capability
`crs_representations` implementa la distinzione candidata di R4.6.5 fra
propagare tutte le rappresentazioni discordanti e sceglierne o derivarne una.
La ratifica resta dell'owner e non viene anticipata dal claim del componente.

## Scope reduction

Il manifesto RC5 distingue i residui ammessi per una component RC
`verified_internally` dai blocker che riemergono per claim più forti:
conformità completa R4.3.1 o R7.5–R7.7, `verified_independently`, system RC e
certificazione avionica.

## Verifica

- fixture IPC reale con `crs_id=EPSG:4326` e `srid=3003`;
- conversione IPC → IPC che preserva il dataset;
- `read_loss.counts.inconsistent_crs_representations == 1`;
- assenza del campo legacy `loss`;
- writer lossless distinto dalla fedeltà complessiva `approximating`;
- test dei livelli combinati e del bound delle motivazioni;
- gate del manifesto del protocollo, rustfmt, Clippy, test workspace e CI
  multipiattaforma.
