#!/usr/bin/env python3
"""Le due verifiche del relocation smoke che non si fanno con un `test`.

Stanno in un file, e non dentro lo script che le invoca, perche' hanno bisogno
di leggere JSON e di confrontare insiemi -- e perche' un controllo che vive
dentro un heredoc non si esegue da solo, quindi non si prova che sappia
diventare rosso.
"""

from __future__ import annotations

import json
import os
import pathlib
import re
import sys

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
import distribuzione  # noqa: E402 -- dopo sys.path, che e' il punto

RADICE = pathlib.Path(__file__).resolve().parent.parent
CHECKER = RADICE / "scripts" / "check-linux-gdal-runtime.py"


def rilettura(percorso: pathlib.Path, profilo: str = "filegdb") -> int:
    """Cio' che e' stato riletto porta schema, geometria e -- dove serve -- CRS.

    Non basta che il comando sia uscito con zero: un dataset vuoto uscirebbe
    con zero. Il CRS si pretende dal profilo `filegdb`, perche' li' e' cio' che
    dimostra che PROJ ha trovato le proprie griglie; il profilo base non spedisce
    PROJ, e pretenderlo da lui sarebbe chiedergli una capability che non ha mai
    promesso.
    """
    testo = json.dumps(json.loads(percorso.read_text(encoding="utf-8")), ensure_ascii=False)
    mancanti = [atteso for atteso in ("nome", "geometry") if atteso not in testo]
    if mancanti:
        print(f"ROSSO: assenti dallo schema riletto: {mancanti}", file=sys.stderr)
        return 1
    if profilo != "filegdb":
        print("   riletto: schema e geometria presenti (profilo senza PROJ)")
        return 0
    if "4326" not in testo:
        print(
            "ROSSO: il CRS non ha attraversato PROJ: EPSG:4326 non e' stato riletto",
            file=sys.stderr,
        )
        return 1
    print("   riletto: schema, geometria e CRS 4326 presenti")
    return 0


def politica_abi() -> set[str]:
    """L'allowlist ABI, letta dal controllo che la definisce.

    Ricopiarla qui sarebbe una seconda verita', e le due divergerebbero proprio
    quando conta: quando qualcuno allarga la politica di la' e questo smoke
    continua a pretendere quella di prima.
    """
    sorgente = CHECKER.read_text(encoding="utf-8")
    blocco = sorgente.split("POLITICA_ABI = {")[1].split("}")[0]
    return set(re.findall(r'"([^"]+)"', blocco))


def librerie(mappa: pathlib.Path, albero: str, referto: pathlib.Path | None = None) -> int:
    """Ogni libreria fuori dall'allowlist ABI e' stata caricata da B."""
    testo = mappa.read_text(errors="replace")
    # I percorsi che il loader stampa passano per l'RPATH cosi' com'e':
    # `bin/../lib/././libbz2.so.1.0`. Confrontarli con il prefisso dell'albero
    # senza normalizzarli non troverebbe mai una corrispondenza, e la verifica
    # direbbe che tutto viene da fuori.
    caricate = {
        os.path.normpath(p) for p in re.findall(r"calling init:\s+(\S+)", testo)
    }
    if not caricate:
        # Zero non e' un buon esito: e' il sintomo che `LD_DEBUG` non ha
        # prodotto nulla, e allora questa verifica non ha guardato niente.
        print(
            "ROSSO: nessuna libreria tracciata. Senza la mappa del loader questa "
            "verifica sarebbe un verde che non ha guardato niente.",
            file=sys.stderr,
        )
        return 1
    politica = politica_abi()
    fuori = [
        percorso
        for percorso in sorted(caricate)
        if percorso.rsplit("/", 1)[-1] not in politica and not percorso.startswith(albero)
    ]
    dall_albero = [p for p in sorted(caricate) if p.startswith(albero)]
    if referto is not None:
        manifesto = json.loads(
            (pathlib.Path(albero) / "MANIFEST.json").read_text(encoding="utf-8")
        )
        distribuzione.scrivi_referto(
            referto,
            verifica="relocation",
            piattaforma=manifesto["piattaforma"],
            profilo=manifesto["profilo"],
            canale=manifesto["canale"],
            esito="verde" if not fuori else "rosso",
            misure={
                "librerie_tracciate": len(caricate),
                "librerie_dall_albero": len(dall_albero),
                "librerie_fuori_dall_albero": fuori,
            },
            errori=[f"caricata fuori dall'albero: {f}" for f in fuori],
            note=(
                "dimostra i percorsi **effettivamente attraversati**. I percorsi TLS, XML, "
                "terminfo e Kerberos che lo smoke non esercita restano governati dalla loro "
                "classificazione strutturale: questo verde non li promuove."
            ),
        )
    if fuori:
        print("ROSSO: librerie non di sistema caricate fuori dall'artefatto:", file=sys.stderr)
        for f in fuori:
            print(f"   {f}", file=sys.stderr)
        return 1
    print(
        f"   {len(caricate)} librerie tracciate, {len(dall_albero)} dall'albero; nessuna "
        "fuori oltre l'allowlist ABI"
    )
    return 0


def main() -> int:
    if len(sys.argv) < 3:
        print(__doc__, file=sys.stderr)
        return 2
    comando = sys.argv[1]
    if comando == "rilettura":
        profilo = sys.argv[3] if len(sys.argv) > 3 else "filegdb"
        return rilettura(pathlib.Path(sys.argv[2]), profilo)
    if comando == "librerie":
        referto = pathlib.Path(sys.argv[4]) if len(sys.argv) > 4 else None
        return librerie(pathlib.Path(sys.argv[2]), sys.argv[3], referto)
    print(f"comando sconosciuto: {comando}", file=sys.stderr)
    return 2


if __name__ == "__main__":
    sys.exit(main())
