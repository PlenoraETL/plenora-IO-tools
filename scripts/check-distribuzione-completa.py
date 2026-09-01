#!/usr/bin/env python3
"""Il gate finale: sei artefatti, e i referti che li dimostrano.

# Perche' esiste

Perche' «il job e' verde» non e' un'evidenza verificabile. Un job verde e'
un'affermazione fatta da chi doveva essere verificato, e le affermazioni si
possono sbagliare in silenzio: un passo saltato per una condizione mai vera, un
`|| true` di troppo, uno smoke che non ha trovato l'artefatto e non ha guardato
niente. Nessuna di queste cose fa rosso, e tutte producono un job verde.

Questo gate riconta. Non chiede a nessuno com'e' andata: legge i referti, e
pretende che ci siano **tutti** quelli che devono esserci, che ciascuno porti le
misure che quella verifica deve produrre, e che le misure dicano cio' che il
contratto promette.

# La matrice

Tre piattaforme per due profili sono sei artefatti, e ogni artefatto vuole le
sue verifiche. Un referto mancante non e' un'omissione da tollerare: e' la
differenza fra «verificato» e «non verificato», e sono le due cose che questo
gate esiste per distinguere.

# Il profilo base non e' un artefatto minore

`base` promette che FileGDB **manchi**, e quella promessa si dimostra come
l'altra. Un gate che pretendesse le prove solo dal profilo pieno lascerebbe
passare un `base` costruito per sbaglio con la feature attiva: piu' grande di
sessanta megabyte, con una superficie e una licenza che chi lo installa non ha
accettato, e con lo stesso nome.
"""

from __future__ import annotations

import argparse
import json
import pathlib
import sys

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
import distribuzione  # noqa: E402 -- dopo sys.path, che e' il punto

PIATTAFORME = ("linux-x86_64", "windows-x86_64", "macos-aarch64")
PROFILI = ("base", "filegdb")

# Le verifiche che ogni artefatto deve portare, e la misura che ciascuna deve
# produrre. La misura conta piu' dell'esito: l'esito e' una conclusione, e una
# conclusione si puo' sbagliare in silenzio; una misura o c'e' e torna, o no.
VERIFICHE_ATTESE = {
    "runtime": {
        "misure_obbligatorie": ("elf_spediti", "dipendenze_esterne", "percorsi_assoluti_classificati"),
        "solo_profilo": "filegdb",
        "perche": "la chiusura, le dipendenze fuori dall'albero e i percorsi cotti dentro",
        "perche_solo_quel_profilo": "il profilo base non spedisce librerie: non c'e' una chiusura",
    },
    "licenze-artefatto": {
        "misure_obbligatorie": ("componenti_con_testo",),
        "perche": "ogni componente che spedisce byte porta il testo della propria licenza",
    },
    "smoke-profilo": {
        "misure_obbligatorie": (),
        "perche": "che cosa l'artefatto **installato** sa fare, e che cosa deve non saper fare",
    },
    "relocation": {
        "misure_obbligatorie": ("librerie_dall_albero",),
        "solo_profilo": "filegdb",
        "perche": "l'artefatto funziona dove non e' stato costruito",
        "perche_solo_quel_profilo": "il profilo base non spedisce librerie da rilocare",
    },
}

# La misura che deve dire una cosa precisa, e non solo esistere.
PRETESE_SULLE_MISURE = {
    ("smoke-profilo", "base"): (
        "filegdb_assente",
        True,
        "il profilo base deve **dimostrare** che FileGDB manca, non solo non usarlo",
    ),
    ("smoke-profilo", "filegdb"): (
        "schema_riletto",
        True,
        "il profilo filegdb deve scrivere e rileggere un FileGDB vero",
    ),
}


def attese_per(profilo: str) -> set[str]:
    return {
        nome
        for nome, regola in VERIFICHE_ATTESE.items()
        if regola.get("solo_profilo") in (None, profilo)
    }


def carica_referti(directory: pathlib.Path) -> tuple[dict, list[str]]:
    """I referti, indicizzati per (piattaforma, profilo, verifica)."""
    referti: dict[tuple[str, str, str], dict] = {}
    errori: list[str] = []
    for percorso in sorted(directory.rglob("*.json")):
        try:
            d = json.loads(percorso.read_text(encoding="utf-8"))
        except json.JSONDecodeError as e:
            errori.append(f"{percorso.name}: non e' JSON ({e})")
            continue
        if "schema_referto" not in d:
            continue  # non e' un referto: un manifesto, un SBOM, altro
        if d["schema_referto"] != distribuzione.SCHEMA_REFERTO:
            errori.append(
                f"{percorso.name}: schema {d['schema_referto']}, atteso "
                f"{distribuzione.SCHEMA_REFERTO}. Un referto di un'altra versione non si "
                "riconta: si rifa'."
            )
            continue
        chiave = (d["piattaforma"], d["profilo"], d["verifica"])
        if chiave in referti:
            errori.append(
                f"due referti per {chiave}: non si sa quale valga, e sceglierne uno sarebbe "
                "una decisione presa in silenzio."
            )
        referti[chiave] = d
    return referti, errori


def verifica(directory: pathlib.Path, canale: str, piattaforme: tuple[str, ...]) -> list[str]:
    referti, errori = carica_referti(directory)

    for piattaforma in piattaforme:
        for profilo in PROFILI:
            for nome in sorted(attese_per(profilo)):
                chiave = (piattaforma, profilo, nome)
                referto = referti.get(chiave)
                if referto is None:
                    errori.append(
                        f"{piattaforma}/{profilo}: manca il referto «{nome}». "
                        f"{VERIFICHE_ATTESE[nome]['perche']}. Un referto assente non e' "
                        "un'omissione da tollerare: e' la differenza fra verificato e non."
                    )
                    continue
                if referto["esito"] != "verde":
                    errori.append(
                        f"{piattaforma}/{profilo}/{nome}: esito «{referto['esito']}» "
                        f"({'; '.join(referto.get('errori') or [])[:200]})"
                    )
                if referto["canale"] != canale:
                    errori.append(
                        f"{piattaforma}/{profilo}/{nome}: referto del canale "
                        f"«{referto['canale']}», atteso «{canale}». Un artefatto di prova non "
                        "qualifica una candidate."
                    )
                misure = referto.get("misure") or {}
                for misura in VERIFICHE_ATTESE[nome]["misure_obbligatorie"]:
                    if misura not in misure:
                        errori.append(
                            f"{piattaforma}/{profilo}/{nome}: la misura «{misura}» non c'e'. "
                            "Un esito verde senza la misura che lo sostiene e' un'affermazione."
                        )
                pretesa = PRETESE_SULLE_MISURE.get((nome, profilo))
                if pretesa:
                    chiave_misura, atteso, perche = pretesa
                    if misure.get(chiave_misura) != atteso:
                        errori.append(
                            f"{piattaforma}/{profilo}/{nome}: «{chiave_misura}» vale "
                            f"{misure.get(chiave_misura)!r}, atteso {atteso!r}. {perche}."
                        )

    # La firma: se il canale la pretende su quella piattaforma, dev'esserci.
    for piattaforma in piattaforme:
        politica = distribuzione.politica_di_firma(piattaforma, canale)
        if not politica["richiesta"]:
            continue
        for profilo in PROFILI:
            referto = referti.get((piattaforma, profilo, "smoke-profilo"))
            if referto is None:
                continue  # gia' segnalato sopra
            firma = (referto.get("misure") or {}).get("firma", {})
            if firma.get("stato") != "apposta":
                errori.append(
                    f"{piattaforma}/{profilo}: il canale «{canale}» pretende una firma "
                    f"{politica['meccanismo']} e il referto dice «{firma.get('stato')}». "
                    "Un artefatto candidate non firmato non e' una candidate meno buona: "
                    "e' un artefatto che chi lo riceve non puo' verificare."
                )
            if firma.get("smoke_prima_della_firma"):
                errori.append(
                    f"{piattaforma}/{profilo}: lo smoke e' stato eseguito prima della firma. "
                    "Un binario firmato -- notarizzato e con lo stapling su macOS, con una "
                    "sezione in piu' su Windows -- e' un altro file: lo smoke va rifatto."
                )
    return errori


def main() -> int:
    a = argparse.ArgumentParser(description=__doc__)
    a.add_argument("--referti", required=True, type=pathlib.Path)
    a.add_argument("--canale", default="candidate", choices=["prova", "candidate"])
    a.add_argument(
        "--piattaforme",
        default=",".join(PIATTAFORME),
        help="le piattaforme da pretendere; restringerlo e' una decisione da dichiarare",
    )
    arg = a.parse_args()

    directory = arg.referti.resolve()
    if not directory.is_dir():
        sys.exit(f"{directory} non e' una directory")
    piattaforme = tuple(p.strip() for p in arg.piattaforme.split(",") if p.strip())
    sconosciute = set(piattaforme) - set(PIATTAFORME)
    if sconosciute:
        sys.exit(f"piattaforme sconosciute: {sorted(sconosciute)}")

    attesi = sum(len(attese_per(profilo)) for profilo in PROFILI) * len(piattaforme)
    print(f"canale: {arg.canale}")
    print(f"piattaforme: {', '.join(piattaforme)}")
    print(f"referti attesi: {attesi}")

    errori = verifica(directory, arg.canale, piattaforme)
    referti, _ = carica_referti(directory)
    print(f"referti trovati: {len(referti)}")

    if errori:
        print("\n--- ROSSO ---")
        for errore in errori:
            print(f"  {errore}")
        return 1
    print("\nogni artefatto porta i propri referti, e le misure dicono cio' che il contratto promette")
    return 0


if __name__ == "__main__":
    sys.exit(main())
