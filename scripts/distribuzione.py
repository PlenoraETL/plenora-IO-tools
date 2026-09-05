#!/usr/bin/env python3
"""Cio' che le due piattaforme hanno in comune: il referto e l'ordine.

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

# La firma e' una scelta esplicita, anche quando non e' richiesta

La release 2.0.0 distribuisce intenzionalmente artefatti senza firma di
piattaforma sia su Linux sia su Windows. Non e' un'omissione dedotta
dall'assenza di un certificato: la politica lo dichiara, il manifesto lo porta
come `non_richiesta`, e checksum e provenance restano obbligatori. Su Windows
questo puo' produrre l'avviso «editore sconosciuto» o un blocco imposto da una
policy aziendale; e' un limite della distribuzione dichiarata, non qualcosa che
il gate deve nascondere.

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
import re
import subprocess

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
        "dichiarare esplicitamente che la release non richiede una firma di piattaforma",
    ),
    (
        "manifesto",
        "generare MANIFEST.json dai byte finali del payload",
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
# Nessun canale della 2.0.0 pretende una firma di piattaforma. Il blocco resta
# esplicito nel manifesto: un campo assente potrebbe significare dimenticanza,
# `non_richiesta` significa decisione.
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
            "meccanismo": None,
            "perche": (
                "la release 2.0.0 distribuisce intenzionalmente il PE senza Authenticode. "
                "Windows puo' mostrare «editore sconosciuto» e una policy aziendale puo' "
                "bloccarlo; integrita' e provenienza si verificano con SHA-256, manifesto "
                "e provenance pubblicati accanto all'archivio."
            ),
        }
    },
}



# --- il manifesto, e perche' si costruisce da un punto solo ------------------
#
# I due costruttori scrivevano ciascuno il proprio dizionario. Le divergenze si
# scoprivano aprendo il deliverable, e due sono arrivate dentro una candidate.
#
# * Il manifesto Windows portava `revisione`, quello Linux no: chi aveva
#   soltanto l'albero installato poteva dire da quale revisione venisse
#   un'installazione Windows e non una Linux.
# * `canale_nota` esisteva solo su Linux, era emesso **sempre**, e il suo testo
#   parlava del canale `prova`. Nei due archivi Linux della candidate diceva che
#   l'artefatto non e' pubblicato e che il gate lo rifiuta dove si pretenda un
#   artefatto di rilascio -- accanto a `canale: candidate`. Era una dichiarazione
#   falsa spedita dentro il deliverable.
#
# Nessun controllo li trovava, e non per distrazione: un campo di prosa e'
# coerente con se stesso, e un campo assente non fallisce da nessuna parte. La
# cura non e' rileggere meglio i due dizionari: e' che ci sia un punto solo che
# li costruisce, e che rifiuti di produrre un documento incompleto.

# Il testo del canale, **derivato** dal canale. Nessuna delle due note nomina
# l'altro canale: una nota che parlasse di entrambi si leggerebbe male da tutti
# e due i lati, e sarebbe la via comoda per «correggere» il difetto -- aggiungere
# una frase invece di derivare il testo.
NOTA_DEL_CANALE = {
    "prova": (
        "«prova» significa che questo artefatto e' stato costruito per essere "
        "misurato, non installato: non e' pubblicato, e il gate di distribuzione "
        "lo rifiuta dove si pretenda un artefatto di rilascio."
    ),
    "candidate": (
        "«candidate» significa che questo artefatto e' quello che si consegna: "
        "viene da una revisione congelata, che il campo `revisione` nomina, e la "
        "sua integrita' si verifica con lo sha256, il manifesto e la provenance "
        "pubblicati accanto. La 2.0.0 non richiede una firma di piattaforma: su "
        "Windows il PE non porta Authenticode, quindi il sistema puo' mostrare "
        "«editore sconosciuto» e una policy aziendale puo' impedirne "
        "l'esecuzione."
    ),
}

# I campi che **entrambi** i manifesti portano, con lo stesso significato. E' un
# insieme chiuso: un campo che sparisce da un costruttore non sparisce in
# silenzio, e uno che compare su una piattaforma sola non e' un campo comune.
CAMPI_COMUNI_DEL_MANIFESTO = frozenset(
    {
        "nome",
        "versione",
        "piattaforma",
        "profilo",
        "canale",
        "canale_nota",
        "non_release",
        "revisione",
        "runtime_nativo",
        "lock",
        "prefisso_di_costruzione",
        "firma",
        "licenze",
        "file",
    }
)

#: La licenza first-party: non dichiarata, e fuori dal perimetro corrente.
#:
#: # Che cosa dice
#:
#: Che non c'e'. Questo repository non dichiara una licenza propria -- nessun
#: `Cargo.toml` ha il campo, il `pyproject.toml` nemmeno -- e per la
#: distribuzione che si sta facendo **non serve**: gli artefatti si consegnano
#: a clienti autorizzati per un canale riservato, e i termini d'uso stanno nel
#: rapporto con loro, non in un file dentro l'archivio.
#:
#: # Che cosa non dice
#:
#: Non e' una verifica riuscita. Nessuno ha stabilito che l'assenza vada bene
#: in generale: va bene **per questo perimetro**. Una distribuzione pubblica --
#: un indice, un repository aperto, un artefatto scaricabile senza contratto --
#: richiede una decisione separata, e quella decisione comprende il testo dei
#: termini e la denominazione legale esatta del titolare.
#:
#: E non e' nemmeno un blocco. Trattarla come tale avrebbe fermato una
#: distribuzione privata che non ne ha bisogno, per un documento che serve a
#: un'altra cosa.
#:
#: # Perche' un dato e non una riga di prosa
#:
#: Perche' compare in tre posti -- i metadati del pacchetto Python, il referto
#: `licenze-artefatto` di ogni albero, la documentazione -- e tre copie
#: divergono. Qui c'e' l'originale, e i referti lo leggono.
#:
#: A restare invariato e' cio' che riguarda i **terzi**: ogni componente che
#: spedisce byte porta il testo della propria licenza, e `componenti_con_testo`
#: lo conta. Sono due domande diverse, e l'assenza della prima non tocca la
#: seconda.
LICENZA_FIRST_PARTY = {
    "dichiarata": False,
    "stato": "fuori_dal_perimetro",
    "registrata_il": "2026-09-05",
    "identificatore_spdx": None,
    "perche_non_dichiarata": (
        "il repository non ne dichiara una, e per la distribuzione privata "
        "corrente non serve: gli artefatti vanno a clienti autorizzati per un "
        "canale riservato, e i termini d'uso stanno nel rapporto con loro."
    ),
    "non_e_una_verifica": (
        "l'assenza non e' stata verificata accettabile in generale, ma per "
        "questo perimetro. Chiamarla verde direbbe che qualcuno ha guardato e "
        "approvato, e nessuno l'ha fatto."
    ),
    "che_cosa_richiederebbe_una_distribuzione_pubblica": (
        "una decisione separata del titolare, che comprende il testo integrale "
        "dei termini e la denominazione legale esatta con cui compaiono. "
        "Nessuno dei due si scrive qui: la prima stesura del `pyproject.toml` "
        "diceva `Apache-2.0` -- una concessione che nessuno aveva fatto -- ed e' "
        "precisamente l'errore che questa struttura esiste per non ripetere."
    ),
    "canale": "riservato a clienti autorizzati; nessun indice pubblico",
    "licenze_di_terzi": (
        "invariate e obbligatorie: ogni componente che spedisce byte porta il "
        "testo della propria licenza, e il conteggio `componenti_con_testo` lo "
        "misura. L'assenza di una licenza first-party non tocca questo."
    ),
    "forma_leggibile": (
        "wheel e sdist sono Python puro: contengono i `.py` cosi' come sono "
        "scritti, e la sdist anche i test. Chi li riceve legge il sorgente. Non "
        "e' una svista: nessuna riservatezza del codice e' stata promessa, e "
        "ottenerla richiederebbe un SDK compilato -- un altro prodotto, con "
        "un'altra qualifica."
    ),
}


def licenza_first_party() -> dict:
    """Una copia dello stato della licenza propria, per chi scrive un referto.

    Copia e non riferimento: un chiamante che aggiungesse una chiave al dato
    condiviso la farebbe comparire nei referti di tutti gli altri.
    """
    return dict(LICENZA_FIRST_PARTY)


REVISIONE = re.compile(r"^[0-9a-f]{40}$")


def nota_del_canale(canale: str) -> str:
    """Il testo del canale, o `KeyError` se il canale non e' fra quelli noti.

    Solleva invece di restituire un testo di ripiego: una nota generica accanto
    a un canale sconosciuto e' esattamente il documento che si contraddice, con
    un passaggio in meno per accorgersene.
    """
    return NOTA_DEL_CANALE[canale]


def revisione_del_repository() -> str | None:
    """La revisione da cui l'artefatto e' stato costruito, o `None`.

    `None` quando non si riesce a leggerla, e non una stringa di comodo: un
    documento che dichiarasse una revisione inventata sarebbe peggio di uno che
    ammette di non saperla, perche' chi legge deve poter distinguere una
    revisione assente da una sbagliata.

    `git` puo' non esserci: l'immagine di costruzione porta cio' che serve a
    costruire, e git non serve a costruire. Anche in quel caso la risposta e'
    «non la so» e non un'eccezione -- il manifesto accompagna l'artefatto, non
    e' una condizione per produrlo. Nel canale `candidate`, pero', `manifesto`
    la pretende: li' un artefatto che nessuno puo' legare a un albero non e' un
    documento onesto, e' un artefatto non qualificabile.

    Stava scritta due volte, una per costruttore, con due docstring diverse e la
    stessa logica. Due definizioni della stessa cosa divergono, e divergono in
    silenzio.
    """
    radice = pathlib.Path(__file__).resolve().parent.parent
    try:
        esito = subprocess.run(
            ["git", "rev-parse", "HEAD"], capture_output=True, text=True, cwd=radice
        )
    except (OSError, subprocess.SubprocessError):
        return None
    return esito.stdout.strip() if esito.returncode == 0 else None


def manifesto(comuni: dict, extra: dict) -> dict:
    """Il manifesto: i campi comuni, piu' quelli che la piattaforma aggiunge.

    Rifiuta di produrre un documento che non si possa leggere allo stesso modo
    su tutte e due le piattaforme. Le quattro cose che pretende:

    1. ogni campo di `CAMPI_COMUNI_DEL_MANIFESTO` c'e';
    2. `canale_nota` e' **esattamente** la nota del proprio canale. Il confronto
       non e' per contenimento: una nota che porti la frase giusta insieme a
       quella sbagliata passerebbe un controllo di sottostringa, e sarebbe
       ancora un documento che si contraddice;
    3. `non_release` segue il canale, invece di essere scritto a parte;
    4. una `candidate` nomina la propria revisione.

    `extra` non puo' ridefinire un campo comune: sarebbe la divergenza di prima
    con un passaggio in piu'.
    """
    mancanti = CAMPI_COMUNI_DEL_MANIFESTO - set(comuni)
    if mancanti:
        raise ValueError(
            f"manifesto senza i campi comuni {sorted(mancanti)}. Sono l'insieme "
            "che i due costruttori devono dire allo stesso modo: uno che manca "
            "su una piattaforma sola e' la divergenza che si scopre aprendo il "
            "deliverable."
        )
    sovrapposti = CAMPI_COMUNI_DEL_MANIFESTO & set(extra)
    if sovrapposti:
        raise ValueError(
            f"la piattaforma ridefinisce i campi comuni {sorted(sovrapposti)}: "
            "un significato comune riscritto da un lato solo e' una divergenza."
        )

    canale = comuni["canale"]
    if canale not in NOTA_DEL_CANALE:
        raise ValueError(
            f"canale «{canale}» non fra {sorted(NOTA_DEL_CANALE)}: un artefatto "
            "di un canale che non esiste non ha una politica da dichiarare."
        )
    if comuni["canale_nota"] != NOTA_DEL_CANALE[canale]:
        raise ValueError(
            f"`canale_nota` non e' la nota del canale «{canale}». La nota si "
            "deriva dal canale reale: scritta a parte, ha descritto un artefatto "
            "di prova dentro una candidate, e nessun controllo se n'e' accorto."
        )
    if comuni["non_release"] is not (canale != "candidate"):
        raise ValueError(
            f"`non_release` vale {comuni['non_release']!r} nel canale «{canale}»: "
            "segue il canale, e scritto a parte puo' contraddirlo."
        )

    revisione = comuni["revisione"]
    if canale == "candidate":
        if not isinstance(revisione, str) or not REVISIONE.match(revisione):
            raise ValueError(
                f"`revisione` vale {revisione!r} in una candidate. `None` e' "
                "onesto dove non si pretende un artefatto installabile; qui e' "
                "un artefatto che nessuno puo' legare a un albero, e il "
                "contratto pretende che ogni artefatto qualificato venga da una "
                "revisione nominata."
            )
    elif revisione is not None and (
        not isinstance(revisione, str) or not REVISIONE.match(revisione)
    ):
        raise ValueError(f"`revisione` vale {revisione!r}, che non e' uno sha")

    return {**comuni, **extra}


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
