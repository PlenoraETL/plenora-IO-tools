#!/usr/bin/env python3
"""Le verifiche fail-closed sul runtime materializzato.

Ognuna risponde a una domanda che il nome di un pacchetto non risponde.

1. **Che cosa si spedisce davvero.** La chiusura `DT_NEEDED` a partire da una
   radice **data**, non dall'elenco dei pacchetti. La radice e' un argomento
   perche' alla fine dovra' essere `bin/plenora-io`: verificare la chiusura di
   `libgdal.so` dice che cosa serve a GDAL, non che cosa serve all'artefatto.

2. **Che cosa sta fuori dall'albero.** Ogni `DT_NEEDED` non risolto dentro il
   prefisso va classificato, e non basta che stia in una politica generosa:
   deve coincidere **esattamente** con l'insieme che il lock dichiara. Una
   politica sola lascerebbe passare la sparizione di una libreria spedita --
   `libstdc++` che smette di essere nell'albero e viene presa dal sistema resta
   verde, perche' `libstdc++` "e' una libreria di sistema ammissibile". I due
   insiemi servono a due cose diverse: la politica dice che cosa **potrebbe**
   essere legittimo, l'atteso dice che cosa **e'**.

3. **La soglia glibc.** Le versioni `GLIBC_*` pretese da **ogni** ELF spedito.

4. **I percorsi assoluti cotti dentro i binari.** Dopo la rilocazione di conda
   il placeholder sparisce e al suo posto c'e' il prefisso d'installazione --
   in `libgdal` per `share/gdal` e `lib/gdalplugins`, cioe' dati e plugin. Non
   sono stringhe inerti, e ritenerle tali perche' l'RPATH e' relativo era un
   falso verde: l'RPATH riguarda le librerie, non i dati. Ogni percorso va
   quindi **classificato**: o e' coperto da una variabile che l'artefatto
   imposta, o e' irrilevante per l'uso che l'artefatto fa della libreria, o e'
   inerte per costruzione. Cio' che non rientra in nessuna delle tre fa rosso.

5. **L'RPATH.** Non basta che contenga `$ORIGIN`: va **radicato** in `$ORIGIN`
   e, normalizzato, non deve poter uscire dall'albero installato.
"""

from __future__ import annotations

import argparse
import json
import pathlib
import re
import subprocess
import sys

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
import distribuzione  # noqa: E402 -- dopo sys.path, che e' il punto

RADICE = pathlib.Path(__file__).resolve().parent.parent
LOCK = RADICE / "scripts" / "linux-gdal-lock.json"

# La **politica**: che cosa e' ammissibile trovare fuori dall'albero, perche' il
# sistema di destinazione lo garantisce per ABI. Non e' l'elenco atteso: quello
# sta nel lock, ed e' un sottoinsieme di questo.
POLITICA_ABI = {
    "ld-linux-x86-64.so.2",
    "libc.so.6",
    "libdl.so.2",
    "libgcc_s.so.1",
    "libm.so.6",
    "libpthread.so.0",
    "libresolv.so.2",
    "librt.so.1",
    "libutil.so.1",
    "linux-vdso.so.1",
}


def leggi_dinamica(percorso: pathlib.Path) -> str:
    return subprocess.run(
        ["readelf", "-d", str(percorso)], capture_output=True, text=True, check=True
    ).stdout


def dt_needed(percorso: pathlib.Path) -> list[str]:
    return re.findall(r"\(NEEDED\)\s+Shared library: \[([^\]]+)\]", leggi_dinamica(percorso))


def rpath_di(percorso: pathlib.Path) -> list[str]:
    trovati = re.findall(
        r"\((?:RPATH|RUNPATH)\)\s+Library r(?:un)?path: \[([^\]]*)\]", leggi_dinamica(percorso)
    )
    return [x for gruppo in trovati for x in gruppo.split(":") if x]


def rpath_esce_dall_albero(voce: str, profondita_dalla_radice: int) -> bool:
    """L'RPATH, normalizzato, resta dentro l'albero installato?

    `$ORIGIN/../lib` da `bin/` resta dentro; `$ORIGIN/../../lib` no. Il conto si
    fa sui segmenti: ogni `..` risale, e risalire piu' della profondita' del
    file dentro l'albero significa uscirne.
    """
    resto = voce[len("$ORIGIN") :].strip("/")
    livello = profondita_dalla_radice
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


def versioni_glibc(percorso: pathlib.Path) -> set[str]:
    uscita = subprocess.run(
        ["readelf", "-V", str(percorso)], capture_output=True, text=True, check=True
    ).stdout
    return set(re.findall(r"GLIBC_([0-9]+(?:\.[0-9]+)+)", uscita))


def chiave(versione: str) -> tuple[int, ...]:
    return tuple(int(x) for x in versione.split("."))


def percorsi_assoluti(elf: pathlib.Path, prefisso: str) -> set[str]:
    uscita = subprocess.run(
        ["strings", "-a", str(elf)], capture_output=True, text=True, check=True
    ).stdout
    trovati = set()
    for riga in uscita.splitlines():
        for m in re.finditer(re.escape(prefisso) + r"(/[^\s\"']*)?", riga):
            trovati.add(m.group(0))
    return trovati


def classifica(percorso: str, prefisso: str, regole: list[dict]) -> dict | None:
    relativo = percorso[len(prefisso) :] or "/"
    for regola in regole:
        if re.fullmatch(regola["schema"], relativo):
            return regola
    return None


def chiusura(
    radice: pathlib.Path, prefisso: pathlib.Path
) -> tuple[dict[str, pathlib.Path], set[str]]:
    """La chiusura `DT_NEEDED` da un ELF, divisa fra interna ed esterna.

    Interna e' cio' che si risolve dentro `<prefisso>/lib`, cioe' cio' che
    l'artefatto porta con se'; esterna e' tutto il resto, cioe' cio' che
    pretende dalla macchina che lo ospita.

    Vive qui, e non in chi assembla l'albero, perche' assemblare e verificare
    devono partire dalla **stessa** lettura. Due implementazioni della stessa
    domanda divergono, e divergerebbero esattamente fra le due parti che devono
    essere d'accordo: chi decide che cosa spedire e chi controlla che cio' che
    e' stato spedito basti.
    """
    da_visitare = [radice]
    interne: dict[str, pathlib.Path] = {}
    esterne: set[str] = set()
    while da_visitare:
        for nome in dt_needed(da_visitare.pop()):
            if nome in interne or nome in esterne:
                continue
            candidato = prefisso / "lib" / nome
            if candidato.exists():
                risolta = candidato.resolve()
                interne[nome] = risolta
                da_visitare.append(risolta)
            else:
                esterne.add(nome)
    return interne, esterne


def main() -> int:
    argomenti = argparse.ArgumentParser(description=__doc__)
    argomenti.add_argument("--prefisso", required=True, type=pathlib.Path)
    argomenti.add_argument("--referto", type=pathlib.Path, default=None)
    argomenti.add_argument(
        "--prefisso-di-costruzione",
        default=None,
        type=pathlib.Path,
        help=(
            "il prefisso in cui il runtime e' stato materializzato, quando si verifica un albero "
            "gia' assemblato. I percorsi assoluti cotti nei binari nominano **quello**, non "
            "l'albero: cercare la stringa sbagliata non trova niente e stampa un verde"
        ),
    )
    argomenti.add_argument(
        "--radice",
        default="lib/libgdal.so",
        help="l'ELF da cui parte la chiusura, relativo al prefisso; alla fine sara' bin/plenora-io",
    )
    opzioni = argomenti.parse_args()

    prefisso: pathlib.Path = opzioni.prefisso.resolve()
    lock = json.loads(LOCK.read_text(encoding="utf-8"))
    contratto = lock["contratto_di_verifica"]
    errori: list[str] = []

    manifesto = prefisso / "MANIFEST.json"

    radice = (prefisso / opzioni.radice).resolve()
    if not radice.exists():
        sys.exit(f"radice della chiusura assente: {radice}")

    # --- 1. la chiusura ----------------------------------------------------
    interne, esterne = chiusura(radice, prefisso)
    spediti = [radice, *sorted(set(interne.values()))]
    print(f"chiusura da {opzioni.radice}: {len(interne)} dipendenze interne, {len(spediti)} ELF")

    # --- 2. esterne: politica **e** atteso ---------------------------------
    # Le attese sono per profilo, perche' i due profili sono due prodotti. Il
    # profilo lo dichiara l'artefatto; su un runtime appena materializzato --
    # dove non c'e' un manifesto -- la domanda e' quella del profilo completo.
    profilo = "filegdb"
    if manifesto.is_file():
        profilo = json.loads(manifesto.read_text(encoding="utf-8")).get("profilo", "filegdb")
    attese = set(contratto["dipendenze_esterne_attese"][profilo])
    print(f"profilo verificato: {profilo}")
    fuori_politica = sorted(esterne - POLITICA_ABI)
    if fuori_politica:
        errori.append(
            f"dipendenze fuori dalla politica ABI: {fuori_politica}. O sono spedite dentro "
            "l'albero, o la politica va allargata con una ragione."
        )
    if esterne != attese:
        errori.append(
            f"le dipendenze esterne non coincidono con quelle attese dal lock. "
            f"In piu': {sorted(esterne - attese)}. In meno: {sorted(attese - esterne)}. "
            "Una libreria che smette di essere spedita e viene presa dal sistema resta dentro la "
            "politica, e solo l'insieme atteso se ne accorge: ogni variazione vuole un lock nuovo."
        )
    print(f"dipendenze esterne: {len(esterne)}, attese {len(attese)}")

    # --- 3. glibc ----------------------------------------------------------
    massima, per_elf = "0.0", {}
    for elf in spediti:
        versioni = versioni_glibc(elf)
        if not versioni:
            continue
        alta = max(versioni, key=chiave)
        per_elf[elf.name] = alta
        if chiave(alta) > chiave(massima):
            massima = alta
    soglia = contratto["glibc_massima_ammessa"]
    print(f"GLIBC massima negli ELF spediti: {massima} (soglia {soglia})")
    if chiave(massima) > chiave(soglia):
        errori.append(
            f"la chiusura pretende GLIBC_{massima}, oltre la soglia {soglia}: il contratto "
            "Linux va rinegoziato, non abbassato in silenzio."
        )

    # --- 3b. nessun DT_NEEDED assoluto -------------------------------------
    #
    # Un `DT_NEEDED` con un percorso assoluto non e' un nome da risolvere: e'
    # una directory precisa, e l'RPATH non la governa. Una libreria che ne porti
    # uno smette di caricarsi appena quella directory non esiste piu', cioe'
    # ovunque tranne che sulla macchina che l'ha costruita.
    #
    # `libgdal.so.35` di conda-forge ne portava uno -- il placeholder del
    # prefisso, riscritto dalla rilocazione -- e il costruttore ora lo
    # normalizza. Questo controllo e' cio' che rende quella normalizzazione un
    # fatto verificato invece di un passo che si spera sia avvenuto, ed e'
    # anche cio' che permette di classificare come inerte la stringa che
    # `patchelf` lascia orfana nella tabella delle stringhe.
    assoluti_dichiarati = {
        elf.name: [n for n in dt_needed(elf) if n.startswith("/")] for elf in spediti
    }
    con_assoluti = {k: v for k, v in assoluti_dichiarati.items() if v}
    if con_assoluti:
        errori.append(
            f"{len(con_assoluti)} ELF dichiarano dipendenze per percorso assoluto: "
            f"{sorted(con_assoluti)[:6]}. Un percorso assoluto non e' risolto dall'RPATH, e "
            "l'artefatto smetterebbe di caricarsi fuori dalla macchina che l'ha costruito."
        )
    print(f"ELF con DT_NEEDED assoluti: {len(con_assoluti)} su {len(spediti)}")

    # --- 4. i percorsi assoluti cotti dentro -------------------------------
    # Due prefissi, e confonderli e' un falso verde gia' capitato: `prefisso` e'
    # dove i file **stanno adesso**, e serve a risolvere la chiusura;
    # `prefisso_di_costruzione` e' cio' che i binari **nominano dentro di se'**.
    # Su un runtime appena materializzato coincidono, e per questo la confusione
    # non si vede; su un albero assemblato altrove no, e allora il controllo dei
    # percorsi assoluti non trova nulla e conclude che non ce ne sono.
    # L'ordine e' voluto: cio' che si passa a mano vince, perche' serve a
    # indagare; poi cio' che l'artefatto ha registrato di se'; poi il prefisso
    # stesso, che e' il caso del runtime appena materializzato.
    dal_manifesto = None
    if manifesto.is_file():
        dal_manifesto = json.loads(manifesto.read_text(encoding="utf-8")).get(
            "prefisso_di_costruzione"
        )
    if opzioni.prefisso_di_costruzione:
        prefisso_di_costruzione = opzioni.prefisso_di_costruzione.resolve()
    elif dal_manifesto:
        prefisso_di_costruzione = pathlib.Path(dal_manifesto)
        print(f"prefisso di costruzione, dal manifesto dell'artefatto: {prefisso_di_costruzione}")
    else:
        prefisso_di_costruzione = prefisso
    testo_prefisso = str(prefisso_di_costruzione)
    regole = contratto["percorsi_assoluti_ammessi"]
    non_classificati: dict[str, list[str]] = {}
    per_categoria: dict[str, int] = {}
    for elf in spediti:
        for percorso in percorsi_assoluti(elf, testo_prefisso):
            regola = classifica(percorso, testo_prefisso, regole)
            if regola is None:
                non_classificati.setdefault(percorso.replace(testo_prefisso, "<PREFISSO>"), []).append(
                    elf.name
                )
            else:
                per_categoria[regola["categoria"]] = per_categoria.get(regola["categoria"], 0) + 1
    print("percorsi assoluti cotti nei binari, per categoria:")
    for categoria, quanti in sorted(per_categoria.items()):
        print(f"  {categoria:22s} {quanti}")
    if not per_categoria and not non_classificati:
        # Zero non e' un buon esito: e' il sintomo di aver cercato la stringa
        # sbagliata. La rilocazione di conda **sostituisce** il placeholder con
        # il prefisso, quindi qualche percorso c'e' sempre, e non trovarne
        # nessuno significa che `testo_prefisso` non e' quello che i binari
        # nominano. Verificando un albero assemblato altrove capita per
        # costruzione, e senza questa riga il controllo stampava un verde.
        errori.append(
            "nessun percorso assoluto trovato negli ELF spediti. Dopo la rilocazione di conda "
            f"qualcuno ce n'e' sempre, quindi «{testo_prefisso}» non e' il prefisso che i binari "
            "nominano: passare --prefisso-di-costruzione."
        )
    if non_classificati:
        errori.append(
            f"{len(non_classificati)} percorsi assoluti non classificati: "
            f"{sorted(non_classificati)[:6]}. Ognuno va dichiarato: coperto da una variabile che "
            "l'artefatto imposta, irrilevante per l'uso che ne fa, o inerte per costruzione."
        )

    # --- 5. RPATH radicato e che non esce ----------------------------------
    difettosi = []
    for elf in spediti:
        try:
            profondita = len(elf.relative_to(prefisso).parts) - 1
        except ValueError:
            profondita = 1
        voci = rpath_di(elf)
        if not voci:
            difettosi.append((elf.name, "senza RPATH"))
            continue
        for voce in voci:
            if not voce.startswith("$ORIGIN"):
                difettosi.append((elf.name, f"non radicato in $ORIGIN: {voce}"))
            elif rpath_esce_dall_albero(voce, profondita):
                difettosi.append((elf.name, f"esce dall'albero: {voce}"))
    print(f"ELF con RPATH radicato in $ORIGIN e interno all'albero: {len(spediti) - len(difettosi)}/{len(spediti)}")
    if difettosi:
        errori.append(f"RPATH non conformi: {difettosi[:5]}")

    (prefisso / "verifica-runtime.json").write_text(
        json.dumps(
            {
                "radice": opzioni.radice,
                "elf_spediti": [e.name for e in spediti],
                "dipendenze_interne": len(interne),
                "dipendenze_esterne": sorted(esterne),
                "glibc_massima": massima,
                "glibc_per_elf": per_elf,
                "percorsi_assoluti_per_categoria": per_categoria,
                "percorsi_assoluti_non_classificati": non_classificati,
            },
            ensure_ascii=False,
            indent=2,
        )
        + "\n",
        encoding="utf-8",
    )

    # Il referto nel formato comune, che il gate finale riconta. E' un documento
    # diverso da `verifica-runtime.json`: quello resta accanto al prefisso ed e'
    # il **dettaglio** della misura -- ogni ELF, ogni versione GLIBC -- mentre
    # questo porta i pochi numeri che il gate confronta, nella forma che le tre
    # piattaforme hanno in comune.
    if opzioni.referto and manifesto.is_file():
        dichiarazione = json.loads(manifesto.read_text(encoding="utf-8"))
        distribuzione.scrivi_referto(
            opzioni.referto,
            verifica="runtime",
            piattaforma=dichiarazione["piattaforma"],
            profilo=dichiarazione["profilo"],
            canale=dichiarazione["canale"],
            esito="verde" if not errori else "rosso",
            misure={
                "elf_spediti": len(spediti),
                "dipendenze_interne": len(interne),
                "dipendenze_esterne": sorted(esterne),
                "glibc_massima": massima,
                "percorsi_assoluti_classificati": sum(per_categoria.values()),
                "percorsi_assoluti_non_classificati": len(non_classificati),
                "elf_con_dt_needed_assoluti": len(con_assoluti),
                "rpath_conformi": len(spediti) - len(difettosi),
            },
            errori=errori,
        )

    if errori:
        print("\n--- ROSSO ---")
        for e in errori:
            print(f"  {e}")
        return 1
    print("\ntutte le verifiche sul runtime materializzato sono verdi")
    return 0


if __name__ == "__main__":
    sys.exit(main())
