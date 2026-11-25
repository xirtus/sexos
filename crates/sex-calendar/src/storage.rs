// SPDX-License-Identifier: GPL-3.0-only
//! Distinguish a failed replacement from an uncertain final durability sync.

#[derive(Debug, PartialEq, Eq)]
pub enum ReplaceFailure<E> { BeforeRename(E), AfterRename(E) }

pub fn finish_replace<T, E>(target: &mut T, rename: impl FnOnce(&mut T) -> Result<(), E>, sync: impl FnOnce(&mut T) -> Result<(), E>) -> Result<(), ReplaceFailure<E>> {
    rename(target).map_err(ReplaceFailure::BeforeRename)?;
    sync(target).map_err(ReplaceFailure::AfterRename)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distinguishes_failure_after_live_file_replacement() {
        let mut file = "old";
        let result = finish_replace(&mut file, |f| { *f = "new"; Ok::<(), &str>(()) }, |_| Err("sync"));
        assert_eq!(file, "new");
        assert_eq!(result, Err(ReplaceFailure::AfterRename("sync")));
        let mut file = "old";
        let result = finish_replace(&mut file, |_| Err::<(), &str>("rename"), |_| panic!("must not sync"));
        assert_eq!(file, "old");
        assert_eq!(result, Err(ReplaceFailure::BeforeRename("rename")));
    }
}
