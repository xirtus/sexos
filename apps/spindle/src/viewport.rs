//! Pure bounded view construction. The command/history/scrollback remain intact.
use sex_pdx::spindle_view::{Layout,MAX_TEXT};
#[derive(Clone,Copy,Debug,PartialEq,Eq)]
pub struct Text{pub bytes:[u8;MAX_TEXT],pub len:usize,pub cols:usize,pub rows:usize}
impl Text{const EMPTY:Self=Self{bytes:[0;MAX_TEXT],len:0,cols:0,rows:0};}
#[derive(Clone,Copy,Debug,PartialEq,Eq)]
pub struct Prompt{pub bytes:[u8;40],pub len:usize,pub caret:Option<usize>,pub start:usize,pub end:usize}
/// A presentation-only cursor and overflow marks never enter the command.
pub fn prompt(command:&[u8],cursor:usize,columns:usize)->Prompt{
 let cols=columns.min(40);let mut out=Prompt{bytes:[b' ';40],len:cols,caret:None,start:0,end:0};
 if cols>0{out.bytes[0]=b'>';}if cols<=2{return out}let room=cols-2;let cursor=cursor.min(command.len());
 for start in 0..=cursor{let left=(start>0)as usize;if 1+left>room{continue}let mut end=(start+room-1-left).min(command.len());if end<command.len()&&end>start{end-=1;}
  let right=(end<command.len())as usize;if cursor>end||1+left+end-start+right>room{continue}
  out.start=start;out.end=end;let mut at=2;if left>0{out.bytes[at]=b'<';at+=1;}
  for i in start..end{if i==cursor{out.caret=Some(at);out.bytes[at]=b'|';at+=1;}out.bytes[at]=if (32..=126).contains(&command[i]){command[i]}else{b'?'};at+=1;}
  if cursor==end{out.caret=Some(at);out.bytes[at]=b'|';at+=1;}if right>0{out.bytes[at]=b'>';}
  return out;
 }
 // At a one-cell body, the caret is more useful than an unrenderable suffix.
 out.bytes[2]=b'|';out.caret=Some(2);out.start=cursor;out.end=cursor;out
}
pub fn render<'a>(view:Layout,command:&[u8],total:usize,offset:usize,line:impl FnMut(usize)->&'a[u8])->[Text;4]{render_at(view,command,command.len(),total,offset,line)}
pub fn render_at<'a>(view:Layout,command:&[u8],cursor:usize,total:usize,offset:usize,mut line:impl FnMut(usize)->&'a[u8])->[Text;4]{
 let mut out=[Text::EMPTY;4];let cols=view.columns();let lines=view.lines();if cols<2||lines==0{return out}
 let mut grid=[b' ';480];let avail=total.min(lines-1);let end=total.saturating_sub(offset).max(avail);let start=end-avail;
 for row in 0..avail{let src=line(start+row);let n=src.len().min(cols);let dst=(lines-1-avail+row)*cols;grid[dst..dst+n].copy_from_slice(&src[..n]);}
 let first=(lines-1)*cols;let p=prompt(command,cursor,cols);grid[first..first+cols].copy_from_slice(&p.bytes[..cols]);
 for i in 0..4{let band=i/2;let half=i%2;let rows=view.rows[band];if rows==0{continue}let first=if band==0{0}else{view.rows[0]};out[i].cols=view.cols;out[i].rows=rows;
  for row in 0..rows{let src=(first+row)*cols+half*view.cols;let dst=row*(view.cols+1);out[i].bytes[dst..dst+view.cols].copy_from_slice(&grid[src..src+view.cols]);out[i].bytes[dst+view.cols]=b'\n';}out[i].len=rows*(view.cols+1);
 }out
}
