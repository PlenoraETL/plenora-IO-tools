## Change impact analysis

- Requisiti `PLN-ASR-*` e hazard interessati:
- Contratti, formati, CRS, limiti o failure mode modificati:
- Piattaforme, filesystem, dipendenze o toolchain coinvolti:
- Compatibilità e migrazione:

## Evidenza di verifica

- Test aggiunti:
- Suite rieseguite:
- Risultati coverage/fuzz/crash test:
- Residui non verificati e motivazione:

## Checklist

- [ ] Nessun dato o metadato viene perso silenziosamente.
- [ ] Gli input non validi producono errori tipizzati, non panic.
- [ ] Limiti e consumo risorse sono stati rivalutati.
- [ ] ADR, stato implementativo e matrice assurance sono aggiornati.
- [ ] `Cargo.lock` e dipendenze sono cambiati solo se necessario.
