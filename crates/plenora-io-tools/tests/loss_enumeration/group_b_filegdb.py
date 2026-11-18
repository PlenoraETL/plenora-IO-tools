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
arguments = [b'-f', b'OpenFileGDB', b'-nln', b'features', b'-lco', b'GEOMETRY_NAME=geometry']
argv = (C.c_char_p * (len(arguments) + 1))(*arguments, None)
options = bind('GDALVectorTranslateOptionsNew', ptr, chars, ptr)(argv, None)
usage = C.c_int()
output = bind('GDALVectorTranslate', ptr, C.c_char_p, ptr, C.c_int, C.POINTER(ptr), ptr, C.POINTER(C.c_int))(os.fsencode(sys.argv[2]), None, 1, (ptr * 1)(source), options, C.byref(usage))
assert output and not usage.value, 'native FileGDB generation failed'
close = bind('GDALClose', C.c_int, ptr)
assert close(output) == 0
close(source)
bind('GDALVectorTranslateOptionsFree', None, ptr)(options)
# Persist the independently obtained definition alongside a failed native fixture.
source = bind('GDALOpenEx', ptr, C.c_char_p, C.c_uint, chars, chars, chars)(os.fsencode(sys.argv[2]), 4, None, None, None)
layer = bind('GDALDatasetExecuteSQL', ptr, ptr, C.c_char_p, ptr, C.c_char_p)(source, b'GetLayerDefinition features', None, None)
feature = bind('OGR_L_GetNextFeature', ptr, ptr)(layer)
xml = bind('OGR_F_GetFieldAsString', C.c_char_p, ptr, C.c_int)(feature, 0)
Path(sys.argv[2]).with_suffix('.xml').write_bytes(xml)
bind('OGR_F_Destroy', None, ptr)(feature)
bind('GDALDatasetReleaseResultSet', None, ptr, ptr)(source, layer)
close(source)
