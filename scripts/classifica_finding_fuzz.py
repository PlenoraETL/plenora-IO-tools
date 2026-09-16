#!/usr/bin/env python3
"""Classifica il crash di un fuzz target: **noto** o **nuovo**.

# Il problema, scritto prima del meccanismo

> gestire finding noti senza disabilitare il bersaglio, senza nascondere
> difetti nuovi, e senza dichiarare completa una campagna che si e' interrotta.

La quarantena del progetto e' per **bersaglio**: mettere `geoparquet_reader` in
`fuzz/quarantine.txt` smetterebbe di esplorarlo, e il file stesso riserva quella
via ai finding dove «uno smoke che fallisce sempre non e' un gate, e' rumore».
Un finding solo non giustifica di smettere di cercarne altri.

# Perche' una lista di digest non basta

Perche' il fuzzer rigenera lo stesso difetto da un input diverso, o ne produce
una variante. La prova sta negli artefatti locali di `geoparquet_reader`: due
misurano 3.966 byte, la stessa dimensione del seme del finding, con digest tutti
diversi dal suo. Stessa famiglia, quattro digest. Una lista di digest li
riconoscerebbe uno per volta, e ogni variante sarebbe un rosso da triare a mano.

# Che cosa si riconosce invece

La **famiglia**, cioe' il punto in cui il difetto si manifesta:

* il modulo, col nome della crate spogliato della versione -- `parquet-59.3.0`
  e `parquet-60.0.0` sono la stessa crate, e un finding aperto a monte non
  smette di esserlo perche' il pin sale;
* la **forma** del messaggio, con le cifre ridotte a `N` -- «the len is 2 but
  the index is 2» e «the len is 56 but the index is 56» sono lo stesso difetto
  su due ingressi.

Il numero di riga **non** e' nella firma: si sposta fra versioni, e legarlo
renderebbe la voce stantia a ogni aggiornamento. Resta registrato come dato,
perche' chi rilegge la voce voglia vedere dove si era manifestato.

# Perche' non nasconde difetti nuovi

Perche' la firma e' stretta e la corrispondenza e' **congiunta**: modulo **e**
forma del messaggio. Un panico nello stesso modulo con un altro messaggio e' un
finding nuovo; lo stesso messaggio in un altro modulo pure. E qualunque crash
che non corrisponda a nessuna voce fa fallire lo smoke come prima.

# Perche' non dichiara completa una campagna interrotta

Perche' un finding noto **interrompe comunque** il bersaglio: libFuzzer si ferma
al primo crash, e il tempo restante non e' stato esplorato. La classificazione
dice «noto», non «completo». Chi chiama distingue i due stati, e l'esito lo
riporta: un bersaglio fermato a meta' non si conta fra quelli che hanno finito.
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parents[1]
REGISTRO = ROOT / "assurance" / "registries" / "finding-noti-fuzz.json"

CAMPI = (
    "id",
    "bersaglio",
    "modulo",
    "forma_del_messaggio",
    "dove_e_tracciato",
    "non_promette",
    "quando_si_toglie",
)

#: `thread '<nome>' panicked at <percorso>:<riga>:<colonna>:`
PANICO = re.compile(r"panicked at ([^\s:]+(?:/[^\s:]+)*):(\d+):(\d+)")

#: Una crate nel registro di cargo: `nome-1.2.3` oppure `nome-1.2.3-rc.1`.
VERSIONE = re.compile(r"^(?P<nome>.+?)-\d+\.\d+\.\d+(?:[-+][0-9A-Za-z.-]+)?$")


def modulo_normalizzato(percorso: str) -> str:
    """Il percorso col segmento di versione tolto dal nome della crate.

    `.../parquet-59.3.0/src/a/b.rs` diventa `parquet/src/a/b.rs`. Senza questo,
    ogni aggiornamento del pin renderebbe stantia una voce che descrive un
    difetto ancora aperto.
    """
    pezzi = percorso.replace("\\", "/").split("/")
    for indice, pezzo in enumerate(pezzi):
        trovato = VERSIONE.match(pezzo)
        if trovato is None:
            continue
        # Trovata la radice della crate: tutto cio' che sta prima e' il
        # percorso del registro di cargo sulla macchina che ha eseguito, e
        # cambia da macchina a macchina. Tenerlo renderebbe la firma dipendente
        # da dove gira il fuzzer.
        return "/".join([trovato.group("nome"), *pezzi[indice + 1 :]])
    # Nessun segmento con versione: e' codice nostro, e il percorso relativo
    # alla radice del repository e' gia' la forma stabile.
    relativo = "/".join(pezzi)
    radice = f"{ROOT.name}/"
    taglio = relativo.rfind(radice)
    return relativo[taglio + len(radice) :] if taglio != -1 else relativo


def forma_del_messaggio(messaggio: str) -> str:
    """Il messaggio con le cifre ridotte a `N`, cioe' la sua famiglia."""
    return re.sub(r"\d+", "N", messaggio).strip()


def crash_osservato(testo: str) -> dict[str, str] | None:
    """Il primo panico nel testo, ridotto a firma; `None` se non ce n'e'."""
    trovato = PANICO.search(testo)
    if trovato is None:
        return None
    # Il messaggio sta sulla riga **dopo** quella del panico: la riga del
    # panico finisce con i due punti, e prenderne la coda dava una stringa
    # vuota. Si salta percio' al primo a capo e si legge la riga seguente.
    coda = testo[trovato.end() :]
    a_capo = coda.find("\n")
    prima_riga = "" if a_capo == -1 else coda[a_capo + 1 :].split("\n", 1)[0]
    return {
        "modulo": modulo_normalizzato(trovato.group(1)),
        "riga": trovato.group(2),
        "messaggio": prima_riga.strip(),
        "forma_del_messaggio": forma_del_messaggio(prima_riga),
    }


def registro_ben_formato(documento: Any) -> list[str]:
    """I motivi per cui il registro non e' leggibile; vuoto se lo e'."""
    if not isinstance(documento, dict) or not isinstance(
        documento.get("finding"), list
    ):
        return ["il registro non porta un elenco `finding`"]
    motivi: list[str] = []
    visti: set[str] = set()
    for indice, voce in enumerate(documento["finding"]):
        dove = f"finding[{indice}]"
        if not isinstance(voce, dict):
            motivi.append(f"{dove}: non e' un oggetto")
            continue
        for campo in CAMPI:
            if not isinstance(voce.get(campo), str) or not voce[campo].strip():
                motivi.append(f"{dove}: `{campo}` assente o vuoto")
        identita = voce.get("id")
        if isinstance(identita, str):
            if identita in visti:
                motivi.append(f"{dove}: `id` «{identita}» ripetuto")
            visti.add(identita)
    return motivi


def classifica(bersaglio: str, testo: str, documento: Any) -> dict[str, Any]:
    """`stato` fra `senza-crash`, `noto` e `nuovo`, con cio' che lo sostiene."""
    osservato = crash_osservato(testo)
    if osservato is None:
        return {"stato": "senza-crash"}

    for voce in documento["finding"]:
        if voce["bersaglio"] != bersaglio:
            continue
        # Congiunta, e non «una delle due»: lo stesso messaggio in un altro
        # modulo e' un difetto diverso, e un altro messaggio nello stesso modulo
        # pure. Allentarla qui sarebbe il modo di nascondere un finding nuovo
        # dietro uno noto.
        if (
            voce["modulo"] == osservato["modulo"]
            and voce["forma_del_messaggio"] == osservato["forma_del_messaggio"]
        ):
            return {"stato": "noto", "id": voce["id"], "osservato": osservato}
    return {"stato": "nuovo", "osservato": osservato}


def main(argv: list[str] | None = None) -> int:
    argomenti = argparse.ArgumentParser(description=__doc__)
    argomenti.add_argument("bersaglio", help="il fuzz target che ha girato")
    argomenti.add_argument(
        "--uscita",
        type=Path,
        required=True,
        help="il file con l'output della corsa, stdout e stderr insieme",
    )
    opzioni = argomenti.parse_args(argv)

    if not REGISTRO.exists():
        print(f"{REGISTRO}: registro assente", file=sys.stderr)
        return 2
    documento = json.loads(REGISTRO.read_text(encoding="utf-8"))
    motivi = registro_ben_formato(documento)
    if motivi:
        for motivo in motivi:
            print(f"registro dei finding noti: {motivo}", file=sys.stderr)
        return 2

    if not opzioni.uscita.is_file():
        print(f"{opzioni.uscita}: uscita della corsa assente", file=sys.stderr)
        return 2
    testo = opzioni.uscita.read_text(encoding="utf-8", errors="replace")
    esito = classifica(opzioni.bersaglio, testo, documento)

    # I tre stati hanno tre codici d'uscita, perche' chi chiama deve poterli
    # distinguere senza rileggere il testo: 0 nessun crash, 3 crash noto,
    # 1 crash nuovo. Un noto non e' un successo, ed e' il motivo per cui non
    # esce 0.
    if esito["stato"] == "senza-crash":
        print(f"{opzioni.bersaglio}: nessun crash nell'uscita")
        return 0
    if esito["stato"] == "noto":
        voce = next(
            v for v in documento["finding"] if v["id"] == esito["id"]
        )
        print(
            f"{opzioni.bersaglio}: finding NOTO «{esito['id']}» "
            f"({voce['dove_e_tracciato']}). {voce['non_promette']} "
            f"Il bersaglio si e' fermato qui: il tempo restante non e' stato "
            f"esplorato."
        )
        return 3
    osservato = esito["osservato"]
    print(
        f"{opzioni.bersaglio}: finding NUOVO in {osservato['modulo']}"
        f":{osservato['riga']} -- {osservato['messaggio']}",
        file=sys.stderr,
    )
    return 1


if __name__ == "__main__":
    raise SystemExit(main())
