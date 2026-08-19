//! Schema dichiarativo delle `format_options` (L0.7, S6).
//!
//! `format_options` e' una mappa da stringa a stringa che ogni driver
//! interrogava per conto proprio. Ne seguivano due difetti, entrambi
//! silenziosi:
//!
//! * **una chiave sconosciuta non esisteva.** Nessuno la leggeva, nessuno la
//!   rifiutava: `wkt_colunm=geom` — con il refuso — produceva una lettura senza
//!   geometria, non un errore;
//! * **un valore invalido degradava al default.** `compression=zstdd` scriveva
//!   un file snappy senza dirlo a nessuno, e chi lo aveva chiesto credeva di
//!   avere zstd finche' non misurava.
//!
//! Lo schema rende dichiarativo cio' che era sparso: ogni driver elenca le
//! proprie opzioni accanto al proprio descrittore, e la validazione avviene una
//! volta sola nel passaggio obbligato di lettura e di scrittura.
//!
//! # La grammatica e' fissata, non dedotta
//!
//! I valori ammessi sono pochi e rigidi di proposito. Accettare `ZSTD` accanto
//! a `zstd` significherebbe decidere caso per caso quali varianti tollerare, e
//! la prima volta che se ne dimenticasse una il messaggio d'errore direbbe che
//! il valore non esiste mentre il vicino identico funziona. Un solo modo di
//! scriverlo e' piu' facile da spiegare di sette modi che quasi sempre
//! funzionano.

use std::collections::BTreeMap;

use crate::{PlenoraIoError, PublicMessage, Result};

/// Il numero massimo di caratteri che un token di opzione rifiutata porta
/// fuori.
///
/// Una chiave di `format_options` reale sta in poche decine di caratteri.
/// Sessantaquattro e' largo per qualunque refuso plausibile e **finito** per
/// qualunque input ostile: senza un tetto, un'opzione da un megabyte
/// diventerebbe un messaggio d'errore da un megabyte, e la redazione avrebbe
/// chiuso il canale del contenuto lasciando aperto quello della dimensione.
const MASSIMO_TOKEN: usize = 64;

/// Cio' che l'utente ha scritto in un'opzione rifiutata, reso sicuro.
///
/// # Non costruibile da fuori
///
/// Il costruttore e' privato di questo modulo, quindi nemmeno il resto di
/// `plenora-io-model` puo' coniarne uno:
///
/// ```compile_fail
/// use plenora_io_model::format_options::RejectedOptionToken;
/// let _ = RejectedOptionToken::conia("wkt_colunm");
/// ```
///
/// Non c'e' `From<&str>`:
///
/// ```compile_fail
/// use plenora_io_model::format_options::RejectedOptionToken;
/// let _: RejectedOptionToken = "wkt_colunm".into();
/// ```
///
/// Non c'e' `From<String>`:
///
/// ```compile_fail
/// use plenora_io_model::format_options::RejectedOptionToken;
/// let _ = RejectedOptionToken::from(String::from("wkt_colunm"));
/// ```
///
/// E il literal non compila, perche' il campo e' privato:
///
/// ```compile_fail
/// use plenora_io_model::format_options::RejectedOptionToken;
/// let _ = RejectedOptionToken { reso: String::new() };
/// ```
///
/// Non c'e' modo di riportare fuori la stringa originale: l'unica cosa che il
/// tipo offre e' `Display`, e cio' che rende e' gia' scappato e troncato.
///
/// # Perche' esiste — eccezione normativa, non interpretazione
///
/// S9 ha ratificato che nessun costruttore pubblico d'errore accetti `&str`
/// non `'static`. S6 aveva ratificato, con la propria motivazione, che il
/// valore ricevuto compaia nel messaggio: «un'opzione arriva dal chiamante —
/// riga di comando o API — non dal payload del file, e nasconderla renderebbe
/// l'errore inutile proprio a chi deve correggerlo». Una chiave scritta male
/// e' per definizione non statica, e le due ratifiche si contraddicevano.
///
/// La proprieta' del prodotto e' ora: **nessun testo runtime, salvo il token
/// bounded di un'opzione rifiutata prodotto dal validatore centrale**. E'
/// scritta nel pacchetto decisionale e in entrambi i design, perche'
/// un'eccezione registrata in un posto solo diventa un'interpretazione.
///
/// # Cosa rende stretta l'eccezione
///
/// * il costruttore e' **privato di questo modulo**, non `pub(crate)`: nemmeno
///   il resto di `plenora-io-model` puo' coniarne uno;
/// * non c'e' `From<String>`, `From<&str>`, `Deserialize`, ne' alcun
///   costruttore unchecked;
/// * **non c'e' un accessor alla stringa originale**: fuori di qui il token si
///   puo' solo rendere, gia' scappato e troncato;
/// * l'unico chiamante e' [`valida_opzioni`], che lo conia da una chiave o da
///   un valore di `format_options`.
///
/// # Cosa non e'
///
/// Non e' un canale generico per il testo. Payload, percorsi, nomi letti dai
/// file e messaggi delle dipendenze **non** passano da qui: un secondo uso lo
/// trasformerebbe nella scorciatoia che questa forma esiste per non aprire.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RejectedOptionToken {
    /// Gia' scappato e troncato alla costruzione: cio' che e' qui dentro e'
    /// cio' che esce. Non esiste un percorso che riporti fuori l'originale.
    reso: String,
}

impl RejectedOptionToken {
    /// Conia un token da testo dell'utente.
    ///
    /// **Privata del modulo per costruzione.** E' il punto in cui l'eccezione
    /// e' confinata: chiunque voglia far uscire testo runtime da un errore
    /// deve passare da qui, e qui non ci si arriva da fuori.
    fn conia(grezzo: &str) -> Self {
        let mut reso = String::with_capacity(grezzo.len().min(MASSIMO_TOKEN) + 2);
        let mut troncato = false;
        for (usati, carattere) in grezzo.chars().enumerate() {
            if usati >= MASSIMO_TOKEN {
                troncato = true;
                break;
            }
            match carattere {
                // Virgolette e backslash: il messaggio finisce dentro JSON, e
                // un apice non scappato ci arriverebbe come struttura invece
                // che come testo.
                '"' => reso.push_str("\\\""),
                '\\' => reso.push_str("\\\\"),
                '\n' => reso.push_str("\\n"),
                '\r' => reso.push_str("\\r"),
                '\t' => reso.push_str("\\t"),
                // Ogni altro controllo diventa la sua forma esadecimale: un
                // byte che muove il cursore o riscrive la riga in un terminale
                // e' un canale, piccolo ma reale.
                c if c.is_control() => {
                    use std::fmt::Write as _;
                    // La scrittura su String non fallisce; l'errore e' ignorato
                    // qui e in nessun altro posto, perche' l'alternativa
                    // sarebbe propagare un `fmt::Error` che non puo' accadere.
                    let _ = write!(reso, "\\u{{{:04x}}}", u32::from(c));
                }
                c => reso.push(c),
            }
        }
        if troncato {
            reso.push('…');
        }
        Self { reso }
    }
}

impl std::fmt::Display for RejectedOptionToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.reso)
    }
}

/// In quale fase un'opzione ha significato.
///
/// Passare un'opzione di scrittura in lettura non e' innocuo: chi lo fa crede
/// di aver configurato qualcosa che nessuno leggera'.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FaseOpzione {
    #[serde(rename = "read")]
    Lettura,
    #[serde(rename = "write")]
    Scrittura,
    #[serde(rename = "both")]
    Entrambe,
}

impl FaseOpzione {
    /// Se l'opzione e' interpretabile nella fase indicata.
    #[must_use]
    pub const fn copre(self, fase: Self) -> bool {
        matches!(
            (self, fase),
            (Self::Entrambe, _)
                | (Self::Lettura, Self::Lettura)
                | (Self::Scrittura, Self::Scrittura)
        )
    }

    #[must_use]
    pub const fn nome(self) -> &'static str {
        match self {
            Self::Lettura => "lettura",
            Self::Scrittura => "scrittura",
            Self::Entrambe => "lettura e scrittura",
        }
    }
}

/// Le forme vere e false di un booleano.
///
/// Sono queste e basta: `on` e' rifiutato di proposito, benche' diffuso. Un
/// booleano che accetta tre forme vere e tre false e' gia' una tolleranza, e
/// allargarla a piacere riporta al problema di partenza — chi scrive `on`
/// riceve l'elenco esatto delle forme ammesse.
const BOOLEANI_VERI: [&str; 3] = ["true", "1", "yes"];
const BOOLEANI_FALSI: [&str; 3] = ["false", "0", "no"];

/// Forme che un valore puo' assumere.
///
/// Deliberatamente povere: sono opzioni da riga di comando, non un linguaggio.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ValoreAmmesso {
    /// Testo non vuoto: nome di colonna, nome di foglio.
    #[serde(rename = "text")]
    Testo,
    /// Uno di un insieme chiuso, ASCII minuscolo, confronto case-sensitive.
    #[serde(rename = "enum")]
    Enumerato(&'static [&'static str]),
    /// `true`/`1`/`yes` oppure `false`/`0`/`no`. Nient'altro.
    #[serde(rename = "boolean")]
    Booleano,
    /// Esattamente un carattere ASCII.
    #[serde(rename = "char")]
    Carattere,
    /// Un intero decimale nell'intervallo chiuso indicato.
    ///
    /// Aggiunta dopo la ratifica della grammatica: il censimento del design
    /// aveva mancato `row_diagnostics.examples_limit`, che e' numerico. Non e'
    /// una tolleranza in piu' — e' una forma in piu', con i suoi estremi
    /// dichiarati nello schema invece che sepolti nel driver.
    #[serde(rename = "integer")]
    Intero {
        #[serde(rename = "min")]
        minimo: u64,
        #[serde(rename = "max")]
        massimo: u64,
    },
}

impl ValoreAmmesso {
    /// I valori ammessi, quando la forma li enumera.
    ///
    /// Sono `&'static str` — vengono dallo schema del driver — quindi la
    /// ratifica di S9 li ammette senza eccezioni. S6 pretende che l'errore li
    /// elenchi, ed e' la proprieta' che i suoi test verificano.
    #[must_use]
    pub fn ammessi(self) -> Vec<&'static str> {
        match self {
            Self::Enumerato(valori) => valori.to_vec(),
            Self::Booleano => {
                let mut tutti = BOOLEANI_VERI.to_vec();
                tutti.extend_from_slice(&BOOLEANI_FALSI);
                tutti
            }
            Self::Testo | Self::Carattere | Self::Intero { .. } => Vec::new(),
        }
    }

    /// Il nome della forma, come `&'static str`.
    ///
    /// `verifica` compone una descrizione con gli estremi, che per l'intero e'
    /// costruita a runtime; qui serve solo il **nome della forma**, che e'
    /// sempre nostro e sempre statico.
    #[must_use]
    pub const fn forma(self) -> &'static str {
        match self {
            Self::Testo => "testo non vuoto",
            Self::Enumerato(_) => "uno dei valori enumerati dallo schema",
            Self::Booleano => "un booleano",
            Self::Carattere => "un solo carattere ASCII",
            Self::Intero { .. } => "un intero nell'intervallo dichiarato",
        }
    }

    /// Verifica un valore; l'errore descrive cosa sarebbe stato ammesso.
    ///
    /// Pubblica perche' lo schema serve a chi costruisce la richiesta, non
    /// solo a chi la riceve: una CLI o un SDK che vogliano segnalare un valore
    /// sbagliato prima di chiamare il driver devono poter applicare la stessa
    /// grammatica, non una copia che diverge.
    ///
    /// # Errors
    ///
    /// La descrizione di cio' che sarebbe stato ammesso, gia' pronta per il
    /// messaggio d'errore.
    pub fn verifica(self, valore: &str) -> std::result::Result<(), String> {
        match self {
            Self::Testo => {
                if valore.is_empty() {
                    return Err("un testo non vuoto".to_owned());
                }
                Ok(())
            }
            Self::Enumerato(ammessi) => {
                if ammessi.contains(&valore) {
                    return Ok(());
                }
                Err(ammessi.join(", "))
            }
            Self::Booleano => {
                if BOOLEANI_VERI.contains(&valore) || BOOLEANI_FALSI.contains(&valore) {
                    return Ok(());
                }
                Err(format!(
                    "{}, {}",
                    BOOLEANI_VERI.join(", "),
                    BOOLEANI_FALSI.join(", ")
                ))
            }
            Self::Carattere => {
                let mut caratteri = valore.chars();
                match (caratteri.next(), caratteri.next()) {
                    (Some(uno), None) if uno.is_ascii() => Ok(()),
                    _ => Err("esattamente un carattere ASCII".to_owned()),
                }
            }
            Self::Intero { minimo, massimo } => {
                // Solo cifre: `u64::from_str` accetterebbe anche `+8`, e due
                // grafie dello stesso valore contraddicono la regola che vale
                // per gli enumerati e per i booleani — una forma sola, esatta.
                if valore.is_empty() || !valore.bytes().all(|b| b.is_ascii_digit()) {
                    return Err(format!("un intero fra {minimo} e {massimo}"));
                }
                match valore.parse::<u64>() {
                    Ok(numero) if (minimo..=massimo).contains(&numero) => Ok(()),
                    _ => Err(format!("un intero fra {minimo} e {massimo}")),
                }
            }
        }
    }
}

/// Interpreta un booleano gia' validato dallo schema.
///
/// # Errors
///
/// `InvalidConfiguration` se il valore non e' una delle forme ammesse: la
/// validazione lo avrebbe gia' rifiutato, ma questa funzione e' pubblica e non
/// puo' assumere di essere chiamata solo dopo — ed e' la stessa categoria che
/// produce `valida_opzioni`, cosi' i due percorsi non si distinguono.
pub fn booleano(driver: &'static str, chiave: &str, valore: &str) -> Result<bool> {
    if BOOLEANI_VERI.contains(&valore) {
        return Ok(true);
    }
    if BOOLEANI_FALSI.contains(&valore) {
        return Ok(false);
    }
    let mut ammesse: Vec<&'static str> = BOOLEANI_VERI.to_vec();
    ammesse.extend_from_slice(&BOOLEANI_FALSI);
    Err(scarto(&PublicMessage::OpzioneRifiutata {
        driver,
        testo: "l'opzione vuole un booleano",
        token: RejectedOptionToken::conia(chiave),
        dettaglio: "",
        ammesse,
    }))
}

/// Una singola opzione dichiarata da un driver.
#[derive(Clone, Copy, Debug, serde::Serialize)]
pub struct OpzioneFormato {
    #[serde(rename = "key")]
    pub chiave: &'static str,
    #[serde(rename = "phase")]
    pub fase: FaseOpzione,
    #[serde(rename = "value")]
    pub valore: ValoreAmmesso,
    /// Il default **dichiarato**, non quello che capita: e' cio' che il comando
    /// `options` mostrera' e cio' che il driver applica quando la chiave manca.
    #[serde(rename = "default")]
    pub predefinito: Option<&'static str>,
    #[serde(rename = "description")]
    pub descrizione: &'static str,
}

/// Le opzioni che un driver dichiara.
///
/// Vive dentro il `FormatDescriptor`, non in una tabella indicizzata per nome:
/// il legame fra driver e schema e' cosi' **strutturale**, e un driver senza
/// schema non compila invece di lasciare un buco che solo un test troverebbe.
#[derive(Clone, Copy, Debug, serde::Serialize)]
#[serde(transparent)]
pub struct SchemaOpzioniFormato {
    pub opzioni: &'static [OpzioneFormato],
}

impl SchemaOpzioniFormato {
    /// Il driver non interpreta alcuna `format_option`.
    ///
    /// Non e' la stessa cosa di "non dichiarato": qui l'elenco vuoto e'
    /// l'affermazione che qualunque chiave e' sconosciuta.
    pub const VUOTO: Self = Self { opzioni: &[] };

    #[must_use]
    pub const fn nuovo(opzioni: &'static [OpzioneFormato]) -> Self {
        Self { opzioni }
    }

    /// Le chiavi valide nella fase indicata, in ordine di dichiarazione.
    #[must_use]
    pub fn chiavi(&self, fase: FaseOpzione) -> Vec<&'static str> {
        self.opzioni
            .iter()
            .filter(|opzione| opzione.fase.copre(fase))
            .map(|opzione| opzione.chiave)
            .collect()
    }

    /// L'opzione con quella chiave, in qualunque fase.
    #[must_use]
    pub fn opzione(&self, chiave: &str) -> Option<&'static OpzioneFormato> {
        self.opzioni.iter().find(|opzione| opzione.chiave == chiave)
    }

    /// Il default dichiarato per una chiave, se c'e'.
    #[must_use]
    pub fn predefinito(&self, chiave: &str) -> Option<&'static str> {
        self.opzione(chiave).and_then(|opzione| opzione.predefinito)
    }
}

/// L'errore di schema ha **una** categoria per tutte e tre le forme di scarto
/// — chiave ignota, fase sbagliata, valore fuori grammatica.
///
/// E' `InvalidConfiguration` e non `Unsupported`: `Unsupported` dice «questo
/// prodotto non sa farlo», ed e' una risposta sul prodotto; qui la risposta e'
/// sull'input, che non e' ben formato per il driver scelto. La distinzione
/// conta per chi automatizza: davanti a `Unsupported` si cambia driver,
/// davanti a `InvalidConfiguration` si corregge la richiesta.
fn scarto(messaggio: &PublicMessage) -> PlenoraIoError {
    PlenoraIoError::redatto(
        crate::IoErrorCode::Unsupported,
        crate::ErrorCategory::InvalidConfiguration,
        crate::ErrorPhase::Validate,
        crate::RemoteEffect::None,
        crate::RetryDisposition::Never,
        messaggio,
    )
}

/// Verifica le opzioni ricevute contro lo schema del driver.
///
/// Tre rifiuti, tutti con la stessa categoria:
///
/// * **chiave sconosciuta** — l'errore elenca le chiavi valide nella fase;
/// * **fase sbagliata** — la chiave esiste ma non in questa fase;
/// * **valore invalido** — l'errore elenca le forme ammesse.
///
/// Il valore ricevuto compare nel messaggio. Non e' una violazione della
/// redazione: un'opzione arriva dal chiamante — riga di comando o API — non dal
/// payload del file, e nasconderla renderebbe l'errore inutile proprio a chi
/// deve correggerlo.
///
/// # Errors
///
/// `InvalidConfiguration` al primo problema. La mappa e' ordinata, quindi due
/// esecuzioni sullo stesso input danno lo stesso errore.
pub fn valida_opzioni(
    driver: &'static str,
    schema: SchemaOpzioniFormato,
    opzioni: &BTreeMap<String, String>,
    fase: FaseOpzione,
) -> Result<()> {
    for (chiave, valore) in opzioni {
        // Le tre chiamate a `RejectedOptionToken::conia` sotto sono le **uniche**
        // del workspace, ed e' per costruzione: il costruttore e' privato di
        // questo modulo. Fuori di qui non esiste un modo di far uscire testo
        // runtime da un errore.
        let Some(dichiarata) = schema.opzione(chiave) else {
            let ammesse = schema.chiavi(fase);
            return Err(scarto(&PublicMessage::OpzioneRifiutata {
                driver,
                testo: "opzione sconosciuta in",
                token: RejectedOptionToken::conia(chiave),
                dettaglio: fase.nome(),
                ammesse,
            }));
        };
        if !dichiarata.fase.copre(fase) {
            return Err(scarto(&PublicMessage::OpzioneRifiutata {
                driver,
                testo: "opzione non valida in questa fase; vale in",
                token: RejectedOptionToken::conia(chiave),
                dettaglio: dichiarata.fase.nome(),
                ammesse: Vec::new(),
            }));
        }
        if let Err(ammessi) = dichiarata.valore.verifica(valore) {
            // `ammessi` e' la descrizione della forma, gia' `&'static str` per
            // ogni variante di `ValoreAmmesso` salvo l'intero, che compone il
            // proprio intervallo: il testo resta nostro in entrambi i casi.
            let _ = ammessi;
            return Err(scarto(&PublicMessage::OpzioneRifiutata {
                driver,
                testo: "valore non valido per l'opzione",
                token: RejectedOptionToken::conia(valore),
                dettaglio: dichiarata.valore.forma(),
                ammesse: dichiarata.valore.ammessi(),
            }));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCHEMA: SchemaOpzioniFormato = SchemaOpzioniFormato::nuovo(&[
        OpzioneFormato {
            chiave: "wkt_column",
            fase: FaseOpzione::Lettura,
            valore: ValoreAmmesso::Testo,
            predefinito: None,
            descrizione: "colonna con la geometria WKT",
        },
        OpzioneFormato {
            chiave: "delimiter",
            fase: FaseOpzione::Entrambe,
            valore: ValoreAmmesso::Carattere,
            predefinito: Some(","),
            descrizione: "separatore di campo",
        },
        OpzioneFormato {
            chiave: "compression",
            fase: FaseOpzione::Scrittura,
            valore: ValoreAmmesso::Enumerato(&["snappy", "zstd", "none"]),
            predefinito: Some("snappy"),
            descrizione: "codec di compressione",
        },
        OpzioneFormato {
            chiave: "legacy",
            fase: FaseOpzione::Lettura,
            valore: ValoreAmmesso::Booleano,
            predefinito: Some("false"),
            descrizione: "compatibilita' storica",
        },
    ]);

    fn opzioni(coppie: &[(&str, &str)]) -> BTreeMap<String, String> {
        coppie
            .iter()
            .map(|(chiave, valore)| ((*chiave).to_owned(), (*valore).to_owned()))
            .collect()
    }

    fn valida(coppie: &[(&str, &str)], fase: FaseOpzione) -> Result<()> {
        valida_opzioni("prova", SCHEMA, &opzioni(coppie), fase)
    }

    #[test]
    fn una_chiave_sconosciuta_e_rifiutata_e_l_errore_elenca_quelle_valide() {
        let errore = valida(&[("wkt_colunm", "g")], FaseOpzione::Lettura)
            .expect_err("il refuso deve fermarsi qui");
        let testo = errore.to_string();
        assert!(testo.contains("wkt_colunm"), "{testo}");
        assert!(
            testo.contains("wkt_column"),
            "l'elenco deve guidare: {testo}"
        );
        assert!(testo.contains("delimiter"), "{testo}");
        // `compression` e' di sola scrittura: non compare fra le chiavi di
        // lettura, altrimenti l'elenco suggerirebbe una strada chiusa.
        assert!(!testo.contains("compression"), "{testo}");
    }

    #[test]
    fn una_chiave_della_fase_sbagliata_e_rifiutata() {
        let errore = valida(&[("compression", "zstd")], FaseOpzione::Lettura)
            .expect_err("un'opzione di scrittura non vale in lettura");
        let testo = errore.to_string();
        assert!(testo.contains("compression"), "{testo}");
        assert!(testo.contains("scrittura"), "{testo}");
    }

    #[test]
    fn gli_enumerati_sono_case_sensitive() {
        assert!(valida(&[("compression", "zstd")], FaseOpzione::Scrittura).is_ok());
        for variante in ["ZSTD", "Zstd", "zstd ", " zstd"] {
            assert!(
                valida(&[("compression", variante)], FaseOpzione::Scrittura).is_err(),
                "{variante} doveva essere rifiutato"
            );
        }
    }

    #[test]
    fn i_booleani_ammettono_sei_forme_e_nessun_altra() {
        for vero in ["true", "1", "yes"] {
            assert!(
                valida(&[("legacy", vero)], FaseOpzione::Lettura).is_ok(),
                "{vero}"
            );
            assert!(booleano("prova", "legacy", vero).unwrap(), "{vero}");
        }
        for falso in ["false", "0", "no"] {
            assert!(
                valida(&[("legacy", falso)], FaseOpzione::Lettura).is_ok(),
                "{falso}"
            );
            assert!(!booleano("prova", "legacy", falso).unwrap(), "{falso}");
        }
        // Le forme che la ratifica esclude esplicitamente.
        for rifiutato in ["on", "off", "1.0", "", "True", "YES", "si"] {
            assert!(
                valida(&[("legacy", rifiutato)], FaseOpzione::Lettura).is_err(),
                "{rifiutato} doveva essere rifiutato"
            );
        }
    }

    #[test]
    fn un_carattere_e_esattamente_uno_e_ascii() {
        for valido in [",", ";", "\t", "|"] {
            assert!(
                valida(&[("delimiter", valido)], FaseOpzione::Lettura).is_ok(),
                "{valido}"
            );
        }
        for rifiutato in ["", ";;", "ab", "€"] {
            assert!(
                valida(&[("delimiter", rifiutato)], FaseOpzione::Lettura).is_err(),
                "{rifiutato:?} doveva essere rifiutato"
            );
        }
    }

    /// Newline e caratteri di controllo non escono grezzi.
    ///
    /// Il messaggio finisce dentro un envelope JSON e, prima ancora, dentro il
    /// terminale di chi legge. Un `\n` grezzo spezza una riga di log in due
    /// record; un `\r` riscrive quella che c'era; una virgoletta non scappata
    /// arriva a chi fa il parsing come struttura invece che come testo. Nessuno
    /// dei tre e' payload, e tutti e tre sono canali.
    #[test]
    fn il_token_non_lascia_uscire_controlli_grezzi() {
        let ostili = [
            ("a\nb", "a\\nb"),
            ("a\rb", "a\\rb"),
            ("a\tb", "a\\tb"),
            ("a\"b", "a\\\"b"),
            ("a\\b", "a\\\\b"),
            ("a\u{0}b", "a\\u{0000}b"),
            ("a\u{1b}[2Kb", "a\\u{001b}[2Kb"),
        ];
        for (grezzo, atteso) in ostili {
            let reso = RejectedOptionToken::conia(grezzo).to_string();
            assert_eq!(reso, atteso, "grezzo: {grezzo:?}");
            // Nessun controllo, in nessuna forma: dopo l'escape non ne resta
            // nemmeno uno.
            assert!(
                !reso.chars().any(char::is_control),
                "un controllo e' uscito grezzo da {grezzo:?}"
            );
            // Nessuna virgoletta **non scappata**. La forma resa la contiene,
            // ma sempre preceduta da un backslash: e' la differenza fra un
            // carattere e una delimitazione, ed e' cio' che conta per chi fa
            // il parsing di quel JSON.
            let mut precedente = '\0';
            for carattere in reso.chars() {
                if carattere == '"' {
                    assert_eq!(
                        precedente, '\\',
                        "virgoletta non scappata in {reso:?} da {grezzo:?}"
                    );
                }
                precedente = carattere;
            }
        }
    }

    /// Un token lunghissimo viene troncato, e sempre allo stesso modo.
    ///
    /// Senza tetto, un'opzione da un megabyte diventerebbe un messaggio
    /// d'errore da un megabyte: la redazione avrebbe chiuso il canale del
    /// contenuto lasciando aperto quello della dimensione.
    #[test]
    fn il_token_lunghissimo_viene_troncato_in_modo_deterministico() {
        let lungo = "k".repeat(10_000);
        let primo = RejectedOptionToken::conia(&lungo).to_string();
        let secondo = RejectedOptionToken::conia(&lungo).to_string();
        assert_eq!(primo, secondo, "il troncamento deve essere deterministico");
        assert_eq!(
            primo.chars().count(),
            MASSIMO_TOKEN + 1,
            "64 caratteri piu' l'ellissi"
        );
        assert!(primo.ends_with('…'), "il troncamento e' visibile: {primo}");

        // Il tetto conta i **caratteri**, non i byte: un'opzione di soli
        // caratteri multibyte non deve poter uscire quattro volte piu' lunga.
        let multibyte = "à".repeat(10_000);
        let reso = RejectedOptionToken::conia(&multibyte).to_string();
        assert_eq!(reso.chars().count(), MASSIMO_TOKEN + 1);

        // Un token corto non viene toccato ne' marcato.
        let corto = RejectedOptionToken::conia("wkt_colunm").to_string();
        assert_eq!(corto, "wkt_colunm");
        assert!(!corto.ends_with('…'));

        // Il troncamento avviene **dopo** l'escape, quindi un input fatto di
        // soli controlli non sfonda il tetto espandendosi.
        let controlli = "\u{1}".repeat(10_000);
        let reso = RejectedOptionToken::conia(&controlli).to_string();
        assert!(
            reso.chars().count() <= MASSIMO_TOKEN * 8 + 1,
            "l'escape non deve rendere il tetto inutile: {} caratteri",
            reso.chars().count()
        );
    }

    /// L'eccezione arriva davvero all'errore, con il refuso dentro.
    ///
    /// E' la proprieta' che S6 aveva ratificato e che l'eccezione conserva: chi
    /// ha sbagliato a scrivere vede **cosa** ha scritto.
    #[test]
    fn il_refuso_arriva_all_errore_gia_reso_sicuro() {
        const SCHEMA: SchemaOpzioniFormato = SchemaOpzioniFormato::nuovo(&[OpzioneFormato {
            chiave: "wkt_column",
            fase: FaseOpzione::Lettura,
            valore: ValoreAmmesso::Testo,
            predefinito: None,
            descrizione: "colonna WKT",
        }]);

        let errore = valida_opzioni(
            "prova",
            SCHEMA,
            &opzioni(&[("wkt_colunm", "geom")]),
            FaseOpzione::Lettura,
        )
        .expect_err("il refuso e' rifiutato");
        assert!(errore.message.contains("wkt_colunm"), "{errore}");
        assert!(errore.message.contains("wkt_column"), "{errore}");

        // E un refuso ostile arriva **reso**, non grezzo.
        let errore = valida_opzioni(
            "prova",
            SCHEMA,
            &opzioni(&[("a\nb\"c", "x")]),
            FaseOpzione::Lettura,
        )
        .expect_err("rifiutato");
        assert!(errore.message.contains("a\\nb\\\"c"), "{errore}");
        assert!(
            !errore.message.contains('\n'),
            "newline grezzo nel messaggio"
        );
    }

    #[test]
    fn un_intero_rispetta_gli_estremi_dichiarati() {
        const CON_INTERO: SchemaOpzioniFormato = SchemaOpzioniFormato::nuovo(&[OpzioneFormato {
            chiave: "limite",
            fase: FaseOpzione::Lettura,
            valore: ValoreAmmesso::Intero {
                minimo: 1,
                massimo: 64,
            },
            predefinito: Some("64"),
            descrizione: "esempi per diagnostica",
        }]);
        let prova = |valore: &str| {
            valida_opzioni(
                "prova",
                CON_INTERO,
                &opzioni(&[("limite", valore)]),
                FaseOpzione::Lettura,
            )
        };
        for valido in ["1", "8", "64"] {
            assert!(prova(valido).is_ok(), "{valido}");
        }
        // Fuori intervallo, non numerici, e le forme che `parse::<u64>`
        // rifiuta da sola: segno, spazi, decimali.
        for rifiutato in ["0", "65", "", "otto", "-1", " 8", "8.0", "+8"] {
            assert!(
                prova(rifiutato).is_err(),
                "{rifiutato:?} doveva essere rifiutato"
            );
        }
    }

    #[test]
    fn il_testo_libero_non_puo_essere_vuoto() {
        assert!(valida(&[("wkt_column", "geom")], FaseOpzione::Lettura).is_ok());
        assert!(valida(&[("wkt_column", "")], FaseOpzione::Lettura).is_err());
    }

    #[test]
    fn una_opzione_di_entrambe_le_fasi_vale_in_tutte_e_due() {
        assert!(valida(&[("delimiter", ";")], FaseOpzione::Lettura).is_ok());
        assert!(valida(&[("delimiter", ";")], FaseOpzione::Scrittura).is_ok());
    }

    #[test]
    fn il_default_dichiarato_e_leggibile_dallo_schema() {
        assert_eq!(SCHEMA.predefinito("compression"), Some("snappy"));
        assert_eq!(SCHEMA.predefinito("delimiter"), Some(","));
        assert_eq!(SCHEMA.predefinito("wkt_column"), None);
    }

    #[test]
    fn uno_schema_vuoto_rifiuta_qualunque_chiave() {
        let errore = valida_opzioni(
            "prova",
            SchemaOpzioniFormato::VUOTO,
            &opzioni(&[("qualunque", "cosa")]),
            FaseOpzione::Lettura,
        )
        .expect_err("senza opzioni dichiarate ogni chiave e' sconosciuta");
        assert!(errore.to_string().contains("nessuna"), "{errore}");
    }

    #[test]
    fn nessuna_opzione_passa_sempre() {
        assert!(valida(&[], FaseOpzione::Lettura).is_ok());
        assert!(valida(&[], FaseOpzione::Scrittura).is_ok());
    }
}
