# Perimetro della release candidate di componente

Data della preparazione: 2026-07-28.

## Dichiarazione

La release candidate preparata da questo repository è una **RC del componente
IO-tools**. Non è una RC del sistema Plenora e non dichiara conformità o
certificazione avionica.

L'identificatore della RC e il relativo commit saranno assegnati soltanto al
freeze. Fino a quel momento lo stato machine-readable è `pre_freeze` in
[`release/contract-provenance.json`](../../release/contract-provenance.json).
L'impatto di questa preparazione è registrato nella
[`CIA dedicata`](CHANGE_IMPACT_2026-07-28_RC_PROVENANCE_FUZZ_COORDINATION.md).

## Due versioni distinte

- `plenora.contract.version=1` identifica il formato wire emesso negli schemi
  Arrow;
- `plenora-contracts@v2.0-rc8`, revisione
  `62b12e3496466d2c908dac3cc098640b99b52e21`, identifica la revisione dell'ICD
  usata come obiettivo di implementazione.

I due numeri non sono intercambiabili. Il tag ICD è annotato ma non firmato e
il relativo candidato di ratifica è esplicitamente non ratificato.

## Stato normativo adottato

La RC implementa anticipatamente parti candidate di §2, §3.4, §4.3.1–§4.3.3,
§9 e §11. Questa scelta non ne cambia lo stato normativo. In particolare:

- non viene dichiarata conformità completa a `v2.0-rc8`;
- l'emissione delle chiavi candidate di §2 prima della ratifica è registrata
  localmente secondo il passo 1 di §15.4 e la deroga `DER-ICD-002`;
- la condizione di rientro è la ratifica con nomi compatibili oppure la
  migrazione dell'emettitore verso la forma ratificata sostitutiva.

Un consumatore della RC adotta quindi un'interfaccia candidata che può ancora
richiedere una migrazione.

## Cosa attesta la RC

La RC attesta esclusivamente che il componente, alle revisioni registrate:

- supera i propri test, lint e gate di assurance;
- emette e valida il wire contract candidato dichiarato;
- espone gli errori a quattro assi e la cancellazione cooperativa;
- rispetta la matrice di capability pubblicata da IO-tools.

Non attesta che data-tools propaghi tutte le proprietà né che database-tools le
consumi. Quella dichiarazione appartiene esclusivamente al gate di sistema in
[`SYSTEM_RC_GATE.md`](SYSTEM_RC_GATE.md).

## Checklist di freeze

Il freeze può avvenire quando:

1. `contract-provenance.json` è verificato dalla CI e riporta tag e revisione
   ICD esatti;
2. la revisione sorgente finale sostituisce `implementation_revision` e
   `freeze_status` passa da `pre_freeze` a `frozen`;
3. test workspace, all-features, safety Clippy, FileGDB feature-on, benchmark e
   gate documentali sono verdi sulla revisione finale;
4. la matrice delle capability è allegata all'evidenza di release;
5. la revisione indipendente registra autore, revisore, rilievi ed esito;
6. il fuzzing WKB/EWKB lungo resta sospeso finché il protocollo condiviso non è
   accettato da IO-tools e database-tools.

Il passaggio a `frozen` richiede una nuova CIA perché modifica la baseline
citabile. Nessun tag di release è creato da questo documento.
