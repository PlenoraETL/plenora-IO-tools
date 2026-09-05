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


@dataclass(frozen=True, kw_only=True)
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


@dataclass(frozen=True, kw_only=True)
class FormatDescriptor:
    """Il descrittore di un formato: che cosa il driver sa fare, e come.

    E' lo **stesso** tipo che `catalog` mette dentro ogni elemento di `drivers`
    e che `inspect` mette sotto `format`. Non e' una somiglianza: il gate delle
    buste ha misurato che i percorsi del secondo sono un sottoinsieme esatto dei
    primi, e i due campi in piu' del catalogo sono quelli che `Driver` aggiunge.

    Scriverlo una volta sola e' cio' che impedisce alle due copie di divergere
    senza che nessuno se ne accorga.

    `kw_only` non e' un vezzo: `Driver` eredita da qui e aggiunge campi
    obbligatori dopo `raw`, che ha un valore predefinito. Senza, la dataclass
    non si costruirebbe.
    """

    id: str
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

    #: I campi che il protocollo dichiara `sempre: true` sotto `.format`.
    OBBLIGATORI = (
        "id",
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
    def from_json(cls, documento: dict[str, Any]) -> "FormatDescriptor":
        _pretendi(documento, cls.OBBLIGATORI, "format")
        return cls(
            **{campo: documento[campo] for campo in cls.OBBLIGATORI},
            raw=dict(documento),
        )

    @property
    def writable(self) -> bool:
        """Il formato dichiara di saper scrivere.

        Derivato da `direction`, che il descrittore porta. E' una comodita' e
        non un'affermazione nuova -- la fonte resta il campo, e chi vuole il
        resto lo legge da li'.
        """
        return self.direction in ("bidirectional", "write_only")

    @property
    def readable(self) -> bool:
        return self.direction in ("bidirectional", "read_only")


@dataclass(frozen=True, kw_only=True)
class Driver(FormatDescriptor):
    """Un elemento di `catalog.drivers`: il descrittore, piu' due campi suoi.

    `available` e `required_feature` non stanno nel descrittore che `inspect`
    restituisce: sono del catalogo, e dicono se **questo** binario possa usare
    quel driver. Un descrittore descrive un formato; il catalogo descrive
    un'installazione.
    """

    available: bool
    required_feature: str | None

    #: I due campi che il catalogo aggiunge. Il gate li somma a quelli del
    #: descrittore e confronta il totale con `.drivers[]`.
    PROPRI = ("available", "required_feature")

    @classmethod
    def from_json(cls, documento: dict[str, Any]) -> "Driver":
        campi = FormatDescriptor.OBBLIGATORI + cls.PROPRI
        _pretendi(documento, campi, "catalog.drivers[]")
        return cls(
            **{campo: documento[campo] for campo in campi},
            raw=dict(documento),
        )


@dataclass(frozen=True, kw_only=True)
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


# --- la sezione di fedelta', che tre buste su cinque portano ----------------


@dataclass(frozen=True, kw_only=True)
class Omissions:
    """Quante voci la diagnostica ha lasciato fuori, e per quale delle quattro
    cause.

    Restano separate perche' portano a decisioni diverse: chi ha perso
    *categorie* sa di non conoscere tutti i tipi di perdita, chi ha perso
    *esempi* li conosce e non ne ha campioni. Sommarle in un numero solo direbbe
    che qualcosa manca senza dire che cosa.
    """

    categorie_omesse: int
    ragioni_omesse: int
    esempi_omessi: int
    omesse_per_byte: int

    OBBLIGATORI = (
        "categorie_omesse",
        "ragioni_omesse",
        "esempi_omessi",
        "omesse_per_byte",
    )

    @classmethod
    def from_json(cls, documento: dict[str, Any]) -> "Omissions":
        _pretendi(documento, cls.OBBLIGATORI, "fidelity.omesse")
        return cls(**{campo: documento[campo] for campo in cls.OBBLIGATORI})

    @property
    def any(self) -> bool:
        """Qualcosa e' stato lasciato fuori, di qualunque delle quattro cause."""
        return any(getattr(self, campo) for campo in self.OBBLIGATORI)


@dataclass(frozen=True, kw_only=True)
class FidelityReason:
    """Una ragione per cui la fedelta' non e' esatta.

    `field_index` e `layer_index` vengono **insieme** o non vengono: una ragione
    che nomina un campo nomina anche il layer in cui sta. Quelle che riguardano
    il formato nel suo insieme non ne hanno, ed e' perche' sono opzionali qui.
    """

    code: str
    detail: str
    field_index: int | None = None
    layer_index: int | None = None
    raw: dict[str, Any] = field(default_factory=dict, repr=False)

    OBBLIGATORI = ("code", "detail")
    OPZIONALI = ("field_index", "layer_index")

    @classmethod
    def from_json(cls, documento: dict[str, Any]) -> "FidelityReason":
        _pretendi(documento, cls.OBBLIGATORI, "fidelity.reasons[]")
        return cls(
            code=documento["code"],
            detail=documento["detail"],
            field_index=documento.get("field_index"),
            layer_index=documento.get("layer_index"),
            raw=dict(documento),
        )

    @property
    def localized(self) -> bool:
        """La ragione nomina un campo preciso invece del formato intero."""
        return self.field_index is not None


@dataclass(frozen=True, kw_only=True)
class Fidelity:
    """La sezione di fedelta': quanto si perde, e perche'.

    `troncato` non e' un dettaglio da ignorare: dice che l'elenco che si sta
    leggendo **non e' tutto**, e un consumatore che lo trascurasse concluderebbe
    dall'assenza di una ragione che quella perdita non c'e'.
    """

    level: str
    reasons: list[FidelityReason]
    troncato: bool
    omesse: Omissions
    omesse_esatte: bool
    raw: dict[str, Any] = field(default_factory=dict, repr=False)

    OBBLIGATORI = ("level", "reasons", "troncato", "omesse", "omesse_esatte")

    @classmethod
    def from_json(cls, documento: dict[str, Any]) -> "Fidelity":
        _pretendi(documento, cls.OBBLIGATORI, "fidelity")
        ragioni = documento["reasons"]
        if not isinstance(ragioni, list):
            raise ProtocolError(
                f"fidelity.reasons e' {type(ragioni).__name__} e non un elenco."
            )
        return cls(
            level=documento["level"],
            reasons=[FidelityReason.from_json(voce) for voce in ragioni],
            troncato=documento["troncato"],
            omesse=Omissions.from_json(documento["omesse"]),
            omesse_esatte=documento["omesse_esatte"],
            raw=dict(documento),
        )

    @property
    def exact(self) -> bool:
        """La lettura non perde niente.

        `level == "exact"` **e** nessun troncamento: una sezione troncata non
        puo' dirsi esatta, perche' le ragioni che mancano non si sono viste.
        """
        return self.level == "exact" and not self.troncato


# --- il layer, e cio' che lo descrive ---------------------------------------


@dataclass(frozen=True, kw_only=True)
class Field:
    """Una colonna: il nome, il tipo Arrow, se ammetta nulli, se sia geometria."""

    name: str
    type: str
    nullable: bool
    geometry: bool
    raw: dict[str, Any] = field(default_factory=dict, repr=False)

    OBBLIGATORI = ("name", "type", "nullable", "geometry")

    @classmethod
    def from_json(cls, documento: dict[str, Any]) -> "Field":
        _pretendi(documento, cls.OBBLIGATORI, "layer.fields[]")
        return cls(
            **{campo: documento[campo] for campo in cls.OBBLIGATORI},
            raw=dict(documento),
        )


@dataclass(frozen=True, kw_only=True)
class CrsResolution:
    """Come il CRS e' stato risolto, e da dove viene.

    `status` e' l'informazione che conta: un identificatore risolto e uno
    assunto dal chiamante hanno lo stesso aspetto in `id`, e solo questo campo
    li distingue. Un consumatore che li confondesse attribuirebbe al file una
    coordinata che gli e' stata suggerita da fuori.
    """

    id: str
    kind: str
    status: str
    axis_order: str
    definition: str | None
    definition_format: str | None
    raw: dict[str, Any] = field(default_factory=dict, repr=False)

    OBBLIGATORI = ("id", "kind", "status", "axis_order", "definition", "definition_format")

    @classmethod
    def from_json(cls, documento: dict[str, Any]) -> "CrsResolution":
        _pretendi(documento, cls.OBBLIGATORI, "layer.geometry.crs_resolution")
        return cls(
            **{campo: documento[campo] for campo in cls.OBBLIGATORI},
            raw=dict(documento),
        )


@dataclass(frozen=True, kw_only=True)
class Geometry:
    """La colonna geometrica di un layer, col suo sistema di riferimento."""

    name: str
    kind: str
    crs: str
    crs_resolution: CrsResolution
    raw: dict[str, Any] = field(default_factory=dict, repr=False)

    OBBLIGATORI = ("name", "kind", "crs", "crs_resolution")

    @classmethod
    def from_json(cls, documento: dict[str, Any]) -> "Geometry":
        _pretendi(documento, cls.OBBLIGATORI, "layer.geometry")
        return cls(
            name=documento["name"],
            kind=documento["kind"],
            crs=documento["crs"],
            crs_resolution=CrsResolution.from_json(documento["crs_resolution"]),
            raw=dict(documento),
        )


@dataclass(frozen=True, kw_only=True)
class Layer:
    """Un layer con il suo schema, come `inspect` lo descrive."""

    id: int
    name: str
    fields: list[Field]
    geometry: Geometry
    raw: dict[str, Any] = field(default_factory=dict, repr=False)

    OBBLIGATORI = ("id", "name", "fields", "geometry")

    @classmethod
    def from_json(cls, documento: dict[str, Any]) -> "Layer":
        _pretendi(documento, cls.OBBLIGATORI, "inspect.layers[]")
        colonne = documento["fields"]
        if not isinstance(colonne, list):
            raise ProtocolError(
                f"layers[].fields e' {type(colonne).__name__} e non un elenco."
            )
        return cls(
            id=documento["id"],
            name=documento["name"],
            fields=[Field.from_json(voce) for voce in colonne],
            geometry=Geometry.from_json(documento["geometry"]),
            raw=dict(documento),
        )

    def field(self, name: str) -> Field:
        """La colonna con quel nome, o `KeyError` che elenca quelle che ci sono."""
        for colonna in self.fields:
            if colonna.name == name:
                return colonna
        noti = ", ".join(colonna.name for colonna in self.fields)
        raise KeyError(f"nessun campo «{name}» nel layer «{self.name}»; ci sono: {noti}")

    @property
    def attributes(self) -> list[Field]:
        """Le colonne che non sono la geometria."""
        return [colonna for colonna in self.fields if not colonna.geometry]


@dataclass(frozen=True, kw_only=True)
class LayerSummary:
    """Un layer come `layers` lo riassume: senza lo schema.

    Non e' un `Layer` incompleto ed e' un tipo a parte: `layers` esiste per
    rispondere «che cosa c'e' qui dentro» senza pagare l'inferenza dello schema,
    e un modello che promettesse `fields` vuoti farebbe credere a un layer senza
    colonne.
    """

    id: int
    name: str
    field_count: int
    geometry_crs: str
    raw: dict[str, Any] = field(default_factory=dict, repr=False)

    OBBLIGATORI = ("id", "name", "field_count", "geometry_crs")

    @classmethod
    def from_json(cls, documento: dict[str, Any]) -> "LayerSummary":
        _pretendi(documento, cls.OBBLIGATORI, "layers.layers[]")
        return cls(
            **{campo: documento[campo] for campo in cls.OBBLIGATORI},
            raw=dict(documento),
        )


# --- le due buste di questo ciclo -------------------------------------------


@dataclass(frozen=True, kw_only=True)
class Inspect:
    """La busta di `plenora-io inspect`: il descrittore e i layer con lo schema."""

    status: str
    protocol_version: int
    contract: str
    format: FormatDescriptor
    fidelity: Fidelity
    layers: list[Layer]
    raw: dict[str, Any] = field(default_factory=dict, repr=False)

    OBBLIGATORI = ("status", "protocol_version", "contract", "format", "fidelity", "layers")

    @classmethod
    def from_json(cls, documento: dict[str, Any]) -> "Inspect":
        _pretendi(documento, cls.OBBLIGATORI, "inspect")
        elenco = documento["layers"]
        if not isinstance(elenco, list):
            raise ProtocolError(
                f"inspect.layers e' {type(elenco).__name__} e non un elenco."
            )
        return cls(
            status=documento["status"],
            protocol_version=documento["protocol_version"],
            contract=documento["contract"],
            format=FormatDescriptor.from_json(documento["format"]),
            fidelity=Fidelity.from_json(documento["fidelity"]),
            layers=[Layer.from_json(voce) for voce in elenco],
            raw=dict(documento),
        )

    def layer(self, name: str) -> Layer:
        """Il layer con quel nome, o `KeyError` che elenca quelli che ci sono."""
        for strato in self.layers:
            if strato.name == name:
                return strato
        noti = ", ".join(strato.name for strato in self.layers)
        raise KeyError(f"nessun layer «{name}»; il file ne ha: {noti}")


@dataclass(frozen=True, kw_only=True)
class Layers:
    """La busta di `plenora-io layers`: i layer riassunti.

    `format` qui e' una **stringa** -- l'identificatore del driver -- e non il
    descrittore che `inspect` restituisce. I due campi hanno lo stesso nome e
    tipi diversi, ed e' il wire a volerlo: modellarli uguali avrebbe richiesto
    di inventare un descrittore che questa busta non porta.
    """

    status: str
    protocol_version: int
    contract: str
    format: str
    fidelity: Fidelity
    layers: list[LayerSummary]
    raw: dict[str, Any] = field(default_factory=dict, repr=False)

    OBBLIGATORI = ("status", "protocol_version", "contract", "format", "fidelity", "layers")

    @classmethod
    def from_json(cls, documento: dict[str, Any]) -> "Layers":
        _pretendi(documento, cls.OBBLIGATORI, "layers")
        elenco = documento["layers"]
        if not isinstance(elenco, list):
            raise ProtocolError(
                f"layers.layers e' {type(elenco).__name__} e non un elenco."
            )
        return cls(
            status=documento["status"],
            protocol_version=documento["protocol_version"],
            contract=documento["contract"],
            format=documento["format"],
            fidelity=Fidelity.from_json(documento["fidelity"]),
            layers=[LayerSummary.from_json(voce) for voce in elenco],
            raw=dict(documento),
        )

    def layer(self, name: str) -> LayerSummary:
        for strato in self.layers:
            if strato.name == name:
                return strato
        noti = ", ".join(strato.name for strato in self.layers)
        raise KeyError(f"nessun layer «{name}»; il file ne ha: {noti}")
