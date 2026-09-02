#!/usr/bin/env python3
"""I semi del target `shp_reader`, generati invece che committati e basta.

# Perche' un generatore e non tre blob

Un seme e' un file binario, e un binario committato senza il modo di
riprodurlo e' un artefatto che nessuno puo' rileggere: se domani il formato del
bundle cambia, o se un seme smette di raggiungere il parsing, non c'e' modo di
capire che cosa contenesse se non aprendolo con un editor esadecimale.

Qui i semi si **derivano**. Il generatore scrive uno Shapefile minimo valido —
header, record, tabella DBF — e lo impacchetta nella forma che il target
attende. `--verifica` ricontrolla che i semi sul disco coincidano byte a byte
con quelli che questo modulo produce oggi: un seme modificato a mano, o
lasciato indietro da un cambio di formato, e' rosso.

# Perche' servono semi validi

Il driver rifiuta prima di parsare le geometrie se il numero di record del DBF
non coincide con il numero di forme del `.shp`. Un blob casuale non soddisfa
quella condizione quasi mai: senza semi il fuzzer eserciterebbe l'apertura e
niente altro, e il target sarebbe una copertura dichiarata e non ottenuta.

Un seme valido gli da' il punto di partenza; le mutazioni fanno il resto, ed e'
proprio dalla forma valida che si raggiungono i rami di rifiuto interessanti —
quelli dove un campo dichiara piu' di quanto il file contenga.

# Che cosa questo modulo non fa

Non usa il writer del driver. Scrivere i semi con il codice che il target deve
esercitare li renderebbe validi **per costruzione**, anche il giorno in cui il
writer sbagliasse: il seme confermerebbe il difetto invece di rivelarlo. I byte
sono percio' costruiti qui, dalla specifica del formato, e la loro validita' e'
verificata dal driver che li legge — non da quello che li ha scritti.
"""

from __future__ import annotations

import argparse
import pathlib
import struct
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
SEMI = ROOT / "fuzz" / "seeds" / "shp_reader"

# L'intestazione del bundle, la stessa che `fuzz_targets/shp_reader.rs` decodifica.
INTESTAZIONE = struct.Struct(">HHH")

# Codice di file e versione dell'header Shapefile (ESRI 1998).
CODICE_FILE = 9994
VERSIONE = 1000

# I tipi di forma usati dai semi.
PUNTO = 1
POLILINEA = 3
MULTIPUNTO = 8

# dBase III senza memo: e' cio' che il lettore DBF del driver accetta, ed e' la
# forma piu' semplice che porti record veri.
VERSIONE_DBF = 0x03
FINE_INTESTAZIONE_DBF = 0x0D
FINE_FILE_DBF = 0x1A
NON_CANCELLATO = 0x20


def _intestazione_shp(lunghezza_totale: int, tipo: int, riquadro: tuple[float, ...]) -> bytes:
    """I 100 byte di testa: meta' big-endian e meta' little-endian, come da specifica."""
    testa = struct.pack(">i", CODICE_FILE) + b"\x00" * 20
    # La lunghezza e' in **parole da 16 bit**, non in byte: e' l'unita' del
    # formato, e sbagliarla e' il difetto piu' comune in uno Shapefile scritto
    # a mano.
    testa += struct.pack(">i", lunghezza_totale // 2)
    testa += struct.pack("<ii", VERSIONE, tipo)
    testa += struct.pack("<4d", *riquadro)
    testa += struct.pack("<4d", 0.0, 0.0, 0.0, 0.0)
    assert len(testa) == 100, len(testa)
    return testa


def _record(numero: int, contenuto: bytes) -> bytes:
    """Numero e lunghezza del record sono big-endian; il contenuto no."""
    return struct.pack(">ii", numero, len(contenuto) // 2) + contenuto


def _assembla(tipo: int, contenuti: list[bytes], riquadro: tuple[float, ...]):
    """`(.shp, .shx)` dai contenuti dei record.

    L'indice si costruisce **dagli stessi** contenuti del `.shp`: derivarlo a
    parte lo farebbe divergere al primo record di lunghezza diversa, e un
    indice che mente e' un difetto del seme, non del lettore.

    Serve al lettore per contare le forme **senza leggerle**. Senza `.shx` il
    driver non puo' confrontare in anticipo il numero di forme con il numero di
    record del DBF, e quel ramo di rifiuto resta irraggiungibile: un seme senza
    indice coprirebbe meno di quanto sembri.
    """
    corpo = b""
    voci = []
    offset = 50  # i 100 byte di intestazione sono 50 parole da 16 bit
    for numero, contenuto in enumerate(contenuti, 1):
        lunghezza = len(contenuto) // 2
        voci.append((offset, lunghezza))
        corpo += _record(numero, contenuto)
        offset += 4 + lunghezza  # 8 byte di testa del record sono 4 parole

    indice = b"".join(struct.pack(">ii", o, l) for o, l in voci)
    return (
        _intestazione_shp(100 + len(corpo), tipo, riquadro) + corpo,
        _intestazione_shp(100 + len(indice), tipo, riquadro) + indice,
    )


def shp_di_punti(punti: list[tuple[float, float]]):
    contenuti = [struct.pack("<i2d", PUNTO, x, y) for x, y in punti]
    riquadro = (
        min(x for x, _ in punti),
        min(y for _, y in punti),
        max(x for x, _ in punti),
        max(y for _, y in punti),
    )
    return _assembla(PUNTO, contenuti, riquadro)


def shp_di_polilinee(linee: list[list[tuple[float, float]]]):
    """Una parte per linea: la forma minima che esercita l'indice delle parti."""
    contenuti = []
    for vertici in linee:
        riquadro = (
            min(x for x, _ in vertici),
            min(y for _, y in vertici),
            max(x for x, _ in vertici),
            max(y for _, y in vertici),
        )
        contenuto = struct.pack("<i", POLILINEA)
        contenuto += struct.pack("<4d", *riquadro)
        contenuto += struct.pack("<ii", 1, len(vertici))
        contenuto += struct.pack("<i", 0)
        for x, y in vertici:
            contenuto += struct.pack("<2d", x, y)
        contenuti.append(contenuto)

    tutti = [vertice for vertici in linee for vertice in vertici]
    riquadro = (
        min(x for x, _ in tutti),
        min(y for _, y in tutti),
        max(x for x, _ in tutti),
        max(y for _, y in tutti),
    )
    return _assembla(POLILINEA, contenuti, riquadro)


def shp_di_multipunto(gruppi: list[list[tuple[float, float]]]):
    """La terza famiglia di geometria del formato.

    Un multipunto dichiara **solo** il numero di punti: niente indice delle
    parti. E' una forma di record diversa sia dal punto -- che non dichiara
    conteggi -- sia dalla polilinea, e percorre nel driver un ramo di
    prevalidazione che nessun'altra geometria raggiunge.
    """
    contenuti = []
    for punti in gruppi:
        riquadro = (
            min(x for x, _ in punti),
            min(y for _, y in punti),
            max(x for x, _ in punti),
            max(y for _, y in punti),
        )
        contenuto = struct.pack("<i", MULTIPUNTO)
        contenuto += struct.pack("<4d", *riquadro)
        contenuto += struct.pack("<i", len(punti))
        for x, y in punti:
            contenuto += struct.pack("<2d", x, y)
        contenuti.append(contenuto)

    tutti = [vertice for punti in gruppi for vertice in punti]
    riquadro = (
        min(x for x, _ in tutti),
        min(y for _, y in tutti),
        max(x for x, _ in tutti),
        max(y for _, y in tutti),
    )
    return _assembla(MULTIPUNTO, contenuti, riquadro)


def dbf_con_record_cancellato(tabella: bytes) -> bytes:
    """Marca cancellato il primo record, lasciandone intatto il contenuto.

    Serve a una prova accoppiata: lo **stesso** valore ostile, con e senza il
    marcatore, deve dare lo stesso esito.

    Qui c'era scritto il contrario -- «`dbase` salta i byte di una riga
    cancellata senza decodificarne un campo» -- e su quella frase poggiava
    l'esenzione della prevalidazione. Non li salta. La fuzz smoke ha trovato una
    riga cancellata il cui campo `D` fa panicare `Date::from_str` attraversando
    l'apertura del driver, e la frase e' caduta insieme all'esenzione.
    """
    grezzo = bytearray(tabella)
    (inizio,) = struct.unpack("<H", bytes(grezzo[8:10]))
    grezzo[inizio] = 0x2A  # '*'
    return bytes(grezzo)


def dbf(campi: list[tuple[str, str, int]], righe: list[list[str]]) -> bytes:
    """Tabella dBase III. `campi` e' `(nome, tipo, larghezza)`.

    Il nome sta in undici byte con terminatore nullo: undici caratteri pieni
    non lascerebbero spazio al terminatore, ed e' un caso che un lettore
    prudente rifiuta.
    """
    lunghezza_intestazione = 32 + 32 * len(campi) + 1
    lunghezza_record = 1 + sum(larghezza for _, _, larghezza in campi)

    testa = struct.pack("<B3B", VERSIONE_DBF, 95, 1, 1)
    testa += struct.pack("<I", len(righe))
    testa += struct.pack("<HH", lunghezza_intestazione, lunghezza_record)
    testa += b"\x00" * 20

    for nome, tipo, larghezza in campi:
        grezzo = nome.encode("ascii")
        assert len(grezzo) <= 10, nome
        testa += grezzo + b"\x00" * (11 - len(grezzo))
        testa += tipo.encode("ascii")
        testa += b"\x00" * 4
        testa += struct.pack("<BB", larghezza, 0)
        testa += b"\x00" * 14
    testa += bytes([FINE_INTESTAZIONE_DBF])
    assert len(testa) == lunghezza_intestazione, (len(testa), lunghezza_intestazione)

    corpo = b""
    for riga in righe:
        assert len(riga) == len(campi), riga
        corpo += bytes([NON_CANCELLATO])
        for valore, (_, tipo, larghezza) in zip(riga, campi):
            grezzo = valore.encode("ascii")
            assert len(grezzo) <= larghezza, (valore, larghezza)
            # I numerici si allineano a destra, il testo a sinistra: e' la
            # convenzione che i lettori DBF si aspettano.
            riempito = (
                grezzo.rjust(larghezza) if tipo == "N" else grezzo.ljust(larghezza)
            )
            corpo += riempito
    return testa + corpo + bytes([FINE_FILE_DBF])


def shx_ostile(indice: bytes, scostamento: int) -> bytes:
    """Un indice il cui primo scostamento non regge il raddoppio.

    `shapefile` calcola la posizione del record come `offset * 2` dentro un
    `i32`: oltre meta' di `i32::MAX` il prodotto trabocca, e il processo panica
    invece di rifiutare il file. E' il secondo difetto che il target ha trovato.
    """
    grezzo = bytearray(indice)
    grezzo[100:104] = struct.pack(">i", scostamento)
    return bytes(grezzo)


def shp_con_conteggio_di_punti(shp: bytes, punti: int) -> bytes:
    """Falsifica `num_points` del primo record, che dev'essere una polilinea.

    `shapefile` prenota un vettore grande quanto il conteggio **prima** di
    leggere i punti: un record da poche decine di byte che ne dichiara un
    miliardo fa tentare un'allocazione da decine di gigabyte, e un conteggio
    negativo diventa un numero enorme passando da `as usize`.

    Lo scostamento e' fisso: cento byte di intestazione, otto di testa del
    record, quattro di tipo, trentadue di riquadro e quattro di `num_parts`.
    """
    grezzo = bytearray(shp)
    grezzo[148:152] = struct.pack("<i", punti)
    return bytes(grezzo)


def shp_con_indice_di_parti(shp: bytes, inizio: int) -> bytes:
    """Falsifica la prima voce dell'indice delle parti di una polilinea.

    `PartIndexIter` passa a `read_xy_in_vec_of` la differenza fra due voci
    consecutive: una voce che scende la rende negativa -- c'e' un
    `debug_assert!` che lo dice, quindi un panico sotto il fuzzer e niente in
    release, dove il numero negativo diventa enorme passando da `as usize` --
    e una che sale oltre il numero di punti fa lo stesso senza nemmeno
    l'asserzione.

    Lo scostamento e' fisso: cento byte di intestazione, otto di testa del
    record, quattro di tipo, trentadue di riquadro, quattro di `num_parts` e
    quattro di `num_points`.
    """
    grezzo = bytearray(shp)
    grezzo[152:156] = struct.pack("<i", inizio)
    return bytes(grezzo)


def dbf_con_campo_corto(tabella: bytes, tipo: bytes, larghezza: int) -> bytes:
    """Cambia tipo e larghezza del primo descrittore di campo.

    `dbase` dichiara la dimensione fissa dei propri tipi in `FieldType::size()`
    e **non la verifica**: legge comunque `field_bytes[0]` da un logico o
    `field_bytes[..4]` da un intero, e la fetta esce dal campo.

    Il primo descrittore comincia al byte 32: il tipo sta al 43, la larghezza
    al 48.
    """
    grezzo = bytearray(tabella)
    grezzo[43:44] = tipo
    grezzo[48] = larghezza
    return bytes(grezzo)


def dbf_con_data_ostile(tabella: bytes, valore: bytes) -> bytes:
    """Sostituisce il valore del primo campo del primo record con byte grezzi.

    `Date::from_str` di `dbase` affetta la stringa a byte -- `s[0..4]`,
    `s[4..6]`, `s[6..8]` -- senza guardare ne' la lunghezza ne' i confini di
    carattere: un valore piu' corto di otto byte utili esce dall'intervallo, uno
    con un carattere multibyte cade dentro di esso. Sono due panici, non due
    errori di parsing, e per raggiungerli serve un valore che nessuna stringa
    ASCII potrebbe esprimere.
    """
    if len(valore) != 8:
        raise ValueError("il campo data e' largo otto byte")
    grezzo = bytearray(tabella)
    (inizio,) = struct.unpack("<H", bytes(grezzo[8:10]))
    # Un byte di flag di cancellazione, poi il primo campo.
    grezzo[inizio + 1 : inizio + 9] = valore
    return bytes(grezzo)


def dbf_ostile(
    tabella: bytes,
    *,
    offset: int | None = None,
    versione: int | None = None,
    terminatore: int | None = None,
) -> bytes:
    """Una tabella DBF con un campo dell'intestazione **falsificato**.

    I tre punti in cui `dbase::File::open` si ferma invece di tornare:

    * `offset` -- il numero di campi si ricava da `offset_to_first_record` con
      una sottrazione non controllata;
    * `versione` -- per i file dichiarati Visual FoxPro ne fa prima una seconda
      sui 263 byte di backlink;
    * `terminatore` -- dopo i descrittori pretende `0x0D` con un
      `debug_assert_eq!`, che panica sotto il fuzzer e sparisce in release.

    Il seme parte da una tabella **valida** e ne cambia un byte o due: cosi' il
    percorso resta quello del DBF vero fino al punto esatto che si vuole
    esercitare, invece di fermarsi prima su un'altra incoerenza.
    """
    grezzo = bytearray(tabella)
    if offset is not None:
        grezzo[8:10] = struct.pack("<H", offset)
    if versione is not None:
        grezzo[0] = versione
    if terminatore is not None:
        # Il terminatore chiude i descrittori, cioe' sta all'ultimo byte
        # dell'intestazione dichiarata dalla tabella valida di partenza.
        (dichiarato,) = struct.unpack("<H", bytes(grezzo[8:10]))
        grezzo[dichiarato - 1] = terminatore
    return bytes(grezzo)


def bundle(shp: bytes, indice: bytes, tabella: bytes, prj: bytes = b"") -> bytes:
    """Impacchetta come il target si aspetta di trovare."""
    for parte in (shp, indice, tabella):
        if len(parte) > 0xFFFF:
            raise ValueError("un seme non deve dichiarare piu' di quanto un u16 porti")
    return (
        INTESTAZIONE.pack(len(shp), len(indice), len(tabella))
        + shp
        + indice
        + tabella
        + prj
    )


# Il `.prj` di WGS84 nella forma che `authority_id_from_wkt` sa leggere: serve a
# esercitare la strada in cui il CRS **viene dal file** invece che da
# `assume_crs`.
PRJ_WGS84 = (
    'GEOGCS["WGS 84",DATUM["WGS_1984",SPHEROID["WGS 84",6378137,298.257223563]],'
    'PRIMEM["Greenwich",0],UNIT["degree",0.0174532925199433],AUTHORITY["EPSG","4326"]]'
).encode("ascii")


def semi() -> dict[str, bytes]:
    """I semi, con il nome che dice che cosa ciascuno raggiunge."""
    punti, indice_punti = shp_di_punti([(11.25, 43.77), (12.49, 41.90)])
    attributi = dbf(
        [("NOME", "C", 8), ("POP", "N", 6)],
        [["FIRENZE", "  3800"], ["ROMA", "  2870"]],
    )
    linee, indice_linee = shp_di_polilinee([[(0.0, 0.0), (1.0, 1.0), (2.0, 0.5)]])
    una_riga = dbf([("NOME", "C", 8)], [["TRATTA"]])
    una_data = dbf([("QUANDO", "D", 8)], [["20260101"]])
    data_ostile = dbf_con_data_ostile(una_data, b"2026\xc3\xa801")
    multipunto, indice_multipunto = shp_di_multipunto([[(9.19, 45.46), (11.34, 44.49)]])
    # Un campo `T`: otto byte binari, giorno giuliano e millisecondi. Il valore
    # di partenza e' valido; i semi ostili ne cambiano gli otto byte.
    una_data_e_ora = dbf([("ISTANTE", "T", 8)], [["        "]])

    # Il tag di una polilinea, e nient'altro: due parole di record dove il tipo
    # ne pretende ventidue.
    troncato = _assembla(POLILINEA, [struct.pack("<i", POLILINEA)], (0.0, 0.0, 1.0, 1.0))

    # Un record lungo **esattamente** la propria testa, che dichiara una parte.
    # I quattro byte dell'indice delle parti non ci stanno: la lettura uscirebbe
    # dal record, e la posizione nel file resterebbe indietro di quattro byte
    # per tutti i record successivi.
    testa_esatta = _assembla(
        POLILINEA,
        [
            struct.pack("<i", POLILINEA)
            + struct.pack("<4d", 0.0, 0.0, 1.0, 1.0)
            + struct.pack("<ii", 1, 0)
        ],
        (0.0, 0.0, 1.0, 1.0),
    )

    # Il conteggio del DBF e quello del `.shp` **non** coincidono. Con l'indice
    # il driver se ne accorge all'apertura, prima di decodificare una sola
    # geometria; senza indice lo scopre durante la lettura. Sono due rami
    # diversi dello stesso rifiuto, e ci vogliono due semi per raggiungerli
    # entrambi.
    disallineato = dbf([("NOME", "C", 8)], [["SOLO_UNA"]])

    return {
        "punti-con-attributi.bundle": bundle(punti, indice_punti, attributi),
        "punti-con-prj.bundle": bundle(punti, indice_punti, attributi, PRJ_WGS84),
        "polilinea.bundle": bundle(linee, indice_linee, una_riga),
        # La terza famiglia di geometria: dichiara i punti e non le parti, e nel
        # driver percorre un ramo che ne' il punto ne' la polilinea raggiungono.
        "multipunto.bundle": bundle(multipunto, indice_multipunto, una_riga),
        "disallineati-con-indice.bundle": bundle(punti, indice_punti, disallineato),
        "disallineati-senza-indice.bundle": bundle(punti, b"", disallineato),
        # I tre punti di arresto di `dbase::File::open`, uno per ramo. Restano semi anche
        # ora che sono chiusi: un seme di regressione vale quanto uno che apre
        # una strada, e il replay e' il posto dove una correzione si dimostra.
        "dbf-offset-corto.bundle": bundle(
            punti, indice_punti, dbf_ostile(attributi, offset=10)
        ),
        "dbf-visual-foxpro-corto.bundle": bundle(
            punti, indice_punti, dbf_ostile(attributi, offset=100, versione=0x31)
        ),
        "dbf-terminatore-non-valido.bundle": bundle(
            punti, indice_punti, dbf_ostile(attributi, terminatore=0x00)
        ),
        # Un intero largo due byte: `dbase` ne legge quattro.
        "dbf-campo-corto.bundle": bundle(
            punti, indice_punti, dbf_con_campo_corto(attributi, b"I", 2)
        ),
        # I due punti di arresto di `shapefile`: lo scostamento dell'indice che
        # non regge il raddoppio, e il conteggio di punti che non e' legato alla
        # dimensione del record.
        "shx-scostamento-traboccante.bundle": bundle(
            punti, shx_ostile(indice_punti, 0x7FFF_FFF0), attributi
        ),
        "shx-scostamento-fuori-dal-shp.bundle": bundle(
            punti, shx_ostile(indice_punti, 5_000), attributi
        ),
        # Uno scostamento che regge il raddoppio, sta dentro il file e punta in
        # mezzo al contenuto del primo record: li' otto byte qualunque
        # diventano una testa di record. E' il caso che la sola catena
        # sequenziale non vede, perche' il lettore con indice non la percorre.
        "shx-scostamento-dentro-un-record.bundle": bundle(
            punti, shx_ostile(indice_punti, 56), attributi
        ),
        # Il valore di un campo data, che e' l'unico tipo il cui **contenuto**
        # -- non il descrittore -- puo' far panicare il lettore.
        "dbf-data-multibyte.bundle": bundle(linee, indice_linee, data_ostile),
        # Lo **stesso** valore, in una riga cancellata: l'esito dev'essere lo
        # stesso. La coppia era li' a dimostrare il contrario, e dimostrava una
        # premessa falsa.
        "dbf-data-multibyte-cancellata.bundle": bundle(
            linee, indice_linee, dbf_con_record_cancellato(data_ostile)
        ),
        "dbf-data-corta.bundle": bundle(
            linee, indice_linee, dbf_con_data_ostile(una_data, b"2026    ")
        ),
        # Il seme che la fuzz smoke ha ridotto: una data di quattro cifre utili
        # in una riga cancellata. `Date::from_str` affetta `s[4..6]` su una
        # stringa lunga uno e panica -- «end byte index 4 is out of bounds for
        # string of length 1» -- e il marcatore non lo impediva affatto.
        #
        # Differisce da `dbf-data-corta.bundle` per **un byte**, e cosi' dice
        # quale proprieta' misura: non «una data corta e' rifiutata», che si sa
        # gia', ma «il marcatore di cancellazione non compra l'esenzione».
        "dbf-data-corta-in-riga-cancellata.bundle": bundle(
            linee,
            indice_linee,
            dbf_con_record_cancellato(dbf_con_data_ostile(una_data, b"2026    ")),
        ),
        # Un record che dichiara una polilinea e porta solo il tag: i conteggi
        # non stanno dentro il record, e il decoder li legge dai byte che
        # seguono. E' cosi' che una campagna ha chiesto quattro gigabyte per un
        # file da trecento byte.
        "shp-record-troppo-corto.bundle": bundle(*troncato, una_riga),
        "shp-parti-oltre-la-testa.bundle": bundle(*testa_esatta, una_riga),
        # L'indice delle parti, nei due modi in cui puo' uscire dai punti che il
        # record dichiara.
        "shp-parti-che-scendono.bundle": bundle(
            shp_con_indice_di_parti(linee, -1), indice_linee, una_riga
        ),
        "shp-parti-oltre-i-punti.bundle": bundle(
            shp_con_indice_di_parti(linee, 99), indice_linee, una_riga
        ),
        # Il campo `T`: il giorno giuliano entra in un'aritmetica `i32` che
        # trabocca, e il parola-tempo negativo trabocca passando da `u32`.
        "dbf-giorno-giuliano-enorme.bundle": bundle(
            linee,
            indice_linee,
            dbf_con_data_ostile(una_data_e_ora, struct.pack("<ii", 2_000_000_000, 0)),
        ),
        "dbf-parola-tempo-negativa.bundle": bundle(
            linee,
            indice_linee,
            dbf_con_data_ostile(una_data_e_ora, struct.pack("<ii", 2_458_685, -1)),
        ),
        "shp-punti-assurdi.bundle": bundle(
            shp_con_conteggio_di_punti(linee, 1 << 30), indice_linee, una_riga
        ),
        "shp-punti-negativi.bundle": bundle(
            shp_con_conteggio_di_punti(linee, -1), indice_linee, una_riga
        ),
    }


def _dove() -> str:
    """Il percorso dei semi, relativo alla radice quando ci sta dentro.

    Le sonde spostano `SEMI` in una directory temporanea per provare i casi
    rossi: un `relative_to` incondizionato le farebbe fallire nel *print*, cioe'
    fuori da cio' che stanno verificando.
    """
    try:
        return SEMI.relative_to(ROOT).as_posix()
    except ValueError:
        return SEMI.as_posix()


def scrivi() -> int:
    SEMI.mkdir(parents=True, exist_ok=True)
    prodotti = semi()
    for nome, contenuto in sorted(prodotti.items()):
        (SEMI / nome).write_bytes(contenuto)
    print(f"{len(prodotti)} semi scritti in {_dove()}")
    return 0


def verifica() -> int:
    """I semi sul disco sono quelli che questo modulo produce oggi."""
    prodotti = semi()
    errori: list[str] = []

    presenti = {p.name for p in SEMI.glob("*")} if SEMI.exists() else set()
    for extra in sorted(presenti - set(prodotti)):
        errori.append(
            f"{extra}: seme non prodotto da questo generatore. Un binario che "
            "nessuno sa riprodurre non si puo' rileggere."
        )
    for nome, atteso in sorted(prodotti.items()):
        percorso = SEMI / nome
        if not percorso.exists():
            errori.append(f"{nome}: seme assente; si rigenera con `--scrivi`")
        elif percorso.read_bytes() != atteso:
            errori.append(
                f"{nome}: differisce da cio' che il generatore produce. O il "
                "seme e' stato modificato a mano, o il formato del bundle e' "
                "cambiato senza rigenerarlo."
            )

    for messaggio in errori:
        print(messaggio, file=sys.stderr)
    if errori:
        return 1
    print(f"semi di shp_reader verificati: {len(prodotti)}, tutti riproducibili.")
    return 0


def main(argv: list[str] | None = None) -> int:
    argomenti = argparse.ArgumentParser(description=__doc__)
    modo = argomenti.add_mutually_exclusive_group()
    modo.add_argument("--scrivi", action="store_true", help="rigenera i semi sul disco")
    modo.add_argument(
        "--verifica",
        action="store_true",
        help="i semi sul disco coincidono con quelli generati (predefinito)",
    )
    opzioni = argomenti.parse_args(argv)
    return scrivi() if opzioni.scrivi else verifica()


if __name__ == "__main__":
    sys.exit(main())
