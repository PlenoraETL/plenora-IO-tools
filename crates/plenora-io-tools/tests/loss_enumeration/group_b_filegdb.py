"""Build a native FileGDB through GDAL C API, never through IO."""
import ctypes as C
import os
from pathlib import Path
import sys

# GDAL runtime is supplied by the invoking verification environment.
bin_dir = next(Path(p) for p in os.environ['PATH'].split(os.pathsep) if (Path(p) / 'gdal.dll').exists())
handle = os.add_dll_directory(str(bin_dir))
gdal = C.CDLL(str(bin_dir / 'gdal.dll'))
def bind(name, result, *args):
    fn = getattr(gdal, name)
    fn.restype = result
    fn.argtypes = list(args)
    return fn
ptr = C.c_void_p
chars = C.POINTER(C.c_char_p)
bind('GDALAllRegister', None)()
source = bind('GDALOpenEx', ptr, C.c_char_p, C.c_uint, chars, chars, chars)(os.fsencode(sys.argv[1]), 4, None, None, None)
assert source, 'native GPKG fixture failed GDAL validation'
mode = sys.argv[4] if len(sys.argv) > 4 else ''
arguments = [b'-f', b'OpenFileGDB', b'-nln', b'features', b'-lco', b'GEOMETRY_NAME=geometry']
if mode in ('Subtypes', 'Domains'):
    arguments.extend([b'-mapFieldType', b'Integer64=Integer'])
if mode == 'SignedInteger':
    arguments.extend([b'-lco', b'TARGET_ARCGIS_VERSION=ARCGIS_PRO_3_2_OR_LATER'])
if mode == 'Hierarchy':
    arguments.extend([b'-lco', b'FEATURE_DATASET=NativeGroup'])
if mode == 'LayerMetadata':
    arguments.extend([b'-lco', b'DOCUMENTATION=<metadata><title>native-documentation</title></metadata>'])
if mode == 'Aliases':
    arguments.extend([b'-lco', b'LAYER_ALIAS=independent native alias'])
argv = (C.c_char_p * (len(arguments) + 1))(*arguments, None)
options = bind('GDALVectorTranslateOptionsNew', ptr, chars, ptr)(argv, None)
usage = C.c_int()
output = bind('GDALVectorTranslate', ptr, C.c_char_p, ptr, C.c_int, C.POINTER(ptr), ptr, C.POINTER(C.c_int))(os.fsencode(sys.argv[2]), None, 1, (ptr * 1)(source), options, C.byref(usage))
assert output and not usage.value, 'native FileGDB generation failed'
close = bind('GDALClose', C.c_int, ptr)
assert close(output) == 0
close(source)
bind('GDALVectorTranslateOptionsFree', None, ptr)(options)
if mode == 'Domains':
    dataset = bind('GDALOpenEx', ptr, C.c_char_p, C.c_uint, chars, chars, chars)(os.fsencode(sys.argv[2]), 5, None, None, None)
    class CodedValue(C.Structure):
        _fields_ = [('code', C.c_char_p), ('value', C.c_char_p)]
    values = (CodedValue * 2)(CodedValue(b'1', b'Road'), CodedValue(None, None))
    domain = bind('OGR_CodedFldDomain_Create', ptr, C.c_char_p, C.c_char_p, C.c_int, C.c_int, C.POINTER(CodedValue))(b'RoadType', b'Native road domain', 0, 0, values)
    reason = C.c_char_p()
    assert bind('GDALDatasetAddFieldDomain', C.c_bool, ptr, ptr, C.POINTER(C.c_char_p))(dataset, domain, C.byref(reason)), reason.value
    bind('OGR_FldDomain_Destroy', None, ptr)(domain)
    close(dataset)
# Modify the actual native catalog record, never a file adjacent to the GDB.
if mode in ('Subtypes', 'Domains', 'Extensions'):
    catalog = bind('GDALOpenEx', ptr, C.c_char_p, C.c_uint, chars, chars, chars)(os.fsencode(Path(sys.argv[2]) / 'a00000004.gdbtable'), 5, None, None, None)
    assert catalog, 'native catalog must be editable'
    table = bind('GDALDatasetGetLayer', ptr, ptr, C.c_int)(catalog, 0)
    next_feature = bind('OGR_L_GetNextFeature', ptr, ptr)
    field_index = bind('OGR_F_GetFieldIndex', C.c_int, ptr, C.c_char_p)
    get_string = bind('OGR_F_GetFieldAsString', C.c_char_p, ptr, C.c_int)
    destroy = bind('OGR_F_Destroy', None, ptr)
    changed = False
    while feature := next_feature(table):
        if get_string(feature, field_index(feature, b'Name')) == b'features':
            index = field_index(feature, b'Definition')
            definition = get_string(feature, index).decode()
            closing = '</typens:DEFeatureClassInfo>'
            if mode == 'Subtypes':
                addition = '<SubtypeFieldName>value</SubtypeFieldName><DefaultSubtypeCode>1</DefaultSubtypeCode><Subtypes><Subtype><SubtypeName>Road</SubtypeName><SubtypeCode>1</SubtypeCode></Subtype></Subtypes>'
                definition = definition.replace(closing, addition + closing)
            elif mode == 'Domains':
                definition = definition.replace('<Name>value</Name>', '<Name>value</Name><DomainName>RoadType</DomainName>', 1)
            else:
                definition = definition.replace(closing, '<ExtensionProperties><PropertySetProperty><Key>native-extension</Key><Value>payload</Value></PropertySetProperty></ExtensionProperties>' + closing)
            marker = {'Subtypes': 'Road', 'Domains': 'RoadType', 'Extensions': 'native-extension'}[mode]
            assert marker in definition, 'requested native definition must exist'
            bind('OGR_F_SetFieldString', None, ptr, C.c_int, C.c_char_p)(feature, index, definition.encode())
            assert bind('OGR_L_SetFeature', C.c_int, ptr, ptr)(table, feature) == 0
            changed = True
        destroy(feature)
    assert changed
    close(catalog)
# Persist the independently obtained definition alongside a failed native fixture.
source = bind('GDALOpenEx', ptr, C.c_char_p, C.c_uint, chars, chars, chars)(os.fsencode(sys.argv[2]), 4, None, None, None)
layer = bind('GDALDatasetExecuteSQL', ptr, ptr, C.c_char_p, ptr, C.c_char_p)(source, b'GetLayerDefinition features', None, None)
feature = bind('OGR_L_GetNextFeature', ptr, ptr)(layer)
xml = bind('OGR_F_GetFieldAsString', C.c_char_p, ptr, C.c_int)(feature, 0)
Path(sys.argv[2]).with_suffix('.xml').write_bytes(xml)
if mode in ('Subtypes', 'Domains', 'Extensions'):
    marker = {'Subtypes': b'Road', 'Domains': b'RoadType', 'Extensions': b'native-extension'}[mode]
    assert marker in xml, 'native catalog mutation did not survive reopen'
if len(sys.argv) > 4 and sys.argv[4] == 'Aliases':
    assert b'independent native alias' in xml, 'GDAL did not store the requested native alias'
bind('OGR_F_Destroy', None, ptr)(feature)
bind('GDALDatasetReleaseResultSet', None, ptr, ptr)(source, layer)
if len(sys.argv) > 3:
    # Independent native oracle after GDAL's storage normalization and before
    # IO reads the file. In particular polygons become native multipolygons.
    layer = bind('GDALDatasetGetLayer', ptr, ptr, C.c_int)(source, 0)
    srs = bind('OGR_L_GetSpatialRef', ptr, ptr)(layer)
    if srs:
        definition = C.c_char_p()
        assert bind('OSRExportToWkt', C.c_int, ptr, C.POINTER(C.c_char_p))(srs, C.byref(definition)) == 0
        Path(sys.argv[3] + '.wkt').write_bytes(definition.value)
    if mode == 'Bbox':
        envelope = (C.c_double * 4)()
        assert bind('OGR_L_GetExtent', C.c_int, ptr, ptr, C.c_int)(layer, envelope, 1) == 0
        assert list(envelope) == [1.0, 1.0, 2.0, 2.0]
    feature = bind('OGR_L_GetNextFeature', ptr, ptr)(layer)
    geometry = bind('OGR_F_GetGeometryRef', ptr, ptr)(feature)
    if geometry:
        size = bind('OGR_G_WkbSize', C.c_int, ptr)(geometry)
        buffer = C.create_string_buffer(size)
        assert bind('OGR_G_ExportToIsoWkb', C.c_int, ptr, C.c_int, ptr)(geometry, 1, buffer) == 0
        Path(sys.argv[3]).write_bytes(buffer.raw)
    bind('OGR_F_Destroy', None, ptr)(feature)
close(source)
