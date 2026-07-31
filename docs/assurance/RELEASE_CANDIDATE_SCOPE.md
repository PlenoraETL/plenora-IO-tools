# Perimetro della release candidate di componente

Data del freeze tecnico corrente: 2026-07-31.

## Dichiarazione

La release candidate preparata da questo repository è una **RC del componente
IO-tools**. Non è una RC del sistema Plenora e non dichiara conformità o
certificazione avionica.

La revisione candidata del codice è
`63a82531f82c4d3d42372fa8499ba1678ae4344b` ed è la baseline tecnica
congelata per `1.0.0-rc.2`. La CI candidata `30625336681` è verde sui job
`rust`, `windows`, `macos-publish` e `coverage`.

La decisione RC2 è committata in
`2d5d606cfb6f83e7a10c5b3e0c05fa3987c5eab4` e la relativa CI
`30627985036` è verde. La revisione e la CI pre-tag devono ancora essere
legate al record prima della creazione del tag annotato `v1.0.0-rc.2`.

La RC precedente resta pubblicata e immutabile come tag annotato non firmato
`v1.0.0-rc.1`, con target
`6e3a942dfd607c8bf4bdbe0075c8e8f5f3761842`.

## Identificativi distinti

- `plenora.contract.version=1` identifica il formato wire emesso negli schemi
  Arrow;
- `v1.0.0-rc.1` identifica la precedente release SemVer immutabile;
- `1.0.0-rc.2` identifica la baseline candidata corrente;
- `plenora-contracts@v2.0-rc14`, revisione
  `65fd2c6418efa7937e3063245913d79a80c6499b`, identifica la revisione dell'ICD
  usata come obiettivo di implementazione.

Gli identificativi non sono intercambiabili. Il tag ICD è annotato ma non
firmato e il relativo candidato di ratifica resta parzialmente ratificato.

## Stato normativo adottato

La RC implementa anticipatamente parti candidate di §2, §3.4,
§4.3.1–§4.3.3, §9 e §11. Questa scelta non ne cambia lo stato normativo:

- non viene dichiarata conformità completa a `v2.0-rc14`;
- l'emissione delle chiavi candidate di §2 prima della ratifica resta
  registrata secondo §15.4 e `DER-ICD-002`;
- la condizione di rientro è la ratifica con nomi compatibili oppure la
  migrazione dell'emettitore verso la forma ratificata sostitutiva.

## Cosa attesta la RC

La RC attesta esclusivamente che il componente, alle revisioni
registrate:

- supera test, lint, gate di assurance e soglia di coverage;
- emette e valida il wire contract candidato dichiarato;
- applica reader bounded XLSX/KML/DXF e pushdown fisico OpenFileGDB;
- supera la matrice nativa Windows GDAL/OpenFileGDB e i benchmark con veto;
- espone gli errori a quattro assi, la cancellazione e le capability dichiarate.

Non attesta che data-tools propaghi tutte le proprietà né che database-tools le
consumi. Quella dichiarazione appartiene al gate di sistema in
[`SYSTEM_RC_GATE.md`](SYSTEM_RC_GATE.md).

## Freeze tecnico e residui

La baseline congelata non può essere sostituita in-place: qualunque modifica
al codice genera un nuovo candidato e una nuova change impact analysis.

Restano esplicitamente fuori dal claim:

1. confronto semantico fra `crs_definition` e SRID;
2. esposizione machine-readable del `LossReport` reader nella CLI;
3. decisione owner sul comando che combina bordo di lettura e scrittura in
   presenza di CRS discordanti;
4. chiarimento end-to-end del campo `loss.lossless`, oggi riferito al writer;
5. qualifica della matrice a tre componenti.

La revisione indipendente resta aperta e non blocca una component RC con claim
`verified_internally`; blocca invece qualunque promozione a
`verified_independently`.

Lo stato machine-readable è in
[`release/contract-provenance.json`](../../release/contract-provenance.json),
[`release/freeze-readiness.json`](../../release/freeze-readiness.json) e
[`release/evidence/technical-freeze-v0.1.0-rc.4.json`](../../release/evidence/technical-freeze-v0.1.0-rc.4.json).
La decisione e la sequenza autorizzata sono nella
[`CIA RC4`](CHANGE_IMPACT_2026-07-30_RC4_RELEASE_DECISION.md).

Le modifiche successive appartengono allo sviluppo `0.1.0-rc.5`, registrato
separatamente in
[`release/rc5-development.json`](../../release/rc5-development.json).
RC5 chiude l'osservabilità machine-readable del `LossReport` reader e
l'ambiguità writer/end-to-end del vecchio `loss.lossless`. Registra inoltre
come candidata alla compatibilità 1.x soltanto la superficie JSON della CLI:
le API Rust restano interne per non bloccare l'estrazione prevista da R15.4.1.
La capability generale delle rappresentazioni CRS e il preflight candidato
R4.6.5 chiudono il precedente blocker sul comando che combina lettura e
scrittura. Il codice RC5 è congelato come candidato `1.0.0-rc.1` sulla
revisione `796a1f9`, verificata dalla CI `30617658910`; la decisione
`409cf93` ha CI `30618471105` verde. La revisione pre-tag `cea2535` ha CI
`30619205139` verde e il record finale autorizza il tag annotato
`v1.0.0-rc.1`, target `6e3a942`; anche la CI del tag `30619802027` è verde.
La qualifica esterna `plenora-contracts@c3f1a8e` misura `83/84` roundtrip e
`27/28` nella catena sulla sola RC.1: l'unico fallimento è il caso R4.1.1.
La candidata `1.0.0-rc.2` implementa quel caso sulla revisione `63a8253`,
verificata dalla CI `30625336681`; la decisione `2d5d606` ha CI
`30627985036` verde. Il record pre-tag è separato in
[`release/rc6-development.json`](../../release/rc6-development.json): finché
non esiste il tag immutabile, l'esito esplorativo della matrice non è evidenza
di qualifica. R2.8 resta proposta e non implementata. Le riduzioni di scope e
i blocker per claim più forti sono machine-readable nel manifesto RC6.

Il materiale per un'eventuale review è raccolto nel
[`pacchetto di revisione indipendente`](INDEPENDENT_REVIEW_PACKET.md); il
record resta `pending_eligible_reviewer` finché una persona eleggibile non
registra identità, comandi, rilievi ed esito.

L'evidenza post-tag e i limiti che impediscono di leggerla come system RC sono
registrati nella
[`CIA di qualifica post-tag`](CHANGE_IMPACT_2026-07-31_1_0_RC1_POST_TAG_QUALIFICATION.md).
