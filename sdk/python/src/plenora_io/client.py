"""Il client: la scoperta, il profilo, e i comandi che questo ciclo copre.

# Che cosa sta qui e che cosa sta altrove

Qui c'e' un metodo per comando, e ciascuno fa due cose: costruire la riga di
argomenti e dire quale tipo si aspetta. L'esecuzione, la scelta del flusso e la
decodifica stanno in `process.py`, perche' sono le stesse per tutti e cinque i
comandi -- e perche' un difetto in quella catena si ripara in un posto solo.

# Perche' nessun comando ha un timeout predefinito

`convert` legge e scrive file, e quanto ci metta dipende da quanto sono grandi:
un timeout scelto da noi sarebbe un limite arbitrario travestito da difesa, e
scatterebbe sul lavoro grosso invece che sul guasto. Il parametro c'e', e chi
sa quanto puo' durare il proprio lavoro lo imposta.

# Perche' l'ambiente si passa intero

`subprocess` eredita l'ambiente del processo, e va bene: la CLI ne legge poco e
quel poco -- `PROJ_DATA`, per esempio -- e' quello che chi installa
l'artefatto ha configurato. Ripulirlo qui romperebbe installazioni che
funzionano, per una difesa che l'SDK non e' il posto giusto per fare.
"""

from __future__ import annotations

import os
from pathlib import Path

from .discovery import Manifest, leggi_manifesto, trova_binario, verifica_profilo
from .models import Catalog, Inspect, Layers, Version
from .process import Runner


class Client:
    """L'ingresso dell'SDK.

    La scoperta avviene nel costruttore, non alla prima chiamata: un client che
    esiste e' un client che ha trovato il proprio binario, e chi lo costruisce
    scopre subito che manca invece di scoprirlo a meta' di un lavoro.
    """

    def __init__(
        self,
        binary: str | os.PathLike[str] | None = None,
        *,
        timeout: float | None = None,
    ) -> None:
        percorso = trova_binario(binary)
        self._runner = Runner(percorso, timeout=timeout)
        self._manifest = leggi_manifesto(percorso)

    @property
    def binary(self) -> Path:
        return self._runner.binary

    @property
    def manifest(self) -> Manifest | None:
        """Il manifesto dell'artefatto, o `None` se il binario non ne ha uno.

        `None` non e' un guasto: un binario costruito da `cargo` e' usabile e
        non porta un manifesto. Cio' che manca e' la capacita' di dire da quale
        artefatto venga -- profilo, canale, revisione -- e i metodi che ne hanno
        bisogno lo dicono invece di indovinare.
        """
        return self._manifest

    def require_profile(self, profile: str) -> None:
        """Solleva `ProfileError` se l'artefatto non ha quel profilo.

        Da chiamare **prima** del lavoro. Il driver FileGDB vuole il profilo
        `filegdb`, e scoprirlo dal fallimento di una conversione a meta' costa
        un file di uscita parziale e un errore che parla di un driver invece che
        di un pacchetto.
        """
        verifica_profilo(self._manifest, profile)

    # --- le buste che l'SDK copre -----------------------------------------

    def version(self) -> Version:
        """La busta di bootstrap.

        E' la prima chiamata che ha senso fare: dice che binario si ha in mano,
        e lo dice senza pretendere di conoscere il protocollo.
        """
        return Version.from_json(self._runner.run(["--version"]))

    def catalog(self) -> Catalog:
        """Il catalogo dei driver di **questa** installazione."""
        return Catalog.from_json(self._runner.run(["catalog"]))

    def inspect(
        self,
        source: str | os.PathLike[str],
        *,
        assume_crs: str | None = None,
        options: dict[str, str] | None = None,
    ) -> Inspect:
        """Il descrittore del formato e i layer con il loro schema.

        Costa piu' di `layers()`: per dire di che tipo e' ogni colonna il driver
        deve inferire lo schema, e su un formato senza schema dichiarato -- CSV,
        GeoJSON -- vuol dire leggere righe. Chi ha bisogno solo dei nomi dei
        layer chieda `layers()`, che non paga quell'inferenza.

        `assume_crs` non e' una preferenza: alcuni file dichiarano un CRS che
        non si risolve, e il driver rifiuta chiuso invece di indovinare. Passarlo
        e' dire «lo so io», e resta distinguibile nella busta -- `crs_resolution`
        porta lo `status` che dice da dove il CRS viene.
        """
        return Inspect.from_json(
            self._runner.run(self._argomenti("inspect", source, assume_crs, options))
        )

    def layers(
        self,
        source: str | os.PathLike[str],
        *,
        assume_crs: str | None = None,
        options: dict[str, str] | None = None,
    ) -> Layers:
        """I layer riassunti: nome, conteggio delle colonne, CRS.

        Non porta lo schema, ed e' il motivo per cui esiste accanto a
        `inspect()`.
        """
        return Layers.from_json(
            self._runner.run(self._argomenti("layers", source, assume_crs, options))
        )

    # --- la riga di argomenti ---------------------------------------------

    @staticmethod
    def _argomenti(
        comando: str,
        source: str | os.PathLike[str],
        assume_crs: str | None,
        options: dict[str, str] | None,
    ) -> list[str]:
        """Gli argomenti, nell'ordine che la CLI attende.

        Le opzioni si passano una `--in-opt` per coppia, e la chiave non viene
        validata qui: il vocabolario delle opzioni lo dichiara il catalogo, per
        driver, e duplicarlo nell'SDK produrrebbe due elenchi destinati a
        divergere. Una chiave sconosciuta e' un rifiuto del prodotto, tipizzato
        come tutti gli altri.
        """
        argomenti = [comando, os.fspath(source)]
        if assume_crs is not None:
            argomenti += ["--assume-crs", assume_crs]
        for chiave, valore in (options or {}).items():
            argomenti += ["--in-opt", f"{chiave}={valore}"]
        return argomenti
