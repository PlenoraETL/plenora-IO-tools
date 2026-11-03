# AGENTS.md

Regole di lavoro per chi modifica questo repository, persone e agenti. Come
è fatto il codice e come si verifica sta in
[`docs/ENGINEERING.md`](docs/ENGINEERING.md); il prodotto e i suoi limiti in
[`docs/PRODUCT.md`](docs/PRODUCT.md); la procedura di rilascio in
[`docs/RELEASE.md`](docs/RELEASE.md).

## Le regole che non cambiano

1. **Prima si raccolgono i difetti, poi si corregge.** Per la logica critica
   il lavoro ha tre fasi. *Raccolta*: uno o più lettori indipendenti cercano
   su tutta la classe (ogni chiave, operazione, formato, percorso) senza
   correggere, fino a esaurirla. *Correzione unica*: chi corregge riceve
   l'elenco intero e lo chiude per costruzione (regola unica, tabella
   dichiarativa, test di enumerazione), non con toppe caso per caso.
   *Conferma*: un solo giro di lettura sull'elenco e sulle regressioni. Mai
   «trova, correggi, trova, correggi». Dopo sei giri di lettura che trovano
   ancora difetti ci si ferma, si cambia approccio e lo si dice al maintainer
   (regola del maintainer, 2026-11-03).
