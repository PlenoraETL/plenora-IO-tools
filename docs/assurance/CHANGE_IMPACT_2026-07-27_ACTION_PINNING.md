# Change impact analysis — pinning della supply chain CI

Data: 2026-07-27

## Identificazione e motivazione

Questa modifica sostituisce tutti i riferimenti mobili `uses:` del workflow CI
con commit SHA completi e introduce `scripts/check_action_pins.py`, eseguito
dalla CI stessa, per impedire la reintroduzione di tag o branch mobili.

La modifica chiude il residuo esplicito di PLN-ASR-010 relativo alle GitHub
Actions. Non modifica codice distribuibile, contratti dati, formati, CRS,
limiti runtime o failure mode della libreria.

## Riferimenti fissati

Gli SHA sono stati risolti il 2026-07-27 dai riferimenti upstream già in uso:

| Action | Riferimento precedente | Commit fissato |
|---|---|---|
| `actions/checkout` | `v4` | `11d5960a326750d5838078e36cf38b85af677262` |
| `actions-rust-lang/setup-rust-toolchain` | `v1` | `166cdcfd11aee3cb47222f9ddb555ce30ddb9659` |
| `Swatinem/rust-cache` | `v2` | `42dc69e1aa15d09112580998cf2ef0119e2e91ae` |
| `taiki-e/install-action` | `cargo-audit` | `f1b61bf3a1373bccc1ebdc11506122709bd40dee` |
| `taiki-e/install-action` | `cargo-llvm-cov` | `8dc1a448f03edf5b0d5a9bb37d054545b2fe246e` |
| `actions/upload-artifact` | `v4` | `ea165f8d65b6e75b540449e92b4886f43607fa02` |

I commenti accanto agli SHA mantengono leggibile la linea di release senza
partecipare alla risoluzione del codice eseguito.

## Requisiti, hazard e verifica

- requisito: PLN-ASR-010;
- hazard: H-07, artefatto non riproducibile o dipendenza non controllata;
- piattaforme: orchestrazione CI Linux, Windows e macOS;
- verifica locale: il nuovo gate analizza tutti i file `.yml` e `.yaml` in
  `.github/workflows`, accetta action locali, richiede SHA Git completi per le
  action remote e digest `sha256` per eventuali action Docker; otto regressioni
  verificano riferimenti validi, tag e branch mobili, SHA corti, revisioni
  mancanti e immagini Docker mobili;
- verifica remota: esecuzione del workflow associato al commit.

Risultati locali:

- 8 test unitari superati;
- 1 workflow e 14 riferimenti `uses:` verificati dal gate;
- validazione `actionlint` superata senza rilievi.

## Residui

- `ubuntu-latest`, `windows-latest` e `macos-latest` identificano immagini
  runner mobili;
- `apt-get install libgdal-dev` non fissa una versione di pacchetto;
- le action e gli strumenti della CI non sono qualificati secondo DO-330;
- l'aggiornamento futuro di uno SHA richiede una nuova change impact analysis
  e la verifica del contenuto upstream selezionato.

Per questi residui PLN-ASR-010 resta `Parziale`; il pinning non costituisce una
dichiarazione di certificazione o di conformità DO-178C.
