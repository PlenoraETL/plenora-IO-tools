"""Reproducible semantic FileGDB fixture, using the same GDAL C API as the loss matrix.

No IO writer participates in construction. GDAL generates catalog UUIDs, so
reproducibility concerns decoded values, not UUID bytes. No network or osgeo wheel.
"""
import ctypes as C
from ctypes.util import find_library
import os
from pathlib import Path
import sys


def generate(destination: Path, *, empty: bool = False) -> None:
    if os.name == "nt":
        directory = next(Path(p) for p in os.environ["PATH"].split(os.pathsep)
                         if (Path(p) / "gdal.dll").is_file())
        dll_directory = os.add_dll_directory(str(directory))
        library = C.CDLL(str(directory / "gdal.dll"))
    else:
        library = C.CDLL(find_library("gdal"))

    def bind(name, result, *arguments):
        function = getattr(library, name)
        function.restype, function.argtypes = result, list(arguments)
        return function

    pointer = C.c_void_p
    strings = C.POINTER(C.c_char_p)
    bind("GDALAllRegister", None)()
    driver = bind("GDALGetDriverByName", pointer, C.c_char_p)(b"OpenFileGDB")
    dataset = bind("GDALCreate", pointer, pointer, C.c_char_p, C.c_int, C.c_int,
                   C.c_int, C.c_int, strings)(driver, os.fsencode(destination), 0, 0, 0, 0, None)
    assert dataset
    close = bind("GDALClose", C.c_int, pointer)
    try:
        class Coded(C.Structure):
            _fields_ = [("code", C.c_char_p), ("description", C.c_char_p)]

        class Scalar(C.Union):
            _fields_ = [("integer", C.c_int), ("real", C.c_double), ("padding", C.c_byte * 32)]

        coded_name, range_name = "stato_è", "intervallo_量"
        if not empty:
            entries = (Coded * 3)(Coded(b"1", "Attivo è".encode()),
                                  Coded(b"2", "Chiuso 水".encode()), Coded(None, None))
            coded = bind("OGR_CodedFldDomain_Create", pointer, C.c_char_p, C.c_char_p,
                         C.c_int, C.c_int, C.POINTER(Coded))(
                             coded_name.encode(), b"", 0, 0, entries)
            minimum, maximum = Scalar(), Scalar()
            minimum.real, maximum.real = -12.5, 105.25
            interval = bind("OGR_RangeFldDomain_Create", pointer, C.c_char_p, C.c_char_p,
                            C.c_int, C.c_int, pointer, C.c_bool, pointer, C.c_bool)(
                                range_name.encode(), b"", 2, 0, C.byref(minimum), True,
                                C.byref(maximum), True)
            for domain in [coded, interval]:
                reason = C.c_char_p()
                assert bind("GDALDatasetAddFieldDomain", C.c_bool, pointer, pointer,
                            C.POINTER(C.c_char_p))(dataset, domain, C.byref(reason))
                bind("OGR_FldDomain_Destroy", None, pointer)(domain)
        options = (C.c_char_p * 2)("LAYER_ALIAS=Strade città".encode(), None)
        srs = bind("OSRNewSpatialReference", pointer, C.c_char_p)(None)
        assert bind("OSRImportFromEPSG", C.c_int, pointer, C.c_int)(srs, 4326) == 0
        layer = bind("GDALDatasetCreateLayer", pointer, pointer, C.c_char_p, pointer,
                     C.c_int, strings)(dataset, b"features", srs, 1, options)
        bind("OSRDestroySpatialReference", None, pointer)(srs)
        assert layer
        for name, kind, domain, alias in [
            ("stato", 0, coded_name, "Stato città"),
            ("misura", 2, range_name, "Misura 水"),
        ]:
            field = bind("OGR_Fld_Create", pointer, C.c_char_p, C.c_int)(name.encode(), kind)
            if not empty:
                bind("OGR_Fld_SetDomainName", None, pointer, C.c_char_p)(field, domain.encode())
                bind("OGR_Fld_SetAlternativeName", None, pointer, C.c_char_p)(field, alias.encode())
            assert bind("OGR_L_CreateField", C.c_int, pointer, pointer, C.c_int)(layer, field, 0) == 0
            bind("OGR_Fld_Destroy", None, pointer)(field)
        definition = bind("OGR_L_GetLayerDefn", pointer, pointer)(layer)
        feature = bind("OGR_F_Create", pointer, pointer)(definition)
        bind("OGR_F_SetFieldInteger", None, pointer, C.c_int, C.c_int)(feature, 0, 1)
        bind("OGR_F_SetFieldDouble", None, pointer, C.c_int, C.c_double)(feature, 1, 2.5)
        geometry = bind("OGR_G_CreateGeometry", pointer, C.c_int)(1)
        bind("OGR_G_SetPoint_2D", None, pointer, C.c_int, C.c_double, C.c_double)(geometry, 0, 1.0, 2.0)
        assert bind("OGR_F_SetGeometryDirectly", C.c_int, pointer, pointer)(feature, geometry) == 0
        assert bind("OGR_L_CreateFeature", C.c_int, pointer, pointer)(layer, feature) == 0
        bind("OGR_F_Destroy", None, pointer)(feature)
    finally:
        close(dataset)
    if not empty:
        # GDAL exposes subtype definitions through GetLayerDefinition, but has no
        # public subtype creation API. Edit the native catalog, as the loss matrix does.
        catalog = bind("GDALOpenEx", pointer, C.c_char_p, C.c_uint, strings, strings, strings)(
            os.fsencode(destination / "a00000004.gdbtable"), 5, None, None, None)
        assert catalog
        table = bind("GDALDatasetGetLayer", pointer, pointer, C.c_int)(catalog, 0)
        field_index = bind("OGR_F_GetFieldIndex", C.c_int, pointer, C.c_char_p)
        get_string = bind("OGR_F_GetFieldAsString", C.c_char_p, pointer, C.c_int)
        changed = False
        while feature := bind("OGR_L_GetNextFeature", pointer, pointer)(table):
            if get_string(feature, field_index(feature, b"Name")) == b"features":
                index = field_index(feature, b"Definition")
                xml = get_string(feature, index).decode()
                end = "</typens:DEFeatureClassInfo>"
                assert end in xml
                addition = ('<SubtypeFieldName>stato</SubtypeFieldName><DefaultSubtypeCode>1</DefaultSubtypeCode>'
                            '<Subtypes><Subtype><SubtypeCode>1</SubtypeCode><SubtypeName>Strada città</SubtypeName>'
                            '<FieldInfos><SubtypeFieldInfo><FieldName>misura</FieldName>'
                            f'<DomainName>{range_name}</DomainName></SubtypeFieldInfo></FieldInfos>'
                            '</Subtype></Subtypes>')
                xml = xml.replace(end, addition + end)
                bind("OGR_F_SetFieldString", None, pointer, C.c_int, C.c_char_p)(feature, index, xml.encode())
                assert bind("OGR_L_SetFeature", C.c_int, pointer, pointer)(table, feature) == 0
                changed = True
            bind("OGR_F_Destroy", None, pointer)(feature)
        close(catalog)
        assert changed
    if os.name == "nt":
        dll_directory.close()


if __name__ == "__main__":
    generate(Path(sys.argv[1]), empty="--empty" in sys.argv[2:])
