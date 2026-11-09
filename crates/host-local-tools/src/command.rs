use std::sync::Arc;

use minicore_runtime::{
    ToolCallId,
    tools::{ToolContext, ToolError, ToolExecutionOutcome},
};
use serde::Deserialize;

use crate::Binding;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct BashInput {
    command: String,
    #[serde(default = "root")]
    cwd: String,
    timeout_seconds: Option<u64>,
}
fn root() -> String {
    ".".to_owned()
}

pub(crate) async fn run(
    binding: Arc<Binding>,
    id: ToolCallId,
    input: BashInput,
    context: ToolContext,
) -> crate::scope::OwnedOutcome {
    if input.command.is_empty() || input.command.contains('\0') || input.timeout_seconds == Some(0)
    {
        return crate::scope::OwnedOutcome::settled(Err(ToolError::InvalidInvocation));
    }
    #[cfg(unix)]
    {
        let mut cleanup_confirmed = true;
        let result = unix::run(binding, id, input, context, &mut cleanup_confirmed).await;
        crate::scope::OwnedOutcome {
            result,
            cleanup_confirmed,
        }
    }
    #[cfg(not(unix))]
    {
        let _ = (binding, id, input, context);
        crate::scope::OwnedOutcome::settled(crate::files::unsupported_command())
    }
}

#[cfg(unix)]
mod unix {
    use super::*;
    use crate::{
        CommandFileChanges, CommandResult, CommandStatus, CommandStream, LocalToolSink,
        OutputChannel, business_error, check, completed,
    };
    use nix::{
        errno::Errno,
        sys::signal::{Signal, killpg},
        unistd::Pid,
    };
    use process_wrap::tokio::{KillOnDrop, ProcessGroup, TokioCommandWrap};
    use std::{
        io,
        os::unix::process::ExitStatusExt,
        process::Stdio,
        time::{Duration, Instant},
    };
    use tokio::{io::AsyncReadExt, process::Command};

    struct Stream {
        channel: OutputChannel,
        bytes: Vec<u8>,
        closed: bool,
        complete: bool,
    }
    impl Stream {
        fn new(channel: OutputChannel) -> Self {
            Self {
                channel,
                bytes: Vec::new(),
                closed: false,
                complete: false,
            }
        }
        fn accept(
            &mut self,
            read: io::Result<usize>,
            buffer: &[u8],
            sink: &dyn LocalToolSink,
            id: &ToolCallId,
            record_failed: &mut bool,
            errors: &mut Vec<String>,
        ) -> bool {
            match read {
                Ok(0) => {
                    self.closed = true;
                    self.complete = true;
                    false
                }
                Ok(count) => {
                    let offset = self.bytes.len() as u64;
                    self.bytes.extend_from_slice(&buffer[..count]);
                    if !*record_failed
                        && sink
                            .output(id, self.channel, offset, &buffer[..count])
                            .is_err()
                    {
                        *record_failed = true;
                        return true;
                    }
                    false
                }
                Err(error) => {
                    self.closed = true;
                    errors.push(format!("{:?} read failed: {error}", self.channel));
                    true
                }
            }
        }
        fn result(self) -> CommandStream {
            CommandStream {
                output: self.bytes.into(),
                complete: self.complete,
            }
        }
    }

    pub(super) async fn run(
        binding: Arc<Binding>,
        id: ToolCallId,
        input: BashInput,
        context: ToolContext,
        cleanup_confirmed: &mut bool,
    ) -> Result<ToolExecutionOutcome, ToolError> {
        let deadline = match input.timeout_seconds {
            Some(seconds) => Instant::now()
                .checked_add(Duration::from_secs(seconds))
                .ok_or(ToolError::InvalidInvocation)?
                .min(context.deadline),
            None => context.deadline,
        };
        check(&context)?;
        let cwd = match crate::files::directory(&binding.workspace, &input.cwd) {
            Ok(cwd) => cwd,
            Err(error) => return business_error("working_directory", error.to_string()),
        };
        check(&context)?;
        let excluded_names = binding.environment.excluded_names();
        let mut command = Command::new("/bin/bash");
        for name in excluded_names {
            command.env_remove(name);
        }
        command
            .args(["--noprofile", "--norc", "-c", &input.command])
            .current_dir(cwd)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        // Adapted from minicore-agent's command owner: the wrapper owns the
        // real group wait. KillOnDrop is a backstop, never completion evidence.
        let mut command = TokioCommandWrap::from(command);
        command.wrap(ProcessGroup::leader()).wrap(KillOnDrop);
        let mut child = match command.spawn() {
            Ok(child) => child,
            Err(error) => return business_error("process_start", error.to_string()),
        };
        *cleanup_confirmed = false;
        let pid = child.id().expect("spawned child has a process ID");
        let mut stdout = child.stdout().take().expect("configured stdout pipe");
        let mut stderr = child.stderr().take().expect("configured stderr pipe");
        let mut out = Stream::new(OutputChannel::Stdout);
        let mut err = Stream::new(OutputChannel::Stderr);
        let mut out_buffer = [0; 16 * 1024];
        let mut err_buffer = [0; 16 * 1024];
        let mut errors = Vec::new();
        let mut record_failed = false;
        let mut handling_failed = false;
        let mut status = CommandStatus::Exited;
        let mut waited = None;
        loop {
            tokio::select! {
                biased;
                _ = context.cancellation.cancelled() => {
                    status = if Instant::now() >= deadline { CommandStatus::TimedOut } else { CommandStatus::Cancelled };
                    break;
                }
                _ = tokio::time::sleep_until(deadline.into()) => {
                    status = CommandStatus::TimedOut;
                    break;
                }
                result = child.inner_mut().wait() => { waited = Some(result); break; }
                read = stdout.read(&mut out_buffer), if !out.closed => {
                    if out.accept(read, &out_buffer, binding.sink.as_ref(), &id, &mut record_failed, &mut errors) {
                        handling_failed = true; status = CommandStatus::Failed; break;
                    }
                }
                read = stderr.read(&mut err_buffer), if !err.closed => {
                    if err.accept(read, &err_buffer, binding.sink.as_ref(), &id, &mut record_failed, &mut errors) {
                        handling_failed = true; status = CommandStatus::Failed; break;
                    }
                }
            }
        }
        // Stop descendants even after a natural leader exit and pipe EOF.
        // A successful signal request is not evidence that the group is gone.
        if let Err(error) = killpg(Pid::from_raw(pid as i32), Signal::SIGKILL)
            && error != Errno::ESRCH
        {
            errors.push(format!("process-group stop failed: {error}"));
            handling_failed = true;
        }
        if waited.is_none()
            && let Err(error) = child.inner_mut().start_kill()
        {
            errors.push(format!("direct-child stop failed: {error}"));
            handling_failed = true;
        }
        // Perform one full wrapper wait, never drop it partway through. Its
        // direct-child wait and group-child reap run alongside both drains.
        // No reader task is detached. An escaped descendant can hold a pipe;
        // that bounded observation ends incomplete, not with a fabricated EOF.
        let cleanup_deadline = tokio::time::Instant::now() + Duration::from_secs(1);
        let drains = async {
            while !out.closed || !err.closed {
                tokio::select! {
                    biased;
                    _ = tokio::time::sleep_until(cleanup_deadline) => {
                        errors.push("output pipe did not reach EOF after process-group cleanup".to_owned());
                        handling_failed = true;
                        break;
                    }
                    read = stdout.read(&mut out_buffer), if !out.closed => {
                        handling_failed |= out.accept(read, &out_buffer, binding.sink.as_ref(), &id, &mut record_failed, &mut errors);
                    }
                    read = stderr.read(&mut err_buffer), if !err.closed => {
                        handling_failed |= err.accept(read, &err_buffer, binding.sink.as_ref(), &id, &mut record_failed, &mut errors);
                    }
                }
            }
        };
        let (reaped, ()) = tokio::join!(Box::into_pin(child.wait()), drains);
        let wrapper_waited = reaped.is_ok();
        let exit = match reaped {
            Ok(exit) => Some(exit),
            Err(error) => {
                errors.push(format!("process-group wait failed: {error}"));
                handling_failed = true;
                waited.and_then(Result::ok)
            }
        };
        // The wrapper can reap only children of this process. A remaining
        // group member need not be our child, so ECHILD is not scope evidence.
        let group_gone = match confirmed_group_gone(pid).await {
            Ok(true) => true,
            Ok(false) => {
                errors.push("process group still exists after cleanup observation".to_owned());
                handling_failed = true;
                false
            }
            Err(error) => {
                errors.push(format!("process-group end could not be confirmed: {error}"));
                handling_failed = true;
                false
            }
        };
        if handling_failed && status == CommandStatus::Exited {
            status = CommandStatus::Failed;
        }
        *cleanup_confirmed = wrapper_waited && group_gone && out.closed && err.closed;
        let result = CommandResult {
            status,
            pid,
            exit_code: exit.as_ref().and_then(|exit| exit.code()),
            signal: exit.as_ref().and_then(|exit| exit.signal()),
            process_waited: exit.is_some(),
            cleanup_confirmed: *cleanup_confirmed,
            stdout: out.result(),
            stderr: err.result(),
            file_changes: CommandFileChanges::Unknown,
            cleanup_errors: errors,
        };
        if binding.sink.command(&id, &result).is_err() {
            record_failed = true;
        }
        // Keep the primary stop fact. Internal recording/cleanup failures are
        // independently retained in the owner callback or finalization result.
        if record_failed || handling_failed {
            return Err(ToolError::Internal);
        }
        match status {
            CommandStatus::Cancelled => Err(ToolError::Cancelled),
            CommandStatus::TimedOut => Err(ToolError::TimedOut),
            CommandStatus::Failed => Err(ToolError::Internal),
            CommandStatus::Exited => completed(result),
        }
    }

    /// Adapted from minicore-agent's scope observation. ESRCH is the only
    /// affirmative group-end evidence; a present or unobservable group stays
    /// unconfirmed even when its leader exited and both pipes reached EOF.
    async fn confirmed_group_gone(pid: u32) -> Result<bool, Errno> {
        for _ in 0..10 {
            if probe_group(pid)? {
                return Ok(true);
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        probe_group(pid)
    }

    fn probe_group(pid: u32) -> Result<bool, Errno> {
        match killpg(Pid::from_raw(pid as i32), None) {
            Ok(()) => Ok(false),
            Err(Errno::ESRCH) => Ok(true),
            Err(error) => Err(error),
        }
    }
}
