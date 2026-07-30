# Perimetro della release candidate di componente

Data del freeze tecnico: 2026-07-30.

## Dichiarazione

La release candidate preparata da questo repository è una **RC del componente
IO-tools**. Non è una RC del sistema Plenora e non dichiara conformità o
certificazione avionica.

La revisione candidata del codice è
`3f3562a4707995549ff5eb8dc03f9e37f2cde355` ed è la baseline tecnica
congelata. Lo stato machine-readable è `frozen` in
[`release/contract-provenance.json`](../../release/contract-provenance.json).
La decisione RC3 autorizza il candidato del componente con claim
`verified_internally`, senza trasformare la review aperta in un blocco. La
revisione pre-tag `ab330f8dfbcc7235c418e3e04f988317d3070525` ha CI
`30501904391` verde ed è legata nel record finale. Il tag viene materializzato
soltanto dopo la CI verde del record finale. L'impatto è registrato nella
[`CIA dedicata`](CHANGE_IMPACT_2026-07-30_RC3_RELEASE_DECISION.md).

RC2 resta pubblicata e immutabile come tag annotato non firmato
`v0.1.0-rc.2`, con target `f47bf4605b248d127205e49a7e6ebd2a0984a83f`.
RC3 è nello stato `component_rc_tagged`, separato in
[`release/rc3-development.json`](../../release/rc3-development.json). Il
precedente `v0.1.0-rc.1` resta a sua volta immutabile.

## Identificativi distinti

- `plenora.contract.version=1` identifica il formato wire emesso negli schemi
  Arrow;
- `0.1.0-rc.2` identifica la precedente release SemVer immutabile;
- `0.1.0-rc.3` identifica la baseline candidata congelata e il record finale
  destinato al tag annotato `v0.1.0-rc.3`;
- `plenora-contracts@v2.0-rc8`, revisione
  `62b12e3496466d2c908dac3cc098640b99b52e21`, identifica la revisione dell'ICD
  usata come obiettivo di implementazione.

Gli identificativi non sono intercambiabili. Il tag ICD è annotato ma non
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
[`decisione rc.3`](CHANGE_IMPACT_2026-07-30_RC3_RELEASE_DECISION.md)
separa i prerequisiti della RC dagli attributi di assurance: la review
indipendente non è un gate per il freeze o per il tag di una RC dichiarata
`verified_internally`.

Restano necessari per claim successivi:

1. una revisione indipendente prima di promuovere il claim a
   `verified_independently`;
2. le altre evidenze aperte prima di eventuali claim più forti. La campagna
   lunga coverage-guided RC3 è stata completata su harness committati, con zero
   finding.

Lo stato corrente è machine-readable in
[`release/freeze-readiness.json`](../../release/freeze-readiness.json):
la baseline tecnica `3f3562a` è congelata e la CI candidata `30500304709` è
verde su Linux, Windows, macOS e coverage. La decisione committata in
`6868990` ha CI `30501136176` verde; la revisione pre-tag `ab330f8` ha CI
`30501904391` verde. `independent_review=false` limita soltanto il livello del
claim; `component_rc=true` e `release_tag_status=created` appartengono al
record finale che diventa il target del tag soltanto dopo la propria CI verde.
Il file `release/rc3-development.json` dichiara i risultati inclusi in RC3, gli
attributi non bloccanti e i tre workstream differiti a RC4.
La decisione di perimetro è registrata nella
[CIA del 30 luglio](CHANGE_IMPACT_2026-07-30_RC3_CRS_SCOPE.md). La CI finale e
la verifica del tag remoto chiudono la sequenza di pubblicazione senza
modificare la baseline di implementazione.

Il materiale da consegnare al revisore è raccolto nel
[`pacchetto di revisione indipendente`](INDEPENDENT_REVIEW_PACKET.md); il record
machine-readable resta `pending_eligible_reviewer` finché una persona
eleggibile non registra identità, comandi, rilievi ed esito.
