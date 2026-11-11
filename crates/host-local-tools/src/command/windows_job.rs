//! The private Windows Job completion boundary.
//!
//! A zero accounting count can precede actual process termination after
//! TerminateJobObject. Only this Job's received ACTIVE_PROCESS_ZERO packet is
//! completion evidence. The wrapper still owns containment and its full wait.
//! See Microsoft's JOB_OBJECT_ASSOCIATE_COMPLETION_PORT reference and
//! https://devblogs.microsoft.com/oldnewthing/20130405-00/?p=4743/.
//!
//! All raw handles stay borrowed from Rust owners or become OwnedHandle exactly
//! once. The completion worker owns its port and a Job clone until it returns;
//! it never dereferences an OVERLAPPED value supplied by a Job notification.
#![allow(unsafe_code)]

use process_wrap::tokio::{ChildWrapper, CommandWrap, JobObjectChild, JobObjectHandle};
use std::{
    any::Any,
    io,
    os::windows::io::{AsHandle, AsRawHandle, BorrowedHandle, FromRawHandle, OwnedHandle},
    sync::mpsc::{self, Receiver, Sender, TryRecvError},
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
use tokio::sync::oneshot;
use windows_sys::Win32::{
    Foundation::{ERROR_NO_MORE_FILES, HANDLE, INVALID_HANDLE_VALUE, WAIT_OBJECT_0, WAIT_TIMEOUT},
    System::{
        Diagnostics::ToolHelp::{
            CreateToolhelp32Snapshot, TH32CS_SNAPTHREAD, THREADENTRY32, Thread32First, Thread32Next,
        },
        IO::{CreateIoCompletionPort, GetQueuedCompletionStatus, OVERLAPPED},
        JobObjects::{
            IsProcessInJob, JOBOBJECT_ASSOCIATE_COMPLETION_PORT,
            JOBOBJECT_BASIC_ACCOUNTING_INFORMATION, JobObjectAssociateCompletionPortInformation,
            JobObjectBasicAccountingInformation, QueryInformationJobObject,
            SetInformationJobObject,
        },
        SystemServices::JOB_OBJECT_MSG_ACTIVE_PROCESS_ZERO,
        Threading::{
            GetProcessId, GetProcessIdOfThread, INFINITE, OpenThread, ResumeThread,
            THREAD_QUERY_LIMITED_INFORMATION, THREAD_SUSPEND_RESUME, TerminateProcess,
            WaitForSingleObject,
        },
    },
};

pub(super) struct StartError {
    pub(super) error: io::Error,
    pub(super) cleanup_confirmed: bool,
}

/// Spawn with explicit CREATE_SUSPENDED already configured by platform::wrap_scope.
/// No child instruction runs before the private completion worker is ready.
pub(super) async fn spawn(
    command: &mut CommandWrap,
) -> Result<(Box<dyn ChildWrapper>, Completion), StartError> {
    let mut started = false;
    let mut unwrapped = None;
    let mut recovery = None;
    let child = command.spawn_with(|command| {
        let child = command.spawn()?;
        started = true;
        let handle = child
            .process_handle()
            .expect("a newly spawned Tokio child owns its process handle");
        match handle.try_clone_to_owned() {
            Ok(handle) => recovery = Some(handle),
            Err(error) => {
                // Keep the original Tokio child for an actual asynchronous
                // stop/wait if a duplicate recovery handle cannot be created.
                unwrapped = Some(child);
                return Err(error);
            }
        }
        Ok(child)
    });
    let mut child = match child {
        Ok(child) => child,
        Err(error) => {
            let cleanup = match recovery {
                Some(handle) => recover(handle).await,
                None => match unwrapped {
                    Some(mut child) => {
                        let stopped = child.start_kill();
                        match child.wait().await {
                            Ok(_) => Ok(()),
                            Err(wait_error) => Err(io::Error::other(format!(
                                "suspended child wait failed: {wait_error}; stop: {stopped:?}"
                            ))),
                        }
                    }
                    None if !started => Ok(()),
                    None => Err(io::Error::other(
                        "suspended child cleanup was not confirmed",
                    )),
                },
            };
            return Err(start_error(error, cleanup));
        }
    };
    let root = recovery.expect("successful native spawn retained its process handle");
    let mut completion = match Completion::attach(child.as_ref(), root.as_handle()) {
        Ok(completion) => completion,
        Err(error) => {
            // attach never resumes the root. It therefore cannot have created
            // descendants, including when port/thread construction fails.
            let _ = child.start_kill();
            return Err(start_error(error, recover(root).await));
        }
    };
    if let Err(error) = completion.resume(root.as_handle()).await {
        // Resume can fail after a thread was resumed. Keep the monitor and Job
        // alive until both the real root wait and scope observation are joined.
        let stop = child.start_kill();
        let deadline = Instant::now() + Duration::from_secs(1);
        let (root_ended, scope_ended) = tokio::join!(recover(root), completion.finish(deadline));
        let cleanup = stop.and(root_ended).and_then(|()| match scope_ended {
            Ok(true) => Ok(()),
            Ok(false) => Err(io::Error::other("Job completion was not confirmed")),
            Err(error) => Err(error),
        });
        return Err(start_error(error, cleanup));
    }
    Ok((child, completion))
}

fn start_error(error: io::Error, cleanup: io::Result<()>) -> StartError {
    match cleanup {
        Ok(()) => StartError {
            error,
            cleanup_confirmed: true,
        },
        Err(cleanup) => StartError {
            error: io::Error::other(format!("{error}; startup cleanup failed: {cleanup}")),
            cleanup_confirmed: false,
        },
    }
}

async fn recover(handle: OwnedHandle) -> io::Result<()> {
    tokio::task::spawn_blocking(move || terminate_and_wait(handle.as_handle()))
        .await
        .map_err(io::Error::other)?
}

fn terminate_and_wait(process: BorrowedHandle<'_>) -> io::Result<()> {
    if wait(process, 0)? {
        return Ok(());
    }
    // SAFETY: this is the retained CreateProcess handle, with terminate and
    // synchronize rights; its Rust owner remains alive through both calls.
    if unsafe { TerminateProcess(process.as_raw_handle(), 1) } == 0 {
        let error = io::Error::last_os_error();
        // A prior successful Job stop can make TerminateProcess fail while
        // this exact process is still completing termination. Observe its
        // actual handle within the existing one-second cleanup budget; neither
        // the stop error nor a zero accounting count is exit evidence.
        return if wait(process, 1_000).is_ok_and(|ended| ended) {
            Ok(())
        } else {
            // A timeout or wait failure must preserve the original stop error.
            Err(error)
        };
    }
    if wait(process, INFINITE)? {
        Ok(())
    } else {
        Err(io::Error::other(
            "infinite process wait unexpectedly timed out",
        ))
    }
}

fn wait(handle: BorrowedHandle<'_>, milliseconds: u32) -> io::Result<bool> {
    // SAFETY: callers supply a live owned process handle with SYNCHRONIZE;
    // no owner closes it while this synchronous wait is in progress.
    match unsafe { WaitForSingleObject(handle.as_raw_handle(), milliseconds) } {
        WAIT_OBJECT_0 => Ok(true),
        WAIT_TIMEOUT => Ok(false),
        _ => Err(io::Error::last_os_error()),
    }
}

enum Control {
    Finish(Instant),
    Abort,
}

pub(super) struct Completion {
    job: JobObjectHandle,
    control: Sender<Control>,
    ready: Option<oneshot::Receiver<()>>,
    worker: Option<JoinHandle<io::Result<bool>>>,
}

impl Completion {
    fn attach(child: &dyn ChildWrapper, root: BorrowedHandle<'_>) -> io::Result<Self> {
        let job = (child as &dyn Any)
            .downcast_ref::<JobObjectChild>()
            .ok_or_else(|| io::Error::other("expected the configured JobObject child"))?
            .job_handle()
            .clone();
        validate_suspended_root(&job, root)?;
        let port = new_port()?;
        // Windows 8+ supports removing an existing association with NULL.
        // Explicit CREATE_SUSPENDED kept the only root from running, so no
        // process transition can be lost between the old and new ports.
        associate(&job, None)?;
        associate(&job, Some(port.as_handle()))?;
        let (control, messages) = mpsc::channel();
        let (ready, receiver) = oneshot::channel();
        let monitored_job = job.clone();
        let worker = thread::Builder::new()
            .name("minicore-job-completion".to_owned())
            .spawn(move || {
                ready
                    .send(())
                    .map_err(|()| io::Error::other("Job monitor startup was abandoned"))?;
                monitor(monitored_job, port, messages)
            })?;
        Ok(Self {
            job,
            control,
            ready: Some(receiver),
            worker: Some(worker),
        })
    }

    async fn resume(&mut self, root: BorrowedHandle<'_>) -> io::Result<()> {
        self.ready
            .take()
            .expect("the suspended root is resumed once")
            .await
            .map_err(io::Error::other)?;
        resume_threads(root)
    }

    pub(super) async fn finish(mut self, deadline: Instant) -> io::Result<bool> {
        // The control channel is separate from the completion port: shutdown
        // never posts a synthetic ACTIVE_PROCESS_ZERO packet.
        let _ = self.control.send(Control::Finish(deadline));
        let worker = self.worker.take().expect("the monitor is joined once");
        tokio::task::spawn_blocking(move || join(worker))
            .await
            .map_err(io::Error::other)?
    }
}

impl Drop for Completion {
    fn drop(&mut self) {
        if let Some(worker) = self.worker.take() {
            // Panic/abandonment backstop only, never completion evidence.
            // Abort has a finite polling path and the thread is really joined.
            let _ = self.job.terminate(1);
            let _ = self.control.send(Control::Abort);
            let _ = join(worker);
        }
    }
}

fn join(worker: JoinHandle<io::Result<bool>>) -> io::Result<bool> {
    worker
        .join()
        .map_err(|_| io::Error::other("Job completion monitor panicked"))?
}

fn monitor(
    job: JobObjectHandle,
    port: OwnedHandle,
    control: Receiver<Control>,
) -> io::Result<bool> {
    let key = job.as_handle().as_raw_handle() as usize;
    let mut deadline = None;
    loop {
        match control.try_recv() {
            Ok(Control::Finish(end)) => deadline = Some(end),
            Ok(Control::Abort) | Err(TryRecvError::Disconnected) => {
                return Err(io::Error::other("Job completion monitor was aborted"));
            }
            Err(TryRecvError::Empty) => {}
        }
        let left = deadline.map(|end: Instant| end.saturating_duration_since(Instant::now()));
        if left.is_some_and(|left| left.is_zero()) {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "the Job did not deliver ACTIVE_PROCESS_ZERO before cleanup observation ended",
            ));
        }
        let interval = left
            .unwrap_or(Duration::from_millis(100))
            .min(Duration::from_millis(100));
        let milliseconds = interval.as_nanos().div_ceil(1_000_000) as u32;
        let mut code = 0;
        let mut received_key = 0;
        let mut payload: *mut OVERLAPPED = std::ptr::null_mut();
        // SAFETY: the port is owned by this worker. All output pointers refer
        // to initialized locals that outlive the call. Job payloads encode a
        // PID or NULL; they are inspected as values and never dereferenced.
        let received = unsafe {
            GetQueuedCompletionStatus(
                port.as_raw_handle(),
                &mut code,
                &mut received_key,
                &mut payload,
                milliseconds,
            )
        };
        if received == 0 {
            let error = io::Error::last_os_error();
            if error.raw_os_error() == Some(WAIT_TIMEOUT as i32) && payload.is_null() {
                continue;
            }
            return Err(error);
        }
        if received_key != key {
            return Err(io::Error::other(
                "completion packet belongs to a different Job",
            ));
        }
        if code == JOB_OBJECT_MSG_ACTIVE_PROCESS_ZERO {
            if !payload.is_null() {
                return Err(io::Error::other(
                    "ACTIVE_PROCESS_ZERO has an invalid payload",
                ));
            }
            return Ok(true);
        }
        // NEW/EXIT/ABNORMAL_EXIT and accounting values are not completion.
    }
}

fn new_port() -> io::Result<OwnedHandle> {
    // SAFETY: INVALID_HANDLE_VALUE requests a new standalone completion port;
    // no borrowed file or completion handle is supplied.
    own(unsafe { CreateIoCompletionPort(INVALID_HANDLE_VALUE, std::ptr::null_mut(), 0, 1) })
}

fn associate(job: &JobObjectHandle, port: Option<BorrowedHandle<'_>>) -> io::Result<()> {
    let info = JOBOBJECT_ASSOCIATE_COMPLETION_PORT {
        CompletionKey: job.as_handle().as_raw_handle(),
        CompletionPort: port.map_or(std::ptr::null_mut(), |port| port.as_raw_handle()),
    };
    // SAFETY: both handles are live Rust-owned handles. This information class
    // copies the correctly sized structure synchronously; it retains no pointer
    // to this local value. NULL port removes only the existing association.
    if unsafe {
        SetInformationJobObject(
            job.as_handle().as_raw_handle(),
            JobObjectAssociateCompletionPortInformation,
            std::ptr::from_ref(&info).cast(),
            std::mem::size_of_val(&info) as u32,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

fn validate_suspended_root(job: &JobObjectHandle, root: BorrowedHandle<'_>) -> io::Result<()> {
    let mut member = 0;
    // SAFETY: root is the retained native process handle; the exact Job clone
    // remains owned, and member is a valid writable BOOL for the whole call.
    if unsafe {
        IsProcessInJob(
            root.as_raw_handle(),
            job.as_handle().as_raw_handle(),
            &mut member,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    let mut info = JOBOBJECT_BASIC_ACCOUNTING_INFORMATION::default();
    // SAFETY: the writable structure has exactly the layout/size requested by
    // this information class; no returned pointer escapes this synchronous call.
    if unsafe {
        QueryInformationJobObject(
            job.as_handle().as_raw_handle(),
            JobObjectBasicAccountingInformation,
            std::ptr::from_mut(&mut info).cast(),
            std::mem::size_of_val(&info) as u32,
            std::ptr::null_mut(),
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    if member == 0 || info.TotalProcesses != 1 || info.ActiveProcesses != 1 || wait(root, 0)? {
        return Err(io::Error::other(
            "Job initialization did not retain its single suspended root",
        ));
    }
    Ok(())
}

fn own(handle: HANDLE) -> io::Result<OwnedHandle> {
    if handle.is_null() || handle == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: every caller passes a fresh successful Win32 handle, transferring
    // its sole close obligation here. No other Rust owner was created for it.
    Ok(unsafe { OwnedHandle::from_raw_handle(handle) })
}

/// Adapted from the pinned process-wrap resume logic. Tokio does not expose the
/// CreateProcess primary-thread handle. The retained root handle pins its PID.
fn resume_threads(root: BorrowedHandle<'_>) -> io::Result<()> {
    // SAFETY: root is a live process handle with query access from CreateProcess.
    let pid = unsafe { GetProcessId(root.as_raw_handle()) };
    if pid == 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: this creates an owned system thread snapshot; no pointers are used.
    let snapshot = own(unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) })?;
    let mut entry = THREADENTRY32 {
        dwSize: std::mem::size_of::<THREADENTRY32>() as u32,
        ..Default::default()
    };
    // SAFETY: snapshot is valid; entry is correctly sized writable storage.
    if unsafe { Thread32First(snapshot.as_raw_handle(), &mut entry) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let mut threads = Vec::new();
    loop {
        if entry.th32OwnerProcessID == pid {
            // SAFETY: OpenThread accepts a numeric snapshot ID and either
            // returns a new owned handle or fails. The snapshot alone does not
            // establish identity: verify the actual handle before resuming it.
            let thread = own(unsafe {
                OpenThread(
                    THREAD_SUSPEND_RESUME | THREAD_QUERY_LIMITED_INFORMATION,
                    0,
                    entry.th32ThreadID,
                )
            })?;
            // SAFETY: the owned thread handle has query access and cannot be
            // closed or reused while this identity check is in progress.
            let owner = unsafe { GetProcessIdOfThread(thread.as_raw_handle()) };
            if owner == 0 {
                return Err(io::Error::last_os_error());
            }
            if owner != pid {
                return Err(io::Error::other(
                    "snapshot thread no longer belongs to the root",
                ));
            }
            threads.push(thread);
        }
        // SAFETY: snapshot and writable entry remain alive and correctly sized.
        if unsafe { Thread32Next(snapshot.as_raw_handle(), &mut entry) } == 0 {
            let error = io::Error::last_os_error();
            if error.raw_os_error() == Some(ERROR_NO_MORE_FILES as i32) {
                break;
            }
            return Err(error);
        }
    }
    // Complete enumeration and retain all verified identities before allowing
    // the primary thread to run and potentially create or destroy more threads.
    let mut resumed = false;
    for thread in threads {
        // SAFETY: this held handle identifies a verified root thread and has
        // THREAD_SUSPEND_RESUME access. Its owner stays alive through the call.
        let previous = unsafe { ResumeThread(thread.as_raw_handle()) };
        if previous == u32::MAX {
            return Err(io::Error::last_os_error());
        }
        resumed |= previous == 1;
    }
    if resumed {
        Ok(())
    } else {
        Err(io::Error::other("no suspended root thread was resumed"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use process_wrap::tokio::{CommandWrapper, CreationFlags, KillOnDrop};
    use std::{
        process::Stdio,
        sync::{Arc, Mutex},
    };
    use windows_sys::Win32::System::Threading::CREATE_SUSPENDED;

    fn exit_command() -> tokio::process::Command {
        let mut command = tokio::process::Command::new("cmd.exe");
        command
            .args(["/D", "/C", "exit", "0"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        command
    }

    fn suspended_command() -> CommandWrap {
        let mut command = CommandWrap::from(exit_command());
        super::super::platform::wrap_scope(&mut command);
        command.wrap(KillOnDrop);
        command
    }

    #[tokio::test]
    async fn monitor_is_ready_while_the_actual_root_is_still_suspended() {
        let mut command = suspended_command();
        let mut child = command.spawn().unwrap();
        let root = child
            .process_handle()
            .unwrap()
            .try_clone_to_owned()
            .unwrap();
        let mut completion = Completion::attach(child.as_ref(), root.as_handle()).unwrap();
        completion.ready.take().unwrap().await.unwrap();
        let still_suspended = !wait(root.as_handle(), 0).unwrap();
        resume_threads(root.as_handle()).unwrap();
        let (reaped, notified) = tokio::join!(
            child.wait(),
            completion.finish(Instant::now() + Duration::from_secs(5)),
        );
        assert!(reaped.unwrap().success());
        assert!(notified.unwrap());
        assert!(wait(root.as_handle(), 0).unwrap());
        assert!(still_suspended, "the root ran before explicit resume");
    }

    #[tokio::test]
    async fn missing_job_notification_is_not_replaced_by_zero_accounting() {
        let mut command = suspended_command();
        let mut child = command.spawn().unwrap();
        let root = child
            .process_handle()
            .unwrap()
            .try_clone_to_owned()
            .unwrap();
        let mut completion = Completion::attach(child.as_ref(), root.as_handle()).unwrap();
        // Real OS fault injection: detach the monitor port while the process
        // is still suspended, so its eventual ZERO cannot reach this monitor.
        associate(&completion.job, None).unwrap();
        completion.resume(root.as_handle()).await.unwrap();
        assert!(child.wait().await.unwrap().success());
        assert!(wait(root.as_handle(), 0).unwrap());
        // The wrapper has confirmed ActiveProcesses == 0. The separate monitor
        // must still report missing evidence and return only after its join.
        let error = completion
            .finish(Instant::now() + Duration::from_millis(20))
            .await
            .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
    }

    #[derive(Debug)]
    struct ObserveRoot(Arc<Mutex<Option<(OwnedHandle, bool)>>>);

    impl CommandWrapper for ObserveRoot {
        fn wrap_child(
            &mut self,
            child: Box<dyn ChildWrapper>,
            _: &CommandWrap,
        ) -> io::Result<Box<dyn ChildWrapper>> {
            let root = child.process_handle().unwrap().try_clone_to_owned()?;
            let suspended = !wait(root.as_handle(), 0)?;
            *self.0.lock().unwrap() = Some((root, suspended));
            Ok(child)
        }
    }

    #[tokio::test]
    async fn initialization_failure_reaps_the_actual_suspended_root() {
        let observed = Arc::new(Mutex::new(None));
        let mut command = CommandWrap::from(exit_command());
        command
            .wrap(CreationFlags(CREATE_SUSPENDED))
            .wrap(KillOnDrop)
            .wrap(ObserveRoot(Arc::clone(&observed)));
        // Deliberately omit JobObject to fail attachment after real creation.
        // This is a construction-failure fixture, not a production launch mode.
        let Err(error) = spawn(&mut command).await else {
            panic!("a child without its configured Job was accepted");
        };
        let (root, suspended) = observed.lock().unwrap().take().unwrap();
        assert!(suspended);
        assert!(error.cleanup_confirmed, "{}", error.error);
        assert!(
            wait(root.as_handle(), 0).unwrap(),
            "startup failure left the actual suspended process alive"
        );
    }
}
