"""SDK Python per la CLI `plenora-io`.

Un wrapper sul protocollo v2, non un binding: nessun codice nativo, nessun
download. Il confine pubblico di questo prodotto e' la busta JSON -- che
`release/cli-protocol-v2.json` ratifica campo per campo -- e l'API Rust e'
dichiarata `internal_unstable`. L'SDK si appoggia alla sola cosa che il
progetto promette.

# Che cosa c'e' oggi

La scoperta del binario, il manifesto dell'artefatto, il controllo del profilo,
e i cinque comandi: `--version`, `catalog`, `inspect`, `layers`, `validate`
e `convert`.

# Gli errori si distinguono per **categoria**, non per messaggio

`except NotFoundError` e non `if "non trovato" in str(errore)`. La categoria e'
un vocabolario chiuso del contratto; il messaggio e' curato per chi legge e ci
riserviamo di riscriverlo. Le diciotto sottoclassi di `CommandFailed`
corrispondono una a una alle categorie, e un gate lo verifica.
"""

from .client import Client
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
    InvalidConfigurationError,
    InvalidPlanError,
    IoError,
    ManifestError,
    NotFoundError,
    PlenoraError,
    ProfileError,
    ProtocolError,
    ProtocolViolationError,
    ResourceLimitError,
    SchemaError,
    TimeoutError,
    TransientError,
    UnsupportedError,
)
from .limits import Limits
from .models import (
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
    Validation,
    Version,
    WriteInput,
    WriteResult,
)
from .process import NativeRunner, Runner

#: La versione dell'SDK, e la **sola** sorgente autorevole.
#:
#: `pyproject.toml` la legge da qui con `dynamic`, invece di ripeterla:
#: scritta in due posti sarebbe divergita alla prima release fatta di fretta, e
#: il pacchetto avrebbe dichiarato una versione e importato un'altra.
#:
#: E' la versione del workspace, cioe' del CLI: lo SDK esce col prodotto, e
#: la wheel nativa la prende dallo stesso `Cargo.toml`. Erano due numeri
#: indipendenti, e la 4.1.0 ha spedito lo SDK 4.0.0 accanto al CLI 4.1.0:
#: `scripts/check_sdk_python.py` ora verifica che coincidano. Chi vuole la
#: versione del binario che esegue la chiede a `Client.version()`.
__version__ = "4.1.0"

#: Il protocollo che questo SDK sa leggere. La busta di bootstrap non lo porta
#: -- si legge prima della negoziazione -- ma tutte le altre lo dichiarano, e
#: l'SDK non pretende di capire una busta che ne dichiari un altro.
PROTOCOL_VERSION = 2

__all__ = [
    "AuthenticationError",
    "AuthorizationError",
    "BinaryNotFound",
    "CancelledError",
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
    "NativeRunner",
    "NotFoundError",
    "Omissions",
    "PROFILES",
    "PROTOCOL_VERSION",
    "PlenoraError",
    "ProfileError",
    "ProtocolError",
    "ProtocolViolationError",
    "ResourceLimitError",
    "Runner",
    "SchemaError",
    "TimeoutError",
    "TransientError",
    "UnsupportedError",
    "Validation",
    "Version",
    "WriteInput",
    "WriteResult",
    "__version__",
]
