# Eccezioni controllate per le dipendenze

Le eccezioni qui registrate riguardano esclusivamente warning di manutenzione,
non vulnerabilità note. `cargo audit --deny warnings` continua a bloccare ogni
nuovo advisory; la CI ignora soltanto gli ID elencati in questo documento.

## RUSTSEC-2023-0089 — `atomic-polyfill`

- Stato: accettato temporaneamente dal 2026-07-27.
- Motivo: il crate è presente nel lockfile attraverso versioni opzionali di
  `rstar`, ma non compare nel grafo attivo di
  `cargo tree --workspace --all-features --target all`.
- Esposizione: nessun codice del componente distribuibile lo collega.
- Chiusura: rimuovere l'eccezione appena l'ecosistema `geo-types`/`rstar` non lo
  include più nella risoluzione del lockfile.
- Trigger di riesame: modifica a feature o dipendenze geospaziali.

## RUSTSEC-2024-0436 — `paste`

- Stato: accettato temporaneamente dal 2026-07-27.
- Motivo: dipendenza build-time di `parquet 59.1.0`, versione Arrow condivisa
  e fissata dal contratto ABI del workspace. `59.1.0` è la versione corrente
  disponibile al momento dell'accettazione.
- Esposizione: macro procedurale eseguita in compilazione; l'advisory segnala
  progetto non mantenuto, non una vulnerabilità nota.
- Chiusura: aggiornare l'intero pin Arrow/Parquet alla prima release compatibile
  che rimuove `paste`, dopo test di interoperabilità con `plenora-data-tools`.
- Trigger di riesame: nuova release Arrow/Parquet o modifica del pin condiviso.

## Advisory risolti nella baseline

Il 2026-07-27 `kml`, `calamine` e `quick-xml` sono stati aggiornati affinché
l'intero grafo attivo usi `quick-xml 0.41.0`. Questo chiude
RUSTSEC-2026-0194 e RUSTSEC-2026-0195, entrambi ad alta severità, senza
introdurre un'eccezione.
