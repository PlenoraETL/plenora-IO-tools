#!/usr/bin/env python3
"""Cio' che le due piattaforme hanno in comune: il referto, la firma, l'ordine.

# Perche' un formato comune, con verificatori separati

Un ELF e un PE si interrogano con strumenti diversi e rispondono a domande
diverse: `DT_NEEDED` e `GLIBC_*` non esistono su Windows, e un PE ha due tabelle
di import invece di una lista di nomi. Scrivere un verificatore solo
significherebbe scrivere il minimo comune, cioe' verificare meno ovunque.

Cio' che invece **deve** essere comune e' la forma del risultato. Un gate finale
che debba ricontare quattro artefatti non puo' leggere quattro formati, e
soprattutto non puo' accontentarsi di sapere che i job sono verdi: un job verde
e' un'affermazione, non un'evidenza. Il referto porta le **misure**, e chi
riconta guarda quelle.

# La firma si misura, non si dichiara

Uno stato che venisse da «il materiale c'era» direbbe soltanto che il
costruttore ha avuto un certificato fra le mani. Non direbbe che la firma sia
stata apposta, ne' da chi, ne' se porti un timestamp -- e senza timestamp una
firma smette di valere alla scadenza del certificato invece che alla scadenza
del suo uso.

Lo stato viene quindi da una **misura** fatta sui byte finali dal verificatore
nativo: presenza della firma, identita' del firmatario, presenza del timestamp.
Una misura che non si e' potuta fare non e' un si': e' `non_misurata`, e su una
candidate e' rossa.

# Il perimetro della v1

Due piattaforme: Linux x86-64 e Windows x86-64. macOS e' fuori scope -- una
decisione di prodotto registrata in `distribuzione-matrice.json` -- e con esso
sono usciti Developer ID, la notarizzazione e la questione dello stapling, che
erano il pezzo piu' costoso di questa catena.
"""

from __future__ import annotations

import hashlib
import json
import pathlib

SCHEMA_REFERTO = 2

# Il contenitore, per piattaforma.
#
# Windows usa ZIP: un `tar.gz` non e' un formato che gli strumenti Windows
# aprano senza aiuto, e chi installa non deve procurarsi uno strumento per
# leggere un artefatto.
CONTENITORE = {
    "linux-x86_64": "tar.gz",
    "windows-x86_64": "zip",
}

# L'ordine delle operazioni, uguale ovunque.
#
# Non e' una lista di buone intenzioni: ogni passo dipende dai byte prodotti dal
# precedente, e invertirne due produce un artefatto le cui verifiche parlano di
# un file diverso da quello che si consegna.
ORDINE = (
    ("payload", "assemblare l'albero: binario, librerie, dati, licenze"),
    ("firma", "firmare i binari, prima di qualunque cosa li descriva"),
    (
        "manifesto",
        "generare MANIFEST.json dai byte **firmati**: un manifesto scritto prima "
        "elencherebbe file che non esistono piu'",
    ),
    ("archivio", "creare il contenitore"),
    (
        "notarizzazione",
        "nessuna piattaforma del perimetro la richiede; il passo resta perche' "
        "l'ordine e' uno solo, e la posizione e' cio' che va fissata",
    ),
    ("checksum", "calcolare i checksum sui byte **finali**"),
    ("smoke", "eseguire lo smoke sull'oggetto finale, non su una sua versione precedente"),
    ("provenance", "produrre la provenance legata a quel checksum"),
)

# Che cosa si pretende, per piattaforma e per canale.
#
# Il canale `prova` non pretende firma: quegli artefatti esistono per essere
# misurati, non installati, e pretendere un certificato per costruirli
# renderebbe impossibile lavorare senza segreti.
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
            "misure_pretese": ("firmato", "firmatario", "timestamp"),
            "smoke_dopo": "la firma",
            "perche": (
                "un PE non firmato fa comparire un avviso a chi lo esegue, e su alcune "
                "configurazioni non si esegue affatto. Il timestamp e' parte della pretesa: "
                "senza, la firma smette di valere quando scade il certificato invece che "
                "quando scade il suo uso."
            ),
        }
    },
}


def contenitore(piattaforma: str) -> str:
    if piattaforma not in CONTENITORE:
        raise SystemExit(f"piattaforma sconosciuta: {piattaforma}")
    return CONTENITORE[piattaforma]


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


def stato_della_firma(piattaforma: str, canale: str, misura: dict | None = None) -> dict:
    """Il blocco che finisce nel manifesto, **da una misura**.

    `misura` e' cio' che il verificatore nativo ha letto sui byte finali:
    `firmato`, `firmatario`, `timestamp`. Non e' «il costruttore aveva un
    certificato»: quello direbbe soltanto che qualcuno ne ha avuto uno fra le
    mani.

    Gli stati sono quattro e sono diversi apposta:

    - `non_richiesta`: il canale non la pretende su questa piattaforma;
    - `non_misurata`: la pretende, e nessuno ha guardato. Non e' un si';
    - `assente`: si e' guardato, e manca qualcosa di preteso;
    - `apposta`: si e' guardato, e c'e' tutto.

    Il blocco sta nel manifesto **gia' adesso**, con gli artefatti di prova,
    perche' aggiungerlo dopo cambierebbe il manifesto e quindi il checksum
    dell'archivio che lo contiene.
    """
    politica = politica_di_firma(piattaforma, canale)
    pretese = tuple(politica.get("misure_pretese", ()))

    if not politica["richiesta"]:
        stato, mancanti = "non_richiesta", ()
    elif misura is None:
        stato, mancanti = "non_misurata", pretese
    else:
        mancanti = tuple(p for p in pretese if not misura.get(p))
        stato = "assente" if mancanti else "apposta"

    return {
        "stato": stato,
        "meccanismo": politica.get("meccanismo"),
        "misure_pretese": list(pretese),
        "misura": misura,
        "mancanti": list(mancanti),
        # Restano nel blocco perche' il gate li legge, e nessuna politica del
        # perimetro li accende: toglierli cambierebbe la forma del manifesto --
        # e quindi il checksum -- per un guadagno di due righe.
        "notarizzazione": politica.get("notarizzazione", False),
        "stapling": politica.get("stapling", False),
        "smoke_dopo": politica.get("smoke_dopo"),
        "perche": politica.get("perche"),
        "ordine_delle_operazioni": [f"{n}. {passo}: {perche}" for n, (passo, perche) in enumerate(ORDINE, 1)],
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
