# CI locale: contratto comune della famiglia, schema 2

Implementazione di riferimento: `scripts/ci_locale.py` in IO. Database deve
adottare questo contratto prima di dichiararsi compatibile; questa modifica
non modifica il repository database. Il confronto dei due strumenti ha
conservato l'espansione IO di matrici, composite, workflow riusabili ed
espressioni, e adottato la distinzione database tra processo non avviabile
e processo avviato che fallisce. Non si duplicano i comandi dei workflow.

## Avvio e opzioni

Da root, con Python disponibile:

```sh
python -m pip install -r requirements-ci-locale.txt
python scripts/ci_locale.py --piano --profilo pr
python scripts/ci_locale.py --esegui --profilo pr
python scripts/ci_locale.py --esegui --profilo pr --offline
```

`requirements-ci-locale.txt` fissa `PyYAML==6.0.3`. Se il modulo manca,
l'avvio termina con codice 2 e istruzione di installazione, senza traceback.
Il piano può essere letto con una versione Python diversa da quella richiesta
dai job; la versione richiesta è una precondizione dell'esecuzione.

Opzioni identiche per ogni libreria:

| Opzione | Contratto |
| --- | --- |
| `--piano` / `--esegui` | Mutuamente esclusive, una obbligatoria |
| `--profilo pr` | Default e unico profilo attuale; sovrainsieme dei workflow pull_request |
| `--job KEY` | Ripetibile; chiave o ID completo della cella; sconosciuto: errore 2 anche nel piano |
| `--offline` | Esplicito; assente significa online |
| `--servizi-live` | Opt-in ai comandi live; non provisiona services/container Actions |
| `--timeout N` | Secondi per passo, default 1800 |

Il piano conserva tutti i passi anche se non selezionati, con disponibilità
`ESEGUIBILE` / `NON ESEGUIBILE` e motivo. Non esegue i gate e non scrive
ricevute. Può interrogare Git e rustup per verificare precondizioni locali.

## Rete e precondizioni

Online imposta `PIP_NO_INDEX=0`, `CARGO_NET_OFFLINE=false`; offline imposta
`PIP_NO_INDEX=1`, `CARGO_NET_OFFLINE=true`. Entrambe impongono
`CARGO_BUILD_JOBS=4`, `CARGO_INCREMENTAL=0` e disabilitano il controllo
versione pip. Questi valori prevalgono su host e workflow.

Offline i download riconosciuti e pip senza `--no-index --find-links` a una
wheelhouse locale esistente non partono: `NON ESEGUITO`, motivo con `offline`.
Una wheelhouse esplicita è responsabilità dell'ambiente (deve contenere tutte
le dipendenze); un errore del comando eseguito resta FAIL.
I comandi Cargo che risolvono dipendenze sono conservativamente NON ESEGUITI:
la mera presenza della cache non ne certifica la completezza per tutte le
feature, target e workspace. Anche una cache completa può quindi produrre
NON ESEGUITO; non si promette esecuzione massima offline. `cargo fmt` non è
escluso per questa ragione. Non si riclassificano errori Cargo leggendo i log.
Il riconoscimento include il contenuto degli script locali direttamente
richiamati; non è un sandbox di rete per programmi arbitrari o import transitivi.

La tabella `PRECONDITIONS` è l'unico punto dichiarativo per adattamenti locali,
oltre ai requisiti già dichiarati nei workflow:

| Precondizione | Verifica prima del passo |
| --- | --- |
| Piattaforma / shell | OS dichiarato, shell mappata e presente nel PATH |
| Python / Rust | Versione Python; toolchain e componenti rustup installati |
| Storia completa | Checkout fetch-depth 0: clone non shallow |
| Oracolo storico IO | BASELINE_REVISION letta dal comparatore, commit/tag risolvibile, file --old e --new presenti |
| Servizi / container | Opt-in esplicito; provisioning Actions non mappato |
| GDAL, Docker, pwsh, cargo, git | Eseguibile richiesto presente nel PATH |
| Provisioning sistema | sudo/apt-get/brew/choco riservati alla CI, non eseguiti localmente |

Le altre librerie estendono gli adattamenti dichiarativi per i propri requisiti,
senza cambiare opzioni o schema. Una precondizione assente produce sempre
NON ESEGUITO con motivo, mai FAIL. La revisione storica è quella del sorgente,
non un tag arbitrario inventato dal runner. I checkout aggiuntivi restano
non mappati: non vengono creati automaticamente.

## Esiti e ricevuta

Esiti dei passi: `PASS` (processo eseguito, codice zero), `FAIL` (processo
eseguito con codice non zero o timeout), `NON ESEGUITO` (precondizione,
selezione, mapping o contesto mancante), `SALTATO` (riservato per compatibilità;
non usato per piattaforme o strumenti mancanti). Condizioni false sono
NON ESEGUITO con `condition_false=true` e non bloccano i passi successivi.
Un errore di avvio è NON ESEGUITO. `continue-on-error` non nasconde un FAIL.
Un log che dichiara prove saltate impedisce PASS anche con codice zero.

Ricevuta UTF-8 JSON in `target/ci-locale/<timestamp-UTC>-<id>/ricevuta.json`,
aggiornata dopo ogni passo. Campi comuni obbligatori:

| Livello | Campi |
| --- | --- |
| Root | schema (2), network_mode (online/offline), services_opt_in, sha, dirty, started_at, host, python, selected_jobs, profile, sources, overrides, jobs, steps, counts |
| Root a fine esecuzione | finished_at, final_sha, final_dirty, exit_code |
| jobs[] | id, matrix, needs, reason, status (a job concluso) |
| steps[] | job, source, raw, name, run, uses, with, shell, working_directory, env, if, continue_on_error, toolchain, status, reason, services, container, duration_seconds, exit_code, command, environment, log, condition_false |

`sources` associa percorsi dei workflow/action ai loro SHA-256; `overrides`
registra la policy effettiva. `exit_code` e `log` del passo sono null se non
eseguito; `command` diventa argv per i passi avviati. Sono ammessi campi
aggiuntivi (children, mapping, parent_conditions, resolved_run).
`counts` conta gli esiti presenti; chiavi assenti valgono zero.
Exit del runner: 1 se almeno un FAIL; altrimenti 2 per copertura incompleta
o piano vuoto; 0 solo con tutti i job PASS. Il piano valido esce con 0.

## Adozione database e verifiche

Database deve sostituire POLICY_ENV sempre offline con la policy parametrica,
adottare il file requisiti e il bootstrap, le precondizioni e i nomi della
ricevuta sopra (`steps/status/exit_code`, non `results/outcome/returncode`).
Non dichiarare compatibili ricevute schema 1 dei due repository.

```sh
python -m unittest scripts.test_ci_locale
```

I test verificano dipendenza assente, rete, precondizioni, errori di avvio,
timeout e controprova di un processo realmente rosso anche offline.
Il confronto indipendente piano/workflow e le sue mutazioni restano nei test
scoperti dal gate CI `check_prove_fonte_unica.py --run`.
