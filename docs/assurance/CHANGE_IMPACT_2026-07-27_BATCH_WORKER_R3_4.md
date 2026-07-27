# Change impact analysis — reader worker e R3.4

Data: 2026-07-27

## Baseline e ambito

Baseline IO-tools: commit `cdeed6bffc9c12660d75583b6b6a369de2985ee9`.

Fonte trasversale ispezionata: repository `plenora-contracts`, tag
`v2.0-rc2`, commit `0faeadbcd34b924430b39647e78e31b34b11bd24`.
La rappresentazione e propagazione delle cinque dimensioni di §3.3 è
ratificata; §3.4 resta proposta, ma la distinzione fra metadato assente e valore
esplicito `unknown` è già adottata come controllo H-01 locale.

L'incremento modifica il confine fra i parser in background e `LayerReader` per
CSV, GeoJSON, Shapefile e FileGDB/GDAL. Non cambia il formato dei dataset, il
layout Arrow, le capability dichiarate o il protocollo di publish.

## Modifica

- `plenora-io-core` introduce un unico `spawn_batch_reader` con canale bounded;
- il producer può emettere soltanto batch attraverso `BatchEmitter`;
- il protocollo interno distingue esplicitamente `Batch`, `Finished` e
  `Failed(PlenoraIoError)`;
- gli errori non vengono più convertiti in `String` al confine del thread;
- un panic del parser viene intercettato e restituito come errore di formato;
- una disconnessione senza evento terminale è un errore, mai un EOF;
- il rilascio anticipato del reader resta cancellazione cooperativa: `send`
  restituisce `false` e il producer interrompe il lavoro;
- CSV, GeoJSON, Shapefile e FileGDB usano la stessa implementazione;
- nel recupero dei contratti GeoArrow legacy il default XY viene stabilito
  esclusivamente dal costruttore prima del parsing dei metadati; è rimossa ogni
  assegnazione a XY successiva a
  `read_geometry_contract_metadata`.

## Failure mode

| Evento | Esito osservabile |
|---|---|
| Parser concluso regolarmente | evento `Finished`, quindi `Ok(None)` |
| Errore del parser | stessa variante `PlenoraIoError` ricevuta dal chiamante |
| Panic nel worker | `PlenoraIoError::Format`, non falso EOF |
| Canale chiuso senza terminale | `PlenoraIoError::Format`, non falso EOF |
| Reader rilasciato dal consumatore | il producer rileva il canale chiuso e termina |
| Dimensioni legacy assenti | default storico XY |
| Dimensioni esplicitamente `unknown` | `Unknown` preservato |

Il panic hook del processo può ancora produrre diagnostica, ma il panic del
worker non attraversa il confine di `LayerReader`. Un abort di processo non è
intercettabile da Rust e resta fuori da questo controllo.

## Impatto API e compatibilità

`spawn_batch_reader` e `BatchEmitter` sono API additive di
`plenora-io-core`, necessarie ai driver che vivono in crate separati. Il
protocollo degli eventi resta privato al core. I crate hanno versione `0.0.0`,
`publish = false`, e non esiste una release pubblica da migrare.

La rimozione del fallback post-parsing R3.4 non cambia il comportamento
corretto già verificato: elimina codice ridondante che rendeva ambigua
l'ispezione statica. Il test distingue esplicitamente assenza e `unknown`.

Non cambiano manifest, dipendenze, lockfile o toolchain.

## Hazard e verifica

- H-01: un errore strutturato o una dimensione `unknown` non vengono
  reinterpretati;
- H-04: un panic nel parser in background non viene presentato come successo;
- H-08: test deterministici coprono completamento, errore tipizzato, panic e
  distinzione R3.4;
- H-09: questa analisi registra l'impatto e i residui.

Evidenza locale con Rust `1.92.0` in container Linux x86_64:

- test mirati core/CSV/GeoJSON/Shapefile superati;
- test workspace `--all-targets --locked` superati;
- regressione CSV: errore WKB preservato attraverso il worker;
- regressioni core: terminale esplicito, errore tipizzato e panic non-EOF;
- regressione R3.4: assente → XY, `unknown` esplicito → `Unknown`;
- FileGDB/GDAL feature-on: 21 test superati, 2 helper ignorati;
- Clippy workspace completo superato;
- Clippy workspace `--all-features` superato;
- safety Clippy sui target `lib`, anche `--all-features`, superato.

L'evidenza CI remota sarà associata al commit dell'incremento dopo il push.

## Residui

- KML, DXF e XLSX materializzano ancora durante `open`;
- la cancellazione è cooperativa al successivo invio di batch, non preemptive;
- non è introdotto un protocollo di restart automatico del parser;
- §3.4.1 e il modello a tre stati dei tipi restano subordinati alla ratifica
  trasversale;
- manca revisione indipendente.

Questa evidenza non costituisce certificazione né dichiarazione di conformità
DO-178C/ED-12C.
