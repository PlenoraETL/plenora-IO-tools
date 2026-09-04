#!/usr/bin/env python3
"""Sonde di `check_protocollo_v2.py`.

Un gate verde sul repository sano dice che oggi e' verde, non che domani
diventerebbe rosso. Ogni proprieta' che il gate afferma ha qui una sonda che la
viola su un manifesto finto e pretende il rosso.

Il manifesto finto porta numeri **tutti diversi fra loro** e diversi da quelli
veri. Con nove chiavi che valgono tutte 64 una permutazione passerebbe
inosservata, e la sonda direbbe di aver verificato un confronto che non ha
fatto.
"""

from __future__ import annotations

import copy
import unittest

from scripts import check_protocollo_v2 as gate

#: Un valore distinto per costante: vedi il docstring del modulo.
NOTE_SANE = {costante: 10 + i for i, costante in enumerate(gate.MAPPATURA.values())}
LIMITI_SANI = {chiave: NOTE_SANE[costante] for chiave, costante in gate.MAPPATURA.items()}
#: I due payload sono **derivati** dai tetti, e il manifesto sano li porta gia'
#: coerenti: le sonde li rompono una alla volta.
LIMITI_SANI["payload_stringhe_v2_trattenute_ragioni"] = (
    LIMITI_SANI["ragioni_trattenute"] * LIMITI_SANI["byte_per_dettaglio_curato"]
)
LIMITI_SANI["payload_stringhe_v2_trattenute_esempi"] = LIMITI_SANI["esempi_trattenuti"] * (
    LIMITI_SANI["byte_per_identificatore_di_categoria"]
    + LIMITI_SANI["byte_per_dettaglio_curato"]
)
SONDE_SANE = {"una_sonda", "un_altra_sonda"}
REGISTRO_SANO = {"limite_di_lunghezza_byte": NOTE_SANE["MAX_BYTE_ID_CATEGORIA"]}

#: Il comportamento **ricavato dal codice**, come lo restituisce il gate. Le
#: sonde del confronto lo iniettano; quelle della derivazione lo producono da un
#: sorgente finto, ed e' li' che si prova che venga davvero dal codice.
COMPORTAMENTO_SANO = {
    "ordine_canonico": {
        "ragioni": ["code", "posizione", "detail"],
        "esempi": ["category", "posizione", "context"],
    },
    "identita_delle_respinte": {
        "ragioni": ["code", "posizione"],
        "esempi": ["posizione"],
    },
    "fonti_di_omesse_per_byte": ["budget_della_sezione", "limite_della_voce"],
    "errori": [],
}

MANIFESTO_SANO = {
    "manifest_version": 1,
    "component": "plenora-IO-tools",
    "protocol_version": 2,
    "status": "in_qualifica",
    "compatibility_scope": "cli_json_only",
    "limiti_della_diagnostica": dict(LIMITI_SANI),
    "determinismo": {
        "ordine_canonico": {
            "ragioni": ["code", "posizione", "detail"],
            "esempi": ["category", "posizione", "context"],
            "come_si_ricava": "dalle due `chiave()`",
            "nota": "prosa che resta",
        }
    },
    "troncamento": {
        "identita_delle_respinte": {
            "ragioni": ["code", "posizione"],
            "esempi": ["posizione"],
        },
        "omesse_per_byte": {"fonti": ["limite_della_voce", "budget_della_sezione"]},
    },
    "sonde_che_lo_provano": sorted(SONDE_SANE),
    "sonde_della_redazione": [],
}


def esito(
    manifesto=None, note=None, esistenti=None, registro=None, comportamento=None
) -> list[str]:
    return gate.verifica(
        copy.deepcopy(MANIFESTO_SANO) if manifesto is None else manifesto,
        dict(NOTE_SANE) if note is None else note,
        set(SONDE_SANE) if esistenti is None else esistenti,
        dict(REGISTRO_SANO) if registro is None else registro,
        copy.deepcopy(COMPORTAMENTO_SANO) if comportamento is None else comportamento,
    )


# --- i sorgenti finti da cui il comportamento si ricava ---------------------
#
# Non sono `loss.rs` e `busta.rs` accorciati per comodita': sono il solo modo di
# provare che il gate **legga** invece di credere. Una sonda che rompesse il
# manifesto proverebbe il confronto; queste rompono il codice, che e' il lato da
# cui la clausola invecchia.

LOSS_FINTO = """\
#[derive({derive_esempi})]
pub struct LossExample {{
    pub category: String,
    pub posizione: Posizione,
    pub context: String,
}}

impl LossExample {{
    fn chiave(&self) -> (&str, Posizione, &str) {{
        ({campi_esempi})
    }}
{chiave_in_piu}}}

impl PartialEq for LossExample {{
    fn eq(&self, altro: &Self) -> bool {{
        {corpo_eq_esempi}
    }}
}}

impl PartialOrd for LossExample {{
    fn partial_cmp(&self, altro: &Self) -> Option<Ordering> {{
        {corpo_partial_cmp_esempi}
    }}
}}

impl Ord for LossExample {{
    fn cmp(&self, altro: &Self) -> Ordering {{
        {corpo_ord_esempi}
    }}
}}

#[derive(Clone, Debug, Serialize)]
pub struct FidelityReason {{
    pub code: FidelityReasonCode,
    pub detail: String,
    pub posizione: Posizione,
}}

impl FidelityReason {{
    fn chiave(&self) -> (FidelityReasonCode, Posizione, &str) {{
        ({campi_ragioni})
    }}
}}

impl PartialEq for FidelityReason {{
    fn eq(&self, altra: &Self) -> bool {{
        self.chiave() == altra.chiave()
    }}
}}

impl PartialOrd for FidelityReason {{
    fn partial_cmp(&self, altra: &Self) -> Option<Ordering> {{
        Some(self.cmp(altra))
    }}
}}

impl Ord for FidelityReason {{
    fn cmp(&self, altra: &Self) -> Ordering {{
        self.chiave().cmp(&altra.chiave())
    }}
}}

#[derive(Clone, Debug)]
pub struct FidelityAssessment {{
    prime_v1: Vec<FidelityReason>,
    respinte: BTreeSet<{elemento_respinte}>,
}}

impl FidelityAssessment {{
    pub fn respinte_per_misura(&self) -> u64 {{
        self.respinte.len() as u64
    }}
}}

#[derive(Clone, Debug, Default)]
pub struct LossReport {{
    esempi: BTreeSet<LossExample>,
    respinti: BTreeSet<{elemento_respinti}>,
}}

impl LossReport {{
    pub fn respinti_per_misura(&self) -> u64 {{
        self.respinti.len() as u64
    }}
}}
"""

BUSTA_FINTA = """\
pub fn sezione_di_perdita(rapporto: &LossReport, budget: usize) -> Value {{
    let (ammesse, fuori_misura): (Vec<_>, Vec<_>) = rapporto
        .counts
        .iter()
        .partition(|(categoria, _)| categoria.len() <= MAX_BYTE_ID_CATEGORIA);
    troncamento.omesse_per_byte = fuori_misura.len() as u64;
    let (counts, per_byte) = entro_il_budget(&base, "counts", ammesse, budget, spingi);
    troncamento.omesse_per_byte = troncamento.omesse_per_byte.saturating_add(per_byte);
    troncamento.omesse_per_byte = troncamento
        .omesse_per_byte
        .saturating_add(rapporto.respinti_per_misura());
{sito_in_piu}}}
"""


def loss_finto(**modifiche) -> str:
    parametri = {
        "derive_esempi": "Clone, Debug, Serialize",
        "campi_esempi": "&self.category, self.posizione, &self.context",
        "campi_ragioni": "self.code, self.posizione, &self.detail",
        "corpo_eq_esempi": "self.chiave() == altro.chiave()",
        "corpo_partial_cmp_esempi": "Some(self.cmp(altro))",
        "corpo_ord_esempi": "self.chiave().cmp(&altro.chiave())",
        "chiave_in_piu": "",
        "elemento_respinte": "(FidelityReasonCode, Posizione)",
        "elemento_respinti": "Posizione",
    }
    parametri.update(modifiche)
    return LOSS_FINTO.format(**parametri)


def busta_finta(sito_in_piu: str = "") -> str:
    return BUSTA_FINTA.format(sito_in_piu=sito_in_piu)


class IlRepositorySano(unittest.TestCase):
    def test_il_gate_passa_su_cio_che_c_e(self):
        self.assertEqual(gate.verifica(), [])

    def test_il_manifesto_finto_e_verde(self):
        # Se il caso sano fosse rosso, ogni sonda qui sotto sarebbe verde per
        # la ragione sbagliata e non proverebbe niente.
        self.assertEqual(esito(), [])

    def test_la_mappatura_copre_esattamente_le_costanti_del_budget(self):
        # Il verso inverso del gate vale solo se questi due file contengono le
        # costanti del budget e nient'altro. Se un giorno ne ospitassero una
        # estranea, il gate la chiamerebbe «non dichiarata» e avrebbe torto:
        # meglio accorgersene qui che in un rosso da interpretare.
        self.assertEqual(set(gate.costanti()), set(gate.MAPPATURA.values()))


class IDueNumeriDevonoCoincidere(unittest.TestCase):
    def test_un_limite_che_diverge_dalla_costante_e_rosso(self):
        for chiave, costante in gate.MAPPATURA.items():
            manifesto = copy.deepcopy(MANIFESTO_SANO)
            manifesto["limiti_della_diagnostica"][chiave] += 1
            errori = esito(manifesto)
            self.assertTrue(
                any(chiave in e and costante in e for e in errori),
                f"{chiave} divergente da {costante} non e' stato segnalato: {errori}",
            )

    def test_un_limite_assente_dal_manifesto_e_rosso(self):
        for chiave in gate.MAPPATURA:
            manifesto = copy.deepcopy(MANIFESTO_SANO)
            del manifesto["limiti_della_diagnostica"][chiave]
            self.assertTrue(any(chiave in e for e in esito(manifesto)))

    def test_una_costante_che_il_manifesto_non_dichiara_e_rossa(self):
        note = dict(NOTE_SANE)
        note["MAX_QUALCOSA_DI_NUOVO"] = 7
        errori = esito(note=note)
        self.assertTrue(any("MAX_QUALCOSA_DI_NUOVO" in e for e in errori), errori)

    def test_una_costante_sparita_col_limite_ancora_dichiarato_e_rossa(self):
        note = dict(NOTE_SANE)
        del note["MAX_CATEGORIE"]
        errori = esito(note=note)
        self.assertTrue(any("MAX_CATEGORIE" in e for e in errori), errori)

    def test_i_limiti_assenti_o_non_oggetto_sono_rossi(self):
        for valore in (None, [], 64, "molti"):
            manifesto = copy.deepcopy(MANIFESTO_SANO)
            manifesto["limiti_della_diagnostica"] = valore
            self.assertTrue(any("limiti_della_diagnostica" in e for e in esito(manifesto)))


class LIdentitaDelManifesto(unittest.TestCase):
    def test_un_campo_di_identita_sbagliato_e_rosso(self):
        for campo, atteso in gate.IDENTITA.items():
            manifesto = copy.deepcopy(MANIFESTO_SANO)
            manifesto[campo] = "sbagliato" if isinstance(atteso, str) else 99
            self.assertTrue(any(campo in e for e in esito(manifesto)))

    def test_un_manifesto_che_si_dichiara_v1_e_rosso(self):
        # E' il caso che conta: un v2 che si presenta come v1 verrebbe letto
        # come il contratto congelato, che promette un'altra cosa.
        manifesto = copy.deepcopy(MANIFESTO_SANO)
        manifesto["protocol_version"] = 1
        self.assertTrue(any("protocol_version" in e for e in esito(manifesto)))

    def test_dichiararsi_congelato_e_rosso(self):
        manifesto = copy.deepcopy(MANIFESTO_SANO)
        manifesto["status"] = "frozen_for_1_0"
        self.assertTrue(any("status" in e for e in esito(manifesto)))



class LoStatoDelManifesto(unittest.TestCase):
    """`ratificato` e' un'affermazione, e ha delle condizioni.

    Prima di questa tranche `status` non lo guardava nessuno: era una parola che
    chiunque poteva riscrivere, e «ratificato» avrebbe voluto dire quel che
    voleva dire chi l'aveva scritta.
    """

    def ratificato(self, **modifiche):
        """Un manifesto sano che si dichiara ratificato, e regge."""
        manifesto = copy.deepcopy(MANIFESTO_SANO)
        manifesto["status"] = "ratificato"
        manifesto["stato_del_manifesto"] = {
            "vocabolario": list(gate.STATI),
            "condizioni": ["una condizione scritta"],
            "cosa_non_afferma": "l'accettazione esterna",
        }
        manifesto["busta_di_bootstrap"] = {"schema_esatto": [".status", ".version"]}
        manifesto["envelopes"] = {"read": {"struttura": {".status": {}}}}
        manifesto["busta_degli_errori"] = {"struttura": {".status": {}}}
        manifesto.update(modifiche)
        return manifesto

    # --- le due controprove positive --------------------------------------

    def test_una_ratifica_completa_e_verde(self):
        """Senza, «sempre rosso» sarebbe una difesa."""
        self.assertEqual(gate.stato_del_manifesto(self.ratificato()), [])

    def test_in_qualifica_non_pretende_niente_di_tutto_questo(self):
        """Le condizioni sono della ratifica, non del manifesto in generale.

        Un manifesto in qualifica **puo'** non avere ancora la struttura delle
        buste: e' la condizione da cui la ratifica lo fa uscire, e pretenderla
        prima renderebbe impossibile lo stato che descrive il lavoro in corso.
        """
        self.assertEqual(gate.stato_del_manifesto(MANIFESTO_SANO), [])

    # --- i modi di dichiararsi ratificato senza esserlo --------------------

    def test_uno_stato_fuori_vocabolario_e_rosso(self):
        manifesto = copy.deepcopy(MANIFESTO_SANO)
        manifesto["status"] = "quasi-ratificato"
        errori = gate.stato_del_manifesto(manifesto)
        self.assertTrue(any("fuori da" in e for e in errori), errori)

    def test_congelato_resta_il_rosso_che_era(self):
        """`frozen_for_1_0` ha gia' la propria ragione, e non dev'essere
        assorbito nel rosso generico del vocabolario: e' l'errore che si fa
        copiando l'intestazione del v1, e il messaggio deve dirlo."""
        manifesto = copy.deepcopy(MANIFESTO_SANO)
        manifesto["status"] = "frozen_for_1_0"
        self.assertEqual(gate.stato_del_manifesto(manifesto), [])
        self.assertTrue(any("congelato" in e for e in esito(manifesto)))

    def test_una_ratifica_senza_condizioni_scritte_e_rossa(self):
        """Una ratifica senza condizioni non si puo' revocare: nessuno sa che
        cosa dovrebbe venire meno."""
        for vuoto in ({}, {"condizioni": []}):
            with self.subTest(caso=vuoto):
                dichiarazione = {
                    "vocabolario": list(gate.STATI),
                    "condizioni": ["c"],
                    "cosa_non_afferma": "x",
                }
                dichiarazione.update(vuoto)
                errori = gate.stato_del_manifesto(
                    self.ratificato(stato_del_manifesto=dichiarazione)
                )
                if vuoto:
                    self.assertTrue(any("condizioni" in e for e in errori), errori)

    def test_una_ratifica_senza_la_dichiarazione_e_rossa(self):
        manifesto = self.ratificato()
        del manifesto["stato_del_manifesto"]
        errori = gate.stato_del_manifesto(manifesto)
        self.assertTrue(any("stato_del_manifesto" in e for e in errori), errori)

    def test_una_ratifica_che_tace_sull_accettazione_esterna_e_rossa(self):
        """Il caso che conta piu' degli altri.

        Un documento che si dicesse ratificato senza quella riga si leggerebbe
        come approvato da qualcuno, e nessuno lo ha approvato: il bloccante
        cross-component e' aperto e ha un owner fuori da questo repository.
        """
        for vuoto in ("", "   "):
            with self.subTest(valore=vuoto):
                errori = gate.stato_del_manifesto(
                    self.ratificato(
                        stato_del_manifesto={
                            "vocabolario": list(gate.STATI),
                            "condizioni": ["c"],
                            "cosa_non_afferma": vuoto,
                        }
                    )
                )
                self.assertTrue(any("esterna" in e for e in errori), errori)

    def test_un_vocabolario_diverso_da_quello_del_gate_e_rosso(self):
        """Il manifesto non puo' inventare uno stato che il gate non conosce:
        lo dichiarerebbe legittimo senza che nessuno ne verifichi le
        condizioni."""
        errori = gate.stato_del_manifesto(
            self.ratificato(
                stato_del_manifesto={
                    "vocabolario": ["in_qualifica", "ratificato", "provvisorio"],
                    "condizioni": ["c"],
                    "cosa_non_afferma": "x",
                }
            )
        )
        self.assertTrue(any("vocabolario" in e for e in errori), errori)

    def test_una_ratifica_senza_busta_di_bootstrap_e_rossa(self):
        """`--version` esce su stdout come le altre, e una busta non censita e'
        cio' che la ratifica esiste per escludere."""
        manifesto = self.ratificato()
        del manifesto["busta_di_bootstrap"]
        errori = gate.stato_del_manifesto(manifesto)
        self.assertTrue(any("bootstrap" in e for e in errori), errori)

    def test_una_busta_senza_struttura_impedisce_la_ratifica(self):
        errori = gate.stato_del_manifesto(
            self.ratificato(envelopes={"read": {"required_top_level": ["status"]}})
        )
        self.assertTrue(any("read" in e and "struttura" in e for e in errori), errori)

    def test_la_busta_degli_errori_senza_struttura_impedisce_la_ratifica(self):
        errori = gate.stato_del_manifesto(
            self.ratificato(busta_degli_errori={"contract": "plenora-io-error-v1"})
        )
        self.assertTrue(any("errore" in e for e in errori), errori)

    # --- e il manifesto vero ----------------------------------------------

    def test_il_manifesto_reale_regge_la_propria_ratifica(self):
        self.assertEqual(gate.stato_del_manifesto(gate.contratto()), [])


class IlRegistroDelleCategorie(unittest.TestCase):
    def test_un_tetto_divergente_dalla_costante_e_rosso(self):
        registro = {"limite_di_lunghezza_byte": NOTE_SANE["MAX_BYTE_ID_CATEGORIA"] + 1}
        errori = esito(registro=registro)
        self.assertTrue(any("MAX_BYTE_ID_CATEGORIA" in e for e in errori), errori)

    def test_un_tetto_assente_e_rosso(self):
        self.assertTrue(any("registro" in e for e in esito(registro={})))

    def test_il_registro_reale_coincide_con_la_costante(self):
        # La catena che rende una sola l'autorita': il registro e' confrontato
        # qui con la costante Rust, e `check_categorie_di_perdita.py` confronta
        # con il registro la propria. Se questo legame si rompesse, quel gate
        # resterebbe verde mentre applica un tetto diverso dal codice.
        self.assertEqual(
            gate.registro_categorie()["limite_di_lunghezza_byte"],
            gate.costanti()["MAX_BYTE_ID_CATEGORIA"],
        )


class LeSondeNominateDalContratto(unittest.TestCase):
    def test_una_sonda_dichiarata_e_inesistente_e_rossa(self):
        manifesto = copy.deepcopy(MANIFESTO_SANO)
        manifesto["sonde_che_lo_provano"] = sorted(SONDE_SANE | {"una_sonda_mai_scritta"})
        errori = esito(manifesto)
        self.assertTrue(any("una_sonda_mai_scritta" in e for e in errori), errori)

    def test_una_sonda_esistente_e_non_nominata_e_rossa(self):
        errori = esito(esistenti=SONDE_SANE | {"una_sonda_non_nominata"})
        self.assertTrue(any("una_sonda_non_nominata" in e for e in errori), errori)

    def test_un_nome_ripetuto_e_rosso(self):
        manifesto = copy.deepcopy(MANIFESTO_SANO)
        manifesto["sonde_che_lo_provano"] = sorted(SONDE_SANE) + ["una_sonda"]
        self.assertTrue(any("due volte" in e for e in esito(manifesto)))

    def test_un_elenco_assente_o_malformato_e_rosso(self):
        for valore in (None, "una_sonda", ["una_sonda", 7], {}):
            manifesto = copy.deepcopy(MANIFESTO_SANO)
            manifesto["sonde_che_lo_provano"] = valore
            self.assertTrue(any("sonde_che_lo_provano" in e for e in esito(manifesto)))

    def test_gli_aiutanti_del_modulo_di_prova_non_sono_sonde(self):
        # `rapporto_con` costruisce il rapporto che le sonde usano e non prova
        # niente: se finisse fra le sonde, il contratto dovrebbe nominarlo.
        self.assertNotIn("rapporto_con", gate.sonde())
        self.assertIn("il_caso_peggiore_dichiarato_entra_nei_dodici_kib", gate.sonde())


class UnaCostanteDefinitaDueVolte(unittest.TestCase):
    """Il falso verde che la sovrascrittura silenziosa produceva.

    Non e' un caso teorico: prima di questa tranche `MAX_BYTE_DETTAGLIO` stava
    in `busta.rs` e la porta che lo applica sta in `loss.rs`. Se una copia
    sopravvivesse allo spostamento, il gate confronterebbe il manifesto con una
    sola delle due e sarebbe verde mentre il codice ne applica un'altra.
    """

    UNO = ("crates/uno.rs", "pub const MAX_CATEGORIE: usize = 64;\n")
    ALTRO = ("crates/altro.rs", "pub const MAX_CATEGORIE: usize = 99;\n")

    def test_due_file_che_la_dichiarano_sollevano(self):
        with self.assertRaises(ValueError) as contesto:
            gate.costanti_dai_testi([self.UNO, self.ALTRO])
        messaggio = str(contesto.exception)
        self.assertIn("MAX_CATEGORIE", messaggio)
        self.assertIn("crates/uno.rs", messaggio)
        self.assertIn("crates/altro.rs", messaggio)

    def test_vale_anche_dentro_lo_stesso_file(self):
        doppia = ("crates/uno.rs", self.UNO[1] + self.ALTRO[1])
        with self.assertRaises(ValueError):
            gate.costanti_dai_testi([doppia])

    def test_lo_stesso_valore_ripetuto_non_e_una_scusante(self):
        # Due definizioni concordi oggi sono due definizioni che domani
        # divergono: il gate rifiuta la doppiezza, non il disaccordo.
        gemella = ("crates/altro.rs", self.UNO[1])
        with self.assertRaises(ValueError):
            gate.costanti_dai_testi([self.UNO, gemella])

    def test_il_gate_lo_riporta_invece_di_esplodere(self):
        errori = gate.verifica(
            copy.deepcopy(MANIFESTO_SANO), None, set(SONDE_SANE), dict(REGISTRO_SANO)
        )
        self.assertIsInstance(errori, list)

    def test_i_sorgenti_reali_non_ne_hanno(self):
        gate.costanti()  # non solleva: nessuna costante del budget e' doppia


class IlPayloadDelleStringheTrattenute(unittest.TestCase):
    def test_un_payload_che_non_deriva_dai_limiti_e_rosso(self):
        for chiave in (
            "payload_stringhe_v2_trattenute_ragioni",
            "payload_stringhe_v2_trattenute_esempi",
        ):
            manifesto = copy.deepcopy(MANIFESTO_SANO)
            manifesto["limiti_della_diagnostica"][chiave] += 1
            self.assertTrue(any(chiave in e for e in esito(manifesto)), chiave)

    def test_un_payload_assente_e_rosso(self):
        for chiave in (
            "payload_stringhe_v2_trattenute_ragioni",
            "payload_stringhe_v2_trattenute_esempi",
        ):
            manifesto = copy.deepcopy(MANIFESTO_SANO)
            del manifesto["limiti_della_diagnostica"][chiave]
            self.assertTrue(any(chiave in e for e in esito(manifesto)), chiave)

    def test_alzare_un_tetto_senza_rifare_il_payload_e_rosso(self):
        # Il caso che conta: il payload e' **derivato**, quindi un tetto che
        # cambia e un payload che resta indietro devono divergere subito.
        manifesto = copy.deepcopy(MANIFESTO_SANO)
        manifesto["limiti_della_diagnostica"]["ragioni_trattenute"] *= 2
        note = dict(NOTE_SANE)
        note["MAX_RAGIONI_TRATTENUTE"] *= 2
        errori = esito(manifesto, note=note)
        self.assertTrue(
            any("payload_stringhe_v2_trattenute_ragioni" in e for e in errori), errori
        )


class LeSondeDellaRedazione(unittest.TestCase):
    def test_una_dichiarata_e_inesistente_e_rossa(self):
        manifesto = copy.deepcopy(MANIFESTO_SANO)
        manifesto["sonde_della_redazione"] = ["una_sonda_della_redazione_mai_scritta"]
        errori = esito(manifesto)
        self.assertTrue(
            any("una_sonda_della_redazione_mai_scritta" in e for e in errori), errori
        )

    def test_un_elenco_assente_o_malformato_e_rosso(self):
        for valore in (None, "una_sonda", ["una_sonda", 7], {}):
            manifesto = copy.deepcopy(MANIFESTO_SANO)
            manifesto["sonde_della_redazione"] = valore
            self.assertTrue(
                any("sonde_della_redazione" in e for e in esito(manifesto)),
                f"{valore!r} doveva essere rosso",
            )

    def test_quelle_reali_esistono_davvero(self):
        # Il verso che il gate pretende, verificato sul repository: se una
        # sparisse, la redazione resterebbe dichiarata e non piu' provata.
        nel_driver = set(gate.SONDA.findall(gate.DRIVER.read_text(encoding="utf-8")))
        for nome in gate.contratto()["sonde_della_redazione"]:
            self.assertIn(nome, nel_driver)


class IlValoreDiUnaCostante(unittest.TestCase):
    def test_risolve_i_prodotti_e_le_somme_di_nomi_noti(self):
        self.assertEqual(gate.valore("12 * 1024", {}), 12288)
        note = {"SEZIONI": 5, "BYTE_PER_SEZIONE": 12288, "BYTE_DELLA_STRUTTURA": 4096}
        self.assertEqual(
            gate.valore("SEZIONI * BYTE_PER_SEZIONE + BYTE_DELLA_STRUTTURA", note), 65536
        )

    def test_un_nome_non_ancora_noto_solleva(self):
        with self.assertRaises(ValueError):
            gate.valore("QUALCOSA + 1", {})

    def test_non_esegue_cio_che_trova(self):
        # La ragione per cui qui non c'e' `eval`: il gate legge un file che
        # qualcuno modifica, e un valutatore generico eseguirebbe qualunque
        # cosa vi trovasse. Rifiutare non e' prudenza, e' il requisito.
        for ostile in (
            '__import__("os").system("echo")',
            "open('/etc/passwd').read()",
            "1 << 64",
            "-1",
        ):
            with self.assertRaises((ValueError, SyntaxError), msg=ostile):
                gate.valore(ostile, {})


if __name__ == "__main__":
    unittest.main()


class IlComportamentoSiRicavaDalCodice(unittest.TestCase):
    """Il gate **legge** i due sorgenti, e non crede a una seconda copia.

    E' la differenza fra questa tranche e ciò che c'era prima: le tre clausole
    erano prosa, e una prosa che descriva un codice cambiato sotto di lei resta
    verde per sempre. Ogni sonda qui sotto cambia il **codice** e pretende che
    quello che il gate ricava cambi con lui.
    """

    def test_il_sorgente_finto_sano_da_le_tre_clausole(self):
        ordine, errori = gate.ordine_canonico_dal_codice(loss_finto())
        self.assertEqual(errori, [])
        self.assertEqual(ordine, COMPORTAMENTO_SANO["ordine_canonico"])
        identita, errori = gate.identita_delle_respinte_dal_codice(loss_finto())
        self.assertEqual(errori, [])
        self.assertEqual(identita, COMPORTAMENTO_SANO["identita_delle_respinte"])
        fonti, errori = gate.fonti_di_omesse_per_byte_dal_codice(
            busta_finta(), loss_finto()
        )
        self.assertEqual(errori, [])
        self.assertEqual(fonti, COMPORTAMENTO_SANO["fonti_di_omesse_per_byte"])

    def test_il_gate_ricava_le_tre_clausole_dai_sorgenti_veri(self):
        """Il caso sano sul repository, che e' cio' che il manifesto dichiara."""
        derivato = gate.comportamento_dal_codice()
        self.assertEqual(derivato["errori"], [])
        self.assertEqual(derivato["ordine_canonico"], COMPORTAMENTO_SANO["ordine_canonico"])
        self.assertEqual(
            derivato["identita_delle_respinte"],
            COMPORTAMENTO_SANO["identita_delle_respinte"],
        )
        self.assertEqual(
            derivato["fonti_di_omesse_per_byte"],
            COMPORTAMENTO_SANO["fonti_di_omesse_per_byte"],
        )

    def test_riordinare_la_chiave_cambia_l_ordine_ricavato(self):
        """Il campo del manifesto seguirebbe il codice, e il confronto sarebbe
        rosso: e' la proprieta' per cui la clausola smette di essere prosa."""
        ordine, errori = gate.ordine_canonico_dal_codice(
            loss_finto(campi_esempi="self.posizione, &self.category, &self.context")
        )
        self.assertEqual(errori, [])
        self.assertEqual(ordine["esempi"], ["posizione", "category", "context"])

    def test_un_campo_tolto_dalla_chiave_si_vede(self):
        ordine, errori = gate.ordine_canonico_dal_codice(
            loss_finto(campi_ragioni="self.code, &self.detail")
        )
        self.assertEqual(errori, [])
        self.assertEqual(ordine["ragioni"], ["code", "detail"])

    def test_un_derive_di_ord_rimesso_sulla_struttura_e_rosso(self):
        """Con il derive l'ordine torna a essere quello di dichiarazione dei
        campi, e `chiave()` resta una funzione vera che non decide piu'."""
        for derivato in ("Clone, Debug, PartialOrd, Ord, Serialize", "Clone, Eq, Serialize"):
            with self.subTest(derivato=derivato):
                _, errori = gate.ordine_canonico_dal_codice(
                    loss_finto(derive_esempi=derivato)
                )
                self.assertTrue(any("deriva" in e for e in errori), errori)

    def test_un_ord_che_non_passa_da_chiave_e_rosso(self):
        _, errori = gate.ordine_canonico_dal_codice(
            loss_finto(corpo_ord_esempi="self.category.cmp(&altro.category)")
        )
        self.assertTrue(any("atteso esattamente" in e for e in errori), errori)

    def test_un_ordine_invertito_e_rosso(self):
        """Passava: nomina `chiave()`, e taglia dalla parte opposta.

        E' il primo dei tre modi in cui cercare la sottostringa lasciava verde
        un ordine che il manifesto non descrive. Un `cmp` invertito non e' un
        ordine diverso in astratto: e' la sezione che tiene le voci **maggiori**
        dove il contratto ne promette le minori.
        """
        _, errori = gate.ordine_canonico_dal_codice(
            loss_finto(corpo_ord_esempi="altro.chiave().cmp(&self.chiave())")
        )
        self.assertTrue(any("atteso esattamente" in e for e in errori), errori)

    def test_un_criterio_in_piu_dopo_la_chiave_e_rosso(self):
        """Il secondo modo: la chiave c'e', e non decide da sola."""
        _, errori = gate.ordine_canonico_dal_codice(
            loss_finto(
                corpo_ord_esempi=(
                    "self.chiave().cmp(&altro.chiave())"
                    ".then(self.context.cmp(&altro.context))"
                )
            )
        )
        self.assertTrue(any("atteso esattamente" in e for e in errori), errori)

    def test_una_menzione_di_chiave_in_un_commento_non_e_delega(self):
        """Il terzo modo, e il piu' facile da scrivere per sbaglio: la
        sottostringa stava in un commento e il corpo confrontava tutt'altro."""
        _, errori = gate.ordine_canonico_dal_codice(
            loss_finto(
                corpo_eq_esempi=(
                    "// passa da chiave(), come l'altro\n"
                    "        self.category == altro.category"
                )
            )
        )
        self.assertTrue(any("atteso esattamente" in e for e in errori), errori)

    def test_un_corpo_canonico_dentro_un_commento_a_blocco_non_e_delega(self):
        """Il quarto modo, e non lo chiudeva la rimozione dei soli `//`.

        L'espressione regolare trovava il **primo** `fn cmp`, che sta nel
        commento ed e' canonico, e il corpo vero non veniva mai guardato: il
        gate restava verde su un ordine che il manifesto non descrive. I
        commenti vanno tolti **prima di cercare**, non prima di confrontare.
        """
        avvelenato = loss_finto().replace(
            "impl Ord for LossExample {\n"
            "    fn cmp(&self, altro: &Self) -> Ordering {\n"
            "        self.chiave().cmp(&altro.chiave())\n"
            "    }\n"
            "}",
            "impl Ord for LossExample {\n"
            "    /*\n"
            "    fn cmp(&self, altro: &Self) -> Ordering {\n"
            "        self.chiave().cmp(&altro.chiave())\n"
            "    }\n"
            "    */\n"
            "    fn cmp(&self, altro: &Self) -> Ordering {\n"
            "        self.category.cmp(&altro.category)\n"
            "    }\n"
            "}",
        )
        self.assertIn("/*", avvelenato, "la fixture non e' stata avvelenata")
        _, errori = gate.ordine_canonico_dal_codice(avvelenato)
        self.assertTrue(any("atteso esattamente" in e for e in errori), errori)

    def test_una_chiave_dentro_un_commento_a_blocco_non_conta(self):
        """La stessa classe dal lato della chiave: se il `chiave()` vero e'
        commentato, l'ordine non si legge piu' da nessuna parte, e il gate deve
        dirlo invece di leggere quello nel commento."""
        commentata = loss_finto().replace(
            "    fn chiave(&self) -> (&str, Posizione, &str) {\n"
            "        (&self.category, self.posizione, &self.context)\n"
            "    }\n",
            "    /*\n"
            "    fn chiave(&self) -> (&str, Posizione, &str) {\n"
            "        (&self.category, self.posizione, &self.context)\n"
            "    }\n"
            "    */\n",
        )
        _, errori = gate.ordine_canonico_dal_codice(commentata)
        self.assertTrue(any("non esiste piu'" in e for e in errori), errori)

    def test_un_corpo_canonico_dentro_una_stringa_non_e_delega(self):
        """La stessa classe del commento a blocco, con una stringa al posto suo.

        Del testo che *sembra* codice e non lo e': l'espressione regolare
        trovava il `fn cmp` dentro la stringa, che e' canonico, e il corpo vero
        non veniva mai guardato. Le stringhe si mascherano prima di cercare,
        come i commenti, e per la stessa ragione.
        """
        avvelenato = loss_finto().replace(
            "impl Ord for LossExample {\n"
            "    fn cmp(&self, altro: &Self) -> Ordering {\n"
            "        self.chiave().cmp(&altro.chiave())\n"
            "    }\n"
            "}",
            "impl Ord for LossExample {\n"
            '    const AIUTO: &str = "\n'
            "    fn cmp(&self, altro: &Self) -> Ordering {\n"
            "        self.chiave().cmp(&altro.chiave())\n"
            "    }\n"
            '";\n'
            "    fn cmp(&self, altro: &Self) -> Ordering {\n"
            "        self.category.cmp(&altro.category)\n"
            "    }\n"
            "}",
        )
        self.assertIn("AIUTO", avvelenato, "la fixture non e' stata avvelenata")
        _, errori = gate.ordine_canonico_dal_codice(avvelenato)
        self.assertTrue(any("atteso esattamente" in e for e in errori), errori)

    def test_una_chiave_dentro_una_stringa_non_conta(self):
        """Se il `chiave()` vero e' sostituito da uno dentro una stringa,
        l'ordine non si legge piu' da nessuna parte, e il gate deve dirlo."""
        finta = loss_finto().replace(
            "    fn chiave(&self) -> (&str, Posizione, &str) {\n"
            "        (&self.category, self.posizione, &self.context)\n"
            "    }\n",
            '    const AIUTO: &str = "\n'
            "    fn chiave(&self) -> (&str, Posizione, &str) {\n"
            "        (&self.category, self.posizione, &self.context)\n"
            "    }\n"
            '";\n',
        )
        _, errori = gate.ordine_canonico_dal_codice(finta)
        self.assertTrue(any("non esiste piu'" in e for e in errori), errori)

    def test_una_raw_string_non_fabbrica_codice(self):
        """Uno scanner che non conosce `r#"..."#` sbaglia in **due** modi
        insieme, e quale prevalga lo decide la parita' delle virgolette.

        `r#""` gli sembra una stringa aperta e subito chiusa, quindi espone come
        codice cio' che segue; il terminatore `"#` gli sembra una stringa nuova,
        quindi maschera il codice vero che viene dopo. Niente di tutto questo ha
        a che fare con il codice.
        """
        vero = (
            "    fn chiave(&self) -> (&str, Posizione, &str) {\n"
            "        (&self.category, self.posizione, &self.context)\n"
            "    }\n"
        )
        avvelenato = loss_finto().replace(
            vero,
            '    const AIUTO: &str = r#""\n'
            "    fn chiave(&self) -> (&str, Posizione, &str) {\n"
            "        (self.posizione, &self.context)\n"
            "    }\n"
            '    "#;\n' + vero,
        )
        self.assertIn('r#"', avvelenato, "la fixture non e' stata avvelenata")
        ordine, errori = gate.ordine_canonico_dal_codice(avvelenato)
        self.assertEqual(errori, [])
        self.assertEqual(ordine["esempi"], ["category", "posizione", "context"])

    def test_le_forme_di_letterale_conosciute_non_disturbano(self):
        """Byte string, raw con piu' cancelletti, byte-raw, e un carattere che
        contiene una virgoletta -- `'\"'` esiste, e aprirebbe una stringa che
        non c'e'."""
        vero = (
            "    fn chiave(&self) -> (&str, Posizione, &str) {\n"
            "        (&self.category, self.posizione, &self.context)\n"
            "    }\n"
        )
        letterali = (
            'b"una \\" dentro"',
            'r##"una "# dentro"##',
            'br#"byte "grezza""#',
            "'\"'",
        )
        for letterale in letterali:
            with self.subTest(letterale=letterale):
                fixture = loss_finto().replace(
                    vero, f"    const AIUTO: &str = {letterale};\n" + vero
                )
                ordine, errori = gate.ordine_canonico_dal_codice(fixture)
                self.assertEqual(errori, [], letterale)
                self.assertEqual(ordine["esempi"], ["category", "posizione", "context"])

    def test_una_forma_di_letterale_sconosciuta_fallisce_chiusa(self):
        """Una forma che lo scanner non sa leggere non la sa mascherare, e cio'
        che resterebbe visibile e' testo arbitrario: meglio dirlo."""
        _, errori = gate.ordine_canonico_dal_codice(
            'fn f() { let x = c"stringa C"; }\n'
        )
        self.assertTrue(any("non riconosciuta" in e for e in errori), errori)

    def test_un_letterale_o_un_commento_non_chiuso_fallisce_chiuso(self):
        casi = {
            'fn f() { let x = "mai chiusa;\n}\n': "stringa non terminato",
            'fn f() { let x = r#"mai chiusa;\n}\n': "grezzo `r#\"` non terminato",
            "fn f() { /* mai chiuso\n}\n": "commento a blocco non chiuso",
        }
        for sorgente, atteso in casi.items():
            with self.subTest(atteso=atteso):
                chiavi, errori = gate.ordine_canonico_dal_codice(sorgente)
                self.assertEqual(chiavi, {}, "su un sorgente illeggibile non si deriva")
                self.assertTrue(any(atteso in e for e in errori), errori)

    def test_un_sorgente_illeggibile_ferma_tutti_e_tre_i_derivatori(self):
        """Non solo l'ordine: un sorgente che non si sa leggere non produce ne'
        identita' delle respinte ne' fonti del contatore."""
        rotto = 'fn f() { let x = c"stringa C"; }\n'
        identita, errori = gate.identita_delle_respinte_dal_codice(rotto)
        self.assertEqual(identita, {})
        self.assertTrue(errori)
        fonti, errori = gate.fonti_di_omesse_per_byte_dal_codice(rotto, loss_finto())
        self.assertEqual(fonti, [])
        self.assertTrue(errori)

    def test_un_sito_nominato_in_una_stringa_non_e_un_sito(self):
        """Il verso opposto, e prima era un rosso vero: una stringa che **nomina**
        il contatore -- una nota, un messaggio -- non e' un uso del contatore."""
        fonti, errori = gate.fonti_di_omesse_per_byte_dal_codice(
            busta_finta(
                '    let nota = "troncamento.omesse_per_byte = nuova_fonte;";\n'
            ),
            loss_finto(),
        )
        self.assertEqual(errori, [])
        self.assertEqual(fonti, COMPORTAMENTO_SANO["fonti_di_omesse_per_byte"])

    def test_un_commento_a_blocco_dentro_la_forma_canonica_resta_verde(self):
        """Come per i `//`: togliere i commenti non deve diventare un divieto
        di spiegarsi dentro una funzione."""
        ordine, errori = gate.ordine_canonico_dal_codice(
            loss_finto(
                corpo_ord_esempi=(
                    "/* la chiave decide, e nient'altro */\n"
                    "        self.chiave().cmp(&altro.chiave())"
                )
            )
        )
        self.assertEqual(errori, [])
        self.assertEqual(ordine["esempi"], ["category", "posizione", "context"])

    def test_un_commento_dentro_la_forma_canonica_resta_verde(self):
        """La forma esatta non deve diventare un divieto di spiegarsi: il
        confronto e' sul codice, e i commenti si tolgono prima."""
        ordine, errori = gate.ordine_canonico_dal_codice(
            loss_finto(
                corpo_eq_esempi=(
                    "// la chiave e' l'identita' che il v2 vede\n"
                    "        self.chiave() == altro.chiave()"
                )
            )
        )
        self.assertEqual(errori, [])
        self.assertEqual(ordine["esempi"], ["category", "posizione", "context"])

    def test_un_partial_cmp_che_non_delega_a_cmp_e_rosso(self):
        """`<` e `>` passano da qui, e le collezioni da `cmp`: se i due
        divergono, gli operatori danno una relazione diversa da quella con cui
        la sezione taglia."""
        _, errori = gate.ordine_canonico_dal_codice(
            loss_finto(
                corpo_partial_cmp_esempi="self.category.partial_cmp(&altro.category)"
            )
        )
        self.assertTrue(any("partial_cmp" in e for e in errori), errori)

    def test_un_impl_delegato_che_sparisce_e_rosso(self):
        for tratto in sorted(gate.FORME_DELEGATE):
            with self.subTest(tratto=tratto):
                senza = loss_finto().replace(
                    f"impl {tratto} for LossExample", f"impl {tratto} for Altrove", 1
                )
                _, errori = gate.ordine_canonico_dal_codice(senza)
                self.assertTrue(any(tratto in e for e in errori), errori)

    def test_una_chiave_sparita_e_rossa(self):
        senza = loss_finto().replace("fn chiave(&self) -> (&str, Posizione, &str)", "fn nulla(&self)")
        _, errori = gate.ordine_canonico_dal_codice(senza)
        self.assertTrue(any("non esiste piu'" in e for e in errori), errori)

    def test_due_chiavi_nello_stesso_tipo_sono_ambigue(self):
        in_piu = (
            "\n    fn chiave(&self) -> (&str, &str) {\n"
            "        (&self.category, &self.context)\n"
            "    }\n"
        )
        _, errori = gate.ordine_canonico_dal_codice(loss_finto(chiave_in_piu=in_piu))
        self.assertTrue(any("ambiguo" in e for e in errori), errori)

    def test_una_chiave_che_ripete_un_campo_e_rossa(self):
        _, errori = gate.ordine_canonico_dal_codice(
            loss_finto(campi_esempi="&self.category, &self.category, &self.context")
        )
        self.assertTrue(any("piu' di una volta" in e for e in errori), errori)

    def test_cambiare_l_identita_delle_respinte_si_vede(self):
        identita, errori = gate.identita_delle_respinte_dal_codice(
            loss_finto(elemento_respinti="(FidelityReasonCode, Posizione)")
        )
        self.assertEqual(errori, [])
        self.assertEqual(identita["esempi"], ["code", "posizione"])

    def test_un_tipo_che_il_gate_non_sa_nominare_e_rosso(self):
        _, errori = gate.identita_delle_respinte_dal_codice(
            loss_finto(elemento_respinte="(FidelityReasonCode, Posizione, String)")
        )
        self.assertTrue(any("non sa nominare" in e for e in errori), errori)

    def test_un_componente_ripetuto_nella_chiave_delle_respinte_e_rosso(self):
        _, errori = gate.identita_delle_respinte_dal_codice(
            loss_finto(elemento_respinte="(Posizione, Posizione)")
        )
        self.assertTrue(any("due volte lo stesso campo" in e for e in errori), errori)

    def test_un_insieme_che_non_e_piu_un_btreeset_e_rosso(self):
        senza = loss_finto().replace("respinti: BTreeSet<Posizione>,", "respinti: Vec<Posizione>,")
        _, errori = gate.identita_delle_respinte_dal_codice(senza)
        self.assertTrue(any("non e' piu' un `BTreeSet`" in e for e in errori), errori)

    def test_un_sito_del_contatore_che_non_si_classifica_e_rosso(self):
        """Una terza fonte comparsa senza dichiarazione: e' il modo normale in
        cui una clausola invecchia, ed e' cio' che il gate deve vedere."""
        _, errori = gate.fonti_di_omesse_per_byte_dal_codice(
            busta_finta(
                "    troncamento.omesse_per_byte = qualcosa_di_nuovo.len() as u64;\n"
            ),
            loss_finto(),
        )
        self.assertTrue(any("nessuna delle due fonti" in e for e in errori), errori)

    def test_una_scrittura_composta_e_censita(self):
        """Il secondo falso verde: il censimento cercava `=` e nient'altro.

        `troncamento.omesse_per_byte += nuova_fonte;` non veniva censita affatto,
        quindi il gate continuava a dichiarare **esattamente due** fonti mentre
        nel codice ce n'era una terza. Qui la scrittura composta e' vista, e la
        fonte nuova non si classifica: il gate diventa rosso invece di tacere.
        """
        _, errori = gate.fonti_di_omesse_per_byte_dal_codice(
            busta_finta(
                "    troncamento.omesse_per_byte += nuova_fonte.len() as u64;\n"
            ),
            loss_finto(),
        )
        self.assertTrue(any("nessuna delle due fonti" in e for e in errori), errori)

    def test_ogni_forma_di_scrittura_composta_e_censita(self):
        """Non solo `+=`: la classificazione e' per operatore, non per caso."""
        for operatore in gate.SCRITTURE_COMPOSTE:
            with self.subTest(operatore=operatore):
                _, errori = gate.fonti_di_omesse_per_byte_dal_codice(
                    busta_finta(
                        f"    troncamento.omesse_per_byte {operatore} ignota.len();\n"
                    ),
                    loss_finto(),
                )
                self.assertTrue(
                    any("nessuna delle due fonti" in e for e in errori), errori
                )

    def test_una_scrittura_composta_da_una_fonte_nota_si_classifica(self):
        """E la censisce **come sito**, non solo come errore: una `+=` da una
        fonte gia' dichiarata non deve inventare una terza fonte."""
        fonti, errori = gate.fonti_di_omesse_per_byte_dal_codice(
            busta_finta("    troncamento.omesse_per_byte += per_byte;\n"),
            loss_finto(),
        )
        self.assertEqual(errori, [])
        self.assertEqual(fonti, COMPORTAMENTO_SANO["fonti_di_omesse_per_byte"])

    def test_una_scrittura_per_auto_borrow_e_rossa(self):
        """`&mut` non compare, eppure il contatore viene scritto.

        `clone_from` prende `&mut self`, e Rust glielo passa da solo. Ammettere
        il **punto** come lettura ammetteva ogni metodo, mutante compreso: una
        chiamata si ammette per nome intero, non perche' comincia con un punto.
        """
        _, errori = gate.fonti_di_omesse_per_byte_dal_codice(
            busta_finta(
                "    troncamento.omesse_per_byte.clone_from(&nuova_fonte);\n"
            ),
            loss_finto(),
        )
        self.assertTrue(any("non sa classificare" in e for e in errori), errori)

    def test_solo_i_metodi_ammessi_passano_per_lettura(self):
        """Non e' `clone_from` a essere in lista nera: e' `.saturating_add(` a
        essere l'unica catena in lista bianca."""
        for metodo in ("set", "replace", "take", "add_assign", "clone_from"):
            with self.subTest(metodo=metodo):
                _, errori = gate.fonti_di_omesse_per_byte_dal_codice(
                    busta_finta(
                        f"    troncamento.omesse_per_byte.{metodo}(nuova_fonte);\n"
                    ),
                    loss_finto(),
                )
                self.assertTrue(any("non sa classificare" in e for e in errori), errori)

    def test_un_assegnazione_destrutturante_e_censita(self):
        """Il contatore e' scritto senza essere seguito da `=`.

        `(troncamento.omesse_per_byte,) = (nuova_fonte,);` e' un'assegnazione, e
        la virgola da sola la faceva passare per una lettura. Qui viene censita
        come sito, e la fonte nuova non si classifica: il gate e' rosso.
        """
        _, errori = gate.fonti_di_omesse_per_byte_dal_codice(
            busta_finta(
                "    (troncamento.omesse_per_byte,) = (nuova_fonte.len() as u64,);\n"
            ),
            loss_finto(),
        )
        self.assertTrue(any("nessuna delle due fonti" in e for e in errori), errori)

    def test_un_uguale_dentro_una_stringa_non_e_un_assegnazione(self):
        """Il verso opposto della sonda precedente, e un caso vero.

        `assert_eq!(troncamento.omesse_per_byte, atteso, "{n} caratteri = {} byte")`
        porta un `=` nel messaggio, e cercarlo sul testo grezzo faceva chiamare
        scrittura una sonda che legge il contatore. Ora il contenuto delle
        stringhe e' mascherato prima di ogni ricerca, non solo prima di questa.
        """
        fonti, errori = gate.fonti_di_omesse_per_byte_dal_codice(
            busta_finta(
                '    assert_eq!(troncamento.omesse_per_byte, atteso, '
                '"{n} caratteri = {} byte", quanti);\n'
            ),
            loss_finto(),
        )
        self.assertEqual(errori, [])
        self.assertEqual(fonti, COMPORTAMENTO_SANO["fonti_di_omesse_per_byte"])

    def test_una_macro_che_riceve_il_contatore_e_rossa(self):
        """Il segno non dice niente su chi lo riceve.

        `scrivi!(troncamento.omesse_per_byte, nuova_fonte)` e' un'assegnazione
        scritta con una virgola: una macro prende i **token**, e da fuori una
        lettura e una scrittura si somigliano. La virgola e la parentesi hanno
        percio' lasciato l'elenco dei segni ammessi, e dentro una chiamata si
        ammette la chiamata.
        """
        _, errori = gate.fonti_di_omesse_per_byte_dal_codice(
            busta_finta("    scrivi!(troncamento.omesse_per_byte, nuova_fonte);\n"),
            loss_finto(),
        )
        self.assertTrue(any("scrivi!" in e for e in errori), errori)

    def test_una_chiamata_sconosciuta_e_rossa(self):
        """Vale anche per le funzioni, e con la parentesi al posto della
        virgola: l'ammissione e' per nome, non per forma dell'argomento."""
        for invocazione in (
            "accumula(troncamento.omesse_per_byte);",
            "somma(altro, troncamento.omesse_per_byte);",
            "scrivi![troncamento.omesse_per_byte];",
        ):
            with self.subTest(invocazione=invocazione):
                _, errori = gate.fonti_di_omesse_per_byte_dal_codice(
                    busta_finta(f"    {invocazione}\n"), loss_finto()
                )
                self.assertNotEqual(errori, [], invocazione)

    def test_una_mutazione_dentro_un_asserzione_ammessa_e_rossa(self):
        """L'asserzione ammette che il contatore le sia **passato**, non che
        dentro di lei gli si faccia qualunque cosa.

            assert_eq!(troncamento.omesse_per_byte.clone_from(&nuova), ());

        `clone_from` muta il campo, `&mut` non compare, e l'asserzione e' fra
        quelle ammesse: ammettere l'occorrenza per la sola chiamata autorizzava
        tutta l'espressione racchiusa. Essere dentro una chiamata di sola
        lettura e' **necessario e non sufficiente**: serve anche che il
        contatore sia un argomento intero, o consumato come valore.
        """
        for mutazione in ("clone_from(&nuova_fonte)", "set(9)", "add_assign(1)"):
            with self.subTest(mutazione=mutazione):
                _, errori = gate.fonti_di_omesse_per_byte_dal_codice(
                    busta_finta(
                        "    assert_eq!(\n"
                        f"        troncamento.omesse_per_byte.{mutazione},\n"
                        "        ()\n"
                        "    );\n"
                    ),
                    loss_finto(),
                )
                self.assertTrue(
                    any("non sa classificare" in e for e in errori), errori
                )
                self.assertTrue(
                    any("non basta" in e for e in errori),
                    "il messaggio deve dire perche' l'asserzione non salva l'uso",
                )

    def test_le_chiamate_di_sola_lettura_restano_verdi(self):
        """Il verso opposto: le tre asserzioni prendono i due lati per
        riferimento condiviso, e sono cio' che le sonde della busta usano."""
        fonti, errori = gate.fonti_di_omesse_per_byte_dal_codice(
            busta_finta(
                '    assert_eq!(troncamento.omesse_per_byte, 0, "{n} = {} byte", q);\n'
                "    assert_ne!(troncamento.omesse_per_byte, 9);\n"
                "    assert!(troncamento.omesse_per_byte > 0);\n"
                "    let letto = troncamento.omesse_per_byte;\n"
            ),
            loss_finto(),
        )
        self.assertEqual(errori, [])
        self.assertEqual(fonti, COMPORTAMENTO_SANO["fonti_di_omesse_per_byte"])

    def test_una_presa_per_riferimento_mutabile_e_rossa(self):
        """La scrittura avvverrebbe altrove, e questo censimento non la segue."""
        _, errori = gate.fonti_di_omesse_per_byte_dal_codice(
            busta_finta("    accumula(&mut troncamento.omesse_per_byte);\n"),
            loss_finto(),
        )
        self.assertTrue(any("preso per riferimento" in e for e in errori), errori)

    def test_un_uso_che_il_gate_non_sa_classificare_e_rosso(self):
        """La seconda meta' della correzione: si fallisce **chiusi**.

        Distinguere lettura da scrittura senza un parser non si puo' fare per
        esaustione, quindi una forma che il gate non conosce e' rossa anche se
        innocua. Aggiungerla all'elenco delle letture ammesse e' una decisione
        che qualcuno deve prendere, ed e' precisamente cio' che deve costare a
        chi tocca il contatore.
        """
        _, errori = gate.fonti_di_omesse_per_byte_dal_codice(
            busta_finta("    let quante = troncamento.omesse_per_byte as u64;\n"),
            loss_finto(),
        )
        self.assertTrue(any("non sa classificare" in e for e in errori), errori)

    def test_le_letture_ammesse_non_sono_siti(self):
        """Un confronto o una lettura in una asserzione non e' una fonte: se lo
        fosse, ogni sonda che guarda il contatore ne inventerebbe una."""
        fonti, errori = gate.fonti_di_omesse_per_byte_dal_codice(
            busta_finta(
                "    assert_eq!(troncamento.omesse_per_byte, 0);\n"
                "    assert!(troncamento.omesse_per_byte > 0);\n"
                "    let letto = troncamento.omesse_per_byte;\n"
            ),
            loss_finto(),
        )
        self.assertEqual(errori, [])
        self.assertEqual(fonti, COMPORTAMENTO_SANO["fonti_di_omesse_per_byte"])

    def test_un_sito_ambiguo_e_rosso(self):
        _, errori = gate.fonti_di_omesse_per_byte_dal_codice(
            busta_finta(
                "    troncamento.omesse_per_byte = per_byte + fuori_misura.len() as u64;\n"
            ),
            loss_finto(),
        )
        self.assertTrue(any("entrambe le fonti" in e for e in errori), errori)

    def test_senza_il_budget_della_sezione_e_rosso(self):
        senza = busta_finta().replace("entro_il_budget", "una_funzione_qualunque")
        _, errori = gate.fonti_di_omesse_per_byte_dal_codice(senza, loss_finto())
        self.assertTrue(any("budget della sezione" in e for e in errori), errori)

    def test_senza_la_partizione_sui_byte_e_rosso(self):
        senza = busta_finta().replace("MAX_BYTE_ID_CATEGORIA", "UN_NUMERO")
        _, errori = gate.fonti_di_omesse_per_byte_dal_codice(senza, loss_finto())
        self.assertTrue(any("filtro alla porta" in e for e in errori), errori)

    def test_senza_siti_il_contatore_non_ha_fonti(self):
        _, errori = gate.fonti_di_omesse_per_byte_dal_codice(
            busta_finta().replace("troncamento.omesse_per_byte", "altro_contatore"),
            loss_finto(),
        )
        self.assertTrue(any("nessun sito incrementa" in e for e in errori), errori)

    def test_un_accessore_delle_respinte_sparito_e_rosso(self):
        senza = loss_finto().replace("pub fn respinti_per_misura", "fn quanti")
        _, errori = gate.fonti_di_omesse_per_byte_dal_codice(busta_finta(), senza)
        self.assertTrue(any("accessori" in e for e in errori), errori)


class IlManifestoDescriveIlComportamento(unittest.TestCase):
    """Il confronto: il campo del manifesto contro cio' che il codice compone."""

    def test_il_manifesto_finto_e_verde(self):
        self.assertEqual(esito(), [])

    def test_un_ordine_canonico_divergente_e_rosso(self):
        for clausola in ("ragioni", "esempi"):
            with self.subTest(clausola):
                manifesto = copy.deepcopy(MANIFESTO_SANO)
                manifesto["determinismo"]["ordine_canonico"][clausola] = ["posizione"]
                errori = esito(manifesto)
                self.assertTrue(
                    any("ordine_canonico" in e and clausola in e for e in errori), errori
                )

    def test_un_identita_delle_respinte_divergente_e_rossa(self):
        for clausola in ("ragioni", "esempi"):
            with self.subTest(clausola):
                manifesto = copy.deepcopy(MANIFESTO_SANO)
                manifesto["troncamento"]["identita_delle_respinte"][clausola] = ["detail"]
                errori = esito(manifesto)
                self.assertTrue(
                    any("identita_delle_respinte" in e and clausola in e for e in errori),
                    errori,
                )

    def test_le_fonti_divergenti_sono_rosse(self):
        manifesto = copy.deepcopy(MANIFESTO_SANO)
        manifesto["troncamento"]["omesse_per_byte"]["fonti"] = ["limite_della_voce"]
        errori = esito(manifesto)
        self.assertTrue(any("fonti" in e for e in errori), errori)

    def test_un_campo_ripetuto_e_rosso(self):
        """`["code", "code"]` e `["code"]` descrivono la stessa identita' e si
        leggono come due: un componente ripetuto non stringe niente."""
        casi = (
            (("determinismo", "ordine_canonico", "ragioni"), ["code", "code", "detail"]),
            (
                ("troncamento", "identita_delle_respinte", "esempi"),
                ["posizione", "posizione"],
            ),
            (
                ("troncamento", "omesse_per_byte", "fonti"),
                ["limite_della_voce", "limite_della_voce", "budget_della_sezione"],
            ),
        )
        for percorso, valore in casi:
            with self.subTest(percorso=percorso):
                manifesto = copy.deepcopy(MANIFESTO_SANO)
                dove = manifesto
                for pezzo in percorso[:-1]:
                    dove = dove[pezzo]
                dove[percorso[-1]] = valore
                errori = esito(manifesto)
                self.assertTrue(
                    any("non vuoti e distinti" in e for e in errori), errori
                )

    def test_una_clausola_che_torna_prosa_e_rossa(self):
        """E' il difetto di partenza: una frase al posto di un campo."""
        casi = (
            ("determinismo", "ordine_canonico"),
            ("troncamento", "identita_delle_respinte"),
            ("troncamento", "omesse_per_byte"),
        )
        for sezione, clausola in casi:
            with self.subTest(clausola=clausola):
                manifesto = copy.deepcopy(MANIFESTO_SANO)
                manifesto[sezione][clausola] = "una frase che descrive tutto"
                errori = esito(manifesto)
                self.assertTrue(any(clausola in e for e in errori), errori)

    def test_una_sezione_intera_assente_e_rossa(self):
        for sezione in ("determinismo", "troncamento"):
            with self.subTest(sezione):
                manifesto = copy.deepcopy(MANIFESTO_SANO)
                del manifesto[sezione]
                self.assertNotEqual(esito(manifesto), [])

    def test_un_campo_in_piu_che_nessuno_confronta_e_rosso(self):
        """Prosa con la forma di un campo: si legge come confrontata e non lo e'."""
        casi = (
            ("determinismo", "ordine_canonico"),
            ("troncamento", "identita_delle_respinte"),
            ("troncamento", "omesse_per_byte"),
        )
        for sezione, clausola in casi:
            with self.subTest(clausola=clausola):
                manifesto = copy.deepcopy(MANIFESTO_SANO)
                manifesto[sezione][clausola]["categorie"] = ["identificatore"]
                errori = esito(manifesto)
                self.assertTrue(any("non ricava da nessuna parte" in e for e in errori), errori)

    def test_la_prosa_accanto_ai_campi_resta_ammessa(self):
        """Il significato residuo va ancora scritto: e' il campo che si aggiunge
        alla frase, non la frase che si toglie."""
        manifesto = copy.deepcopy(MANIFESTO_SANO)
        for sezione, clausola in (
            ("troncamento", "identita_delle_respinte"),
            ("troncamento", "omesse_per_byte"),
        ):
            manifesto[sezione][clausola]["come_si_ricava"] = "da dove"
            manifesto[sezione][clausola]["nota"] = "perche'"
        self.assertEqual(esito(manifesto), [])

    def test_un_errore_di_derivazione_arriva_all_elenco(self):
        """Se il codice non si lascia leggere il gate e' rosso, e lo e' per la
        ragione giusta: non si confronta un manifesto con il vuoto."""
        errori = esito(comportamento={"errori": ["`LossExample::chiave()` non esiste piu'"]})
        self.assertTrue(any("chiave()" in e for e in errori), errori)
        self.assertFalse(any("non ricava da nessuna parte" in e for e in errori), errori)
