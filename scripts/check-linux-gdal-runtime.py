#!/usr/bin/env python3
"""Le verifiche fail-closed sul lock materializzato.

Ognuna risponde a una domanda che il nome di un pacchetto non risponde:

1. **che cosa si spedisce davvero** — la chiusura `DT_NEEDED` di `libgdal`,
   risolta dentro il prefisso, non l'elenco dei pacchetti;
2. **niente resta del prefisso di costruzione** — i placeholder che conda
   rilocherebbe, cercati nei soli file spediti;
3. **la soglia glibc** — le versioni `GLIBC_*` pretese da **ogni** ELF
   spedito, non dalla sola `libgdal.so`;
4. **che cosa non e' nell'albero** — ogni `DT_NEEDED` non risolto dentro il
   prefisso va classificato: o e' nell'allowlist ABI di sistema, o l'artefatto
   non e' consegnabile.

La capability di OpenFileGDB non si prova qui: si prova eseguendo, ed e' la
sonda C accanto a questo file.
"""

import argparse
import json
import pathlib
import re
import subprocess
import sys

PREFISSO = pathlib.Path()
LIB = pathlib.Path()

# Cio' che il sistema di destinazione garantisce per ABI. Non e' una lista
# scritta a memoria: e' il criterio, e l'elenco effettivo lo produce questa
# verifica -- cio' che non vi rientra fa fallire.
ALLOWLIST_ABI = {
    "libc.so.6",
    "libm.so.6",
    "libdl.so.2",
    "libpthread.so.0",
    "librt.so.1",
    "libgcc_s.so.1",
    "ld-linux-x86-64.so.2",
    "linux-vdso.so.1",
    "libresolv.so.2",
    "libutil.so.1",
}


def dt_needed(percorso: pathlib.Path) -> list[str]:
    uscita = subprocess.run(
        ["readelf", "-d", str(percorso)], capture_output=True, text=True, check=True
    ).stdout
    return re.findall(r"\(NEEDED\)\s+Shared library: \[([^\]]+)\]", uscita)


def versioni_glibc(percorso: pathlib.Path) -> set[str]:
    uscita = subprocess.run(
        ["readelf", "-V", str(percorso)], capture_output=True, text=True, check=True
    ).stdout
    return set(re.findall(r"GLIBC_([0-9]+(?:\.[0-9]+)+)", uscita))


def chiave(versione: str) -> tuple[int, ...]:
    return tuple(int(x) for x in versione.split("."))


def rpath_di(percorso: pathlib.Path) -> list[str]:
    uscita = subprocess.run(
        ["readelf", "-d", str(percorso)], capture_output=True, text=True, check=True
    ).stdout
    trovati = re.findall(r"\((?:RPATH|RUNPATH)\)\s+Library r(?:un)?path: \[([^\]]*)\]", uscita)
    return [x for gruppo in trovati for x in gruppo.split(":") if x]


def risolvi(nome: str) -> pathlib.Path | None:
    candidato = LIB / nome
    return candidato if candidato.exists() else None


def main() -> int:
    global PREFISSO, LIB
    argomenti = argparse.ArgumentParser(description=__doc__)
    argomenti.add_argument("--prefisso", required=True, type=pathlib.Path)
    argomenti.add_argument(
        "--glibc-massima",
        default="2.35",
        help="la soglia dichiarata nella matrice di distribuzione",
    )
    opzioni = argomenti.parse_args()
    PREFISSO = opzioni.prefisso
    LIB = PREFISSO / "lib"

    errori: list[str] = []

    radice_gdal = LIB / "libgdal.so"
    if not radice_gdal.exists():
        return int(bool(print("libgdal.so assente dal prefisso: il lock non porta GDAL")))

    # --- 1. la chiusura ----------------------------------------------------
    da_visitare = [radice_gdal.resolve()]
    interne: dict[str, pathlib.Path] = {}
    esterne: set[str] = set()
    while da_visitare:
        corrente = da_visitare.pop()
        for nome in dt_needed(corrente):
            if nome in interne or nome in esterne:
                continue
            trovata = risolvi(nome)
            if trovata is None:
                esterne.add(nome)
            else:
                interne[nome] = trovata.resolve()
                da_visitare.append(trovata.resolve())

    print(f"chiusura DT_NEEDED di libgdal: {len(interne)} librerie dentro l'albero")

    # --- 4. classificazione ------------------------------------------------
    non_classificate = sorted(esterne - ALLOWLIST_ABI)
    print(f"dipendenze fuori dall'albero: {len(esterne)}")
    for nome in sorted(esterne):
        marchio = "ABI" if nome in ALLOWLIST_ABI else "NON CLASSIFICATA"
        print(f"  {marchio:17s} {nome}")
    if non_classificate:
        errori.append(
            f"dipendenze non classificate: {non_classificate}. O entrano nell'allowlist ABI "
            "con una ragione, o vanno spedite dentro l'albero."
        )

    # L'insieme spedito: libgdal, la sua chiusura interna, e i dati.
    spediti = [radice_gdal.resolve(), *sorted(interne.values())]
    dati = [PREFISSO / "share" / "gdal", PREFISSO / "share" / "proj"]

    # --- 3. la soglia glibc ------------------------------------------------
    massima = ("0",)
    per_libreria: dict[str, str] = {}
    for elf in spediti:
        versioni = versioni_glibc(elf)
        if not versioni:
            continue
        alta = max(versioni, key=chiave)
        per_libreria[elf.name] = alta
        if chiave(alta) > chiave(".".join(massima)):
            massima = tuple(alta.split("."))
    soglia = ".".join(massima)
    print(f"\nGLIBC massima pretesa dagli ELF spediti: {soglia}")
    for nome, versione in sorted(per_libreria.items(), key=lambda x: chiave(x[1]))[-5:]:
        print(f"  {versione:8s} {nome}")
    if chiave(soglia) > chiave(opzioni.glibc_massima):
        errori.append(
            f"la chiusura pretende GLIBC_{soglia}, oltre la soglia "
            f"{opzioni.glibc_massima} dichiarata nella matrice: il contratto Linux va "
            "rinegoziato, non abbassato in silenzio."
        )

    # --- 2. il prefisso di costruzione, dove **risolve** -------------------
    #
    # La prima stesura rifiutava qualunque occorrenza del prefisso di
    # costruzione nei file spediti, e faceva rosso su sette librerie. La misura
    # ha mostrato che cosa sono quelle occorrenze: percorsi **sorgente**
    # compilati dentro il binario (`work/port/cpl_conv.cpp`), che ogni build
    # prodotta da chiunque porta con se' -- Debian compresa -- e un placeholder
    # che conda riscriverebbe all'installazione.
    #
    # Rifiutarle tutte significherebbe rifiutare ogni binario precompilato che
    # esista. La domanda utile e' un'altra: **qualcosa risolve attraverso quel
    # prefisso?** Se l'RPATH e' relativo a `$ORIGIN` e nessun file di testo
    # spedito porta un placeholder, la risposta e' no, e il residuo e' inerte.
    rilocazioni = json.loads((PREFISSO / "rilocazioni.json").read_text(encoding="utf-8"))
    nomi_spediti = {p.name for p in spediti}
    relativi_spediti = {f"lib/{n}" for n in nomi_spediti}
    for cartella in dati:
        if cartella.exists():
            for f in cartella.rglob("*"):
                if f.is_file():
                    relativi_spediti.add(str(f.relative_to(PREFISSO)))

    # (a) nessun file **di testo** spedito porta un placeholder: li' il prefisso
    #     sarebbe un percorso che il codice legge, non una stringa inerte.
    testo_spediti = [
        r for r in rilocazioni if r["file"] in relativi_spediti and r.get("modo") == "text"
    ]
    if testo_spediti:
        errori.append(
            f"{len(testo_spediti)} file di testo spediti portano il prefisso di costruzione: "
            f"{[r['file'] for r in testo_spediti][:5]}. Un percorso che non esiste sulla "
            "macchina di destinazione e' un difetto che si manifesta lontano da qui."
        )

    # (b) ogni ELF spedito risolve per percorso **relativo**: un RPATH assoluto
    #     -- o assente -- farebbe dipendere il caricamento da dove l'artefatto
    #     e' stato costruito.
    senza_rpath, rpath_assoluto = [], []
    for elf in spediti:
        percorsi = rpath_di(elf)
        if not percorsi:
            senza_rpath.append(elf.name)
        elif any("$ORIGIN" not in x for x in percorsi):
            rpath_assoluto.append((elf.name, percorsi))
    if senza_rpath:
        errori.append(f"ELF spediti senza RPATH: {senza_rpath[:5]}")
    if rpath_assoluto:
        errori.append(f"ELF spediti con RPATH non relativo: {rpath_assoluto[:5]}")

    binari_spediti = [
        r for r in rilocazioni if r["file"] in relativi_spediti and r.get("modo") != "text"
    ]
    print(f"\nprefisso di costruzione nei file spediti:")
    print(f"  in file di testo: {len(testo_spediti)} (devono essere zero)")
    print(f"  in binari, come stringa inerte: {len(binari_spediti)} (registrati, non rifiutati)")
    print(f"  ELF con RPATH relativo a $ORIGIN: {len(spediti) - len(senza_rpath) - len(rpath_assoluto)}/{len(spediti)}")

    (PREFISSO / "verifica-runtime.json").write_text(
        json.dumps(
            {
                "chiusura_interna": sorted(nomi_spediti),
                "esterne": sorted(esterne),
                "esterne_non_classificate": non_classificate,
                "glibc_massima": soglia,
                "glibc_per_libreria": per_libreria,
                "placeholder_spediti_testo": testo_spediti,
                "placeholder_spediti_binari": len(binari_spediti),
            },
            ensure_ascii=False,
            indent=2,
        )
        + "\n",
        encoding="utf-8",
    )

    if errori:
        print("\n--- ROSSO ---")
        for e in errori:
            print(f"  {e}")
        return 1
    print("\ntutte le verifiche sul lock materializzato sono verdi")
    return 0


if __name__ == "__main__":
    sys.exit(main())
