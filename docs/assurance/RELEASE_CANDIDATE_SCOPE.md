# Perimetro della release candidate di componente

Data del freeze tecnico: 2026-07-29.

## Dichiarazione

La release candidate preparata da questo repository è una **RC del componente
IO-tools**. Non è una RC del sistema Plenora e non dichiara conformità o
certificazione avionica.

La revisione candidata del codice è
`179ad037aad18c3c92ff3c703315a7033ff43773` ed è la baseline tecnica
congelata. Lo stato machine-readable è `frozen` in
[`release/contract-provenance.json`](../../release/contract-provenance.json).
Il freeze non crea un tag, non dichiara una revisione indipendente e non
autorizza da solo una promozione assurance. La decisione `rc.2` autorizza il
candidato del componente con claim `verified_internally`, senza
trasformare la review aperta in un blocco. L'impatto del freeze è registrato nella
[`CIA dedicata`](CHANGE_IMPACT_2026-07-29_RC2_RELEASE_DECISION.md).

La versione dei 16 crate del workspace è `0.1.0-rc.2`. Il tag omonimo è
autorizzato ma non ancora creato: il record dichiara `pending_pre_tag_ci`.
Il precedente `v0.1.0-rc.1` resta pubblicato, annotato, non firmato e
immutabile.

## Identificativi distinti

- `plenora.contract.version=1` identifica il formato wire emesso negli schemi
  Arrow;
- `0.1.0-rc.2` identifica la versione SemVer dei crate del componente;
- `plenora-contracts@v2.0-rc8`, revisione
  `62b12e3496466d2c908dac3cc098640b99b52e21`, identifica la revisione dell'ICD
  usata come obiettivo di implementazione.

I tre identificativi non sono intercambiabili. Il tag ICD è annotato ma non
firmato e il relativo candidato di ratifica è esplicitamente non ratificato.

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

## Freeze tecnico e promozione assurance

Il freeze tecnico della baseline è avvenuto dopo:

1. `contract-provenance.json` è verificato dalla CI e riporta tag e revisione
   ICD esatti;
2. la revisione sorgente finale è registrata in `implementation_revision`;
3. test workspace, all-features, safety Clippy, FileGDB feature-on, benchmark e
   gate documentali sono verdi sulla revisione finale;
4. la matrice delle capability è allegata all'evidenza di release;
5. il replay deterministico WKB/EWKB resta verde;
6. il bundle `release/evidence/` identifica ambiente, comandi, artifact e gap
   senza trasformare evidenza locale in verifica indipendente.

La baseline congelata non può essere sostituita in-place: qualunque modifica al
codice genera un nuovo candidato e una nuova CIA. La
[`decisione rc.2`](CHANGE_IMPACT_2026-07-29_RC2_RELEASE_DECISION.md)
separa i prerequisiti della RC dagli attributi di assurance: la review
indipendente non è un gate per il freeze o per il tag di una RC dichiarata
`verified_internally`.

Restano necessari:

1. una CI verde sul commit che registra la decisione rc.2;
2. una CI verde sulla revisione pre-tag che allinea versione e manifesti;
3. una revisione indipendente prima di promuovere il claim a
   `verified_independently`;
4. la campagna lunga coverage-guided e le altre evidenze aperte prima di
   eventuali claim più forti.

Lo stato corrente è machine-readable in
[`release/freeze-readiness.json`](../../release/freeze-readiness.json):
la baseline tecnica `179ad03` è congelata e la CI candidata `30442548998` è
verde su Linux, Windows, macOS e coverage. `independent_review=false` limita
soltanto il livello del claim; `release_tag_created=false` e
`release_tag_status=pending_pre_tag_ci` impediscono di presentare il tag
`v0.1.0-rc.2` come già pubblicato.

Il materiale da consegnare al revisore è raccolto nel
[`pacchetto di revisione indipendente`](INDEPENDENT_REVIEW_PACKET.md); il record
machine-readable resta `pending_eligible_reviewer` finché una persona
eleggibile non registra identità, comandi, rilievi ed esito.
