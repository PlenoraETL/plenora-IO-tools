"""Sonde negative del gate anti-chiamata-nuda (FZ-0).

Un gate che non fallisce mai non e' un gate. Queste sonde costruiscono un
albero minimo conforme, verificano che passi, poi introducono un modo di
aggirare la prevalidazione per volta e verificano che venga intercettato.

L'albero e' finto apposta: provare le sonde mutando i file veri lascerebbe il
repository sporco se un test si interrompe a meta'.
"""

from __future__ import annotations

import tempfile
import unittest
from pathlib import Path

from scripts.check_prevalidazione_decoder import righe_di_test, verifica

IPC_CONFORME = """use arrow_ipc::reader::FileReader;

impl FormatDriver for IpcDriver {
    fn open(&self, source: Source, mut opts: ReadOptions) -> Result<Box<dyn Handle>> {
        let path = preflight_source(source, &mut opts)?;
        driver_common::prevalida_arrow::valida_file_ipc("arrow", &path)?;
        let reader = leggendo_arrow("arrow", || {
            FileReader::try_new(File::open(&path)?, None)
        })?;
        Ok(reader)
    }

    fn open_layer_reader(&self, request: &ReadRequest) -> Result<Box<dyn LayerReader>> {
        let path = self.path.clone();
        driver_common::prevalida_arrow::valida_file_ipc("arrow", &path)?;
        let reader = leggendo_arrow("arrow", move || {
            FileReader::try_new(File::open(&path)?, projection)
        })?;
        Ok(reader)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rilegge_cio_che_ha_scritto() {
        // Input costruito dal test: nessun dato non fidato da prevalidare.
        let schema = FileReader::try_new(File::open(output).unwrap(), None).unwrap();
        assert!(schema.is_ok());
    }
}
"""

PARQUET_CONFORME = """use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;

impl FormatDriver for GeoParquetDriver {
    fn open(&self, source: Source, mut opts: ReadOptions) -> Result<Box<dyn Handle>> {
        let path = percorso_verificato(source, &mut opts)?;
        valida_schema_arrow_incorporato(&path)?;
        let builder = leggendo_arrow("parquet", || {
            ParquetRecordBatchReaderBuilder::try_new(File::open(&path)?)
        })?;
        Ok(builder)
    }
}
"""

ALBERO = {
    "crates/driver-ipc/src/lib.rs": IPC_CONFORME,
    "crates/driver-geoparquet/src/lib.rs": PARQUET_CONFORME,
}


class SondeChiamataNuda(unittest.TestCase):
    def albero(self, sostituzioni: dict[str, str] | None = None) -> Path:
        radice = Path(tempfile.mkdtemp())
        self.addCleanup(self._rimuovi, radice)
        contenuti = dict(ALBERO)
        contenuti.update(sostituzioni or {})
        for relativo, testo in contenuti.items():
            percorso = radice / relativo
            percorso.parent.mkdir(parents=True, exist_ok=True)
            percorso.write_text(testo, encoding="utf-8")
        return radice

    @staticmethod
    def _rimuovi(radice: Path) -> None:
        for percorso in sorted(radice.rglob("*"), reverse=True):
            if percorso.is_file():
                percorso.unlink()
            else:
                percorso.rmdir()
        radice.rmdir()

    @staticmethod
    def violazioni(esito: list[str]) -> list[str]:
        return [voce for voce in esito if not voce.startswith("__esclusi__")]

    # --- l'albero conforme passa ----------------------------------------

    def test_albero_conforme_non_produce_violazioni(self) -> None:
        self.assertEqual(self.violazioni(verifica(self.albero())), [])

    def test_il_codice_di_test_e_riconosciuto(self) -> None:
        """L'esclusione dei test e' mirata, non un'amnistia sul file intero."""
        righe = righe_di_test(IPC_CONFORME)
        sorgenti = IPC_CONFORME.splitlines()
        dentro = {sorgenti[numero] for numero in righe}
        self.assertTrue(any("rilegge_cio_che_ha_scritto" in riga for riga in dentro))
        self.assertFalse(any("fn open(&self" in riga for riga in dentro))

    # --- modi di aggirare la prevalidazione -----------------------------

    def test_prevalidazione_assente(self) -> None:
        mutato = IPC_CONFORME.replace(
            '        driver_common::prevalida_arrow::valida_file_ipc("arrow", &path)?;\n'
            "        let reader = leggendo_arrow(\"arrow\", || {\n",
            "        let reader = leggendo_arrow(\"arrow\", || {\n",
            1,
        )
        esito = self.violazioni(verifica(self.albero({"crates/driver-ipc/src/lib.rs": mutato})))
        self.assertTrue(esito, "una costruzione senza prevalidazione deve fallire")

    def test_prevalidazione_dopo_la_costruzione(self) -> None:
        """Prevalidare dopo non serve: il panico avviene durante la costruzione."""
        mutato = IPC_CONFORME.replace(
            '        driver_common::prevalida_arrow::valida_file_ipc("arrow", &path)?;\n'
            "        let reader = leggendo_arrow(\"arrow\", || {\n"
            "            FileReader::try_new(File::open(&path)?, None)\n"
            "        })?;\n",
            "        let reader = leggendo_arrow(\"arrow\", || {\n"
            "            FileReader::try_new(File::open(&path)?, None)\n"
            "        })?;\n"
            '        driver_common::prevalida_arrow::valida_file_ipc("arrow", &path)?;\n',
            1,
        )
        esito = self.violazioni(verifica(self.albero({"crates/driver-ipc/src/lib.rs": mutato})))
        self.assertTrue(esito, "l'ordine deve contare")

    def test_prevalidazione_in_un_altra_funzione(self) -> None:
        """Verificare in `open` non copre `open_layer_reader`, che riapre il file."""
        mutato = IPC_CONFORME.replace(
            "    fn open_layer_reader(&self, request: &ReadRequest) -> Result<Box<dyn LayerReader>> {\n"
            "        let path = self.path.clone();\n"
            '        driver_common::prevalida_arrow::valida_file_ipc("arrow", &path)?;\n',
            "    fn open_layer_reader(&self, request: &ReadRequest) -> Result<Box<dyn LayerReader>> {\n"
            "        let path = self.path.clone();\n",
            1,
        )
        esito = self.violazioni(verifica(self.albero({"crates/driver-ipc/src/lib.rs": mutato})))
        self.assertTrue(esito, "ogni funzione che costruisce deve prevalidare")

    def test_costruzione_spostata_in_un_altra_crate(self) -> None:
        """Spostare la chiamata fuori dal driver non la mette al sicuro."""
        radice = self.albero(
            {
                "crates/plenora-io-cli/src/main.rs": "fn leggi() {\n"
                "    let reader = FileReader::try_new(File::open(&path)?, None)?;\n"
                "}\n"
            }
        )
        esito = self.violazioni(verifica(radice))
        self.assertTrue(esito, "la costruzione fuori perimetro deve fallire")

    def test_percorso_sparito(self) -> None:
        """Se il costruttore non c'e' piu', il gate va aggiornato, non ignorato."""
        mutato = IPC_CONFORME.replace("FileReader::try_new", "AltroLettore::apri")
        esito = self.violazioni(verifica(self.albero({"crates/driver-ipc/src/lib.rs": mutato})))
        self.assertTrue(esito, "un perimetro che non trova piu' nulla deve dirlo")

    def test_prevalidazione_solo_in_un_commento(self) -> None:
        mutato = IPC_CONFORME.replace(
            '        driver_common::prevalida_arrow::valida_file_ipc("arrow", &path)?;',
            "        // qui andrebbe valida_file_ipc, prima o poi",
            1,
        )
        esito = self.violazioni(verifica(self.albero({"crates/driver-ipc/src/lib.rs": mutato})))
        self.assertTrue(
            esito,
            "un commento che nomina la prevalidazione non e' una prevalidazione",
        )


if __name__ == "__main__":
    unittest.main()
