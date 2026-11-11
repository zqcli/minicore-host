use process_wrap::tokio::{ChildWrapper, CommandWrap};
use std::{io, process::ExitStatus};
use tokio::{io::AsyncRead, process::Command};

type OutputReader = Box<dyn AsyncRead + Unpin + Send>;

#[cfg(unix)]
pub(super) fn shell_command() -> io::Result<Command> {
    Ok(Command::new("/bin/bash"))
}

#[cfg(windows)]
pub(super) fn shell_command() -> io::Result<Command> {
    use std::path::PathBuf;

    let inherited = std::env::var_os("PATH").unwrap_or_default();
    let mut candidates = Vec::new();
    for directory in std::env::split_paths(&inherited).filter(|path| path.is_absolute()) {
        candidates.push(directory.join("bash.exe"));
        candidates.push(directory.join("../bin/bash.exe"));
        candidates.push(directory.join("../usr/bin/bash.exe"));
    }
    for name in ["ProgramFiles", "ProgramFiles(x86)", "LocalAppData"] {
        if let Some(root) = std::env::var_os(name).map(PathBuf::from) {
            candidates.push(root.join("Git/bin/bash.exe"));
            candidates.push(root.join("Programs/Git/bin/bash.exe"));
        }
    }
    for executable in candidates {
        let Some(directory) = executable.parent() else {
            continue;
        };
        // The Windows system bash.exe launches WSL, outside this native job.
        // Bind an installed MSYS Bash and its utilities, never a workspace name.
        let utilities = [directory.to_path_buf(), directory.join("../usr/bin")]
            .into_iter()
            .find(|path| path.join("msys-2.0.dll").is_file());
        if !executable.is_file() || utilities.is_none() {
            continue;
        }
        let executable = std::fs::canonicalize(executable)?;
        let utilities = std::fs::canonicalize(utilities.expect("checked utilities"))?;
        let paths = std::iter::once(utilities).chain(std::env::split_paths(&inherited));
        let path = std::env::join_paths(paths).map_err(io::Error::other)?;
        let mut command = Command::new(executable);
        command.env("PATH", path);
        return Ok(command);
    }
    Err(io::Error::new(
        io::ErrorKind::NotFound,
        "Git Bash is not installed or available in PATH",
    ))
}

pub(super) fn wrap_scope(command: &mut CommandWrap) {
    #[cfg(unix)]
    command.wrap(process_wrap::tokio::ProcessGroup::leader());
    #[cfg(windows)]
    {
        // Keep the root suspended until our own Job completion port is bound
        // and its monitor is ready. The wrapper preserves explicit suspension.
        command.wrap(process_wrap::tokio::CreationFlags(
            windows_sys::Win32::System::Threading::CREATE_SUSPENDED,
        ));
        command.wrap(process_wrap::tokio::JobObject);
    }
}

pub(super) fn stop_scope(child: &mut Box<dyn ChildWrapper>, pid: u32) -> io::Result<()> {
    #[cfg(unix)]
    {
        let _ = child;
        stop_group(pid)
    }
    #[cfg(windows)]
    {
        let _ = pid;
        child.start_kill()
    }
}

#[cfg(unix)]
fn stop_group(pid: u32) -> io::Result<()> {
    use nix::{errno::Errno, sys::signal::Signal, unistd::Pid};
    match nix::sys::signal::killpg(Pid::from_raw(pid as i32), Signal::SIGKILL) {
        Ok(()) | Err(Errno::ESRCH) => Ok(()),
        Err(error) => Err(io::Error::from_raw_os_error(error as i32)),
    }
}

#[cfg(unix)]
pub(super) async fn finish_scope(pid: u32) -> io::Result<bool> {
    for _ in 0..10 {
        let observation = match probe_group(pid) {
            Ok(true) => return Ok(true),
            // The leader has been reaped. Stop members that a fork may have
            // added after the first group signal's member snapshot.
            Ok(false) => stop_group(pid),
            Err(error) => Err(error),
        };
        match observation {
            Ok(()) => {}
            // Darwin filters zombies when signalling a still-existing group.
            // EPERM remains unconfirmed until an actual later ESRCH; a lasting
            // permission failure is returned by the final probe below.
            #[cfg(target_os = "macos")]
            Err(error) if error.raw_os_error() == Some(nix::errno::Errno::EPERM as i32) => {}
            Err(error) => return Err(error),
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    probe_group(pid)
}

#[cfg(unix)]
fn probe_group(pid: u32) -> io::Result<bool> {
    use nix::{errno::Errno, unistd::Pid};
    match nix::sys::signal::killpg(Pid::from_raw(pid as i32), None) {
        Ok(()) => Ok(false),
        Err(Errno::ESRCH) => Ok(true),
        Err(error) => Err(io::Error::from_raw_os_error(error as i32)),
    }
}

pub(super) fn exit_signal(status: &ExitStatus) -> Option<i32> {
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        status.signal()
    }
    #[cfg(windows)]
    {
        let _ = status;
        None
    }
}

#[cfg(unix)]
pub(super) struct Capture;

#[cfg(unix)]
impl Capture {
    pub(super) async fn new() -> io::Result<Self> {
        Ok(Self)
    }

    pub(super) fn configure(&mut self, command: &mut Command) {
        command
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
    }

    pub(super) fn take_readers(
        self,
        child: &mut Box<dyn ChildWrapper>,
    ) -> (OutputReader, OutputReader) {
        (
            Box::new(child.stdout().take().expect("configured stdout pipe")),
            Box::new(child.stderr().take().expect("configured stderr pipe")),
        )
    }
}

#[cfg(windows)]
pub(super) struct Capture {
    stdout: PipeEnds,
    stderr: PipeEnds,
}

#[cfg(windows)]
struct PipeEnds {
    server: tokio::net::windows::named_pipe::NamedPipeServer,
    client: Option<std::fs::File>,
}

#[cfg(windows)]
impl Capture {
    pub(super) async fn new() -> io::Result<Self> {
        let stdout = capture_pipe("stdout")?;
        let stderr = capture_pipe("stderr")?;
        tokio::try_join!(stdout.server.connect(), stderr.server.connect())?;
        Ok(Self { stdout, stderr })
    }

    pub(super) fn configure(&mut self, command: &mut Command) {
        command
            .stdout(self.stdout.client.take().expect("owned stdout client"))
            .stderr(self.stderr.client.take().expect("owned stderr client"));
    }

    pub(super) fn take_readers(
        self,
        _: &mut Box<dyn ChildWrapper>,
    ) -> (OutputReader, OutputReader) {
        (Box::new(self.stdout.server), Box::new(self.stderr.server))
    }
}

#[cfg(windows)]
fn capture_pipe(stream: &str) -> io::Result<PipeEnds> {
    use std::sync::atomic::{AtomicU64, Ordering};
    use tokio::net::windows::named_pipe::{PipeMode, ServerOptions};

    static NEXT_PIPE: AtomicU64 = AtomicU64::new(1);
    let id = NEXT_PIPE.fetch_add(1, Ordering::Relaxed);
    let name = format!(
        r"\\.\pipe\minicore-host-{}-{id}-{stream}",
        std::process::id()
    );
    // Adapted from minicore-agent: own asynchronous server readers so cancelling
    // an incomplete drain never leaves a blocking inherited-pipe read task.
    let server = ServerOptions::new()
        .pipe_mode(PipeMode::Byte)
        .access_outbound(false)
        .first_pipe_instance(true)
        .create(&name)?;
    let client = std::fs::OpenOptions::new().write(true).open(name)?;
    Ok(PipeEnds {
        server,
        client: Some(client),
    })
}

#[cfg(all(test, unix))]
mod scope_tests {
    use super::{finish_scope, probe_group, stop_group};
    use nix::{errno::Errno, sys::signal::kill, unistd::Pid};
    use std::{process::Stdio, time::Duration};
    use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};

    struct GroupBackstop(u32);

    impl Drop for GroupBackstop {
        fn drop(&mut self) {
            let _ = stop_group(self.0);
        }
    }

    #[tokio::test]
    async fn final_scope_cleanup_stops_live_members_after_the_leader_was_reaped() {
        // A real exited leader leaves a live member holding both pipes. This
        // exercises the final cleanup condition, without claiming to control
        // the kernel's timing when an initial group signal races with fork.
        let mut child = tokio::process::Command::new("/bin/bash")
            .args([
                "--noprofile",
                "--norc",
                "-c",
                "sleep 30 & printf '%s\\n' \"$!\"",
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .process_group(0)
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        let pid = child.id().unwrap();
        let _backstop = GroupBackstop(pid);
        let mut stdout = BufReader::new(child.stdout.take().unwrap());
        let mut stderr = child.stderr.take().unwrap();
        let mut line = String::new();
        tokio::time::timeout(Duration::from_secs(5), stdout.read_line(&mut line))
            .await
            .expect("background member did not publish its actual PID")
            .unwrap();
        let member = Pid::from_raw(line.trim().parse().unwrap());
        let leader = tokio::time::timeout(Duration::from_secs(5), child.wait())
            .await
            .expect("leader did not exit")
            .unwrap();
        assert!(leader.success());
        assert_eq!(kill(member, None), Ok(()));
        assert!(!probe_group(pid).unwrap());

        let drain = async {
            let mut out = Vec::new();
            let mut err = Vec::new();
            tokio::try_join!(stdout.read_to_end(&mut out), stderr.read_to_end(&mut err)).unwrap();
            assert!(out.is_empty() && err.is_empty());
        };
        let (confirmed, ()) = tokio::time::timeout(Duration::from_secs(5), async {
            tokio::join!(finish_scope(pid), drain)
        })
        .await
        .expect("final cleanup did not close the actual member's pipes");
        assert!(confirmed.unwrap());
        assert_eq!(kill(member, None), Err(Errno::ESRCH));
        assert!(probe_group(pid).unwrap());
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::{finish_scope, probe_group};
    use nix::errno::Errno;
    use std::{
        os::unix::process::CommandExt,
        process::{Child, Command, Stdio},
        time::Duration,
    };

    struct UnreapedChild(Child);

    impl Drop for UnreapedChild {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    #[tokio::test]
    async fn darwin_group_permission_error_requires_actual_reaping_before_confirmation() {
        // Keep an actual exited process unreaped: Darwin still has its process
        // group but filters its sole zombie member when probing killpg(..., 0).
        let mut child = UnreapedChild(
            Command::new("/bin/bash")
                .args(["--noprofile", "--norc", "-c", "exit 0"])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .process_group(0)
                .spawn()
                .unwrap(),
        );
        let pid = child.0.id();
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                match probe_group(pid) {
                    Err(error) if error.raw_os_error() == Some(Errno::EPERM as i32) => break,
                    Ok(false) => tokio::time::sleep(Duration::from_millis(5)).await,
                    result => panic!("unexpected unreaped process group state: {result:?}"),
                }
            }
        })
        .await
        .expect("the real child did not reach Darwin's unreaped group state");

        let error = finish_scope(pid).await.unwrap_err();
        assert_eq!(error.raw_os_error(), Some(Errno::EPERM as i32));

        let confirmation = finish_scope(pid);
        tokio::pin!(confirmation);
        assert!(
            tokio::time::timeout(Duration::from_millis(40), &mut confirmation)
                .await
                .is_err(),
            "an unreaped group must not be reported as absent"
        );
        assert!(child.0.wait().unwrap().success());
        assert!(confirmation.await.unwrap());
    }
}
