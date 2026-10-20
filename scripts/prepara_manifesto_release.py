"""Materializza il manifesto senza ricostruire gli artefatti congelati.

Le release (incluse le draft) si leggono con l'API autenticata. Errori di rete,
permessi o documenti invalidi non sono assenza. I messaggi non riversano dati
remoti, stderr dei sottoprocessi o contenuto degli asset nei log.
"""
from __future__ import annotations

import argparse
import hashlib
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile

try:
    from . import strict_json, check_release_contract as gate, check_manifesto_adozione as adozione
except ImportError:
    import strict_json
    import check_release_contract as gate
    import check_manifesto_adozione as adozione

class PreparazioneError(ValueError):
    """Diagnostica controllata, senza dati degli asset."""


ROOT = Path(__file__).resolve().parents[1]


def esegui(*argv: str) -> bytes:
    result = subprocess.run(argv, cwd=ROOT, capture_output=True)
    if result.returncode:
        raise PreparazioneError("acquisizione o generazione fallita; nessun manifesto qualificato")
    return result.stdout


def api(repo: str, endpoint: str):
    return strict_json.loads(esegui("gh", "api", f"repos/{repo}/{endpoint}"))


def attesi(candidate: dict) -> dict:
    versione = candidate["versione_manifesto"]
    errori = gate._assenza_sorgente(candidate, versione)
    if errori:
        raise PreparazioneError("; ".join(errori))
    nomi = {f"plenora-io-{versione}-{piattaforma}-{profilo}.{estensione}": "cli"
            for piattaforma, estensione in (("linux-x86_64", "tar.gz"), ("windows-x86_64", "zip"))
            for profilo in ("base", "filegdb")}
    # I verbali precedenti a #42 fissano soltanto i sei archivi nativi/Python.
    # Non inventare un digest sorgente per completarli retroattivamente.
    sorgente = f"plenora-io-{versione}-src.tar.gz"
    if sorgente in gate.artefatti_attesi(versione, candidate=candidate):
        nomi[sorgente] = "rust"
    fissati = {}
    for voce in candidate.get("artefatti", []):
        nome = voce.get("nome")
        if nome not in nomi:
            continue
        if (nome in fissati or voce.get("revisione") != candidate["revisione_candidate"]
                or not re.fullmatch(r"[0-9a-f]{64}", voce.get("sha256", ""))
                or type(voce.get("dimensione")) is not int or voce["dimensione"] <= 0):
            raise PreparazioneError("artefatti congelati incoerenti")
        fissati[nome] = dict(voce, superficie=nomi[nome])
    if set(fissati) != set(nomi):
        raise PreparazioneError("manifesto mancante: artefatti pubblici non tutti fissati nella candidate")
    return fissati


def verifica_manifesto(manifesto: dict, candidate: dict, fissati: dict) -> None:
    voci = manifesto.get("artifacts", [])
    if len(voci) != len(fissati) or {v.get("name") for v in voci} != set(fissati):
        raise PreparazioneError("manifesto non corrispondente al perimetro congelato")
    for voce in voci:
        fissato = fissati[voce["name"]]
        if (voce.get("digest") != "sha256:" + fissato["sha256"]
                or voce.get("version") != candidate["versione_manifesto"]
                or voce.get("surface") != fissato["superficie"]):
            raise PreparazioneError("manifesto non corrispondente ai byte congelati")


def valida_manifesto(manifesto: dict) -> None:
    """Forma, adozione e pin; i digest sono confrontati col verbale congelato.

    Il CLI check_manifesto_adozione richiede anche tutti gli archivi locali:
    qui l'asset di release puo' arrivare da solo, e il riferimento dei digest
    e' il verbale, non una seconda dichiarazione scaricata insieme all'asset.
    """
    contracts = ROOT / ".plenora-contracts"
    schema = strict_json.loads((contracts / "schemas/adoption-manifest-v4.schema.json").read_text(encoding="utf-8"))
    errori = adozione.valida_forma(manifesto, schema)
    if errori:
        raise PreparazioneError("forma del manifesto non valida")
    errori.extend(adozione.verifica_redatto(manifesto, contracts))
    errori.extend(adozione.riferimenti_incrociati(manifesto))
    if errori:
        raise PreparazioneError("adozione o pin del manifesto non validi")


def prepara(repo: str, run_id: str | None = None) -> None:
    destinazione = ROOT / "contracts/adoption-manifest.json"
    # Mai riusare un manifesto residuo dopo un'acquisizione fallita.
    destinazione.unlink(missing_ok=True)
    stato = strict_json.loads((ROOT / "assurance/current-state.json").read_text(encoding="utf-8"))
    candidate = stato["aperto"]["candidate_release"]
    if gate.evidenza_ancora_valida(candidate.get("revisione_candidate")):
        raise PreparazioneError("candidate non valida per il checkout qualificato")
    fissati = attesi(candidate)
    tag = candidate["tag_previsto"]
    if tag != "v" + candidate["versione_manifesto"]:
        raise PreparazioneError("tag non coerente con la versione congelata")
    nome = f"adoption-manifest-{candidate['versione_manifesto']}.json"
    # L'elenco autenticato include le draft; paginare evita falsi 'assente'.
    releases = strict_json.loads(esegui("gh", "api", "--paginate", "--slurp",
                                        f"repos/{repo}/releases?per_page=100"))
    trovate = [r for pagina in releases for r in pagina if r.get("tag_name") == tag]
    if len(trovate) > 1:
        raise PreparazioneError("release ambigua per la candidate")
    with tempfile.TemporaryDirectory(prefix="manifesto-release-") as temp:
        directory = Path(temp)
        manifesto_path = directory / nome
        if trovate:
            release = trovate[0]
            # Un tag esistente e' l'autorita'; per la draft senza tag serve
            # uno SHA esatto, non un ramo che possa avanzare.
            revisione = gate.revisione_risolta(f"refs/tags/{tag}")
            if revisione is None and release.get("draft"):
                target = release.get("target_commitish", "")
                if re.fullmatch(r"[0-9a-f]{40}", target):
                    revisione = target
            if revisione is None or gate.evidenza_ancora_valida(candidate["revisione_candidate"], revisione):
                raise PreparazioneError("release/draft non legata alla revisione qualificata")
            pagine = strict_json.loads(esegui("gh", "api", "--paginate", "--slurp",
                f"repos/{repo}/releases/{release['id']}/assets?per_page=100"))
            assets = [asset for pagina in pagine for asset in pagina]
            matches = [a for a in assets if a.get("name") == nome]
            if len(matches) > 1:
                raise PreparazioneError("asset manifesto ambiguo")
            if matches:
                manifesto_path.write_bytes(esegui("gh", "api", "-H", "Accept: application/octet-stream",
                    f"repos/{repo}/releases/assets/{matches[0]['id']}"))
        if not manifesto_path.exists():
            if not run_id:
                raise PreparazioneError("manifesto mancante: nessun asset di release/draft e nessun run di distribuzione indicato")
            run = api(repo, f"actions/runs/{run_id}")
            if run.get("path") != ".github/workflows/distribuzione.yml" or run.get("conclusion") != "success":
                raise PreparazioneError("il run indicato non e' una distribuzione riuscita")
            artefatti = directory / "artefatti"
            esegui("gh", "run", "download", run_id, "--repo", repo,
                   "--pattern", "deliverable-*", "--dir", str(artefatti))
            specifiche = []
            for nome_artefatto, fissato in fissati.items():
                files = list(artefatti.rglob(nome_artefatto))
                if len(files) != 1 or not files[0].is_file():
                    raise PreparazioneError("manifesto mancante: artefatti fissati assenti o ambigui nel run")
                file = files[0]
                if file.stat().st_size != fissato["dimensione"] or hashlib.sha256(file.read_bytes()).hexdigest() != fissato["sha256"]:
                    raise PreparazioneError("artefatti del run diversi dai byte congelati")
                comando = ("python3 scripts/check_superficie_rust.py" if fissato["superficie"] == "rust"
                           else "python3 scripts/check_public_contracts.py --contracts .plenora-contracts --artefatto " + nome_artefatto)
                specifiche.append(f"{fissato['superficie']}:{file}:{comando}")
            esegui(sys.executable, "scripts/costruisci-manifesto-adozione.py", *specifiche,
                   "--uscita", str(manifesto_path))
        manifesto = strict_json.loads(manifesto_path.read_text(encoding="utf-8"))
        verifica_manifesto(manifesto, candidate, fissati)
        valida_manifesto(manifesto)
        destinazione.write_bytes(manifesto_path.read_bytes())


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--run-id", default=os.environ.get("DISTRIBUTION_RUN_ID", ""))
    args = parser.parse_args()
    try:
        repo = os.environ["GITHUB_REPOSITORY"]
        if args.run_id and not re.fullmatch(r"[0-9]+", args.run_id):
            raise PreparazioneError("identificativo del run non valido")
        prepara(repo, args.run_id or None)
    except PreparazioneError as errore:
        print(f"ERRORE: {errore}", file=sys.stderr)
        return 1
    except (OSError, ValueError, KeyError, TypeError, AttributeError):
        print("ERRORE: dati necessari al manifesto assenti o non validi", file=sys.stderr)
        return 1
    print("Manifesto acquisito e confrontato con gli artefatti congelati")
    return 0


if __name__ == "__main__":
    sys.exit(main())
