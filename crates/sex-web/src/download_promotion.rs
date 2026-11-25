// SPDX-License-Identifier: GPL-3.0-only
//! Order the Files operations that turn a hidden download into a visible one.

/// The part must be durable before it receives the public name. The new name
/// must then be durable before the browser reports a completed download.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step { SyncPart, Rename, SyncName }

pub fn promote<E>(mut apply: impl FnMut(Step) -> Result<(), E>) -> Result<(), (Step, E)> {
    for step in [Step::SyncPart, Step::Rename, Step::SyncName] {
        apply(step).map_err(|error| (step, error))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec::Vec;

    #[test]
    fn promotion_requires_both_syncs_in_order() {
        let mut seen = Vec::new();
        assert_eq!(promote::<()>(|step| { seen.push(step); Ok(()) }), Ok(()));
        assert_eq!(seen, [Step::SyncPart, Step::Rename, Step::SyncName]);
        for failed in [Step::SyncPart, Step::Rename, Step::SyncName] {
            let mut seen = Vec::new();
            assert_eq!(promote(|step| { seen.push(step); if step == failed { Err(()) } else { Ok(()) } }), Err((failed, ())));
            assert_eq!(seen.last(), Some(&failed));
            assert_eq!(seen.len(), [Step::SyncPart, Step::Rename, Step::SyncName].iter().position(|s| *s == failed).unwrap() + 1);
        }
    }
}
