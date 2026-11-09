//! A real child holds the exact open-file description, without timing a fork.
use std::{
    cell::RefCell,
    fs::File,
    io::{BufRead, BufReader, Read, Write},
    process::{Child, Command, Stdio},
};

type LockHook = Box<dyn FnOnce(&File)>;
thread_local! {
    static AFTER_LOCK: RefCell<Option<LockHook>> = const { RefCell::new(None) };
    static INHERITED: RefCell<Option<InheritedDescriptor>> = const { RefCell::new(None) };
}

pub(crate) fn on_next_lock(hook: impl FnOnce(&File) + 'static) {
    AFTER_LOCK.with(|next| assert!(next.borrow_mut().replace(Box::new(hook)).is_none()));
}
pub(crate) fn inherit_next_lock() {
    on_next_lock(|file| {
        INHERITED.with(|child| {
            assert!(
                child
                    .borrow_mut()
                    .replace(InheritedDescriptor::new(file))
                    .is_none()
            );
        });
    });
}
pub(crate) fn after_lock(file: &File) {
    let hook = AFTER_LOCK.with(|next| next.borrow_mut().take());
    if let Some(hook) = hook {
        hook(file);
    }
}
pub(crate) fn inherited() -> InheritedDescriptor {
    INHERITED.with(|child| child.borrow_mut().take().unwrap())
}

pub(crate) struct InheritedDescriptor(Child);
impl InheritedDescriptor {
    pub(crate) fn new(file: &File) -> Self {
        // Duplication into stderr deliberately keeps the same description alive
        // across exec, extending the otherwise short fork-before-exec window.
        // This is not a storage worker: the child never uses or unlocks stderr.
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "lease_tests::inherited_descriptor_child",
                "--nocapture",
            ])
            .env("MINICORE_STORAGE_INHERITED_DESCRIPTOR", "1")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::from(file.try_clone().unwrap()))
            .spawn()
            .unwrap();
        let mut lines = BufReader::new(child.stdout.take().unwrap()).lines();
        while lines.next().unwrap().unwrap() != "inherited descriptor ready" {}
        Self(child)
    }
    pub(crate) fn is_alive(&mut self) -> bool {
        self.0.try_wait().unwrap().is_none()
    }
}
impl Drop for InheritedDescriptor {
    fn drop(&mut self) {
        // Also reap cleanly when a red regression assertion unwinds.
        if let Some(mut input) = self.0.stdin.take() {
            let _ = input.write_all(b"x");
        }
        let _ = self.0.wait();
    }
}

#[test]
fn inherited_descriptor_child() {
    if std::env::var_os("MINICORE_STORAGE_INHERITED_DESCRIPTOR").is_none() {
        return;
    }
    println!("inherited descriptor ready");
    std::io::stdout().flush().unwrap();
    std::io::stdin().read_exact(&mut [0]).unwrap();
    // Do not run any inherited Rust owner's destructor in this unrelated child.
    std::process::exit(0);
}
