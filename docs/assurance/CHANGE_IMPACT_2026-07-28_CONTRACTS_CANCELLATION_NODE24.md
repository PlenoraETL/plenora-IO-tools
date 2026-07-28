# Change impact analysis — metadati, errori, cancellazione e Node 24

Data: 2026-07-28

## Baseline e autorità

Baseline IO-tools: commit
`1c1ee61e6d87ce2810ec482298b483e90a3abe80`.

Obiettivo ICD candidato: `plenora-contracts@v2.0-rc3`, revisione
`ef2640348426425585ad228312468e7cf1d0e50f`. Il tag è annotato ma non firmato;
il registro dichiara §2, §3.4, le estensioni §4.3.1–§4.3.3, §9 e §11 come
`proposta`.

La modifica adotta localmente i contratti trasversali richiesti per
`contract.version`, `types_declaration`, CRS, errore a quattro assi e
cancellazione. Il documento trasversale resta distinto dalla relativa
ratifica: questa implementazione è autorizzata per IO-tools, ma non dichiara
da sola la ratifica degli altri componenti Plenora né conformità completa
all'ICD. L'emissione delle chiavi candidate prima della ratifica è registrata
come deroga locale nel manifest di provenienza della RC; la condizione di
rientro è ratifica compatibile o migrazione alla forma sostitutiva.

## Modifiche al confine dati

- gli schemi prodotti emettono `plenora.contract.version=1`;
- ogni campo geometrico emette `crs_id`, `crs_resolution`, `crs_definition`,
  `crs_definition_format`, `axis_order` e `types_declaration` nel namespace
  `plenora.geometry`;
- `types_declaration` distingue `exact`, `mixed` e `unresolved`;
- il consumer accetta lo schema legacy senza versione, ma rifiuta versioni
  future e combinazioni incoerenti;
- una definizione CRS non può essere consumata senza formato esplicito;
- `ResolvedCrs` conserva anche il formato della definizione.

L'impatto wire è additivo per i consumer legacy. I nuovi consumer sono
fail-closed su una versione futura. Il valore mancante non viene trasformato
in un CRS o in un tipo inventato.

## Modello d'errore

`PlenoraIoError` passa da enum piatta a record serializzabile con quattro assi
indipendenti:

1. categoria;
2. fase;
3. effetto remoto;
4. disposizione di retry.

Il codice locale dettagliato (`IoErrorCode`) resta separato dagli assi e
mantiene la discriminazione già usata dai driver. I costruttori redigono path,
contenuti CRS e payload esterni. `during` e `with_effect` permettono al bordo
driver di rendere esplicito il contesto senza riscrivere la causa.

L'API pubblica cambia in modo incompatibile per il pattern matching sulle
vecchie varianti. I crate hanno versione `0.0.0`, `publish = false`, e tutti i
call site del workspace, incluso FileGDB feature-on, sono migrati e compilati.

## Cancellazione R11

`ReadOptions`, `WriteOptions` e `ReadRequest` portano un
`CancellationToken` clonabile con richiesta esplicita, deadline e token figli.
I controlli avvengono:

- prima e durante il probe dell'albero filesystem;
- a ogni confine di batch in lettura;
- prima di ogni write;
- prima di `finish`, quindi prima del publish.

Quando un reader osserva la cancellazione rilascia il reader sottostante e il
lease di concorrenza; il report di perdita già osservato viene conservato. Una
cancellazione prima di `finish` non invoca il publish.

Residuo: le chiamate sincrone interne ai parser materializzanti KML, DXF e XLSX
non sono preemptive. La richiesta viene osservata al successivo confine
controllato; per interrompere durante una singola chiamata della dipendenza
servirà isolamento del parser o un'API cancellabile della dipendenza.

## Rinnovo GitHub Actions

Tutti i riferimenti restano pin SHA immutabili.

| Action | Prima | Dopo | Runtime |
|---|---|---|---|
| `actions/checkout` | `11d5960a326750d5838078e36cf38b85af677262` (`v4`) | `3d3c42e5aac5ba805825da76410c181273ba90b1` (`v7.0.1`) | Node 24 |
| `Swatinem/rust-cache` | `42dc69e1aa15d09112580998cf2ef0119e2e91ae` (`v2`) | `c19371144df3bb44fab255c43d04cbc2ab54d1c4` (`v2.9.1`) | Node 24 |
| `actions/upload-artifact` | `ea165f8d65b6e75b540449e92b4886f43607fa02` (`v4`) | `043fb46d1a93c77aae656e7c1c64a875d1fc6a0a` (`v7.0.1`) | Node 24 |
| `actions-rust-lang/setup-rust-toolchain` | stesso SHA, commento `v1` | stesso SHA, identificato `v1.17.0` | composite; incorpora `rust-cache` sopra |
| `taiki-e/install-action` | due SHA distinti | `41049aa56687c35e0afa74eed4f09cec4f9afabf` (`v2.85.2`) | composite |

Fonti ufficiali ispezionate:

- <https://github.com/actions/checkout/releases/tag/v7.0.1>
- <https://raw.githubusercontent.com/actions/checkout/3d3c42e5aac5ba805825da76410c181273ba90b1/action.yml>
- <https://github.com/actions/upload-artifact/releases/tag/v7.0.1>
- <https://raw.githubusercontent.com/actions/upload-artifact/043fb46d1a93c77aae656e7c1c64a875d1fc6a0a/action.yml>
- <https://github.com/Swatinem/rust-cache/releases/tag/v2.9.1>
- <https://raw.githubusercontent.com/Swatinem/rust-cache/c19371144df3bb44fab255c43d04cbc2ab54d1c4/action.yml>
- <https://github.com/actions-rust-lang/setup-rust-toolchain/releases/tag/v1.17.0>

Il workflow usa soltanto runner GitHub-hosted `*-latest`; non introduce un
requisito verso runner self-hosted. Permessi, eventi, input delle action,
retention e nomi degli artefatti restano invariati. Il rischio residuo è la
mobilità dell'immagine `*-latest`, già registrata in PLN-ASR-010.

## Prestazioni

La baseline `1c1ee61` e il post sono stati compilati ed eseguiti nello stesso
container Linux, con lo stesso harness, release Rust 1.92.0, 100.000 righe,
cinque ripetizioni e geometria Point. Il veto è 5% su throughput e picco RSS.

| Driver/operazione | Throughput | Picco RSS | Esito |
|---|---:|---:|---|
| DXF read | +1,31% | +0,13% | OK |
| DXF write | +3,51% | +0,08% | OK |
| KML read | +2,50% | +0,03% | OK |
| KML write | +64,32% | -85,63% | OK |
| XLSX read | -0,62% | +0,29% | OK |
| XLSX write | -0,79% | +0,00% | OK |

Nessun percorso supera il budget di regressione. La baseline è
`baseline/streaming-before.json`; il risultato post è
`target/paired-after.json` e resta un artefatto locale non versionato.

## Verifica

- test core/model: 68 test superati;
- `cargo check --workspace --all-targets --all-features --locked`: superato;
- Clippy workspace completo `--all-features -D warnings`: superato;
- safety Clippy sui target `lib`: superato;
- gate pin action, identità pubbliche, dipendenze e fallback: superati;
- confronto prestazionale: superato;
- suite workspace completa: superata;
- FileGDB/GDAL feature-on: 21 test superati, 2 helper ignorati perché eseguiti
  dai test di sottoprocesso.

## Hazard e residui

- H-01/H-06: metadati incompleti o incoerenti falliscono senza valori inventati;
- H-02: la cancellazione pre-publish non rende visibile la destinazione;
- H-03: deadline e cancellazione limitano il lavoro ai confini controllati;
- H-07: action e dipendenze restano identificate da SHA/pin esatti;
- H-08/H-09: regressioni, benchmark e CIA sono espliciti.

Restano aperti la cancellazione preemptive dentro parser sincroni, una revisione
indipendente, MC/DC e la qualifica degli strumenti. Questa evidenza non
costituisce certificazione né dichiarazione DO-178C/ED-12C.
