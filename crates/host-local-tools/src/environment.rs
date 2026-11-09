//! Name-only child inheritance policy, independent of any Host owner.

/// Snapshot names to remove from a command's inherited environment. Called
/// immediately before launch, outside owner locks. The implementation must be
/// short and synchronous, with no I/O or credential-value reads.
pub trait CommandEnvironment: Send + Sync + 'static {
    fn excluded_names(&self) -> Vec<String>;
}

/// Explicit standalone choice: inherit the environment without exclusions.
/// This does not provide the production Host's configured-credential policy.
pub struct InheritCommandEnvironment;
impl CommandEnvironment for InheritCommandEnvironment {
    fn excluded_names(&self) -> Vec<String> {
        Vec::new()
    }
}
