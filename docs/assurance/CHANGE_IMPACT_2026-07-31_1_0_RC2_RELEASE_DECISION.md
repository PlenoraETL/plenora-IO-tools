# Change impact analysis — decisione component RC 1.0.0-rc.2

Data: 2026-07-31.

## Decisione

La revisione
`63a82531f82c4d3d42372fa8499ba1678ae4344b` è autorizzata come candidato
tecnico per `plenora-IO-tools v1.0.0-rc.2`, con claim
`verified_internally`.

Questa decisione:

- autorizza una RC del solo componente IO-tools;
- conserva `v1.0.0-rc.1` e la sua qualifica `83/84`/`27/28` come record
  storico immutabile;
- mantiene come superficie compatibile 1.x soltanto le sei buste JSON della
  CLI dichiarate in `release/cli-protocol-v1.json`;
- mantiene le API Rust interne, non pubblicate e fuori dalla superficie SemVer;
- non dichiara una RC del sistema, revisione indipendente o certificazione
  avionica;
- autorizza il tag annotato non firmato `v1.0.0-rc.2` soltanto dopo
  allineamento della versione, manifesti pre-tag coerenti, CI verde della
  revisione pre-tag e record finale verificato.

## Evidenza del candidato

La CI
[`30625336681`](https://github.com/PlenoraETL/plenora-IO-tools/actions/runs/30625336681)
copre esattamente
`63a82531f82c4d3d42372fa8499ba1678ae4344b` ed è verde sui job `rust`,
`windows`, `macos-publish` e `coverage`.

La pipeline comprende formattazione, gate di assurance, Clippy con tutte le
feature, safety lint, test dell'intero workspace su Linux e Windows, build
release locked, audit dipendenze, coverage, publish macOS e matrice
GDAL/OpenFileGDB nativa Windows con benchmark narrow.

## Delta rispetto a v1.0.0-rc.1

La candidata implementa R4.1.1 di `plenora-contracts v2.0-rc14`, revisione
`65fd2c6418efa7937e3063245913d79a80c6499b`:

1. `RawCrs::definition` e `definition_format` diventano opzionali e restano
   presenti o assenti insieme;
2. `declared_unresolved` accetta il solo `crs_id`, senza sintetizzare
   definizione, formato, SRID o CRS operativo;
3. uno stato `declared_unresolved` privo sia di identificatore sia di
   definizione continua a essere rifiutato;
4. il driver IPC esercita il percorso reale GeoArrow con `EPSG:99999` e
   conserva l'assenza della definizione nel roundtrip;
5. diagnostica redatta e `LossReport` trattano la definizione assente senza
   inventare valori o occorrenze.

La posizione R16.3 del componente è `accetta`, senza deroga né rilievo
bloccante. L'intenzione di ratifica è registrata ma la ratifica non viene
assunta da questa release.

## Stato normativo e residui

R4.1.1 è implementata contro la revisione esatta sopra indicata. Finché resta
proposta, il claim è di implementazione candidata e non di conformità a una
regola ratificata.

Restano esplicitamente fuori dal claim:

- R2.8, riconoscimento geometrico IPC dalle sole chiavi canoniche;
- confronto semantico fra `crs_definition` e SRID, che richiede un resolver;
- budget condivisi R7.5–R7.7;
- qualifica inversa e Windows della catena a tre componenti;
- consumo machine-readable di `read_loss` da parte del runner esterno;
- revisione indipendente.

Una passata della matrice su un commit di `main`, anche se integralmente verde,
resta esplorativa. L'evidenza registrabile richiede il tag immutabile RC.2 e
un nuovo pin esterno a quel tag.

## Sequenza autorizzata

1. committare questa decisione e ottenere CI verde;
2. allineare workspace, lockfile e nuovi manifesti a `1.0.0-rc.2`, mantenendo
   `component_rc: false`;
3. eseguire la CI sulla revisione pre-tag;
4. legare SHA e run CI nel record finale;
5. eseguire la CI del record finale;
6. creare e pushare il tag annotato non firmato `v1.0.0-rc.2`;
7. verificare la CI attivata dal tag;
8. consegnare il tag al runner esterno per la qualifica registrata.
