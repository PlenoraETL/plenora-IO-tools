# Installazione e migrazione — mettere in funzione 2.0.0

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

per esempio `plenora-io-2.0.0-linux-x86_64-filegdb.tar.gz`. Le piattaforme
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

`Requires-Python` è `>=3.11,<3.14`, e copre **soltanto** le versioni che la CI
prova. Dichiarare `>=3.11` senza limite superiore prometterebbe ogni Python
futuro, e una promessa del genere la si mantiene provandola.

## Scegliere il profilo

`base` porta il binario, il manifesto, l'SBOM e le licenze. Non porta GDAL: il
workspace senza `gdal-backend` è Rust puro, e il driver FileGDB degrada a uno
stub che **rifiuta** la capability invece di fingerla. Su Windows porta comunque
il runtime C ridistribuibile, che ogni binario MSVC pretende.

`filegdb` aggiunge il runtime GDAL fissato — 3.9.3, dalla stessa catena su
entrambe le piattaforme — e i suoi dati.

Il modo per sapere che cosa si è installato non è leggere il nome del file:

```
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
radice = pathlib.Path(".")
manifesto = json.loads((radice / "MANIFEST.json").read_text(encoding="utf-8"))
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

### La prima prova

```
bin/plenora-io --version
bin/plenora-io catalog
```

La prima risponde `{"status":"ok","version":"2.0.0"}` — due campi, né uno di
più. La seconda enumera i dieci driver con ciò che ciascuno sa fare.

Nulla va aggiunto al `PATH` perché il binario funzioni: se lo si aggiunge è per
comodità, e l'SDK Python lo troverebbe anche di lì (vedi sotto).

## Installare l'SDK Python

```
pip install plenora_io-2.0.0-py3-none-any.whl
```

oppure, se si preferisce ricostruire dai sorgenti:

```
pip install plenora_io-2.0.0.tar.gz
```

L'installazione non scarica nient'altro: il pacchetto non ha dipendenze a
runtime, e in particolare **non scarica il binario**. Un pacchetto Python che
tirasse giù un eseguibile sarebbe una via d'esecuzione di codice che nessun
lockfile controlla, e chi lo installa non l'ha chiesta.

### Come l'SDK trova il binario

Quattro posti, in quest'ordine, e nessun quinto:

1. il percorso passato esplicitamente: `Client(binary="/opt/plenora/bin/plenora-io")`;
2. la variabile d'ambiente `PLENORA_IO_BIN`;
3. `bin/plenora-io` nell'albero distribuito, se il pacchetto è stato installato
   accanto a uno;
4. il `PATH`.

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
print(cliente.version())          # Version(status='ok', version='2.0.0')
print(len(cliente.catalog().drivers))
```

Chi ha bisogno di FileGDB può pretenderlo invece di sperarlo:

```python
cliente = Client()
cliente.require_profile("filegdb")    # solleva ProfileError se l'albero è `base`
```

Va chiamato **prima** del lavoro. Scoprire il profilo sbagliato dal fallimento
di una conversione a metà costa un file d'uscita parziale e un errore che parla
di un driver invece che di un pacchetto.

Il controllo legge `MANIFEST.json` accanto al binario. Un binario costruito da
`cargo` non ha un manifesto, ed è perfettamente usabile: l'assenza non è un
errore. Un manifesto **presente e illeggibile** lo è, perché vuol dire che
l'artefatto è rotto.

## Migrazione 1.x → 2.0.0

Il perimetro di compatibilità del prodotto è dichiarato e stretto:
`cli_json_only`. Ciò che si migra sono le buste JSON su stdout. L'API Rust è
interna e instabile in entrambe le versioni — non era un contratto pubblico
nella 1.x e non lo è ora — e chi ne dipendeva dipendeva da qualcosa che nessuna
regola proteggeva.

### La differenza è il protocollo predefinito

La 1.x emetteva il protocollo v1. La 2.0.0 emette il **v2**, e il v1 resta
raggiungibile con un'opzione esplicita.

| Comando | 1.x | 2.0.0 |
|---|---|---|
| `inspect` | `plenora-io-inspect-v1` | `plenora-io-inspect-v2` |
| `layers` | `plenora-io-layers-v1` | `plenora-io-layers-v2` |
| `read` | `plenora-io-read-v1` | `plenora-io-read-v2` |
| `convert` | `plenora-io-convert-v1` | `plenora-io-convert-v2` |
| `catalog` | — | `plenora-io-catalog-v2` sempre (vedi sotto) |
| errori | `plenora-io-error-v1` | `plenora-io-error-v1`, invariato |

Un consumatore che verifica `contract` — ed è ciò che un consumatore dovrebbe
fare — se ne accorge alla prima chiamata. Uno che verifica `protocol_version`
legge `2` dove leggeva `1`.

### Le tre differenze che richiedono di toccare il codice

Non sono dedotte dai manifesti: sono **misurate** eseguendo il binario nei due
protocolli sulla stessa fixture. Chi vuole rifare la misura:

```
python3 scripts/delta-protocollo.py --binario target/debug/plenora-io --lavoro /tmp/dp
```

**1. `counts` passa da oggetto a lista.** È la sola rottura vera, e riguarda
`convert`:

```jsonc
// v1 — le chiavi sono le categorie
"write_loss": { "counts": { "crs_id_not_preserved_absent": 1 } }

// v2 — le chiavi sono fisse, le categorie sono valori
"write_loss": { "counts": [ { "categoria": "crs_id_not_preserved_absent", "conteggio": 1 } ] }
```

Chi scriveva `loss["counts"]["qualche_categoria"]` deve scorrere la lista. Il
cambiamento non è cosmetico: nel v1 quelle chiavi non avevano tetto e potevano
arrivare a 4096, con identificatori controllati da chi fornisce il file. Un
oggetto le cui chiavi le decide l'input è una superficie che il consumatore non
può dimensionare.

**2. Sette campi nuovi dicono che cosa manca.** Ogni sezione diagnostica del v2
porta `troncato`, `omesse_esatte` e `omesse` con le quattro cause separate —
`categorie_omesse`, `ragioni_omesse`, `esempi_omessi`, `omesse_per_byte`:

```jsonc
"fidelity": {
  "troncato": false,
  "omesse_esatte": true,
  "omesse": { "categorie_omesse": 0, "ragioni_omesse": 0, "esempi_omessi": 0, "omesse_per_byte": 0 }
}
```

Sono campi aggiunti: chi non li legge continua a funzionare. Ma sono anche il
motivo per cui il v2 può permettersi dei tetti — nel v1 la diagnostica non ne
aveva, e la sua dimensione la decideva chi forniva il file. Nel v2 qualcosa può
restare fuori, e quando succede **lo dice**, con quanto e per quale delle quattro
ragioni. Un consumatore che aggrega conteggi dovrebbe guardarli: `troncato: true`
vuol dire che i numeri che sta sommando sono parziali, e nel v1 quel caso non
era distinguibile da un file senza perdite.

**3. I nomi presi dal file spariscono dai testi.** Nel v1 `reasons[].detail` e
gli esempi portavano nomi di layer e di attributo, più la forma `Debug` di tipi
di una dipendenza. Nel v2 non più: al loro posto ci sono `layer_index`,
`field_index` e `type_class`, e il testo è curato.

```jsonc
// v2
"reasons": [ { "code": "…", "detail": "…", "layer_index": 0, "field_index": 3 } ]
```

Chi mostrava `detail` all'utente finale mostra ora una frase che non nomina il
suo file; per risalire al nome deve incrociare l'indice con lo schema che ha già.
È il prezzo della regola, ed è deliberato: un identificatore che viene dal file
non è pubblicabile per il solo fatto di essere finito in un messaggio d'errore —
e nemmeno un suo hash, che resta un identificatore controllato da chi il file lo
fornisce. Il secondo effetto è che una dipendenza che cambia la propria `Debug`
non può più cambiare la busta senza che nessuno tocchi il protocollo.

### `catalog` non ha un v1

`catalog` emette il v2 in ogni caso, anche col flag legacy, e senza avviso.
Non è un residuo: il dispatch tiene separati i quattro comandi che possono
consegnare un documento legacy — `inspect`, `layers`, `read`, `convert` — da
`catalog`, che il documento legacy non lo produce.

Il flag, passato a `catalog`, **non viene rifiutato**: chi scrive
`catalog --legacy-protocol-v1-unsafe` riceve una busta v2 senza che niente
glielo dica. Vale la pena saperlo prima di scriverlo in uno script.

### Restare sul v1

```
plenora-io read dati.geojson --legacy-protocol-v1-unsafe
```

L'opzione va **dopo** il sottocomando. Il nome dice che cosa si sta scegliendo:
`--protocol 1` sarebbe stato più corto e avrebbe fatto sembrare le due versioni
due opzioni pari, e non lo sono.

Chi la usa riceve su **stderr** un avviso che nomina i due difetti che si sta
riprendendo: identificatori controllati dal file, e fino a 4096 chiavi in
`counts`. Su stdout non ci va niente, perché il v1 è congelato byte per byte e
aggiungere un avviso al documento sarebbe cambiarlo.

Che cosa si accetta scegliendolo: il v1 è `frozen_for_1_0` e non riceverà
correzioni — le sue regole dicono che cambiare il tipo o il significato di un
campo obbligatorio richiede una versione nuova, ed è esattamente ciò che il v2
ha fatto. I gate di release usano il v2 e soltanto il v2.

### Che cosa non cambia

`--version` risponde con due campi, `status` e `version`, e non è una busta v2:
è la busta di **bootstrap**, quella che si legge prima di sapere quale protocollo
il binario parli. Non porta `contract` né `protocol_version`, e non li porterà.

La busta d'errore resta `plenora-io-error-v1` con `protocol_version: 1`
qualunque sia il protocollo scelto per stdout. Non è un'incoerenza: sono due
superfici, va su stderr, e né la sua struttura né il suo significato cambiano
col v2. Crearne una v2 identica avrebbe moltiplicato i nomi senza aggiungere
un'affermazione.

Restano invariati anche il quartetto che classifica ogni errore — `category`,
`code`, `phase`, `retry` — e la regola che su di esso, e mai sul testo del
messaggio, un consumatore prende le proprie decisioni.

### L'SDK Python non si migra

Non esisteva nella 1.x. La sua prima versione è la 2.0.0, parla il v2 e basta, e
un binario che rispondesse v1 lo tratta come un errore di protocollo invece di
adattarsi. Non c'è codice 1.x da riscrivere: c'è, semmai, codice che chiamava
`subprocess` a mano e che l'SDK può sostituire.

## Dove guardare dopo

| | |
|---|---|
| [PRODUCT.md](PRODUCT.md) | che cosa ciascun driver promette, opzione per opzione |
| [ENGINEERING.md](ENGINEERING.md) | come è fatto e come viene verificato |
| [sdk/python/README.md](../sdk/python/README.md) | l'SDK: metodi, errori, deadline, cancellazione |
| [`release/cli-protocol-v2.json`](../release/cli-protocol-v2.json) | il contratto delle buste, campo per campo |
