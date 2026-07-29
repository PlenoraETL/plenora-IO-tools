# Perimetro della release candidate di componente

Data del freeze tecnico: 2026-07-29.

## Dichiarazione

La release candidate preparata da questo repository è una **RC del componente
IO-tools**. Non è una RC del sistema Plenora e non dichiara conformità o
certificazione avionica.

La revisione candidata del codice è
`78c2d150b9c7d0ac48e4c97b03f86228e0f0a068` ed è la baseline tecnica
congelata. Lo stato machine-readable è `frozen` in
[`release/contract-provenance.json`](../../release/contract-provenance.json).
Il freeze non crea un tag, non dichiara una revisione indipendente e non
autorizza da solo una promozione assurance. La successiva decisione di release
autorizza una RC del componente con claim `verified_internally`, senza
trasformare la review aperta in un blocco. L'impatto del freeze è registrato nella
[`CIA dedicata`](CHANGE_IMPACT_2026-07-29_TECHNICAL_FREEZE.md).

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
[`decisione di release interna`](CHANGE_IMPACT_2026-07-29_INTERNAL_RC_RELEASE.md)
separa i prerequisiti della RC dagli attributi di assurance: la review
indipendente non è un gate per il freeze o per il tag di una RC dichiarata
`verified_internally`.

Restano necessari:

1. una CI verde sul commit che registra la decisione prima del tag
   (soddisfatta: revisione `75ea508`, run `30435854122`);
2. una revisione indipendente prima di promuovere il claim a
   `verified_independently`;
3. la campagna lunga coverage-guided e le altre evidenze aperte prima di
   eventuali claim più forti.

Lo stato corrente è machine-readable in
[`release/freeze-readiness.json`](../../release/freeze-readiness.json):
la baseline tecnica è congelata, la CI candidata `30415766905` è verde su
Linux, Windows, macOS e coverage e la RC di componente è autorizzata come
`verified_internally`. `independent_review` e `release_tag_created` restano
esplicitamente `false`, ma soltanto la prima limita il livello del claim.

Il materiale da consegnare al revisore è raccolto nel
[`pacchetto di revisione indipendente`](INDEPENDENT_REVIEW_PACKET.md); il record
machine-readable resta `pending_eligible_reviewer` finché una persona
eleggibile non registra identità, comandi, rilievi ed esito.
