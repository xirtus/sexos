# SPDX-License-Identifier: GPL-3.0-only
"""Independent every-sample oracle for consented source-built recovery effects."""
import hashlib,struct
FRAMES=1025
def verify(request,result,vst=False):
 def header(data,magic,count):
  if len(data)!=40+FRAMES*8+count*16 or data[:8]!=magic:raise ValueError('exact file extent/magic')
  nonce=data[8:24]
  if nonce==bytes(16)or struct.unpack_from('<4I',data,24)!=(48000,FRAMES,2,count):raise ValueError('header/nonce')
  return nonce
 count=int(vst)
 nonce=header(request,b'MOANREQ1',count)
 if header(result,b'MOANPCM1',0)!=nonce:raise ValueError('result not bound to request nonce')
 if vst and struct.unpack_from('<IId',request,40+FRAMES*8)!=(0,0,.25):raise ValueError('exact VST gain fixture parameter')
 for i in range(FRAMES):
  right=.5 if i%2==0 else -.25
  if struct.unpack_from('<2f',request,40+i*8)!=(.125,right):raise ValueError('original source PCM changed')
  expected=(.03125,right*.25)if vst else(right,.125)
  if struct.unpack_from('<2f',result,40+i*8)!=expected:raise ValueError(f'non-bypass exact output sample {i}')
 return dict(frames=FRAMES,format='vst3_gain_025'if vst else'clap_stereo_swap',request_sha256=hashlib.sha256(request).hexdigest(),result_sha256=hashlib.sha256(result).hexdigest(),every_sample_exact=True)
