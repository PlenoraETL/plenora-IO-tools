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
import pathlib
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
POLITICA_ABI = {
    "kernel32.dll",
    "advapi32.dll",
    "user32.dll",
    "gdi32.dll",
    "shell32.dll",
    "ole32.dll",
    "oleaut32.dll",
    "ws2_32.dll",
    "crypt32.dll",
    "bcrypt.dll",
    "ncrypt.dll",
    "secur32.dll",
    "shlwapi.dll",
    "version.dll",
    "winmm.dll",
    "wldap32.dll",
    "userenv.dll",
    "iphlpapi.dll",
    "dbghelp.dll",
    "psapi.dll",
    "rpcrt4.dll",
    "setupapi.dll",
    "cfgmgr32.dll",
    "ntdll.dll",
    "msvcrt.dll",
    "api-ms-win-crt-runtime-l1-1-0.dll",
}

MACCHINA_X86_64 = 0x8664


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
    arg = a.parse_args()

    albero = arg.albero.resolve()
    manifesto_percorso = albero / "MANIFEST.json"
    if not manifesto_percorso.is_file():
        sys.exit(f"{manifesto_percorso} assente")
    manifesto = json.loads(manifesto_percorso.read_text(encoding="utf-8"))
    lock = json.loads(LOCK.read_text(encoding="utf-8"))
    contratto = lock.get("contratto_di_verifica")
    if contratto is None:
        sys.exit(
            "il lock di Windows non porta un `contratto_di_verifica`. Va scritto misurando su "
            "un runner Windows: quello di Linux parla di ELF, di DT_NEEDED e di GLIBC, e qui "
            "le domande sono altre. Senza, questo controllo non ha una soglia da applicare."
        )

    radice = albero / arg.radice
    if not radice.is_file():
        sys.exit(f"radice della chiusura assente: {radice}")

    errori: list[str] = []
    interne, esterne, ritardate = chiusura(radice, albero)
    spediti = [radice, *sorted(set(interne.values()))]
    print(f"chiusura da {arg.radice}: {len(interne)} DLL interne, {len(spediti)} PE")
    print(f"import ritardati: {len(ritardate)}")

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

    # 2. le esterne coincidono **esattamente** con l'atteso
    attese = {n.lower() for n in contratto["dll_di_sistema_attese"][manifesto["profilo"]]}
    fuori_politica = sorted(esterne - POLITICA_ABI)
    if fuori_politica:
        errori.append(
            f"DLL fuori dalla politica di sistema: {fuori_politica}. O sono spedite dentro "
            "l'albero, o la politica va allargata con una ragione."
        )
    if esterne != attese:
        errori.append(
            f"le DLL di sistema non coincidono con quelle attese. In piu': "
            f"{sorted(esterne - attese)}. In meno: {sorted(attese - esterne)}. Una DLL che "
            "smette di essere spedita e viene presa dal sistema resta dentro la politica, e "
            "solo l'insieme atteso se ne accorge."
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
                "dll_esterne": sorted(esterne),
                "import_ritardati": sorted(ritardate),
                "percorsi_assoluti_classificati": sum(per_categoria.values()),
                "percorsi_assoluti_non_classificati": len(non_classificati),
                # Il gate finale confronta i nomi con quelli di Linux: la forma
                # e' comune, le misure sono native.
                "elf_spediti": len(spediti),
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
