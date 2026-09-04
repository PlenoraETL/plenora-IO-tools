"""SDK Python per la CLI `plenora-io`.

Un wrapper sul protocollo v2, non un binding: nessun codice nativo, nessun
download. Il confine pubblico di questo prodotto e' la busta JSON -- che
`release/cli-protocol-v2.json` ratifica campo per campo -- e l'API Rust e'
dichiarata `internal_unstable`. L'SDK si appoggia alla sola cosa che il
progetto promette.

Questo primo ciclo copre la scoperta del binario, il manifesto dell'artefatto,
il controllo del profilo e le due buste `--version` e `catalog`. `inspect`,
`layers` e `convert` non ci sono ancora.
"""

from .client import Client
from .discovery import PROFILI as PROFILES
from .discovery import Manifest
from .errors import (
    BinaryNotFound,
    CommandFailed,
    ErrorEnvelope,
    ManifestError,
    PlenoraError,
    ProfileError,
    ProtocolError,
)
from .models import Catalog, Driver, Version

#: La versione dell'SDK, che **non** e' quella del binario.
#:
#: Sono due cose distinte e vanno tenute tali: un SDK puo' uscire per un difetto
#: proprio senza che il prodotto cambi, e un binario nuovo puo' funzionare con
#: un SDK vecchio finche' il protocollo regge. Chi vuole la versione del
#: prodotto la chiede a `Client.version()`, che la prende dal binario.
__version__ = "0.1.0"

#: Il protocollo che questo SDK sa leggere. La busta di bootstrap non lo porta
#: -- si legge prima della negoziazione -- ma tutte le altre lo dichiarano, e
#: l'SDK non pretende di capire una busta che ne dichiari un altro.
PROTOCOL_VERSION = 2

__all__ = [
    "BinaryNotFound",
    "Catalog",
    "Client",
    "CommandFailed",
    "Driver",
    "ErrorEnvelope",
    "Manifest",
    "ManifestError",
    "PROFILES",
    "PROTOCOL_VERSION",
    "PlenoraError",
    "ProfileError",
    "ProtocolError",
    "Version",
    "__version__",
]
