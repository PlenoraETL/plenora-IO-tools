"""I modelli delle due buste che questo ciclo copre: bootstrap e catalogo.

# I nomi sono quelli del wire

`hostile_input_hardened`, non `hardened_against_hostile_input`. Tradurre
costringerebbe chi legge `release/cli-protocol-v2.json` a tenere due
vocabolari, e la prima volta che i due divergono nessuno sa quale sia quello
giusto.

# Perche' `raw` c'e' sempre

Ogni modello conserva il documento da cui e' nato. Non e' pigrizia: un campo che
il protocollo aggiunge come `add_optional_field` -- che le sue regole di
compatibilita' consentono -- arriva a chi usa l'SDK anche prima che l'SDK lo
modelli. Senza, un consumatore dovrebbe aspettare una nostra release per leggere
un campo che il prodotto gli sta gia' mandando.

# Perche' i campi mancanti sono un errore e non `None`

`from_json` pretende i campi che il protocollo dichiara `sempre: true`. Un
modello che li riempisse di `None` trasformerebbe un'incompatibilita' di
versione -- «questo binario e' piu' vecchio dell'SDK» -- in dati sbagliati piu'
avanti, dove nessuno la riconosce piu'. I campi dichiarati `sempre: false` sono
opzionali qui, ed e' la stessa distinzione.

# Perche' `write_capabilities` non e' modellato in profondita'

Le sue foglie sono undici sottostrutture e un vocabolario chiuso per ciascuna.
Modellarle qui vorrebbe dire ratificare in questo ciclo una superficie che
serve a `convert`, che questo ciclo non copre: resterebbe scritta e non
esercitata, cioe' la promessa non onorata che il gate delle buste esiste per
impedire. Resta un dizionario, e il gate dei modelli sa che e' voluto.
"""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import Any

from .errors import ProtocolError


def _pretendi(documento: dict[str, Any], campi: tuple[str, ...], dove: str) -> None:
    mancanti = [campo for campo in campi if campo not in documento]
    if mancanti:
        raise ProtocolError(
            f"{dove}: mancano i campi {mancanti}, che il protocollo v2 dichiara "
            "sempre presenti. Il binario e' piu' vecchio dell'SDK, o il "
            "documento non e' quello che dice di essere."
        )


@dataclass(frozen=True)
class Version:
    """La busta di bootstrap: `plenora-io --version`.

    Non porta `contract` ne' `protocol_version`, e non e' una mancanza: si legge
    **prima** di sapere con quale protocollo si sta parlando. Il manifesto la
    dichiara con schema chiuso -- esattamente questi due campi -- proprio perche'
    chi la consuma non ha una versione su cui appoggiarsi per capire che cosa
    sia cambiato.
    """

    status: str
    version: str
    raw: dict[str, Any] = field(default_factory=dict, repr=False)

    @classmethod
    def from_json(cls, documento: dict[str, Any]) -> "Version":
        _pretendi(documento, ("status", "version"), "busta di bootstrap")
        # Lo schema e' **chiuso**: qui un campo in piu' non si ignora, si
        # nomina. E' la meta' che rende utile una busta letta prima della
        # negoziazione -- se cambia, chi la legge deve accorgersene subito
        # invece di scoprirlo quando il campo nuovo gli serviva.
        in_piu = sorted(set(documento) - {"status", "version"})
        if in_piu:
            raise ProtocolError(
                f"busta di bootstrap con i campi in piu' {in_piu}. Il suo schema "
                "e' chiuso: due campi, ne' uno di meno ne' uno di piu'."
            )
        return cls(status=documento["status"], version=documento["version"], raw=dict(documento))


@dataclass(frozen=True)
class Driver:
    """Un elemento di `catalog.drivers`: il descrittore, piu' due campi suoi.

    `available` e `required_feature` non stanno nel descrittore che `inspect`
    restituisce: sono del catalogo, e dicono se **questo** binario possa usare
    quel driver. Un descrittore descrive un formato; il catalogo descrive
    un'installazione.
    """

    id: str
    available: bool
    required_feature: str | None
    direction: str
    runtime: str
    fidelity_class: str
    crs_handling: str
    multi_layer: bool
    multi_file: bool
    hostile_input_hardened: bool
    spec_version_supported: str | None
    buffering: str
    descriptor_version: int
    driver_version: int
    semantic_version: int
    effective_delivery: str
    native_read_mode: str
    read_mode: str
    write_mode: str
    read_determinism: str
    write_determinism: str
    reader_concurrency: str
    projection_support: str
    predicate_pruning_support: str
    spatial_pruning_support: str
    format_options: list[dict[str, Any]]
    write_capabilities: dict[str, Any]
    raw: dict[str, Any] = field(default_factory=dict, repr=False)

    #: I campi che il protocollo dichiara `sempre: true` sotto `.drivers[]`.
    #: Il gate `scripts/check_sdk_python.py` confronta questa tupla con la
    #: struttura del manifesto, cosi' le due non divergono in silenzio.
    OBBLIGATORI = (
        "id",
        "available",
        "required_feature",
        "direction",
        "runtime",
        "fidelity_class",
        "crs_handling",
        "multi_layer",
        "multi_file",
        "hostile_input_hardened",
        "spec_version_supported",
        "buffering",
        "descriptor_version",
        "driver_version",
        "semantic_version",
        "effective_delivery",
        "native_read_mode",
        "read_mode",
        "write_mode",
        "read_determinism",
        "write_determinism",
        "reader_concurrency",
        "projection_support",
        "predicate_pruning_support",
        "spatial_pruning_support",
        "format_options",
        "write_capabilities",
    )

    @classmethod
    def from_json(cls, documento: dict[str, Any]) -> "Driver":
        _pretendi(documento, cls.OBBLIGATORI, "catalog.drivers[]")
        return cls(
            **{campo: documento[campo] for campo in cls.OBBLIGATORI},
            raw=dict(documento),
        )

    @property
    def writable(self) -> bool:
        """Il driver dichiara di saper scrivere.

        Derivato da `direction`, che il descrittore porta: `bidirectional` o
        `write_only`. E' una comodita' e non un'affermazione nuova -- la fonte
        resta il campo, e chi vuole il resto lo legge da li'.
        """
        return self.direction in ("bidirectional", "write_only")

    @property
    def readable(self) -> bool:
        return self.direction in ("bidirectional", "read_only")


@dataclass(frozen=True)
class Catalog:
    """La busta di `plenora-io catalog`.

    `determinism` sta al primo livello e riguarda **la busta**, non i driver: e'
    l'unica superficie che il progetto promette byte per byte, ed e' scritto li'
    perche' e' una proprieta' del documento intero.
    """

    status: str
    protocol_version: int
    contract: str
    determinism: str
    drivers: list[Driver]
    raw: dict[str, Any] = field(default_factory=dict, repr=False)

    OBBLIGATORI = ("status", "protocol_version", "contract", "determinism", "drivers")

    @classmethod
    def from_json(cls, documento: dict[str, Any]) -> "Catalog":
        _pretendi(documento, cls.OBBLIGATORI, "catalog")
        elenco = documento["drivers"]
        if not isinstance(elenco, list):
            raise ProtocolError(
                f"catalog.drivers e' {type(elenco).__name__} e non un elenco."
            )
        return cls(
            status=documento["status"],
            protocol_version=documento["protocol_version"],
            contract=documento["contract"],
            determinism=documento["determinism"],
            drivers=[Driver.from_json(voce) for voce in elenco],
            raw=dict(documento),
        )

    def driver(self, identificatore: str) -> Driver:
        """Il driver con quell'id, o `KeyError`.

        Non restituisce `None`: un id che non c'e' e' quasi sempre un refuso, e
        un `None` restituito lo trasforma in un `AttributeError` tre righe piu'
        in la', dove il nome sbagliato non si vede piu'.
        """
        for driver in self.drivers:
            if driver.id == identificatore:
                return driver
        noti = ", ".join(sorted(d.id for d in self.drivers))
        raise KeyError(f"nessun driver «{identificatore}»; il catalogo ha: {noti}")

    @property
    def available(self) -> list[Driver]:
        """I driver che **questo** binario puo' usare davvero."""
        return [driver for driver in self.drivers if driver.available]
