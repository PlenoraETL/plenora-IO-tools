# Change impact analysis — codec WKB esteso RC3

Data: 2026-07-29

## Modifica

Il codec lossless autoritativo accettava soltanto i type code WKB 1–7. Il
candidato RC3 aggiunge i tipi concreti SQL/MM rappresentati dal contratto:

- `CircularString` (8);
- `CompoundCurve` (9);
- `CurvePolygon` (10);
- `MultiCurve` (11);
- `MultiSurface` (12);
- `PolyhedralSurface` (15);
- `TIN` (16);
- `Triangle` (17).

Con i sette tipi semplici, il codec copre ora tutti i 15 tipi geometrici
concreti del modello. `Unknown` resta intenzionalmente uno stato della
dichiarazione di tipo e non viene trasformato in una geometria inventata.

## Invarianti

- decode, visitor senza AST ed encoder condividono la stessa mappa type code;
- Z, M e SRID restano lossless sia nella forma ISO sia in EWKB;
- i figli degli aggregati sono validati secondo la famiglia canonica;
- `WkbLimits` continua a limitare byte, componenti e profondità anche nei nuovi
  contenitori;
- gli adattatori `geo-types`, WKT, GeoJSON, KML, Shapefile e DXF rifiutano i
  tipi estesi che non possono rappresentare senza perdita;
- nessuna curva o superficie viene linearizzata implicitamente.

## Verifica funzionale

- `cargo check --workspace --all-targets --all-features --locked`: pass;
- `cargo test -p plenora-io-model --locked`: 34 pass;
- test golden di round-trip sui 15 tipi concreti: pass;
- test EWKB `CircularString XYZM + SRID`: pass;
- figlio `Polygon` dentro `TIN`: rifiutato;
- adattatore XY sui tipi estesi: rifiutato.

## Veto prestazionale

Confronto A/B nello stesso container, build release, 5.000.000 Point WKB per
campione, sette campioni sequenziali. La baseline è il binario RC2 già
costruito; il candidato è ricostruito dopo la modifica.

| Metrica mediana | RC2 | RC3 candidato | Delta |
|---|---:|---:|---:|
| wall time | 135,83 ms | 136,79 ms | +0,71% |
| throughput | 36.811.092 geom/s | 36.551.338 geom/s | −0,71% |
| allocazioni nel tratto misurato | 0 | 0 | invariato |
| peak heap | 90.733 byte | 90.733 byte | invariato |
| peak RSS | 4.079.616 byte | 4.005.888 byte | −1,81% |

Il delta resta entro il veto del 5%; il cambiamento è accettato. Il percorso
semplice 1–7 non introduce allocazioni aggiuntive.

## Impatto di capability

Questa modifica estende il codec, non promuove automaticamente i descrittori
dei formati. Ogni driver continua a dichiarare soltanto i tipi che può
preservare nel proprio formato. L'eventuale promozione dei formati passthrough
richiede una CIA e test di round-trip dedicati.
