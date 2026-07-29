# Change impact analysis — autorizzazione RC verificata internamente

Data: 2026-07-29

## Decisione

La baseline tecnica congelata
`78c2d150b9c7d0ac48e4c97b03f86228e0f0a068` è autorizzata come release
candidate del solo componente IO-tools con claim `verified_internally`.

Lo stato del freeze, il livello di verifica e la revisione indipendente sono
dimensioni separate:

- `freeze_status=frozen` identifica la baseline immutabile;
- `verification_claim=verified_internally` dichiara il livello di evidenza;
- `independent_review=false` registra un attributo aperto e non blocca la RC;
- `independently_verified_claim_authorized=false` impedisce una promozione
  semantica non sostenuta dall'evidenza.

La revisione indipendente resta utile e il relativo pacchetto rimane aperto. È
necessaria prima di dichiarare `verified_independently`, non per congelare o
pubblicare una RC che espone esplicitamente il livello
`verified_internally`.

## Base normativa

`plenora-contracts@v2.0-rc8`, revisione
`62b12e3496466d2c908dac3cc098640b99b52e21`, limita in R0.4 lo stato massimo
dichiarabile in assenza di revisione indipendente a `verificato internamente`.
Non impone che l'assenza della review impedisca il freeze o una RC di
componente con quel claim.

La precedente associazione fra review indipendente e autorizzazione del tag
era quindi una restrizione del processo locale, non una conseguenza necessaria
di R0.4. Questa decisione rimuove tale restrizione senza cambiare o indebolire
il significato dei claim.

## Impatto

La modifica interessa esclusivamente gate, manifesti ed evidenze di release.
Non modifica:

- codice distribuibile o revisione candidata;
- wire contract, formati o capability;
- dipendenze e toolchain;
- stato della RC di sistema;
- dichiarazioni di conformità o certificazione avionica.

## Hazard

- H-07: la baseline resta identificata dallo SHA congelato e non viene
  sostituita in-place.
- H-08: l'assenza della review resta machine-readable; il gate impedisce il
  claim `verified_independently`.
- H-09: la decisione locale è esplicita, versionata e coperta da test positivi
  e negativi.

## Verifica richiesta

- il gate accetta `release_authorized=true` con
  `verification_claim=verified_internally` e `independent_review=false`;
- il gate rifiuta `verification_claim=verified_independently` finché la review
  è aperta;
- il gate continua a rifiutare una RC di sistema o una dichiarazione avionica;
- CI completa sul commit che registra la decisione prima della creazione del
  tag.

## Evidenza di verifica

La decisione è registrata nella revisione
`75ea508cec257dc46252ec267e5b1e9ecaa78b73`. La CI `30435854122` è terminata
con esito `success` sui job `rust`, `coverage`, `windows` e `macos-publish`.
Il tag può quindi essere creato senza attendere la revisione indipendente,
mantenendo il claim `verified_internally`.

## Residui dichiarati

- revisione indipendente non eseguita;
- claim `verified_independently` non autorizzato;
- tag RC non ancora creato;
- RC di sistema e certificazione avionica non dichiarate;
- residui tecnici già elencati nel bundle di evidenza invariati.
