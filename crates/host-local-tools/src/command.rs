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
) -> Result<ToolExecutionOutcome, ToolError> {
    if input.command.is_empty() || input.command.contains('\0') || input.timeout_seconds == Some(0)
    {
        return Err(ToolError::InvalidInvocation);
    }
    #[cfg(unix)]
    {
        unix::run(binding, id, input, context).await
    }
    #[cfg(not(unix))]
    {
        let _ = (binding, id, input, context);
        crate::files::unsupported_command()
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
        let mut command = Command::new("/bin/bash");
        command
            .args(["--noprofile", "--norc", "-c", &input.command])
            .current_dir(cwd)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .process_group(0)
            .kill_on_drop(true);
        let mut child = match command.spawn() {
            Ok(child) => child,
            Err(error) => return business_error("process_start", error.to_string()),
        };
        let pid = child.id().expect("spawned child has a process ID");
        let mut stdout = child.stdout.take().expect("configured stdout pipe");
        let mut stderr = child.stderr.take().expect("configured stderr pipe");
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
                result = child.wait() => { waited = Some(result); break; }
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
        // Stop descendants even after a natural leader exit. Requesting a kill
        // is not a completed wait; the direct child is always actually joined.
        if let Err(error) = killpg(Pid::from_raw(pid as i32), Signal::SIGKILL)
            && error != Errno::ESRCH
        {
            errors.push(format!("process-group stop failed: {error}"));
            handling_failed = true;
        }
        if waited.is_none() {
            if let Err(error) = child.start_kill() {
                errors.push(format!("direct-child stop failed: {error}"));
                handling_failed = true;
            }
            waited = Some(child.wait().await);
        }
        let exit = match waited.expect("a real wait was attempted") {
            Ok(exit) => Some(exit),
            Err(error) => {
                errors.push(format!("direct-child wait failed: {error}"));
                handling_failed = true;
                None
            }
        };
        // No reader task is detached. Drain actual buffered output and EOF;
        // an escaped descendant may hold a pipe, so mark that stream incomplete.
        let cleanup_deadline = tokio::time::Instant::now() + Duration::from_secs(1);
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
        if handling_failed && status == CommandStatus::Exited {
            status = CommandStatus::Failed;
        }
        let result = CommandResult {
            status,
            pid,
            exit_code: exit.as_ref().and_then(|exit| exit.code()),
            signal: exit.as_ref().and_then(|exit| exit.signal()),
            process_waited: exit.is_some(),
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
}
