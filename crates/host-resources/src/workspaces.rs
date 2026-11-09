//! One builder's actual Workspace bindings, shared across its publications.
use crate::session_invalid;
use minicore_host_protocol::{Fault, WorkspaceRef};
use minicore_host_workspace::Workspace;
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{Arc, Mutex, Weak},
};

#[derive(Default)]
pub(crate) struct WorkspaceBindings(Mutex<BTreeMap<PathBuf, Weak<Workspace>>>);
impl WorkspaceBindings {
    pub(crate) async fn bind(&self, selection: &WorkspaceRef) -> Result<Arc<Workspace>, Fault> {
        // Filesystem resolution is outside owner state. Concurrent candidates
        // canonicalize independently, then share exactly one actual capability.
        let candidate = Workspace::bind(Some(selection))
            .await
            .map_err(|error| session_invalid(format!("workspace binding failed: {error}")))?;
        let mut bindings = self.0.lock().expect("workspace bindings mutex poisoned");
        bindings.retain(|_, binding| binding.strong_count() != 0);
        if let Some(existing) = bindings.get(candidate.root()).and_then(Weak::upgrade) {
            return Ok(existing);
        }
        let binding = Arc::new(candidate);
        bindings.insert(binding.root().to_path_buf(), Arc::downgrade(&binding));
        Ok(binding)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        future::{Future, poll_fn},
        task::Poll,
    };

    #[tokio::test]
    async fn canonical_aliases_share_one_actual_mutation_guard_and_cache_is_weak() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().to_str().unwrap().to_owned();
        let selection = WorkspaceRef { path: path.clone() };
        let alias = WorkspaceRef {
            path: format!("{path}/."),
        };
        let bindings = WorkspaceBindings::default();
        let (first, second) = tokio::join!(bindings.bind(&selection), bindings.bind(&alias));
        let first = first.unwrap();
        let second = second.unwrap();
        assert!(Arc::ptr_eq(&first, &second));
        let weak = Arc::downgrade(&first);
        let guard = first.begin_mutation().await;
        {
            let mut waiting = std::pin::pin!(second.begin_mutation());
            poll_fn(|cx| {
                assert!(
                    waiting.as_mut().poll(cx).is_pending(),
                    "same binding must serialize actual mutations"
                );
                Poll::Ready(())
            })
            .await;
            drop(guard);
            drop(waiting.await);
        }
        drop(first);
        drop(second);
        assert!(
            weak.upgrade().is_none(),
            "cache must not retain abandoned roots"
        );
        let rebound = bindings.bind(&selection).await.unwrap();
        assert_eq!(rebound.root(), directory.path());
        assert_eq!(bindings.0.lock().unwrap().len(), 1);
    }
}
