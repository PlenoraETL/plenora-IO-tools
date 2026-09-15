#!/usr/bin/env python3
"""Misura un soak Linux e distingue durata, finding e fallimento della misura.

MONOTONIC e BOOTTIME possono fermarsi insieme nella VM. REALTIME aggiunge il
confronto con la parete risincronizzata: uno scarto invalida la misura, senza
attribuirlo con certezza a una sospensione (anche una correzione dell'orologio
puo' produrlo). La concordanza significa solo nessuna discontinuita' rilevata.
La CPU dei figli include preparazione e build: e' diagnostica, senza soglia.

Il riepilogo del fuzzer deve coprire la durata richiesta: il tempo speso nella
build non puo' completare un soak breve. Un crash puo' non stampare `Done`;
un exit nonzero, da solo, non dimostra invece alcun finding.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import math
import pathlib
import re
import subprocess
import sys
import time

ROOT = pathlib.Path(__file__).resolve().parent.parent
OROLOGI = ("monotonic_s", "boottime_s", "parete_s", "cpu_figli_s")


def giudica(bersaglio: str, secondi: int, codice: int | None,
            tempi: dict, testo: str) -> dict:
    """Giudizio puro: nessun orologio reale e nessuna campagna nelle sonde."""
    if secondi <= 0:
        raise ValueError("la durata richiesta deve essere positiva")
    # Limita i messaggi al singolo target: build e wrapper possono fallire
    # prima di averlo invocato. La riga di annuncio non prova l'avvio.
    sezioni = re.split(rf"^=== {re.escape(bersaglio)}: {secondi}s ===\s*$",
                       testo, flags=re.M)
    log = sezioni[1] if len(sezioni) == 2 else ""
    finali = re.findall(r"^Done (\d+) runs in (\d+) second\(s\)\s*$", log, re.M)
    fine = finali[0] if len(finali) == 1 else None
    segnali = re.findall(
        r"^.*(?:ERROR: (?:libFuzzer|AddressSanitizer|LeakSanitizer):|"
        r"SUMMARY: (?:libFuzzer|AddressSanitizer|UndefinedBehaviorSanitizer):).*$",
        log, re.M)
    avvio = bool(finali or segnali or re.search(r"^#\d+\s+(?:INITED|NEW|REDUCE|pulse)\b", log, re.M))
    stat = dict(re.findall(r"^stat::(\w+):\s+(\S+)", log, re.M))
    problemi = []
    validi = all(isinstance(tempi.get(k), (int, float))
                 and not isinstance(tempi[k], bool)
                 and math.isfinite(tempi[k]) and tempi[k] >= 0 for k in OROLOGI)
    derivati = {}
    diagnostica = {"cpu_figli_s": tempi.get("cpu_figli_s"),
                   "quota_cpu_intervallo": None,
                   "perimetro": "figli del wrapper, preparazione e build incluse; nessuna soglia"}
    if not validi:
        problemi.append("orologi mancanti, negativi o non finiti")
    else:
        mono = tempi["monotonic_s"]
        derivati = {"boottime_meno_monotonic_s": tempi["boottime_s"] - mono,
                    "parete_meno_monotonic_s": tempi["parete_s"] - mono}
        diagnostica["quota_cpu_intervallo"] = tempi["cpu_figli_s"] / mono if mono else None
        if mono < secondi:
            problemi.append("intervallo monotono inferiore alla durata richiesta")
        # Il 5% e' la tolleranza del misuratore storico, dichiarata nel referto.
        # Lo scarto negativo e' altrettanto incompatibile con una misura sana.
        if any(abs(sc) > 0.05 * secondi for sc in derivati.values()):
            problemi.append("discontinuita' fra gli orologi oltre la tolleranza")
        if fine and int(fine[1]) > mono + 0.05 * secondi:
            problemi.append("riepilogo del fuzzer incompatibile con l'intervallo misurato")
    if not fine:
        problemi.append("riepilogo conclusivo assente o multiplo")
    elif int(fine[0]) <= 0 or int(fine[1]) < secondi:
        problemi.append("riepilogo senza esecuzioni o con durata insufficiente")
    if codice != 0:
        problemi.append("campagna non conclusa con exit zero")
    if segnali:
        problemi.append("finding nel log: il soak non e' completato senza finding")
    # None significa che un fallimento resta da classificare, non zero finding.
    finding = True if segnali else (False if codice == 0 and fine else None)
    durata = not problemi
    if finding:
        esito = "finding"
    elif not avvio:
        esito = "avvio_non_dimostrato"
    elif codice != 0 or not fine:
        esito = "interrotta"
    else:
        esito = "misura_invalida" if problemi else "completa"
    return {
        "schema_version": 1, "bersaglio": bersaglio, "secondi_richiesti": secondi,
        "codice_uscita_campagna": codice, "tempi_grezzi": tempi,
        "derivati": derivati, "tolleranza_orologi_s": 0.05 * secondi,
        "diagnostica_cpu": diagnostica, "avvio_osservato": avvio,
        "riepilogo_del_fuzzer": {
            "runs": int(fine[0]) if fine else None,
            "secondi_dichiarati": int(fine[1]) if fine else None,
            "stat": stat},
        "finding": finding, "segnali_finding": segnali,
        "durata_dimostrata": durata, "esito": esito,
        "problemi_della_misura": problemi,
        "limite_della_misura": "concordanza: nessuna discontinuita' rilevata dagli orologi usati",
    }


def campiona() -> dict:
    """Tre orologi Linux e CPU dei figli; importabile anche nelle sonde Windows."""
    import resource

    cpu = resource.getrusage(resource.RUSAGE_CHILDREN)
    return {"monotonic_s": time.clock_gettime(time.CLOCK_MONOTONIC),
            "boottime_s": time.clock_gettime(time.CLOCK_BOOTTIME),
            "parete_s": time.time(), "cpu_figli_s": cpu.ru_utime + cpu.ru_stime}


def misura(bersaglio: str, secondi: int, cartella: pathlib.Path) -> dict:
    """Una cartella nuova per corsa impedisce di sovrascrivere prove precedenti."""
    if secondi <= 0 or not re.fullmatch(r"[a-z][a-z0-9_]*", bersaglio):
        raise ValueError("bersaglio o durata non validi")
    prima = campiona()
    cartella.mkdir(parents=True, exist_ok=False)
    log = cartella / "campagna.log"
    comando = ["bash", "scripts/fuzz-smoke.sh", "--seconds", str(secondi), bersaglio]
    errore = None
    codice = None
    with log.open("xb") as uscita:
        try:
            codice = subprocess.run(comando, cwd=ROOT, stdout=uscita,
                                    stderr=subprocess.STDOUT).returncode
        except OSError as exc:
            errore = str(exc)
    dopo = campiona()
    dati = log.read_bytes()
    referto = giudica(bersaglio, secondi, codice,
                      {k: dopo[k] - prima[k] for k in OROLOGI},
                      dati.decode("utf-8", errors="replace"))
    referto.update({"comando": comando, "log": log.name,
                    "log_sha256": hashlib.sha256(dati).hexdigest(),
                    "misuratore_sha256": hashlib.sha256(pathlib.Path(__file__).read_bytes()).hexdigest(),
                    "campioni": {"prima": prima, "dopo": dopo},
                    "errore_avvio": errore})
    with (cartella / "referto.json").open("x", encoding="utf-8", newline="\n") as f:
        json.dump(referto, f, ensure_ascii=False, indent=2, allow_nan=False)
        f.write("\n")
    return referto


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("bersaglio")
    parser.add_argument("--seconds", type=int, default=3600)
    parser.add_argument("--output-dir", type=pathlib.Path, required=True,
                        help="directory nuova per log e referto, su un percorso persistente")
    args = parser.parse_args(argv)
    if not hasattr(time, "CLOCK_BOOTTIME"):
        parser.error("la raccolta richiede Linux con CLOCK_BOOTTIME")
    try:
        referto = misura(args.bersaglio, args.seconds, args.output_dir)
    except (OSError, ValueError) as exc:
        parser.exit(2, f"misura non eseguita: {exc}\n")
    print(json.dumps({k: referto[k] for k in ("esito", "durata_dimostrata", "finding")}))
    return 0 if referto["esito"] == "completa" else 1


if __name__ == "__main__":
    sys.exit(main())
