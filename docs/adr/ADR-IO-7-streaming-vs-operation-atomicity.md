# ADR-IO 7 — Streaming vs operation-atomicity del `BudgetedReader`

**Stato:** Accettato (Lotto 0, step S0, 2026-08-16). Documento normativo.
Ratificato insieme al pacchetto `docs/DECISION-PACKAGE-Lotto-0.md`, che ne
recepisce l'**opzione A** come vincolo di progetto (INV-5, INV-8,
`buffering = AdaptiveMemoryThenDisk`, `effective_delivery = OperationAtomic`).
Sblocca il lotto L2 di `ROADMAP-1.1.0.md` (finding #2 della review
2026-08-15).

**Perimetro della ratifica**: la decisione vincola la semantica
dell'adapter comune di lettura, non le date di implementazione. Lo spool
bounded entra in M2/S2; i tre campi descriptor (`native_read_mode`,
`effective_delivery`, `buffering`) sono dichiarati dai driver in M5/S8. Fino
ad allora il comportamento resta operation-atomic e il wire `catalog` non
cambia.

**Opzioni non ratificate**: B (streaming con
`TerminatedAfterAcceptedBatches`) e C (ibrido) restano registrate qui come
alternative valutate e **scartate**; riaprirle richiede una nuova ADR, non
una revisione di questa.

## Contesto

Il contratto pubblico di `LayerReader::next_batch(&mut self) ->
Result<Option<RecordBatch>>` (ADR-IO 1) suggerisce una lettura
streaming pull-based: un chiamante idiomatico si aspetta che il primo
batch arrivi in tempo O(dimensione del primo batch), non in tempo
O(dimensione della sorgente).

`BudgetedReader` (adapter comune in
`plenora-io-core::driver::reader_adapters`) invece esegue
`drain_operation` durante la *prima* chiamata di `next_batch`: itera
tutta la sorgente fino a EOF, valida il contratto batch per batch, e
accumula i batch in una `VecDeque<RecordBatch>`. Solo dopo il drain
completo il primo batch viene consegnato al chiamante.

La ragione dichiarata nel codice (`reader_adapters.rs`, commento a
`drain_operation`): **atomicita' operativa**. Se una violazione emerge
in un qualsiasi punto della sorgente, il chiamante non deve aver mai
visto un prefisso accepted; l'intera operazione viene rigettata come
un blocco unico. Il pattern semplifica il rollback lato consumer
(writer, aggregazioni) al costo di:

- **latenza al primo batch** pari alla lettura completa della sorgente;
- **memoria** O(dataset), limitata dal budget `MemoryBytes` (default
  512 MiB); superata quella soglia l'operazione fallisce fail-closed.

Il finding #2 (chiuso come deferred/ADR nel documento
`REVIEW-2026-08-15.md`) segnala la tensione fra il naming
(`LayerReader`/`next_batch`, che suggerisce streaming) e il
comportamento reale (materializzazione totale). Un chiamante che usi
`BudgetedReader` come se fosse davvero streaming (per esempio in una
pipeline dove i batch vengono aggregati man mano) non ottiene ne'
latenza bassa ne' memoria bounded al singolo batch.

## Vincoli

- ADR-IO 1 vincola la firma di `LayerReader`, non la semantica interna
  dell'adapter comune. Modifiche a `BudgetedReader` non richiedono
  cambio di ADR-IO 1 ma cambiano il **comportamento osservabile** dei
  driver che lo usano.
- ADR-IO 2 richiede publish atomico per la scrittura. La
  operation-atomicity in lettura NON e' richiesta da ADR-IO 2: ADR-IO 2
  parla di scrittura.
- `plenora-contracts` congela la CLI su sei buste JSON. Un cambio
  della semantica di lettura potrebbe richiedere una variante nuova
  della busta di errore (per segnalare "consegnato prefisso, poi
  fallito"). L'ADR deve toccare anche i contratti CLI.

## Opzioni

### Opzione A — Conservare l'operation-atomicity con spool bounded

Mantenere il contratto attuale ("nessun prefisso accepted esposto in
caso di violazione anywhere") sostituendo la `VecDeque` in RAM con uno
**spool bounded su file temporaneo**. Il `drain_operation` continua a
consumare la sorgente fino a EOF, ma serializza i batch validi su disco
(nuovo `StagedSpool` in `plenora-io-core::publish`) e li rilegge in
streaming al chiamante.

Semantica invariata: primo batch dopo scan completo, ma memoria O(spool
budget) invece di O(dataset). Il chiamante non nota la differenza se
non per la nuova quota `ResourceKind::SpillBytes` che ora viene
realmente usata (oggi e' dichiarata ma non consumata da questo
adapter).

Vantaggi:
- il contratto pubblico resta identico;
- niente cambi ai consumer (writer, aggregatori);
- niente cambi ai contratti CLI;
- niente cambio di `PlenoraIoError` (la semantica di errore resta
  "operazione atomica fallita").

Costi:
- circa 400-600 righe di codice nuovo per `StagedSpool`, serializzazione
  Arrow IPC su tmpfile, rilettura lazy;
- costo su disco: i dataset validi passano da RAM a IPC-on-disk e
  ritorno. Overhead I/O misurabile su dataset >100 MB;
- persiste il costo di latenza al primo batch (nessuno streaming vero);
- una nuova risorsa da governare (spill space) — la campagna
  prestazionale deve confermare che il ratio scrittura/rilettura non
  domina il tempo utente.

Rischio: **medio**. Aggiunge una superficie I/O nuova (staging spool)
gia' testata in altri driver ma non integrata nel core reader.

### Opzione B — Rilasciare l'operation-atomicity e restituire batch in streaming

Cambiare la semantica di `BudgetedReader::next_batch` a streaming vero:
consegna un batch alla volta dal sorgente sottostante, applicando la
validazione batch-per-batch. Se una violazione emerge dopo che uno o
piu' batch sono gia' stati consegnati al chiamante, restituisce un
errore terminale nuovo:

```rust
PlenoraIoError {
    category: ErrorCategory::TerminatedAfterAcceptedBatches,
    // ... i batch gia' consegnati restano validi per l'aggregatore
    //     che li ha consumati; nessun rollback automatico.
}
```

Semantica: il chiamante che ha aggregato/pubblicato i batch precedenti
si assume la responsabilita' del rollback. La CLI `convert` (che oggi
copia tutti i batch in un `Vec` prima di scrivere) va aggiornata per
scrivere in streaming: `finish()` del writer diventa il punto di
publish atomico, l'errore prima di `finish()` risulta in nessuna
destinazione pubblicata.

Vantaggi:
- streaming reale: primo batch in tempo O(batch), memoria O(batch);
- eliminata l'accumulazione;
- allineato al naming di `LayerReader`.

Costi:
- **rottura di contratto**: la busta d'errore CLI acquisisce una
  categoria nuova (`TerminatedAfterAcceptedBatches`) — richiede bump
  di `plenora-io-error-v1` o wire version, con qualifica cross-component;
- la CLI `convert` va riscritta in streaming;
- consumer downstream (data-tools, database-tools) devono gestire il
  caso di errore post-consegna: rollback esplicito nei writer, o
  documentazione formale che "batch consegnati sono committati";
- la promessa `Ok(Published)` del writer resta valida solo perche' il
  writer chiama `finish()` dopo l'ultimo batch; ma un errore
  intermedio del reader lascia il writer in uno stato intermedio
  (nessun publish, ma buffer/staging popolato da abortire).

Rischio: **alto**. Cambia il contratto pubblico e ha impatto su tre
componenti Plenora.

### Opzione C — Ibrido: streaming per default, atomicity opt-in

Introdurre un parametro nel `ReadRequest`:

```rust
enum ReadAtomicity {
    Streaming,           // default: batch in streaming, errore terminale post-consegna
    OperationAtomic,     // materializzazione bounded via spool
}
```

Il chiamante sceglie esplicitamente. La CLI `convert` puo' usare
`OperationAtomic` (che invoca lo spool dell'opzione A internamente),
i chiamanti "streaming-aware" usano `Streaming` (opzione B).

Vantaggi:
- entrambe le semantiche disponibili;
- migrazione graduale possibile.

Costi:
- superficie API piu' grande;
- due percorsi di errore da mantenere;
- entrambi i costi di A e B insieme;
- il default va scelto e documentato: se `OperationAtomic` la
  regressione osservabile non c'e', se `Streaming` la CLI cambia
  comportamento.

Rischio: **medio-alto**. Duplica il codice ma preserva la
compatibilita'.

## Raccomandazione

**Opzione A**, con motivazione pragmatica. La operation-atomicity e' un
invariante che i consumer downstream stanno gia' assumendo (writer,
aggregatori nella CLI `convert`). Rimuoverla richiede coordinamento
cross-component che ha un costo di validazione elevato. Sostituire la
`VecDeque` in RAM con uno spool bounded chiude il problema di memoria
senza toccare il contratto pubblico.

L'opzione B resta la scelta corretta se in futuro emergessero pipeline
streaming-aware che richiedono latenza al primo batch bassa (per
esempio un file watch, o un canale continuo). Fino a quel momento
l'atomicita' e' un beneficio che vale il costo del disco.

L'opzione C e' l'opzione "non decidere": aggiunge complessita' senza
risolvere.

## Stato di attuazione (S2, 2026-08-16)

Attuata. `StagedSpool` vive in `plenora-io-core::driver::spool` e
`BudgetedReader` non usa piu' la `VecDeque`. I punti dove
l'implementazione **diverge dal piano** sono errata di questa ADR, non
scostamenti taciuti:

1. **Modulo**: `driver::spool`, non `publish`. Lo spool serve la lettura;
   `publish` governa la scrittura atomica e non ha nulla a che vedere.
2. **File temporaneo senza nome**, non "same-directory" con permessi e
   sweep degli orfani. `tempfile::tempfile_in` scollega l'inode appena
   aperto su Unix e usa `FILE_FLAG_DELETE_ON_CLOSE` su Windows. E' piu'
   forte del piano originale: nessun path che un altro processo possa
   aprire, quindi la protezione non dipende dai permessi; nessun orfano
   da spazzare nemmeno dopo un `SIGKILL`, quindi lo sweep su lock — con
   i suoi casi limite e le sue race — sparisce invece di dover essere
   reso corretto; nessuna finestra TOCTOU fra creazione e apertura;
   nessun symlink da seguire. `PLENORA_SPILL_DIR` resta e sceglie il
   volume che ospita l'inode, fallendo chiuso se inutilizzabile.
3. **Quota di spill RAII sui byte realmente scritti**, non `commit` sulla
   stima di occupazione in RAM. Le due grandezze divergono — l'IPC
   allinea, comprime i buffer di validita', aggiunge intestazioni — e un
   `commit` avrebbe consumato la quota per sempre, facendo esaurire lo
   spill a una pipeline lunga che ha gia' rimosso i suoi file.
4. **Memoria come prenotazione viva**, non consumo definitivo: e' il
   finding L0.3, chiuso insieme allo spool perche' senza di esso lo
   spool avrebbe spostato i byte su disco continuando a pagarli in RAM.
5. **Test end-to-end** su dataset oltre `memory_bytes`: presente
   (`dataset_over_memory_bytes_succeeds_via_spool`), insieme alla
   copertura del replay corrotto e della cancellazione durante
   migrazione e rilettura.
6. **CIA** registrata in
   `docs/assurance/CHANGE_IMPACT_2026-08-16_LOTTO_0_BUDGET_MODEL.md`,
   accorpata a quella del modello budget: le due modifiche condividono
   baseline, perimetro e hazard.

7. **Campagna prestazionale bounded** eseguita con un percorso senza spill e
   uno con spill forzato, entrambi su `convert` completo: sul percorso comune
   il delta e' **+2,7%**, dentro il veto e nell'ordine del rumore; il
   percorso con spill forzato **prima non completava affatto** e ora riesce
   nello stesso tempo del percorso senza spill. Risultati e limiti della
   misura in `CHANGE_IMPACT_2026-08-16_LOTTO_0_BUDGET_MODEL.md`.

## Fuori scope di questa ADR

- Semantica dei writer (ADR-IO 2 resta autoritativa).
- Semantica dei singoli driver che NON passano da `BudgetedReader`
  (KML/DXF/XLSX: hanno il proprio comportamento gia' documentato in
  `STREAMING_READER_DECISION.md`).
- Cambio del contratto `plenora-io-error-v1`.

## Riferimenti

- `docs/REVIEW-2026-08-15.md` — finding #2.
- `docs/ROADMAP-1.1.0.md` — lotto L2.
- `docs/assurance/STREAMING_READER_DECISION.md` — decisione precedente
  per KML/DXF/XLSX.
- `plenora-io-core/src/driver/reader_adapters.rs` — `BudgetedReader`
  attuale e commento inline sull'operation-atomicity.
