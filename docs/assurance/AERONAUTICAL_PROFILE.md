# Profilo di assurance per impiego aeronautico

## Scopo e limite della dichiarazione

Questo profilo guida Plenora IO verso un livello di evidenza compatibile con
un'integrazione aeronautica safety-critical. Non costituisce certificazione
DO-178C/ED-12C, non assegna un Design Assurance Level e non rende la libreria
idonea da sola a funzioni di bordo. Il DAL, l'ambiente operativo, il compilatore,
l'hardware, le dipendenze native e gli strumenti di verifica devono essere
valutati nel sistema che integra la libreria.

FAA AC 20-115D ed EASA AMC 20-115D riconoscono DO-178C/ED-12C come mezzo di
compliance per il software airborne e richiedono processi di pianificazione,
sviluppo, verifica, configuration management, quality assurance e liaison:

- <https://www.faa.gov/regulations_policies/advisory_circulars/index.cfm/go/document.information/documentID/1032046>
- <https://www.easa.europa.eu/en/document-library/easy-access-rules/online-publications/easy-access-rules-acceptable-means-1?page=22>

Questo repository fornisce soltanto una baseline tecnica e di tracciabilità
riusabile all'interno di tali processi.

## Confine del software distribuibile

Il confine safety analizzato comprende i crate `lib`:

- `plenora-io-model` e `plenora-io-core`;
- `driver-common`;
- tutti i `driver-*`.

`plenora-io-cli`, `plenora-bench`, `plenora-fuzz`, gli esempi e i test non fanno
parte del componente distribuibile. In particolare il benchmark usa `unsafe`
per la misura e non deve essere collegato a un artefatto operativo.

## Regole obbligatorie

1. Input esterni malformati devono produrre un errore tipizzato, mai un panic.
2. Il codice di libreria non può contenere `unsafe` né primitive esplicite di
   panic (`unwrap`, `expect`, `panic!`, `unreachable!`, `todo!`,
   `unimplemented!`).
3. Limiti di byte, righe, colonne, componenti geometrici e profondità devono
   essere applicati prima dell'allocazione o pubblicazione quando tecnicamente
   possibile; i punti ancora non bounded sono gap dichiarati.
4. Conversioni non lossless, CRS non risolti e capability non rappresentabili
   devono fallire chiuse o produrre un `LossReport` esplicito secondo l'ADR.
5. Il publish non deve sovrascrivere output concorrenti e non deve dichiarare
   durabilità non confermata dalla piattaforma.
6. Toolchain e dipendenze sono bloccate dal lockfile; ogni variazione richiede
   change impact analysis.
7. Ogni requisito safety deve avere evidenza nella matrice di tracciabilità.

La CI applica le regole 1-2 ai soli target `lib`, così le asserzioni dei test
restano ammesse come meccanismo di verifica.

Gli advisory di dipendenza sono fail-closed. Eventuali eccezioni devono
riguardare warning non vulnerabili, avere esposizione e criterio di chiusura
documentati in [`DEPENDENCY_EXCEPTIONS.md`](DEPENDENCY_EXCEPTIONS.md), ed essere
ignorate in CI per ID esatto.

I fallback semantici che non generano panic sono censiti separatamente in
[`FALLBACK_REGISTER.md`](FALLBACK_REGISTER.md). La CI ne blocca l'aumento anche
quando Clippy non li considera pericolosi.

## Change impact analysis

Ogni modifica deve indicare:

- requisiti e hazard interessati;
- cambiamenti a contratti, formati, CRS, limiti e failure mode;
- piattaforme, filesystem, dipendenze e toolchain coinvolti;
- test aggiunti o rieseguiti;
- residui non verificati e motivazione.

Il template delle pull request rende queste informazioni parte della baseline
di configurazione. Un commit diretto deve riportare gli stessi elementi nella
documentazione o nel messaggio di change record.

## Gap che impediscono una dichiarazione aeronautica

- nessuna assegnazione DAL né analisi di sistema ARP4754A/ARP4761;
- nessuna verifica MC/DC o object-code verification;
- compilatore Rust, GitHub Actions, Clippy, fuzzing e generatori di coverage non
  qualificati secondo DO-330;
- dipendenze transitive e librerie native con `unsafe` non ancora inventariate
  e approvate singolarmente;
- indicizzazioni, overflow aritmetici e worst-case resource usage non ancora
  dimostrati globalmente; il gate corrente elimina solo le primitive di panic
  esplicite;
- determinismo temporale e WCET non valutati;
- matrice hardware/filesystem incompleta, incluso FileGDB/GDAL nativo Windows;
- copertura linee all'80% utile come regressione, ma non equivalente a structural
  coverage aeronautica.
