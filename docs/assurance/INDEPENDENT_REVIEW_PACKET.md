# Pacchetto di revisione indipendente

Stato: **in attesa di un revisore eleggibile**.

## Artefatti congelati

- componente: `plenora-IO-tools`;
- base di confronto:
  `8d3f25f109f6ea8910da71e098db6924438e481c`;
- codice candidato:
  `796a1f94e0735e4f5b9e8bfca1056c295bda4814`;
- record del freeze tecnico e record di provenienza:
  `cea2535c2ddcbae4ba7ec49e72b65a9524b8711b`, CI `30619205139`;
- ICD: `plenora-contracts@v2.0-rc13`, revisione
  `8f684a4edd9bbeaadd4f5f0375cc0b86aefe6417`.

I commit successivi a `796a1f9` possono modificare soltanto versione,
assurance, provenienza e gate. La verifica applicata allo SHA pre-tag è
`git diff --exit-code 796a1f9 cea2535 -- crates rust-toolchain.toml`.

## Eleggibilità

Il revisore deve essere una persona diversa da ogni autore o coautore delle
modifiche esaminate. L'owner è eleggibile soltanto se non è autore o coautore.
Automazione, assistenti che hanno scritto il candidato e self-review
dell'autore non soddisfano il gate.

Prima di iniziare, il revisore deve registrare nel file
[`release/independent-review.json`](../../release/independent-review.json):

- nome e affiliazione;
- riferimento d'identità o contatto verificabile;
- attestazione esplicita di eleggibilità;
- data della review.

## Perimetro minimo

La review deve esaminare almeno:

1. diff completo `ea0de79..dc85f51`, con priorità ai confini esterni;
2. publish atomico/no-clobber/durability e failure mode;
3. WKB/EWKB, dimensioni, SRID, geometry/geography e metadati CRS;
4. cancellazione, limiti, backpressure e parser materializzanti residui;
5. capability, fidelity/loss report e modello d'errore a quattro assi;
6. ottimizzazioni GeoJSON, CSV e GeoParquet e relativi invarianti;
7. corpus differenziale e gap dichiarati; la catena a tre componenti è
   esplicitamente fuori dal repository e dal perimetro della review IO;
8. coerenza fra codice, ADR, ICD, matrice assurance e manifesti release.

Le CIA in `docs/assurance/CHANGE_IMPACT_*.md` sono indici del lavoro, non
sostituiscono l'ispezione del diff.

## Comandi riproducibili

```text
git diff --stat ea0de79 dc85f51
git diff ea0de79 dc85f51 -- crates fuzz scripts
git diff --exit-code dc85f51 f5dc5d4 -- crates Cargo.toml Cargo.lock rust-toolchain.toml
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo clippy --workspace --lib --all-features --locked -- -D warnings -D unsafe-code -D clippy::unwrap_used -D clippy::expect_used -D clippy::panic -D clippy::unreachable -D clippy::todo -D clippy::unimplemented
cargo test --workspace --all-targets --all-features --locked
cargo build --workspace --release --all-features --locked
python scripts/check_release_contract.py
```

Il revisore deve registrare i comandi realmente eseguiti; non deve copiare la
lista come se fosse un risultato.

## Formato dei rilievi

`findings` deve diventare un array, anche quando è vuoto. Ogni rilievo usa:

```json
{
  "id": "IR-001",
  "severity": "blocking | major | minor | observation",
  "requirement_or_hazard": "R... / H-...",
  "location": "percorso:riga o artefatto",
  "description": "rilievo verificabile",
  "disposition": "open | accepted | corrected | rejected_with_rationale",
  "evidence": "comando, test o riferimento"
}
```

L'esito ammesso è `pass`, `pass_with_non_blocking_findings` oppure
`blocking_findings`. Un array vuoto è coerente soltanto con `pass`; un rilievo
bloccante aperto richiede `blocking_findings`.

## Chiusura

Quando il record è completo:

1. il gate verifica identità del candidato, campi obbligatori, coerenza dei
   rilievi ed esito;
2. `independent_review` può diventare `true` soltanto con esito non bloccante;
3. una review completata autorizza la promozione del claim a
   `verified_independently`;
4. la RC `verified_internally` e il suo tag seguono la decisione di release
   separata e non dipendono dal completamento di questo record.

Fino ad allora `independent_review` e
`independently_verified_claim_authorized` restano `false`. La decisione
[`CHANGE_IMPACT_2026-07-30_RC4_RELEASE_DECISION.md`](CHANGE_IMPACT_2026-07-30_RC4_RELEASE_DECISION.md)
autorizza separatamente RC4 come `verified_internally`; il record corrente
diventa target del tag soltanto dopo la propria CI verde.
