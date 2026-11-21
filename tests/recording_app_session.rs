//! Actual app admission state with production recording wire; no capture authority.
extern crate self as sex_pdx;
#[path="../crates/sex-pdx/src/desktop_recording.rs"]pub mod desktop_recording;
#[path="../apps/record/src/session.rs"]mod session;
use desktop_recording as w;
fn prepare(task:u32)->w::AppCommand{w::AppCommand::Prepare{session:w::SessionSequence::new(2,1).unwrap(),length:5,shell_epoch:2,task,files_epoch:3,spindle_task:12}}
fn name()->w::AppCommand{w::AppCommand::Name{session:w::SessionSequence::new(2,1).unwrap(),offset:0,words:[u64::from_le_bytes(*b"x.mkv\0\0\0"),0]}}
fn active(task:u32,display_epoch:u32)->w::AppCommand{w::AppCommand::Active{session:w::SessionSequence::new(2,1).unwrap(),shell_epoch:2,display_epoch,task}}
#[test]fn old_task_or_epoch_cannot_install_preparation(){let mut s=session::Session::new();assert!(!s.command(3,3,prepare(42).args(),1073741830,2));assert!(!s.command(3,3,prepare(1073741830).args(),1073741830,3));assert!(!s.command(3,99,prepare(1073741830).args(),1073741830,2));assert_eq!(s.phase,session::Phase::Idle);}
#[test]fn active_requires_prepared_current_epoch_and_exact_task(){let mut s=session::Session::new();let task=1073741830;assert!(s.command(3,3,prepare(task).args(),task,2));assert!(!s.command(3,3,active(task,4).args(),task,2));assert!(s.command(3,3,name().args(),task,2));assert_eq!(s.filename(),Some(&b"x.mkv"[..]));assert!(s.prepared(4));assert!(!s.command(3,3,active(task,5).args(),task,2));assert!(!s.command(3,3,active(42,4).args(),task,2));assert!(s.command(3,3,active(task,4).args(),task,2));assert_eq!(s.phase,session::Phase::Active);}
#[test]fn stop_is_irreversible_and_stale_prepare_cannot_rearm(){let mut s=session::Session::new();assert!(s.command(3,3,prepare(42).args(),42,2));assert!(s.command(3,3,name().args(),42,2));assert!(s.prepared(4));s.stop();assert!(!s.command(3,3,active(42,4).args(),42,2));assert!(!s.command(3,3,prepare(42).args(),42,2));assert!(s.saved());assert!(!s.command(3,3,active(42,4).args(),42,2));}
