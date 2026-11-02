# Regressioni Excel

`excel-workbook-1904.xml` riproduce la struttura ridotta di `xl/workbook.xml`
di un file Excel per Mac reale pubblicata in
[calamine #706](https://github.com/tafia/calamine/issues/706).
Provenienza puntuale e adattamenti sono in `source.json`: dichiarazioni
namespace rese esplicite e foglio S rinominato Sheet1. Il blocco extLst,
il suo URI e x15:workbookPr sono copiati dalla riproduzione pubblicata.
Non si attribuisce il contenitore di test completo a Excel: viene creato
con rust_xlsxwriter 0.99.1, poi il workbook XML viene sostituito con questa
fixture. I test confrontano i valori Arrow e i quattro booleani XML.

Il tentativo locale di creare un nuovo workbook con Excel COM il 2026-11-02
e' fallito con 80070520 (sessione di accesso non disponibile).
Le formule TIME con millisecondi nei test sono OOXML con cache esplicita:
non attestano un ricalcolo locale con Excel. Il test sui float estremi
controlla la notazione esponenziale e il round-trip esatto con calamine;
l'apertura in Excel richiede ancora una sessione COM utilizzabile.
