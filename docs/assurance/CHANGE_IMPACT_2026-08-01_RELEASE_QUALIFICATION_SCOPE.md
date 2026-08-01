# Change impact analysis — ambito della qualifica release

Data: 2026-08-01

## Problema

`scripts/check_release_contract.py` convalidava correttamente i record immutabili
di `v1.0.0-rc.2`, ma il messaggio generico poteva essere interpretato come una
qualifica del checkout corrente. Inserire nel repository lo SHA del commit che
contiene il controllo sarebbe inoltre un vincolo autoreferenziale impossibile.

## Decisione

Il comportamento senza opzioni resta compatibile, ma è ora esplicitamente
storico; `--historical` rende la scelta visibile in CI e l'output dichiara che
HEAD corrente non è né ispezionato né qualificato.

La modalità `--qualify-current` aggiunge un gate distinto. Richiede
`--expected-revision` da una fonte esterna, risolve sia quel ref sia `HEAD` a SHA
di commit completi, ne richiede l'uguaglianza e accetta soltanto uno stato Git
vuoto per file tracciati e non tracciati. Ref assente o non risolvibile, errore
Git, mismatch e worktree sporco falliscono tutti in modo chiuso.

Il workflow dedicato `release-qualification.yml` passa `github.sha` come valore
esterno. La modalità corrente non è aggiunta ai normali test di pull request:
viene eseguita nel percorso manuale/tag di release, dopo un checkout pulito e
prima di poter usare quel run come evidenza del commit selezionato.

## Verifica e limite operativo

I test iniettano risultati Git per i casi clean, mismatch, modifica tracciata e
file non tracciato. Nel worktree di sviluppo la modalità corrente deve fallire
finché le modifiche non sono state committate dal maintainer: soltanto il commit
pulito risultante può essere qualificato con il relativo SHA fornito dalla CI.
