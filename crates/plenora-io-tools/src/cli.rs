//! Gli argomenti della riga di comando, e la struttura che ne esce.
//!
//! Estratto da `lib.rs` per M1. E' dove vive la conformita' a CLI-2.0, e il
//! confine non e' stato scelto adesso: il file lo delimitava gia' con un
//! proprio commento di sezione.
//!
//! Fra questo modulo e i comandi si condivide **`Cli` e nient'altro** -- dati,
//! tre mappe di opzioni, `PipelineLimits`, e un solo elemento vivo, il
//! `CancellationToken` che `parse_legato` inietta. I comandi la leggono e non
//! vi scrivono nulla che il parsing rilegga: e' la misura che ha motivato
//! questa estrazione.
//!
//! Lo spostamento e' meccanico: nessuna firma cambia, nessun ordine di
//! operazioni cambia, e `Cli` e `parse` restano dove erano perche' `lib.rs` li
//! riesporta.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde_json::Value;

use plenora_io_model::budget::PipelineLimits;
use plenora_io_model::{CancellationToken, PublicMessage};

use crate::{usage_err, FORMATO_JSON, OPZIONI_AMMESSE};

// --- parsing argomenti ------------------------------------------------------

#[derive(Default)]
pub struct Cli {
    pub(crate) source: Option<plenora_io_core::Source>,
    pub(crate) source_base64: Option<String>,
    pub(crate) archive_entry: Option<String>,
    pub(crate) archive_limits: plenora_io_core::LimitiArchivio,
    pub(crate) flags: std::collections::BTreeSet<String>,
    pub(crate) positionals: Vec<String>,
    /// Dove `read` consegna il dataset Arrow.
    ///
    /// E' un flag e non un posizionale perche' la consegna e' **facoltativa**:
    /// `read SORGENTE` legge e riporta senza materializzare, ed e' il modo in
    /// cui si valida una sorgente grande. Un secondo posizionale avrebbe reso
    /// la consegna obbligatoria o la sua assenza indistinguibile da un refuso.
    pub(crate) output: Option<PathBuf>,
    pub(crate) assume_crs: Option<String>,
    /// Il formato del sink di `write`, **nominato**.
    ///
    /// Non dedotto dall'estensione della destinazione: il profilo io-tools dice
    /// che il comportamento specifico di un formato non va scelto analizzando
    /// l'estensione quando l'operazione richiede un formato esplicito, e il
    /// catalogo descrive `io.write` come «publish an Arrow dataset to an
    /// **explicit** sink format». Un'estensione e' una convenzione del
    /// filesystem, non un identificatore di formato: `.json` e' `GeoJSON` qui e
    /// mille altre cose altrove, e una destinazione senza estensione non
    /// avrebbe risposta.
    ///
    /// I valori ammessi sono gli `id` che `io.catalog` rende, e una prova lo
    /// verifica nei due versi: nessun formato del catalogo senza driver, e
    /// nessun driver che il catalogo non dichiari.
    pub(crate) to: Option<String>,
    /// Il formato della **sorgente** di `convert`, nominato.
    ///
    /// Esiste per la stessa ragione di `to`, e la ragione e' nel catalogo:
    /// `io.convert` vi e' descritto come «convert an external dataset between
    /// **explicit** formats». Due formati, due nomi.
    ///
    /// Non vale per `inspect`, `layers` e `read`, e non e' una dimenticanza:
    /// quelle operazioni **riconoscono** la sorgente invece di riceverla
    /// dichiarata -- `io.inspect` rende «its **declared** format» -- e il
    /// suffisso li' e' l'unico segnale che c'e'. Riconoscere e dichiarare sono
    /// due cose, e il catalogo le distingue operazione per operazione.
    pub(crate) from_: Option<String>,
    pub(crate) layer: Option<u32>,
    pub(crate) limit: Option<usize>,
    pub(crate) durable: bool,
    pub(crate) opts: BTreeMap<String, String>,
    pub(crate) in_opts: BTreeMap<String, String>,
    pub(crate) out_opts: BTreeMap<String, String>,
    /// I flag di quota, gia' nel tipo del modello unificato.
    ///
    /// Fino a S4.d atterravano in un `Limits` legacy e venivano tradotti piu'
    /// tardi. Il tipo intermedio non serviva a nulla se non a tenere in vita
    /// il modello vecchio nel punto piu' visibile del componente.
    pub(crate) limits: PipelineLimits,
    /// Il token che il gestore dei segnali arma.
    ///
    /// `parse` ne mette uno **nuovo e non armato**: un token e' un canale, e
    /// fabbricarlo qui non lega niente a niente. Chi lo lega e' `run`, che
    /// sostituisce quello del processo dopo il parsing. Un test che chiama
    /// `parse` ottiene percio' un token isolato, e non puo' annullare per
    /// sbaglio l'operazione di un altro test.
    pub(crate) cancellazione: CancellationToken,
}

pub fn kv(s: &str) -> Result<(String, String), (i32, Value)> {
    s.split_once('=')
        .map(|(k, v)| (k.to_owned(), v.to_owned()))
        // Il valore non esce: viene da argv. Il flag che lo ha introdotto e'
        // gia' nel messaggio del chiamante.
        .ok_or_else(|| {
            usage_err(&PublicMessage::Curated(
                "opzione di formato non nel formato chiave=valore",
            ))
        })
}

// `flag` e' `&'static str`: tutti i chiamanti passano il nome di un nostro
// flag. Il vocabolario e' chiuso, quindi il nome resta nel messaggio senza
// violare INV-10 — non e' testo runtime, e' uno dei nostri letterali.
// `flag` e' `&'static str`: tutti i chiamanti passano il nome di un nostro
// flag. Il vocabolario e' chiuso, quindi il nome resta nel messaggio senza
// violare INV-10 — non e' testo runtime, e' uno dei nostri letterali.
fn parse_u64(value: Option<&String>, flag: &'static str) -> Result<u64, (i32, Value)> {
    value
        .ok_or_else(|| usage_err(&PublicMessage::CuratedPair(flag, "richiede un valore")))?
        .parse()
        .map_err(|_| {
            usage_err(&PublicMessage::CuratedPair(
                flag,
                "richiede un intero non negativo",
            ))
        })
}

/// I flag che governano una quota, tutti insieme.
///
/// Estratti da `parse` perche' la funzione superava il tetto di righe, ma
/// stanno bene insieme anche di merito: sono l'unico gruppo di flag che
/// finisce nello stesso posto — `PipelineLimits` — e che condivide la stessa
/// disciplina fail-closed. Nessuno di loro degrada a un default quando il
/// valore e' assente o malformato.
///
/// Ritorna `None` se il flag non e' una quota, cosi' `parse` prosegue con i
/// propri casi invece di dover sapere quali sono.
///
/// # Errors
///
/// Se il flag e' una quota ma il valore manca o non e' un intero.
fn limite_da_flag<'a>(
    flag: &str,
    limiti: PipelineLimits,
    it: &mut impl Iterator<Item = &'a String>,
) -> Result<Option<PipelineLimits>, (i32, Value)> {
    if let Some(rule) = plenora_io_model::quota::QUOTAS
        .iter()
        .find(|rule| rule.cli == Some(flag))
    {
        let value = it
            .next()
            .ok_or_else(|| crate::map_err(rule.quota.invalid()))?;
        let value = rule.quota.parse(value).map_err(crate::map_err)?;
        return rule
            .quota
            .set(limiti, value)
            .map(Some)
            .map_err(crate::map_err);
    }
    let aggiornati = match flag {
        // La deadline della pipeline, finora fissata a 30 000 ms nel modello e
        // non raggiungibile da riga di comando. Sta con le altre quote perche'
        // e' una quota: governa il tempo come `--max-rows` governa le righe, e
        // condivide la disciplina fail-closed del gruppo — lo zero lo rifiuta
        // `PipelineLimits::validate`, un valore che sposta la scadenza oltre
        // cio' che `Instant` rappresenta lo rifiuta `build`. Nessuno dei due
        // degrada al default.
        "--deadline-ms" => limiti.with_duration_ms(parse_u64(it.next(), "--deadline-ms")?),
        _ => return Ok(None),
    };
    Ok(Some(aggiornati))
}

/// Legge gli argomenti in una `Cli`.
///
/// # Errors
///
/// `Err` quando un argomento manca, e' ripetuto o non e' del tipo atteso: la
/// busta ha categoria `invalid_configuration` e codice `CLI_USAGE`, e l'`i32`
/// e' la proiezione che il binding applichera'.
pub fn parse(args: &[String]) -> Result<Cli, (i32, Value)> {
    let mut cli = Cli::default();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        if a.starts_with('-')
            && !cli.flags.insert(a.clone())
            && !matches!(a.as_str(), "--opt" | "--in-opt" | "--out-opt")
        {
            return Err(usage_err(&PublicMessage::Curated("flag ripetuto")));
        }
        // Le quote si riconoscono prima: sono un gruppo omogeneo che finisce
        // tutto in `PipelineLimits`, e tenerle qui allungava `parse` senza
        // aggiungere niente a chi la legge.
        if let Some(aggiornati) = limite_da_flag(a.as_str(), cli.limits, &mut it)? {
            cli.limits = aggiornati;
            continue;
        }
        match a.as_str() {
            "--source-base64" | "--archive-entry" => {
                let value = it
                    .next()
                    .ok_or_else(|| {
                        usage_err(&PublicMessage::Curated("opzione sorgente senza valore"))
                    })?
                    .clone();
                if a == "--source-base64" {
                    cli.source_base64 = Some(value);
                } else {
                    cli.archive_entry = Some(value);
                }
            }
            "--archive-max-entries" | "--archive-max-bytes" | "--archive-max-ratio" => {
                let value = parse_u64(it.next(), "limite archivio")?;
                match a.as_str() {
                    "--archive-max-bytes" => cli.archive_limits.max_byte_decompressi = value,
                    "--archive-max-entries" => {
                        cli.archive_limits.max_voci = u32::try_from(value).map_err(|_| {
                            usage_err(&PublicMessage::Curated("limite archivio fuori intervallo"))
                        })?
                    }
                    _ => {
                        cli.archive_limits.max_rapporto = u32::try_from(value).map_err(|_| {
                            usage_err(&PublicMessage::Curated("limite archivio fuori intervallo"))
                        })?
                    }
                }
            }
            "--assume-crs" => {
                cli.assume_crs = Some(
                    it.next()
                        .ok_or_else(|| {
                            usage_err(&PublicMessage::Curated("--assume-crs richiede un valore"))
                        })?
                        .clone(),
                );
            }
            // Il selettore del modo macchina e' accettato da ogni comando, ed
            // e' l'unico valore ammesso: `--format` con altro non e' una
            // modalita' che non abbiamo, e' un refuso che fallisce chiuso.
            //
            // Il percorso e' **qualificato**, e non e' uno stile. Un nome nudo
            // in posizione di pattern che non risolve a una costante diventa un
            // binding che cattura tutto, e il compilatore lo dice soltanto con
            // un avviso: e' successo estraendo questo modulo, e otto prove di
            // `consegna_arrow` sono diventate rosse. Con `crate::` davanti, un
            // nome che non risolve e' un **errore**, non un catch-all.
            crate::FLAG_FORMATO => {
                let v = it.next().ok_or_else(|| {
                    usage_err(&PublicMessage::Curated("--format richiede un valore"))
                })?;
                if v != FORMATO_JSON {
                    return Err(usage_err(&PublicMessage::Curated(
                        "--format ammette soltanto json",
                    )));
                }
            }
            "--output" => {
                let v = it.next().ok_or_else(|| {
                    usage_err(&PublicMessage::Curated("--output richiede un percorso"))
                })?;
                cli.output = Some(PathBuf::from(v));
            }
            "--layer" => {
                let v = it.next().ok_or_else(|| {
                    usage_err(&PublicMessage::Curated("--layer richiede un valore"))
                })?;
                cli.layer = Some(v.parse().map_err(|_| {
                    usage_err(&PublicMessage::Curated("--layer richiede un intero"))
                })?);
            }
            "--limit" => {
                let v = it.next().ok_or_else(|| {
                    usage_err(&PublicMessage::Curated("--limit richiede un valore"))
                })?;
                cli.limit = Some(v.parse().map_err(|_| {
                    usage_err(&PublicMessage::Curated("--limit richiede un intero"))
                })?);
            }
            "--from" => {
                cli.from_ = Some(
                    it.next()
                        .ok_or_else(|| {
                            usage_err(&PublicMessage::Curated(
                                "--from richiede un identificatore di formato",
                            ))
                        })?
                        .clone(),
                );
            }
            "--to" => {
                cli.to = Some(
                    it.next()
                        .ok_or_else(|| {
                            usage_err(&PublicMessage::Curated(
                                "--to richiede un identificatore di formato",
                            ))
                        })?
                        .clone(),
                );
            }
            "--durable" => cli.durable = true,
            // Il nome dice che cosa si sta scegliendo. `--protocol 1` sarebbe
            // stato piu' corto e avrebbe fatto sembrare le due versioni due
            // opzioni pari: non lo sono.
            "--opt" => insert_option(&mut cli.opts, it.next())?,
            "--in-opt" => insert_option(&mut cli.in_opts, it.next())?,
            "--out-opt" => insert_option(&mut cli.out_opts, it.next())?,
            other if other.starts_with("--") => {
                // Il token non esce: viene da argv. Resta la condizione, e
                // l'uso completo e' nel messaggio del comando senza argomenti.
                return Err(usage_err(&PublicMessage::CuratedPair(
                    "opzione sconosciuta; ammesse:",
                    OPZIONI_AMMESSE,
                )));
            }
            _ => cli.positionals.push(a.clone()),
        }
    }
    if cli.limit == Some(0) {
        return Err(usage_err(&PublicMessage::Curated(
            "--limit deve essere maggiore di zero",
        )));
    }
    Ok(cli)
}

/// Insieme chiuso dei flag pertinenti alle operazioni sui dataset.
pub fn validate_command(command: &str, cli: &Cli) -> Result<(), (i32, Value)> {
    let positional_count = if matches!(command, "write" | "convert") {
        2
    } else {
        1
    };
    if cli.positionals.len() > positional_count {
        return Err(usage_err(&PublicMessage::Curated(
            "numero di argomenti posizionali non valido",
        )));
    }
    for flag in &cli.flags {
        let allowed = match flag.as_str() {
            "--source-base64"
            | "--archive-entry"
            | "--archive-max-entries"
            | "--archive-max-bytes"
            | "--archive-max-ratio" => true,
            "--format"
            | "--opt"
            | "--in-opt"
            | "--assume-crs"
            | "--deadline-ms"
            | "--max-input-bytes"
            | "--max-input-entries"
            | "--max-output-bytes"
            | "--max-rows"
            | "--max-columns"
            | "--max-vertices"
            | "--max-geometry-components"
            | "--max-wkb-cell-bytes"
            | "--max-wkb-components"
            | "--max-wkb-depth"
            | "--memory-bytes"
            | "--spill-bytes"
            | "--output-expansion-ratio"
            | "--decompression-ratio" => true,
            "--from" => command == "convert",
            "--to" => matches!(command, "convert" | "write"),
            "--layer" => matches!(command, "read" | "convert" | "write"),
            "--limit" | "--output" => command == "read",
            "--durable" | "--out-opt" => matches!(command, "read" | "convert" | "write"),
            _ => false,
        };
        if !allowed {
            return Err(usage_err(&PublicMessage::Curated(
                "flag non pertinente al comando",
            )));
        }
    }
    Ok(())
}

fn insert_option(
    options: &mut BTreeMap<String, String>,
    value: Option<&String>,
) -> Result<(), (i32, Value)> {
    let (key, value) = kv(value.ok_or_else(|| {
        usage_err(&PublicMessage::Curated(
            "opzione di formato richiede chiave=valore",
        ))
    })?)?;
    if options.insert(key, value).is_some() {
        return Err(usage_err(&PublicMessage::Curated(
            "chiave di opzione ripetuta",
        )));
    }
    Ok(())
}
