# SPDX-License-Identifier: GPL-3.0-only
"""Run original C probe ABI implementation, not a Python reimplementation."""
import pathlib,shutil,subprocess,tempfile,unittest
ROOT=pathlib.Path(__file__).resolve().parents[2]
class BoundaryABI(unittest.TestCase):
 def test_real_c_swap_lifecycle_one_and_full_blocks(self):
  compiler=shutil.which('clang')
  if not compiler:self.skipTest('clang unavailable')
  with tempfile.TemporaryDirectory()as directory:
   d=pathlib.Path(directory);source=d/'test.c';binary=d/'test'
   source.write_text('#define RIBBON_MODE 0\n#include "'+str(ROOT/'fixtures/ribbon-boundary/plugin.c')+'"\n'+r'''
#include <assert.h>
int main(void) {
 assert(clap_entry.init("fixture"));
 const clap_plugin_factory_t *f=clap_entry.get_factory(CLAP_PLUGIN_FACTORY_ID);
 assert(f&&f->get_plugin_count(f)==1&&f->get_plugin_descriptor(f,1)==0);
 clap_host_t host={.clap_version=CLAP_VERSION_INIT};
 const clap_plugin_t *p=f->create_plugin(f,&host,descriptor.id);
 assert(p&&p->init(p)&&p->activate(p,48000,1,256)&&p->start_processing(p));
 assert(!p->activate(p,44100,1,256));
 float a[256],b[256],x[256],y[256];for(int i=0;i<256;i++){a[i]=(float)i/512;b[i]=-(float)i/1024;}
 float *in[]={a,b},*out[]={x,y};
 clap_audio_buffer_t ib={.data32=in,.channel_count=2},ob={.data32=out,.channel_count=2};
 clap_process_t process={.audio_inputs=&ib,.audio_outputs=&ob,.audio_inputs_count=1,.audio_outputs_count=1};
 for(int n=1;n<=256;n+=255){process.frames_count=n;assert(p->process(p,&process)==CLAP_PROCESS_CONTINUE);for(int i=0;i<n;i++){assert(x[i]==b[i]&&y[i]==a[i]);}}
 process.audio_inputs_count=0;assert(p->process(p,&process)==CLAP_PROCESS_ERROR);
 p->stop_processing(p);p->deactivate(p);p->destroy(p);clap_entry.deinit();return 0;
}
''')
   subprocess.run([compiler,'-std=c11','-I',str(ROOT/'vendor/clap-fixture/include'),str(source),'-o',str(binary)],check=True)
   subprocess.run([str(binary)],check=True)
 def test_real_read_write_probe_touches_encoded_parent_address(self):
  compiler=shutil.which('clang')
  if not compiler:self.skipTest('clang unavailable')
  with tempfile.TemporaryDirectory()as directory:
   d=pathlib.Path(directory)
   for mode in (3,4):
    source=d/f'test-{mode}.c';binary=d/f'test-{mode}'
    source.write_text('#define RIBBON_MODE '+str(mode)+'\n#include "'+str(ROOT/'fixtures/ribbon-boundary/plugin.c')+'"\n'+r'''
#include <assert.h>
_Alignas(4096) static unsigned char parent[4096];
int main(void) {
 parent[0]=0x37;uintptr_t address=(uintptr_t)parent;
 float a[256]={0},b[256]={0},x[256]={0},y[256]={0};
 for(int i=0;i<16;i++)a[i]=(float)((address>>(4*i))&15);
 float *in[]={a,b},*out[]={x,y};
 clap_audio_buffer_t ib={.data32=in,.channel_count=2},ob={.data32=out,.channel_count=2};
 clap_process_t block={.audio_inputs=&ib,.audio_outputs=&ob,.audio_inputs_count=1,.audio_outputs_count=1,.frames_count=256};
 assert(process(&plugin,&block)==CLAP_PROCESS_CONTINUE);
 for(int i=0;i<256;i++){assert(x[i]==13.0f&&y[i]==11.0f);}
#if RIBBON_MODE==3
 assert(parent[0]==0x37);
#else
 assert(parent[0]==0xa5);
#endif
 parent[0]=0x37;a[1]=0.5f;
 assert(process(&plugin,&block)==CLAP_PROCESS_ERROR);assert(parent[0]==0x37);
 return 0;
}
''')
    subprocess.run([compiler,'-std=c11','-I',str(ROOT/'vendor/clap-fixture/include'),str(source),'-o',str(binary)],check=True)
    subprocess.run([str(binary)],check=True)
if __name__=='__main__':unittest.main()
