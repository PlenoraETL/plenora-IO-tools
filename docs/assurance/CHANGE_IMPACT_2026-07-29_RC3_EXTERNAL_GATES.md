# Change impact analysis — gate esterni RC3

Data: 2026-07-29

## Revisione indipendente

Il record resta `pending_eligible_reviewer`. Non è stata registrata
un'identità eleggibile, quindi RC3 non dichiara `verified_independently`.
Preparare codice, test o documentazione aggiuntiva non sostituisce la persona
richiesta dal processo.

## Ratifica ICD

Verifica read-only del checkout locale `plenora-contracts`:

- ultimo tag: `v2.0-rc8`;
- target adottato da IO-tools:
  `62b12e3496466d2c908dac3cc098640b99b52e21`;
- stato del candidato: `non ratificato`;
- HEAD successivo:
  `7ad15872be26defabfb171c426086bd14a2d4be2`, relativo al corpus e al runner di
  conformità, senza nuovo tag di contratto.

Non esiste quindi una ratifica verso cui migrare RC3. Le chiavi wire e le
deroghe correnti restano invariate; lo stato è
`revalidated_rc8_unratified`.

## FileGDB/GDAL Windows e matrice filesystem

Il runner Windows corrente non dispone di una distribuzione GDAL/OpenFileGDB
nativa, pinnata e ridistribuibile. Dichiarare verde la matrice usando soltanto
lo stub pure-Rust sarebbe falso. Lo stato resta `environment_open_not_run`.

L'immagine Linux locale espone GDAL 3.10.3, mentre `gdal-sys = 0.10.0` non
include binding pre-generati per quella versione. Il build release
`--all-features` si arresta quindi nel build script della dipendenza; non è
registrato come prova FileGDB. Il build release standard e i test workspace
all-feature eseguiti per questa tranche sono invece verdi. Non si
abilita `bindgen` e non si altera il pin soltanto per rendere verde un ambiente
non qualificato.

La chiusura richiede:

- pacchetto x64 identificato per versione e digest;
- licenza e provenienza registrate;
- test read/write, projection, CRS/axis, Z/M, crash/recovery e determinismo;
- NTFS e filesystem Linux qualificato con la stessa fixture;
- CI ancorata alla revisione esatta del componente.

## Ownership della catena

L'harness a tre componenti non è contenuto in IO-tools. La qualifica di sistema
è esterna al componente; il checkout locale `plenora-contracts` contiene ora
il proprio perimetro `conformance/`. Questa osservazione non autorizza IO-tools
a modificarlo o a rivendicarne gli esiti.

## Esito

I tre gate restano aperti in modo esplicito. Nessun claim RC3, di sistema o
avionico viene promosso.
