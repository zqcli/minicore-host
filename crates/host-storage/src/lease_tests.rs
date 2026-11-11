//! A real child holds the exact open-file description, without timing a fork.
use std::{
    cell::RefCell,
    fs::File,
    io::{self, BufRead, BufReader, Read, Write},
    process::{Child, Command, Stdio},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

type LockHook = Box<dyn FnOnce(&File)>;
thread_local! {
    static AFTER_LOCK: RefCell<Option<LockHook>> = const { RefCell::new(None) };
    #[cfg(unix)]
    static INHERITED: RefCell<Option<InheritedDescriptor>> = const { RefCell::new(None) };
}

pub(crate) fn on_next_lock(hook: impl FnOnce(&File) + 'static) {
    AFTER_LOCK.with(|next| assert!(next.borrow_mut().replace(Box::new(hook)).is_none()));
}
#[cfg(unix)]
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
#[cfg(unix)]
pub(crate) fn inherited() -> InheritedDescriptor {
    INHERITED.with(|child| child.borrow_mut().take().unwrap())
}

pub(crate) struct ChildProbe(Child);
#[cfg(unix)]
pub(crate) type InheritedDescriptor = ChildProbe;
impl ChildProbe {
    #[cfg(unix)]
    pub(crate) fn new(file: &File) -> Self {
        // Duplication into stderr deliberately keeps the same description alive
        // across exec, extending the otherwise short fork-before-exec window.
        // This is not a storage worker: the child never uses or unlocks stderr.
        Self::spawn(Stdio::from(file.try_clone().unwrap()))
    }
    #[cfg(windows)]
    pub(crate) fn unrelated() -> Self {
        Self::spawn(Stdio::null())
    }
    fn spawn(stderr: Stdio) -> Self {
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "lease_tests::inherited_descriptor_child",
                "--nocapture",
            ])
            .env("MINICORE_STORAGE_INHERITED_DESCRIPTOR", "1")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(stderr)
            .spawn()
            .unwrap();
        let stdout = child.stdout.take().unwrap();
        let (ready, received) = mpsc::channel();
        let reader = thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                match line {
                    // libtest may prefix this line with its current test name.
                    Ok(line) if line.ends_with("inherited descriptor ready") => {
                        let _ = ready.send(Ok(()));
                        return;
                    }
                    Ok(_) => {}
                    Err(error) => {
                        let _ = ready.send(Err(error));
                        return;
                    }
                }
            }
            let _ = ready.send(Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "descriptor child exited before readiness",
            )));
        });
        let readiness = received.recv_timeout(Duration::from_secs(30));
        if !matches!(&readiness, Ok(Ok(()))) {
            let _ = child.kill();
            let status = child.wait();
            let _ = reader.join();
            panic!("descriptor child did not become ready: {readiness:?}; {status:?}");
        }
        reader.join().unwrap();
        Self(child)
    }
    pub(crate) fn is_alive(&mut self) -> bool {
        self.0.try_wait().unwrap().is_none()
    }
}
impl Drop for ChildProbe {
    fn drop(&mut self) {
        // Also reap cleanly when a red regression assertion unwinds.
        if let Some(mut input) = self.0.stdin.take() {
            let _ = input.write_all(b"x");
        }
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            match self.0.try_wait() {
                Ok(Some(_)) => return,
                Ok(None) if Instant::now() < deadline => {
                    thread::sleep(Duration::from_millis(10));
                }
                _ => break,
            }
        }
        let _ = self.0.kill();
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
