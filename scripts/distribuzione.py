#!/usr/bin/env python3
"""Cio' che le tre piattaforme hanno in comune: il referto e la firma.

# Perche' un formato comune, con verificatori separati

Un ELF, un PE e un Mach-O si interrogano con strumenti diversi e rispondono a
domande diverse: `DT_NEEDED` e `GLIBC_*` non esistono su Windows, e un
`LC_VERSION_MIN_MACOSX` non esiste altrove. Scrivere un verificatore solo
significherebbe scrivere il minimo comune, cioe' verificare meno ovunque.

Cio' che invece **deve** essere comune e' la forma del risultato. Un gate finale
che debba ricontare sei artefatti non puo' leggere sei formati, e soprattutto
non puo' accontentarsi di sapere che sei job sono verdi: un job verde e'
un'affermazione, non un'evidenza. Il referto porta le **misure**, e chi
riconta guarda quelle.

# La firma

La decisione e' qui, e non nei workflow, perche' cambia l'ordine delle
operazioni e quindi i byte. Firmare **dopo** aver calcolato il checksum
produrrebbe un archivio il cui checksum non corrisponde, e uno smoke eseguito
prima della firma non direbbe nulla sull'artefatto che si consegna: su macOS un
binario notarizzato e con lo stapling e' un file diverso, e su Windows un PE
firmato ha una sezione in piu'.

L'ordine e' quindi fissato ora, quando ancora non c'e' un certificato:
**assembla, firma, poi calcola i checksum, poi esegui lo smoke.** Cosi' il
giorno in cui il certificato arrivera' non cambiera' nulla di strutturale.
"""

from __future__ import annotations

import hashlib
import json
import pathlib

SCHEMA_REFERTO = 1

# Che cosa si pretende, per piattaforma e per canale.
#
# Il canale `prova` non pretende firma: quegli artefatti esistono per essere
# misurati, non installati, e pretendere un certificato per costruirli
# renderebbe impossibile lavorare senza segreti.
#
# Linux non compare perche' non ha un meccanismo di firma della piattaforma che
# il sistema verifichi all'esecuzione. Restano i checksum e la provenance, che
# valgono ovunque. Dichiararlo qui invece di lasciarlo implicito e' la
# differenza fra «non serve» e «ce ne siamo dimenticati».
POLITICA_DI_FIRMA = {
    "linux-x86_64": {
        "candidate": {
            "meccanismo": None,
            "perche": (
                "Linux non ha una firma di piattaforma che il sistema verifichi "
                "all'esecuzione. L'integrita' passa dai checksum e dalla provenance, che "
                "non sostituiscono una firma ma sono cio' che questa piattaforma offre."
            ),
        }
    },
    "windows-x86_64": {
        "candidate": {
            "meccanismo": "authenticode",
            "smoke_dopo": "la firma",
            "perche": (
                "un PE non firmato fa comparire un avviso a chi lo esegue, e su alcune "
                "configurazioni non si esegue affatto. Lo smoke va fatto **dopo**: un PE "
                "firmato ha una sezione in piu', ed e' quel file che si consegna."
            ),
        }
    },
    "macos-aarch64": {
        "candidate": {
            "meccanismo": "developer-id",
            "notarizzazione": True,
            "stapling": True,
            "smoke_dopo": "lo stapling",
            "perche": (
                "senza Developer ID e notarizzazione Gatekeeper rifiuta il binario, e "
                "l'artefatto non si installa affatto. Lo stapling attacca la ricevuta al "
                "file, cosi' che valga anche senza rete: lo smoke va fatto dopo, perche' "
                "prima si starebbe provando un altro file."
            ),
        }
    },
}


def politica_di_firma(piattaforma: str, canale: str) -> dict:
    """Che cosa il canale pretende su quella piattaforma.

    Un canale che non compare non pretende niente, e lo dice: restituire un
    dizionario vuoto lascerebbe a chi chiama il compito di distinguere «non
    richiesta» da «non l'ho trovata».
    """
    per_piattaforma = POLITICA_DI_FIRMA.get(piattaforma)
    if per_piattaforma is None:
        raise SystemExit(
            f"piattaforma sconosciuta alla politica di firma: {piattaforma}. "
            "Una piattaforma nuova va decisa, non dedotta dal silenzio."
        )
    regola = per_piattaforma.get(canale)
    if regola is None:
        return {"richiesta": False, "perche": f"il canale «{canale}» non pretende firma"}
    if regola.get("meccanismo") is None:
        return {"richiesta": False, **regola}
    return {"richiesta": True, **regola}


def stato_della_firma(piattaforma: str, canale: str, materiale_disponibile: bool) -> dict:
    """Il blocco che finisce nel manifesto.

    Sta nel manifesto **gia' adesso**, con gli artefatti di prova, perche'
    aggiungerlo dopo cambierebbe il manifesto e quindi il checksum
    dell'archivio. Il campo esiste prima del certificato.
    """
    politica = politica_di_firma(piattaforma, canale)
    if not politica["richiesta"]:
        stato = "non_richiesta"
    elif materiale_disponibile:
        stato = "apposta"
    else:
        stato = "assente"
    return {
        "stato": stato,
        "meccanismo": politica.get("meccanismo"),
        "notarizzazione": politica.get("notarizzazione", False),
        "stapling": politica.get("stapling", False),
        "smoke_dopo": politica.get("smoke_dopo"),
        "perche": politica.get("perche"),
        "ordine_delle_operazioni": (
            "assembla, firma, poi calcola i checksum, poi esegui lo smoke. Firmare dopo il "
            "checksum lo invaliderebbe, e uno smoke prima della firma proverebbe un file "
            "diverso da quello che si consegna."
        ),
    }


def sha256(percorso: pathlib.Path) -> str:
    digesto = hashlib.sha256()
    with percorso.open("rb") as f:
        for blocco in iter(lambda: f.read(1 << 20), b""):
            digesto.update(blocco)
    return digesto.hexdigest()


def scrivi_referto(
    percorso: pathlib.Path,
    *,
    verifica: str,
    piattaforma: str,
    profilo: str,
    canale: str,
    esito: str,
    misure: dict,
    errori: list[str],
    note: str | None = None,
) -> None:
    """Il formato comune, che il gate finale riconta.

    `misure` e' la parte che conta: l'esito e' una conclusione, e una
    conclusione si puo' sbagliare in silenzio. I numeri no -- o ci sono e
    tornano, o non ci sono.
    """
    percorso.parent.mkdir(parents=True, exist_ok=True)
    percorso.write_text(
        json.dumps(
            {
                "schema_referto": SCHEMA_REFERTO,
                "verifica": verifica,
                "piattaforma": piattaforma,
                "profilo": profilo,
                "canale": canale,
                "esito": esito,
                "misure": misure,
                "errori": errori,
                "note": note,
            },
            ensure_ascii=False,
            indent=2,
        )
        + "\n",
        encoding="utf-8",
    )
