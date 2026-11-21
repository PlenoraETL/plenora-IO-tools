# Installazione e migrazione — mettere in funzione il prodotto

<!-- generato da codice/versione: inizio -->

Versione del prodotto: **10.1.0**.

| Artefatto Python | Nome |
| --- | --- |
| wheel | plenora_io-10.1.0-py3-none-any.whl |
| sdist | plenora_io-10.1.0.tar.gz |

<!-- generato da codice/versione: fine -->

Questo documento si rivolge a chi **riceve** il prodotto: installa un artefatto,
lo collega a qualcosa che già funziona, e — se veniva da una 1.x — deve sapere
che cosa nel frattempo è cambiato sul filo.

Non descrive come gli artefatti si costruiscono né come si qualificano: quello
sta in [ENGINEERING.md § Distribuzione](ENGINEERING.md#distribuzione), ed è una
domanda che si fa chi lavora al prodotto, non chi lo usa. Qui la domanda è
un'altra: che cosa scaricare, come accertarsi che sia arrivato intero, e che
cosa riscrivere nel proprio codice.

## Due cose separate, e non per caso

Il prodotto è un **eseguibile nativo**, `plenora-io`, che parla JSON su stdout.
L'SDK Python è un **wrapper** che lo esegue e ne tipizza le risposte. Sono due
artefatti distinti, e il pacchetto Python non contiene l'eseguibile.

La separazione ha un prezzo e una ragione. Il prezzo è che installare l'SDK non
basta: va anche installato il binario, e i due vanno fatti trovare. La ragione è
che incorporare l'eseguibile vorrebbe dire una wheel per ogni piattaforma, il
triplo degli artefatti da qualificare, e un prodotto che si aggiorna soltanto
cambiando il pacchetto Python — legando la cadenza di rilascio del motore a
quella di un wrapper che ha ragioni di cambiare del tutto diverse.

Chi non usa Python installa soltanto il primo. È il caso normale.

## Che cosa esiste

### Gli artefatti nativi

Un archivio per **piattaforma × profilo**, chiamato secondo la forma

```
plenora-io-<versione>-<piattaforma>-<profilo>.<estensione>
```

per esempio `plenora-io-<versione>-linux-x86_64-filegdb.tar.gz`. Le piattaforme
distribuite sono `linux-x86_64` (`tar.gz`) e `windows-x86_64` (`zip`); i profili
sono `base` e `filegdb`.

Il profilo sta nel nome perché due archivi della stessa versione e piattaforma
differiscono per una **capability**, non per un dettaglio di build: chi scarica
deve poterlo leggere dal nome invece di scoprirlo eseguendo `catalog`.

Le soglie di sistema sono dichiarate e misurate, non dedotte dall'ambiente di
sviluppo:

| Piattaforma | Requisito dichiarato |
|---|---|
| `linux-x86_64` | glibc ≥ 2.35 — Ubuntu 22.04 e successive |
| `windows-x86_64` | Windows 10 22H2 e successive, Windows 11, Windows Server 2022 e successive |

### I due artefatti Python

| Formato | Nome | Che cos'è |
|---|---|---|
| wheel | `plenora_io-<versione>-py3-none-any.whl` | il pacchetto installabile senza ricostruzione |
| sdist | `plenora_io-<versione>.tar.gz` | i sorgenti da cui il pacchetto si ricostruisce, con i test dentro |

`py3-none-any` vuol dire che di piattaforme non ne serve nessuna: non c'è un
solo file compilato. La sdist porta i test perché chi la riceve possa verificare
ciò che installa — e non tutti girano fuori dal repository: quali saltano, e
perché, è un elenco chiuso in
[`assurance/registries/sonde-saltate-nella-sdist.json`](../assurance/registries/sonde-saltate-nella-sdist.json).

`Requires-Python` è `>=3.14,<3.15`, e copre **soltanto** le versioni che la CI
prova. Dichiarare `>=3.14` senza limite superiore prometterebbe ogni Python
futuro, e una promessa del genere la si mantiene provandola.

## Come si ottiene, e sotto quali termini

Gli artefatti — i quattro nativi e i due Python — si consegnano per un **canale
riservato a clienti autorizzati**. Non stanno su PyPI né su altri indici
pubblici, e il pacchetto Python porta nei metadati il classificatore
`Private :: Do Not Upload`, che i servizi d'indice leggono per rifiutare il
caricamento.

Il codice è sotto la licenza **proprietaria** del repository,
[`LICENSE`](../LICENSE): «Copyright (c) Plenora ETL. All rights reserved.»,
nessun diritto d'uso, copia, modifica, distribuzione o pubblicazione se non per
accordo scritto con Plenora ETL. I crate la dichiarano con `license-file`, il
pacchetto Python con `License: Proprietary`. Chi riceve un artefatto non è
autorizzato a ridistribuirlo né a pubblicarlo.

Lo stesso testo viaggia con ogni artefatto: `LICENSE` alla radice degli alberi
nativi, `.dist-info/licenses/LICENSE` nella wheel, `LICENSE` alla radice della
sdist; il referto `licenze-artefatto` lo dichiara (`licenza_first_party`) e la
verifica lo pretende identico a quello del repository. Fino alla 4.1.0 nessuna
licenza first-party era dichiarata, e il contratto di release lo registra
nell'invariante `distribuzione.licenza-first-party` come **capacità differita**,
con la riga in [docs/RELEASE.md](RELEASE.md): chiuderla è della registrazione
della prossima release.

Le licenze dei **componenti di terzi** sono un'altra cosa, e restano dovute:
ogni artefatto nativo porta in `LICENSES/` il testo di ognuna, e un gate conta
che ci siano tutte. Il pacchetto Python non ne porta perché non spedisce byte
di terzi.

Wheel e sdist sono Python puro e contengono i sorgenti leggibili; la sdist
anche i test. La consegna è riservata, non segreta: non è stata promessa alcuna
riservatezza del codice.

## Scegliere il profilo

`base` porta il binario, il manifesto, l'SBOM e le licenze. Non porta GDAL: il
workspace senza `gdal-backend` è Rust puro, e il driver FileGDB degrada a uno
stub che **rifiuta** la capability invece di fingerla. Su Windows porta comunque
il runtime C ridistribuibile, che ogni binario MSVC pretende.

`filegdb` aggiunge il runtime GDAL fissato — 3.9.3, dalla stessa catena su
entrambe le piattaforme — e i suoi dati.

Il modo per sapere che cosa si è installato non è leggere il nome del file:

```
# esito: {"exit":0,"status":"ok"}
plenora-io catalog
```

dichiara, per ogni driver, `available` e `required_feature`. Sul profilo `base`
il driver `filegdb` risulta non disponibile, e lo dice prima che qualcuno provi
a usarlo.

## Installare l'albero nativo

L'archivio si estrae dove si vuole: nessun percorso assoluto è incorporato nel
binario o nelle librerie. Il layout è

```
bin/plenora-io          (bin/plenora-io.exe su Windows)
lib/                    solo profilo filegdb — su Windows le DLL stanno in bin/
share/gdal/             solo profilo filegdb
share/proj/             solo profilo filegdb
LICENSE
LICENSES/
SBOM.spdx.json
MANIFEST.json
```

Su Windows le DLL stanno in `bin/` e non in `lib/` perché è lì che il caricatore
guarda per prima; spostarle romperebbe l'avvio.

### Verificare ciò che è arrivato

`MANIFEST.json` porta un digest per **ogni** file spedito, insieme a versione,
piattaforma, profilo, revisione e lock del runtime. La verifica si fa
sull'albero estratto, che è dove i file possono essere cambiati o spariti:

```
python3 - <<'FINE'
import hashlib, json, pathlib

def unique_pairs(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise json.JSONDecodeError("chiave JSON duplicata", "", 0)
        result[key] = value
    return result

radice = pathlib.Path(".")
manifesto = json.loads(
    (radice / "MANIFEST.json").read_text(encoding="utf-8"),
    object_pairs_hook=unique_pairs,
)
for voce in manifesto["file"]:
    # I percorsi del manifesto costruito su Windows usano il separatore di lì.
    percorso = radice / voce["percorso"].replace("\\", "/")
    digest = hashlib.sha256(percorso.read_bytes()).hexdigest()
    if digest != voce["sha256"]:
        print("DIVERSO:", voce["percorso"])
print(len(manifesto["file"]), "file verificati")
FINE
```

Oggi nessuno strumento di verifica viene **spedito dentro** l'archivio: la
verifica sopra è a carico di chi riceve, e questo documento la scrive per esteso
proprio perché non c'è un comando da invocare al suo posto. Nel repository lo
stesso controllo lo fa `scripts/check-digest-manifesto.py --albero <estratto>`,
e la CI lo esegue su ogni artefatto costruito.

Il manifesto dice che i file sono quelli dell'archivio; non dice da dove venga
l'archivio. Per gli artefatti del canale `candidate` lo dice un'attestazione di
provenienza **firmata** (GitHub attestations, Sigstore), scritta dal workflow
`Distribuzione` sulla revisione da cui l'archivio è uscito. Si verifica
sull'archivio scaricato, prima di estrarlo, e senza fidarsi di nessun file che
lo accompagna:

```
gh attestation verify plenora-io-<versione>-linux-x86_64-base.tar.gz \
  --repo PlenoraETL/plenora-IO-tools \
  --signer-workflow PlenoraETL/plenora-IO-tools/.github/workflows/distribuzione.yml
```

Le `*.provenance.json` accanto agli archivi restano, e dicono di più -- il lock
del runtime, gli strumenti -- ma le scrivono i nostri script e nessuno le firma.

### La prima prova

```
# esito: {"exit":0,"status":"ok","result.cli_protocol_version":2}
bin/plenora-io --version
# esito: {"exit":0,"status":"ok"}
bin/plenora-io catalog
```

La prima risponde con la busta `plenora-io-version-v2`; `result` contiene
`component_version` e `cli_protocol_version`. La seconda enumera i dieci driver con ciò che ciascuno sa fare.

Nulla va aggiunto al `PATH` perché il binario funzioni: se lo si aggiunge è per
comodità, e l'SDK Python lo troverebbe anche di lì (vedi sotto).

## Installare l'SDK Python

```
pip install plenora_io-<versione>-py3-none-any.whl
```

oppure, se si preferisce ricostruire dai sorgenti:

```
pip install plenora_io-<versione>.tar.gz
```

Con l'adattatore Arrow facoltativo (`read_table()`, `write()` da un oggetto
PyArrow) si chiede l'extra, che installa `pyarrow>=25,<26`:

```
pip install "plenora_io-<versione>-py3-none-any.whl[pyarrow]"
```

L'installazione non scarica nient'altro: il pacchetto non ha dipendenze
obbligatorie, e in particolare **non scarica il binario**. Un pacchetto Python che
tirasse giù un eseguibile sarebbe una via d'esecuzione di codice che nessun
lockfile controlla, e chi lo installa non l'ha chiesta.

### Come l'SDK trova il binario

La selezione segue questo ordine; i primi due sono vincolanti e non ripiegano se il file manca:

1. il percorso passato esplicitamente: `Client(binary="/opt/plenora/bin/plenora-io")`;
2. la variabile d'ambiente `PLENORA_IO_BIN`;
3. il modulo `_native`, se disponibile e senza selezione esplicita;
4. `bin/plenora-io` nell'albero distribuito, se il pacchetto è stato installato
   accanto a uno;
5. il `PATH`.

Se non lo trova solleva `BinaryNotFound` **dicendo dove ha cercato**. Non ne
inventa uno e non ne procura uno.

L'ordine non è arbitrario. L'esplicito batte l'ambiente perché chi scrive una
riga di codice sta dicendo qualcosa di più preciso di chi ha esportato una
variabile tre shell fa; l'ambiente batte l'albero perché è il modo di provare un
binario diverso senza reinstallare; l'albero batte il `PATH` perché un artefatto
installato porta con sé le proprie librerie, e prendere dal `PATH` il binario di
un'altra installazione le mescolerebbe.

### La prima prova

```python
from plenora_io import Client

cliente = Client()
print(cliente.version())          # Version(component_version=..., cli_protocol_version=2)
print(len(cliente.catalog().drivers))
```

Chi ha bisogno di FileGDB può pretenderlo invece di sperarlo:

```python
cliente = Client()
cliente.require_profile("filegdb")    # solleva ProfileError se l'albero è `base`
```

Va chiamato **prima** del lavoro. Scoprire il profilo sbagliato dal fallimento
di una conversione a metà produce un errore del driver anziché una verifica preventiva del pacchetto.

Il controllo legge `MANIFEST.json` accanto al binario. Un binario costruito da
`cargo` non ha un manifesto, ed è perfettamente usabile: l'assenza non è un
errore per l’uso normale, ma `require_profile()` solleva `ProfileError` senza manifesto,
anche col backend nativo. Un manifesto **presente e illeggibile** lo è, perché vuol dire che
l'artefatto è rotto.

## Protocollo corrente

La CLI emette il protocollo 2 e il contratto comune `plenora-error-v1`.
Non sono disponibili protocolli delle release Plenora precedenti.
`convert` richiede `--from` e `--to`; `catalog` descrive le opzioni correnti.

## Dove guardare dopo

| | |
|---|---|
| [PRODUCT.md](PRODUCT.md) | che cosa ciascun driver promette, opzione per opzione |
| [ENGINEERING.md](ENGINEERING.md) | come è fatto e come viene verificato |
| [sdk/python/README.md](../sdk/python/README.md) | l'SDK: metodi, errori, deadline, cancellazione |
| [`release/cli-protocol-v2.json`](../release/cli-protocol-v2.json) | il contratto delle buste, campo per campo |


## Identificatori e opzioni dei formati

<!-- generato da codice/formati: inizio -->

| Id | Estensioni | Direzione | Feature | Lettura nativa | Lettura effettiva | Pruning spaziale / predicati | Fedeltà | Tipi scrivibili |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| csv | ["csv"] | bidirectional | assente | streaming_sequential | streaming_sequential | none / none | conditional | ["boolean","signed_integer","unsigned_integer","floating","utf8","temporal"] |
| dxf | ["dxf"] | bidirectional | assente | materialized | streaming_sequential | none / none | approximating | ["boolean","signed_integer","unsigned_integer","floating","utf8","binary","temporal","decimal","nested","other"] |
| filegdb | ["gdb"] | bidirectional | gdal-backend | streaming_sequential | materializing | none / none | conditional | ["signed_integer","floating","utf8"] |
| geojson | ["geojson","json"] | bidirectional | assente | streaming_sequential | streaming_sequential | none / none | conditional | ["boolean","signed_integer","unsigned_integer","floating","utf8","temporal"] |
| geoparquet | ["parquet"] | bidirectional | assente | streaming_random | streaming_columnar | bounding_box_statistics / numeric_min_max_statistics | lossless | ["boolean","signed_integer","unsigned_integer","floating","utf8","binary","temporal","decimal","nested","other"] |
| gpkg | ["gpkg"] | bidirectional | assente | streaming_random | streaming_sequential | optional_rtree_index / none | conditional | ["boolean","signed_integer","unsigned_integer","floating","utf8","binary"] |
| ipc | ["arrow","arrows"] | bidirectional | assente | streaming_random | streaming_sequential | none / none | lossless | ["boolean","signed_integer","unsigned_integer","floating","utf8","binary","temporal","decimal","nested","other"] |
| kml | ["kml","kmz"] | bidirectional | assente | materialized | streaming_sequential | none / none | conditional | ["boolean","signed_integer","unsigned_integer","floating","utf8","temporal"] |
| shp | ["shp","shp.d"] | bidirectional | assente | streaming_sequential | streaming_sequential | none / none | conditional | ["boolean","signed_integer","unsigned_integer","floating","utf8","temporal"] |
| xls | ["xlsx"] | bidirectional | assente | materialized | streaming_sequential | none / none | conditional | ["boolean","signed_integer","unsigned_integer","floating","utf8","temporal"] |

<!-- generato da codice/formati: fine -->

<!-- generato da codice/opzioni: inizio -->

| Driver | Chiave | Fase | Dominio | Default | Significato |
| --- | --- | --- | --- | --- | --- |
| csv | delimiter | both | char | , | separatore di campo ASCII, esclusi virgolette, CR e LF |
| csv | geometry_encoding | write | {"enum":["wkt","xy"]} | wkt | come scrivere la geometria: colonna WKT o colonne x/y |
| csv | wkt_column | read | text | assente | colonna che contiene la geometria in WKT |
| csv | x_column | read | text | assente | colonna dell'ascissa, da usare insieme a y_column |
| csv | y_column | read | text | assente | colonna dell'ordinata, da usare insieme a x_column |
| geoparquet | compression | write | {"enum":["brotli","gzip","lz4","none","snappy","uncompressed","zstd"]} | snappy | codec di compressione delle pagine |
| ipc | serialization | write | {"enum":["file","stream"]} | file | serializzazione IPC: contenitore `file` con footer, o flusso `stream` |
| shp | publish_mode | write | {"enum":["shapefile_directory_dataset","loose_shapefile_set"]} | assente | forma di pubblicazione; .shp.d si deduce, .shp va accettata |
| shp | row_diagnostics.examples_limit | read | {"integer":{"max":64,"min":1}} | 64 | numero massimo di righe di esempio per diagnostica |
| shp | row_diagnostics.key_field | read | text | assente | campo DBF usato come chiave nelle diagnostiche di riga |
| shp | row_diagnostics.key_policy | read | {"enum":["emit","redact"]} | assente | chiave sempre redatta; emit alias di redact; richiede row_diagnostics.key_field |
| xls | geometry_encoding | write | {"enum":["wkt","xy"]} | wkt | come scrivere la geometria: colonna WKT o colonne x/y |
| xls | sheet | read | text | assente | nome esatto del foglio da leggere; in assenza valgono le regole comuni di selezione |
| xls | wkt_column | read | text | assente | colonna che contiene la geometria in WKT |
| xls | x_column | read | text | assente | colonna dell'ascissa, da usare insieme a y_column |
| xls | y_column | read | text | assente | colonna dell'ordinata, da usare insieme a x_column |

<!-- generato da codice/opzioni: fine -->
