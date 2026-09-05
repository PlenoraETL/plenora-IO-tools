#!/usr/bin/env python3
"""Il gate finale: quattro artefatti, e i referti che li dimostrano.

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

Due piattaforme per due profili sono quattro artefatti, e ogni artefatto vuole
le sue verifiche. Un referto mancante non e' un'omissione da tollerare: e' la
differenza fra «verificato» e «non verificato», e sono le due cose che questo
gate esiste per distinguere.

Il perimetro non e' cablato qui: viene dalla matrice, dove macOS e' registrato
come **fuori scope della v1**. Un artefatto in meno perche' qualcuno ha tolto un
job e' un buco; un artefatto in meno perche' una piattaforma e' fuori scope e'
una decisione.

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

MATRICE = (
    pathlib.Path(__file__).resolve().parent.parent
    / "assurance"
    / "registries"
    / "distribuzione-matrice.json"
)

# Tutte le piattaforme che questo progetto conosce. Non e' l'elenco di cio' che
# si distribuisce: e' l'elenco di cio' su cui **una decisione va presa**, e
# serve perche' una piattaforma non possa uscire dal perimetro sparendo.
PIATTAFORME_NOTE = ("linux-x86_64", "windows-x86_64", "macos-aarch64")
PROFILI = ("base", "filegdb")


def perimetro() -> tuple[tuple[str, ...], dict[str, dict]]:
    """Le piattaforme distribuite, **dalla decisione** e non dai job che esistono.

    Un artefatto in meno perche' qualcuno ha tolto un job e' un buco; un
    artefatto in meno perche' una piattaforma e' fuori scope e' una decisione.
    Nel conteggio si somigliano, e per il resto non si somigliano per niente:
    derivare il perimetro dalla matrice invece che da una costante e' cio' che
    tiene le due cose distinte.

    Ogni piattaforma nota deve stare in una delle due liste con una motivazione
    scritta. Una che non stia in nessuna delle due ferma il gate: significa che
    e' uscita senza che nessuno lo dicesse.
    """
    matrice = json.loads(MATRICE.read_text(encoding="utf-8"))
    distribuite = tuple(p["id"] for p in matrice["piattaforme"])
    fuori = {p["id"]: p for p in matrice.get("piattaforme_non_distribuite", [])}

    senza_decisione = sorted(set(PIATTAFORME_NOTE) - set(distribuite) - set(fuori))
    if senza_decisione:
        raise SystemExit(
            f"piattaforme senza decisione: {senza_decisione}. Una piattaforma non esce dal "
            "perimetro sparendo: o e' distribuita, o e' dichiarata fuori scope con una "
            "motivazione. Il conteggio degli artefatti attesi deve venire da una scelta, non "
            "dall'assenza di un job."
        )
    for identita, voce in sorted(fuori.items()):
        for campo in ("decisione", "perche", "che_cosa_la_ribalterebbe"):
            if not voce.get(campo):
                raise SystemExit(
                    f"{identita} e' dichiarata fuori dal perimetro e «{campo}» e' vuoto. "
                    "Dichiararlo costa dire perche', altrimenti la decisione e' un modo di non "
                    "prenderla."
                )
    return distribuite, fuori

# Le verifiche che ogni artefatto deve portare, e la misura che ciascuna deve
# produrre. La misura conta piu' dell'esito: l'esito e' una conclusione, e una
# conclusione si puo' sbagliare in silenzio; una misura o c'e' e torna, o no.
VERIFICHE_ATTESE = {
    "runtime": {
        # `binari_spediti` e non `elf_spediti`: su Windows sono PE, e un nome
        # che nomina un formato in un contratto comune e' un'ipotesi travestita
        # da campo.
        "misure_obbligatorie": ("binari_spediti", "dipendenze_esterne"),
        # Nessun `solo_profilo`: era un'ipotesi ereditata da Linux, dove il
        # profilo base non spedisce librerie perche' `libgcc_s` e `libm` sono
        # garantite dal sistema. La prima corsa di scoperta su Windows l'ha
        # smentita: anche il base importa `vcruntime140.dll`, che **non** e' un
        # componente del sistema operativo -- e' il runtime C ridistribuibile,
        # e va spedito. Un profilo che spedisce qualcosa ha una chiusura da
        # verificare, e chi non spedisce niente lo dimostra con un referto che
        # dice zero.
        "perche": "la chiusura, le dipendenze fuori dall'albero e i percorsi cotti dentro",
    },
    "licenze-artefatto": {
        # Due domande diverse, e prima ce n'era una sola.
        #
        # `componenti_con_testo` conta le licenze **altrui**: un albero nativo
        # ne porta quarantatre, la wheel zero perche' non spedisce byte di
        # terzi. Nessuna delle due dice sotto quale licenza l'artefatto stesso
        # viene consegnato, e per un prodotto proprietario e' la domanda che
        # conta di piu': un artefatto puo' avere tutti i testi altrui in ordine
        # e non dire niente del proprio regime.
        "misure_obbligatorie": ("componenti_con_testo", "licenza_first_party"),
        "perche": (
            "ogni componente che spedisce byte porta il testo della propria "
            "licenza, e l'artefatto dichiara sotto quale licenza viene consegnato"
        ),
    },
    "smoke-profilo": {
        # Anche `non_richiesta` e' una misura: distingue la decisione unsigned
        # da un costruttore che ha dimenticato l'intero blocco.
        "misure_obbligatorie": ("firma",),
        "perche": "che cosa l'artefatto **installato** sa fare, e che cosa deve non saper fare",
    },
    "provenance": {
        "misure_obbligatorie": ("archivio_sha256", "revisione", "lock_sha256"),
        "perche": "che cosa e' stato costruito, da quale revisione, e con quale lock",
    },
    # Nessun `solo_profilo`: era la premessa che su Windows il profilo base
    # spedisce `vcruntime140.dll`, e quindi ha librerie da rilocare come
    # l'altro. Anche dove non ne spedisce, l'artefatto va estratto in una
    # directory nuova e provato **dall'archivio finale**: che funzioni dove e'
    # stato costruito non e' cio' che si promette a chi installa.
    "relocation": {
        "misure_obbligatorie": ("librerie_dall_albero",),
        "perche": "l'artefatto funziona dove non e' stato costruito",
    },
    # I digest del manifesto, ricalcolati sull'albero **estratto**. Erano
    # scritti e nessuno li rileggeva: un digest che nessuno verifica e' un
    # numero, e per giunta una garanzia apparente -- chi legge il manifesto
    # suppone che qualcuno l'abbia controllata.
    "digest-manifesto": {
        "misure_obbligatorie": ("file_dichiarati", "file_verificati", "digest_divergenti"),
        "perche": "ogni file dichiarato c'e', e il suo digest corrisponde",
    },
    # --- le due della classe `python-puro` --------------------------------
    "sbom": {
        "misure_obbligatorie": ("componenti_di_terzi",),
        "perche": (
            "che cosa il pacchetto contiene di altri. Zero e' una misura: un "
            "pacchetto senza dipendenze deve **dichiarare** di non averne, o "
            "l'assenza dell'elenco si legge come un elenco non compilato"
        ),
    },
    "smoke-installato": {
        "misure_obbligatorie": (
            "manifesto_letto",
            "profilo_accettato",
            "profilo_altrui_rifiutato",
            "binari_nel_pacchetto",
            "dipendenze",
        ),
        "perche": (
            "il pacchetto **installato** trova un artefatto nativo vero, ne "
            "legge il profilo e rifiuta l'altro -- e non porta binari dentro di "
            "se'"
        ),
    },
}

# La misura che deve dire una cosa precisa, e non solo esistere.
# Le misure che su una **candidate** devono avere un valore vero. Su un
# artefatto di prova possono restare dichiarate: quegli artefatti esistono per
# essere misurati, e pretendere una revisione da una macchina senza `git`
# renderebbe impossibile costruirli.
PRETESE_DELLA_CANDIDATE = {
    ("provenance", "revisione"): (
        "una provenance che non sa da quale revisione viene non lega niente: dice che esiste "
        "un archivio con un checksum, e quello lo dice gia' il checksum"
    ),
}

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


def classi(matrice: dict) -> dict[str, dict]:
    """Le classi di artefatto dichiarate, con i loro obblighi.

    Fallisce chiuso se una classe nomina una verifica che il gate non conosce:
    un obbligo che nessuno sa ricontare e' un obbligo che non esiste, e
    lasciarlo passare renderebbe la matrice il posto dove si promette senza
    conseguenze.
    """
    dichiarate = {
        nome: voce
        for nome, voce in matrice.get("classi_di_artefatto", {}).items()
        if isinstance(voce, dict) and "obblighi" in voce
    }
    if not dichiarate:
        raise SystemExit(
            "la matrice non dichiara nessuna classe di artefatto: senza, gli "
            "obblighi tornerebbero a essere un elenco uguale per tutti, e un "
            "pacchetto Python puro pretenderebbe un referto di chiusura ELF."
        )
    for nome, voce in sorted(dichiarate.items()):
        ignote = sorted(set(voce["obblighi"]) - set(VERIFICHE_ATTESE))
        if ignote:
            raise SystemExit(
                f"la classe «{nome}» pretende {ignote}, che questo gate non sa "
                "ricontare. Un obbligo che nessuno riconta non e' un obbligo."
            )
    return dichiarate


def artefatti_attesi(matrice: dict, piattaforme: tuple[str, ...]) -> list[tuple[str, str, str]]:
    """`(piattaforma, profilo, classe)` per ogni artefatto che deve esistere.

    Il numero non e' scritto da nessuna parte: e' la somma di cio' che le due
    classi producono. Aggiungerne una si vede nel conteggio, invece di
    richiedere una modifica qui -- che e' la differenza fra una matrice che
    decide e una che descrive quello che il gate faceva gia'.
    """
    fuori: list[tuple[str, str, str]] = []
    for piattaforma in piattaforme:
        for profilo in PROFILI:
            fuori.append((piattaforma, profilo, "nativo"))
    for voce in matrice.get("artefatti_python", {}).get("elenco", []):
        # `any` e non una piattaforma: `py3-none-any` vuol dire che ne servono
        # zero, e il formato prende il posto del profilo.
        fuori.append(("any", voce["formato"], "python-puro"))
    return fuori


def attese_per(profilo: str, classe: str, dichiarate: dict[str, dict]) -> set[str]:
    """Gli obblighi di un artefatto: dalla sua **classe**, non da un elenco unico."""
    obblighi = set(dichiarate[classe]["obblighi"])
    return {
        nome
        for nome in obblighi
        if VERIFICHE_ATTESE[nome].get("solo_profilo") in (None, profilo)
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


def verifica(
    directory: pathlib.Path,
    canale: str,
    piattaforme: tuple[str, ...],
    attesi: list[tuple[str, str, str]],
    dichiarate: dict[str, dict],
) -> list[str]:
    referti, errori = carica_referti(directory)

    for piattaforma, profilo, classe in attesi:
        for nome in sorted(attese_per(profilo, classe, dichiarate)):
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
                if canale == "candidate":
                    for (v, misura), perche in PRETESE_DELLA_CANDIDATE.items():
                        if v == nome and not misure.get(misura):
                            errori.append(
                                f"{piattaforma}/{profilo}/{nome}: «{misura}» vale "
                                f"{misure.get(misura)!r} su una candidate. {perche}."
                            )
                pretesa = PRETESE_SULLE_MISURE.get((nome, profilo))
                if pretesa:
                    chiave_misura, atteso, perche = pretesa
                    if misure.get(chiave_misura) != atteso:
                        errori.append(
                            f"{piattaforma}/{profilo}/{nome}: «{chiave_misura}» vale "
                            f"{misure.get(chiave_misura)!r}, atteso {atteso!r}. {perche}."
                        )

    # La firma: se una futura politica la pretendesse, dovrebbe essere stata
    # **misurata**. La 2.0.0 non entra in questo ramo su nessuna piattaforma;
    # mantenerlo generico impedisce che una futura richiesta diventi un solo
    # booleano dichiarato dal costruttore.
    #
    # Uno stato che venisse da «il materiale c'era» direbbe soltanto che il
    # costruttore ha avuto un certificato fra le mani. In quel caso si pretende che i
    # verificatori nativi abbiano letto i byte finali: firma presente, identita'
    # del firmatario, timestamp, e su macOS l'accettazione notarile.
    # La firma riguarda i soli artefatti **nativi**: e' una proprieta' dei
    # binari che una piattaforma esegue, e un pacchetto Python puro non ne ha.
    for piattaforma in piattaforme:
        politica = distribuzione.politica_di_firma(piattaforma, canale)
        if not politica["richiesta"]:
            continue
        for profilo in PROFILI:
            referto = referti.get((piattaforma, profilo, "smoke-profilo"))
            if referto is None:
                continue  # gia' segnalato sopra
            firma = (referto.get("misure") or {}).get("firma", {})
            stato = firma.get("stato")
            if stato == "non_misurata":
                errori.append(
                    f"{piattaforma}/{profilo}: la firma {politica['meccanismo']} e' pretesa e "
                    "nessuno l'ha misurata. «Non ho potuto guardare» non e' «va bene»: sono "
                    "esattamente le due cose che questo gate esiste per distinguere."
                )
            elif stato != "apposta":
                errori.append(
                    f"{piattaforma}/{profilo}: il canale «{canale}» pretende una firma "
                    f"{politica['meccanismo']} e la misura dice «{stato}» "
                    f"(mancanti: {firma.get('mancanti')}). Un artefatto candidate non firmato "
                    "non e' una candidate meno buona: e' un artefatto che chi lo riceve non "
                    "puo' verificare."
                )
            else:
                # Anche `apposta` va riletto: lo stato e' una conclusione, e la
                # conclusione si controlla contro le misure che la sostengono.
                misura = firma.get("misura") or {}
                senza_valore = [
                    p for p in politica.get("misure_pretese", ()) if not misura.get(p)
                ]
                if senza_valore:
                    errori.append(
                        f"{piattaforma}/{profilo}: la firma e' dichiarata apposta e le misure "
                        f"{senza_valore} sono vuote. Lo stato e' una conclusione: le misure "
                        "sono cio' che la sostiene."
                    )
            if firma.get("smoke_prima_della_firma"):
                errori.append(
                    f"{piattaforma}/{profilo}: lo smoke e' stato eseguito prima della firma. "
                    "Un binario firmato -- con una sezione in piu' su Windows, con "
                    "`LC_CODE_SIGNATURE` su macOS -- e' un altro file: lo smoke va rifatto."
                )

    return errori


def main(argv: list[str] | None = None) -> int:
    a = argparse.ArgumentParser(description=__doc__)
    a.add_argument("--referti", required=True, type=pathlib.Path)
    a.add_argument("--canale", default="candidate", choices=["prova", "candidate"])
    a.add_argument(
        "--piattaforme",
        default=None,
        help=(
            "le piattaforme da pretendere. Senza, sono quelle **distribuite** secondo la "
            "matrice: restringerle a mano serve a verificare una corsa parziale, e non "
            "cambia il perimetro"
        ),
    )
    arg = a.parse_args(argv)

    directory = arg.referti.resolve()
    if not directory.is_dir():
        sys.exit(f"{directory} non e' una directory")
    distribuite, fuori = perimetro()
    if arg.piattaforme:
        piattaforme = tuple(p.strip() for p in arg.piattaforme.split(",") if p.strip())
        sconosciute = set(piattaforme) - set(distribuite)
        if sconosciute:
            sys.exit(
                f"piattaforme non distribuite o sconosciute: {sorted(sconosciute)}. "
                f"Distribuite: {sorted(distribuite)}."
            )
    else:
        piattaforme = distribuite

    matrice = json.loads(MATRICE.read_text(encoding="utf-8"))
    dichiarate = classi(matrice)
    artefatti = artefatti_attesi(matrice, piattaforme)
    # Il numero non e' scritto: e' la somma degli obblighi di ciascun artefatto,
    # e ciascun artefatto viene dalla propria classe. Aggiungerne una si vede
    # qui, invece di richiedere una modifica al gate.
    attesi = sum(
        len(attese_per(profilo, classe, dichiarate))
        for _, profilo, classe in artefatti
    )
    print(f"canale: {arg.canale}")
    print(f"piattaforme distribuite: {', '.join(distribuite)}")
    for identita, voce in sorted(fuori.items()):
        print(f"fuori dal perimetro: {identita} -- {voce['decisione']}")
    if piattaforme != distribuite:
        print(f"verifica ristretta a: {', '.join(piattaforme)}")
    per_classe: dict[str, int] = {}
    for _, _, classe in artefatti:
        per_classe[classe] = per_classe.get(classe, 0) + 1
    dettaglio = ", ".join(f"{n} {c}" for c, n in sorted(per_classe.items()))
    print(f"artefatti attesi: {len(artefatti)} ({dettaglio})")
    print(f"referti attesi: {attesi}")

    # `artefatti` e `dichiarate` si passano invece di essere ricalcolati dentro:
    # sono gli stessi che hanno prodotto i due numeri stampati qui sopra, e
    # ricavarli una seconda volta avrebbe permesso al conteggio annunciato e a
    # quello verificato di divergere senza che nessuno lo vedesse.
    errori = verifica(directory, arg.canale, piattaforme, artefatti, dichiarate)
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
