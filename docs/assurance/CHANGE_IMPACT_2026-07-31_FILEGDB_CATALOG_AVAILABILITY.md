# Change impact analysis — disponibilità FileGDB nel catalogo CLI

Data: 2026-07-31

## Problema

La build predefinita non abilita `gdal-backend`: `FileGdbDriver` è quindi uno
stub i cui `open` e `create` falliscono con `Unsupported`. Il comando `catalog`
registrava comunque FileGDB come `bidirectional` senza esporre la disponibilità
della build, inducendo i consumer machine-to-machine a trattarlo come operativo.

## Decisione

FileGDB resta nel catalogo per consentire discovery e diagnosi, ma ogni voce del
catalogo espone due campi build-specifici:

- `available`: booleano, vero soltanto quando il driver è operativo nella build;
- `required_feature`: feature Cargo richiesta, oppure `null`.

Nella build predefinita FileGDB dichiara `available: false` e
`required_feature: "gdal-backend"`. Con la feature abilitata conserva il nome
della feature ma dichiara `available: true` solo se la probe del runtime GDAL
trova `OpenFileGDB` e le capability `DCAP_VECTOR`, `DCAP_OPEN` e `DCAP_CREATE`
tutte a `YES`. Driver o capability mancanti producono `false` (fail-closed).
Gli altri driver puro-Rust dichiarano `available: true` e
`required_feature: null`.

L'esclusione della voce FileGDB è stata respinta: sarebbe fail-closed, ma
perderebbe la discovery della capability installabile e non soddisferebbe il
requisito di rendere machine-readable la feature necessaria.

## Impatto contrattuale

I campi sono aggiunti alle voci di `drivers` della busta
`plenora-io-catalog-v1`. Non vengono rimossi o rinominati campi esistenti e non
cambia il tipo di alcun campo esistente. L'estensione è quindi additiva secondo
`compatibility_rules.add_optional_field` del manifest v1: sono dichiarati in
`optional_driver_fields`, così i producer v1 precedenti restano conformi. Il
solo producer corrente li rende entrambi obbligatori tramite
`current_producer.required_driver_fields`; non serve una nuova versione.

La superficie Rust pubblica ma esplicitamente instabile aggiunge
`driver_filegdb::runtime_available`; i crate restano `publish = false` e senza
garanzia semver. Non cambiano formati su disco, dipendenze, versioni di crate o
la provenienza storica di release.

## Verifica richiesta

- test regressione della build senza feature, osservato RED prima della fix;
- test di tipo e semantica per tutti i driver, inclusa la probe runtime negativa;
- mutation test del checker su dichiarazione e semantica dei campi driver;
- mutation test che accetta un producer v1 legacy senza i campi additivi ma
  rifiuta un campo opzionale presente con tipo errato;
- test mirati e correlati di `plenora-io-cli` senza feature e, se disponibile,
  con `gdal-backend`;
- esecuzione in CI del binario CLI con `gdal-backend` contro GDAL/OpenFileGDB
  reale, con verifica fail-closed che `available` sia il booleano `true` e
  `required_feature` la stringa esatta `gdal-backend`;
- Clippy del crate CLI con warning negati;
- validazione JSON del manifest e controllo del catalogo emesso dal binario.

## Rischio residuo

La probe è non distruttiva e interroga il driver realmente registrato dal GDAL
caricato. Congela le capability di alto livello necessarie al descriptor
bidirezionale, ma non sostituisce i test di scrittura/lettura su dataset reali:
un runtime che dichiari capability errate può ancora fallire durante
`open`/`create`. In quel caso l'errore operativo resta tipizzato.
