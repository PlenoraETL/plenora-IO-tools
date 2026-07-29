# Change impact analysis — pacchetto di revisione indipendente

Data: 2026-07-29

## Scopo

Viene preparato un pacchetto riproducibile per il revisore indipendente della
baseline tecnica congelata `78c2d15`.

La modifica aggiunge soltanto documentazione, un record machine-readable in
stato `pending_eligible_reviewer` e controlli fail-closed. Non modifica codice
distribuibile, baseline congelata, contratto, dipendenze o tag.

## Decisioni

- nessun revisore è inventato o precompilato;
- nome, identità, attestazione, data, comandi, rilievi ed esito restano `null`;
- il record identifica separatamente base di confronto, candidato, freeze,
  evidenza e ICD;
- il gate accetta soltanto lo stato pendente finché non viene registrata una
  persona eleggibile;
- una falsa auto-promozione del record viene rifiutata dai test negativi.

## Hazard

- H-07: tutti gli artefatti esaminati sono identificati da SHA completi.
- H-08: il pacchetto impone perimetro, comandi e formato dei rilievi senza
  trasformarli in risultati pretesi.
- H-09: lo stato pendente non può essere confuso con una review completata.

## Verifica

- confronto senza differenze del codice distribuibile fra `78c2d15` e il
  commit di freeze `824cbc9`;
- gate release e test negativi del record;
- gate pin, dipendenze, identità e fallback;
- CI completa sul commit del pacchetto.

## Residui

La revisione deve ancora essere svolta da una persona eleggibile. Il pacchetto
non autorizza release, tag, claim indipendente o certificazione avionica.
