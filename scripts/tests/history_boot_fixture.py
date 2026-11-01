"""Independent current nineteen-subject BOOT fixture for history proof tests."""
ROWS=[(9,156,9,1,0,38,41,11,0),(12,154,12,1,0,39,42,14,0),(15,300,15,1,0,40,43,43,0),(16,301,9,2,9,45,46,47,1),(17,302,12,2,12,50,51,52,1),(18,307,9,3,16,54,56,57,1),(19,308,9,4,18,110,111,113,1),(20,309,9,5,19,114,115,117,1),(21,310,9,6,20,118,119,121,1),(22,311,9,7,21,122,123,125,1),(23,312,9,8,22,126,127,129,1),(24,313,12,3,17,130,131,133,1),(25,318,12,4,24,134,135,137,1),(26,323,12,5,25,138,139,141,1),(27,328,12,6,26,142,143,145,1),(28,333,12,7,27,146,147,149,1),(29,338,12,8,28,150,151,153,1),(30,343,15,2,15,154,155,157,1),(31,344,15,3,30,158,159,161,1)]
def boot(tasks=None):
 tasks={}if tasks is None else tasks
 text='[boot.history.recovery] holder=11 slot=53 logical=12 participants=8 version=1 base=1808 sectors=18 held_query_verified=1 runtime_mint=0\n'
 for i,pd in enumerate([12,17,24,25,26,27,28,29]):text+=f'[boot.history.participant] index={i} target={pd} task={tasks.get(pd,pd)} holder=11 immutable=1 runtime_mint=0\n'
 for pd,surface,logical,inc,pred,life,close,route,dormant in ROWS:
  text+=f'[boot.app.catalog] target={pd} task={tasks.get(pd,pd)} surface={surface} logical={logical} incarnation={inc} predecessor={pred} lifecycle={life} close={close} route={route} dormant={dormant} grants_verified=1 data_only=1\n'
 return text+'[boot.app.catalog.complete] rows=19 actual_grants=1 runtime_mint=0\n[boot.memory.authority.sealed] runtime_alloc=0 runtime_map=capability_leaf runtime_mint=0\n[bootgraph.boot.handoff] target=sexdisplay id=1 entry=0x4000cae0\n'
