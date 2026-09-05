#!/usr/bin/env bash
# Checkpoint per S9 (design § 20). Due modalita', **una sola definizione dei
# passi**.
#
#   bash scripts/s9-checkpoint.sh              livello 2, la corsa completa
#   S9_LIVELLO=1 bash scripts/s9-checkpoint.sh livello 1, per una tranche
#
# La modalita' di livello 1 esiste per un difetto trovato il 2026-08-21: la
# batteria intermedia veniva composta a mano a ogni tranche, e non conteneva
# `cargo fmt --check`. Quattro commit sono stati dichiarati «verificati a
# livello 1» essendo piu' deboli del checkpoint di esattamente quel passo, e
# leggendo l'esito nessuno poteva accorgersene: la batteria stampava «nessun
# fallito» su un insieme di passi piu' piccolo di quello che il livello 1
# comprende.
#
# La lezione non e' «ricordarsi di formattare», ed e' la ragione per cui questa
# modalita' vive qui dentro invece che in un secondo script:
#
#   una batteria composta a mano diverge dal checkpoint, e diverge in silenzio.
#
# Il livello 1 **non e'** un sottoinsieme scelto a mano: esegue tutti i passi
# tranne quelli marcati `passo_pesante`, cioe' fuzz e copertura. Aggiungere un
# passo al livello 2 lo aggiunge al livello 1 per omissione, che e' il verso
# giusto dell'errore.
#
# Esiste come script versionato, e non come comando improvvisato, per una
# ragione trovata sul campo: la prima esecuzione del 2026-08-20 girava da un
# harness ad hoc, e quell'harness aveva tre difetti che nessuno poteva vedere
# perche' non stava in nessun repository. Uno dei tre ha quasi fatto archiviare
# un finding reale.
#
# I tre difetti, e come sono chiusi qui:
#
#   1. lo smoke 13/13 veniva eseguito **due volte** — una dentro la batteria e
#      una a parte — per quaranta minuti che non aggiungevano nulla. Qui la
#      batteria **non** include `fuzz-smoke.sh`: lo smoke e' un passo dichiarato,
#      eseguito una volta sola;
#
#   2. il log di un gate rosso veniva troncato a sei righe, e i dettagli del
#      crash andavano persi. Qui **ogni** passo scrive un log completo su file,
#      il file resta, e il percorso viene stampato quando il passo fallisce;
#
#   3. il gate sulle esclusioni di copertura girava **prima** che il report LCOV
#      esistesse, quindi era rosso a ogni corsa per assenza dell'input. Un rosso
#      che si ripete sempre smette di essere letto: qui la copertura e' generata
#      prima, e il gate legge un report che c'e'.
#
# L'esito di ogni passo viene dal comando, mai da una pipe: nessun `| tail`
# decide se qualcosa e' andato bene.

set -u

cd "$(dirname "$0")/.."

LOG_DIR="${S9_CHECKPOINT_LOG_DIR:-/tmp/s9-checkpoint}"
mkdir -p "${LOG_DIR}"

# Il risultato della corsa, su disco.
#
# L'esito viveva **solo sullo stdout**. La corsa del 2026-08-21 lo ha mostrato
# nel modo piu' chiaro possibile: il container girava con `--rm`, ha portato via
# la directory degli artefatti, e il verdetto e' stato osservato mentre le
# misure sono sparite. Non si combinano il verdetto di una corsa e i numeri di
# un'altra, quindi quella corsa e' stata **scartata per intero**.
#
# Una corsa interrotta lasciava anche meno: nessuna traccia. Chi la ritrovava
# non poteva distinguere «non e' mai partita» da «e' morta a meta'», e le due
# si chiudono in modi diversi.
RISULTATO="${S9_CHECKPOINT_RISULTATO:-${LOG_DIR}/risultato.json}"
mkdir -p "$(dirname "${RISULTATO}")"

passi=0
verdi=0
omessi=0
falliti=()
catena_rotta=""

# L'identita' di ogni passo, con il proprio esito e il proprio log.
#
# I contatori dicono **quanti**; questo elenco dice **quali**. La differenza
# conta: il manifest degli artefatti poteva essere ridotto a due file mentre la
# riconciliazione continuava a dichiarare 57/57, perche' nulla legava i due.
# Ogni voce e' `id`, `esito` e il nome del log — vuoto per i passi in linea, che
# non ne scrivono uno.
passi_registrati=()

registra_passo() {
    passi_registrati+=("$1|$2|$3")
}

# Il registro canonico dei passi, condiviso con il verificatore dell'evidenza.
#
# Il verificatore controllava perfettamente cio' che l'evidenza dichiarava, e
# non sapeva che `fmt` dovesse esistere: togliere un gate e aggiornare
# coerentemente contatori, artefatti e digest passava. Un elenco che vive in un
# posto solo chiude la classe, e lo chiude da **entrambi i lati** — qui la
# corsa dice se ha eseguito cio' che doveva, li' l'evidenza dice se descrive
# cio' che esiste.
REGISTRO_DEI_PASSI="assurance/registries/passi-del-checkpoint.json"

# Confronta gli identificatori eseguiti con quelli dichiarati. Stampa le
# differenze e restituisce 1 se ce ne sono.
insieme_dei_passi_dichiarato() {
    # Il programma si legge in una variabile e si passa con `-c`: con
    # `python3 -` il heredoc **occuperebbe lo stdin**, e la pipe con gli
    # identificatori eseguiti verrebbe scartata in silenzio. Il confronto
    # direbbe allora che nessun passo e' stato eseguito, cioe' sarebbe rosso
    # sempre e per la ragione sbagliata.
    local programma
    programma="$(cat <<'PYTHON'
import json, sys

dichiarati = [
    voce["id"]
    for voce in json.load(open(sys.argv[1], encoding="utf-8"))["passi"]
]
eseguiti = [riga.split("|", 1)[0] for riga in sys.stdin.read().splitlines() if riga]

mancanti = [identita for identita in dichiarati if identita not in eseguiti]
estranei = [identita for identita in eseguiti if identita not in dichiarati]
ripetuti = sorted({i for i in eseguiti if eseguiti.count(i) > 1})

for nome, elenco in (
    ("mai eseguiti", mancanti),
    ("eseguiti ma non dichiarati", estranei),
    ("eseguiti piu' di una volta", ripetuti),
):
    if elenco:
        print(f"    {nome}: {elenco}")

# L'ordine e' dichiarato, quindi si confronta: presenza, estranei e duplicati
# non lo vedono, e due passi scambiati lasciano l'insieme identico. Ci si ferma
# alla prima posizione divergente.
fuori_ordine = ""
if not (mancanti or estranei or ripetuti) and eseguiti != dichiarati:
    for posizione, (osservato, atteso) in enumerate(zip(eseguiti, dichiarati), 1):
        if osservato != atteso:
            fuori_ordine = (
                f"    ordine divergente alla posizione {posizione}: "
                f"'{osservato}' dove il registro dichiara '{atteso}'"
            )
            print(fuori_ordine)
            break

sys.exit(1 if mancanti or estranei or ripetuti or fuori_ordine else 0)
PYTHON
)"
    printf '%s
' ${passi_registrati[@]+"${passi_registrati[@]}"} |
        python3 -c "${programma}" "${REGISTRO_DEI_PASSI}"
}

# `1` = livello 1: si omettono fuzz e copertura, non i gate.
LIVELLO="${S9_LIVELLO:-2}"

# Il risultato in forma strutturata, dallo stato corrente dei contatori.
#
# Le variabili di coda possono non esistere ancora — una corsa interrotta non
# ha una revisione finale — e valgono la stringa vuota: un campo vuoto dice
# «non acquisito», che e' cio' che si vuole sapere.
_risultato_json() {
    local esito="$1" primo nome
    printf '{\n'
    printf '  "schema_version": 1,\n'
    printf '  "descrizione": "Risultato della corsa del checkpoint S9. Scritto in modo atomico: il file appare gia\u0027 completo, perche\u0027 viene prodotto a parte e poi rinominato. Un lettore non puo\u0027 osservarne meta\u0027.",\n'
    printf '  "livello": %s,\n' "${LIVELLO}"
    printf '  "esito": "%s",\n' "${esito}"
    printf '  "revisione_iniziale": "%s",\n' "${REVISIONE:-}"
    printf '  "revisione_finale": "%s",\n' "${REVISIONE_FINE:-}"
    printf '  "impronta_iniziale": "%s",\n' "${IMPRONTA_INIZIO:-}"
    printf '  "impronta_finale": "%s",\n' "${IMPRONTA_FINE:-}"
    printf '  "file_sporchi_all_avvio": %s,\n' "${SPORCHI:-0}"
    printf '  "passi": %s,\n' "${passi}"
    printf '  "verdi": %s,\n' "${verdi}"
    printf '  "omessi": %s,\n' "${omessi}"
    printf '  "falliti": ['
    primo=1
    for nome in ${falliti[@]+"${falliti[@]}"}; do
        [ "${primo}" -eq 1 ] || printf ', '
        primo=0
        printf '"%s"' "${nome}"
    done
    printf '],\n'
    printf '  "artefatti": "%s",\n' "${LOG_DIR}"
    printf '  "elenco_dei_passi": ['
    primo=1
    for nome in ${passi_registrati[@]+"${passi_registrati[@]}"}; do
        [ "${primo}" -eq 1 ] || printf ','
        primo=0
        printf '\n    {"id": "%s", "esito": "%s", "log": ' \
            "${nome%%|*}" "$(printf '%s' "${nome#*|}" | cut -d"|" -f1)"
        if [ -z "${nome##*|}" ]; then
            printf 'null}'
        else
            printf '"%s"}' "${nome##*|}"
        fi
    done
    [ "${primo}" -eq 1 ] || printf '\n  '
    printf ']\n'
    printf '}\n'
}

# Scrive il risultato in modo **atomico**: prima un file a parte nella stessa
# directory, poi `mv`, che su uno stesso filesystem e' una rename atomica. Un
# lettore vede il file precedente o quello nuovo, mai meta' del nuovo.
#
# Scrivere direttamente sulla destinazione lascerebbe una finestra in cui il
# risultato e' troncato, e un JSON troncato non e' distinguibile da un JSON
# scritto male: entrambi non si aprono, e i due si chiudono in modi diversi.
#
# Il temporaneo porta il PID, cosi' due corse concorrenti non si sovrascrivono
# il file a meta'.
scrivi_risultato() {
    local esito="$1" parziale
    parziale="${RISULTATO}.parziale.$$"
    # Le graffe portano la redirezione **dentro** il gruppo: un `2>/dev/null`
    # sul solo comando non zittisce l'errore che la shell stampa quando non
    # riesce ad aprire il file di destinazione.
    if ! { _risultato_json "${esito}" > "${parziale}"; } 2>/dev/null; then
        rm -f "${parziale}" 2>/dev/null
        return 1
    fi
    if ! mv -f "${parziale}" "${RISULTATO}" 2>/dev/null; then
        rm -f "${parziale}" 2>/dev/null
        return 1
    fi
    return 0
}

# La coda di **ogni** percorso terminale, senza eccezioni: `exit` non compare
# altrove in questo file, ed e' una regola verificata dalle sonde invece che
# ricordata. La prima stesura ne contava le occorrenze, e un conteggio non
# distingue l'uscita ammessa da quella vietata: toglierne una e aggiungerne
# un'altra lasciava il totale a tre.
#
# Se il risultato non si puo' scrivere, l'esito tornerebbe a vivere solo sullo
# stdout — cioe' il difetto che questo file esiste per chiudere — e la corsa lo
# dice invece di tacerlo.
concludi() {
    local esito="$1" codice="$2"
    if scrivi_risultato "${esito}"; then
        echo "risultato:            ${RISULTATO}"
        exit "${codice}"
    fi
    echo "RISULTATO NON REGISTRATO in «${RISULTATO}»." >&2
    echo "L'esito della corsa e' «${esito}» e vive solo su questo stdout:" >&2
    echo "una corsa il cui esito non e' su disco non e' citabile da" >&2
    echo "un'evidenza, ed e' la ragione per cui una corsa e' gia' stata" >&2
    echo "scartata per intero." >&2
    echo "Se l'esito e' «risultato_non_scrivibile», la corsa e' stata rifiutata" >&2
    echo "in partenza: la destinazione non era scrivibile e nulla e' stato" >&2
    echo "misurato." >&2
    exit 2
}

# Esegue un passo, conserva il log per intero, e prende l'esito dal comando.
passo() {
    local nome="$1"
    shift
    local log="${LOG_DIR}/${nome}.log"
    passi=$((passi + 1))
    printf '  %-38s ' "${nome}"
    # L'esito si cattura **subito dopo il comando**, non dopo un `if`: un `if`
    # con condizione falsa e senza `else` restituisce 0, quindi `$?` letto dopo
    # il `fi` diceva sempre «exit 0» anche sui rossi. Trovato al checkpoint del
    # 2026-08-21, che ha stampato «ROSSO (exit 0)» -- una contraddizione che
    # rendeva inutile proprio il numero che serve a capire come si e' fallito.
    "$@" > "${log}" 2>&1
    local esito=$?
    if [ "${esito}" -eq 0 ]; then
        verdi=$((verdi + 1))
        registra_passo "${nome}" verde "${nome}.log"
        echo "verde"
        return 0
    fi
    echo "ROSSO (exit ${esito}) — ${log}"
    falliti+=("${nome}")
    registra_passo "${nome}" "rosso" "${nome}.log"
    return "${esito}"
}

# Marca un passo come **non eseguito** perche' una sua precondizione e'
# fallita. Non e' verde: un passo che non e' girato non ha verificato niente, e
# contarlo fra i verdi e' il modo in cui un checkpoint si autoconvalida.
salta() {
    local nome="$1"
    local causa="$2"
    passi=$((passi + 1))
    printf '  %-38s ' "${nome}"
    echo "SALTATO (${causa} e' fallito)"
    falliti+=("${nome}(saltato)")
    registra_passo "${nome}" "saltato" ""
}

# Gli **undici** passi che il livello 1 puo' omettere. Elenco chiuso.
#
# Senza questo elenco `passo_pesante` accetterebbe qualunque nome, e marcare
# per sbaglio un gate come pesante lo farebbe sparire dal livello 1 in
# silenzio: esattamente il difetto che la modalita' esiste per chiudere,
# reintrodotto dalla porta di servizio. Un nome che non e' qui e' un errore di
# programmazione dello script, non un passo nuovo.
PASSI_PESANTI=(
    fuzz_replay
    fuzz_smoke
    coverage_pulizia
    coverage_misura
    coverage_export
    coverage_report_non_vuoto
    check_coverage_exclusions
    coverage_export_cli
    coverage_report_cli_non_vuoto
    coverage_soglia_dal_report
    coverage_soglia_controprova
)

e_pesante_autorizzato() {
    local cercato="$1" noto
    for noto in "${PASSI_PESANTI[@]}"; do
        [ "${noto}" = "${cercato}" ] && return 0
    done
    return 1
}

# Il rifiuto vale in **entrambe** le modalita': riguarda la dichiarazione, non
# l'esecuzione. Un passo dichiarato pesante e non autorizzato e' un difetto
# dello script anche quando lo script lo eseguirebbe comunque.
rifiuta_non_autorizzato() {
    local nome="$1"
    passi=$((passi + 1))
    printf '  %-38s ' "${nome}"
    echo "NON AUTORIZZATO come passo pesante"
    registra_passo "${nome}" "non_autorizzato" ""
    echo "    `${nome}` non e' nell'elenco chiuso PASSI_PESANTI." >&2
    echo "    Marcare pesante un passo che non lo e' lo fa sparire dal" >&2
    echo "    livello 1 in silenzio. Se il passo e' davvero pesante," >&2
    echo "    aggiungilo all'elenco e alla sonda, nello stesso commit." >&2
    falliti+=("${nome}(non autorizzato)")
}

# Un passo che il livello 1 **omette**: fuzz e copertura.
#
# Omesso non e' saltato. `salta` marca un passo che *doveva* girare e non ha
# potuto, e lo conta fra i falliti; qui il passo non doveva girare affatto, e
# contarlo fra i falliti renderebbe il livello 1 rosso per costruzione. Resta
# pero' **stampato**, perche' un esito che non elenca cio' che non ha misurato
# e' esattamente il difetto che questa modalita' esiste per chiudere.
passo_pesante() {
    local nome="$1"
    shift
    if ! e_pesante_autorizzato "${nome}"; then
        rifiuta_non_autorizzato "${nome}"
        return 1
    fi
    if [ "${LIVELLO}" = "1" ]; then
        omessi=$((omessi + 1))
        printf '  %-38s ' "${nome}"
        echo "omesso (livello 1)"
        registra_passo "${nome}" "omesso" ""
        return 0
    fi
    passo "${nome}" "$@"
}

# Come `passo_in_catena`, ma omesso al livello 1.
passo_pesante_in_catena() {
    local nome="$1"
    shift
    if ! e_pesante_autorizzato "${nome}"; then
        rifiuta_non_autorizzato "${nome}"
        return 1
    fi
    if [ "${LIVELLO}" = "1" ]; then
        omessi=$((omessi + 1))
        printf '  %-38s ' "${nome}"
        echo "omesso (livello 1)"
        registra_passo "${nome}" "omesso" ""
        return 1
    fi
    passo_in_catena "${nome}" "$@"
}

# Esegue un passo **solo se la catena non e' gia' rotta**.
#
# La prima versione faceva dipendere i passi della copertura dalla sola misura:
# se `coverage_export` falliva, il gate delle esclusioni e la soglia giravano lo
# stesso, su un file che poteva essere assente o vecchio. Ogni passo dipende ora
# dal **precedente**, non dal primo della catena.
#
# `catena_rotta` porta il nome del passo che l'ha spezzata, cosi' il motivo del
# salto e' quello vero e non un generico «a monte e' andato male».
passo_in_catena() {
    local nome="$1"
    shift
    if [ -n "${catena_rotta}" ]; then
        salta "${nome}" "${catena_rotta}"
        return 1
    fi
    if passo "${nome}" "$@"; then
        return 0
    fi
    catena_rotta="${nome}"
    return 1
}

# Impronta di **cio' che l'albero contiene**, non di quanti file sono sporchi.
#
# `git status --porcelain | wc -l` conta le righe: un passo che modificasse un
# file gia' marcato `M` lascerebbe il conteggio identico. Il conteggio dice
# «quante cose sono diverse», e serve invece sapere «quali, e come».
#
# Tre componenti, perche' una sola non basta:
#
#   * `git diff --cached` — cio' che e' in staging;
#   * `git diff` — le modifiche non in staging ai file tracciati, dove finisce
#     una scrittura su un file gia' sporco;
#   * il **contenuto** dei file non tracciati e non ignorati — il loro elenco
#     non basta, perche' riscrivere un untracked gia' presente non cambia
#     l'elenco.
#
# Chiude la classe di difetto incontrata il 2026-08-21 con le sonde
# distruttive, dove un ripristino cancello' modifiche non committate e i
# conteggi non se ne accorsero.
impronta_albero() {
    # Prima si **acquisisce**, poi si hasha. La versione precedente convogliava
    # i tre comandi in una pipe con `2>/dev/null`: se git falliva, la pipe
    # riceveva zero byte e l'impronta risultava lo sha256 della stringa vuota
    # -- lo stesso valore di un albero pulito.
    #
    # Era la famiglia di difetto che questa serie insegue: **un valore che
    # significa due cose**. Trovata alla qualifica di `1c2707e`, e gia'
    # incontrata senza riconoscerla quando una prova giro' per sbaglio da `/`
    # e stampo' quella costante.
    #
    # Non basta controllare `rev-parse`: un fallimento interno di `git diff`
    # arriverebbe comunque. Ogni comando ha il proprio controllo, e l'hash si
    # produce solo se **tutti** e tre hanno acquisito.
    local temporanea esito
    temporanea="$(mktemp -d 2>/dev/null)" || return 1

    _impronta_fallisci() {
        rm -rf "${temporanea}"
        return 1
    }

    git rev-parse --is-inside-work-tree > /dev/null 2>&1 || { _impronta_fallisci; return 1; }

    git diff --cached --binary --no-ext-diff --no-textconv > "${temporanea}/staged" 2>/dev/null ||
        { _impronta_fallisci; return 1; }
    git diff --binary --no-ext-diff --no-textconv > "${temporanea}/unstaged" 2>/dev/null ||
        { _impronta_fallisci; return 1; }
    git ls-files --others --exclude-standard -z > "${temporanea}/elenco" 2>/dev/null ||
        { _impronta_fallisci; return 1; }

    # Percorso **e** hash del contenuto, ciascuno delimitato: dare in pasto il
    # contenuto grezzo renderebbe ambiguo il confine fra un percorso e il file
    # precedente, e due alberi diversi potrebbero collidere.
    : > "${temporanea}/untracked"
    sort -z < "${temporanea}/elenco" |
        while IFS= read -r -d "" percorso; do
            if [ -f "${percorso}" ] && [ ! -L "${percorso}" ]; then
                contenuto="$(sha256sum < "${percorso}" 2>/dev/null | cut -d" " -f1)"
                [ -n "${contenuto}" ] || contenuto="illeggibile"
            else
                contenuto="non-regolare"
            fi
            printf "U\0%s\0%s\0" "${percorso}" "${contenuto}"
        done >> "${temporanea}/untracked"

    # Il prefisso versionato serve a due cose: nessuna impronta puo' piu'
    # valere lo sha256 della stringa vuota -- nemmeno su un albero pulito --
    # e il formato dell'impronta e' dichiarato, cosi' un cambio futuro si
    # riconosce invece di somigliare a un albero cambiato.
    esito=0
    {
        printf "impronta-albero-v1\0"
        cat "${temporanea}/staged" "${temporanea}/unstaged" "${temporanea}/untracked"
    } | sha256sum | cut -d" " -f1 || esito=1
    rm -rf "${temporanea}"
    return "${esito}"
}

# Consente alle sonde di caricare solo le funzioni, senza eseguire il
# checkpoint. Un gate che non si puo' provare e' un gate di cui ci si fida
# perche' non si e' mai visto sbagliare.
if [ "${S9_CHECKPOINT_SOLO_FUNZIONI:-0}" = "1" ]; then
    return 0
fi

# Una corsa che parte lo dice **su disco**, prima di misurare qualunque cosa.
# Se muore a meta', questo file resta a dichiarare `in_corso`: e' la differenza
# fra «non e' mai partita» e «e' morta a meta'», che nessun altro artefatto
# distingue.
# Anche il rifiuto in partenza passa da `concludi`: la scrittura ritentera' e
# fallira' di nuovo — costa una `printf` — e la corsa uscira' dal ramo che
# dichiara «risultato non registrato». Cosi' **nessuna** uscita terminale vive
# fuori da quella funzione, e la regola si puo' verificare invece di contare
# righe.
scrivi_risultato in_corso || concludi risultato_non_scrivibile 2

REVISIONE="$(git rev-parse HEAD)"
SPORCHI="$(git status --porcelain | wc -l)"
if ! IMPRONTA_INIZIO="$(impronta_albero)"; then
    echo "IMPRONTA NON CALCOLABILE in testa alla corsa." >&2
    echo "Un comando git non ha acquisito. Senza impronta iniziale non c'e'" >&2
    echo "niente con cui confrontare quella finale, e il passo" >&2
    echo "`albero_invariato` sarebbe verde per assenza di dati." >&2
    concludi impronta_iniziale_non_calcolabile 2
fi
scrivi_risultato in_corso || true

echo "=============================================================="
if [ "${LIVELLO}" = "1" ]; then
    echo "S9 — verifica di livello 1"
else
    echo "S9 — checkpoint di livello 2"
fi
echo "revisione:      ${REVISIONE}"
echo "albero:         ${SPORCHI} file non committati"
echo "impronta:       ${IMPRONTA_INIZIO}"
echo "log:            ${LOG_DIR}"
echo "=============================================================="

# L'albero sporco e' un errore solo al livello 2. Il livello 1 gira **durante**
# una tranche, cioe' proprio quando l'albero e' sporco: pretenderlo pulito
# renderebbe la modalita' inutilizzabile nel momento in cui serve, e la
# spingerebbe di nuovo fuori dallo script — che e' l'errore che si sta
# chiudendo. Il livello 1 non produce un'evidenza, quindi non deve essere
# same-SHA.
if [ "${SPORCHI}" -ne 0 ] && [ "${LIVELLO}" != "1" ]; then
    echo
    echo "ALBERO SPORCO: la misura non sarebbe same-SHA." >&2
    echo "Un checkpoint su un albero che non coincide con la revisione" >&2
    echo "dichiarata e' peggio di nessun checkpoint: sembra un'evidenza." >&2
    # Un'uscita terminale passa da `concludi`, sempre. Questa usciva con 2
    # lasciando `in_corso` sul disco: la causa era **nota** — l'albero e'
    # sporco — e il file diceva «e' morta a meta'», cioe' esattamente la
    # confusione che il risultato esiste per togliere.
    concludi albero_sporco 2
fi


echo
echo "--- 1. compilazione e test -----------------------------------"
# Le sonde del checkpoint stesso vengono per prime: se il gate che misura e'
# rotto, tutto cio' che segue e' una misura di cui non si sa niente.
passo sonde_checkpoint bash scripts/test_s9_checkpoint.sh
passo fmt cargo fmt --all -- --check
passo clippy cargo clippy --workspace --all-targets --all-features -- -D warnings
# I tre fork governati, con lo stesso `-D warnings` che la CI impone.
#
# `--workspace` non li tocca: `vendor/` e' escluso, ed e' giusto che lo sia --
# non sono codice nostro. La CI pero' compila **ogni** crate del grafo con
# `-D warnings`, perche' `setup-rust-toolchain` lo esporta in `RUSTFLAGS`: un
# avviso dentro un fork e' un errore li' e non qui.
#
# Il 2026-09-04 la differenza ha lasciato passare un fork che in locale
# compilava e in CI no -- trentadue deprecazioni di `geo_types::Coordinate` --
# e il livello 1 diceva 74/74 su un albero che la pipeline rifiutava. Questo
# passo esiste perche' quella differenza non torni.
#
# La forma e' quella della CI, e non un `-p` per ciascun fork: `--all-features`
# non si puo' dare a un pacchetto fuori dal workspace, e senza feature `gdal`
# non e' nemmeno nel grafo. `RUSTFLAGS` invece raggiunge ogni crate che si
# compila, che e' esattamente cio' che la pipeline fa.
passo clippy_vendor env RUSTFLAGS=-Dwarnings cargo check --workspace --all-features --all-targets
# I lint di sicurezza, nella forma **esatta** in cui CI li impone.
#
# Mancavano, e il 2026-08-26 hanno lasciato passare un `unreachable!` in codice
# consegnato attraverso un livello 1 e un livello 2: entrambi dichiaravano
# «nessun fallito» eseguendo un insieme di controlli piu' debole di quello che
# la pipeline applica sullo stesso commit. E' lo stesso difetto che questa
# intestazione racconta per `cargo fmt --check`, ripetuto: un elenco di passi
# tenuto a mano diverge, e diverge in silenzio.
#
# `plenora-bench` e `plenora-fuzz` restano fuori con le stesse ragioni di CI:
# sono attrezzaggio, non vengono consegnati, e li' un `panic!` e' il modo
# giusto di fermarsi.
passo clippy_sicurezza cargo clippy --workspace --lib --bins --all-features --locked     --exclude plenora-bench --exclude plenora-fuzz     -- -D warnings -D unsafe-code -D clippy::unwrap_used -D clippy::expect_used     -D clippy::panic -D clippy::unreachable -D clippy::todo -D clippy::unimplemented
# `--all-features` abilita `gdal-backend`, quindi il percorso stub di
# driver-filegdb non veniva compilato da nessun passo. Il 2026-08-21 e'
# rimasto rotto per un'intera tranche, e a trovarlo e' stata la misura di
# copertura, che gira senza feature: un gate non dovrebbe dipendere da un
# altro per accorgersi di una compilazione fallita.
passo clippy_default cargo clippy --workspace --all-targets -- -D warnings
passo test cargo test --workspace --all-features
# Il set di feature **predefinito** va compilato a parte: `--all-features`
# abilita `gdal-backend`, e il percorso stub di driver-filegdb non veniva
# compilato da nessun passo di livello 1. Il 2026-08-21 e' rimasto rotto per
# un'intera tranche, e a trovarlo e' stata la misura di copertura -- che gira
# senza feature. Un passo non deve dipendere da un altro per accorgersi che
# qualcosa non compila.
passo test_default cargo test --workspace --all-targets

echo
echo "--- 2. gate del censimento e delle sonde ---------------------"
passo sonde_quartetto python3 -m unittest scripts.test_check_quartetto_sito
passo check_quartetto python3 scripts/check_quartetto_sito.py
passo sonde_errori_redatti python3 -m unittest scripts.test_check_errori_redatti
passo check_errori_redatti python3 scripts/check_errori_redatti.py

# `&'static str` garantisce la durata, non la provenienza: senza questo
# passo un `Box::leak` riporterebbe testo runtime dentro un messaggio
# curato, e il censimento resterebbe verde.
passo sonde_niente_leak python3 -m unittest scripts.test_check_niente_leak
passo check_niente_leak python3 scripts/check_niente_leak.py
passo sonde_wkb_limits python3 -m unittest scripts.test_check_wkb_limits_defaults
passo check_wkb_limits python3 scripts/check_wkb_limits_defaults.py
# I gate del lotto S12. Girano in CI dal primo commit del lotto; qui erano
# assenti, cioe' il checkpoint dichiarava verde un commit senza avere guardato
# la capability che quel commit pubblica.
passo sonde_capability_ostile python3 -m unittest scripts.test_check_capability_input_ostile
passo check_capability_ostile python3 scripts/check_capability_input_ostile.py
passo prove_di_confine python3 scripts/check_prove_di_confine.py
passo sonde_semi_s12 python3 -m unittest scripts.test_genera_semi_s12
passo semi_s12 python3 scripts/genera_semi_s12.py --verifica
# I gate del lotto S10: la validazione dei metadati GeoParquet, e il perimetro
# di versione che il catalogo dichiara.
passo sonde_metadati_geoparquet python3 -m unittest scripts.test_check_metadati_geoparquet
passo check_metadati_geoparquet python3 scripts/check_metadati_geoparquet.py
# Gli schemi ufficiali come autorita' indipendente: lock, byte, sha256, `$id`,
# draft, `$ref`, gli elenchi chiusi ricavati dallo schema, la closure del driver
# e il suo censimento.
passo sonde_schemi_geoparquet python3 -m unittest scripts.test_check_schemi_geoparquet
passo check_schemi_geoparquet python3 scripts/check_schemi_geoparquet.py
# Il vocabolario delle categorie di perdita: chiuso, dichiarato in un registro
# fuori dal codice, e con una sola via dinamica ammessa -- quella di DXF, che
# il lotto wire.loss-report chiude. Una seconda via renderebbe la cardinalita'
# della busta CLI una decisione di chi fornisce il file.
# I byte delle fixture canoniche della matrice cross-format. Il generatore non
# gira in CI -- una fixture rigenerata a ogni corsa renderebbe l'atteso una
# funzione dello strumento del giorno -- quindi a rispondere dei byte
# committati e' questo gate, che confronta insiemi e digest uno per uno. La
# directory vuota e' rossa per costruzione: li' ogni digest sarebbe soddisfatto
# per assenza di confronti.
# La matrice cross-format: quali conversioni il prodotto promette davvero, e
# se l'insieme copre le classi che contano. La copertura non e' scritta nel
# registro -- si deriva dai descrittori -- e un rifiuto non copre l'estremo:
# prova che il driver non e' stato attraversato.
passo sonde_conversioni python3 -m unittest scripts.test_check_conversioni
passo check_conversioni python3 scripts/check-conversioni.py
passo sonde_fixture_canoniche python3 -m unittest scripts.test_check_fixture_canoniche
passo check_fixture_canoniche python3 scripts/check-fixture-canoniche.py
passo sonde_categorie_di_perdita python3 -m unittest scripts.test_check_categorie_di_perdita
passo check_categorie_di_perdita python3 scripts/check_categorie_di_perdita.py
# I numeri del protocollo v2, confrontati col codice che li applica. Il
# manifesto ne dichiara nove, e stanno come costanti in due crate diversi:
# nessuno li confrontava. Un tetto alzato nel codice e non nel manifesto
# lascerebbe il contratto a promettere il numero vecchio, e i due resterebbero
# ciascuno coerente con se stesso.
passo sonde_protocollo_v2 python3 -m unittest scripts.test_check_protocollo_v2
passo check_protocollo_v2 python3 scripts/check_protocollo_v2.py
# Le **buste**, confrontate con il binario che le emette.
#
# Il passo qui sopra confronta i numeri del manifesto con le costanti del
# codice: due dichiarazioni, entrambe lette dal sorgente. Questo esegue la CLI
# su fixture versionate e guarda che cosa esce davvero dai due flussi.
#
# Serviva perche' il manifesto descriveva il **primo livello** di ciascuna
# busta e sotto quelle chiavi taceva: la ratifica del 2026-09-04 ha trovato
# quattro campi diagnostici non elencati, una clausola che affermava sei chiavi
# dove ce ne sono sette, e una busta -- quella di `--version` -- che nessuno
# censiva. Nessuna sonda poteva vederli, perche' verificano che i campi
# dichiarati ci siano, non che non ce ne siano altri.
#
# Costruisce il binario da se': un gate che verifica cio' che il binario emette
# non puo' dipendere da chi lo ha costruito e quando.
passo sonde_buste_v2 python3 -m unittest scripts.test_check_buste_v2
passo check_buste_v2 python3 scripts/check_buste_v2.py
# L'SDK Python, e i suoi modelli confrontati col protocollo.
#
# Le dataclass dell'SDK sono una **seconda scrittura** del contratto: i campi
# stanno nel manifesto e stanno di nuovo in `models.py`. Un campo che il
# protocollo dichiara e il modello non ha e' un pezzo di busta che l'SDK butta
# via in silenzio; uno che il modello ha e il protocollo non dichiara fa fallire
# l'SDK sulla prima busta valida che non lo porta.
#
# Le sonde girano con `PYTHONPATH` sul sorgente invece che su un pacchetto
# installato: il checkpoint verifica l'albero, non cio' che qualcuno ha messo
# nell'ambiente.
passo sonde_sdk_python env PYTHONPATH=sdk/python/src python3 -m unittest discover -s sdk/python/tests -p "test_*.py"
passo check_sdk_python python3 scripts/check_sdk_python.py
# Il pacchetto si costruisce, e i byte sono gli stessi due volte.
#
# La riproducibilita' non e' un vezzo: un checksum che cambia senza che cambi il
# contenuto non lega niente, e la pubblicazione deve poter riusare **gli stessi
# byte** che sono stati qualificati invece di ricostruirli.
passo costruisci_pacchetto_python python3 scripts/costruisci-pacchetto-python.py --uscita target/pacchetto-python --referti target/referti-python
passo pacchetto_python_riproducibile bash scripts/verifica-riproducibilita-python.sh
# Lo smoke in ambienti **puliti**: la wheel installata, la sdist ricostruita, e
# la suite eseguita contro il pacchetto installato invece che contro i sorgenti
# che stanno li' accanto.
passo smoke_pacchetto_python bash scripts/smoke-pacchetto-python.sh target/pacchetto-python
# Il canale del pacchetto e' chiuso **per assenza**: nessun workflow carica su
# un indice. Una promessa mantenuta per assenza si rompe senza far rosso da
# nessuna parte, e il primo a scoprirlo sarebbe chi trova il pacchetto dove non
# doveva essere.
passo canale_privato python3 scripts/check_canale_privato.py
# `requires-python` copre esattamente le versioni che la CI prova, nei due
# versi. Una riga piu' larga della matrice promette un Python su cui nessuno ha
# guardato; una piu' stretta rifiuta installazioni che funzionano.
passo check_requires_python python3 scripts/check_requires_python.py
# Il confine del v1: `detail_v1()` restituisce i nomi presi dal file, e li
# pubblica **un solo** adattatore. La visibilita' di Rust non sa dire «questo
# modulo e nessun altro», e un accessore pubblico e' pubblico: senza questo
# passo la prima chiamata fuori posto rimetterebbe sul filo del v2 cio' che il
# v2 esiste per togliere.
passo sonde_confine_v1 python3 -m unittest scripts.test_check_confine_v1
passo check_confine_v1 python3 scripts/check_confine_v1.py
passo sonde_quarantena python3 -m unittest scripts.test_check_quarantena_fuzz
passo check_quarantena python3 scripts/check_quarantena_fuzz.py
passo sonde_prevalidazione python3 -m unittest scripts.test_check_prevalidazione_decoder
passo check_prevalidazione python3 scripts/check_prevalidazione_decoder.py
passo sonde_identita python3 -m unittest scripts.test_check_public_identity
passo check_identita python3 scripts/check_public_identity.py
passo sonde_release python3 -m unittest scripts.test_check_release_contract
passo check_release python3 scripts/check_release_contract.py
passo sonde_distribuzione python3 -m unittest scripts.test_distribuzione_matrice
passo sonde_licenze python3 -m unittest scripts.test_licenze
passo sonde_dist_completa python3 -m unittest scripts.test_distribuzione_completa
passo sonde_verificatori_nativi python3 -m unittest scripts.test_verificatori_nativi
passo sonde_sbom python3 -m unittest scripts.test_sbom
passo sonde_action_pins python3 -m unittest scripts.test_check_action_pins
passo check_action_pins python3 scripts/check_action_pins.py
passo sonde_toolchain python3 -m unittest scripts.test_check_toolchain_pins
passo check_toolchain python3 scripts/check_toolchain_pins.py
passo sonde_coverage_excl python3 -m unittest scripts.test_check_coverage_exclusions
passo sonde_filegdb python3 -m unittest scripts.test_check_filegdb_catalog
passo sonde_wkb_condiviso python3 -m unittest scripts.test_compare_shared_wkb_observations
passo corpus_condiviso python3 scripts/generate_shared_wkb_corpus.py --check
passo sonde_audit_ignores python3 -m unittest scripts.test_audit_ignores
passo check_dependency_pins python3 scripts/check_dependency_pins.py
passo check_gdal_fork python3 scripts/check_gdal_fork.py
passo sonde_fork python3 -m unittest scripts.test_fork_comune
passo check_dxf_fork python3 scripts/check_dxf_fork.py
# Il terzo fork governato: il delta espone la scrittura di un record con
# geometria nulla, che la specifica ammette e che l'API upstream non sa
# esprimere. Senza, il prodotto non sapeva riscrivere un file che sapeva
# leggere.
passo check_shapefile_fork python3 scripts/check_shapefile_fork.py
passo check_no_legacy_budget python3 scripts/check_no_legacy_budget.py
passo check_permit_boundary python3 scripts/check_permit_boundary.py
passo sonde_fallback python3 -m unittest scripts.test_check_assurance_fallbacks
passo assurance_fallbacks bash scripts/check_assurance_fallbacks.sh

# ASSURANCE-N1 non era cablato qui: lo eseguivo nelle batterie composte a mano,
# e da quando il livello 1 deriva da questo script ha smesso di girare del
# tutto. E' la stessa lezione di `fmt`, e stavolta e' costata meno solo perche'
# il registro non e' cambiato nel frattempo.
#
# Due gate, e la distinzione conta: `--integrita` dice che il **registro** e'
# coerente; `check_assurance_n1_prove.py` **esegue** i test dichiarati e legge
# gli esiti. Il primo resta verde su un test marcato `#[ignore]`, il secondo no
# -- verificato.
# Il docset e' minimo per scelta: un documento in piu' e' un documento che
# nessuno rileggera'. Il gate impedisce che la cronaca rientri.
passo sonde_docset python3 -m unittest scripts.test_check_docset
passo check_docset python3 scripts/check_docset.py
passo sonde_assurance_n1 python3 -m unittest scripts.test_check_assurance_n1
passo assurance_n1_integrita python3 scripts/check_assurance_n1.py --integrita
passo sonde_assurance_n1_prove python3 -m unittest scripts.test_check_assurance_n1_prove
passo assurance_n1_prove python3 scripts/check_assurance_n1_prove.py

echo
echo "--- 3. catalogo FileGDB reale --------------------------------"
catalogo_filegdb() {
    local json="${LOG_DIR}/catalog.json"
    cargo run --quiet -p plenora-io-cli --features gdal-backend --locked -- catalog > "${json}" || return
    python3 scripts/check_filegdb_catalog.py < "${json}"
}
passo check_filegdb_catalog catalogo_filegdb

echo
echo "--- 4. fuzz: replay deterministico, poi smoke ----------------"
# Il replay viene **prima**: rigioca semi, corpus e artefatti gia' noti, ed e'
# deterministico. Lo smoke cerca input nuovi, e sessanta secondi di mutazioni
# non ritrovano quello che il replay ritrova sempre. Invertirli significa
# scoprire una regressione nota solo per fortuna.
passo_pesante fuzz_replay bash scripts/fuzz-replay.sh
passo_pesante fuzz_smoke bash scripts/fuzz-smoke.sh

echo
echo "--- 5. copertura, poi il suo gate ----------------------------"
ESCLUSIONI='(^|/)(plenora-bench|plenora-fuzz|plenora-io-cli)/src/.*\.rs$'
# Il secondo report **non esclude niente**, e la selezione del perimetro sta
# nello strumento che lo legge.
#
# La strada alternativa era una regex complementare -- «tutto cio' che non e'
# la CLI» -- e passava da un lookahead negativo, che non tutte le sintassi di
# `--ignore-filename-regex` supportano. Una regex che non morde non fallisce:
# include tutto in silenzio, e la misura dedicata direbbe i numeri di quella
# di libreria con un altro nome. Filtrare per prefisso di percorso in
# `coverage_diff.py` e' verificabile da una sonda, un lookahead no.
LCOV_COMPLETO="${LOG_DIR}/lcov-completo.info"
export PLENORA_CROSS_FS_TEST_ROOT="${PLENORA_CROSS_FS_TEST_ROOT:-/dev/shm}"
# I dati di profiling precedenti vanno rimossi **prima** di misurare. Senza,
# una misura fallita lascia in piedi quelli della corsa precedente, e i passi
# che seguono li leggono senza sapere che descrivono un altro albero: e'
# successo il 2026-08-21, e `coverage_soglia` ha detto «verde» su un albero che
# non era quello dichiarato.
LCOV="${LOG_DIR}/lcov.info"
# Il percorso del report parte **vuoto**: se l'export fallisce, non deve restare
# in piedi il file di una corsa precedente per i passi che seguono.
rm -f "${LCOV}"

catena_rotta=""
passo_pesante_in_catena coverage_pulizia cargo llvm-cov clean --workspace
# `--all-features` non e' un dettaglio di invocazione: `driver-filegdb` tiene
# l'intero percorso GDAL dietro `gdal-backend`, e senza quella feature
# cinquecento righe di produzione restavano **fuori dal denominatore** -- non
# «scoperte», ma invisibili alla soglia. Lo scope si chiama «library coverage»,
# e una libreria misurata a meta' non e' la libreria.
passo_pesante_in_catena coverage_misura cargo llvm-cov --workspace --all-targets --all-features --locked --no-report
passo_pesante_in_catena coverage_export cargo llvm-cov report --lcov --output-path "${LCOV}" \
    --ignore-filename-regex "${ESCLUSIONI}"
# Un export che finisce con esito zero e un file vuoto e' un caso che nessuno
# guarda finche' non succede: qui e' un passo, non un'assunzione.
passo_pesante_in_catena coverage_report_non_vuoto test -s "${LCOV}"
passo_pesante_in_catena check_coverage_exclusions python3 scripts/check_coverage_exclusions.py \
    --lcov "${LCOV}"
# La misura dedicata alla CLI: stesso profdata, secondo report con il perimetro
# complementare. Non ha soglia -- e' diagnostica come il differenziale di
# libreria -- e non entra nel denominatore della prima: sommarle darebbe un
# terzo numero che non e' ne' l'una ne' l'altra.
passo_pesante_in_catena coverage_export_cli cargo llvm-cov report --lcov \
    --output-path "${LCOV_COMPLETO}"
passo_pesante_in_catena coverage_report_cli_non_vuoto test -s "${LCOV_COMPLETO}"
# La soglia si legge **dallo stesso file** delle esclusioni.
#
# La versione di cargo resta accanto, ma NON come conferma dello stesso numero:
# sulla corsa dell'8e64965 le due hanno dato 85,88% e 83,98% sugli stessi 38
# file. Non e' un errore di nessuna delle due -- i record `DA:` di LCOV e la
# colonna «Lines» di llvm-cov contano insiemi diversi di righe strumentate, e il
# denominatore differisce di 1.181 su 32.243.
#
# Sono quindi due **proiezioni diverse** dello stesso profdata, ed entrambe
# devono stare sopra la soglia. Chiamarle «la stessa misura» era un'imprecisione
# mia, e avrebbe reso illeggibile il giorno in cui una delle due si rompesse
# davvero.
passo_pesante_in_catena coverage_soglia_dal_report python3 scripts/check_coverage_threshold.py \
    --lcov "${LCOV}" --min-lines 80
passo_pesante_in_catena coverage_soglia_controprova cargo llvm-cov report --summary-only \
    --ignore-filename-regex "${ESCLUSIONI}" --fail-under-lines 80

if [ -z "${catena_rotta}" ]; then
    copertura_misurata=1
else
    copertura_misurata=0
fi

# --- diagnostica: la copertura delle sole righe cambiate ---------------------
#
# NON e' un gate e non ha soglia. La copertura totale non distingue due cose che
# vanno distinte dopo un refactor: la crescita meccanica del denominatore --
# messaggi curati su piu' righe, funzioni estratte -- da un ramo semantico nuovo
# che nessun test esercita. Guardando solo le righe cambiate, la prima risulta
# coperta e la seconda no.
#
# La prima esecuzione, sull'intervallo 107b7b5..effc4ab, ha dato 37,27% e ha
# trovato 22 righe mai eseguite dentro `classe_sqlite`: un vocabolario nuovo che
# nessun test attraversava. Non era crescita meccanica.
#
# Serve un riferimento: S9_CHECKPOINT_BASE e' la revisione dell'ultimo
# checkpoint **superato**, non l'ultimo commit. La differenza conta: fra un
# checkpoint e il successivo ci sono anche commit di infrastruttura e di
# documentazione, e sono parte del delta da qualificare. Prendere come base
# l'ultimo commit escluderebbe dalla misura proprio cio' che non e' ancora stato
# verificato.
#
# Senza, il passo si salta invece di misurare un intervallo arbitrario -- una
# diagnostica su un intervallo scelto a caso e' peggio di nessuna diagnostica,
# perche' ha comunque l'aria di un numero.
echo
echo "--- 6. diagnostica: copertura delle righe cambiate -----------"
if [ "${copertura_misurata}" -ne 1 ]; then
    echo "  saltata: la catena della copertura si e' rotta a «${catena_rotta}»."
    echo "  Un numero calcolato su un report assente o stantio somiglia a una"
    echo "  misura, e descrive un albero che non e' quello dichiarato."
elif [ -n "${S9_CHECKPOINT_BASE:-}" ]; then
    # `--mostra 0` elenca **tutte** le righe scoperte, non le prime venti.
    #
    # Il valore predefinito dello strumento e' pensato per chi legge a
    # schermo; qui il log e' un artefatto d'evidenza, e `check_release`
    # pretende che l'evidenza porti `righe_scoperte` -- l'elenco, non il
    # conteggio. Con il tetto predefinito un'evidenza con piu' di venti
    # righe scoperte non e' scrivibile: i numeri dicono **quante**, e
    # nessun artefatto della corsa dice **quali**.
    python3 scripts/coverage_diff.py --lcov "${LCOV}" \
        --base "${S9_CHECKPOINT_BASE}" --head "${REVISIONE}" --mostra 0 \
        > "${LOG_DIR}/coverage_diff.log" 2>&1
    python3 scripts/coverage_diff.py --lcov "${LCOV_COMPLETO}" \
        --base "${S9_CHECKPOINT_BASE}" --head "${REVISIONE}" --mostra 0 \
        --solo plenora-io-cli \
        > "${LOG_DIR}/coverage_diff_cli.log" 2>&1
    esito_diff=$?
    cat "${LOG_DIR}/coverage_diff.log"
    if [ "${esito_diff}" -ne 0 ]; then
        echo "  la diagnostica non ha potuto misurare (exit ${esito_diff});"
        echo "  non e' un gate, ma il suo silenzio non va letto come un verde."
    fi
else
    echo "  saltata: S9_CHECKPOINT_BASE non impostata."
    echo "  Impostala alla revisione dell'ultimo checkpoint superato."
fi

# L'albero non deve essere cambiato **sotto la misura**. Vale anche al
# livello 1: li' l'albero puo' essere sporco, ma deve restare sporco **allo
# stesso modo**. Un passo che scrive nell'albero che sta verificando invalida
# la verifica, e lo fa in modo che nessun conteggio rivela.
# La revisione va **riletta**, non ristampata. Fino al 2026-08-21 la coda
# stampava la stessa variabile acquisita in testa: «SHA iniziale e finale
# identici» era una tautografia, non una misura, e le evidenze la elencavano
# fra i criteri verificati. Un commit durante la corsa lascerebbe l'albero
# invariato e sposterebbe HEAD, cioe' descriverebbe una revisione diversa da
# quella misurata.
REVISIONE_FINE="$(git rev-parse HEAD)"
if [ "${REVISIONE_FINE}" != "${REVISIONE}" ]; then
    passi=$((passi + 1))
    printf '  %-38s ' "revisione_invariata"
    echo "ROSSO — HEAD si e' mosso durante la corsa"
    echo "    revisione iniziale: ${REVISIONE}" >&2
    echo "    revisione finale:   ${REVISIONE_FINE}" >&2
    echo "    La misura descrive un albero, l'esito ne nominerebbe un altro." >&2
    falliti+=("revisione_invariata")
    registra_passo revisione_invariata rosso ""
else
    passi=$((passi + 1))
    verdi=$((verdi + 1))
    printf '  %-38s ' "revisione_invariata"
    echo "verde"
    registra_passo revisione_invariata verde ""
fi

if ! IMPRONTA_FINE="$(impronta_albero)"; then
    passi=$((passi + 1))
    printf '  %-38s ' "albero_invariato"
    echo "ROSSO — impronta finale non calcolabile"
    echo "    Un comando git non ha acquisito a fine corsa." >&2
    echo "    Non sapere se l'albero e' cambiato non e' sapere che non lo e'." >&2
    falliti+=("albero_invariato")
    registra_passo albero_invariato rosso ""
elif [ "${IMPRONTA_FINE}" != "${IMPRONTA_INIZIO}" ]; then
    passi=$((passi + 1))
    printf '  %-38s ' "albero_invariato"
    echo "ROSSO — l'albero e' cambiato durante la corsa"
    echo "    impronta iniziale: ${IMPRONTA_INIZIO}" >&2
    echo "    impronta finale:   ${IMPRONTA_FINE}" >&2
    echo "    Un passo ha scritto nell'albero che stava verificando." >&2
    falliti+=("albero_invariato")
    registra_passo albero_invariato rosso ""
else
    passi=$((passi + 1))
    verdi=$((verdi + 1))
    printf '  %-38s ' "albero_invariato"
    echo "verde"
    registra_passo albero_invariato verde ""
fi

# L'insieme dei passi eseguiti e' quello dichiarato dal registro.
#
# Non e' un passo: e' una **precondizione del verdetto**, come l'albero pulito
# in partenza. Un passo lo conterebbe fra i propri, e la domanda «li ho
# eseguiti tutti?» non puo' essere uno degli elementi contati.
if ! divergenze="$(insieme_dei_passi_dichiarato)"; then
    echo
    echo "INSIEME DEI PASSI DIVERGENTE dal registro." >&2
    printf '%s\n' "${divergenze}" >&2
    echo "    Il registro e' ${REGISTRO_DEI_PASSI}." >&2
    echo "    Una corsa che non esegue cio' che il registro dichiara misura" >&2
    echo "    un altro insieme, e i suoi conteggi sono coerenti con se stessi." >&2
    falliti+=("insieme_dei_passi")
    echo "=============================================================="
    echo "esito: NON SUPERATO"
    concludi insieme_dei_passi_divergente 1
fi

echo
echo "=============================================================="
echo "revisione verificata: ${REVISIONE_FINE}"
echo "impronta albero:      ${IMPRONTA_FINE}"
echo "passi: ${verdi}/${passi}"
if [ "${#falliti[@]}" -ne 0 ]; then
    echo "ROSSI: ${falliti[*]}"
    echo "esito: NON SUPERATO"
    echo "=============================================================="
    concludi non_superato 1
fi
if [ "${LIVELLO}" = "1" ]; then
    echo "omessi: ${omessi} passi (fuzz e copertura)"
    echo "esito: S9 livello 1 verificato"
    echo
    echo "**Non e' un checkpoint.** Fuzz e copertura non sono stati misurati,"
    echo "e un commit verificato cosi' e' verificato, non release-qualified."
    echo "Il livello 2 si esegue senza S9_LIVELLO, su un albero pulito."
    echo "=============================================================="
    concludi livello_1_verificato 0
fi
echo "esito: S9 checkpoint level 2 passed"
echo
echo "Questo esito NON autorizza una release e non promuove la readiness"
echo "di alcun componente ne' del sistema. Va registrato in un'evidenza S9"
echo "separata. La readiness di sistema e' un'altra cosa, di proprieta'"
echo "esterna: vedi release/system-rc-gate.json."
echo "=============================================================="
concludi superato 0
