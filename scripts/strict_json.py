"""Lettura JSON di controllo: chiavi uniche, senza dati negli errori di duplicazione."""

import json


def _unique_pairs(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise json.JSONDecodeError("chiave JSON duplicata", "", 0)
        result[key] = value
    return result


def loads(text):
    """Legge un documento controllando anche gli oggetti annidati e negli array."""
    return json.loads(text, object_pairs_hook=_unique_pairs)


def load(handle):
    """Legge un documento da un file aperto."""
    return loads(handle.read())
