# Fixture ostili

Marcatore: `ZZ-MARCATORE-PAYLOAD-9F3A-ZZ`

Ogni file porta il marcatore in chiaro. La proprieta' verificata e' che
**non compaia mai nella busta d'errore**: se compare, il payload e'
uscito.

I file sono byte-esatti e versionati, non generati a ogni corsa: una
fixture generata cambia con la libreria che la genera, e un test ostile
che cambia da solo non prova niente.

| Gruppo | Che cosa rompe |
|---|---|
| `apertura.*` | il file non e' nel formato dichiarato: `open` deve fallire |
| `lettura.*` | header valido, riga rotta: `open` riesce, `next_batch` deve fallire |

Le prove di scrittura non hanno file: la sorgente ostile e' il
`DataContract` passato a `create`, costruito in Rust.
