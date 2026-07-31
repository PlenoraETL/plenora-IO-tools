# Change impact analysis — validazione post-tag 1.0.0-rc.2

Data: 2026-07-31

## Esito

Il tag annotato non firmato `v1.0.0-rc.2` è stato creato e pubblicato:

- oggetto tag: `d69a10d405a71af93809ba038971279e107a2fc4`;
- target immutabile: `9804d775d0d46df9137d44cf0c6963d66a563753`;
- CI del record finale: `30633367104`, verde;
- CI innescata dal tag: `30633636716`, verde sui job `rust`, `coverage`,
  `windows` e `macos-publish`.

Il tag conserva il claim `verified_internally` del solo componente. Non
dichiara revisione indipendente, RC di sistema o certificazione avionica.

## Qualifica esterna

La matrice `83/84` roundtrip e `27/28` di catena resta evidenza storica della
sola `v1.0.0-rc.1` e non viene trasferita. La qualifica della RC.2 è
`not_run`: `plenora-contracts/conformance` deve aggiornare il pin al target
immutabile sopra e rieseguire la matrice. Fino ad allora il system gate resta
`not_satisfied`.

## Impatto sul codice

Nessuno. Questo record è successivo al tag e modifica soltanto documenti e
manifesti di assurance su `main`; il tag e il suo target restano immutati.
