# Perimetro della release candidate di componente

Data del freeze tecnico: 2026-07-30.

## Dichiarazione

La release candidate preparata da questo repository è una **RC del componente
IO-tools**. Non è una RC del sistema Plenora e non dichiara conformità o
certificazione avionica.

La revisione candidata del codice è
`dc85f5163860bd16c4cf0bfa1066276980d38e8c` ed è la baseline tecnica
congelata. La CI candidata `30605882153` è verde sui job `rust`, `windows`,
`macos-publish` e `coverage`.

La decisione RC4 è committata in
`322ff57abd872f728d3f4e10c50c800ad39fa29c` e la relativa CI
`30606393196` è verde. La revisione pre-tag
`f5dc5d46668062b4016ac9e50229bc869a12d380` ha CI `30606830974` verde.
Il record finale `8d3f25f109f6ea8910da71e098db6924438e481c` ha CI
`30607124206` verde ed è il target del tag annotato `v0.1.0-rc.4`; anche la CI
del tag `30607373426` è verde.

RC3 resta pubblicata e immutabile come tag annotato non firmato
`v0.1.0-rc.3`, con target
`ea0de79677e8fc794d96ac3d95c5bc2c6e30358c`.

## Identificativi distinti

- `plenora.contract.version=1` identifica il formato wire emesso negli schemi
  Arrow;
- `0.1.0-rc.3` identifica la precedente release SemVer immutabile;
- `0.1.0-rc.4` identifica la baseline congelata e il tag annotato
  `v0.1.0-rc.4`;
- `plenora-contracts@v2.0-rc8`, revisione
  `62b12e3496466d2c908dac3cc098640b99b52e21`, identifica la revisione dell'ICD
  usata come obiettivo di implementazione.

Gli identificativi non sono intercambiabili. Il tag ICD è annotato ma non
firmato e il relativo candidato di ratifica resta parzialmente ratificato.

## Stato normativo adottato

La RC implementa anticipatamente parti candidate di §2, §3.4,
§4.3.1–§4.3.3, §9 e §11. Questa scelta non ne cambia lo stato normativo:

- non viene dichiarata conformità completa a `v2.0-rc8`;
- l'emissione delle chiavi candidate di §2 prima della ratifica resta
  registrata secondo §15.4 e `DER-ICD-002`;
- la condizione di rientro è la ratifica con nomi compatibili oppure la
  migrazione dell'emettitore verso la forma ratificata sostitutiva.

## Cosa attesta la RC

RC4 attesta esclusivamente che il componente, alle revisioni
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
Il passaggio a `1.0.0-rc.1` resta bloccato dalla decisione owner sul comando che
combina bordo di lettura e scrittura con rappresentazioni CRS discordanti. Le
riduzioni di scope e i blocker che riemergono per claim più forti sono
machine-readable nel manifesto RC5.

Il materiale per un'eventuale review è raccolto nel
[`pacchetto di revisione indipendente`](INDEPENDENT_REVIEW_PACKET.md); il
record resta `pending_eligible_reviewer` finché una persona eleggibile non
registra identità, comandi, rilievi ed esito.
