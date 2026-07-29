# Gate per la release candidate di sistema

Questo documento separa la readiness del componente dalla readiness della
catena `IO-tools → data-tools → database-tools`. Lo stato corrente è
**non soddisfatto**. La qualifica di sistema e il relativo harness sono di
proprietà esterna: questo repository non contiene né esegue test che compilano
gli altri due componenti.
Il checkout `plenora-contracts` possiede il perimetro `conformance/`; IO-tools
non ne modifica né incorpora il runner.

La definizione machine-readable è
[`release/system-rc-gate.json`](../../release/system-rc-gate.json).

## Perimetro del test

Il gate deve eseguire entrambe le direzioni:

1. formato file → IO-tools → data-tools → database-tools;
2. database-tools → data-tools → IO-tools → formato file.

Ogni esecuzione registra le revisioni complete dei tre componenti e dell'ICD,
la piattaforma, i comandi, gli hash delle fixture e gli esiti.

## Fixture minime

La matrice comprende almeno:

- Point XY con CRS risolto;
- MultiPolygon XYZM con SRID;
- tipi geometrici misti;
- CRS dichiarato ma irrisolto;
- CRS assente;
- semantica geography;
- metadati nativi su una trasformazione identity-preserving;
- rappresentazioni CRS/SRID deliberatamente conflittuali.

Per ogni fixture vengono confrontati `contract.version`, dichiarazione e lista
dei tipi, dimensioni, encoding, semantica geometry/geography, precisione, SRID,
cinque proprietà CRS e metadati nativi.

## Oracolo

- Un pass-through o riordino compatibile deve preservare byte-per-byte i
  metadati che restano validi.
- Un campo derivato deve ricostruire i metadati dal risultato, non ereditarli
  dalla sorgente.
- Un conflitto fra rappresentazioni deve fallire prima dell'effetto esterno.
- Una perdita ammessa deve comparire nel `LossReport`; un report vuoto non può
  accompagnare una proprietà scomparsa.
- Categoria, fase, effetto remoto e retry devono restare semanticamente
  equivalenti ai due bordi.

## Condizione di promozione

La RC di sistema può essere dichiarata soltanto quando tutte le fixture passano
in entrambe le direzioni almeno su Linux e Windows, senza proprietà perse o
inventate, e il bundle di evidenza è riproducibile.

Il tag immutabile `v0.1.0-rc.2` conserva l'osservazione storica di una tratta
Point XYZ con CRS risolto, SRID e `field_id`; non è un gate eseguibile della
baseline corrente. Direzione inversa, matrice completa, Windows e ratifica
delle sezioni candidate dell'ICD devono essere verificate dal proprietario
della qualifica di sistema. Il superamento dei test di IO-tools non modifica lo
stato `not_satisfied`.
