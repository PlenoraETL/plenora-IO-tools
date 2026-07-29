# Change impact analysis — completamento degli otto punti

Data: 2026-07-28.

## Baseline e configurazione

- IO-tools baseline immutabile:
  `1c37fb5d525647b264ce977e26fc07b346bb7914`;
- ICD: `plenora-contracts@v2.0-rc8`,
  `62b12e3496466d2c908dac3cc098640b99b52e21`;
- data-tools osservato:
  `97e48ba469f9f55a2cc83e9598d72899c29e2be6`;
- database-tools corrente:
  `2588523bf6a4ad57e62ae3d44e9f58025c55a913`;
- database-tools usato per il replay EWKB:
  `ef18e80c798126f872fd366c36ee96a029598958`.

La revisione candidata del codice IO è
`92be3f4cd9a84b4dffbfd8b1621cc85a6ec9aa7a`. È una baseline immutabile
verificata localmente, ma non una RC congelata: CI candidata, revisione
indipendente e tag restano separati.

## Modifiche e impatto

1. La baseline CI è stata rieseguita su Linux, Windows e macOS. L'artifact LCOV
   misura 12.616/15.112 linee di libreria (83,48%) e il suo SHA-256 è
   registrato nel bundle di evidenza.
2. È stato introdotto un corpus WKB/EWKB deterministico di 18 casi, con replay
   sui due parser e classificazione obbligatoria delle divergenze. Non cambia
   il wire format.
3. Un harness usa componenti reali per la tratta IO → data → database. Ha
   rilevato e chiuso la mancata propagazione di `plenora.field_id` e ha reso
   esplicita nel profilo IPC la rappresentazione del CRS assente.
4. FileGDB ha un test di determinismo semantico. Il pushdown Arrow resta esatto,
   ma il pushdown nativo OpenFileGDB non viene dichiarato perché l'API safe
   richiesta non è esposta dalla dipendenza pinnata.
5. I prototipi KML streaming hanno attivato il veto prestazionale e sono stati
   rimossi. DXF e XLSX restano materializzanti per blocchi rispettivamente
   upstream e contrattuali, documentati senza promesse prestazionali inventate.
6. `GeometryType` copre i 16 nomi canonici R3.1. I codec e i driver continuano
   a dichiarare soltanto i sette tipi semplici realmente supportati e
   rifiutano gli altri nove prima dell'effetto esterno.
7. La matrice di tracciabilità collega coverage, fuzz, FileGDB e catena reale a
   evidenze riproducibili, senza dichiarare certificazione aeronautica.
8. I manifest RC distinguono baseline, worktree candidato e freeze. Un gate
   fail-closed impedisce di presentare come congelata una revisione non
   committata, priva di CI candidata o revisione indipendente.

## Hazard interessati

- H-01: identità campo, tipi estesi, WKB/EWKB e CRS non devono essere persi o
  reinterpretati silenziosamente;
- H-03: conteggi WKB avversari e reader materializzanti richiedono limiti
  dichiarati;
- H-06: CRS assente, irrisolto e risolto devono restare distinti;
- H-07: toolchain, revisioni, corpus e ambiente devono essere identificabili;
- H-08: test di componenti isolati non sostituiscono l'evidenza della catena;
- H-09: freeze e promozione richiedono una baseline citabile e review.

## Compatibilità

L'aggiunta delle nove varianti canoniche all'enum pubblico è additive sul wire,
ma rende intenzionalmente fail-closed i match esaustivi e i contratti
`unresolved` verso driver che supportano soltanto i sette tipi semplici.
L'aggiunta di `plenora.field_id` ai metadati IPC è compatibile con consumer che
ignorano chiavi sconosciute; valori malformati sono ora rifiutati.

La versione del descrittore IPC passa a 6 perché cambia la capability CRS
dichiarata. Non viene promossa la wire version globale.

## Verifica eseguita

- `cargo test --workspace --all-targets --all-features --locked`: pass;
- safety Clippy sui target `lib`, all-features e dipendenze locked: pass;
- FileGDB feature-on su Linux x86_64, Rust 1.92.0, GDAL 3.10.3: pass;
- replay WKB/EWKB: 18 casi, zero divergenze non classificate;
- smoke fuzz: seed 20260728, 68.740.000 iterazioni/60 s, zero finding;
- catena IO → data → database: Point XYZ, 3 → 2 righe, pass;
- gate documentali e generator check: richiesti dalla CI.

La CI candidata `30412487233`, head
`0dbd4fedfa2f3494cc2692d4e1f5b1e169024ed7`, ha successivamente superato tutti
i job Linux/Rust, Windows, macOS e coverage. Il filtro librerie misura
12.675/15.175 linee (83,53%); l'hash SHA-256 dell'artifact LCOV è registrato
nel bundle machine-readable.

## Residui e decisione

Non sono chiusi MC/DC, qualifica strumenti, campagna fuzz lunga, catena inversa,
sette fixture di sistema, FileGDB nativo Windows, matrice multi-GDAL e
preemption delle chiamate upstream KML/DXF/XLSX. Per questo:

- FileGDB e i reader materializzanti restano fuori dal sottoinsieme operativo
  aeronautico congelabile;
- la RC di sistema resta `not_satisfied`;
- la RC di componente resta `pre_freeze`;
- ogni peggioramento misurato nella futura sostituzione streaming riattiva il
  veto prestazionale.
