"""SDK Python per la CLI `plenora-io`.

Un wrapper sul protocollo v2, non un binding: nessun codice nativo, nessun
download. Il confine pubblico di questo prodotto e' la busta JSON -- che
`release/cli-protocol-v2.json` ratifica campo per campo -- e l'API Rust e'
dichiarata `internal_unstable`. L'SDK si appoggia alla sola cosa che il
progetto promette.

# Che cosa c'e' oggi

La scoperta del binario, il manifesto dell'artefatto, il controllo del profilo,
`version()` del pacchetto, `Client.capabilities()` del binario, e i comandi:
`--version`, `catalog`, `inspect`, `layers`, `validate`, `read`, `write` e
`convert`. Con l'extra `plenora-io[pyarrow]`, `Client.read_table()` e
`Client.write()` da un oggetto Arrow (vedi `arrow.py`).

# Il contratto Python

Il pacchetto e' scritto contro `plenora-python-sdk-v1` (PYTHON-SDK-1.0 di
plenora-contracts), requisito per requisito: la tabella di conformita' e le
deviazioni dichiarate stanno in `sdk/python/README.md`. `capabilities()`
dichiara la superficie `python_sdk` (PYTHON-SDK-1.0 §7); il manifesto di
adozione la reclamera' quando il catalogo dei contratti la dichiarera' per
IO-tools.

# Gli errori si distinguono per **categoria**, non per messaggio

`except NotFoundError` e non `if "non trovato" in str(errore)`. La categoria e'
un vocabolario chiuso del contratto; il messaggio e' curato per chi legge e ci
riserviamo di riscriverlo. Le diciotto sottoclassi di `CommandFailed`
corrispondono una a una alle categorie, e un gate lo verifica.
"""

from importlib import metadata as _metadata

from .client import Client, TableRead
from .discovery import PROFILI as PROFILES
from .discovery import Manifest
from .errors import (
    AuthenticationError,
    AuthorizationError,
    BinaryNotFound,
    CancelledError,
    CommandFailed,
    ConflictError,
    CrsError,
    DataMappingError,
    ErrorEnvelope,
    ExecutionError,
    InternalError,
    InvalidArgumentError,
    InvalidConfigurationError,
    InvalidPlanError,
    IoError,
    ManifestError,
    NotFoundError,
    OptionalDependencyError,
    LocalIoError,
    CleanupError,
    PlenoraError,
    ProfileError,
    ProtocolError,
    ProtocolViolationError,
    ResourceLimitError,
    SchemaError,
    TimeoutError,
    TransientError,
    UnsupportedError,
    PackageMetadataError,
    ResultLookupError,
    UnexpectedError,
)
from . import confine as _confine
from .limits import Limits
from .models import (
    Capabilities,
    CapabilityInterface,
    CapabilityOperation,
    Catalog,
    ConvertResult,
    ConvertedLayer,
    CrsResolution,
    Delivered,
    Driver,
    Fidelity,
    FidelityReason,
    Field,
    FormatDescriptor,
    Geometry,
    Inspect,
    Layer,
    LayerSummary,
    Layers,
    LossCount,
    LossExample,
    LossReport,
    Omissions,
    OperationContent,
    OperationControls,
    Validation,
    Version,
    WriteInput,
    WriteResult,
)
from .process import Runner

#: La versione dell'SDK, e la **sola** sorgente autorevole.
#:
#: `pyproject.toml` la legge da qui con `dynamic`, invece di ripeterla:
#: scritta in due posti sarebbe divergita alla prima release fatta di fretta, e
#: il pacchetto avrebbe dichiarato una versione e importato un'altra.
#:
#: Non e' la versione del **binario**, e le due vanno tenute distinte: un SDK
#: puo' uscire per un difetto proprio senza che il prodotto cambi, e un binario
#: nuovo puo' funzionare con un SDK vecchio finche' il protocollo regge. Che
#: qui dica `2.0.0` come il prodotto e' la scelta di partire allineati, non un
#: vincolo: chi vuole la versione del prodotto la chiede a `Client.version()`.
__version__ = "4.1.1"

#: Il protocollo che questo SDK sa leggere. La busta di bootstrap non lo porta
#: -- si legge prima della negoziazione -- ma tutte le altre lo dichiarano, e
#: l'SDK non pretende di capire una busta che ne dichiari un altro.
PROTOCOL_VERSION = 2

#: Il nome della distribuzione, come pip la registra.
DISTRIBUTION = "plenora-io"


def version() -> str:
    """La versione del pacchetto **installato**, dai suoi metadati.

    PYTHON-SDK-1.0 §2: `version()` rende cio' che
    `importlib.metadata.version("plenora-io")` rende. Non e' la versione del
    binario -- quella la dice `Client.version()` -- ne' una copia di
    `__version__`: e' la lettura dei metadati, e lo smoke del pacchetto
    installato verifica che coincidano con `__version__` e con il nome della
    wheel.

    Da un checkout non installato i metadati non ci sono, e la funzione lo
    dice con `PackageMetadataError` invece di ripiegare su `__version__`: un
    ripiego risponderebbe anche quando la domanda -- che cosa e' installato --
    non ha risposta. Era `PackageNotFoundError`, fuori dalla gerarchia.
    """
    try:
        letta = _metadata.version(DISTRIBUTION)
    except Exception:  # noqa: BLE001 - il confine traduce tutto, apposta
        raise PackageMetadataError(
            "i metadati del pacchetto `plenora-io` non si leggono: il pacchetto "
            "non e' installato in questo ambiente."
        ) from None
    if type(letta) is not str:
        raise PackageMetadataError("i metadati del pacchetto non portano una versione.")
    return letta

__all__ = [
    "DISTRIBUTION",
    "AuthenticationError",
    "AuthorizationError",
    "BinaryNotFound",
    "CancelledError",
    "Capabilities",
    "CapabilityInterface",
    "CapabilityOperation",
    "Catalog",
    "Client",
    "CommandFailed",
    "ConflictError",
    "ConvertResult",
    "ConvertedLayer",
    "CrsError",
    "CrsResolution",
    "DataMappingError",
    "Delivered",
    "Driver",
    "ErrorEnvelope",
    "ExecutionError",
    "Fidelity",
    "FidelityReason",
    "Field",
    "FormatDescriptor",
    "Geometry",
    "Inspect",
    "InternalError",
    "InvalidArgumentError",
    "InvalidConfigurationError",
    "InvalidPlanError",
    "IoError",
    "Layer",
    "LayerSummary",
    "Layers",
    "Limits",
    "LossCount",
    "LossExample",
    "LossReport",
    "Manifest",
    "ManifestError",
    "NotFoundError",
    "Omissions",
    "OperationContent",
    "OperationControls",
    "OptionalDependencyError",
    "LocalIoError",
    "CleanupError",
    "PROFILES",
    "PROTOCOL_VERSION",
    "PackageMetadataError",
    "ResultLookupError",
    "UnexpectedError",
    "PlenoraError",
    "ProfileError",
    "ProtocolError",
    "ProtocolViolationError",
    "ResourceLimitError",
    "Runner",
    "SchemaError",
    "TableRead",
    "TimeoutError",
    "TransientError",
    "UnsupportedError",
    "Validation",
    "Version",
    "WriteInput",
    "WriteResult",
    "__version__",
    "version",
]

# La rete (vedi `confine.py`): ogni classe e ogni funzione esportata, salvo le
# eccezioni, passa da `confinato`. Qui e non classe per classe, perche' l'elenco
# delle cose pubbliche e' `__all__`, e una prova lo confronta con cio' che e'
# avvolto: un nome nuovo esportato senza rete non puo' sfuggire.
_confine.confina_esportati(globals(), __all__)
