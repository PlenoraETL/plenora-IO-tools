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
    ConvertedLayer,
    ConvertResult,
    CrsResolution,
    Driver,
    Fidelity,
    FidelityReason,
    Field,
    FormatDescriptor,
    Geometry,
    Inspect,
    Layer,
    Layers,
    LayerSummary,
    LossCount,
    LossExample,
    LossReport,
    Omissions,
    Validation,
    Version,
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
__version__ = "2.0.0"

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
    "__version__",
]
