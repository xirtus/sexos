// Actual pure SDK modules; synthetic reply metadata never proves held BOOT caps.
#![allow(dead_code)]
#[path="../crates/sex-pdx/src/memory_region.rs"]pub mod memory_region;
pub use memory_region::MemoryRegionInfo;
pub const ERR_CAP_INVALID:u64=0xFFFF_FFFF_FFFF_FFFC;
#[path="../crates/sex-pdx/src/display_frames.rs"]mod display_frames;
use display_frames::{FrameBacking,Layout,SLOT,REGION_ID,MAX_WIDTH,MAX_HEIGHT,PAGE};
use memory_region::{RegionView,RegionGrant,QUERY_VIEW,READ,WRITE,DMA_ADDRESS,USER_VA_END};
fn replies(layout:Layout)->(MemoryRegionInfo,RegionView){
 let grant=RegionGrant{region_id:11,virtual_base:0x400000000000,length:layout.total_bytes(),rights:QUERY_VIEW|READ|WRITE,dma_base:None,peer_pd:None};
 (memory_region::resolve_info(Some(&grant)).unwrap(),memory_region::resolve_view(Some(&grant),memory_region::ViewRequest{offset:0,length:layout.total_bytes(),rights:QUERY_VIEW|READ|WRITE}).unwrap())
}
fn denied(layout:Layout,info:MemoryRegionInfo,view:RegionView){assert!(matches!(unsafe{FrameBacking::from_resolved(layout,info,view)},Err(ERR_CAP_INVALID)));}
#[test]fn actual_compact_wide_geometry_sizes_and_two_addresses(){
 assert_eq!((SLOT,REGION_ID,MAX_WIDTH,MAX_HEIGHT,PAGE),(98,11,8192,4320,4096));
 for(w,h,pitch,bytes,total)in[(1024,768,4096,3_145_728,6_291_456),(1280,800,5120,4_096_000,8_192_000)]{
  let layout=Layout::new(w,h,pitch).unwrap();assert_eq!((layout.width(),layout.height(),layout.pitch()),(w as usize,h as usize,pitch as u32));assert_eq!((layout.frame_bytes(),layout.span_bytes(),layout.total_bytes(),layout.buddy_bytes()),(bytes,bytes,total,8_388_608));
  let(i,v)=replies(layout);let bound=unsafe{FrameBacking::from_resolved(layout,i,v)}.unwrap();assert_eq!(bound.layout(),layout);assert_eq!(bound.address(0),Some(v.virtual_address));assert_eq!(bound.address(1),Some(v.virtual_address+bytes));assert_eq!(bound.address(2),None);assert_eq!(bound.address(usize::MAX),None);
 }
}
#[test]fn tiny_page_rounding_and_maximum_supported_extent_are_literal(){
 let tiny=Layout::new(1,1,4).unwrap();assert_eq!((tiny.frame_bytes(),tiny.span_bytes(),tiny.total_bytes(),tiny.buddy_bytes()),(4,4096,8192,8192));
 let crossing=Layout::new(1025,1,4100).unwrap();assert_eq!((crossing.frame_bytes(),crossing.span_bytes(),crossing.total_bytes(),crossing.buddy_bytes()),(4100,8192,16384,16384));
 let maximum=Layout::new(8192,4320,32768).unwrap();assert_eq!((maximum.frame_bytes(),maximum.span_bytes(),maximum.total_bytes(),maximum.buddy_bytes()),(141_557_760,141_557_760,283_115_520,536_870_912));
 for layout in [tiny,crossing,maximum]{let(i,v)=replies(layout);let bound=unsafe{FrameBacking::from_resolved(layout,i,v)}.unwrap();let a=bound.address(0).unwrap();let b=bound.address(1).unwrap();assert_eq!(a%4096,0);assert_eq!(b%4096,0);assert_eq!(a+layout.span_bytes(),b);assert!(a+layout.frame_bytes()<=b);assert!(b+layout.frame_bytes()<=a+layout.total_bytes());assert!(layout.total_bytes()<=layout.buddy_bytes());}
}
#[test]fn dimensions_pitch_and_overflow_inputs_reject_before_any_backing(){
 for(w,h,p)in[(0,768,0),(1024,0,4096),(8193,1,32772),(1,4321,4),(1024,768,4095),(1024,768,4097),(1,1,8),(1,1,0),(u64::MAX,1,4),(1,u64::MAX,4),(1,1,u64::MAX),(u64::MAX,u64::MAX,u64::MAX),(1<<32,1,4),(1024,768,4096+(1<<32))]{assert_eq!(Layout::new(w,h,p),None,"{w}/{h}/{p}");}
}
#[test]fn exact_info_region_length_rights_and_peer_are_required(){
 let layout=Layout::new(1024,768,4096).unwrap();let(info,view)=replies(layout);
 for id in [0,10,12,u32::MAX]{denied(layout,MemoryRegionInfo{region_id:id,..info},view);}
 for length in [0,1,layout.total_bytes()-1,layout.total_bytes()+1,layout.span_bytes(),layout.buddy_bytes(),u64::MAX]{denied(layout,MemoryRegionInfo{length,..info},view);}
 for rights in 0u8..=u8::MAX{if rights!=7{denied(layout,MemoryRegionInfo{rights,..info},view);}}
 for peer in [0,1,15,u32::MAX]{denied(layout,MemoryRegionInfo{peer_pd:Some(peer),..info},view);}
}
#[test]fn exact_full_view_id_length_and_no_dma_are_required(){
 let layout=Layout::new(1280,800,5120).unwrap();let(info,view)=replies(layout);
 for id in [0,10,12,u32::MAX]{denied(layout,info,RegionView{region_id:id,..view});}
 for length in [0,1,layout.total_bytes()-1,layout.total_bytes()+1,layout.span_bytes(),layout.buddy_bytes(),u64::MAX]{denied(layout,info,RegionView{length,..view});}
 for dma in [0,4096,u64::MAX]{denied(layout,info,RegionView{dma_address:Some(dma),..view});}
 // An actual generic partial resolution remains a valid generic view but must
 // not qualify as this two-buffer full-backing descriptor.
 let grant=RegionGrant{region_id:11,virtual_base:view.virtual_address,length:layout.total_bytes(),rights:7,dma_base:None,peer_pd:None};
 let partial=memory_region::resolve_view(Some(&grant),memory_region::ViewRequest{offset:layout.span_bytes(),length:layout.span_bytes(),rights:7}).unwrap();denied(layout,info,partial);
}
#[test]fn base_alignment_canonical_user_extent_and_wrap_are_checked(){
 let layout=Layout::new(1,1,4).unwrap();let(info,view)=replies(layout);
 for base in [0,1,4095,4097,USER_VA_END,USER_VA_END-4096,USER_VA_END+4096,u64::MAX,u64::MAX-4095]{denied(layout,info,RegionView{virtual_address:base,..view});}
 for base in [4096,USER_VA_END-8192]{let bound=unsafe{FrameBacking::from_resolved(layout,info,RegionView{virtual_address:base,..view})}.unwrap();assert_eq!(bound.address(0),Some(base));assert_eq!(bound.address(1),Some(base+4096));}
}
#[test]fn foreign_geometry_and_crossed_typed_replies_do_not_retarget(){
 let small=Layout::new(1024,768,4096).unwrap();let large=Layout::new(1280,800,5120).unwrap();let(si,sv)=replies(small);let(li,lv)=replies(large);
 denied(small,li,lv);denied(small,si,lv);denied(large,si,lv);denied(large,li,sv);
 let bound=unsafe{FrameBacking::from_resolved(small,si,sv)}.unwrap();assert_eq!(bound.layout(),small);assert_eq!(bound.address(1),Some(sv.virtual_address+3_145_728));
}
