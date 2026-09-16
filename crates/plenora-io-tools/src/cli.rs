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
fn parse_usize(value: Option<&String>, flag: &'static str) -> Result<usize, (i32, Value)> {
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
    let aggiornati = match flag {
        // La deadline della pipeline, finora fissata a 30 000 ms nel modello e
        // non raggiungibile da riga di comando. Sta con le altre quote perche'
        // e' una quota: governa il tempo come `--max-rows` governa le righe, e
        // condivide la disciplina fail-closed del gruppo — lo zero lo rifiuta
        // `PipelineLimits::validate`, un valore che sposta la scadenza oltre
        // cio' che `Instant` rappresenta lo rifiuta `build`. Nessuno dei due
        // degrada al default.
        "--deadline-ms" => limiti.with_duration_ms(parse_u64(it.next(), "--deadline-ms")?),
        "--max-input-bytes" => {
            limiti.with_max_input_bytes(parse_u64(it.next(), "--max-input-bytes")?)
        }
        // Quota di memoria, distinta da quella dell'ingresso: da FZ-0.2.1
        // il tetto su una pagina Parquet non compressa ne e' la meta'.
        // Zero e incoerenze le rifiuta il modello, non il parser; la
        // motivazione estesa sta nel README.
        "--memory-bytes" => limiti.with_memory_bytes(parse_u64(it.next(), "--memory-bytes")?),
        "--max-input-entries" => {
            limiti.with_max_input_entries(parse_u64(it.next(), "--max-input-entries")?)
        }
        "--max-output-bytes" => {
            limiti.with_max_output_bytes(parse_u64(it.next(), "--max-output-bytes")?)
        }
        "--max-rows" => limiti.with_max_rows(parse_u64(it.next(), "--max-rows")?),
        "--max-columns" => limiti.with_max_columns(parse_u64(it.next(), "--max-columns")?),
        "--max-vertices" => limiti.with_max_vertices(parse_usize(it.next(), "--max-vertices")?),
        "--max-wkb-cell-bytes" => {
            limiti.with_max_wkb_cell_bytes(parse_usize(it.next(), "--max-wkb-cell-bytes")?)
        }
        "--max-wkb-components" => {
            limiti.with_max_wkb_components(parse_usize(it.next(), "--max-wkb-components")?)
        }
        "--max-wkb-depth" => limiti.with_max_wkb_depth(parse_usize(it.next(), "--max-wkb-depth")?),
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
        // Le quote si riconoscono prima: sono un gruppo omogeneo che finisce
        // tutto in `PipelineLimits`, e tenerle qui allungava `parse` senza
        // aggiungere niente a chi la legge.
        if let Some(aggiornati) = limite_da_flag(a.as_str(), cli.limits, &mut it)? {
            cli.limits = aggiornati;
            continue;
        }
        match a.as_str() {
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
            "--opt" => {
                let (k, v) = kv(it.next().ok_or_else(|| {
                    usage_err(&PublicMessage::Curated("--opt richiede chiave=valore"))
                })?)?;
                cli.opts.insert(k, v);
            }
            "--in-opt" => {
                let (k, v) = kv(it.next().ok_or_else(|| {
                    usage_err(&PublicMessage::Curated("--in-opt richiede chiave=valore"))
                })?)?;
                cli.in_opts.insert(k, v);
            }
            "--out-opt" => {
                let (k, v) = kv(it.next().ok_or_else(|| {
                    usage_err(&PublicMessage::Curated("--out-opt richiede chiave=valore"))
                })?)?;
                cli.out_opts.insert(k, v);
            }
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
    Ok(cli)
}
