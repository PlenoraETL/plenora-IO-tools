# Protocollo coordinato di fuzzing WKB/EWKB

Stato: **protocollo proposto; campagna lunga non avviata**.

Baseline esaminate:

- IO-tools `59369fdfb6dbb5d1d7c97a29865ca39ae21c6f76`,
  codec lossless `plenora-io-model::wkb_lossless` e adattatore XY `wkb`;
- database-tools branch `assurance/ewkb-fuzzing`, revisione
  `834fff4fbe0c62cc2f02278073e58b0cf2159f8d`, scanner
  `plenora-database-core::ewkb`.

## Proprietà del corpus

Il corpus condiviso contiene soltanto payload WKB/EWKB grezzi. Byte di controllo,
limiti e metadati esterni non vengono anteposti al file: ciascun harness li
deriva da parametri separati. Questo rende riutilizzabili i casi dal target
database-tools, che oggi usa tre byte di controllo, e dal target IO-tools, che
oggi riceve direttamente il payload.

Ogni caso è identificato da SHA-256 e descritto tramite il formato
[`fuzz/shared-corpus-manifest.schema.json`](../../fuzz/shared-corpus-manifest.schema.json).
La sede futura proposta è
`plenora-contracts/conformance/wkb-ewkb/`; fino all'accettazione dei due team
nessun repository dichiara autorevole la propria copia locale.

## Invarianti comuni

Entrambe le implementazioni devono verificare:

1. nessun panic, hang o crescita non bounded su input arbitrario;
2. consumo completo dell'input e rifiuto dei trailing bytes;
3. controllo di byte, componenti, conteggi e profondità prima
   dell'allocazione corrispondente;
4. aritmetica controllata su offset, lunghezze e prodotti;
5. stabilità dell'esito quando i limiti accettati vengono allentati;
6. identità di tipo, dimensioni, endianess normalizzata e SRID dopo un
   round-trip lossless;
7. coerenza tra SRID EWKB incorporato e SRID/CRS dichiarato nei metadati;
8. rifiuto fail-closed di type word, flag dimensionali o gerarchie incoerenti.

I seed devono coprire WKB ed EWKB, little/big endian, empty e null dove
applicabili, XY/XYZ/XYM/XYZM, collezioni annidate, conteggi avversari,
troncamenti a ogni offset e SRID discordanti.

## Oracolo differenziale

Per ogni payload si registrano separatamente:

- accettato/rifiutato;
- categoria portabile dell'errore;
- tipo geometrico canonico;
- dimensioni;
- SRID radice;
- componenti e profondità osservate;
- hash della ricodifica canonica, quando disponibile.

Una differenza non è automaticamente un bug. Deve essere classificata come:

- `defect_io`;
- `defect_database`;
- `intentional_capability_difference`;
- `ambiguous_or_noncanonical_input`;
- `contract_gap`.

La classificazione deve citare il caso per SHA-256 e le due revisioni. Nessun
caso può essere rimosso dal corpus per far passare una campagna: viene corretto
il codec, aggiornato l'oracolo con motivazione oppure aperto un gap contrattuale.

## Sequenza operativa

1. IO-tools e database-tools approvano questo protocollo e lo schema del
   manifest.
2. I corpus esistenti vengono esportati come payload grezzi, deduplicati per
   SHA-256 e accompagnati dalle aspettative note.
3. Si esegue prima il replay deterministico incrociato, poi il fuzzing
   differenziale bounded.
4. Solo dopo il replay verde parte la campagna lunga coverage-guided.
5. Quando §15.3 viene ratificata e il codec migra nel crate condiviso, corpus e
   invarianti diventano proprietà del crate senza cambiare formato.

Gli smoke fuzz locali già presenti restano ammessi come regressione del
componente; non costituiscono la campagna coordinata.
