#!/usr/bin/env python3
"""Le verifiche fail-closed sull'artefatto macOS.

# Perche' non e' il verificatore Linux con qualche `if`

Un Mach-O dichiara le proprie dipendenze per **install name**, non per SONAME:
non un nome da cercare, ma un percorso che il caricatore usa cosi' com'e'. Se
quel percorso e' assoluto e punta al prefisso di costruzione, l'artefatto
funziona sulla macchina che l'ha costruito e su nessun'altra -- ed e' lo stesso
difetto che su Linux si presentava come `DT_NEEDED` assoluto, con una faccia
diversa. La cura e' `@rpath`, e l'`LC_RPATH` che lo risolve dev'essere radicato
in `@loader_path`.

Il deployment target non e' una soglia da confrontare come `GLIBC_*`: e' un
campo dichiarato in ogni Mach-O, e vale la promessa piu' alta fra tutti quelli
spediti. Un solo binario compilato con un target piu' recente alza il requisito
dell'intero artefatto senza che nulla lo dica.

Fuori dall'albero, su macOS, si ammettono `/usr/lib` e i framework di sistema.
Non «una libreria che sembra di sistema»: quei due percorsi, che sono gli unici
che il dyld shared cache garantisce.

# Che cosa questo file **non** e'

Non e' mai stato eseguito su un artefatto vero: in questo lotto non c'e' un
runner macOS. Le sue funzioni sono provate su Mach-O sintetici, costruiti dalle
sonde byte per byte -- il che dimostra che sanno leggere un Mach-O, non che
l'artefatto macOS sia conforme. Sono due affermazioni diverse, e senza artefatto
non si produce nessun referto.
"""

from __future__ import annotations

import argparse
import json
import pathlib
import struct
import sys

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
import distribuzione  # noqa: E402 -- dopo sys.path, che e' il punto

LOCK = pathlib.Path(__file__).resolve().parent / "macos-gdal-lock.json"

MH_MAGIC_64 = 0xFEEDFACF
CPU_TYPE_ARM64 = 0x0100000C

LC_LOAD_DYLIB = 0x0C
LC_LOAD_WEAK_DYLIB = 0x80000018
LC_REEXPORT_DYLIB = 0x8000001F
LC_ID_DYLIB = 0x0D
LC_RPATH = 0x8000001C
LC_BUILD_VERSION = 0x32
LC_VERSION_MIN_MACOSX = 0x24

DYLIB_CHE_CARICANO = (LC_LOAD_DYLIB, LC_LOAD_WEAK_DYLIB, LC_REEXPORT_DYLIB)

# I soli percorsi che il sistema garantisce. Non e' una politica generosa: fuori
# da questi due un Mach-O sta chiedendo qualcosa che la macchina di destinazione
# potrebbe non avere.
PREFISSI_DI_SISTEMA = ("/usr/lib/", "/System/Library/Frameworks/")


class MachOMalformato(ValueError):
    """Il file non e' un Mach-O leggibile: tacere sarebbe dirlo innocuo."""


def _comandi(dati: bytes):
    """(tipo, blocco) per ogni load command. Solo Mach-O a 64 bit."""
    if len(dati) < 32:
        raise MachOMalformato("troppo corto")
    (magia,) = struct.unpack_from("<I", dati, 0)
    if magia == 0xCAFEBABE or magia == 0xBEBAFECA:
        raise MachOMalformato(
            "e' un binario universale. L'artefatto e' ARM64 soltanto: un fat binary "
            "porterebbe un'architettura che il contratto non dichiara"
        )
    if magia != MH_MAGIC_64:
        raise MachOMalformato(f"magia {magia:#x}: non e' un Mach-O a 64 bit little-endian")
    cpu, _, _, quanti, _, _ = struct.unpack_from("<IIIIII", dati, 4)
    offset = 32
    for _ in range(quanti):
        if offset + 8 > len(dati):
            raise MachOMalformato("load commands troncati")
        tipo, dimensione = struct.unpack_from("<II", dati, offset)
        if dimensione < 8:
            raise MachOMalformato("load command di dimensione impossibile")
        yield tipo, dati[offset : offset + dimensione]
        offset += dimensione


def cpu_type(percorso: pathlib.Path) -> int:
    dati = percorso.read_bytes()
    list(_comandi(dati))  # valida la forma
    (cpu,) = struct.unpack_from("<I", dati, 4)
    return cpu


def _stringa_del_comando(blocco: bytes, scarto: int) -> str:
    (offset,) = struct.unpack_from("<I", blocco, scarto)
    grezza = blocco[offset:]
    fine = grezza.find(b"\0")
    return grezza[: fine if fine >= 0 else len(grezza)].decode("utf-8", "replace")


def dipendenze(percorso: pathlib.Path) -> list[str]:
    """Gli install name delle librerie caricate."""
    return [
        _stringa_del_comando(blocco, 8)
        for tipo, blocco in _comandi(percorso.read_bytes())
        if tipo in DYLIB_CHE_CARICANO
    ]


def install_name(percorso: pathlib.Path) -> str | None:
    for tipo, blocco in _comandi(percorso.read_bytes()):
        if tipo == LC_ID_DYLIB:
            return _stringa_del_comando(blocco, 8)
    return None


def rpath(percorso: pathlib.Path) -> list[str]:
    return [
        _stringa_del_comando(blocco, 8)
        for tipo, blocco in _comandi(percorso.read_bytes())
        if tipo == LC_RPATH
    ]


def deployment_target(percorso: pathlib.Path) -> str | None:
    """Il minimo sistema dichiarato, da `LC_BUILD_VERSION` o dal predecessore.

    Sono due comandi diversi perche' il secondo e' quello che i toolchain
    vecchi emettono: leggerne uno solo significherebbe non trovare il campo su
    un binario di terze parti e concluderne che non lo dichiara.
    """
    for tipo, blocco in _comandi(percorso.read_bytes()):
        if tipo == LC_BUILD_VERSION:
            (minimo,) = struct.unpack_from("<I", blocco, 12)
            return f"{minimo >> 16}.{(minimo >> 8) & 0xFF}.{minimo & 0xFF}"
        if tipo == LC_VERSION_MIN_MACOSX:
            (minimo,) = struct.unpack_from("<I", blocco, 8)
            return f"{minimo >> 16}.{(minimo >> 8) & 0xFF}.{minimo & 0xFF}"
    return None


LC_CODE_SIGNATURE = 0x1D


def ha_firma(percorso: pathlib.Path) -> bool:
    """`LC_CODE_SIGNATURE`: il load command che porta la firma.

    Si legge dai byte, senza strumenti, e le sonde possono provarlo su un Mach-O
    costruito a mano. Dice che il file **e' firmato**, non da chi ne' se la
    firma sia valida: quelle domande vogliono `codesign`, e quella
    sull'accettazione notarile vuole `spctl`.
    """
    return any(tipo == LC_CODE_SIGNATURE for tipo, _ in _comandi(percorso.read_bytes()))


def misura_della_firma(percorso: pathlib.Path, archivio: pathlib.Path | None = None) -> dict:
    """Firma, firmatario, timestamp, hardened runtime e accettazione notarile.

    La presenza si legge dal Mach-O. Il resto lo dicono gli strumenti Apple, e
    fuori da macOS restano **non misurati** -- che non e' «non firmato» e non e'
    «va bene»: e' una domanda a cui non si e' potuto rispondere, e su una
    candidate resta rossa.

    L'accettazione notarile si chiede sull'**archivio**, non sul singolo
    binario: e' l'archivio che viene sottoposto al servizio. E non c'e' stapling
    da verificare, perche' su uno ZIP non si puo' fare: la ricevuta resta al
    servizio, e la prima verifica di Gatekeeper richiedera' rete.
    """
    misura = {
        "firmato": ha_firma(percorso),
        "firmatario": None,
        "timestamp": None,
        "hardened_runtime": None,
        "notarizzato": None,
        "come": "LC_CODE_SIGNATURE letto dal Mach-O",
    }
    if sys.platform != "darwin":
        misura["non_misurabile_qui"] = (
            "firmatario, timestamp, hardened runtime e accettazione notarile vogliono gli "
            f"strumenti Apple: questo controllo gira su {sys.platform}"
        )
        return misura

    import subprocess

    dettaglio = subprocess.run(
        ["codesign", "--display", "--verbose=4", str(percorso)],
        capture_output=True,
        text=True,
    )
    # `codesign --display` scrive su stderr anche quando riesce: e' il suo modo
    # di parlare, non un errore.
    testo = dettaglio.stderr + dettaglio.stdout
    if dettaglio.returncode == 0:
        for riga in testo.splitlines():
            if riga.startswith("Authority=") and misura["firmatario"] is None:
                misura["firmatario"] = riga.split("=", 1)[1]
            if riga.startswith("Timestamp="):
                misura["timestamp"] = riga.split("=", 1)[1]
            if riga.startswith("CodeDirectory") and "flags=" in riga:
                misura["hardened_runtime"] = "runtime" in riga
        misura["come"] = "codesign --display"
    else:
        misura["non_misurabile_qui"] = f"codesign ha fallito: {testo[:200]}"

    verifica = subprocess.run(
        ["codesign", "--verify", "--strict", str(percorso)], capture_output=True, text=True
    )
    misura["firma_valida"] = verifica.returncode == 0

    if archivio is not None:
        # `spctl --assess` su un archivio notarizzato ma non stapled interroga
        # il servizio: serve rete, ed e' esattamente la condizione che chi
        # installa incontrera' la prima volta.
        assess = subprocess.run(
            ["spctl", "--assess", "--type", "install", "--context", "context:primary-signature",
             "--verbose=4", str(archivio)],
            capture_output=True,
            text=True,
        )
        misura["notarizzato"] = assess.returncode == 0
        misura["spctl"] = (assess.stderr + assess.stdout)[:300]
    return misura


def chiave(versione: str) -> tuple[int, ...]:
    return tuple(int(x) for x in versione.split("."))


def rpath_esce_dall_albero(voce: str, profondita: int) -> bool:
    """Come su Linux, con `@loader_path` al posto di `$ORIGIN`."""
    resto = voce[len("@loader_path") :].strip("/")
    livello = profondita
    for segmento in resto.split("/"):
        if segmento in ("", "."):
            continue
        if segmento == "..":
            livello -= 1
            if livello < 0:
                return True
        else:
            livello += 1
    return False


def main() -> int:
    a = argparse.ArgumentParser(description=__doc__)
    a.add_argument("--albero", required=True, type=pathlib.Path)
    a.add_argument("--radice", default="bin/plenora-io")
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
            "il lock di macOS non porta un `contratto_di_verifica`. Va scritto misurando su un "
            "runner macOS: quello di Linux parla di ELF e di GLIBC, e qui le domande sono "
            "l'install name, l'`LC_RPATH` e il deployment target."
        )

    radice = albero / arg.radice
    if not radice.is_file():
        sys.exit(f"radice della chiusura assente: {radice}")

    errori: list[str] = []

    # La chiusura, per install name.
    da_visitare = [radice]
    interne: dict[str, pathlib.Path] = {}
    esterne: set[str] = set()
    assoluti: dict[str, list[str]] = {}
    while da_visitare:
        corrente = da_visitare.pop()
        for nome in dipendenze(corrente):
            if nome in interne or nome in esterne:
                continue
            if nome.startswith("@rpath/"):
                candidato = albero / "lib" / nome[len("@rpath/") :]
                if candidato.exists():
                    interne[nome] = candidato
                    da_visitare.append(candidato)
                else:
                    errori.append(f"{corrente.name}: «{nome}» non si risolve dentro l'albero")
                continue
            if nome.startswith(PREFISSI_DI_SISTEMA):
                esterne.add(nome)
                continue
            # Un install name assoluto che non sia di sistema e' il difetto che
            # su Linux aveva la faccia del `DT_NEEDED` assoluto: l'artefatto
            # funziona dove e' nato e altrove no.
            assoluti.setdefault(nome, []).append(corrente.name)

    spediti = [radice, *sorted(set(interne.values()))]
    print(f"chiusura da {arg.radice}: {len(interne)} librerie interne, {len(spediti)} Mach-O")

    if assoluti:
        errori.append(
            f"install name assoluti fuori da /usr/lib e dai framework: {sorted(assoluti)[:6]}. "
            "Un percorso assoluto non e' un nome da risolvere: e' una directory precisa, e "
            "l'artefatto smette di caricarsi appena non c'e' piu'."
        )

    attese = {n for n in contratto["dipendenze_di_sistema_attese"][manifesto["profilo"]]}
    if esterne != attese:
        errori.append(
            f"le dipendenze di sistema non coincidono con quelle attese. In piu': "
            f"{sorted(esterne - attese)}. In meno: {sorted(attese - esterne)}."
        )

    # Architettura: ARM64, e nessun binario universale.
    non_arm64 = []
    for m in spediti:
        try:
            if cpu_type(m) != CPU_TYPE_ARM64:
                non_arm64.append(m.name)
        except MachOMalformato as e:
            errori.append(f"{m.name}: {e}")
    if non_arm64:
        errori.append(f"Mach-O non ARM64: {non_arm64}")

    # Deployment target: vale il piu' alto fra tutti gli spediti.
    soglia = contratto["deployment_target"]
    massimo, per_file, senza = "0.0.0", {}, []
    for m in spediti:
        try:
            dichiarato = deployment_target(m)
        except MachOMalformato:
            continue
        if dichiarato is None:
            senza.append(m.name)
            continue
        per_file[m.name] = dichiarato
        if chiave(dichiarato) > chiave(massimo):
            massimo = dichiarato
    print(f"deployment target massimo: {massimo} (soglia {soglia})")
    if senza:
        errori.append(
            f"Mach-O senza deployment target dichiarato: {senza[:5]}. Senza il campo non si sa "
            "su quale sistema il binario pretenda di girare."
        )
    if chiave(massimo) > chiave(soglia):
        errori.append(
            f"il deployment target massimo e' {massimo}, oltre la soglia {soglia}. Un solo "
            "binario compilato piu' in alto alza il requisito dell'intero artefatto, e nulla "
            "nel nome lo direbbe."
        )

    # RPATH radicato in @loader_path e interno all'albero.
    difettosi = []
    for m in spediti:
        profondita = len(m.relative_to(albero).parts) - 1
        voci = rpath(m)
        if not voci and m != radice and interne:
            difettosi.append((m.name, "senza LC_RPATH"))
        for voce in voci:
            if not voce.startswith("@loader_path"):
                difettosi.append((m.name, f"non radicato in @loader_path: {voce}"))
            elif rpath_esce_dall_albero(voce, profondita):
                difettosi.append((m.name, f"esce dall'albero: {voce}"))
    print(f"Mach-O con LC_RPATH radicato e interno: {len(spediti) - len(difettosi)}/{len(spediti)}")
    if difettosi:
        errori.append(f"LC_RPATH non conformi: {difettosi[:5]}")

    if arg.referto:
        distribuzione.scrivi_referto(
            arg.referto,
            verifica="runtime",
            piattaforma=manifesto["piattaforma"],
            profilo=manifesto["profilo"],
            canale=manifesto["canale"],
            esito="verde" if not errori else "rosso",
            misure={
                "macho_spediti": len(spediti),
                "librerie_interne": len(interne),
                "dipendenze_di_sistema": sorted(esterne),
                "deployment_target_massimo": massimo,
                "deployment_target_per_file": per_file,
                "install_name_assoluti": sorted(assoluti),
                "rpath_conformi": len(spediti) - len(difettosi),
                # La forma e' comune: il gate finale confronta questi nomi.
                "elf_spediti": len(spediti),
                "dipendenze_esterne": sorted(esterne),
                "percorsi_assoluti_classificati": len(spediti) - len(assoluti),
            },
            errori=errori,
            note="verificatore nativo Mach-O: install name, LC_RPATH, deployment target",
        )

    if errori:
        print("\n--- ROSSO ---")
        for e in errori:
            print(f"  {e}")
        return 1
    print("\ntutte le verifiche sull'artefatto macOS sono verdi")
    return 0


if __name__ == "__main__":
    sys.exit(main())
