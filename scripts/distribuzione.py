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
nativo: presenza della firma, identita' e impronta del firmatario, presenza del
timestamp.
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
    (
        "firma",
        "firmare l'entrypoint dove la piattaforma lo richiede, prima di qualunque "
        "cosa ne descriva i byte",
    ),
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
            "misure_pretese": (
                "firmato",
                "firmatario",
                "impronta_firmatario",
                "timestamp",
            ),
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
    `firmato`, `firmatario`, `impronta_firmatario`, `timestamp`. Non e' «il costruttore aveva un
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


def identificatori_di(espressione: str) -> list[str]:
    """Gli identificatori SPDX dentro un'espressione, in ordine.

    `MIT OR Apache-2.0` sono due testi e non uno: un'espressione con `OR`
    lascia a chi riceve la scelta, e per poter scegliere deve avere entrambi.
    `WITH` e' l'altro caso -- `Apache-2.0 WITH LLVM-exception` -- dove il
    secondo e' cio' che rende utilizzabile il primo.
    """
    operatori = {"WITH", "AND", "OR"}
    grezza = espressione.replace("(", " ").replace(")", " ").replace("/", " ")
    return [p for p in grezza.split() if p.upper() not in operatori]


def componenti_rust(elenco: list[dict]) -> list[dict]:
    """I crate di terzi, nella forma dei package SPDX.

    I nostri crate non compaiono: non sono componenti di terzi, sono il
    prodotto. Distinguere non e' un dettaglio -- un SBOM esiste per dire a chi
    riceve che cosa **altro** ha sul disco.
    """
    return [
        {
            "SPDXID": f"SPDXRef-Crate-{p['nome']}-{p['versione']}".replace(".", "-"),
            "name": p["nome"],
            "versionInfo": p["versione"],
            "downloadLocation": p["origine"],
            "licenseConcluded": "NOASSERTION",
            "licenseDeclared": p["licenza"] or "NOASSERTION",
            "filesAnalyzed": False,
            "comment": "crate Rust linkato staticamente nel binario",
        }
        for p in elenco
        if not p["nostro"]
    ]


def documento_spdx(
    nome: str, digesto_identita: str, componenti: list[dict], commento: str
) -> dict:
    """Il documento SPDX 2.3, con un namespace che distingue due build.

    `documentNamespace` deve identificare **questo** documento e non «un
    documento per questa versione»: due build della stessa versione producono
    due SBOM diversi -- fosse anche solo per l'ordine dei crate risolti -- e un
    namespace uguale li renderebbe indistinguibili. Vi entra quindi un digesto
    che dipende dai contenuti.
    """
    return {
        "spdxVersion": "SPDX-2.3",
        "dataLicense": "CC0-1.0",
        "SPDXID": "SPDXRef-DOCUMENT",
        "name": nome,
        "documentNamespace": f"https://plenora.invalid/sbom/{nome}/{digesto_identita}",
        "creationInfo": {
            "created": "1970-01-01T00:00:00Z",
            "creators": ["Tool: plenora-io-distribuzione"],
            "comment": (
                "la data e' fissa di proposito: un SBOM che cambiasse a ogni costruzione per il "
                "solo orologio non sarebbe confrontabile con quello della costruzione precedente, "
                "e confrontarli e' il modo di accorgersi che qualcosa e' cambiato davvero."
            ),
        },
        "comment": commento,
        "packages": componenti,
    }


class SpdxNonValido(ValueError):
    """L'SBOM non e' un documento SPDX 2.3.

    E' un'eccezione e non un avviso perche' un SBOM malformato e' peggio di
    nessun SBOM: chi lo riceve lo dara' in pasto a uno strumento, e uno
    strumento che non lo legge produce un vuoto invece di un errore.
    """


# I campi che SPDX 2.3 pretende, e il perche' di ciascuno. Non e' lo schema
# completo -- quello e' un documento di migliaia di righe -- ma le condizioni
# senza le quali nessuno strumento riesce a leggere il file. Scriverle qui
# invece di scaricare lo schema tiene la verifica **offline** e deterministica,
# ed e' un compromesso dichiarato: valida la forma che serve, non ogni vincolo
# della specifica.
CAMPI_DEL_DOCUMENTO = {
    "spdxVersion": "senza, nessuno strumento sa quale specifica applicare",
    "dataLicense": "SPDX 2.3 pretende CC0-1.0, ed e' l'unico valore ammesso",
    "SPDXID": "l'identificatore del documento, che dev'essere SPDXRef-DOCUMENT",
    "name": "il nome del documento",
    "documentNamespace": "l'identita' univoca: due documenti diversi non possono condividerla",
    "creationInfo": "chi e quando, senza cui il documento non ha provenienza",
    "packages": "i componenti, che sono la ragione per cui il documento esiste",
}

CAMPI_DEL_PACCHETTO = {
    "SPDXID": "l'identificatore, che dev'essere unico nel documento",
    "name": "il nome del componente",
    "downloadLocation": "da dove viene, o NOASSERTION se non lo si sa",
    "licenseConcluded": "la licenza conclusa, o NOASSERTION",
    "licenseDeclared": "la licenza dichiarata dal componente, o NOASSERTION",
    "filesAnalyzed": "se i file del componente sono stati analizzati",
}


def valida_spdx(documento: dict) -> None:
    """Che il documento sia leggibile come SPDX 2.3.

    Il difetto che questa funzione chiude: l'SBOM veniva scritto e nessuno
    verificava che fosse SPDX. Un documento con un campo mancante o uno SPDXID
    ripetuto passa per valido finche' non lo si da' a uno strumento, e a quel
    punto e' gia' stato consegnato.
    """
    mancanti = [c for c in CAMPI_DEL_DOCUMENTO if c not in documento]
    if mancanti:
        raise SpdxNonValido(
            "campi obbligatori assenti dal documento: "
            + ", ".join(f"{c} ({CAMPI_DEL_DOCUMENTO[c]})" for c in mancanti)
        )
    if documento["spdxVersion"] != "SPDX-2.3":
        raise SpdxNonValido(f"spdxVersion e' {documento['spdxVersion']!r}, atteso 'SPDX-2.3'")
    if documento["dataLicense"] != "CC0-1.0":
        raise SpdxNonValido(
            f"dataLicense e' {documento['dataLicense']!r}: SPDX 2.3 ammette solo 'CC0-1.0'"
        )
    if documento["SPDXID"] != "SPDXRef-DOCUMENT":
        raise SpdxNonValido(f"SPDXID del documento e' {documento['SPDXID']!r}")
    if not str(documento["documentNamespace"]).startswith(("http://", "https://")):
        raise SpdxNonValido(
            f"documentNamespace {documento['documentNamespace']!r} non e' un URI assoluto"
        )
    if "creators" not in documento.get("creationInfo", {}):
        raise SpdxNonValido("creationInfo senza `creators`")

    if not documento["packages"]:
        raise SpdxNonValido(
            "nessun componente. Un SBOM vuoto e' peggio di nessun SBOM: dice che l'artefatto "
            "non contiene niente di terzi, e non e' vero di nessun artefatto che spedisca "
            "qualcosa."
        )
    visti: set[str] = set()
    for componente in documento["packages"]:
        assenti = [c for c in CAMPI_DEL_PACCHETTO if c not in componente]
        if assenti:
            raise SpdxNonValido(
                f"{componente.get('name', '(senza nome)')}: campi assenti "
                + ", ".join(f"{c} ({CAMPI_DEL_PACCHETTO[c]})" for c in assenti)
            )
        identita = componente["SPDXID"]
        if not str(identita).startswith("SPDXRef-"):
            raise SpdxNonValido(f"SPDXID {identita!r} non comincia per 'SPDXRef-'")
        if identita in visti:
            raise SpdxNonValido(
                f"SPDXID ripetuto: {identita}. Due componenti con la stessa identita' rendono "
                "il documento ambiguo proprio dove serve essere precisi."
            )
        visti.add(identita)


#: Il runtime C/C++ ridistribuibile, per piattaforma.
#:
#: Non e' un componente del sistema operativo: e' software di terzi che
#: l'artefatto **spedisce**, e che su una macchina senza gli strumenti di
#: sviluppo non ci sarebbe. Il runner di CI lo possiede perche' ci gira Visual
#: Studio, ed e' esattamente il motivo per cui una prova fatta li' non basta a
#: dichiararlo assente.
#:
#: L'elenco vive qui e non dentro il verificatore PE perche' lo leggono in tre:
#: il verificatore per rifiutare chi lo prende dal sistema, i due costruttori
#: per dichiarare che cosa spediscono, e la sonda che confronta le due cose. Tre
#: copie a mano divergerebbero, e la divergenza non farebbe rosso da nessuna
#: parte -- direbbe solo due verita' diverse in due file.
RUNTIME_C_RIDISTRIBUIBILE = {
    "windows-x86_64": {
        "vcruntime140.dll": "runtime C ridistribuibile di Visual Studio, non un componente di Windows",
        "vcruntime140_1.dll": "come sopra",
        "msvcp140.dll": "libreria standard C++ di Visual Studio, ridistribuibile",
    },
    # Su Linux `libc`, `libgcc_s` e `libm` appartengono al sistema e la soglia
    # GLIBC dichiara quale. Non c'e' niente di ridistribuibile da spedire, e un
    # insieme vuoto lo dice meglio di una piattaforma assente dalla tabella --
    # che si leggerebbe come «non ci ho pensato».
    "linux-x86_64": {},
}


def nome_archivio(versione: str, piattaforma: str, profilo: str) -> str:
    """Il nome dell'artefatto, senza estensione.

    Sta qui perche' lo dichiara anche la matrice, e le due scritture erano
    indipendenti: la matrice diceva `<versione>-<profilo>-<piattaforma>`, i due
    costruttori producevano `<versione>-<piattaforma>-<profilo>`, e nessuno le
    confrontava. Il nome e' la prima cosa che legge chi scarica, ed era
    descritto da un registro che descriveva un altro nome.

    L'ordine e' piattaforma poi profilo: chi cerca raggruppa per macchina prima
    che per capability, e due archivi della stessa piattaforma stanno vicini.
    """
    return f"plenora-io-{versione}-{piattaforma}-{profilo}"


def runtime_nativo(
    piattaforma: str, file_spediti: list[dict], gdal: dict | None
) -> dict:
    """Che cosa l'artefatto porta di nativo, **misurato** dai file spediti.

    # Perche' non un booleano

    Il campo era `{"presente": false}` sul profilo base, e voleva dire «non
    spedisce GDAL». Diceva pero' «non spedisce runtime nativo», che sul base
    Windows e' falso: `vcruntime140.dll` e' spedita, e la discovery l'ha
    trovata proprio perche' non spedirla era un difetto. Un campo il cui nome
    promette piu' di quanto misura e' peggio di un campo assente: chi lo legge
    conclude qualcosa, e la conclusione e' sbagliata.

    Sono due componenti indipendenti. GDAL c'e' solo nel profilo pieno; il
    runtime C ridistribuibile c'e' su Windows in **tutti e due** i profili,
    perche' il binario Rust stesso lo importa.

    # Perche' dai file e non dal profilo

    Perche' cosi' e' una misura. `arg.profilo == "filegdb"` e' cio' che
    volevamo costruire; l'elenco dei file e' cio' che abbiamo costruito, ed e'
    l'unico dei due che si accorge di un passo saltato.
    """
    nomi = {pathlib.PurePosixPath(f["percorso"]).name.lower() for f in file_spediti}
    ridistribuibili = RUNTIME_C_RIDISTRIBUIBILE.get(piattaforma, {})
    spediti = sorted(nome for nome in ridistribuibili if nome in nomi)
    return {
        "gdal": gdal,
        "c_ridistribuibile": {
            "presente": bool(spediti),
            "file": spediti,
            "perche": (
                "il binario importa il runtime C di Visual Studio, che non e' un componente "
                "di Windows: va spedito, in tutti e due i profili. Su una macchina senza "
                "strumenti di sviluppo non ci sarebbe."
                if spediti
                else "nessun runtime C ridistribuibile fra i file spediti: su questa "
                "piattaforma la libreria C appartiene al sistema, e la soglia dichiarata "
                "dice quale."
            ),
        },
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
