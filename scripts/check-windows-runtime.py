#!/usr/bin/env python3
"""Le verifiche fail-closed sull'artefatto Windows.

# Perche' non e' il verificatore Linux con qualche `if`

Le domande sono altre. Un PE non ha `DT_NEEDED` ma una **import table**, e ne ha
due: quella normale e quella dei *delay import*, che il caricatore risolve alla
prima chiamata invece che all'avvio. Una DLL che comparisse solo fra i delay
import sfuggirebbe a chi guardasse la prima e si manifesterebbe molto dopo, in
esecuzione, su una macchina che non ha quella libreria.

Non esiste un `GLIBC_*` da misurare: su Windows la soglia e' il runtime C
ridistribuibile, e la si affronta spedendo cio' che serve invece di misurare una
versione. Non esiste `$ORIGIN`: il caricatore cerca accanto all'eseguibile, ed
e' per questo che le DLL stanno in `bin/` e non in `lib/`.

Cio' che resta uguale e' la **forma del risultato**: il referto che il gate
finale riconta e' lo stesso di Linux e di macOS.

# Che cosa pretende

1. **Architettura.** Ogni PE spedito e' x86-64. Un PE a 32 bit in un artefatto
   x86-64 e' un artefatto che su alcune macchine non si carica, e il nome non lo
   direbbe.
2. **La chiusura degli import, normali e ritardati.** A partire da
   `bin/plenora-io.exe`, e non da `gdal.dll`: la domanda e' che cosa serve
   all'artefatto.
3. **Le DLL fuori dall'albero coincidono esattamente con l'allowlist di
   sistema.** Non «stanno in una politica generosa»: coincidono. Una DLL che
   smettesse di essere spedita e venisse presa dal sistema resterebbe dentro una
   politica -- e solo l'insieme atteso se ne accorge.
4. **Nessun prefisso di costruzione cotto dentro.** Su Linux la rilocazione di
   conda lascia il prefisso d'installazione nei binari; su Windows il problema
   ha la stessa forma, e la stessa cura: ogni percorso assoluto che nomini il
   prefisso va classificato, e cio' che non rientra in una regola fa rosso.

# Che cosa questo file **non** e'

Non e' mai stato eseguito su un artefatto vero: in questo lotto non c'e' un
runner Windows. Le sue funzioni sono provate su PE sintetici, costruiti dalle
sonde byte per byte, e questo dimostra che sanno leggere un PE -- non che
l'artefatto Windows sia conforme. Sono due affermazioni diverse e il referto le
tiene distinte: senza artefatto non se ne produce nessuno.
"""

from __future__ import annotations

import argparse
import json
import os
import pathlib
import platform
import re
import struct
import sys

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
import distribuzione  # noqa: E402 -- dopo sys.path, che e' il punto

LOCK = pathlib.Path(__file__).resolve().parent / "windows-gdal-lock.json"

# La **politica**: che cosa e' ammissibile trovare fuori dall'albero perche' il
# sistema lo garantisce. Non e' l'elenco atteso: quello sta nel lock, ed e' un
# sottoinsieme di questo.
#
# I nomi sono minuscoli perche' Windows non distingue le maiuscole nei nomi di
# file, e confrontare `KERNEL32.dll` con `kernel32.dll` come stringhe diverse
# sarebbe un rosso che non significa niente.
# La **politica**: che cosa e' ammissibile trovare fuori dall'albero, perche' il
# sistema operativo lo garantisce. Non e' l'elenco atteso: quello sta nel lock,
# per profilo, ed e' un sottoinsieme di questo.
#
# I nomi sono minuscoli perche' Windows non distingue le maiuscole nei nomi di
# file, e confrontare `KERNEL32.dll` con `kernel32.dll` come stringhe diverse
# sarebbe un rosso che non significa niente.
#
# # Che cosa autorizza una voce a stare qui
#
# Che sia un componente **del sistema operativo**, cioe' presente in
# `%SystemRoot%\System32` di un'installazione pulita di ogni versione di
# Windows che il prodotto dichiara di supportare. Non «il runner ce l'ha»: un
# runner di GitHub ha Visual Studio, il .NET SDK e decine di altre cose che un
# server pulito non ha, e classificare guardando il runner produrrebbe un
# artefatto che parte in CI e non parte dove viene installato.
#
# La distinzione ha un caso concreto, ed e' quello che la prima corsa di
# scoperta ha portato: `vcruntime140.dll` **non** e' qui. E' il runtime C
# ridistribuibile di Visual Studio, non un componente di Windows, e il fatto che
# il runner la possieda non dice nulla su una macchina di destinazione. Si
# spedisce.
#
# # Il limite di questa classificazione
#
# Le voci qui sotto sono classificate sulla documentazione dei componenti di
# Windows, non su una misura fatta su un'installazione pulita: un runner di
# GitHub non e' una baseline, e da qui non ne ho una. Una misura su un'immagine
# Server pulita sarebbe una garanzia piu' forte, e resta da fare. Fino ad
# allora la garanzia e' quella che c'e', e questa nota impedisce di scambiarla
# per un'altra.
POLITICA_ABI = {
    # Il nucleo Win32, presente in ogni installazione.
    "kernel32.dll",
    "ntdll.dll",
    "advapi32.dll",
    "user32.dll",
    "gdi32.dll",
    "shell32.dll",
    "shlwapi.dll",
    "ole32.dll",
    "oleaut32.dll",
    "version.dll",
    "psapi.dll",
    "rpcrt4.dll",
    "userenv.dll",
    "setupapi.dll",
    "cfgmgr32.dll",
    "winmm.dll",
    "dbghelp.dll",
    # Rete.
    "ws2_32.dll",
    "iphlpapi.dll",
    "wldap32.dll",
    # `wsock32.dll` e' la Winsock 1.1: un livello di compatibilita' che Windows
    # conserva da NT e che ogni versione supportata porta ancora. Vi arriva
    # `libcurl`, che la usa per le chiamate storiche invece che per quelle di
    # `ws2_32`. Non e' un residuo del runner: e' un componente del sistema, e
    # sparirebbe soltanto in una versione di Windows che rompesse quella
    # compatibilita' -- che sarebbe una notizia, non un dettaglio.
    "wsock32.dll",
    # Crittografia.
    "crypt32.dll",
    "bcrypt.dll",
    "ncrypt.dll",
    "secur32.dll",
    # `bcryptprimitives.dll` e' l'implementazione dei primitivi CNG, introdotta
    # con Vista e presente da allora in ogni versione. Vi arriva la libreria
    # standard di Rust, che la usa per generare numeri casuali: e' quindi una
    # dipendenza del **nostro** binario, non di GDAL, e compare infatti anche
    # nel profilo base.
    "bcryptprimitives.dll",
    # `odbc32.dll` e' il Driver Manager ODBC, parte dei Windows Data Access
    # Components: presente in ogni Windows da NT 4. Vi arriva GDAL, che offre un
    # driver ODBC anche quando nessuno lo usa -- l'import c'e' perche' la
    # libreria e' compilata con quel driver dentro, non perche' l'artefatto lo
    # eserciti.
    "odbc32.dll",
    # Il runtime C **non** e' qui: `vcruntime140.dll`, `vcruntime140_1.dll` e
    # `msvcp140.dll` sono il redistributable di Visual Studio, non componenti
    # del sistema operativo. Vanno spediti, e il costruttore li spedisce.
    "msvcrt.dll",
}

# Le DLL che si e' tentati di ammettere e che invece vanno **spedite**.
#
# Sta scritto qui e non in un commento perche' il controllo lo verifica: se una
# di queste comparisse fra le esterne, il rifiuto direbbe che va spedita invece
# di dire genericamente che non e' attesa.
DA_SPEDIRE_NON_AMMETTERE = {
    "vcruntime140.dll": "runtime C ridistribuibile di Visual Studio, non un componente di Windows",
    "vcruntime140_1.dll": "come sopra",
    "msvcp140.dll": "libreria standard C++ di Visual Studio, ridistribuibile",
}

MACCHINA_X86_64 = 0x8664

# Le API-set non sono DLL: sono nomi virtuali che il caricatore risolve verso
# l'implementazione reale del sistema. `api-ms-win-crt-runtime-l1-1-0.dll` non
# esiste come file, e cercarla in `bin/` o pretenderla in un elenco di DLL
# sarebbe chiedere l'esistenza di qualcosa che per costruzione non esiste.
#
# Vanno quindi in una categoria propria. Metterle nell'allowlist ABI insieme a
# `kernel32.dll` funzionerebbe e direbbe una cosa falsa: che sono file che il
# sistema fornisce, invece che nomi che il sistema traduce.
SCHEMA_API_SET = re.compile(r"^(api-ms-win-|ext-ms-win-)", re.I)


def e_api_set(nome: str) -> bool:
    return bool(SCHEMA_API_SET.match(nome))


# Le quattro classi in cui ogni dipendenza deve cadere. Non e' una tassonomia
# per ordine: e' che le quattro hanno quattro conseguenze diverse, e una
# categoria unica «esterna» le confonderebbe.
CATEGORIE = (
    "interna",       # spedita dentro l'artefatto: la si trova in bin/
    "api_set",       # nome virtuale che il caricatore risolve
    "abi_windows",   # DLL che il sistema garantisce, nell'insieme atteso
    "inattesa",      # nessuna delle tre: blocca
)


class PeMalformato(ValueError):
    """Il file non e' un PE leggibile.

    E' un'eccezione e non un `None` perche' un PE che non si legge non e' un PE
    conforme: tacere sarebbe classificarlo come innocuo.
    """


def _sezioni(dati: bytes) -> tuple[int, int, list[tuple[int, int, int]]]:
    """Macchina, offset delle directory, e le sezioni come (rva, dimensione, offset)."""
    if dati[:2] != b"MZ":
        raise PeMalformato("manca la firma MZ")
    (inizio_pe,) = struct.unpack_from("<I", dati, 0x3C)
    if dati[inizio_pe : inizio_pe + 4] != b"PE\0\0":
        raise PeMalformato("manca la firma PE")
    macchina, quante_sezioni = struct.unpack_from("<HH", dati, inizio_pe + 4)
    (dimensione_opzionale,) = struct.unpack_from("<H", dati, inizio_pe + 20)
    inizio_opzionale = inizio_pe + 24
    (magia,) = struct.unpack_from("<H", dati, inizio_opzionale)
    if magia not in (0x10B, 0x20B):
        raise PeMalformato(f"intestazione opzionale sconosciuta: {magia:#x}")
    # Le directory stanno dopo l'intestazione opzionale, e la loro posizione
    # dipende dal formato: 96 byte per PE32, 112 per PE32+.
    inizio_directory = inizio_opzionale + (96 if magia == 0x10B else 112)
    inizio_sezioni = inizio_opzionale + dimensione_opzionale
    sezioni = []
    for n in range(quante_sezioni):
        base = inizio_sezioni + n * 40
        dimensione_virtuale, rva, dimensione_grezza, offset_grezzo = struct.unpack_from(
            "<IIII", dati, base + 8
        )
        sezioni.append((rva, max(dimensione_virtuale, dimensione_grezza), offset_grezzo))
    return macchina, inizio_directory, sezioni


def _da_rva(rva: int, sezioni: list[tuple[int, int, int]]) -> int | None:
    for base, dimensione, offset in sezioni:
        if base <= rva < base + dimensione:
            return offset + (rva - base)
    return None


def _stringa(dati: bytes, offset: int) -> str:
    fine = dati.find(b"\0", offset)
    return dati[offset : fine if fine >= 0 else len(dati)].decode("ascii", "replace")


def architettura(percorso: pathlib.Path) -> int:
    return _sezioni(percorso.read_bytes())[0]


def importazioni(percorso: pathlib.Path) -> tuple[set[str], set[str]]:
    """Le DLL importate: normali e **ritardate**, tenute distinte.

    Un delay import e' risolto alla prima chiamata invece che all'avvio: una
    DLL che comparisse solo li' sfuggirebbe a chi guardasse la sola import table
    e si manifesterebbe molto dopo, in esecuzione, su una macchina che non ce
    l'ha. Le due tabelle si leggono uguali e vanno cercate in due posti diversi.
    """
    dati = percorso.read_bytes()
    _, inizio_directory, sezioni = _sezioni(dati)

    def nomi(indice_directory: int, scarto_del_nome: int) -> set[str]:
        rva, dimensione = struct.unpack_from("<II", dati, inizio_directory + indice_directory * 8)
        if not rva or not dimensione:
            return set()
        offset = _da_rva(rva, sezioni)
        if offset is None:
            raise PeMalformato(f"directory {indice_directory}: RVA {rva:#x} fuori dalle sezioni")
        trovati: set[str] = set()
        passo = 20
        for n in range(dimensione // passo):
            blocco = offset + n * passo
            if blocco + passo > len(dati):
                break
            (rva_nome,) = struct.unpack_from("<I", dati, blocco + scarto_del_nome)
            if rva_nome == 0:
                break
            offset_nome = _da_rva(rva_nome, sezioni)
            if offset_nome is None:
                continue
            trovati.add(_stringa(dati, offset_nome).lower())
        return trovati

    # Directory 1: import. Directory 13: delay import. Lo scarto del campo
    # «nome» differisce fra le due strutture, ed e' l'unica differenza che conta.
    return nomi(1, 12), nomi(13, 4)


def chiusura(radice: pathlib.Path, albero: pathlib.Path) -> tuple[dict, set[str], set[str]]:
    """La chiusura degli import da un PE, dentro `bin/`.

    Su Windows il caricatore cerca accanto all'eseguibile: le DLL spedite stanno
    in `bin/`, e non c'e' un `$ORIGIN` da dichiarare.
    """
    da_visitare = [radice]
    interne: dict[str, pathlib.Path] = {}
    esterne: set[str] = set()
    ritardate: set[str] = set()
    while da_visitare:
        corrente = da_visitare.pop()
        normali, delay = importazioni(corrente)
        ritardate |= delay
        for nome in normali | delay:
            if nome in interne or nome in esterne:
                continue
            candidato = albero / "bin" / nome
            if candidato.exists():
                interne[nome] = candidato
                da_visitare.append(candidato)
            else:
                esterne.add(nome)
    return interne, esterne, ritardate


def ha_tabella_dei_certificati(percorso: pathlib.Path) -> bool:
    """La directory 4 di un PE: la Certificate Table, dove sta la firma.

    Si legge dai byte, senza strumenti: e' la parte della misura che si puo'
    fare ovunque, e che le sonde possono provare su un PE costruito a mano.
    Dice se il file **e' firmato**, non da chi ne' quando -- quelle due domande
    vogliono il PKCS#7, e per quelle si chiama Windows.
    """
    dati = percorso.read_bytes()
    _, inizio_directory, _ = _sezioni(dati)
    rva, dimensione = struct.unpack_from("<II", dati, inizio_directory + 4 * 8)
    return bool(rva and dimensione)


def misura_della_firma(percorso: pathlib.Path) -> dict:
    """Firma, firmatario e timestamp, misurati sui byte finali.

    La presenza si legge dal PE. L'identita' del firmatario e il timestamp
    vogliono il parsing di un PKCS#7, e su Windows la risposta autorevole la da'
    il sistema: si chiama PowerShell. Fuori da Windows quelle due domande
    restano **non misurate**, che non e' «non firmato» e non e' «va bene»: e'
    una domanda a cui non si e' potuto rispondere, e su una candidate resta
    rossa.
    """
    misura = {
        "firmato": ha_tabella_dei_certificati(percorso),
        "firmatario": None,
        "timestamp": None,
        "come": "tabella dei certificati letta dal PE",
    }
    if sys.platform != "win32":
        misura["non_misurabile_qui"] = (
            "firmatario e timestamp vogliono il sistema che verifica la catena: "
            f"questo controllo gira su {sys.platform}"
        )
        return misura

    import subprocess

    comando = (
        "$f = Get-AuthenticodeSignature -LiteralPath '"
        + str(percorso)
        + "'; "
        "[pscustomobject]@{ stato = [string]$f.Status; "
        "firmatario = [string]$f.SignerCertificate.Subject; "
        "timestamp = [string]$f.TimeStamperCertificate.Subject } | ConvertTo-Json -Compress"
    )
    esito = subprocess.run(
        ["powershell", "-NoProfile", "-NonInteractive", "-Command", comando],
        capture_output=True,
        text=True,
    )
    if esito.returncode != 0:
        misura["non_misurabile_qui"] = f"Get-AuthenticodeSignature ha fallito: {esito.stderr[:200]}"
        return misura
    letto = json.loads(esito.stdout or "{}")
    misura["stato_authenticode"] = letto.get("stato")
    misura["firmato"] = letto.get("stato") == "Valid"
    misura["firmatario"] = letto.get("firmatario") or None
    misura["timestamp"] = letto.get("timestamp") or None
    misura["come"] = "Get-AuthenticodeSignature"
    return misura


def classifica_dipendenza(nome: str, interne: dict, attese: set[str]) -> str:
    """In quale delle quattro classi cade una dipendenza.

    L'ordine delle domande conta: una libreria spedita e' interna anche se
    porta un nome che somiglia a una di sistema, e un'API-set e' un'API-set
    anche se qualcuno l'avesse messa fra le attese. `inattesa` e' il caso che
    resta, ed e' quello che blocca: non «probabilmente va bene», ma «nessuno ha
    deciso che cosa sia».
    """
    if nome in interne:
        return "interna"
    if e_api_set(nome):
        return "api_set"
    if nome in attese:
        return "abi_windows"
    return "inattesa"


def percorsi_assoluti(percorso: pathlib.Path, prefisso: str) -> set[str]:
    """Le stringhe che nominano il prefisso di costruzione.

    Si cercano sia in ASCII sia in UTF-16LE: i binari Windows portano entrambe,
    e cercarne una sola e' un modo di trovarne meno di quante ce ne sono.
    """
    dati = percorso.read_bytes()
    trovati: set[str] = set()
    # L'escape si fa sui **byte**, non sul testo prima di codificarlo. Facendolo
    # prima, i backslash che `re.escape` inserisce vengono codificati insieme al
    # resto: in UTF-16 diventano `\` seguito da `\x00`, cioe' un escape del byte
    # nullo, e il pattern smette di corrispondere a qualunque cosa. La sonda
    # sulle due codifiche ha trovato esattamente questo.
    nudo = prefisso.replace("/", "\\")
    schema = re.escape(nudo.encode("ascii"))
    for m in re.finditer(schema + rb"[^\x00\"'<>|]*", dati):
        trovati.add(m.group(0).decode("ascii", "replace"))
    schema16 = re.escape(nudo.encode("utf-16-le"))
    for m in re.finditer(schema16 + rb"(?:[^\x00][\x00])*", dati):
        trovati.add(m.group(0).decode("utf-16-le", "replace"))
    return trovati


def main() -> int:
    a = argparse.ArgumentParser(description=__doc__)
    a.add_argument("--albero", required=True, type=pathlib.Path)
    a.add_argument("--radice", default="bin/plenora-io.exe")
    a.add_argument("--prefisso-di-costruzione", default=None)
    a.add_argument("--referto", type=pathlib.Path, default=None)
    a.add_argument(
        "--discovery",
        type=pathlib.Path,
        default=None,
        help=(
            "modo scoperta: misura e scrive il rilievo, **non** legge il contratto e "
            "termina rosso. Serve a produrre cio' su cui il contratto verra' scritto, e "
            "un modo che potesse diventare verde da solo lo scriverebbe da se'"
        ),
    )
    arg = a.parse_args()

    albero = arg.albero.resolve()
    manifesto_percorso = albero / "MANIFEST.json"
    if not manifesto_percorso.is_file():
        sys.exit(f"{manifesto_percorso} assente")
    manifesto = json.loads(manifesto_percorso.read_text(encoding="utf-8"))
    lock = json.loads(LOCK.read_text(encoding="utf-8"))
    contratto = lock.get("contratto_di_verifica")
    if contratto is None and arg.discovery is None:
        sys.exit(
            "il lock di Windows non porta un `contratto_di_verifica`. Va scritto misurando su "
            "un runner Windows: quello di Linux parla di ELF, di DT_NEEDED e di GLIBC, e qui "
            "le domande sono altre. Senza, questo controllo non ha una soglia da applicare -- "
            "e per produrre cio' su cui scriverlo c'e' `--discovery`."
        )

    radice = albero / arg.radice
    if not radice.is_file():
        sys.exit(f"radice della chiusura assente: {radice}")

    errori: list[str] = []
    interne, esterne, ritardate = chiusura(radice, albero)
    spediti = [radice, *sorted(set(interne.values()))]
    print(f"chiusura da {arg.radice}: {len(interne)} DLL interne, {len(spediti)} PE")
    print(f"import ritardati: {len(ritardate)}")

    architetture = {}
    for pe in spediti:
        try:
            architetture[pe.name] = f"{architettura(pe):#x}"
        except PeMalformato as e:
            architetture[pe.name] = f"illeggibile: {e}"

    prefisso_dichiarato = arg.prefisso_di_costruzione or manifesto.get("prefisso_di_costruzione")
    incorporati: dict[str, list[str]] = {}
    if prefisso_dichiarato:
        for pe in spediti:
            for percorso in percorsi_assoluti(pe, prefisso_dichiarato):
                incorporati.setdefault(
                    percorso[len(prefisso_dichiarato) :] or "\\", []
                ).append(pe.name)

    # --- il modo scoperta -------------------------------------------------
    #
    # Misura e scrive. **Non** legge il contratto, e termina rosso: un modo che
    # potesse diventare verde da solo scriverebbe il proprio contratto, e un
    # contratto scritto da cio' che deve verificare non verifica niente.
    #
    # Il rosso non e' un difetto trovato: e' l'assenza di una revisione umana.
    # Il referto va riletto, ogni dipendenza va classificata a mano, e solo un
    # commit successivo mette nel lock l'insieme esatto **e il digest di questo
    # referto**, cosi' che si sappia da quale misura viene.
    if arg.discovery is not None:
        rilievo = {
            "schema_discovery": 1,
            "non_qualificante": True,
            "perche_rosso": (
                "questa corsa scopre, non qualifica. Termina rossa perche' manca un contratto "
                "revisionato da una persona: il referto va riletto, ogni dipendenza va "
                "classificata, e solo un commit successivo mette nel lock l'insieme atteso e "
                "il digest di questo documento. Una corsa di scoperta che potesse diventare "
                "verde da sola scriverebbe il proprio contratto."
            ),
            "artefatto": {
                "piattaforma": manifesto["piattaforma"],
                "profilo": manifesto["profilo"],
                "canale": manifesto["canale"],
                "versione": manifesto.get("versione"),
                "prefisso_di_costruzione": prefisso_dichiarato,
            },
            "provenienza_della_misura": {
                "runner": os.environ.get("RUNNER_NAME") or platform.node(),
                "immagine_runner": os.environ.get("ImageOS") or os.environ.get("ImageVersion"),
                "sistema": platform.platform(),
                "sha_sorgente": os.environ.get("GITHUB_SHA") or manifesto.get("revisione"),
                "lock_sha256": distribuzione.sha256(LOCK),
                "lock_gdal_version": lock.get("gdal_version"),
            },
            "misure": {
                "radice": arg.radice,
                "architetture": architetture,
                "import_normali": sorted(
                    {n for pe in spediti for n in importazioni(pe)[0]}
                ),
                "import_ritardati": sorted(ritardate),
                "dll_interne": sorted(interne),
                "api_set": sorted(n for n in esterne if e_api_set(n)),
                "dll_esterne": sorted(n for n in esterne if not e_api_set(n)),
                "percorsi_incorporati": {k: sorted(set(v)) for k, v in sorted(incorporati.items())},
            },
            "da_classificare": (
                "ogni voce di `dll_esterne` va messa in una di queste classi: **interna** "
                "all'artefatto, **api_set** fornita dal sistema, **abi_windows** attesa, "
                "oppure **inattesa** e quindi bloccante. Non si ammettono insiemi larghi -- "
                "`C:\\Windows\\*`, il `PATH`, «qualunque DLL Microsoft» -- perche' un insieme "
                "largo non si accorge di cio' che smette di essere spedito e viene preso dal "
                "sistema, che e' il difetto che l'insieme esatto esiste per cogliere."
            ),
            "contratti_distinti": (
                "`base` e `filegdb` sono due prodotti e vogliono due insiemi attesi. Questo "
                "rilievo riguarda **soltanto** il profilo «"
                + manifesto["profilo"]
                + "»: usarlo per l'altro sarebbe attribuire a un artefatto una misura fatta su "
                "un altro."
            ),
        }
        arg.discovery.parent.mkdir(parents=True, exist_ok=True)
        arg.discovery.write_text(
            json.dumps(rilievo, ensure_ascii=False, indent=2) + "\n", encoding="utf-8"
        )
        print(f"\nrilievo scritto: {arg.discovery}")
        print(f"  DLL interne:   {len(rilievo['misure']['dll_interne'])}")
        print(f"  API-set:       {len(rilievo['misure']['api_set'])}")
        print(f"  DLL esterne:   {len(rilievo['misure']['dll_esterne'])}")
        print(f"  import ritardati: {len(rilievo['misure']['import_ritardati'])}")
        print(f"  percorsi incorporati: {len(rilievo['misure']['percorsi_incorporati'])}")
        print("\n--- ROSSO, e volutamente ---")
        print(f"  {rilievo['perche_rosso']}")
        return 1

    # 1. architettura
    non_x64 = []
    for pe in spediti:
        try:
            if architettura(pe) != MACCHINA_X86_64:
                non_x64.append(pe.name)
        except PeMalformato as e:
            errori.append(f"{pe.name}: {e}")
    if non_x64:
        errori.append(
            f"PE non x86-64: {non_x64}. Un artefatto che ne contiene uno non si carica su "
            "alcune macchine, e il nome non lo direbbe."
        )

    # 2. ogni dipendenza cade in una delle quattro classi
    #
    # `interna`, `api_set`, `abi_windows`, `inattesa`. Le quattro hanno quattro
    # conseguenze diverse, e una categoria unica «esterna» le confonderebbe: una
    # API-set non e' un file che il sistema fornisce ma un nome che traduce, e
    # una DLL attesa non e' una DLL qualunque che sembri di sistema.
    #
    # L'insieme atteso e' **esatto**, e per profilo. Non ci sono insiemi larghi
    # -- `C:\Windows\*`, il `PATH`, «qualunque DLL Microsoft» -- perche' un
    # insieme largo non si accorge di cio' che smette di essere spedito e viene
    # preso dal sistema, che e' il difetto per cui l'insieme esatto esiste.
    per_profilo = contratto["dll_di_sistema_attese"]
    if manifesto["profilo"] not in per_profilo:
        errori.append(
            f"il contratto non ha un insieme atteso per il profilo «{manifesto['profilo']}». "
            "`base` e `filegdb` sono due prodotti: usare l'insieme dell'uno per l'altro "
            "attribuirebbe a un artefatto una misura fatta su un altro."
        )
        per_profilo = {manifesto["profilo"]: []}
    attese = {n.lower() for n in per_profilo[manifesto["profilo"]]}

    classi: dict[str, list[str]] = {c: [] for c in CATEGORIE}
    for nome in sorted(esterne | set(interne)):
        classi[classifica_dipendenza(nome, interne, attese)].append(nome)
    print("dipendenze per classe:")
    for classe in CATEGORIE:
        print(f"  {classe:14s} {len(classi[classe])}")

    da_spedire = [n for n in classi["inattesa"] if n in DA_SPEDIRE_NON_AMMETTERE]
    if da_spedire:
        errori.append(
            "dipendenze che vanno **spedite**, non ammesse: "
            + ", ".join(f"{n} ({DA_SPEDIRE_NON_AMMETTERE[n]})" for n in da_spedire)
            + ". Il runner le possiede perche' ci gira Visual Studio; una macchina di "
            "destinazione pulita potrebbe non averle, e l'artefatto non partirebbe con un "
            "errore che parla di una DLL mancante invece che di cio' che manca davvero."
        )
    if [n for n in classi["inattesa"] if n not in DA_SPEDIRE_NON_AMMETTERE]:
        errori.append(
            "dipendenze inattese: "
            f"{[n for n in classi['inattesa'] if n not in DA_SPEDIRE_NON_AMMETTERE]}. "
            "Nessuna delle quattro classi le "
            "accoglie, e «inattesa» non significa «probabilmente va bene»: significa che "
            "nessuno ha deciso che cosa siano. Vanno spedite dentro l'albero, oppure "
            "aggiunte all'insieme atteso di questo profilo con una ragione e con il digest "
            "del rilievo da cui la decisione viene."
        )

    # E l'insieme atteso dev'essere **esaurito**: una DLL attesa che non compare
    # piu' significa che qualcosa e' cambiato in cio' che l'artefatto chiede, e
    # un contratto che descrive una chiusura che non esiste piu' non verifica.
    mai_viste = sorted(attese - esterne)
    if mai_viste:
        errori.append(
            f"DLL attese e mai richieste: {mai_viste}. L'insieme atteso descrive una chiusura "
            "che non e' piu' questa: va rifatto sul rilievo, non allargato."
        )

    # La politica resta, e serve a un'altra domanda: se una DLL attesa non fosse
    # nemmeno ammissibile, il contratto avrebbe concesso a se stesso un'eccezione.
    fuori_politica = sorted(n for n in classi["abi_windows"] if n not in POLITICA_ABI)
    if fuori_politica:
        errori.append(
            f"DLL attese ma fuori dalla politica ABI: {fuori_politica}. L'insieme atteso non "
            "puo' ammettere cio' che la politica non ammette: sarebbe un'eccezione che il "
            "contratto concede a se stesso."
        )

    # 3. i percorsi di costruzione
    prefisso = arg.prefisso_di_costruzione or manifesto.get("prefisso_di_costruzione")
    per_categoria: dict[str, int] = {}
    non_classificati: dict[str, list[str]] = {}
    if prefisso:
        regole = contratto.get("percorsi_assoluti_ammessi", [])
        for pe in spediti:
            for percorso in percorsi_assoluti(pe, prefisso):
                relativo = percorso[len(prefisso) :] or "\\"
                regola = next(
                    (r for r in regole if re.fullmatch(r["schema"], relativo.replace("\\", "/"))),
                    None,
                )
                if regola is None:
                    non_classificati.setdefault(relativo, []).append(pe.name)
                else:
                    per_categoria[regola["categoria"]] = per_categoria.get(regola["categoria"], 0) + 1
        print("percorsi di costruzione, per categoria:")
        for categoria, quanti in sorted(per_categoria.items()):
            print(f"  {categoria:26s} {quanti}")
        if non_classificati:
            errori.append(
                f"{len(non_classificati)} percorsi di costruzione non classificati: "
                f"{sorted(non_classificati)[:6]}."
            )
    else:
        errori.append(
            "il prefisso di costruzione non e' noto: senza, la ricerca dei percorsi cotti "
            "dentro cercherebbe la stringa sbagliata e non troverebbe nulla -- che e' un "
            "verde che non ha guardato niente."
        )

    if arg.referto:
        distribuzione.scrivi_referto(
            arg.referto,
            verifica="runtime",
            piattaforma=manifesto["piattaforma"],
            profilo=manifesto["profilo"],
            canale=manifesto["canale"],
            esito="verde" if not errori else "rosso",
            misure={
                "pe_spediti": len(spediti),
                "dll_interne": len(interne),
                "dll_esterne": sorted(n for n in esterne if not e_api_set(n)),
                "api_set": sorted(n for n in esterne if e_api_set(n)),
                "dipendenze_per_classe": {c: sorted(classi[c]) for c in CATEGORIE},
                "import_ritardati": sorted(ritardate),
                "percorsi_assoluti_classificati": sum(per_categoria.values()),
                "percorsi_assoluti_non_classificati": len(non_classificati),
                # Il gate finale confronta i nomi con quelli di Linux: la forma
                # e' comune, le misure sono native.
                "binari_spediti": len(spediti),
                "dipendenze_esterne": sorted(esterne),
            },
            errori=errori,
            note="verificatore nativo PE: import e delay-import, architettura, prefissi",
        )

    if errori:
        print("\n--- ROSSO ---")
        for e in errori:
            print(f"  {e}")
        return 1
    print("\ntutte le verifiche sull'artefatto Windows sono verdi")
    return 0


if __name__ == "__main__":
    sys.exit(main())
