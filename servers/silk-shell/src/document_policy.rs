//! A revocable shell policy over the existing BOOT storage workspace.
//! This record is not a kernel capability and cannot expand the service grant.
pub const WORKSPACE_SCOPE: u64 = u64::MAX;
pub fn allows(
    subject: u64,
    operation: u8,
    requested: u64,
    published_id: u64,
    disk_path: u8,
    published: bool,
    active: bool,
    explicit_denial: bool,
) -> bool {
    active
        && !explicit_denial
        && published
        && subject == 200
        && matches!(operation, 0 | 6)
        && requested != 0
        && requested != WORKSPACE_SCOPE
        && requested == published_id
        && (3..=15).contains(&disk_path)
}
