# Readiness della metadata candidate 1.0.1

La candidate patch `1.0.1` congela il cleanup behavior-preserving e la
correzione del catalogo release FileGDB già qualificati sul commit funzionale
`966005d67b6f2d4fcfe5d62e58fced17881eff06`.

Non modifica API pubbliche, contratti, formati wire, ordering, CRS/WKB,
concorrenza o semantica degli errori. La versione resta patch.

## Evidence base

Il workflow CI `30742404497` è passato sullo SHA funzionale esatto. Include i
gate Linux, Windows, macOS publish, FileGDB/GDAL e manifest/provenance previsti
dal workflow corrente a quella revisione.

La metadata candidate aggiunge un checker versionato `1.0.1`; il checker
`1.0.0` e i suoi record restano storici e immutabili. Il nuovo checker verifica
workspace e lockfile, claim fail-closed, HEAD pulito, revisione attesa e binding
del tag alla versione workspace.

## Blocchi prima del tag

1. CI PR e post-merge same-SHA del commit metadata `1.0.1`;
2. release Data Tools e Database Tools candidate qualificate;
3. catene PostgreSQL/MySQL Database → Data → IO sui nuovi artefatti;
4. comparativa Plenora separata dopo il freeze delle tre librerie.

Il tag `v1.0.1` non è ancora creato e i claim `system_rc` e certificazione
avionica restano falsi.
