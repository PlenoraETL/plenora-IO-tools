#!/usr/bin/env python3
"""Cio' che le tre piattaforme hanno in comune: il referto, la firma, l'ordine.

# Perche' un formato comune, con verificatori separati

Un ELF, un PE e un Mach-O si interrogano con strumenti diversi e rispondono a
domande diverse: `DT_NEEDED` e `GLIBC_*` non esistono su Windows, un
`LC_VERSION_MIN_MACOSX` non esiste altrove. Scrivere un verificatore solo
significherebbe scrivere il minimo comune, cioe' verificare meno ovunque.

Cio' che invece **deve** essere comune e' la forma del risultato. Un gate finale
che debba ricontare sei artefatti non puo' leggere sei formati, e soprattutto
non puo' accontentarsi di sapere che sei job sono verdi: un job verde e'
un'affermazione, non un'evidenza. Il referto porta le **misure**, e chi riconta
guarda quelle.

# La firma si misura, non si dichiara

Uno stato che venisse da «il materiale c'era» direbbe soltanto che il
costruttore ha avuto un certificato fra le mani. Non direbbe che la firma sia
stata apposta, ne' da chi, ne' se porti un timestamp -- e senza timestamp una
firma smette di valere alla scadenza del certificato invece che alla scadenza
del suo uso.

Lo stato viene quindi da una **misura** fatta sui byte finali dai verificatori
nativi: presenza della firma, identita' del firmatario, presenza del timestamp,
e su macOS l'accettazione notarile. Una misura che non si e' potuta fare non e'
un si': e' `non_misurata`, e su una candidate e' rossa.

# Lo stapling non e' disponibile ovunque

Apple consente di notarizzare un archivio ZIP, ma `stapler` attacca la ricevuta
solo ad app bundle, DMG e PKG -- non a uno ZIP ne' a un binario sciolto. Il
deliverable macOS e' oggi uno ZIP di una CLI rilocabile: si notarizza e **non**
si fa stapling, e la conseguenza va detta a chi installa invece che scoperta da
lui. La prima verifica di Gatekeeper richiedera' rete, perche' andra' a
chiedere la ricevuta al servizio.

Se il funzionamento offline diventasse un requisito, il deliverable dovrebbe
cambiare forma -- DMG o PKG -- e a quel punto lo stapling sarebbe possibile.
E' una decisione sul prodotto, non un dettaglio di confezionamento, e sta
scritta nella matrice di distribuzione.
"""

from __future__ import annotations

import hashlib
import json
import pathlib

SCHEMA_REFERTO = 2

# Il contenitore, per piattaforma.
#
# macOS usa ZIP e non tar.gz: la notarizzazione accetta ZIP, e un tar.gz non e'
# un formato che gli strumenti Apple sappiano ispezionare. Non e' una preferenza
# di stile -- e' cio' che rende l'artefatto sottoponibile al servizio.
CONTENITORE = {
    "linux-x86_64": "tar.gz",
    "windows-x86_64": "zip",
    "macos-aarch64": "zip",
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
        "notarizzare, e fare stapling dove e' possibile -- su ZIP non lo e'",
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
    "macos-aarch64": {
        "candidate": {
            "meccanismo": "developer-id",
            "notarizzazione": True,
            "stapling": False,
            "misure_pretese": ("firmato", "firmatario", "timestamp", "hardened_runtime", "notarizzato"),
            "smoke_dopo": "la notarizzazione",
            "perche": (
                "senza Developer ID e notarizzazione Gatekeeper rifiuta il binario, e "
                "l'artefatto non si installa affatto. L'hardened runtime e' una condizione "
                "della notarizzazione, non un extra."
            ),
            "perche_niente_stapling": (
                "`stapler` attacca la ricevuta ad app bundle, DMG e PKG; non a uno ZIP ne' a "
                "un binario sciolto. Il deliverable e' uno ZIP di una CLI rilocabile, quindi "
                "si notarizza e basta -- e **la prima verifica di Gatekeeper richiedera' "
                "rete**, perche' andra' a chiedere la ricevuta al servizio. Va detto a chi "
                "installa invece che lasciato scoprire. Se l'uso offline diventasse un "
                "requisito, il deliverable dovrebbe diventare un DMG o un PKG: e' una "
                "decisione sul prodotto, non sul confezionamento."
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

    `misura` e' cio' che un verificatore nativo ha letto sui byte finali:
    `firmato`, `firmatario`, `timestamp`, e su macOS `hardened_runtime` e
    `notarizzato`. Non e' «il costruttore aveva un certificato»: quello direbbe
    soltanto che qualcuno ne ha avuto uno fra le mani.

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
        "notarizzazione": politica.get("notarizzazione", False),
        "stapling": politica.get("stapling", False),
        "perche_niente_stapling": politica.get("perche_niente_stapling"),
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
