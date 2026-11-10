import pyarrow as pa,struct,json
from pathlib import Path
dest=Path(__file__).resolve().parent
def pack(fmt,*v):return pa.py_buffer(struct.pack(fmt,*v))
constructors={
 'list-offset-oob':lambda:pa.Array.from_buffers(pa.list_(pa.int32()),3,[None,pack('<4i',0,4,1,3)],children=[pa.array([1,2,3],pa.int32())]),
 'list-offset-negative':lambda:pa.Array.from_buffers(pa.list_(pa.int32()),3,[None,pack('<4i',0,-1,1,3)],children=[pa.array([1,2,3],pa.int32())]),
 'primitive-short':lambda:pa.Array.from_buffers(pa.int32(),3,[None,pack('<2i',1,2)]),
 'struct-children-different':lambda:pa.Array.from_buffers(pa.struct([('a',pa.int32()),('b',pa.int32())]),3,[None],children=[pa.array([1,2,3],pa.int32()),pa.array([1,2],pa.int32())]),
 'dictionary-oob':lambda:pa.DictionaryArray.from_arrays(pa.array([0,2,0],pa.int8()),pa.array(['x']),safe=False),
}
res=[]
for name,f in constructors.items():
 try:a=f()
 except Exception as e:res.append([name,'construction rejects',str(e)]);continue
 try:a.validate(full=True);v='valid'
 except Exception as e:v=str(e)
 b=pa.record_batch([a],names=['v'])
 for mode in ['file','stream']:
  sink=pa.BufferOutputStream()
  try:
   with (pa.ipc.new_file if mode=='file' else pa.ipc.new_stream)(sink,b.schema) as w:w.write_batch(b)
   (dest/f'{name}.{mode}').write_bytes(sink.getvalue().to_pybytes())
   res.append([name,mode,'serialized',v])
  except Exception as e:res.append([name,mode,'write rejects',str(e)])
print(json.dumps(res,indent=2))
# Additional structural corruptions in IPC metadata; body allocations remain intact.
def slot(data,table,index):
 vt=table-struct.unpack_from('<i',data,table)[0]
 return table+struct.unpack_from('<H',data,vt+4+2*index)[0]
def rec(data,start):
 while True:
  size=struct.unpack_from('<I',data,start+4)[0];m=start+8;t=m+struct.unpack_from('<I',data,m)[0]
  h=slot(data,t,2);header=h+struct.unpack_from('<I',data,h)[0]
  if data[slot(data,t,1)]==3:return header
  vt=t-struct.unpack_from('<i',data,t)[0];sz=struct.unpack_from('<H',data,vt)[0]
  off=struct.unpack_from('<H',data,vt+10)[0] if sz>10 else 0
  body=struct.unpack_from('<q',data,t+off)[0] if off else 0
  start=m+size+body
s=pa.StructArray.from_arrays([pa.array([1]*9,pa.int32())],names=['c'],mask=pa.array([True]+[False]*8))
extra=[('primitive-short',pa.array([1,2,3],pa.int32()),[(2,1,8,8)]),('struct-children-different',pa.StructArray.from_arrays([pa.array([1,2,3],pa.int32()),pa.array([4,5,6],pa.int32())],names=['a','b']),[(1,2,0,2),(2,4,8,8)]),('list-struct-short-bitmap',pa.ListArray.from_arrays([0,9],s),[(2,2,8,1)]),('struct-child-short-bitmap',pa.StructArray.from_arrays([pa.array([None]+[1]*8,pa.int32())],names=['c']),[(2,1,8,1)])]
for name,a,patches in extra:
 b=pa.record_batch([a],names=['v'])
 for mode in ['file','stream']:
  sink=pa.BufferOutputStream()
  with (pa.ipc.new_file if mode=='file' else pa.ipc.new_stream)(sink,b.schema) as w:w.write_batch(b)
  data=bytearray(sink.getvalue().to_pybytes());r=rec(data,8 if mode=='file' else 0)
  for field,index,within,value in patches:
   pos=slot(data,r,field);vec=pos+struct.unpack_from('<I',data,pos)[0]
   struct.pack_into('<q',data,vec+4+16*index+within,value)
  (dest/f'{name}.{mode}').write_bytes(data)
a=pa.array([[('a',1)],[('b',2)],[('c',3)]],type=pa.map_(pa.string(),pa.int32()))
b=pa.record_batch([a],names=['v'])
for mode in ['file','stream']:
 sink=pa.BufferOutputStream()
 with (pa.ipc.new_file if mode=='file' else pa.ipc.new_stream)(sink,b.schema) as w:w.write_batch(b)
 data=bytearray(sink.getvalue().to_pybytes());r=rec(data,8 if mode=='file' else 0)
 pos=slot(data,r,2);vec=pos+struct.unpack_from('<I',data,pos)[0]
 # first offsets buffer body offset; locate record message by scanning
 st=8 if mode=='file' else 0
 while True:
  size=struct.unpack_from('<I',data,st+4)[0];m=st+8;t=m+struct.unpack_from('<I',data,m)[0]
  if data[slot(data,t,1)]==3:break
  st=m+size
 body=m+size;off=struct.unpack_from('<q',data,vec+4+16)[0]
 struct.pack_into('<i',data,body+off+4,4)
 (dest/f'map-offset-oob.{mode}').write_bytes(data)

# REE: il run end fisico non copre la lunghezza logica dichiarata.
b=pa.record_batch([pa.RunEndEncodedArray.from_arrays(pa.array([257],pa.int32()),pa.array([1],pa.int32()))],names=['v'])
for mode in ['file','stream']:
 sink=pa.BufferOutputStream()
 with (pa.ipc.new_file if mode=='file' else pa.ipc.new_stream)(sink,b.schema) as w:w.write_batch(b)
 data=bytearray(sink.getvalue().to_pybytes());r=rec(data,8 if mode=='file' else 0)
 pos=slot(data,r,2);vec=pos+struct.unpack_from('<I',data,pos)[0]
 st=8 if mode=='file' else 0
 while True:
  size=struct.unpack_from('<I',data,st+4)[0];m=st+8;t=m+struct.unpack_from('<I',data,m)[0]
  if data[slot(data,t,1)]==3:break
  st=m+size
 body=m+size;off=struct.unpack_from('<q',data,vec+4+16)[0]
 struct.pack_into('<i',data,body+off,256)
 (dest/f'ree-short.{mode}').write_bytes(data)

# Campo 0 valido; gli offset corrotti stanno nella colonna esclusa.
a=constructors['list-offset-oob']()
b=pa.record_batch([pa.array([1,2,3],pa.int64()),a],names=['selected','excluded'])
sink=pa.BufferOutputStream()
with pa.ipc.new_file(sink,b.schema) as w:w.write_batch(b)
(dest/'projection-list.file').write_bytes(sink.getvalue().to_pybytes())
