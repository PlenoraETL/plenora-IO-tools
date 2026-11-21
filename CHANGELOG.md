# Changelog

## 10.1.0 - in sviluppo (L-IO1, 2026-11-21)

Minor: nessuna rottura dell'API Rust 10.0.0. `Source` resta l'enum con la sola
variante `Path`; `Richiesta.source` resta `Option<PathBuf>` e `sulla_sorgente`
accetta gli stessi tipi di prima.

### Sorgenti senza percorso

- Rust: `plenora_io_core::Sorgente` (`Percorso`, `Byte`, `Archivio`, non
  esaustivo) con `Sorgente::per_lettura`, che passa a qualunque driver il nome
  logico in `Source::Path` e affida i byte alle opzioni di lettura; un solo
  preflight comune li legge per tutti i driver. `Richiesta::con_byte_sorgente`,
  `con_voce_archivio`, `con_limiti_archivio`; `operazioni::elenca_archivio`,
  `LimitiArchivio`, `QuotaArchivio`.
- CLI: `--source-stdin` (i byte arrivano su stdin, entro `--max-input-bytes`;
  il posizionale ne e' il nome logico), `--archive-entry`, `--archive-max-entries`,
  `--archive-max-bytes`, `--archive-max-ratio`, comando locale `archive-list`
  (contratti `plenora-io-archive-list-input-v1` e `plenora-io-archive-list-v1`).
  `--source-base64` della bozza non esiste: i byte non passano mai in argv.
- SDK Python: `Bytes`, `Archivio`, `LimitiArchivio`, `Client.elenca_archivio`
  (modelli `ElencoArchivio`, `VoceArchivio`, `LimiteArchivio`), anche in
  `AsyncClient`; `inspect`, `layers`, `validate`, `read`, `read_table`, `write`
  e `convert` accettano `Bytes` e `Archivio`. Il processo riceve i byte su
  stdin, il modulo nativo in memoria. I tipi sbagliati sono
  `InvalidArgumentError` con chiave stabile (`sorgente.*`).
- `archive-list` non e' un'operazione del catalogo comune e non e' annunciata
  in `capabilities.operations` ne' nel binding runtime: e' scoperta locale,
  come `version`, con contratto nostro.

### Una regola per nomi, errori e limiti

- Nomi: componenti portabili (anche `CONIN$`, `CONOUT$`, `COM¹-³`, `LPT¹-³`,
  spazi prima dell'estensione), chiave per componente, prefissi confrontati per
  componente e mai per lunghezza in byte; ogni file della vista nasce con
  `create_new`, quindi un alias del filesystem e' un errore e mai una
  sovrascrittura. Nomi UTF-8 (bit 11), Info-ZIP 0x7075 con CRC, altrimenti
  CP437 come APPNOTE.
- Errori dalla tabella unica (`error_table`): archivio non valido
  `FORMAT_ERROR`, voce assente `SOURCE_NOT_FOUND`, nome scelto dal chiamante
  non portabile `CLI_USAGE`, archivio annidato e metodi diversi da stored e
  deflate `UNSUPPORTED`, nomi confusi dal filesystem `FORMAT_ERROR`, limite
  `LIMIT_EXCEEDED` con `details.quota` del limite vincolante.
- Limiti: `LimitiArchivio` si costruisce solo validato (zero e' `CLI_USAGE`);
  ogni limite e' stretto dalla quota della pipeline corrispondente
  (`max_input_entries`, `max_input_bytes`, `decompression_ratio`) e la
  riduzione e' dichiarata (`archive-list`.`limits[].bound_by`, `details.quota`).
  Il rapporto vale per voce e sull'archivio intero, anche per le voci non
  selezionate; le voci devono essere disgiunte.

### Driver

- Le voci d'archivio si leggono da una vista nella directory di spill
  (`PLENORA_SPILL_DIR`) con quota di spill, mai materializzate in RAM; i byte
  in memoria restano in memoria. Una vista resta dopo un `SIGKILL` come
  directory `plenora-source-*` (limite dichiarato).
- KMZ: il primo `.kml` alla radice in ordine di directory centrale; gli altri
  `.kml` sono la perdita dichiarata `kml.kmz_document_not_read`. `.kmz` e'
  riconosciuto dal suffisso; un KMZ dentro uno ZIP e' un archivio annidato.
- Shapefile in uno ZIP: companion nella cartella del principale, per chiave;
  `x.shp.d/` come sottoalbero.
- `write` misura il content type sui byte della sorgente (file, stdin o voce),
  non sul posizionale.
- Nuovo target `archive_reader`, semi e perimetro registrati. Campagna e
  misure di copertura/profondita da eseguire al congelamento 10.1.0;
  le evidenze 10.0.0 non qualificano queste modifiche.

### Limiti dichiarati

- Metodi ZIP: solo stored (0) e deflate (8); cifratura rifiutata.
- Un nome scritto in CP850 senza bit 11 si legge con i glifi CP437: il
  codepage OEM non e' scritto nell'archivio.
- Dimensioni di `archive-list` dichiarate, non verificate (i campi si chiamano
  `declared_*`); CRC e lunghezza reale si verificano alla lettura.
- Dati anteposti a uno ZIP senza offset corretti (autoestraenti) sono rifiutati.

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
