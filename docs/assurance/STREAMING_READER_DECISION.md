# Decisione sui reader materializzanti

Stato al 2026-07-28: **intervento valutato e non mantenuto per KML; DXF e
XLSX bloccati da prerequisiti espliciti**.

La regola di accettazione della campagna è: nessuna riduzione di memoria viene
scambiata per un peggioramento sensibile del throughput, e nessun reader
inventa uno schema per poter partire prima.

## KML

Il parser `kml` espone `KmlReader::read`, che restituisce il documento, non un
iteratore pubblico di eventi/placemark. Sono stati provati due percorsi
file-backed e un parser per frammenti:

| Variante | Effetto memoria | Effetto throughput | Decisione |
|---|---:|---:|---|
| buffer 8 KiB | RSS ridotta | fino a -52% | respinta |
| buffer 1 MiB | RSS ridotta | fino a -22% | respinta |
| pull per frammenti | circa 406 → 212 MiB | circa 169k → 40k righe/s (-76%) | respinta |

Il codice dei prototipi è stato rimosso; il reader corrente è la baseline
prestazionale. Il writer diretto su `BufWriter` resta invece mantenuto perché
ha superato il veto.

Riapertura consentita soltanto con un parser event-based che:

- conservi `GeometryCollection`, stili e gerarchie KML richieste;
- produca gli stessi errori fail-closed del parser corrente;
- superi un confronto interlacciato su throughput, RSS e allocazioni.

## DXF

`dxf 0.6.1` espone `Drawing::load*` e l'iteratore sulle entità già
materializzate. L'iteratore progressivo interno `EntityIter` e il parser dei
code-pair sono `pub(crate)`. Duplicare quel parser nel componente creerebbe una
seconda implementazione non qualificata di un formato complesso.

Il reader progressivo resta bloccato finché non esiste:

- un iteratore upstream pubblico che preservi errori di parsing, blocchi,
  INSERT e riferimenti; oppure
- un fork governato, pinnato e sottoposto a change impact analysis propria.

Il writer DXF diretto è già streaming e applica il limite durante la
serializzazione.

## XLSX

`calamine 0.36.x` espone `worksheet_cells_reader`, quindi l'accesso fisico
progressivo alle celle esiste. Il blocco è semantico: il contratto corrente
inferisce tipo e nullability dall'intero foglio prima di pubblicare lo schema
Arrow. Una sola passata può essere esatta soltanto se il chiamante fornisce
uno `schema_hint` governato; senza hint deve:

- fare due passate;
- bufferizzare valori;
- oppure scegliere tipi prima di vedere tutti i dati, violando H-01.

`schema_hint` non compare nell'ICD v2.0-rc8 e non viene aggiunto unilateralmente.
Quando sarà ratificato, il benchmark dovrà confrontare almeno:

- baseline materializzante;
- due passate senza buffering;
- una passata con schema dichiarato;
- file wide, sparse, shared strings, date, formule ed errori cella.

## Esito

Il punto non è “rimandato senza analisi”: KML ha prodotto una misura negativa
e ha attivato il veto; DXF ha un blocco di API verificabile; XLSX ha un blocco
contrattuale verificabile. Nel profilo aeronautico i tre reader restano
esplicitamente fuori dal sottoinsieme streaming bounded e non possono essere
descritti come cancellabili durante la chiamata upstream sincrona.
