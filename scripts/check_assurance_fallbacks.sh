#!/usr/bin/env bash
set -eu

# Registro conservativo dell'intero workspace: include anche i moduli
# #[cfg(test)] e i target non distribuibili. Ogni nuova occorrenza richiede una
# revisione H-01 e l'aggiornamento esplicito del registro.
#
# ATTENZIONE — questo registro misura la SINTASSI, non la semantica. La regex
# conta solo la forma `unwrap_or*(`, quindi NON vede due forme equivalenti:
#   - `map_or(default, f)`, che esprime lo stesso fallback;
#   - il `match` esplicito `match x { Some(v) => v, None => default }`.
# Di conseguenza un lint che riscrive fra queste forme sposta i conteggi senza
# che cambi un solo comportamento.
#
# Il 2026-08-06 la bonifica lint (pedantic + nursery a deny) ha fatto
# esattamente questo in dieci crate, nei due sensi: totale da 99 a 93.
#   - in calo per `map_unwrap_or`: `map(f).unwrap_or(d)` -> `map_or(d, f)`
#     (dxf -3, io-cli -2, geoparquet/gpkg/ipc/kml/bench/fuzz -1 ciascuno);
#   - in aumento per `manual_unwrap_or`: `match` esplicito -> `unwrap_or*`
#     (filegdb +2 nell'example projection_bench, io-core +2, shp +1).
# Ogni delta e' stato verificato riga per riga: nessun fallback e' stato
# introdotto o rimosso, solo riscritto. Nessuna revisione H-01 dovuta.
#
# Il punto aperto e' il meccanismo: finche' il registro conta una forma
# sintattica, un fallback nuovo scritto come `map_or` o come `match` entra
# senza passare da H-01. Renderlo semantico non e' una correzione meccanica
# ed e' una decisione di policy.
#
# Il 2026-08-07 la barriera sui panic di arrow ha aggiunto un'occorrenza in
# plenora-io-core (10 -> 11, totale 93 -> 94): in `messaggio_del_panico`, il
# payload di un panico che non e' ne' `&'static str` ne' `String` diventa
# "panico senza messaggio". Il fallback e' sul testo diagnostico, non sul
# dato: qualunque sia il payload, la lettura fallisce comunque e nessuna riga
# viene prodotta. Riscriverlo come `match` per non far muovere il contatore
# era possibile ed e' stato scartato: sarebbe stato esattamente il modo di
# eludere H-01 che questo commento denuncia. **Serve ratifica H-01.**
#
# Il 2026-08-16 la barriera anti-panic del binario CLI ha aggiunto la stessa
# occorrenza in plenora-io-cli (18 -> 19, totale 94 -> 95): in
# `messaggio_del_panico`, il payload di un panico che non e' ne'
# `&'static str` ne' `String` diventa "panico senza messaggio". E' lo stesso
# fallback gia' analizzato per plenora-io-core, applicato al confine del
# binario invece che a quello della libreria, e ha la stessa natura: cade sul
# testo diagnostico, non sul dato. Qualunque sia il payload il processo sta
# comunque terminando per panico, l'envelope `plenora-io-error-v1` viene
# emesso ugualmente e nessun valore derivato dall'input entra nel messaggio,
# che porta solo l'impronta FNV-1a. Anche qui riscriverlo come `match` per non
# far muovere il contatore sarebbe stato il modo di eludere H-01.
# **Serve ratifica H-01, con la stessa motivazione dell'occorrenza di
# plenora-io-core.**
#
# Il 2026-08-16 il benchmark A/B dello spool ha aggiunto tre occorrenze in
# plenora-bench (21 -> 24, totale 95 -> 98). Sono default di argomenti da
# riga di comando dell'harness — directory dei fixture, numero di righe,
# nome della variante — non fallback su dati letti da un file. plenora-bench
# non e' codice spedito, non entra nel gate anti-panic e non produce output
# che qualcuno consumi: un default sbagliato qui fa misurare la cosa
# sbagliata, e il benchmark lo dichiara nel proprio JSON. Nessuna revisione
# H-01 dovuta.
#
# Sempre il 2026-08-16 lo spool bounded ha aggiunto un'occorrenza in
# plenora-io-core (11 -> 12, totale 98 -> 99): in `write_failure`, quando una
# scrittura sul file di spool fallisce e il guardiano della quota non ha
# registrato un rifiuto tipizzato, l'errore diventa quello generico di
# scrittura. Il fallback e' sulla *classificazione* dell'errore, non sul
# dato: l'operazione fallisce comunque e nessun batch raggiunge il consumer.
# Il ramo coperto e' il fallimento per una ragione del filesystem invece che
# per quota. E' raccolto in un helper unico proprio per non moltiplicarsi sui
# quattro punti di scrittura. Nessuna revisione H-01 dovuta.
#
# Il 2026-08-16 l'handoff della memoria (S4.d) ha aggiunto tre occorrenze in
# plenora-io-core (12 -> 15, totale 99 -> 102). Nessuna e' un fallback su
# dati letti da un file:
#
#   1. `cell_bytes_u64` converte il tetto per cella da `usize` a `u64` per
#      sommarlo al target del batch. Su un target a 64 bit la conversione non
#      puo' fallire; il saturante esiste per architetture ipotetiche a 128
#      bit, dove `u64::MAX` resterebbe comunque un tetto piu' stretto della
#      memoria disponibile — quindi fail-closed, non permissivo.
#   2. e 3. sono in moduli `#[cfg(test)]`: la fabbrica di budget dello spool
#      e il reader di sequenza che restituisce `Ok(None)` a coda vuota. Non
#      sono codice spedito e non entrano nel gate anti-panic.
#
# Nello stesso commit ne sono state tolte quattro evitabili, nate dalla
# traduzione meccanica dei test: `usize::try_from` su un letterale, e una
# conversione eliminata cambiando il tipo del contatore in `AtomicU64`.
# Nessuna revisione H-01 dovuta per le tre residue.
expected='
driver-csv 3
driver-dxf 15
driver-filegdb 5
driver-geojson 1
driver-geoparquet 3
driver-gpkg 3
driver-ipc 1
driver-kml 3
driver-shp 3
driver-xls 1
plenora-io-model 1
plenora-io-core 15
plenora-io-cli 19
plenora-bench 24
plenora-fuzz 5
'

actual_total=0
while read -r crate expected_count; do
    if [ -z "${crate}" ]; then
        continue
    fi
    if command -v rg >/dev/null 2>&1; then
        actual_count=$(
            rg -o 'unwrap_or(_else|_default)?\(' "crates/${crate}" -g '*.rs' |
                wc -l |
                tr -d ' '
        )
    else
        actual_count=$(
            grep -R -E -o --include='*.rs' 'unwrap_or(_else|_default)?\(' "crates/${crate}" |
                wc -l |
                tr -d ' '
        )
    fi
    if [ "${actual_count}" != "${expected_count}" ]; then
        echo "${crate}: fallback registrati=${expected_count}, trovati=${actual_count}" >&2
        exit 1
    fi
    actual_total=$((actual_total + actual_count))
done <<EOF
${expected}
EOF

if [ "${actual_total}" -ne 102 ]; then
    echo "totale fallback del workspace inatteso: ${actual_total}" >&2
    exit 1
fi

echo "fallback assurance verificati: ${actual_total}"
