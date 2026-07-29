# Change impact analysis — dichiarazione CRS e perimetro RC3

Data: 2026-07-30.

## Decisione funzionale

Il rilievo storico che chiedeva a IO-tools di respingere in lettura la fixture
`conflicting_crs` è ritirato. Il bordo di lettura preserva le rappresentazioni
discordanti e le dichiara; il bordo di scrittura continua a fallire chiuso prima
del publish. La decisione su un'eventuale conciliazione resta al componente
centrale.

Il modello espone ora `authority_srid`, sollevato dalla stessa regola sintattica
già usata da database-tools: soltanto un identificatore `EPSG:<numero>` è
risolvibile senza un resolver CRS. Gli adattatori comuni dei reader confrontano
quel codice con `plenora.geometry.srid`. In caso di divergenza non modificano il
contratto e aggiungono al `LossReport` la categoria stabile
`inconsistent_crs_representations`, con un esempio bounded.

La modifica attua l'obbligo di non perdere informazione di R5, già ratificato.
È anche allineata alla collocazione proposta da R4.6.1, senza dichiararne per
questo la ratifica.

## Impatto e invarianti

- il controllo vale per WKB ed EWKB e per tutti i driver, perché vive nei due
  adattatori reader comuni;
- `crs_id`, `srid`, payload e metadati non vengono conciliati né riscritti;
- identificatori non EPSG non vengono indovinati;
- report già prodotti dal driver vengono conservati e la dichiarazione
  strutturale è idempotente quando gli adattatori sono composti;
- il controllo EWKB del writer resta invariato e fail-closed.

## Decisione di perimetro

RC3 comprende tre risultati tecnici coesi e già implementati:

1. codec lossless dei tipi WKB canonici;
2. campagna fuzz lunga con harness committato e provenienza riproducibile;
3. dichiarazione delle rappresentazioni CRS discordanti al bordo di lettura.

La riduzione della materializzazione in KML/DXF/XLSX, il pushdown nativo
OpenFileGDB e la matrice FileGDB/GDAL Windows sono tre iniziative indipendenti
e passano esplicitamente al programma RC4. Il rinvio non cambia le capability:
ciò che non è implementato continua a non essere dichiarato.

La review indipendente resta un attributo aperto e non blocca un claim
`verified_internally`. La ratifica delle sezioni candidate dell'ICD è una
decisione dell'owner, non un prerequisito tecnico esterno di IO-tools; le
sezioni già adottate restano registrate come tali senza anticipare la decisione.

`component_rc` resta `false` finché questa revisione non diventa una nuova
baseline immutabile e non supera i gate pre-tag.

## Verifica locale

Eseguito in immagine locale Rust 1.92:

```text
cargo test -p plenora-io-model -p plenora-io-core
79 passed; 0 failed
```

I test nuovi verificano che `EPSG:4326` con `srid=3003` sia preservato e
dichiarato una sola volta, e che `EPSG:4326` con `srid=4326` non produca una
segnalazione.
