#!/usr/bin/env python3
"""Costruisce l'evidenza di un checkpoint S9 **dai log della corsa**.

# Il difetto che chiude

L'evidenza era trascritta a mano dal sommario che la corsa stampa a schermo. Due
cose andavano storte, e sono andate storte davvero.

La prima: il sommario non porta i numeri. Dice `fuzz_replay verde`, non quanti
input siano stati rieseguiti; e i numeri stanno nei log dei passi, che vivono
nella directory di corsa. Una corsa lanciata in un container `--rm` senza
montarla se li porta via, e allora l'evidenza non si puo' scrivere: e' successo
il 2026-09-06, ed e' costata una rimisura intera.

La seconda: trascrivere a mano da un rapporto significa che una cifra sbagliata
non si distingue da una misura diversa. L'evidenza serve proprio a dire quali
numeri ha prodotto una corsa; se li scrive chi la racconta invece di chi la
esegue, dice qualcos'altro.

# Che cosa deriva e che cosa no

Deriva tutto cio' che e' **misurato**: revisioni, impronte, l'elenco dei passi
coi loro esiti, gli input del fuzz, i target, i finding, la copertura, e il
manifesto dei file di log col proprio digest.

Non deriva cio' che e' **giudizio**: che cosa la corsa chiude, che cosa resta
aperto, perche' la candidate e' cambiata. Quello lo scrive chi decide, e va
passato in un file a parte -- che questo strumento inserisce senza toccarlo.
La distinzione e' la ragione per cui l'evidenza non e' generata per intero: una
misura si estrae, una conclusione si prende.

# Perche' il digest dei log

Perche' l'evidenza dice «questi numeri vengono da quella corsa», e senza
un'impronta della corsa la frase non e' verificabile. Il manifesto elenca ogni
file di log con la propria dimensione e il proprio digest, e il digest del
manifesto lega l'evidenza a quell'insieme preciso.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parents[1]

#: Le righe di riepilogo che i passi stampano alla fine del proprio log.
#:
#: Si leggono quelle e non l'uscita grezza degli strumenti: le stampa il nostro
#: script, dopo aver contato, ed e' il punto in cui la misura e' gia' stata
#: fatta. Analizzare l'uscita di libFuzzer vorrebbe dire rifare quel conteggio
#: qui, con il rischio di farlo diversamente.
REPLAY = re.compile(
    r"replay completato: (?P<input>\d+) input rieseguiti su (?P<target>\d+) "
    r"target, (?P<crash>nessun crash|\d+ crash)"
)
SMOKE = re.compile(
    r"smoke fuzz completato (?P<esito>senza finding|con \d+ finding) su "
    r"(?P<target>\d+) target eseguiti \((?P<quarantena>\d+) in quarantena"
)
LCOV = re.compile(
    r"copertura di riga dal report: (?P<percentuale>[\d.]+)% "
    r"\((?P<coperte>\d+)/(?P<strumentate>\d+) righe strumentate\)"
)
#: Quanti bersagli il fuzz **dichiara**, contro quanti ne esegue.
#:
#: I due numeri servono insieme: «quindici eseguiti» da solo non dice se ne
#: mancasse uno. La riga viene dalla build strumentata, che li compila tutti.
TARGET_DICHIARATI = re.compile(r"tutti i (?P<totali>\d+) target dichiarati")

#: La soglia che il comando di copertura impone, letta dal comando stesso.
#:
#: Sta nell'invocazione di `cargo llvm-cov` che il checkpoint esegue, e si
#: estrae da li' invece di ricopiarla: un numero scritto due volte diverge, e
#: quello che conta e' quello che ha davvero fatto fallire o passare la corsa.
SOGLIA = re.compile(r"--fail-under-lines[= ]+(?P<soglia>\d+)")

#: La riga `TOTAL` del sommario di `cargo llvm-cov`, colonna delle righe.
TOTALE_CARGO = re.compile(
    r"^TOTAL\s+\d+\s+\d+\s+[\d.]+%\s+\d+\s+\d+\s+[\d.]+%\s+(?P<righe>\d+)\s+"
    r"(?P<scoperte>\d+)\s+(?P<percentuale>[\d.]+)%",
    re.MULTILINE,
)


#: Il riepilogo della diagnostica differenziale, e le righe che nomina.
#:
#: `righe_scoperte` e' l'elenco e non il conteggio: i numeri dicono **quante**,
#: e senza i nomi nessun artefatto della corsa dice **quali**. E' il motivo per
#: cui il passo gira con `--mostra 0`.
DIFFERENZIALE = re.compile(
    r"copertura delle righe cambiate fra (?P<base>\S+) e (?P<head>\S+)"
    r".*?righe cambiate ed eseguibili:\s+(?P<cambiate>\d+)"
    r".*?coperte:\s+(?P<coperte>\d+)"
    r".*?scoperte:\s+(?P<scoperte>\d+)"
    r".*?cambiate ma non eseguibili:\s+(?P<non_eseguibili>\d+)"
    r".*?percentuale:\s+(?P<percentuale>[\d.]+)%",
    re.DOTALL,
)
RIGA_SCOPERTA = re.compile(r"^  (?P<riga>\S+:\d+)$", re.MULTILINE)


def sha256(dati: bytes) -> str:
    return hashlib.sha256(dati).hexdigest()


def cerca(schema: re.Pattern[str], percorso: pathlib.Path) -> re.Match[str] | None:
    if not percorso.is_file():
        return None
    return schema.search(percorso.read_text(encoding="utf-8", errors="replace"))


def misure(corsa: pathlib.Path) -> tuple[dict, list[str]]:
    """Le misure della corsa, e i motivi per cui qualcuna manca.

    Una misura assente **non** diventa uno zero: torna fra i motivi, e chi
    registra l'evidenza decide che farne. Uno zero silenzioso direbbe «misurato,
    e nessun crash», che e' l'affermazione opposta a «non misurato».
    """
    fuori: dict = {}
    motivi: list[str] = []

    replay = cerca(REPLAY, corsa / "fuzz_replay.log")
    if replay is None:
        motivi.append("fuzz_replay: nessuna riga di riepilogo nel log")
    else:
        fuori["fuzz_replay"] = {
            "input": int(replay["input"]),
            "target": int(replay["target"]),
            "crash": 0 if replay["crash"] == "nessun crash" else int(
                replay["crash"].split()[0]
            ),
            "quando": "dentro il checkpoint, passo `fuzz_replay`",
        }

    smoke = cerca(SMOKE, corsa / "fuzz_smoke.log")
    if smoke is None:
        motivi.append("fuzz_smoke: nessuna riga di riepilogo nel log")
    else:
        fuori["fuzz_smoke"] = {
            "target_eseguiti": int(smoke["target"]),
            "finding": 0 if smoke["esito"] == "senza finding" else int(
                smoke["esito"].split()[1]
            ),
            "quarantena": int(smoke["quarantena"]),
            "quando": "dentro il checkpoint, passo `fuzz_smoke`",
        }

    if "fuzz_smoke" in fuori:
        dichiarati = cerca(TARGET_DICHIARATI, corsa / "fuzz_replay.log")
        if dichiarati is None:
            motivi.append(
                "fuzz_smoke: la build strumentata non dice quanti target siano "
                "dichiarati, e senza quel numero «eseguiti» non si sa se sia tutti"
            )
        else:
            fuori["fuzz_smoke"]["target_totali"] = int(dichiarati["totali"])
        fuori["fuzz_replay"]["target_totali"] = fuori["fuzz_smoke"].get(
            "target_totali"
        )

    lcov = cerca(LCOV, corsa / "coverage_soglia_dal_report.log")
    cargo = cerca(TOTALE_CARGO, corsa / "coverage_soglia_controprova.log")
    if lcov is None:
        motivi.append("copertura: nessuna percentuale nel log della soglia")
    else:
        copertura = {
            "lcov_percentuale": float(lcov["percentuale"]),
            "lcov_righe_coperte": int(lcov["coperte"]),
            "lcov_righe_strumentate": int(lcov["strumentate"]),
        }
        if cargo is None:
            motivi.append(
                "copertura: la controprova non porta una riga `TOTAL` da cui "
                "leggere la percentuale di `cargo llvm-cov`"
            )
        else:
            copertura["cargo_lines_percentuale"] = float(cargo["percentuale"])
        soglia = cerca(SOGLIA, corsa / "coverage_soglia_controprova.log") or cerca(
            SOGLIA, corsa / "coverage_soglia_dal_report.log"
        )
        if soglia is None:
            # La soglia sta nell'invocazione del checkpoint: se il log non la
            # riporta, si legge dallo script che l'ha eseguita -- che e'
            # comunque la fonte, non una copia.
            testo = (ROOT / "scripts" / "s9-checkpoint.sh").read_text(
                encoding="utf-8"
            )
            soglia = SOGLIA.search(testo)
        if soglia is None:
            motivi.append(
                "copertura: nessuna soglia ne' nei log ne' nel checkpoint"
            )
        else:
            copertura["soglia"] = float(soglia["soglia"])
        fuori["copertura"] = copertura

    # La diagnostica differenziale gira solo con `S9_CHECKPOINT_BASE`
    # impostata. Quando manca il log, non si registra «zero righe scoperte»:
    # direbbe che e' stata misurata e non ha trovato niente, che e'
    # l'affermazione opposta a «non e' stata misurata».
    diagnostica = corsa / "coverage_diff.log"
    trovata = cerca(DIFFERENZIALE, diagnostica)
    if trovata is None:
        motivi.append(
            "diagnostica differenziale: nessun `coverage_diff.log` con un "
            "riepilogo. Il passo gira solo con `S9_CHECKPOINT_BASE` impostata "
            "alla revisione dell'ultimo checkpoint superato"
        )
    else:
        testo = diagnostica.read_text(encoding="utf-8", errors="replace")
        _, _, coda = testo.partition("righe cambiate e mai eseguite:")
        fuori["diagnostica_differenziale"] = {
            "base": trovata["base"],
            "esito": f"{trovata['percentuale']}%",
            "righe_cambiate_eseguibili": int(trovata["cambiate"]),
            "coperte": int(trovata["coperte"]),
            "scoperte": int(trovata["scoperte"]),
            "cambiate_non_eseguibili": int(trovata["non_eseguibili"]),
            "righe_scoperte": [m["riga"] for m in RIGA_SCOPERTA.finditer(coda)],
        }

    return fuori, motivi


def manifesto_dei_log(corsa: pathlib.Path) -> dict:
    """Ogni file della directory di corsa, col proprio digest.

    E' cio' che lega l'evidenza a **quella** corsa: i numeri qui sopra vengono
    da questi file, e il digest del manifest li identifica tutti insieme.

    La forma -- una mappa da percorso relativo a sha256, e l'impronta come
    `percorso NUL sha256 LF` in ordine di percorso -- e' quella che
    `check_release_contract.digest_del_manifest` ricalcola. Non e' una scelta
    di questo strumento: e' la sola forma che il gate sa rifare, e un digest che
    nessuno puo' ricalcolare e' una stringa, non un'impronta.
    """
    manifest = {
        percorso.relative_to(corsa).as_posix(): sha256(percorso.read_bytes())
        for percorso in sorted(p for p in corsa.rglob("*") if p.is_file())
    }
    accumulatore = hashlib.sha256()
    for percorso in sorted(manifest):
        accumulatore.update(percorso.encode("utf-8"))
        accumulatore.update(b"\0")
        accumulatore.update(manifest[percorso].encode("ascii"))
        accumulatore.update(b"\n")
    return {
        "percorso_di_corsa": str(corsa),
        "file": len(manifest),
        "digest_manifest": accumulatore.hexdigest(),
        "forma_del_manifest": (
            "percorso relativo alla directory di corsa -> sha256 del contenuto; "
            "il digest e' `percorso NUL sha256 LF` concatenati in ordine di "
            "percorso, la stessa forma che il gate del contratto ricalcola"
        ),
        "manifest": manifest,
    }


def main(argv: list[str] | None = None) -> int:
    argomenti = argparse.ArgumentParser(description=__doc__)
    argomenti.add_argument(
        "--corsa",
        required=True,
        type=pathlib.Path,
        help="la directory di corsa del checkpoint, con `risultato.json` e i log",
    )
    argomenti.add_argument(
        "--giudizio",
        required=True,
        type=pathlib.Path,
        help=(
            "il JSON con la parte non misurata: `descrizione`, `non_significa`, "
            "`cosa_chiude`, `resta_aperto`, e quanto altro chi registra decide"
        ),
    )
    argomenti.add_argument("--uscita", required=True, type=pathlib.Path)
    opzioni = argomenti.parse_args(argv)

    risultato = json.loads(
        (opzioni.corsa / "risultato.json").read_text(encoding="utf-8")
    )
    if risultato.get("esito") == "in_corso":
        print(
            "la corsa non e' finita: `risultato.json` dice «in_corso». "
            "Un'evidenza scritta ora descriverebbe una misura a meta'.",
            file=sys.stderr,
        )
        return 1
    if risultato.get("livello") != 2:
        print(
            f"la corsa e' di livello {risultato.get('livello')}: il livello 1 "
            "omette fuzz e copertura, e non produce un'evidenza.",
            file=sys.stderr,
        )
        return 1

    giudizio = json.loads(opzioni.giudizio.read_text(encoding="utf-8"))
    # La prosa che **accompagna** una misura -- come e' stata presa, che cosa
    # significa un numero -- non si estrae da un log: la scrive chi registra. Si
    # fonde qui invece di stare in una sezione a parte perche' il gate confronta
    # stato ed evidenza campo per campo, e due posti diversi per lo stesso campo
    # sono due verita' che divergono.
    prosa = giudizio.pop("prosa_delle_misure", {})

    valori, motivi = misure(opzioni.corsa)
    if motivi:
        for motivo in motivi:
            print(f"misura assente -- {motivo}", file=sys.stderr)
        print(
            "\nUn'evidenza senza le proprie misure non e' un'evidenza: o i log "
            "ci sono, o la corsa va rifatta conservandoli.",
            file=sys.stderr,
        )
        return 1

    # `passi` e' il **conteggio**; l'elenco sta in `elenco_dei_passi`. Sono due
    # campi diversi con nomi che si somigliano, e leggere il primo come una
    # lista produce un `TypeError` invece di un numero sbagliato -- il che, fra i
    # due modi di sbagliare, e' quello buono.
    passi = risultato.get("elenco_dei_passi") or []
    evidenza = {
        "schema_version": 1,
        **giudizio,
        "corsa": {
            "revisione_iniziale": risultato["revisione_iniziale"],
            "revisione_finale": risultato["revisione_finale"],
            "impronta_iniziale": risultato["impronta_iniziale"],
            "impronta_finale": risultato["impronta_finale"],
            "albero_all_avvio": (
                f"pulito: {risultato['file_sporchi_all_avvio']} file sporchi."
            ),
            "registro_dei_passi": "assurance/registries/passi-del-checkpoint.json",
            "percorso_dei_log": risultato.get("log", str(opzioni.corsa)),
        },
        "riconciliazione": {
            "identificatori_distinti": len({p["id"] for p in passi}),
            "eseguiti": len(passi),
            "eseguiti_dichiarati_dalla_corsa": risultato.get("passi"),
            "verdi": sum(1 for p in passi if p.get("esito") == "verde"),
            "omessi": sum(1 for p in passi if p.get("esito") == "omesso"),
            "falliti": sum(1 for p in passi if p.get("esito") not in ("verde", "omesso")),
            "duplicati": len(passi) - len({p["id"] for p in passi}),
            "metodo": (
                "i conteggi sono **derivati** dall'elenco dei passi che la corsa "
                "ha registrato in `risultato.json`, non contati a mano dal "
                "sommario a schermo"
            ),
            "insieme_dichiarato": "assurance/registries/passi-del-checkpoint.json",
            "passi": passi,
        },
        "misure": {
            nome: {**misura, **prosa.get(nome, {})}
            for nome, misura in valori.items()
        },
        "artefatti": manifesto_dei_log(opzioni.corsa),
    }
    # `risultato.json` dice «superato»; l'evidenza porta la frase che la corsa
    # stampa e che lo stato pretende. Sono due vocabolari per lo stesso fatto, e
    # la corrispondenza si dichiara qui invece di lasciarla a chi trascrive.
    ESITI = {"superato": "S9 checkpoint level 2 passed"}
    if risultato["esito"] not in ESITI:
        print(
            f"esito «{risultato['esito']}» non riconosciuto: sono {sorted(ESITI)}. "
            "Un'evidenza non inaugura un vocabolario nuovo.",
            file=sys.stderr,
        )
        return 1
    evidenza["esito"] = ESITI[risultato["esito"]]

    opzioni.uscita.write_text(
        json.dumps(evidenza, ensure_ascii=False, indent=2) + "\n", encoding="utf-8"
    )
    r = evidenza["riconciliazione"]
    print(
        f"evidenza scritta in {opzioni.uscita}: "
        f"{r['verdi']}/{r['eseguiti']} passi verdi, "
        f"{valori['fuzz_replay']['input']} input rieseguiti, "
        f"copertura {valori['copertura']['lcov_percentuale']}%"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
