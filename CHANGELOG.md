# Changelog

## 10.1.0 - non rilasciata

- L-IO7: FileGDB consegna alias e riferimenti ai domini nei campi Arrow, domini codificati e a intervallo tipizzati nel dataset, sottotipi e domini per sottotipo dalla definizione GDAL. SDK Python e inspect espongono gli stessi metadati. Ogni dominio porta anche descrizione, sottotipo del campo e politiche di divisione e unione; un dominio nativo che non si rappresenta per intero non viene trasportato ed e una perdita `native_domain_not_transported`, anche se nessun campo lo usa.
- Codec validati e chiavi qualificate nella tabella delle proprieta. Le perdite residue usano i codici enumerati `native_domain_not_transported`, `native_subtype_not_transported` e `native_alias_not_transported`; i metadati consegnati non sono perdite.
- Scrittura FileGDB dei domini e degli alias tramite GDAL; sottotipi non scrivibili tramite API GDAL e metadati non supportati dal writer GeoPackage dichiarati con i nuovi codici.
- Fixture GDAL indipendente con nomi non ASCII e prove Rust/SDK. Le misure fuzz FileGDB devono essere rifatte al congelamento della 10.1.0.

## 10.0.0 - 2026-11-18

### Cambiamenti incompatibili

- Rimossi confronto historical-readers, ricostruzione del reader 7.0.3 e relativi job e registri.
- Rimossi diagnostica Rust v1, frasi congelate, accessori `detail_v1` e `ragioni_v1` e guardia dedicata. I costruttori redatti accettano solo dettaglio curato e posizione.
- Ritirati ICD CLI v1, script delta e promesse di selezione del vecchio protocollo. Resta il solo protocollo 2 con i contratti correnti.
- Rimossi benchmark contro il tag rc.3 e baseline di prerelease; la misura FileGDB corrente conserva l'oracolo di righe e checksum.
- Rimosse opzioni GeoParquet `accept_legacy_crs_id_only` e `bbox_legacy_by_name`: CRS conformi e covering dichiarato sono obbligatori per quelle interpretazioni. Le colonne `_bbox_*` senza covering restano dati utente.
- Rimossi archivio eseguibile delle release precedenti, ricevute non piu richieste e note obsolete; handoff ridotto al contratto corrente.
- Allineata la busta di panico al protocollo 2; `plenora-error-v1` resta il contratto comune corrente.

La raccolta 10.0.0 ? congelata; le correzioni successive non attestano una nuova qualifica o pubblicazione.

## 9.0.0 - 2026-11-15

### Superfici e comportamento

- Sicurezza temporale I1–I7: conversioni DXF checked (durate in giorni), rifiuto
  preventivo GeoParquet INT96, formatter record Parquet senza panic o orari
  avvolti, setter GDAL esatto o errore. Candidate da ricongelare dopo la
  correzione per decisione del titolare; rimisure e verifica nativa ancora richieste.

- Python minimo 3.14, deciso dal titolare; fonte `.python-version`, ABI cp314
  e controlli CI su Linux e Windows. Workspace, SDK e binding nativo sono 9.0.0.
- **B14-GPKG-FIELD-ID**: anche i GeoPackage privi dell'estensione di identita
  espongono `plenora.field_id` per geometria, attributi e chiave primaria,
  nell'ordine del reader. Gli ID dichiarati rimangono distinti da quelli derivati.
- **B-11/B-15, piano dichiarativo**: gestione condivisa nel core delle proprieta
  Arrow/geometriche e delle trasformazioni dichiarate. La proiezione GPKG
  delega al core; report di perdita puntuali per campo/chiave e relative
  strutture nella CLI e nell'SDK Python. I controlli CRS e la semantica delle
  operazioni seguono i contratti adottati, senza propagare conflitti proibiti.
- AsyncClient e stub derivano le operazioni dal client sincrono. I test delle
  nuove superfici SDK sono eseguiti sul wheel installato; nessun salto implicito.
- Il runtime usa il parser JSON rigoroso del model. Il censimento dei quartetti
  segue i siti effettivi: il controllo GPKG condiviso resta in
  `project_layer_contract`, senza duplicare il sito locale.

### Qualifica, dipendenze e distribuzione

- Validatore di sistema riallineato a GEO-005/REJ-001: separazione di ingressi
  leciti, rifiuti CRS proibiti ed esclusioni lecite decise dal validatore IO.
  Il codice `C-G02` e facoltativo nei casi previsti dal contratto.
- Qualifica esterna congiunta conclusa il 15/11/2026 secondo i referti owner
  al pin IO `a5a90ef`, data 5.0.0, database 11.0.0 e ICD `38d68b4`.
  Per decisione del titolare il bundle non viene incluso nel repository e
  non e richiesto per la release; resta la registrazione documentale senza
  promuovere lo stato dell'invariante cross-component.
- Allineamento delle dipendenze alla politica comune e snapshot del manifesto
  di famiglia 14-11b; aggiornamento dei lock e dell'impronta del fork Parquet.
- CI: contratti caricati al pin immutabile, fixture derivate dalla policy,
  inventari delle superfici rigenerati e dipendenze Rust acquisite dal lock
  prima dei self-test nativi.

### Politica della release e prove ancora distinte

La 9.0.0 non promette compatibilita con le release precedenti e non richiede
che rimangano pubblicate. Le note delle versioni precedenti sono cronaca,
non requisiti per questa release. L'ultima 8.0.0 verra rimossa dal titolare
dopo la pubblicazione della 9.0.0; nessuna rimozione viene eseguita qui.

Le nuove misure di profondita e ASan fornite dall'operatore sono riferite
a `a5a90ef` con `fuzz/Cargo.lock` riordinato dalla toolchain VM. Il lock della
9.0.0 conserva quella forma, senza cambi a pacchetti, versioni o checksum:
le cinque impronte coincidono con il perimetro corrente.
La distribuzione candidate, L2 e i due soak da 3600 s restano da eseguire;
nessun risultato della 8.0.0 viene attribuito alla 9.0.0.
