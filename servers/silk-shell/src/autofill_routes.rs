// SPDX-License-Identifier: GPL-3.0-only
//! Native hooks for the pure broker. Only these functions bridge the held BOOT
//! catalogs/routes to IPC. New protocol is consumed before generic Shell handling.
use crate::autofill_broker::{Admission, Broker, Endpoint};
use sex_pdx::{
    autofill::{self as wire, Packet, Subject},
    PdxMessage,
};
static mut BROKER: Broker = Broker::new();
#[derive(Clone, Copy)]
struct Identity {
    subject: Subject,
    name: [u8; 16],
    part: usize,
    sent: bool,
    deadline: u64,
}
static mut IDENTITIES: [Option<Identity>; 16] = [None; 16];
static mut INTAKE: [Option<(Subject, Packet)>; 8] = [const { None }; 8];
pub unsafe fn bound(pd: u32, task: u32) {
    let Some(row) =
        sex_pdx::app_catalog::by_target(pd).filter(|r| r.app == sex_pdx::app_catalog::App::Program)
    else {
        return;
    };
    let Ok(c) = sex_pdx::app_lifecycle::catalog(row.lifecycle_slot) else {
        return;
    };
    if !c.is_sender(pd, task)
        || !c.is_subject(pd, row.surface, 3)
        || sex_pdx::app_lifecycle::info(row.lifecycle_slot) != Err(sex_pdx::app_lifecycle::LIVE)
    {
        return;
    }
    let subject = Subject { pd, task };
    BROKER.bind(Endpoint {
        subject,
        route: row.close_slot,
        name: [0; 16],
    });
    for item in &mut IDENTITIES {
        if item.is_some_and(|i| i.subject.pd == pd) {
            *item = None;
        }
    }
    if let Some(slot) = IDENTITIES.iter_mut().find(|i| i.is_none()) {
        *slot = Some(Identity {
            subject,
            name: [0; 16],
            part: 0,
            sent: false,
            deadline: sex_pdx::get_ticks().saturating_add(5000),
        });
    }
}
unsafe fn live(s: Subject) -> Option<bool> {
    crate::clipboard_live(sex_pdx::clipboard::Subject {
        pd: s.pd,
        task: s.task,
    })
}
pub unsafe fn retire(s: Subject) {
    for item in &mut IDENTITIES {
        if item.is_some_and(|i| i.subject == s) {
            *item = None;
        }
    }
    BROKER.retire(s);
    for item in &mut INTAKE {
        if item.as_ref().is_some_and(|(owner, _)| *owner == s) {
            *item = None;
        }
    }
}
pub unsafe fn receive(msg: &mut PdxMessage) -> bool {
    if msg.type_id == 1 && msg.caller_pd == 11 && msg.caller_task == 11 {
        if let Some((task, part, bytes)) = wire::identity_decode(msg.arg0) {
            if let Some(item) = IDENTITIES
                .iter_mut()
                .flatten()
                .find(|i| i.subject.task == task && i.part == part && i.sent)
            {
                for (n, b) in bytes.iter().enumerate() {
                    let at = part * 3 + n;
                    if at < 16 {
                        item.name[at] = *b;
                    }
                }
                item.part += 1;
                item.sent = false;
                return true;
            }
        }
    }
    if !wire::request(msg.type_id) {
        return false;
    }
    let subject = Subject {
        pd: msg.caller_pd,
        task: msg.caller_task,
    };
    let packet = Packet::new(msg.type_id, msg.arg0, msg.arg1, msg.arg2);
    for v in [
        &mut msg.arg0,
        &mut msg.arg1,
        &mut msg.arg2,
        &mut msg.type_id,
    ] {
        core::ptr::write_volatile(v, 0);
    }
    if matches!(packet.opcode, wire::UNREGISTER | wire::CANCEL | wire::DENY) {
        if let Some(scope) = BROKER.revocation_scope(subject, &packet) {
            BROKER.receive(subject, &packet, false, sex_pdx::get_ticks());
            if let Some(owners) = scope {
                for slot in &mut INTAKE {
                    if slot.as_ref().is_some_and(|(s, _)| owners.contains(s)) {
                        *slot = None;
                    }
                }
            }
        }
        poll();
        return true;
    }
    if let Some(slot) = INTAKE.iter_mut().find(|s| s.is_none()) {
        *slot = Some((subject, packet));
    }
    poll();
    true
}
pub unsafe fn poll() {
    let now = sex_pdx::get_ticks();
    for slot in &mut IDENTITIES {
        let Some(item) = slot.as_mut() else {
            continue;
        };
        if now >= item.deadline {
            *slot = None;
            continue;
        }
        match live(item.subject) {
            Some(false) => {
                *slot = None;
                continue;
            }
            None => continue,
            Some(true) => {}
        }
        if item.part == 6 {
            let n = item.name.iter().position(|b| *b == 0).unwrap_or(16);
            if n > 0
                && item.name[..n].iter().all(u8::is_ascii_graphic)
                && item.name[n..].iter().all(|b| *b == 0)
            {
                BROKER.label(item.subject, item.name);
            }
            *slot = None;
            continue;
        }
        if !item.sent {
            let (status, _) = sex_pdx::pdx_call(
                sex_pdx::program_store::SHELL_ROUTE_SLOT,
                wire::IDENTIFY,
                item.subject.word(),
                item.part as u64,
                0,
            );
            if status == 0 {
                item.sent = true;
            }
        }
    }
    if let Some(index) = INTAKE.iter().position(Option::is_some) {
        let s = INTAKE[index].as_ref().unwrap().0;
        match live(s) {
            Some(true) => {
                let source = &mut INTAKE[index].as_mut().unwrap().1;
                let packet = Packet::new(
                    source.opcode,
                    source.args[0],
                    source.args[1],
                    source.args[2],
                );
                source.erase();
                INTAKE[index] = None;
                let focused = crate::focused_program_participant()
                    .is_some_and(|p| p.subject.pd == s.pd && p.subject.task == s.task);
                BROKER.receive(s, &packet, focused, now);
            }
            Some(false) => {
                retire(s);
            }
            None => {}
        }
    }
    BROKER.poll(
        now,
        |s| live(s),
        |endpoint, packet| {
            let (status, _) = sex_pdx::pdx_call(
                endpoint.route,
                packet.opcode,
                packet.args[0],
                packet.args[1],
                packet.args[2],
            );
            if status == 0 {
                Admission::Accepted
            } else if status == sex_pdx::ERR_SERVICE_NOT_READY {
                Admission::Full
            } else {
                Admission::Rejected
            }
        },
    );
}
pub unsafe fn pending() -> bool {
    BROKER.pending()
        || INTAKE.iter().any(Option::is_some)
        || IDENTITIES.iter().any(|i| i.is_some_and(|i| !i.sent))
}
pub unsafe fn wake() -> Option<u64> {
    BROKER
        .wake(sex_pdx::get_ticks())
        .or_else(|| IDENTITIES.iter().any(Option::is_some).then_some(100))
}

#[cfg(test)]
pub unsafe fn reset_for_host_test() {
    BROKER = Broker::new();
    for slot in &mut INTAKE {
        *slot = None;
    }
    IDENTITIES = [None; 16];
}
