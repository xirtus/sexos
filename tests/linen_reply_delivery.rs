#[path="../servers/linen/src/reply_delivery.rs"]mod reply_delivery;
use reply_delivery::ReplyDelivery;
#[test]fn buffer_full_retains_exact_response_until_accepted(){let mut r=ReplyDelivery::new();assert!(r.queue(3,0x12345678));for status in [2,2,1,2]{assert!(!r.poll(|pd,value|{assert_eq!((pd,value),(3,0x12345678));status}));assert!(r.pending());}assert!(r.poll(|pd,value|{assert_eq!((pd,value),(3,0x12345678));0}));assert!(!r.pending());assert!(r.poll(|_,_|panic!("duplicate delivery")));}
#[test]fn pending_reply_cannot_be_overwritten(){let mut r=ReplyDelivery::new();assert!(r.queue(3,7));assert!(!r.queue(3,8));assert!(r.poll(|_,v|{assert_eq!(v,7);0}));assert!(r.queue(3,8));assert!(r.poll(|_,v|{assert_eq!(v,8);0}));}
#[test]fn permanent_peer_is_only_authorized_recipient(){for caller in [0,1,7,9,12,15,u32::MAX]{let mut r=ReplyDelivery::new();assert!(!r.queue(caller,99));assert!(!r.pending());}}
#[test]fn zero_and_error_response_values_are_preserved(){for value in [0,u64::MAX,u64::MAX-1]{let mut r=ReplyDelivery::new();assert!(r.queue(3,value));assert!(!r.poll(|_,v|{assert_eq!(v,value);2}));assert!(r.poll(|_,v|{assert_eq!(v,value);0}));}}
#[test]fn spindle_only_existing_operations_require_live_attestation(){for opcode in [0x42,0x46,0x47]{assert!(reply_delivery::admit(12,opcode,Some(5)));for status in [None,Some(0),Some(1),Some(2),Some(3),Some(u64::MAX)]{assert!(!reply_delivery::admit(12,opcode,status));}}for opcode in [0x41,0x43,0x44,0x45,0x49,0x202]{assert!(!reply_delivery::admit(12,opcode,Some(5)));}assert!(reply_delivery::admit(3,0x44,None));assert!(!reply_delivery::admit(15,0x46,Some(5)));}
#[test]fn stale_queued_spindle_open_has_no_handler_effect(){let mut effects=0;for status in [Some(0),Some(2),Some(3),None]{if reply_delivery::admit(12,0x46,status){effects+=1;}}assert_eq!(effects,0);}
#[test]fn best_effort_legacy_reply_never_enters_permanent_peer_custody(){let mut calls=0;assert!(!reply_delivery::legacy_reply(12,99,|pd,v|{calls+=1;assert_eq!((pd,v),(12,99));2}));assert_eq!(calls,1);assert!(!reply_delivery::legacy_reply(15,99,|_,_|panic!("foreign reply")));let mut r=ReplyDelivery::new();assert!(r.queue(3,22));assert!(r.poll(|pd,v|{assert_eq!((pd,v),(3,22));0}));}

#[test]fn reserved_successor_has_independent_observed_authority_and_best_effort_reply(){
 for op in [0x42,0x46,0x47]{assert!(reply_delivery::admit(17,op,Some(5)));for state in [None,Some(0),Some(2),Some(3)]{assert!(!reply_delivery::admit(17,op,state));}}
 for op in [0x41,0x44,0x45,0x49]{assert!(!reply_delivery::admit(17,op,Some(5)));}
 let mut sent=Vec::new();assert!(reply_delivery::legacy_reply(17,9,|p,v|{sent.push((p,v));0}));assert_eq!(sent,vec![(17,9)]);
 assert!(!reply_delivery::legacy_reply(16,9,|_,_|panic!("Quil is not a Spindle route")));
}
