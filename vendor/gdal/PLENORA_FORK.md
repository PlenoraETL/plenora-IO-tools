# Fork governato `gdal` 0.17.1

Provenienza:

- crate: `gdal` 0.17.1 da crates.io;
- checksum crate:
  `82ab834e8be6b54fee3d0141fce5e776ad405add1f9d0da054281926e0d35a9f`;
- revisione upstream registrata nel pacchetto:
  `a324724c60dbf1e9bf0fb05203c4e0a1eefbf312`;
- licenza upstream: MIT.

Il percorso vendorizzato conserva manifest, `build.rs`, `src/`, licenza,
readme e metadato VCS richiesti per compilazione e attribuzione. Fixture,
workflow, script, esempi e test upstream non collegati dalla dipendenza sono
esclusi per ridurre il perimetro governato; l'intero sottoinsieme è fissato dal
tree hash in `scripts/gdal-fork-lock.json`.

Delta Plenora:

- `LayerAccess::set_ignored_fields`, wrapper safe e fallibile di
  `OGR_L_SetIgnoredFields`;
- i lint `unexpected_cfgs` e `mismatched_lifetime_syntaxes` restano soppressi
  come avviene per la crate registry tramite `--cap-lints`; il passaggio a
  dipendenza path non deve trasformare warning upstream estranei in warning
  della workspace;
- nessun'altra modifica alla sorgente upstream.

Il wrapper conserva le `CString` e il vettore di puntatori per l'intera
chiamata FFI, aggiunge il terminatore nullo richiesto e converte ogni ritorno
OGR diverso da `OGRERR_NONE` in `GdalError::OgrError`.

La dipendenza resta semanticamente e numericamente fissata a 0.17.1. A ogni
aggiornamento upstream il fork deve essere ricreato dalla nuova crate
verificata, il delta riapplicato e le matrici Linux/Windows GDAL più il
benchmark narrow devono essere rieseguiti. Il fork non autorizza nuove API FFI
senza una voce separata in questo documento.
