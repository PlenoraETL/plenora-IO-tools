# Change impact analysis — versione e tag della RC

Data: 2026-07-29

## Decisione

La release candidate del componente IO-tools usa la versione SemVer
`0.1.0-rc.1` e il tag Git `v0.1.0-rc.1`.

La versione è definita una sola volta in `[workspace.package]`; tutti i 16
crate membri ereditano `version.workspace = true`. `Cargo.lock` registra la
stessa versione per ogni package del workspace. Anche `fuzz/Cargo.lock` e
`conformance/three-component-chain/Cargo.lock` registrano `0.1.0-rc.1` per i
path dependency IO. Il gate di release controlla tutte le rappresentazioni e
rifiuta ogni divergenza.

I package `fuzz/` e `conformance/three-component-chain/` non appartengono al
workspace distribuibile: sono harness di verifica con `publish = false` e
restano intenzionalmente a `0.0.0`.

## Relazione con la baseline congelata

La revisione candidata dell'implementazione resta
`78c2d150b9c7d0ac48e4c97b03f86228e0f0a068`. Il cambio di versione modifica
soltanto i metadati Cargo e il lockfile; non modifica sorgenti, wire contract,
capability, dipendenze risolte o comportamento.

Il tag identifica la revisione di confezionamento della RC, mentre il manifesto
e il messaggio annotato preservano separatamente la revisione candidata. Non
viene riutilizzata l'identità della baseline per codice diverso.

## Forma e contenuto del tag

Nell'ambiente corrente non è disponibile una chiave di firma. Il tag è quindi
annotato e non firmato, registrato come `tag_form=annotated_unsigned`.

Il messaggio annotato deve contenere almeno:

- nome e versione della RC;
- revisione candidata `78c2d15`;
- claim `verified_internally`;
- stato della review indipendente `not_performed`;
- esclusione di RC di sistema e certificazione avionica;
- revisione ICD adottata.

## Hazard

- H-07: tag, package e lockfile non possono dichiarare versioni divergenti.
- H-08: il messaggio del tag espone il livello di verifica senza promozioni
  implicite.
- H-09: candidato di implementazione e revisione di confezionamento restano
  distinguibili e citabili.

## Verifica e sequenza

1. gate di coerenza versione/tag e relativi test negativi;
2. `cargo check`, Clippy, test e build `--locked` sull'intero workspace;
3. CI Linux, Windows, macOS e coverage sul commit di preparazione;
4. creazione del tag annotato `v0.1.0-rc.1` sul commit verificato;
5. push del tag senza riscritture.

La verifica pre-tag è stata completata sulla revisione
`83798768a9bd86c6ebab85331afda1cb2e049229`: CI `30438104745` verde sui job
`rust`, `coverage`, `windows` e `macos-publish`.

## Claim esclusi

Il tag non dichiara `verified_independently`, RC del sistema Plenora,
conformità DO-178C/DO-330 o certificazione avionica.
