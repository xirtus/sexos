// SPDX-License-Identifier: GPL-3.0-only
//! Latest explicit command is retained while older accepted work settles.
//! This queue holds presentation intent only; it creates no authority.
use crate::{clock_probe,entropy_probe,tls_probe,trust_probe};

#[derive(Clone,Copy,Debug,PartialEq,Eq)]
pub enum Intent{Echo,Dns,HttpDns,Http,TlsDns,TlsStart,TlsPrepare,Seed,Clock}
#[derive(Clone,Copy,Debug,PartialEq,Eq)]
pub struct Busy{pub route:bool,pub providers:bool,pub tls:bool}

/// Cancel private preparation before replacing its purpose. Each held provider
/// retains an already accepted operation's correlation until its actual receipt.
pub fn replace(pending:&mut Option<Intent>,next:Intent,tls:&mut tls_probe::Probe,
    policy:&mut trust_probe::Client,seed:&mut entropy_probe::Client,time:&mut clock_probe::Client)->usize{
    let erased=tls.cancel(policy,seed,time);*pending=Some(next);erased
}

/// A fresh local/TLS attempt cannot consume the abandoned attempt's provider
/// result. Continuations may advance the already owned TLS preparation.
pub fn take_ready(pending:&mut Option<Intent>,busy:Busy)->Option<Intent>{
    if busy.route{return None;}
    if matches!(pending,Some(Intent::TlsDns|Intent::TlsPrepare|Intent::Seed|Intent::Clock))
        &&(busy.providers||busy.tls){return None;}
    pending.take()
}
