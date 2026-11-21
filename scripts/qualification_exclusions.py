"""Owner policy for declared campaign exclusions (I).

Port of the harness gate_i policy and fixture_qualifica.limiti_sql. No harness
imports, state rewriting, or additional normative Arrow vocabulary. Property
names come from IO's property table; the adopted oracle owns error phases.
"""
from collections import Counter

DIREZIONI = ('io_to_data_to_database', 'database_to_data_to_io')
HOP = {
    DIREZIONI[0]: ('io.read', 'data.run', 'database.write/read', 'io.write ipc'),
    DIREZIONI[1]: ('database.read', 'database.read stabile', 'data.run', 'io.write ipc'),
}
PG_REJECTIONS = {
    'dimensions_unknown': {'crs'},
    'axis_lat_lon': {'unsupported', 'crs'},
    'uint64_ordering': {'data_mapping', 'unsupported'},
}
LIMITI_SORGENTE_SQL = {
    "axis_lat_lon": "PostgreSQL richiede assi east-first: lat_lon non registrabile senza trasformare le coordinate",
    "conflicting_crs": "GEO-005: dichiarazione CRS conflittuale non registrabile lecitamente",
    "dimensions_unknown": "GEO-009: dimensioni unknown con SRID non rappresentabili nel typmod",
    "uint64_ordering": "PostgreSQL non offre uint64: numeric(20,0) non e una sorgente Arrow equivalente",
}


# I metadati nativi del formato (regola Removed/Preserved della tabella delle
# proprieta): una sorgente SQL non li ha, come `native_metadata`.
METADATI_NATIVI = ['native_metadata', 'field_alias', 'field_domain', 'domains', 'subtypes', 'layer_alias']


def limiti_sql(nome, properties):
    if nome not in LIMITI_SORGENTE_SQL:
        return {}
    reason = 'limite sorgente SQL: ' + LIMITI_SORGENTE_SQL[nome]
    keys = list(properties) if nome == 'uint64_ordering' else ['axis_order', *METADATI_NATIVI]
    if nome == 'conflicting_crs':
        keys += ['crs_id', 'crs_resolution', 'crs_definition', 'crs_definition_format']
    if nome == 'dimensions_unknown':
        keys += ['types_declaration', 'geometry_types']
    return dict.fromkeys(keys, reason)

REJECT = "rifiuto atteso verificato separatamente; conservazione non eseguita"
DEPENDENCY = "hop non raggiunto: dipendenza fallita"
CONFLICT_REASONS = {
    "GEO-005 e gate IO: propagazione non applicabile dopo rifiuto verificato",
    "report positivo non applicabile alla prova di rifiuto; nessuna conservazione attestata",
    "ingresso conflittuale rifiutato da GEO-005 prima della decisione; ResolvedByDecision su contratto gia costruito non prova questa catena",
}

# Inventario delle prove indipendenti richieste dalle decisioni del titolare.
SUPPLEMENTARY = {
    "crs_missing": ["gpkg-crs-obbligatorio", "senza CRS: IPC positivo", "GeoPackage negativo e IPC positivo: report io.write"],
    "unknown_key": ["gpkg_ritorno: inventario per chiave", "gpkg_ritorno: attesa esatta",
                    "GeoPackage ritorno: report io.write", "GeoPackage ritorno: report io.read"],
    "uint64_ordering": ["uint64 sort effetti True", "uint64 sort True", "uint64 sort effetti False", "uint64 sort False",
                        "uint64 ordinamento: report data.run", "uint64 ordinamento: report data.run"],
    "polygon_xy_projected": ["centroid: " + p for p in ("effetti localizzati", "types_declaration", "types", "dimensions",
        "precision", "crs_id", "crs_resolution", "axis_order", "encoding", "eredita nativa",
        "identita della colonna sostituita", "colonne passanti", "coordinate")] + ["derivata centroid: report data.run"],
    "conflicting_crs": ["conflitto: rifiuto GEO-005", "conflitto propagato", "conflitto propagato: report data.run",
                        "conflitto write fail-closed", "conflitto rifiutato: report io.write", "piano risolutivo"],
}

def supplementary_missing(cell):
    f = cell["assi"]["fixture"]
    required = Counter(("supplementare indipendente", p) for p in SUPPLEMENTARY.get(f, []))
    if f == "conflicting_crs":
        required.update((component, "GEO-005 indipendente") for component in ("io", "data", "database"))
    if f == "crs_missing":
        required[("io.write gpkg negativo", "gpkg-crs-obbligatorio")] = 1
    if f == "unknown_key":
        required.update([("data.run", "piano esplicito lecito"), ("data.run", "piano esplicito: rapporto localizzato")])
    direction = cell["assi"].get("direzione")
    if f in PG_REJECTIONS and direction == DIREZIONI[0]:
        required[("database.write", "rifiuto senza effetto permanente")] = 1
    if f == "dimensions_unknown" and direction == DIREZIONI[1]:
        required[("database.read", "rifiuto contrattuale GEO-009")] = 1
    actual = Counter((e.get("hop"), e.get("prova")) for e in cell["extra"])
    return [{"stato": "SALTATO", "motivo": "prova indipendente assente o duplicata", "hop": h, "prova": p}
            for (h, p), n in required.items() if actual[(h, p)] != n]


def witness(cell, phases):
    a = cell["assi"]
    f, direction = a["fixture"], a["direzione"]
    good = [e for e in cell["extra"] if e.get("stato") == "VERDE"]
    def axes(e, categories, phases, effects):
        x = e.get("assi", {})
        return (x.get("category") in categories and x.get("phase") in phases
                and x.get("remote_effect") in effects and x.get("retry") == "never")
    if f == "conflicting_crs":
        return {e["hop"] for e in good if e.get("prova") == "GEO-005 indipendente"
                and axes(e, {"crs"}, set(phases), {"none"})
                and e.get("boundary") == "first_validating_boundary"
                and all(e.get(k) for k in ("before", "after", "error"))} == {"io", "data", "database"}
    if direction == DIREZIONI[0] and f in {"axis_lat_lon", "dimensions_unknown", "uint64_ordering"}:
        return any(e.get("prova") == "rifiuto senza effetto permanente" and e["hop"] == "database.write"
                   and axes(e, PG_REJECTIONS[f], {"validate", "prepare"}, {"none"}) for e in good)
    if f == "dimensions_unknown" and direction == DIREZIONI[1]:
        return any(e.get("prova") == "rifiuto contrattuale GEO-009" and e["hop"] == "database.read"
                   and (axes(e, {"crs"}, {"read"}, {"none", "rolled_back"}) or
                        (a["strada"] == "Runtime" and axes(e, {"crs"}, {"read"}, {"partial"})
                         and bool(e.get("partial_sink")))) for e in good)
    return False


def rule(cell, row, properties, phases):
    """Una riga ignota, un motivo mutato o una prova assente restano blocchi."""
    state, reason = row.get("stato"), row.get("motivo")
    if (cell['assi']['fixture'] == 'crs_missing' and row.get('prova') == 'gpkg-crs-obbligatorio'
            and state != 'ATTESO-ROSSO'):
        return None
    if (cell['assi']['fixture'] == 'conflicting_crs' and
            (('proprieta' in row and cell['assi']['direzione'] == DIREZIONI[0]) or row.get('prova') in {
                'conflitto propagato', 'conflitto propagato: report data.run',
                'conflitto rifiutato: report io.write', 'piano risolutivo'}) and state != 'SALTATO'):
        return None
    if state == "VERDE":
        return "executed" if not row.get("sorpresa") and reason not in {REJECT, DEPENDENCY, *CONFLICT_REASONS} else None
    a = cell["assi"]
    f, direction, hop = a["fixture"], a["direzione"], row.get("hop")
    if state == "ATTESO-ROSSO":
        if (f == "crs_missing" and row.get("prova") == "gpkg-crs-obbligatorio"
                and row.get("lacune") == ["gpkg-crs-obbligatorio"] and not row.get("sorpresa")
                and hop in {"io.write gpkg negativo", "supplementare indipendente"}
                and reason in {"rifiuto contrattuale distinto da IPC", "negativo distinto; IPC resta prova positiva"}):
            return "GPKG-CRS-required"
        return None
    if state != "SALTATO":
        return None
    if f in LIMITI_SORGENTE_SQL and direction == DIREZIONI[1] and hop == "database.read":
        limits = limiti_sql(f, properties)
        if row.get("proprieta") in limits and reason == limits[row["proprieta"]]:
            return "SQL-source-limit"
        if row.get("prova") == "variante inversa" and reason == LIMITI_SORGENTE_SQL[f]:
            return "SQL-source-limit"
        if (f == "uint64_ordering" and row.get("prova") == "valori/tipi/nullabilita/ordine"
                and reason == "numeric SQL non e uint64; semina non equivalente"):
            return "SQL-source-limit"
    if not witness(cell, phases):
        return None
    if f == "conflicting_crs":
        if (reason in {REJECT, DEPENDENCY} and hop in {"io.read", "database.read", "database.read stabile", "data.run", "database.write/read", "io.write ipc"}
                or reason in CONFLICT_REASONS and hop == "supplementare indipendente"
                and row.get("prova") in {"conflitto propagato", "conflitto propagato: report data.run", "conflitto rifiutato: report io.write", "piano risolutivo"}):
            return "GEO-005/REJ-001"
    rejected = "database.write/read" if direction == DIREZIONI[0] else "database.read"
    downstream = {"io.write ipc"} if direction == DIREZIONI[0] else {"database.read stabile", "data.run", "io.write ipc"}
    if reason == REJECT and hop == rejected or reason == DEPENDENCY and hop in downstream:
        return "declared-PG-rejection"
    return None


def missing(cell, properties, phases):
    required = {(h, p) for h in HOP[cell["assi"]["direzione"]] for p in properties}
    actual = [(r.get("hop"), r.get("proprieta")) for r in cell["righe"]]
    absent = [] if set(actual) == required and len(actual) == len(required) else [{"stato": "SALTATO", "motivo": "inventario proprieta/hop incompleto o duplicato"}]
    return absent + supplementary_missing(cell) + [r for r in cell["righe"] + cell["extra"] if rule(cell, r, properties, phases) is None]


def qualified(cell, properties, phases):
    if cell.get("status") not in {"passed", "partial"} or not cell.get("observed") or missing(cell["observed"], properties, phases):
        return False
    for hop in cell["hops"]:
        if cell['fixture'] == 'conflicting_crs' and hop.get('status') != 'passed':
            return False  # All three independent boundary probes remain mandatory.
        if hop["status"] == "passed":
            if hop.get('source_limits') or hop.get('execution') in ('not_run', 'attempted'):
                return False
            continue
        if hop["status"] != "skipped":
            return False
        if hop.get("source_limits"):
            if (cell["direction"] != DIREZIONI[1] or hop["component"] != "plenora-database-tools"
                    or cell["fixture"] not in {"axis_lat_lon", "uint64_ordering"}
                    or hop["source_limits"] != limiti_sql(cell["fixture"], properties)
                    or hop.get("reason") != "limite sorgente SQL: " + LIMITI_SORGENTE_SQL[cell["fixture"]]
                    or hop.get('execution') in ('not_run', 'attempted')
                    or not all(hop.get(k) for k in ('command', 'output', 'expected'))):
                return False
        else:
            label = {"plenora-database-tools": "database.write/read" if cell["direction"] == DIREZIONI[0] else "database.read",
                     "plenora-data-tools": "data.run", "plenora-IO-tools": "io.read" if cell["direction"] == DIREZIONI[0] else "io.write ipc"}[hop["component"]]
            if not (witness(cell["observed"], phases) and hop.get("reason") in {REJECT, DEPENDENCY}
                    and hop.get("execution") == ("attempted" if hop["reason"] == REJECT else "not_run")
                    and any(r["hop"] == label and r["stato"] == "SALTATO" and r["motivo"] == hop["reason"]
                            for r in cell["observed"]["righe"])):
                return False
    return True

