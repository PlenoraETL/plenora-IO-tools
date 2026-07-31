# Change impact — R4.1.1, identificatore CRS irrisolto senza definizione

**Data:** 2026-07-31

**Baseline:** `v1.0.0-rc.1`

**ICD esaminato:** `plenora-contracts v2.0-rc14`, revisione
`c3f1a8ef2e6950570a33adf5a964f7f40e9cf1ab`

**Posizione R16.3:** `accetta`, senza deroga né rilievo bloccante

**Stato normativo al momento dell'intervento:** ratifica owner annunciata, non
assunta da questo componente

## Problema

`RawCrs::definition` era obbligatoria. Il reader IPC respingeva quindi una
colonna GeoArrow che dichiarava `crs_resolution=declared_unresolved` e il solo
`crs_id`, pur non essendo quello stato né `resolved` né `missing`. Nella
qualifica esterna post-tag questo era l'unico fallimento: `83/84` roundtrip e
`27/28` nella catena, caso `crs_unresolved__geoarrow`.

## Decisione

`RawCrs` conserva ora `definition` e `definition_format` come valori opzionali.
Il formato della definizione resta presente se e solo se è presente la
definizione. Il costruttore `from_authority_hint` rappresenta il caso con il
solo identificatore senza sintetizzare WKT, PROJJSON, SRID o un CRS operativo.

Il codec dei metadati accetta `declared_unresolved` quando è presente almeno
una dichiarazione fra `crs_id` e `crs_definition`; continua a rifiutare lo
stato se entrambe mancano. `axis_order` conserva la propria regola precedente:
deve essere dichiarato e non viene ricavato da una definizione assente.

In emissione, `crs_definition` e `crs_definition_format` sono entrambe omesse
quando non esistono. Il `LossReport` di scrittura e la diagnostica redatta
trattano l'assenza come tale, senza attribuirle byte o perdita inventata.

## Impatto e compatibilità

L'accettazione di un input prima respinto è additiva per il protocollo CLI. Le
sei buste JSON candidate alla compatibilità 1.x non cambiano. Il passaggio a
`Option<String>` e `Option<CrsDefinitionFormat>` modifica soltanto l'API Rust,
che è dichiarata interna, `publish = false` e fuori dalla superficie SemVer
congelata.

Il tag immutabile `v1.0.0-rc.1` e la sua qualifica `83/84`/`27/28` restano
record storici invariati. Questa revisione è una candidata successiva: il
passaggio atteso a `84/84` e `28/28` deve essere dimostrato rieseguendo la
matrice esterna sul nuovo commit, non dedotto dai test locali.

R2.8 resta separata e non implementata: senza `ARROW:extension:name`, il
driver IPC conserva ancora le chiavi canoniche come metadati opachi ma non
riconosce semanticamente la colonna come geometrica.

## Verifica

- regressione del modello: identificatore `EPSG:99999`, definizione e formato
  assenti, stato non operativo;
- roundtrip dei metadati con il solo identificatore;
- rifiuto di `declared_unresolved` senza identificatore né definizione;
- apertura IPC reale della variante GeoArrow con il solo identificatore e
  riemissione senza chiavi inventate;
- `cargo check --workspace --all-targets --locked`;
- `cargo test --workspace --all-targets --locked`;
- rustfmt, Clippy e gate assurance del repository.

Esito locale: tutti i controlli sopra sono verdi. Clippy è stato eseguito
anche con `--all-features` e i lint safety; la suite FileGDB feature-on è verde
su GDAL 3.6.2. La matrice esterna a tre componenti resta intenzionalmente una
prova separata.
